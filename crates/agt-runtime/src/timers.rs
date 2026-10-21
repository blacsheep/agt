//! Timers: a message the conversation receives later, once or on a cron schedule.
//! It is posted as the conversation's human, so it authorizes a fresh discussion
//! exactly like a message they send; nothing runs between firings.
use crate::ScheduleStore;
use crate::schedule_store::now_ms;
use agt_im::{
    ConversationId, ConversationState, ImStore, PrincipalId, PrincipalKind, SendMessageRequest,
};
use agt_service::{Actor, Refusal};
use anyhow::{Result, bail};
use croner::Cron;
use croner::parser::{CronParser, Seconds};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) const SCHEMA: agt_storage::sqlite::Schema = agt_storage::sqlite::Schema {
    module: "timers",
    version: 1,
    ddl: "CREATE TABLE timers(id TEXT PRIMARY KEY,conversation TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('active','done','canceled')),next_at INTEGER,record TEXT NOT NULL);
        CREATE INDEX timer_due ON timers(state,next_at);
        CREATE TRIGGER timer_created AFTER INSERT ON timers BEGIN
            INSERT INTO change_outbox(conversation,kind,at) VALUES(NEW.conversation,'schedule',CAST(unixepoch('subsec')*1000 AS INTEGER));
        END;
        CREATE TRIGGER timer_changed AFTER UPDATE ON timers BEGIN
            INSERT INTO change_outbox(conversation,kind,at) VALUES(NEW.conversation,'schedule',CAST(unixepoch('subsec')*1000 AS INTEGER));
        END;",
};

const MAX_ACTIVE: i64 = 20;
const MAX_AHEAD_MS: i64 = 366 * 86_400_000;
const MIN_AHEAD_MS: i64 = 10_000;
/// Recurring timers each start a discussion, so they may not fire more often.
const MIN_INTERVAL_MS: i64 = 10 * 60_000;
const LATE_MS: i64 = 60_000;
const RETRY_MS: i64 = 60_000;
const MAX_ATTEMPTS: u32 = 5;
const MAX_DISCUSSION_SECONDS: u32 = 7 * 86_400;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerState {
    Active,
    Done,
    Canceled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Timer {
    pub id: String,
    pub conversation: String,
    pub name: Option<String>,
    pub text: String,
    pub mentions: Vec<String>,
    /// Who set it: an Agent or a person.
    pub creator: String,
    /// The human it speaks as when it fires.
    pub sender: String,
    pub cron: Option<String>,
    pub timezone: String,
    /// How long the discussion it starts stays authorized; workspace setting when absent.
    #[serde(default)]
    pub discussion_seconds: Option<u32>,
    pub next_at: Option<i64>,
    pub state: TimerState,
    pub created_at: i64,
    pub fired: u32,
    pub last_fired_at: Option<i64>,
    pub last_message: Option<String>,
    pub reason: Option<String>,
    #[serde(default)]
    attempts: u32,
    /// The firing in progress, kept so a retry posts the identical message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<Pending>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pending {
    scheduled_for: i64,
    body: String,
    mentions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    conversation: Option<String>,
    text: String,
    name: Option<String>,
    #[serde(default)]
    mentions: Vec<String>,
    after_seconds: Option<i64>,
    at: Option<String>,
    cron: Option<String>,
    timezone: Option<String>,
    discussion_seconds: Option<u32>,
}

/// Timer tools for a fixed actor; `task` is the hosted execution, if any.
pub(crate) fn call(
    home: &Path,
    actor: &Actor,
    task: Option<&crate::ScheduledTask>,
    name: &str,
    args: Value,
) -> Result<Value> {
    let operation = |store: &ScheduleStore| -> Result<Value> {
        let im = open_im(home)?;
        match name {
            "timer_create" => create(
                store,
                &im,
                actor,
                task,
                serde_json::from_value(args.clone())?,
            ),
            "timer_list" => list(store, &im, actor, &args),
            "timer_cancel" => cancel(store, &im, actor, &args),
            _ => bail!("unknown timer tool"),
        }
    };
    if let Some(task) = task {
        // Hosted changes commit only while this execution still holds its authorization.
        crate::scheduler::guarded_tool(home, task, operation)
    } else {
        let store = ScheduleStore::open(home)?;
        let db = store.connection.clone();
        let tx = agt_storage::sqlite::write(&db)?;
        let value = operation(&store)?;
        tx.commit()?;
        agt_storage::wake::notify(home);
        Ok(value)
    }
}

fn create(
    store: &ScheduleStore,
    im: &ImStore,
    actor: &Actor,
    task: Option<&crate::ScheduledTask>,
    input: CreateInput,
) -> Result<Value> {
    let conversation = match (task, input.conversation) {
        (Some(task), Some(c)) if c != task.conversation => {
            return Err(Refusal(
                "Agents can set timers only in the conversation they are working in",
            )
            .into());
        }
        (Some(task), _) => task.conversation.clone(),
        (None, Some(c)) => c,
        (None, None) => return Err(Refusal("conversation is required").into()),
    };
    let creator = actor.principal.to_string();
    member(im, &conversation, &creator)?;
    let human = im.get_principal(&actor.principal)?.kind == PrincipalKind::User;
    if task.is_none() && !human {
        bail!("local timer changes require a human actor");
    }
    // A timer speaks as a person: whoever sets it, or the workspace owner for an Agent.
    let sender = if human {
        creator.clone()
    } else {
        im.workspace_owner()?
            .map(|p| p.to_string())
            .ok_or(Refusal("timers need a workspace owner"))?
    };
    if member(im, &conversation, &sender).is_err()
        || im.get_principal(&PrincipalId::new(&sender))?.kind != PrincipalKind::User
    {
        return Err(Refusal(
            "A timer posts as the workspace owner, who is not in this conversation, so timers cannot be set here. To wait for other agents, end your turn: their replies, or a goal review once the workspace goes quiet, wake you.",
        )
        .into());
    }
    let text = input.text.trim().to_owned();
    if text.is_empty() || text.chars().count() > 4000 {
        return Err(Refusal("text must contain 1..4000 characters").into());
    }
    let name = input
        .name
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty());
    if name.as_ref().is_some_and(|n| n.chars().count() > 80) {
        return Err(Refusal("name must be at most 80 characters").into());
    }
    if input
        .discussion_seconds
        .is_some_and(|s| !(60..=MAX_DISCUSSION_SECONDS).contains(&s))
    {
        return Err(Refusal("discussion_seconds must be 60..604800 (one week)").into());
    }
    if input.mentions.len() > 10 {
        return Err(Refusal("at most 10 mentions").into());
    }
    let mut mentions = Vec::new();
    for principal in input.mentions {
        if !mentions.contains(&principal) {
            mentions.push(principal);
        }
    }
    let ids: Vec<PrincipalId> = mentions.iter().map(PrincipalId::new).collect();
    agt_service::check_mentions(im, &ConversationId::new(conversation.as_str()), &ids)?;
    let config = agt_config::RuntimeConfig::load(&store.home)?;
    let zone = match &input.timezone {
        Some(name) => crate::clock::named_zone(name)?,
        None => crate::clock::zone(&config),
    };
    let now = now_ms();
    let (next_at, cron) = match (input.after_seconds, input.at, input.cron) {
        (Some(seconds), None, None) => (now.saturating_add(seconds.saturating_mul(1000)), None),
        (None, Some(at), None) => (crate::clock::parse(&at, &zone)?, None),
        (None, None, Some(pattern)) => {
            let parsed = parse_cron(&pattern)?;
            check_interval(&parsed, now, &zone)?;
            (
                next_after(&parsed, now, &zone)?,
                Some(pattern.trim().to_owned()),
            )
        }
        _ => return Err(Refusal("choose exactly one of after_seconds, at or cron").into()),
    };
    if cron.is_none() && !(now + MIN_AHEAD_MS..=now + MAX_AHEAD_MS).contains(&next_at) {
        return Err(
            Refusal("a one-off timer must fire between 10 seconds and 366 days from now").into(),
        );
    }
    let active: i64 = store.connection.query_row(
        "SELECT COUNT(*) FROM timers WHERE conversation=?1 AND state='active'",
        [&conversation],
        |r| r.get(0),
    )?;
    if active >= MAX_ACTIVE {
        return Err(
            Refusal("this conversation already has 20 active timers; cancel one first").into(),
        );
    }
    let timer = Timer {
        id: uuid::Uuid::new_v4().to_string(),
        conversation,
        name,
        text,
        mentions,
        creator,
        sender,
        cron,
        timezone: crate::clock::zone_name(&zone).to_owned(),
        discussion_seconds: input.discussion_seconds,
        next_at: Some(next_at),
        state: TimerState::Active,
        created_at: now,
        fired: 0,
        last_fired_at: None,
        last_message: None,
        reason: None,
        attempts: 0,
        pending: None,
    };
    store.connection.execute(
        "INSERT INTO timers(id,conversation,state,next_at,record) VALUES(?1,?2,'active',?3,?4)",
        params![
            timer.id,
            timer.conversation,
            next_at,
            serde_json::to_string(&timer)?
        ],
    )?;
    view(&timer, now)
}

fn list(store: &ScheduleStore, im: &ImStore, actor: &Actor, args: &Value) -> Result<Value> {
    let finished = args["include_finished"].as_bool().unwrap_or(false);
    let filter = args["conversation"].as_str();
    let now = now_ms();
    let mut timers = Vec::new();
    for timer in query(
        store,
        "SELECT record FROM timers ORDER BY state='active' DESC, next_at, rowid DESC LIMIT 500",
    )? {
        if (finished || timer.state == TimerState::Active)
            && filter.is_none_or(|c| c == timer.conversation)
            && member(im, &timer.conversation, actor.principal.as_str()).is_ok()
        {
            timers.push(view(&timer, now)?);
        }
        if timers.len() == 100 {
            break;
        }
    }
    Ok(json!({ "timers": timers }))
}

fn cancel(store: &ScheduleStore, im: &ImStore, actor: &Actor, args: &Value) -> Result<Value> {
    let id = args["timer_id"]
        .as_str()
        .ok_or(Refusal("timer_id required"))?;
    let mut timer = get(store, id)?.ok_or(Refusal("timer not found; use timer_list"))?;
    member(im, &timer.conversation, actor.principal.as_str())
        .map_err(|_| Refusal("timer not found; use timer_list"))?;
    let human = im.get_principal(&actor.principal)?.kind == PrincipalKind::User;
    if !human && timer.creator != actor.principal.as_str() {
        return Err(Refusal("Agents can cancel only timers they set").into());
    }
    if timer.state == TimerState::Active {
        let reason = args["reason"].as_str().unwrap_or("").trim();
        timer.state = TimerState::Canceled;
        timer.next_at = None;
        timer.pending = None;
        timer.reason = Some(if reason.is_empty() {
            format!("canceled by {}", actor.principal)
        } else {
            reason.chars().take(240).collect()
        });
        save(store, &timer)?;
    }
    view(&timer, now_ms())
}

/// Post every due timer. Run only by the scheduler that holds the home lease.
/// A failed delivery is retried a few times; it never stops the scheduler.
pub(crate) fn fire_due(home: &Path, store: &ScheduleStore) -> Result<()> {
    let now = now_ms();
    let due = query_due(store, now)?;
    if due.is_empty() {
        return Ok(());
    }
    let im = open_im(home)?;
    for mut timer in due {
        if let Err(error) = fire(store, &im, &mut timer, now) {
            timer.attempts += 1;
            if timer.attempts >= MAX_ATTEMPTS {
                timer.state = TimerState::Canceled;
                timer.next_at = None;
                timer.pending = None;
                timer.reason = Some(crate::Reason::because("delivery failed", error).to_string());
            } else {
                timer.next_at = Some(now + RETRY_MS);
            }
            save(store, &timer)?;
        }
    }
    Ok(())
}

fn fire(store: &ScheduleStore, im: &ImStore, timer: &mut Timer, now: i64) -> Result<()> {
    let conversation = ConversationId::new(&timer.conversation);
    if im.get_conversation(&conversation)?.state != ConversationState::Active
        || member(im, &timer.conversation, &timer.sender).is_err()
    {
        timer.state = TimerState::Canceled;
        timer.next_at = None;
        timer.pending = None;
        timer.reason = Some("the conversation or its human is gone".into());
        return save(store, timer);
    }
    let zone = crate::clock::named_zone(&timer.timezone)?;
    let pending = match timer.pending.clone() {
        Some(pending) => pending,
        None => {
            let scheduled_for = timer.next_at.unwrap_or(now);
            let mut body = format!(
                "[AGT timer {}{}]\n{}",
                timer.id,
                timer
                    .name
                    .as_ref()
                    .map(|n| format!(" · {n}"))
                    .unwrap_or_default(),
                timer.text
            );
            if now - scheduled_for > LATE_MS {
                body.push_str(&format!(
                    "\n\n(Scheduled for {}; delivered late because agt was not running.)",
                    crate::clock::local(scheduled_for, &zone)
                ));
            }
            let pending = Pending {
                scheduled_for,
                body,
                mentions: timer
                    .mentions
                    .iter()
                    .filter(|p| member(im, &timer.conversation, p).is_ok())
                    .cloned()
                    .collect(),
            };
            timer.pending = Some(pending.clone());
            save(store, timer)?;
            pending
        }
    };
    let sent = im.send_message(SendMessageRequest {
        conversation_id: conversation,
        sender_id: PrincipalId::new(&timer.sender),
        body: pending.body,
        client_message_id: Some(format!("timer:{}:{}", timer.id, pending.scheduled_for)),
        reply_to_id: None,
        metadata: json!({
            "mentions": pending.mentions,
            "timer": {"id": timer.id, "creator": timer.creator, "scheduled_for": pending.scheduled_for},
        }),
        idempotency_key: None,
    })?;
    // Exactly what a person's message does: start or renew this conversation's discussion.
    store.authorize_human(
        sent.message.id.as_str(),
        &timer.conversation,
        sent.message.seq,
        now,
    )?;
    if let Some(seconds) = timer.discussion_seconds
        && let Some(discussion) = store.human_discussion(sent.message.id.as_str())?
    {
        store.set_discussion_expiry(&discussion, now + i64::from(seconds) * 1000)?;
    }
    timer.fired += 1;
    timer.attempts = 0;
    timer.last_fired_at = Some(now);
    timer.last_message = Some(sent.message.id.to_string());
    timer.pending = None;
    timer.next_at = match &timer.cron {
        // Missed occurrences collapse into this one delivery.
        Some(pattern) => Some(next_after(&parse_cron(pattern)?, now, &zone)?),
        None => {
            timer.state = TimerState::Done;
            None
        }
    };
    save(store, timer)
}

fn parse_cron(pattern: &str) -> Result<Cron> {
    CronParser::builder()
        .seconds(Seconds::Disallowed)
        .build()
        .parse(pattern.trim())
        .map_err(|_| {
            Refusal(
                "invalid cron pattern; use five fields: minute hour day-of-month month day-of-week",
            )
            .into()
        })
}

fn next_after(cron: &Cron, after: i64, zone: &jiff::tz::TimeZone) -> Result<i64> {
    let next = cron
        .find_next_occurrence(&crate::clock::zoned(after, zone), false)
        .map_err(|e| anyhow::anyhow!("cron pattern has no upcoming time: {e}"))?;
    Ok(next.timestamp().as_millisecond())
}

fn check_interval(cron: &Cron, now: i64, zone: &jiff::tz::TimeZone) -> Result<()> {
    let mut previous = next_after(cron, now, zone)?;
    if previous - now > MAX_AHEAD_MS {
        return Err(Refusal("cron pattern must fire within the next 366 days").into());
    }
    for _ in 0..48 {
        let Ok(next) = next_after(cron, previous, zone) else {
            break;
        };
        if next - previous < MIN_INTERVAL_MS {
            return Err(Refusal("recurring timers must fire at least 10 minutes apart").into());
        }
        previous = next;
    }
    Ok(())
}

fn view(timer: &Timer, now: i64) -> Result<Value> {
    let zone = crate::clock::named_zone(&timer.timezone)?;
    let mut value = serde_json::to_value(timer)?;
    value["next_at"] = json!(timer.next_at.map(|at| crate::clock::local(at, &zone)));
    value["next_in_seconds"] = json!(timer.next_at.map(|at| (at - now).max(0) / 1000));
    value["created_at"] = json!(crate::clock::local(timer.created_at, &zone));
    value["last_fired_at"] = json!(timer.last_fired_at.map(|at| crate::clock::local(at, &zone)));
    value["schedule"] = json!(match &timer.cron {
        Some(pattern) => parse_cron(pattern)?.describe(),
        None => "once".into(),
    });
    if let Some(map) = value.as_object_mut() {
        map.remove("attempts");
    }
    Ok(value)
}

pub(crate) fn member(im: &ImStore, conversation: &str, principal: &str) -> Result<()> {
    let chat = ConversationId::new(conversation);
    let principal = PrincipalId::new(principal);
    if im.get_conversation(&chat)?.state != ConversationState::Active
        || im.get_principal(&principal)?.deleted_at.is_some()
        || im.get_member(&chat, &principal)?.left_message_seq.is_some()
    {
        bail!("active identity and conversation membership required");
    }
    Ok(())
}

pub(crate) fn open_im(home: &Path) -> Result<ImStore> {
    Ok(ImStore::open(crate::store::database_path(
        home, "im", "im.db",
    )?)?)
}

fn get(store: &ScheduleStore, id: &str) -> Result<Option<Timer>> {
    store
        .connection
        .query_row("SELECT record FROM timers WHERE id=?1", [id], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .map(|record| Ok(serde_json::from_str(&record)?))
        .transpose()
}

fn query(store: &ScheduleStore, sql: &str) -> Result<Vec<Timer>> {
    let mut statement = store.connection.prepare(sql)?;
    let rows = statement
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.iter()
        .map(|record| Ok(serde_json::from_str(record)?))
        .collect()
}

fn query_due(store: &ScheduleStore, now: i64) -> Result<Vec<Timer>> {
    let mut statement = store.connection.prepare(
        "SELECT record FROM timers WHERE state='active' AND next_at<=?1 ORDER BY next_at LIMIT 50",
    )?;
    let rows = statement
        .query_map([now], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.iter()
        .map(|record| Ok(serde_json::from_str(record)?))
        .collect()
}

fn save(store: &ScheduleStore, timer: &Timer) -> Result<()> {
    store.connection.execute(
        "UPDATE timers SET state=?2,next_at=?3,record=?4 WHERE id=?1",
        params![
            timer.id,
            serde_json::to_value(timer.state)?.as_str(),
            timer.next_at,
            serde_json::to_string(timer)?
        ],
    )?;
    Ok(())
}
