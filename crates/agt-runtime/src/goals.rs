//! Goals: an outcome an agent owns until it is done. When the workspace goes quiet
//! while a goal is still open, its owner is asked to review it: report it done, unblock
//! the work, or say it waits for a person. Work that stalls silently, because nobody
//! was woken to take the next step, then does not stay stalled.
use crate::ScheduleStore;
use crate::schedule_store::now_ms;
use crate::timers::{member, open_im};
use agt_im::{
    ConversationId, ImStore, MessageListOptions, PrincipalId, PrincipalKind, SendMessageRequest,
};
use agt_service::{Actor, Refusal};
use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) const SCHEMA: agt_storage::sqlite::Schema = agt_storage::sqlite::Schema {
    module: "goals",
    version: 1,
    ddl: "CREATE TABLE goals(id TEXT PRIMARY KEY,owner TEXT NOT NULL,conversation TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('active','waiting_human','done','abandoned')),record TEXT NOT NULL);
        CREATE INDEX goal_state ON goals(state);
        CREATE TRIGGER goal_created AFTER INSERT ON goals BEGIN
            INSERT INTO change_outbox(conversation,kind,at) VALUES(NEW.conversation,'schedule',CAST(unixepoch('subsec')*1000 AS INTEGER));
        END;
        CREATE TRIGGER goal_changed AFTER UPDATE ON goals BEGIN
            INSERT INTO change_outbox(conversation,kind,at) VALUES(NEW.conversation,'schedule',CAST(unixepoch('subsec')*1000 AS INTEGER));
        END;",
};

/// Open goals one identity may own at once.
const MAX_OPEN: i64 = 5;
/// Reviews of one goal in all, whatever their outcome.
const MAX_REVIEWS: u32 = 50;
/// A goal waiting for a person is reviewed again after this long, doubling each time
/// nobody answers, up to `MAX_RECHECK_MS`.
const RECHECK_MS: i64 = 30 * 60_000;
const MAX_RECHECK_MS: i64 = 8 * 60 * 60_000;
/// Timers or durable waits due this soon mean work resumes by itself.
const RESUMES_SOON_MS: i64 = 10 * 60_000;
const MAX_TEXT: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalState {
    Active,
    WaitingHuman,
    Done,
    Abandoned,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    /// The identity responsible for it, usually the lead.
    pub owner: String,
    pub conversation: String,
    pub text: String,
    pub done_when: String,
    pub state: GoalState,
    /// The person reviews speak as: who set the goal, or the workspace owner for an agent.
    pub sender: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub reviews: u32,
    /// Reviews in a row after which nobody but the owner had done anything.
    pub stalled_reviews: u32,
    pub last_review_at: Option<i64>,
    /// Latest activity by anyone but the owner when it was last reviewed.
    pub progress_mark: Option<i64>,
    pub last_review_message: Option<String>,
    /// Evidence when done, the question when waiting, or the reason when abandoned.
    pub outcome: Option<String>,
    pub waiting_since: Option<i64>,
    /// Reviews in a row that found the goal still waiting for a person.
    #[serde(default)]
    pub waiting_rechecks: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    conversation: Option<String>,
    text: String,
    done_when: String,
}

/// Goal tools for a fixed actor; `task` is the hosted execution, if any.
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
            "goal_create" => create(
                store,
                &im,
                actor,
                task,
                serde_json::from_value(args.clone())?,
            ),
            "goal_get" => {
                let goal = visible(store, &im, actor, &args)?;
                Ok(view(&goal))
            }
            "goal_list" => list(store, &im, actor, &args),
            "goal_complete" => finish(store, &im, actor, &args, GoalState::Done, "evidence"),
            "goal_wait_human" => finish(
                store,
                &im,
                actor,
                &args,
                GoalState::WaitingHuman,
                "question",
            ),
            "goal_abandon" => finish(store, &im, actor, &args, GoalState::Abandoned, "reason"),
            _ => bail!("unknown goal tool"),
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

fn human(im: &ImStore, principal: &str) -> Result<bool> {
    Ok(im.get_principal(&PrincipalId::new(principal))?.kind == PrincipalKind::User)
}

fn text(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > MAX_TEXT {
        return Err(Refusal(
            "text, done_when, evidence, question and reason must each contain 1..2000 characters",
        )
        .into());
    }
    Ok(value.to_owned())
}

fn create(
    store: &ScheduleStore,
    im: &ImStore,
    actor: &Actor,
    task: Option<&crate::ScheduledTask>,
    input: CreateInput,
) -> Result<Value> {
    let owner = actor.principal.to_string();
    let conversation = match (task, input.conversation) {
        (Some(task), None) => task.conversation.clone(),
        (Some(task), Some(c)) if c == task.conversation => c,
        (Some(_), Some(_)) => {
            return Err(Refusal("agents can only set goals in their current conversation").into());
        }
        (None, Some(c)) => c,
        (None, None) => return Err(Refusal("conversation is required").into()),
    };
    member(im, &conversation, &owner)?;
    // Reviews speak as a person, like timers: who set it, or the workspace owner.
    let sender = if human(im, &owner)? {
        owner.clone()
    } else {
        im.workspace_owner()?
            .map(|p| p.to_string())
            .ok_or(Refusal("goals need a workspace owner"))?
    };
    if member(im, &conversation, &sender).is_err() || !human(im, &sender)? {
        return Err(Refusal(
            "A goal review posts as the workspace owner, who is not in this conversation. Set the goal in your conversation with the person who asked for the work.",
        )
        .into());
    }
    let open: i64 = store.connection.query_row(
        "SELECT COUNT(*) FROM goals WHERE owner=?1 AND state IN ('active','waiting_human')",
        [&owner],
        |r| r.get(0),
    )?;
    if open >= MAX_OPEN {
        return Err(
            Refusal("at most 5 open goals per identity; complete or abandon one first").into(),
        );
    }
    let now = now_ms();
    let goal = Goal {
        id: uuid::Uuid::new_v4().to_string(),
        owner,
        conversation,
        text: text(&input.text)?,
        done_when: text(&input.done_when)?,
        state: GoalState::Active,
        sender,
        created_at: now,
        updated_at: now,
        reviews: 0,
        stalled_reviews: 0,
        last_review_at: None,
        progress_mark: None,
        last_review_message: None,
        outcome: None,
        waiting_since: None,
        waiting_rechecks: 0,
    };
    save(store, &goal)?;
    Ok(view(&goal))
}

/// The goal named in `args`, if the actor owns it or is a person in its conversation.
fn visible(store: &ScheduleStore, im: &ImStore, actor: &Actor, args: &Value) -> Result<Goal> {
    let id = args["goal_id"]
        .as_str()
        .ok_or(Refusal("goal_id required"))?;
    let goal = get(store, id)?.ok_or(Refusal("goal not found; use goal_list"))?;
    let principal = actor.principal.as_str();
    let allowed = goal.owner == principal
        || (human(im, principal)? && member(im, &goal.conversation, principal).is_ok());
    if !allowed {
        return Err(Refusal("goal not found; use goal_list").into());
    }
    Ok(goal)
}

fn list(store: &ScheduleStore, im: &ImStore, actor: &Actor, args: &Value) -> Result<Value> {
    let finished = args["include_finished"].as_bool().unwrap_or(false);
    let principal = actor.principal.as_str();
    let person = human(im, principal)?;
    let goals = all(store)?
        .into_iter()
        .filter(|g| finished || matches!(g.state, GoalState::Active | GoalState::WaitingHuman))
        .filter(|g| {
            g.owner == principal || (person && member(im, &g.conversation, principal).is_ok())
        })
        .map(|g| view(&g))
        .collect::<Vec<_>>();
    Ok(json!({ "goals": goals }))
}

fn finish(
    store: &ScheduleStore,
    im: &ImStore,
    actor: &Actor,
    args: &Value,
    state: GoalState,
    field: &str,
) -> Result<Value> {
    let mut goal = visible(store, im, actor, args)?;
    if !matches!(goal.state, GoalState::Active | GoalState::WaitingHuman) {
        return Err(Refusal("this goal is already done or abandoned").into());
    }
    let detail = text(args[field].as_str().unwrap_or(""))?;
    let now = now_ms();
    goal.state = state;
    goal.outcome = Some(detail);
    goal.updated_at = now;
    goal.waiting_since = (state == GoalState::WaitingHuman).then_some(now);
    if state == GoalState::WaitingHuman {
        goal.stalled_reviews = 0;
        // The question must reach the person, whatever the owner's own reply says.
        let body = format!(
            "[AGT goal waiting {}]\nGoal: {}\nWaiting for a person: {}",
            goal.id,
            goal.text,
            goal.outcome.as_deref().unwrap_or_default()
        );
        notify_person(im, &goal, body, &format!("waiting:{now}"))?;
    }
    save(store, &goal)?;
    Ok(view(&goal))
}

fn state_name(state: GoalState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn view(goal: &Goal) -> Value {
    json!({
        "goal_id": goal.id,
        "owner": goal.owner,
        "conversation": goal.conversation,
        "text": goal.text,
        "done_when": goal.done_when,
        "state": goal.state,
        "reviews": goal.reviews,
        "stalled_reviews": goal.stalled_reviews,
        "outcome": goal.outcome,
    })
}

fn save(store: &ScheduleStore, goal: &Goal) -> Result<()> {
    store.connection.execute(
        "INSERT INTO goals(id,owner,conversation,state,record) VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(id) DO UPDATE SET state=excluded.state,record=excluded.record",
        params![
            goal.id,
            goal.owner,
            goal.conversation,
            state_name(goal.state),
            serde_json::to_string(goal)?
        ],
    )?;
    Ok(())
}

fn get(store: &ScheduleStore, id: &str) -> Result<Option<Goal>> {
    store
        .connection
        .query_row("SELECT record FROM goals WHERE id=?1", [id], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .map(|record| Ok(serde_json::from_str(&record)?))
        .transpose()
}

fn all(store: &ScheduleStore) -> Result<Vec<Goal>> {
    let mut statement = store
        .connection
        .prepare("SELECT record FROM goals ORDER BY rowid DESC LIMIT 200")?;
    let rows = statement
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.iter()
        .map(|record| Ok(serde_json::from_str(record)?))
        .collect()
}

fn open_goals(store: &ScheduleStore) -> Result<Vec<Goal>> {
    let mut statement = store.connection.prepare(
        "SELECT record FROM goals WHERE state IN ('active','waiting_human') ORDER BY rowid",
    )?;
    let rows = statement
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.iter()
        .map(|record| Ok(serde_json::from_str(record)?))
        .collect()
}

/// Something is running, ready to run, waiting for a person's approval, or will resume
/// by itself soon; the workspace is not stalled.
fn busy(store: &ScheduleStore, now: i64) -> Result<bool> {
    let running: bool = store.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM tasks WHERE state IN ('evaluating','responding'))
             OR EXISTS(SELECT 1 FROM approvals WHERE json_extract(value,'$.state')='awaiting_human')
             OR EXISTS(SELECT 1 FROM timers WHERE state='active' AND next_at<=?1)
             OR EXISTS(SELECT 1 FROM work_waits WHERE outcome IS NULL AND deadline<=?1)",
        [now + RESUMES_SOON_MS],
        |r| r.get(0),
    )?;
    Ok(running || !store.candidates()?.is_empty())
}

/// Ask the owners of open goals to review them when the workspace has been quiet for
/// `scheduler.goal_idle_seconds`. Each quiet spell gets one review; a review after which
/// only the owner acted counts as stalled, and after `scheduler.goal_stalled_reviews` of
/// those in a row the goal waits for a person instead.
pub(crate) fn review_due(home: &Path, store: &ScheduleStore) -> Result<()> {
    let goals = open_goals(store)?;
    if goals.is_empty() {
        return Ok(());
    }
    let config = agt_config::RuntimeConfig::load(home)?;
    let im = open_im(home)?;
    let now = now_ms();
    for mut goal in goals
        .iter()
        .filter(|g| g.state == GoalState::WaitingHuman)
        .cloned()
    {
        if person_spoke_since(&im, &goal)? {
            goal.state = GoalState::Active;
            goal.waiting_since = None;
            goal.stalled_reviews = 0;
            goal.waiting_rechecks = 0;
            goal.updated_at = now;
            save(store, &goal)?;
        }
    }
    let Some(idle) = config.scheduler.goal_idle_seconds else {
        return Ok(());
    };
    if busy(store, now)? {
        return Ok(());
    }
    let quiet_since = im.latest_activity(None)?.unwrap_or(0);
    if now - quiet_since < i64::from(idle) * 1000 {
        return Ok(());
    }
    for mut goal in open_goals(store)?
        .into_iter()
        .filter(|g| g.state == GoalState::Active)
    {
        // One review per quiet spell: nothing happened since the last one.
        if goal.last_review_at.is_some_and(|at| at >= quiet_since) {
            continue;
        }
        let progress = im.latest_activity(Some(&PrincipalId::new(&goal.owner)))?;
        if goal.last_review_at.is_some() {
            if progress > goal.progress_mark {
                goal.stalled_reviews = 0;
            } else {
                goal.stalled_reviews += 1;
            }
        }
        let limit = config.scheduler.goal_stalled_reviews;
        if limit.is_some_and(|n| goal.stalled_reviews >= n) || goal.reviews >= MAX_REVIEWS {
            pause(store, &im, &mut goal, now)?;
            continue;
        }
        review(
            home,
            store,
            &im,
            &mut goal,
            now,
            now - quiet_since,
            progress,
            None,
        )?;
    }
    // A goal waiting for a person is looked at again after a while: the question may
    // have gone unseen, or what blocked it may have cleared without anyone answering.
    for mut goal in open_goals(store)?
        .into_iter()
        .filter(|g| g.state == GoalState::WaitingHuman)
    {
        let since = goal.waiting_since.unwrap_or(goal.updated_at);
        let after = (RECHECK_MS << goal.waiting_rechecks.min(4)).min(MAX_RECHECK_MS);
        if goal.reviews >= MAX_REVIEWS || now - since < after {
            continue;
        }
        let waited = (now - since, goal.outcome.clone().unwrap_or_default());
        goal.state = GoalState::Active;
        goal.waiting_since = None;
        goal.stalled_reviews = 0;
        goal.waiting_rechecks += 1;
        let progress = im.latest_activity(Some(&PrincipalId::new(&goal.owner)))?;
        review(
            home,
            store,
            &im,
            &mut goal,
            now,
            now - quiet_since,
            progress,
            Some(waited),
        )?;
    }
    Ok(())
}

fn person_spoke_since(im: &ImStore, goal: &Goal) -> Result<bool> {
    let since = goal.waiting_since.unwrap_or(goal.updated_at);
    let page = im.list_messages(
        &ConversationId::new(&goal.conversation),
        &PrincipalId::new(&goal.sender),
        MessageListOptions {
            limit: 20,
            before: None,
            after: None,
        },
    )?;
    for message in page.messages.iter().filter(|m| m.created_at > since) {
        if message.metadata.get("goal_review").is_none()
            && message.metadata.get("timer").is_none()
            && human(im, message.sender_id.as_str())?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `waited`: how long the goal had waited for a person, and for what.
#[allow(clippy::too_many_arguments)]
fn review(
    home: &Path,
    store: &ScheduleStore,
    im: &ImStore,
    goal: &mut Goal,
    now: i64,
    quiet_ms: i64,
    progress: Option<i64>,
    waited: Option<(i64, String)>,
) -> Result<()> {
    if member(im, &goal.conversation, &goal.sender).is_err()
        || member(im, &goal.conversation, &goal.owner).is_err()
    {
        goal.state = GoalState::Abandoned;
        goal.outcome = Some("the conversation, its owner or its person is gone".into());
        goal.updated_at = now;
        return save(store, goal);
    }
    let number = goal.reviews + 1;
    let sent = im.send_message(SendMessageRequest {
        conversation_id: ConversationId::new(&goal.conversation),
        sender_id: PrincipalId::new(&goal.sender),
        body: review_body(home, im, goal, quiet_ms, number, waited)?,
        client_message_id: Some(format!("goal:{}:review:{number}", goal.id)),
        reply_to_id: None,
        metadata: json!({
            "mentions": [goal.owner],
            "goal_review": {"goal_id": goal.id, "review": number},
        }),
        idempotency_key: None,
    })?;
    // Like a timer, the review starts a discussion as the goal's person.
    store.authorize_human(
        sent.message.id.as_str(),
        &goal.conversation,
        sent.message.seq,
        now,
    )?;
    goal.reviews = number;
    goal.last_review_at = Some(now);
    goal.progress_mark = progress;
    goal.last_review_message = Some(sent.message.id.to_string());
    goal.updated_at = now;
    save(store, goal)
}

fn pause(store: &ScheduleStore, im: &ImStore, goal: &mut Goal, now: i64) -> Result<()> {
    let body = format!(
        "[AGT goal paused {}]\nGoal: {}\nAutomatic reviews stopped after {} reviews, the last {} of them without anyone else acting. A person needs to look at it; any message here from a person resumes the reviews.",
        goal.id, goal.text, goal.reviews, goal.stalled_reviews
    );
    notify_person(im, goal, body, &format!("paused:{}", goal.reviews))?;
    goal.state = GoalState::WaitingHuman;
    goal.waiting_since = Some(now);
    goal.outcome = Some("automatic reviews paused: no progress".into());
    goal.updated_at = now;
    save(store, goal)
}

/// The owner tells the goal's person, by mention, that the goal needs them. Posting it
/// does not run anyone: reviews pause until a person speaks or a recheck is due.
fn notify_person(im: &ImStore, goal: &Goal, body: String, key: &str) -> Result<()> {
    im.send_message(SendMessageRequest {
        conversation_id: ConversationId::new(&goal.conversation),
        sender_id: PrincipalId::new(&goal.owner),
        body,
        client_message_id: Some(format!("goal:{}:{key}", goal.id)),
        reply_to_id: None,
        metadata: json!({
            "mentions": [goal.sender],
            "goal_review": {"goal_id": goal.id, "paused": true},
        }),
        idempotency_key: None,
    })?;
    Ok(())
}

fn review_body(
    home: &Path,
    im: &ImStore,
    goal: &Goal,
    quiet_ms: i64,
    number: u32,
    waited: Option<(i64, String)>,
) -> Result<String> {
    let mut body = format!(
        "[AGT goal review {}]\nGoal: {}\nDone when: {}\n\nNothing has happened in this workspace for {} minutes and this goal is still open (review {number}). Decide now, with tools rather than words: if the goal is met, call goal_complete with the evidence. If work is stuck, unblock it: mention or message whoever should act next, retry or reassign tasks, or fix what blocks them. If it can only continue after a person answers, call goal_wait_human and ask them.",
        goal.id,
        goal.text,
        goal.done_when,
        quiet_ms.max(60_000) / 60_000
    );
    if let Some((waited_ms, reason)) = waited {
        body.push_str(&format!(
            "\n\nThis goal has waited {} minutes for a person and nobody has answered: {reason}\nCheck whether that is still true. If what blocked it has cleared, or you can clear it yourself, continue the work. If only a person can unblock it, call goal_wait_human again with a clear question.",
            waited_ms / 60_000
        ));
    }
    let owner = goal.owner.as_str();
    let work = crate::work::WorkLedger::open(home)?.pending_execution()?;
    let mine = work
        .iter()
        .filter(|w| w.request.requester == owner || w.request.assignee == owner)
        .take(10)
        .map(|w| {
            format!(
                "- {} for {}: {}",
                &w.id[..8.min(w.id.len())],
                w.request.assignee,
                state_label(&w.state)
            )
        })
        .collect::<Vec<_>>();
    if !mine.is_empty() {
        body.push_str(&format!("\n\nUnfinished tasks:\n{}", mine.join("\n")));
    }
    let failed = crate::RunStore::open(home)?
        .list(30)?
        .into_iter()
        .filter(|r| {
            matches!(
                r.state,
                crate::RunState::Failed
                    | crate::RunState::OutcomeUnknown
                    | crate::RunState::Interrupted
            )
        })
        .take(5)
        .map(|r| {
            let error: String = r.error.unwrap_or_default().chars().take(120).collect();
            format!("- {} {:?}: {error}", r.request.agent, r.state)
        })
        .collect::<Vec<_>>();
    if !failed.is_empty() {
        body.push_str(&format!("\n\nRecent failed runs:\n{}", failed.join("\n")));
    }
    let latest = latest_messages(im, owner)?;
    if !latest.is_empty() {
        body.push_str(&format!(
            "\n\nLatest message in your conversations:\n{}",
            latest.join("\n")
        ));
    }
    Ok(body)
}

fn state_label(state: &crate::work::WorkState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|v| {
            v.get("state")
                .or(Some(&v))
                .and_then(|s| s.as_str().map(str::to_owned))
        })
        .unwrap_or_else(|| format!("{state:?}"))
}

/// The last message of each conversation the owner is in, most recent first.
fn latest_messages(im: &ImStore, owner: &str) -> Result<Vec<String>> {
    let principal = PrincipalId::new(owner);
    let mut lines = Vec::new();
    for summary in im.list_conversations(&principal)? {
        if summary.left_message_seq.is_some() || summary.dissolved {
            continue;
        }
        let conversation = summary.conversation;
        let page = im.list_messages(
            &conversation.id,
            &principal,
            MessageListOptions {
                limit: 1,
                before: None,
                after: None,
            },
        )?;
        if let Some(message) = page.messages.last()
            && message.metadata.get("goal_review").is_none()
        {
            let text: String = message.body.chars().take(120).collect();
            let title = conversation
                .title
                .clone()
                .unwrap_or_else(|| conversation.id.to_string());
            lines.push((
                message.created_at,
                format!(
                    "- {title} · {}: {}",
                    message.sender_id,
                    text.replace('\n', " ")
                ),
            ));
        }
    }
    lines.sort_by_key(|line| std::cmp::Reverse(line.0));
    Ok(lines.into_iter().take(8).map(|(_, line)| line).collect())
}
