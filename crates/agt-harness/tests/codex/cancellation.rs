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
