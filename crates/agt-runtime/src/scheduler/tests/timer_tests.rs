use super::*;
use crate::work::TaskControl;
use agt_im::MessageListOptions;
use agt_service::WorkControl;

fn control(home: &Path, principal: &str) -> TaskControl {
    TaskControl::local(
        home,
        Actor {
            principal: PrincipalId::new(principal),
            role: Role::Manager,
            automated_depth: None,
        },
    )
}
/// Move a timer's next firing into the past, as if its time had come.
fn make_due(home: &Path, id: &str, at: i64) {
    let store = ScheduleStore::open(home).unwrap();
    let record: String = store
        .connection
        .query_row("SELECT record FROM timers WHERE id=?1", [id], |r| r.get(0))
        .unwrap();
    let mut record: Value = serde_json::from_str(&record).unwrap();
    record["next_at"] = json!(at);
    store
        .connection
        .execute(
            "UPDATE timers SET next_at=?2,record=?3 WHERE id=?1",
            rusqlite::params![id, at, record.to_string()],
        )
        .unwrap();
}
fn messages(home: &Path, chat: &str) -> Vec<agt_im::Message> {
    im(home)
        .unwrap()
        .list_messages(
            &ConversationId::new(chat),
            &PrincipalId::new("user"),
            MessageListOptions {
                limit: 50,
                before: None,
                after: None,
            },
        )
        .unwrap()
        .messages
}
fn list(control: &TaskControl, finished: bool) -> Vec<Value> {
    control
        .call("timer_list", json!({"include_finished":finished}))
        .unwrap()["timers"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn a_timer_fires_once_as_its_humans_message_and_starts_a_discussion() {
    let (home, _user, chat) = fixture();
    let user = control(home.path(), "user");
    let timer = user
        .call(
            "timer_create",
            json!({"conversation":chat,"text":"Time for the stand-up","mentions":["a"],"after_seconds":600}),
        )
        .unwrap();
    assert_eq!(timer["state"], "active");
    assert_eq!(timer["sender"], "user");
    assert!((598..=600).contains(&timer["next_in_seconds"].as_i64().unwrap()));
    assert!(timer["next_at"].as_str().unwrap().len() == 25, "{timer}");
    let mut scheduler = Scheduler::open(home.path(), 3).unwrap();
    // Nothing runs before its time, but the scheduler wakes for it.
    crate::timers::fire_due(home.path(), &scheduler.store).unwrap();
    assert!(
        messages(home.path(), &chat)
            .iter()
            .all(|m| m.metadata["timer"].is_null())
    );
    assert!(scheduler.next_wake_delay().unwrap() <= Duration::from_secs(30));
    let id = timer["id"].as_str().unwrap();
    make_due(home.path(), id, now_ms() - 1_000);
    assert!(scheduler.next_wake_delay().unwrap() <= Duration::from_secs(30));

    scheduler.tick(false).unwrap();
    scheduler.tick(false).unwrap();
    let fired: Vec<_> = messages(home.path(), &chat)
        .into_iter()
        .filter(|m| m.metadata["timer"]["id"] == id)
        .collect();
    assert_eq!(fired.len(), 1, "a firing posts exactly once");
    let message = &fired[0];
    assert_eq!(message.sender_id.as_str(), "user");
    assert!(message.body.starts_with(&format!("[AGT timer {id}]")));
    assert!(message.body.ends_with("Time for the stand-up"));
    assert_eq!(message.metadata["mentions"], json!(["a"]));
    // It authorized a discussion, so the mentioned member must answer it.
    let discussion = scheduler
        .store
        .human_discussion(message.id.as_str())
        .unwrap()
        .expect("the timer message is authorized");
    assert_eq!(
        scheduler
            .store
            .discussion(&discussion)
            .unwrap()
            .unwrap()
            .state,
        DiscussionState::Active
    );
    let task = scheduler
        .store
        .tasks()
        .unwrap()
        .into_iter()
        .find(|t| t.principal == "a")
        .expect("the mentioned member has a task");
    assert_eq!(task.message, message.id.as_str());
    assert!(!task.decision_required);

    assert!(list(&user, false).is_empty());
    let done = list(&user, true);
    assert_eq!(done[0]["state"], "done");
    assert_eq!(done[0]["fired"], 1);
    assert!(done[0]["next_at"].is_null());
}

#[test]
fn an_agent_timer_speaks_as_the_owner_and_recurs_in_its_zone() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let agent = TaskControl::hosted(
        home.path(),
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: None,
        },
        task,
    );
    let monthly = json!({"text":"Collect this month's data","mentions":["a","b"],"cron":"0 9 1 * *","timezone":"Asia/Shanghai","name":"monthly data"});
    let timer = agent.call("timer_create", monthly.clone()).unwrap();
    assert_eq!(timer["creator"], "a");
    assert_eq!(timer["sender"], "user");
    assert_eq!(timer["timezone"], "Asia/Shanghai");
    let next = timer["next_at"].as_str().unwrap();
    assert!(next.ends_with("-01T09:00:00+08:00"), "{next}");
    assert!(
        timer["schedule"].as_str().unwrap().contains("09:00"),
        "{timer}"
    );

    let other = user
        .call("group_create", json!({"title":"other","members":["a"]}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut elsewhere = monthly.clone();
    elsewhere["conversation"] = json!(other);
    assert!(agent.call("timer_create", elsewhere).is_err());
    let mut frequent = monthly.clone();
    frequent["cron"] = json!("*/5 * * * *");
    let error = agent
        .call("timer_create", frequent)
        .unwrap_err()
        .to_string();
    assert!(error.contains("10 minutes"), "{error}");

    let id = timer["id"].as_str().unwrap();
    make_due(home.path(), id, now_ms() - 1_000);
    crate::timers::fire_due(home.path(), &scheduler.store).unwrap();
    let message = messages(home.path(), &chat)
        .into_iter()
        .find(|m| m.metadata["timer"]["id"] == id)
        .unwrap();
    assert_eq!(message.sender_id.as_str(), "user");
    assert_eq!(message.metadata["timer"]["creator"], "a");
    assert!(
        message
            .body
            .starts_with(&format!("[AGT timer {id} · monthly data]"))
    );
    let after = list(&control(home.path(), "user"), false);
    assert_eq!(after[0]["state"], "active");
    assert_eq!(after[0]["fired"], 1);
    assert!(
        after[0]["next_at"]
            .as_str()
            .unwrap()
            .ends_with("-01T09:00:00+08:00")
    );
}

#[test]
fn agents_cancel_only_their_own_timers_and_people_cancel_any() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let agent = TaskControl::hosted(
        home.path(),
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: None,
        },
        task,
    );
    let person = control(home.path(), "user");
    let theirs = person
        .call(
            "timer_create",
            json!({"conversation":chat,"text":"x","after_seconds":60}),
        )
        .unwrap()["id"]
        .clone();
    let mine = agent
        .call("timer_create", json!({"text":"y","after_seconds":60}))
        .unwrap()["id"]
        .clone();
    assert!(
        agent
            .call("timer_cancel", json!({"timer_id":theirs}))
            .is_err()
    );
    let canceled = agent
        .call(
            "timer_cancel",
            json!({"timer_id":mine,"reason":"not needed"}),
        )
        .unwrap();
    assert_eq!(canceled["state"], "canceled");
    assert_eq!(canceled["reason"], "not needed");
    assert_eq!(
        person
            .call("timer_cancel", json!({"timer_id":theirs}))
            .unwrap()["state"],
        "canceled"
    );
    assert!(list(&person, false).is_empty());
}

#[test]
fn missed_occurrences_collapse_into_one_late_delivery() {
    let (home, _user, chat) = fixture();
    let user = control(home.path(), "user");
    let id = user
        .call(
            "timer_create",
            json!({"conversation":chat,"text":"hourly check","cron":"0 * * * *"}),
        )
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    make_due(home.path(), &id, now_ms() - 3 * 3_600_000);
    let store = ScheduleStore::open(home.path()).unwrap();
    crate::timers::fire_due(home.path(), &store).unwrap();
    crate::timers::fire_due(home.path(), &store).unwrap();
    let fired: Vec<_> = messages(home.path(), &chat)
        .into_iter()
        .filter(|m| m.metadata["timer"]["id"] == id.as_str())
        .collect();
    assert_eq!(fired.len(), 1);
    assert!(
        fired[0].body.contains("delivered late"),
        "{}",
        fired[0].body
    );
    assert!(list(&user, false)[0]["next_in_seconds"].as_i64().unwrap() > 0);
}

#[test]
fn invalid_timers_are_refused_and_timers_end_with_their_conversation() {
    let (home, _user, chat) = fixture();
    let user = control(home.path(), "user");
    for (args, needle) in [
        (json!({"conversation":chat,"text":"x"}), "exactly one"),
        (
            json!({"conversation":chat,"text":"x","after_seconds":60,"cron":"0 9 * * *"}),
            "exactly one",
        ),
        (
            json!({"conversation":chat,"text":"x","at":"2020-01-01T09:00"}),
            "between 10 seconds",
        ),
        (
            json!({"conversation":chat,"text":"x","after_seconds":60,"mentions":["ghost"]}),
            "",
        ),
        (
            json!({"conversation":chat,"text":"x","cron":"61 * * * *"}),
            "cron",
        ),
        (
            json!({"conversation":chat,"text":"x","after_seconds":60,"timezone":"Mars/Base"}),
            "time zone",
        ),
        (json!({"text":"x","after_seconds":60}), "conversation"),
    ] {
        let error = user
            .call("timer_create", args.clone())
            .unwrap_err()
            .to_string();
        assert!(error.contains(needle), "{args}: {error}");
    }
    assert!(
        control(home.path(), "a")
            .call(
                "timer_create",
                json!({"conversation":chat,"text":"x","after_seconds":60})
            )
            .is_err(),
        "local Agent processes cannot set timers"
    );

    let id = user
        .call(
            "timer_create",
            json!({"conversation":chat,"text":"x","at":"2099-01-01T09:00"}),
        )
        .unwrap_err();
    assert!(id.to_string().contains("366 days"));
    let id = user
        .call(
            "timer_create",
            json!({"conversation":chat,"text":"x","after_seconds":60}),
        )
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    dissolve_scheduled_group(home.path(), &chat, "user").unwrap();
    make_due(home.path(), &id, now_ms() - 1_000);
    let store = ScheduleStore::open(home.path()).unwrap();
    crate::timers::fire_due(home.path(), &store).unwrap();
    let ended = store
        .connection
        .query_row("SELECT state FROM timers WHERE id=?1", [&id], |r| {
            r.get::<_, String>(0)
        })
        .unwrap();
    assert_eq!(ended, "canceled");
}
