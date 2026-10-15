use super::*;

#[test]
fn context_errors_in_both_stream_protocols_are_not_generic_retryable_errors() {
    for (responses, event) in [
        (
            false,
            json!({"error":{"code":"context_length_exceeded","message":"private"}}),
        ),
        (
            true,
            json!({"type":"response.failed","response":{"error":{"code":"context_length_exceeded","message":"private"}}}),
        ),
        (
            true,
            json!({"type":"error","code":"context_length_exceeded","message":"private"}),
        ),
    ] {
        let frame = format!("data: {event}\n\n");
        assert!(matches!(
            read(
                frame.as_bytes(),
                responses,
                std::time::Duration::from_secs(60),
                &mut |_| Ok(())
            ),
            Err(RuntimeError::ContextWindowRejected)
        ));
    }
}
