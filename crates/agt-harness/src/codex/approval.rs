use super::*;
use crate::{ApprovalHandler, ApprovalRequest};
pub(super) struct PendingApproval {
    pub request: ApprovalRequest,
    pub host_id: String,
    pub sent: bool,
}
pub(super) fn parse(value: &Value) -> Result<ApprovalRequest> {
    let method = value["method"]
        .as_str()
        .context("missing approval method")?;
    let p = &value["params"];
    if !(value["id"].is_string() || value["id"].is_i64())
        || ["threadId", "turnId", "itemId"]
            .iter()
            .any(|k| p[k].as_str().is_none_or(str::is_empty))
    {
        bail!("invalid approval identity")
    }
    let choices = match method {
        "item/commandExecution/requestApproval" => {
            let offered = p["availableDecisions"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![json!("accept"), json!("decline"), json!("cancel")]);
            offered
                .into_iter()
                .filter(|v| {
                    matches!(
                        v.as_str(),
                        Some("accept" | "acceptForSession" | "decline" | "cancel")
                    )
                })
                .collect::<Vec<_>>()
        }
        "item/fileChange/requestApproval" => vec![
            json!("accept"),
            json!("acceptForSession"),
            json!("decline"),
            json!("cancel"),
        ],
        "item/permissions/requestApproval" => {
            if !p["permissions"].is_object() {
                bail!("missing requested permissions")
            }
            vec![
                json!({"permissions":p["permissions"],"scope":"turn"}),
                json!({"permissions":{},"scope":"turn"}),
                json!("cancel"),
            ]
        }
        _ => bail!("unsupported approval method"),
    };
    if choices.is_empty() {
        bail!("no supported approval choices")
    }
    let options = choices
        .into_iter()
        .map(|value| {
            let label = match value.as_str() {
                Some("accept") => "批准本次",
                Some("acceptForSession") => "按引擎规则批准本会话",
                Some("decline") => "拒绝本次",
                Some("cancel") => "取消执行",
                _ if value["permissions"] == json!({}) => "不授予权限",
                _ => "授予所请求权限（本回合）",
            };
            json!({"value":value,"label":label})
        })
        .collect();
    Ok(ApprovalRequest {
        rpc_id: value["id"].clone(),
        method: method.into(),
        params: p.clone(),
        options,
        item_context: None,
    })
}
impl Codex {
    pub(super) fn flush_approvals(
        &mut self,
        pending: &mut [PendingApproval],
        handler: &mut dyn ApprovalHandler,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<()> {
        for entry in pending.iter_mut().filter(|p| !p.sent) {
            let Some(decision) = handler.poll(&entry.host_id)? else {
                continue;
            };
            event(Event::Tick)?;
            if !entry.request.options.iter().any(|o| o["value"] == decision) {
                bail!("approval decision not offered")
            }
            if entry.request.method == "item/permissions/requestApproval" && decision == "cancel" {
                return Err(
                    Failure::new(FailureCode::Interrupted, "permission request canceled").into(),
                );
            }
            let result = if entry.request.method == "item/permissions/requestApproval" {
                decision
            } else {
                json!({"decision":decision})
            };
            handler.delivery(&entry.host_id, "sending")?;
            self.transport
                .send(json!({"id":entry.request.rpc_id,"result":result}))?;
            entry.sent = true;
            handler.delivery(&entry.host_id, "sent")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn choices_never_expand_native_offered_authority() {
        let mut value = json!({"id":42,"method":"item/commandExecution/requestApproval","params":{"threadId":"t","turnId":"r","itemId":"i","availableDecisions":["decline",{"acceptWithExecpolicyAmendment":{}}]}});
        let request = parse(&value).unwrap();
        assert_eq!(request.rpc_id, json!(42));
        assert_eq!(request.options.len(), 1);
        assert_eq!(request.options[0]["value"], "decline");
        value["params"]["availableDecisions"] = json!([]);
        assert!(parse(&value).is_err());
        value["params"]["availableDecisions"] = json!(["accept"]);
        value["params"]["turnId"] = Value::Null;
        assert!(parse(&value).is_err());
    }
}
