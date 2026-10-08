use super::*;

#[test]
fn cancellation_waits_for_matching_terminal_not_rpc_ack() {
    let (dir, config) = fixture();
    std::fs::write(dir.path().join("wrong-turn-interrupt"), "1").unwrap();
    let mut terminal = Vec::new();
    let mut event = |event: Event<'_>| {
        match event {
            Event::Turn(_) => anyhow::bail!("user canceled"),
            Event::Settled(status) => terminal.push(status.to_owned()),
            _ => {}
        }
        Ok(())
    };
    let mut client = Codex::connect(&config, &mut event).unwrap();
    assert!(
        client
            .execute(
                &config,
                Execution {
                    input: "CANCELABLE",
                    instructions: "",
                    thread: None,
                    tools: &[]
                },
                &mut event,
                &mut |_, _| unreachable!()
            )
            .is_err()
    );
    assert_eq!(terminal, ["interrupted"]);
}

#[test]
fn completed_result_wins_race_against_cancellation() {
    let (_dir, config) = fixture();
    let mut terminal = None;
    let mut event = |event: Event<'_>| {
        match event {
            Event::Turn(_) => anyhow::bail!("user canceled"),
            Event::Settled(status) => terminal = Some(status.to_owned()),
            _ => {}
        }
        Ok(())
    };
    let mut client = Codex::connect(&config, &mut event).unwrap();
    let result = client
        .execute(
            &config,
            Execution {
                input: "hello",
                instructions: "",
                thread: None,
                tools: &[],
            },
            &mut event,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
    assert_eq!(result, "fixture final answer");
    assert_eq!(terminal.as_deref(), Some("completed"));
}

#[test]
fn cancellation_before_start_ack_preserves_protocol_and_confirms_interrupt() {
    for stop_at_protocol in [false, true] {
        let (dir, config) = fixture();
        let mut submitted = false;
        let mut stopped = false;
        let mut terminal = Vec::new();
        let mut event = |event: Event<'_>| {
            match event {
                Event::Submitted => submitted = true,
                Event::Tick if submitted && !stopped && !stop_at_protocol => {
                    stopped = true;
                    anyhow::bail!("cancel before reading start acknowledgement");
                }
                Event::Protocol if submitted && !stopped && stop_at_protocol => {
                    stopped = true;
                    anyhow::bail!("cancel after reading start acknowledgement");
                }
                Event::Settled(status) => terminal.push(status.to_owned()),
                _ => {}
            }
            Ok(())
        };
        let mut client = Codex::connect(&config, &mut event).unwrap();
        assert!(
            client
                .execute(
                    &config,
                    Execution {
                        input: "CANCELABLE",
                        instructions: "",
                        thread: None,
                        tools: &[],
                    },
                    &mut event,
                    &mut |_, _| panic!("must not execute tools after cancellation")
                )
                .is_err()
        );
        assert_eq!(terminal, ["interrupted"]);
        let protocol = std::fs::read_to_string(dir.path().join("protocol.jsonl")).unwrap();
        assert_eq!(
            protocol
                .lines()
                .filter(|line| line.contains("turn/start"))
                .count(),
            1
        );
        assert_eq!(
            protocol
                .lines()
                .filter(|line| line.contains("turn/interrupt"))
                .count(),
            1
        );
    }
}

#[test]
fn stopping_notification_failure_does_not_skip_interrupt_confirmation() {
    let (dir, config) = fixture();
    let mut terminal = Vec::new();
    let mut event = |event: Event<'_>| {
        match event {
            Event::Turn(_) => anyhow::bail!("original cancellation"),
            Event::Stopping(_) => anyhow::bail!("cannot save stop notification"),
            Event::Settled(status) => terminal.push(status.to_owned()),
            _ => {}
        }
        Ok(())
    };
    let mut client = Codex::connect(&config, &mut event).unwrap();
    let error = client
        .execute(
            &config,
            Execution {
                input: "CANCELABLE",
                instructions: "",
                thread: None,
                tools: &[],
            },
            &mut event,
            &mut |_, _| panic!("must not execute tools during cleanup"),
        )
        .unwrap_err();
    assert_eq!(terminal, ["interrupted"]);
    assert!(error.to_string().contains("original cancellation"));
    let protocol = std::fs::read_to_string(dir.path().join("protocol.jsonl")).unwrap();
    assert_eq!(
        protocol
            .lines()
            .filter(|line| line.contains("turn/interrupt"))
            .count(),
        1
    );
}

#[test]
fn compaction_stop_notification_failure_still_confirms_interrupt() {
    let (dir, config) = fixture();
    let mut submitted = false;
    let mut frames = 0;
    let mut terminal = Vec::new();
    let error = Codex::compact(&config, "cancelable-compact", &mut |event| {
        match event {
            Event::Submitted => submitted = true,
            Event::Protocol if submitted => frames += 1,
            Event::Tick if frames >= 2 => anyhow::bail!("original compaction cancellation"),
            Event::Stopping(_) => anyhow::bail!("cannot save stop notification"),
            Event::Settled(status) => terminal.push(status.to_owned()),
            _ => {}
        }
        Ok(())
    })
    .unwrap_err();
    assert_eq!(terminal, ["interrupted"]);
    assert!(
        error
            .to_string()
            .contains("original compaction cancellation")
    );
    let protocol = std::fs::read_to_string(dir.path().join("protocol.jsonl")).unwrap();
    assert_eq!(
        protocol
            .lines()
            .filter(|line| line.contains("turn/interrupt"))
            .count(),
        1
    );
    assert!(!protocol.contains("turn/start"));
}
