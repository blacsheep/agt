use serde_json::{Value, json};
pub(crate) fn schemas() -> Vec<Value> {
    let string = json!({"type":"string"});
    [
        ("harness_models","Read actual models and supported reasoning levels from a Harness connection. No inference, no UI. Use resource_write(kind=harness) for defaults; existing sessions remain unchanged.",json!({"reference":string}),vec!["reference"]),
        ("harness_session_get","Read or prepare the current session for target Agent in conversation. Both caller and target must be current members. Returns AGT record id, engine-generated session_id, selected model and execution state. Does not start a turn.",json!({"conversation":string,"target":string}),vec!["conversation","target"]),
        ("harness_session_models","Read real model/effort choices using the current session connection snapshot. Both caller and target must be current members. Does not open a Web dialog.",json!({"conversation":string,"target":string,"session":string}),vec!["conversation","target","session"]),
        ("harness_session_configure","Select model and optional effort for this Agent/chat only, using session id returned by harness_session_get. Validate against harness_session_models. Active/recovering/needs_review sessions reject changes; wait or report the conflict, never automatically cancel/reset. Does not change Harness defaults or other chats.",json!({"conversation":string,"target":string,"session":string,"model":string,"effort":string}),vec!["conversation","target","session","model"]),
    ].into_iter().map(|(name,description,properties,required)|json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})).collect()
}
