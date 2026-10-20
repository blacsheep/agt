use super::*;
use crate::work::TaskControl;
use agt_im::MessageListOptions;
use agt_service::WorkControl;

/// The lead `a` sets a goal in the group while answering the user's question.
fn goal_fixture() -> (tempfile::TempDir, Scheduler, String, String) {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let lead = TaskControl::hosted(
        home.path(),
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: None,
        },
        task,
    );
    let goal = lead
        .call(
            "goal_create",
            json!({"text":"Ship the report","done_when":"report.md exists and the user approved it"}),
        )
        .unwrap();
    assert_eq!(goal["state"], "active");
    let id = goal["goal_id"].as_str().unwrap().to_owned();
    settle_all(&scheduler);
    (home, scheduler, chat, id)
}

/// Every task has finished: nothing runs or waits to run.
fn settle_all(scheduler: &Scheduler) {
    scheduler
        .store
        .connection
        .execute(
            "UPDATE tasks SET state='completed',record=json_set(record,'$.state','completed')",
            [],
        )
        .unwrap();
}

/// Let `ms` pass: move every message and goal timestamp back by the same amount.
fn rewind(scheduler: &Scheduler, ms: i64) {
    rusqlite::Connection::open(scheduler.home.join("im/im.db"))
        .unwrap()
        .execute("UPDATE messages SET created_at=created_at-?1", [ms])
        .unwrap();
    let connection = &scheduler.store.connection;
    for field in [
        "last_review_at",
        "progress_mark",
        "updated_at",
        "waiting_since",
    ] {
        connection
            .execute(
                &format!("UPDATE goals SET record=json_set(record,'$.{field}',json_extract(record,'$.{field}')-?1) WHERE json_extract(record,'$.{field}') IS NOT NULL"),
                [ms],
            )
            .unwrap();
    }
}

fn goal(scheduler: &Scheduler, id: &str) -> Value {
    let record: String = scheduler
        .store
        .connection
        .query_row("SELECT record FROM goals WHERE id=?1", [id], |r| r.get(0))
        .unwrap();
    serde_json::from_str(&record).unwrap()
}

fn reviews(home: &Path, chat: &str) -> Vec<agt_im::Message> {
    all_messages(home, chat)
        .into_iter()
        .filter(|m| m.metadata.get("goal_review").is_some())
        .collect()
}

fn all_messages(home: &Path, chat: &str) -> Vec<agt_im::Message> {
    im(home)
        .unwrap()
        .list_messages(
            &ConversationId::new(chat),
            &PrincipalId::new("user"),
            MessageListOptions {
                limit: 100,
                before: None,
                after: None,
            },
        )
        .unwrap()
        .messages
}

fn post(home: &Path, chat: &str, principal: &str, body: &str, key: &str) {
    ToolService::open(
        home,
        Actor {
            principal: PrincipalId::new(principal),
            role: Role::Participant,
            automated_depth: None,
        },
    )
    .unwrap()
    .call(
        "message_send",
        json!({"conversation":chat,"body":body,"request_key":key}),
    )
    .unwrap();
}

#[test]
fn a_quiet_workspace_asks_the_goal_owner_once_per_quiet_spell() {
    let (home, mut scheduler, chat, id) = goal_fixture();
    // Not quiet long enough yet.
    crate::goals::review_due(home.path(), &scheduler.store).unwrap();
    assert!(reviews(home.path(), &chat).is_empty());
    rewind(&scheduler, 200_000);
    crate::goals::review_due(home.path(), &scheduler.store).unwrap();
    let found = reviews(home.path(), &chat);
    assert_eq!(found.len(), 1);
    let review = &found[0];
    // Like a timer, it speaks as the person and wakes the owner.
    assert_eq!(review.sender_id.as_str(), "user");
    assert_eq!(review.metadata["mentions"], json!(["a"]));
    assert!(review.body.starts_with(&format!("[AGT goal review {id}]")));
    assert!(review.body.contains("report.md exists"));
    assert_eq!(goal(&scheduler, &id)["reviews"], 1);
    scheduler.scan().unwrap();
    assert!(
        scheduler
            .store
            .candidates()
            .unwrap()
            .iter()
            .any(|t| t.principal == "a" && t.message == review.id.as_str())
    );
    // Nothing happened since: no second review, even after more quiet time.
    settle_all(&scheduler);
    rewind(&scheduler, 200_000);
    crate::goals::review_due(home.path(), &scheduler.store).unwrap();
    assert_eq!(reviews(home.path(), &chat).len(), 1);
}

#[test]
fn reviews_where_only_the_owner_acts_pause_the_goal_until_a_person_speaks() {
    let (home, scheduler, chat, id) = goal_fixture();
    rewind(&scheduler, 200_000);
    crate::goals::review_due(home.path(), &scheduler.store).unwrap();
    // Three times the owner answers but nobody else does anything.
    for round in 0..3 {
        post(
            home.path(),
            &chat,
            "a",
            "Still waiting.",
            &format!("a-{round}"),
        );
        settle_all(&scheduler);
        rewind(&scheduler, 200_000);
        crate::goals::review_due(home.path(), &scheduler.store).unwrap();
    }
    let state = goal(&scheduler, &id);
    assert_eq!(state["state"], "waiting_human", "{state}");
    assert_eq!(state["reviews"], 3);
    let last = reviews(home.path(), &chat).pop().unwrap();
    assert!(last.body.starts_with(&format!("[AGT goal paused {id}]")));
    assert_eq!(last.sender_id.as_str(), "a");
    // A person writing in the conversation resumes the reviews.
    post(home.path(), &chat, "user", "Any news?", "user-1");
    crate::goals::review_due(home.path(), &scheduler.store).unwrap();
    assert_eq!(goal(&scheduler, &id)["state"], "active");
}

#[test]
fn a_goal_waiting_for_a_person_asks_them_and_is_looked_at_again_later() {
    let (home, scheduler, chat, id) = goal_fixture();
    let person = TaskControl::local(
        home.path(),
        Actor {
            principal: PrincipalId::new("user"),
            role: Role::Manager,
            automated_depth: None,
        },
    );
    person
        .call(
            "goal_wait_human",
            json!({"goal_id":id,"question":"Which logo should the report use?"}),
        )
        .unwrap();
    // The question reaches the person even if the owner's reply leaves it out.
    let notice = all_messages(home.path(), &chat).pop().unwrap();
    assert!(notice.body.starts_with(&format!("[AGT goal waiting {id}]")));
    assert!(notice.body.contains("Which logo"));
    assert_eq!(notice.metadata["mentions"], json!(["user"]));
    // A quiet spell alone leaves it waiting...
    rewind(&scheduler, 200_000);
    crate::goals::review_due(home.path(), &scheduler.store).unwrap();
    assert_eq!(goal(&scheduler, &id)["state"], "waiting_human");
    // ...but after half an hour without an answer the owner looks at it again.
    rewind(&scheduler, 30 * 60_000);
    crate::goals::review_due(home.path(), &scheduler.store).unwrap();
    let state = goal(&scheduler, &id);
    assert_eq!(state["state"], "active", "{state}");
    assert_eq!(state["waiting_rechecks"], 1);
    let review = reviews(home.path(), &chat).pop().unwrap();
    assert!(review.body.starts_with(&format!("[AGT goal review {id}]")));
    assert!(review.body.contains("waited"));
}

#[test]
fn busy_workspaces_and_finished_goals_are_not_reviewed() {
    let (home, mut scheduler, chat, id) = goal_fixture();
    // A task ready to run means work will continue by itself.
    post(home.path(), &chat, "user", "please check", "user-b");
    let latest = all_messages(home.path(), &chat).pop().unwrap();
    start_discussion(home.path(), &chat, Some(latest.id.as_str())).unwrap();
    scheduler.scan().unwrap();
    assert!(!scheduler.store.candidates().unwrap().is_empty());
    rewind(&scheduler, 200_000);
    crate::goals::review_due(home.path(), &scheduler.store).unwrap();
    assert!(reviews(home.path(), &chat).is_empty());
    // Once done, a goal is never reviewed again.
    let user = TaskControl::local(
        home.path(),
        Actor {
            principal: PrincipalId::new("user"),
            role: Role::Manager,
            automated_depth: None,
        },
    );
    let done = user
        .call(
            "goal_complete",
            json!({"goal_id":id,"evidence":"report.md approved"}),
        )
        .unwrap();
    assert_eq!(done["state"], "done");
    settle_all(&scheduler);
    rewind(&scheduler, 200_000);
    crate::goals::review_due(home.path(), &scheduler.store).unwrap();
    assert!(reviews(home.path(), &chat).is_empty());
}

#[test]
fn a_refused_goal_tool_tells_the_agent_why() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let lead = TaskControl::hosted(
        home.path(),
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: None,
        },
        task,
    );
    let error = lead
        .call(
            "goal_create",
            json!({"conversation":"elsewhere","text":"Ship it","done_when":"shipped"}),
        )
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<agt_service::Refusal>().map(|r| r.0),
        Some("agents can only set goals in their current conversation")
    );
}

#[test]
fn only_the_owner_and_people_in_the_conversation_see_a_goal() {
    let (home, _scheduler, _chat, id) = goal_fixture();
    let as_principal = |principal: &str| {
        TaskControl::local(
            home.path(),
            Actor {
                principal: PrincipalId::new(principal),
                role: Role::Manager,
                automated_depth: None,
            },
        )
    };
    assert!(
        as_principal("b")
            .call("goal_get", json!({"goal_id":id}))
            .is_err()
    );
    let listed = as_principal("user").call("goal_list", json!({})).unwrap();
    assert_eq!(listed["goals"][0]["goal_id"], json!(id));
}
