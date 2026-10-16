use super::*;
fn response(arguments: Value) -> Value {
    json!({"role":"assistant","tool_calls":[{"id":"decision","type":"function","function":{"name":"participation_decision","arguments":arguments.to_string()}}]})
}
#[test]
fn action_contract_accepts_all_variants_and_rejects_mixed_or_incomplete_arguments() {
    for arguments in [
        json!({"action":"participate","reason_code":"direct_question"}),
        json!({"action":"silent","reason_code":"already_answered","public_reason":null}),
        json!({"action":"wait","reason_code":"observe_discussion"}),
    ] {
        assert!(
            parse_decision(&response(arguments.clone()), true).is_ok(),
            "{arguments}"
        );
    }
    for arguments in [
        json!({"action":"silent","reason_code":"already_answered","wake":null}),
        json!({"action":"participate","reason_code":"direct_question","wake":{"type":"after","seconds":1}}),
        json!({"action":"wait","reason_code":"observe_discussion","wake":{"type":"after","seconds":30}}),
        json!({"action":"silent","reason_code":"unknown"}),
        json!({"action":"silent","reason_code":"not_relevant","extra":true}),
    ] {
        assert!(
            parse_decision(&response(arguments.clone()), true).is_err(),
            "{arguments}"
        );
    }
}
#[test]
fn wait_is_refused_when_nobody_is_working() {
    let error = parse_decision(
        &response(json!({"action":"wait","reason_code":"observe_discussion"})),
        false,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("nobody is working"));
    assert!(
        parse_decision(
            &response(json!({"action":"silent","reason_code":"not_relevant"})),
            false
        )
        .is_ok()
    );
}
#[test]
fn stored_timed_waits_from_earlier_versions_still_load() {
    let stored = json!({"action":"wait","reason_code":"observe_discussion","wake":{"type":"conversation_quiet","quiet_seconds":3,"max_wait_seconds":20},"decided_at":1,"through_seq":2});
    let decision: crate::schedule_store::RecordedDecision = serde_json::from_value(stored).unwrap();
    assert!(matches!(
        decision.decision,
        ParticipationDecision::Wait { .. }
    ));
    assert!(
        serde_json::to_value(&decision)
            .unwrap()
            .get("wake")
            .is_none()
    );
}
#[test]
fn unicode_reason_limit_is_enforced() {
    for (count, valid) in [(240, true), (241, false)] {
        assert_eq!(parse_decision(&response(json!({"action":"silent","reason_code":"not_relevant","public_reason":"中".repeat(count)})), true).is_ok(),valid);
    }
}
#[test]
fn advertised_schema_offers_wait_only_when_someone_is_working() {
    for (wait, actions) in [
        (true, json!(["participate", "silent", "wait"])),
        (false, json!(["participate", "silent"])),
    ] {
        let tool = decision_tool(wait);
        let schema = &tool["function"]["parameters"];
        assert_eq!(tool["_agt_required"], true);
        assert_eq!(schema["properties"]["action"]["enum"], actions);
        assert!(schema["properties"].get("wake").is_none());
        assert_eq!(
            schema["properties"]["public_reason"]["type"],
            json!(["string", "null"])
        );
    }
}
#[test]
fn malformed_envelopes_have_distinct_diagnostics_without_echoing_output() {
    let error = parse_decision(&json!({"role":"assistant","content":"private text"}), true)
        .unwrap_err()
        .to_string();
    assert!(error.contains("missing tool call"));
    assert!(!error.contains("private text"));
    let mut message = response(json!({"action":"silent","reason_code":"not_relevant"}));
    message["tool_calls"][0]["function"]["name"] = json!("framework_resource_list");
    assert!(
        parse_decision(&message, true)
            .unwrap_err()
            .to_string()
            .contains("expected exactly one")
    );
}
