use super::*;
fn response(arguments: Value) -> Value {
    json!({"role":"assistant","tool_calls":[{"id":"decision","type":"function","function":{"name":"participation_decision","arguments":arguments.to_string()}}]})
}
#[test]
fn action_contract_accepts_all_variants_and_rejects_mixed_or_incomplete_arguments() {
    for arguments in [
        json!({"action":"participate","reason_code":"direct_question"}),
        json!({"action":"silent","reason_code":"already_answered","public_reason":null}),
        json!({"action":"wait","reason_code":"observe_discussion","wake":{"type":"after","seconds":300}}),
        json!({"action":"wait","reason_code":"observe_discussion","wake":{"type":"conversation_quiet","quiet_seconds":30,"max_wait_seconds":300}}),
    ] {
        assert!(
            parse_decision(&response(arguments.clone())).is_ok(),
            "{arguments}"
        );
    }
    for arguments in [
        json!({"action":"silent","reason_code":"already_answered","wake":null}),
        json!({"action":"participate","reason_code":"direct_question","wake":{"type":"after","seconds":1}}),
        json!({"action":"wait","reason_code":"observe_discussion"}),
        json!({"action":"wait","reason_code":"observe_discussion","wake":{"type":"after"}}),
        json!({"action":"wait","reason_code":"observe_discussion","wake":{"type":"after","seconds":1,"quiet_seconds":1}}),
        json!({"action":"wait","reason_code":"observe_discussion","wake":{"type":"conversation_quiet","quiet_seconds":1}}),
        json!({"action":"silent","reason_code":"unknown"}),
        json!({"action":"silent","reason_code":"not_relevant","extra":true}),
    ] {
        assert!(
            parse_decision(&response(arguments.clone())).is_err(),
            "{arguments}"
        );
    }
}
#[test]
fn duration_and_unicode_reason_limits_are_enforced() {
    for seconds in [0, 301] {
        let result = parse_decision(&response(
            json!({"action":"wait","reason_code":"observe_discussion","wake":{"type":"after","seconds":seconds}}),
        ));
        assert!(result.unwrap_err().to_string().contains("wake duration"));
    }
    assert!(parse_decision(&response(json!({"action":"wait","reason_code":"observe_discussion","wake":{"type":"conversation_quiet","quiet_seconds":10,"max_wait_seconds":9}}))).is_err());
    for (count, valid) in [(240, true), (241, false)] {
        assert_eq!(parse_decision(&response(json!({"action":"silent","reason_code":"not_relevant","public_reason":"中".repeat(count)}))).is_ok(),valid);
    }
}
#[test]
fn advertised_schema_discriminates_actions_and_wake_fields() {
    let tool = decision_tool();
    let schema = &tool["function"]["parameters"];
    assert_eq!(tool["_agt_required"], true);
    assert_eq!(schema["oneOf"][0]["not"]["required"], json!(["wake"]));
    assert_eq!(schema["oneOf"][1]["required"], json!(["wake"]));
    let variants = &schema["properties"]["wake"]["oneOf"];
    assert_eq!(variants[0]["required"], json!(["type", "seconds"]));
    assert_eq!(
        variants[1]["required"],
        json!(["type", "quiet_seconds", "max_wait_seconds"])
    );
    assert_eq!(variants[0]["properties"]["seconds"]["minimum"], 1);
    assert_eq!(variants[0]["properties"]["seconds"]["maximum"], 300);
    assert_eq!(variants[1]["properties"]["quiet_seconds"]["maximum"], 30);
    assert_eq!(
        schema["properties"]["public_reason"]["type"],
        json!(["string", "null"])
    );
}
#[test]
fn malformed_envelopes_have_distinct_diagnostics_without_echoing_output() {
    let error = parse_decision(&json!({"role":"assistant","content":"private text"}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("missing tool call"));
    assert!(!error.contains("private text"));
    let mut message = response(json!({"action":"silent","reason_code":"not_relevant"}));
    message["tool_calls"][0]["function"]["name"] = json!("framework_resource_list");
    assert!(
        parse_decision(&message)
            .unwrap_err()
            .to_string()
            .contains("expected exactly one")
    );
}
