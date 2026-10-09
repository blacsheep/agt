use super::*;
use agt_harness::{ApprovalHandler, ApprovalRequest};
use serde_json::Value;
#[derive(Default)]
struct Human {
    requests: Vec<ApprovalRequest>,
    deliveries: Vec<agt_harness::DeliveryProgress>,
    choice: Option<Value>,
    polls: usize,
}
impl ApprovalHandler for Human {
    fn request(&mut self, r: &ApprovalRequest) -> anyhow::Result<String> {
        self.requests.push(r.clone());
        Ok("host-approval".into())
    }
    fn poll(&mut self, _: &str) -> anyhow::Result<Option<Value>> {
        self.polls += 1;
        Ok(if self.polls > 1 {
            self.choice.clone()
        } else {
            None
        })
    }
    fn delivery(&mut self, _: &str, progress: agt_harness::DeliveryProgress) -> anyhow::Result<()> {
        self.deliveries.push(progress);
        Ok(())
    }
}
#[test]
fn native_approval_responses_preserve_identity_and_original_turn() {
    for (input, choice, response) in [
        (
            "RELAY_COMMAND_DUP",
            json!("accept"),
            json!({"decision":"accept"}),
        ),
        (
            "RELAY_COMMAND",
            json!("decline"),
            json!({"decision":"decline"}),
        ),
        (
            "RELAY_FILE",
            json!("acceptForSession"),
            json!({"decision":"acceptForSession"}),
        ),
        (
            "RELAY_PERMISSIONS",
            json!({"permissions":{},"scope":"turn"}),
            json!({"permissions":{},"scope":"turn"}),
        ),
    ] {
        let (dir, config) = fixture();
        let mut human = Human {
            choice: Some(choice),
            ..Default::default()
        };
        let mut client = Codex::connect(&config, &mut |_| Ok(())).unwrap();
        let output = client
            .execute_with_approvals(
                &config,
                Execution {
                    input,
                    instructions: "",
                    thread: None,
                    tools: &[],
                },
                &mut |_| Ok(()),
                &mut |_, _| unreachable!(),
                Some(&mut human),
            )
            .unwrap();
        assert_eq!(output, "approval returned");
        assert_eq!(human.requests.len(), 1);
        assert_eq!(human.deliveries, {
            use agt_harness::DeliveryProgress::*;
            vec![Sending, Sent, Resolved]
        });
        drop(client);
        let log: Vec<Value> = std::fs::read_to_string(dir.path().join("protocol.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            log.iter().filter(|v| v["method"] == "turn/start").count(),
            1
        );
        assert_eq!(
            log.iter().find(|v| v["method"] == "thread/start").unwrap()["params"]["approvalPolicy"],
            "on-request"
        );
        let replies: Vec<_> = log
            .iter()
            .filter(|v| v["id"] == human.requests[0].rpc_id)
            .collect();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["result"], response);
    }
}
#[test]
fn pending_approval_does_not_block_terminal_or_cancellation() {
    for input in [
        "RELAY_COMMAND_TERMINAL",
        "RELAY_COMMAND",
        "RELAY_PERMISSIONS",
    ] {
        let (dir, config) = fixture();
        let mut human = Human {
            choice: if input == "RELAY_PERMISSIONS" {
                Some(json!("cancel"))
            } else {
                None
            },
            ..Default::default()
        };
        let started = std::time::Instant::now();
        let mut event = |e: Event<'_>| {
            if matches!(e, Event::Tick) && started.elapsed().as_secs_f32() > 0.3 {
                anyhow::bail!("canceled")
            }
            Ok(())
        };
        let mut client = Codex::connect(&config, &mut event).unwrap();
        let result = client.execute_with_approvals(
            &config,
            Execution {
                input,
                instructions: "",
                thread: None,
                tools: &[],
            },
            &mut event,
            &mut |_, _| unreachable!(),
            Some(&mut human),
        );
        assert_eq!(result.is_ok(), input.ends_with("TERMINAL"));
        assert!(human.deliveries.is_empty());
        drop(client);
        let log = std::fs::read_to_string(dir.path().join("protocol.jsonl")).unwrap();
        assert_eq!(log.contains("turn/interrupt"), !input.ends_with("TERMINAL"));
    }
}
