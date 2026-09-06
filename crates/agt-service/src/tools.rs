use crate::{Actor, Binding, BindingStore, MessageInput, Messaging, Resources, Role};
use agt_im::{ConversationId, PrincipalId, PrincipalInput, PrincipalKind};
use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Application operations shared by CLI, stdio MCP and hosted agent tools.
pub struct ToolService {
    home: PathBuf,
    messaging: Messaging,
}
impl ToolService {
    pub fn open(home: &Path, actor: Actor) -> Result<Self> {
        Ok(Self {
            home: home.to_owned(),
            messaging: Messaging::open(home, actor)?,
        })
    }
    pub fn schemas(&self) -> Vec<Value> {
        schemas(self.messaging.actor.role)
    }
    pub fn call(&self, name: &str, args: Value) -> Result<Value> {
        if !self.schemas().iter().any(|s| s["name"] == name) {
            bail!("tool unavailable for this role");
        }
        let m = &self.messaging;
        let resources = Resources::new(&self.home);
        Ok(match name {
            "resource_list" => {
                let d: Kind = serde_json::from_value(args)?;
                resources.list(&d.kind)?
            }
            "resource_read" => {
                let d: Reference = serde_json::from_value(args)?;
                resources.read(&d.kind, &d.reference)?
            }
            "resource_write" => {
                m.actor.require_manager()?;
                let d: Write = serde_json::from_value(args)?;
                resources.write(&d.kind, &d.reference, d.create, d.data)?
            }
            "resource_delete" => {
                m.actor.require_manager()?;
                let d: Delete = serde_json::from_value(args)?;
                resources.delete(&d.kind, &d.reference, d.recursive, d.confirm)?
            }
            "principal_create" => {
                m.actor.require_manager()?;
                let d: Principal = serde_json::from_value(args)?;
                serde_json::to_value(m.im.create_principal(PrincipalInput {
                    id: PrincipalId::new(d.id),
                    kind: d.kind,
                    display_name: d.name,
                    metadata: json!({}),
                })?)?
            }
            "principal_list" => {
                let _: Empty = serde_json::from_value(args)?;
                json!(
                    m.im.list_principals()?
                        .into_iter()
                        .map(|p| json!({"id":p.id,"name":p.display_name,"kind":p.kind}))
                        .collect::<Vec<_>>()
                )
            }
            "agent_bind" => {
                m.actor.require_manager()?;
                let binding: Binding = serde_json::from_value(args)?;
                serde_json::to_value(BindingStore::open(&self.home)?.bind(binding)?)?
            }
            "agent_unbind" => {
                m.actor.require_manager()?;
                let d: Target = serde_json::from_value(args)?;
                BindingStore::open(&self.home)?.unbind(&d.principal)?;
                json!({"unbound":d.principal})
            }
            "binding_list" => {
                m.actor.require_manager()?;
                let _: Empty = serde_json::from_value(args)?;
                serde_json::to_value(BindingStore::open(&self.home)?.list()?)?
            }
            "direct_create" => {
                let d: Target = serde_json::from_value(args)?;
                serde_json::to_value(m.im.create_direct(&m.actor.principal, &d.principal)?)?
            }
            "group_create" => {
                let d: Group = serde_json::from_value(args)?;
                serde_json::to_value(m.im.create_group(
                    &m.actor.principal,
                    Some(d.title),
                    d.members,
                )?)?
            }
            "conversation_list" => {
                let _: Empty = serde_json::from_value(args)?;
                serde_json::to_value(m.im.list_conversations(&m.actor.principal)?)?
            }
            "member_list" => {
                let d: Conversation = serde_json::from_value(args)?;
                m.member(&d.conversation)?;
                serde_json::to_value(m.im.list_members(&d.conversation)?)?
            }
            "member_add" | "member_remove" => {
                let d: Member = serde_json::from_value(args)?;
                m.member(&d.conversation)?;
                if name == "member_add" {
                    serde_json::to_value(m.im.add_member(
                        &d.conversation,
                        &m.actor.principal,
                        &d.principal,
                    )?)?
                } else {
                    serde_json::to_value(m.im.remove_member(
                        &d.conversation,
                        &m.actor.principal,
                        &d.principal,
                    )?)?
                }
            }
            "response_policy" => {
                let d: Policy = serde_json::from_value(args)?;
                m.member(&d.conversation)?;
                BindingStore::open(&self.home)?.set_policy(
                    &d.conversation,
                    &m.actor.principal,
                    d.policy,
                )?;
                json!({"updated":true})
            }
            "message_send" => {
                serde_json::to_value(m.send(serde_json::from_value::<MessageInput>(args)?)?)?
            }
            "message_history" => {
                let d: History = serde_json::from_value(args)?;
                serde_json::to_value(m.history(&d.conversation, d.before, d.limit)?)?
            }
            "message_inbox" => {
                let d: Inbox = serde_json::from_value(args)?;
                let mut items = Vec::new();
                for delivery in
                    m.im.deliveries(&m.actor.principal, d.pending_only, d.limit)?
                {
                    let message = m.im.get_message(&delivery.message_id)?;
                    m.member(&message.conversation_id)?;
                    items.push(json!({"id":delivery.id,"message_id":delivery.message_id,"conversation":message.conversation_id,"state":delivery.state,"run_id":delivery.run_id,"error":delivery.error}));
                }
                json!(items)
            }
            "message_ack" => {
                let d: Ack = serde_json::from_value(args)?;
                m.im.ack_delivery(d.id, &m.actor.principal)?;
                json!({"acknowledged":d.id})
            }
            "message_status" => {
                let d: MessageIdInput = serde_json::from_value(args)?;
                let message = m.im.get_message(&d.message_id)?;
                m.member(&message.conversation_id)?;
                json!(m.im.delivery_status(&d.message_id)?.into_iter().map(|d|json!({"principal":d.principal_id,"state":d.state,"run_id":d.run_id,"error":d.error})).collect::<Vec<_>>())
            }
            _ => bail!("unknown tool"),
        })
    }
}

fn schema(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
}
fn schemas(role: Role) -> Vec<Value> {
    let string = json!({"type":"string"});
    let mut tools = vec![
        schema(
            "message_status",
            "Inspect per-recipient processing status for a message in your conversation. Does not expose private model input or transcripts.",
            json!({"message_id":string}),
            &["message_id"],
        ),
        schema(
            "principal_list",
            "List available participant identities.",
            json!({}),
            &[],
        ),
        schema(
            "direct_create",
            "Create or find a private conversation between yourself and another participant.",
            json!({"principal":string}),
            &["principal"],
        ),
        schema(
            "group_create",
            "Create a group owned by yourself. Messages broadcast to current members.",
            json!({"title":string,"members":{"type":"array","items":string}}),
            &["title", "members"],
        ),
        schema(
            "conversation_list",
            "List your conversations.",
            json!({}),
            &[],
        ),
        schema(
            "member_list",
            "List members of a conversation you belong to.",
            json!({"conversation":string}),
            &["conversation"],
        ),
        schema(
            "member_add",
            "Invite a participant. Requires group owner/admin membership.",
            json!({"conversation":string,"principal":string}),
            &["conversation", "principal"],
        ),
        schema(
            "member_remove",
            "Remove a participant or leave a group. IM role checks apply.",
            json!({"conversation":string,"principal":string}),
            &["conversation", "principal"],
        ),
        schema(
            "response_policy",
            "Set your own response policy in a conversation: all, mentioned, manual. Direct chats respond unless manual.",
            json!({"conversation":string,"policy":{"enum":["all","mentioned","manual"]}}),
            &["conversation", "policy"],
        ),
        schema(
            "message_send",
            "Send as your fixed identity. Reuse request_key only for identical content. Use mentions to request group responses; sending does not synchronously wait for agents.",
            json!({"conversation":string,"body":string,"request_key":string,"mentions":{"type":"array","items":string},"reply_to":string}),
            &["conversation", "body", "request_key"],
        ),
        schema(
            "message_history",
            "Read shared messages oldest-to-newest within a page. Pass next_cursor as before for older history.",
            json!({"conversation":string,"before":string,"limit":{"type":"integer","minimum":1,"maximum":200,"default":50}}),
            &["conversation"],
        ),
        schema(
            "message_inbox",
            "List your deliveries and processing status. Read messages through message_history. External clients acknowledge using message_ack.",
            json!({"pending_only":{"type":"boolean","default":true},"limit":{"type":"integer","minimum":1,"maximum":200,"default":50}}),
            &[],
        ),
        schema(
            "message_ack",
            "Acknowledge your pending delivery. Cannot acknowledge a hosted execution already processing.",
            json!({"id":{"type":"integer","minimum":1}}),
            &["id"],
        ),
    ];
    if role == Role::Manager {
        let kind = json!({"enum":["provider","model","prompt","skill","mcp","agent"]});
        tools.extend([
            schema("resource_list","List resource summaries. Secrets are redacted.",json!({"kind":kind}),&["kind"]),
            schema("resource_read","Read a resource by category/name (default category optional). Skills support @scope/package. Secrets are redacted.",json!({"kind":kind,"reference":string}),&["kind","reference"]),
            schema("resource_write","Create or update resource. data: provider create {base_url,api_key_env}, update either field; model create {provider,model,params?: ModelConfigUpdate}, update ModelConfigUpdate fields directly; prompt {text}; skill create {description}, update {text: full SKILL.md}; mcp single-server {mcpServers:{name:config}}; agent {model,prompt?,skills?,mcps?,description?}, model required on create. Agent arrays replace mounts (empty unmounts all); omitted fields unchanged. Categories must exist. Provider env credentials resolve in the server process, never returned.",json!({"kind":kind,"reference":string,"create":{"type":"boolean"},"data":{"type":"object"}}),&["kind","reference","create","data"]),
            schema("resource_delete","First omit confirm to get dependency-aware deletion plan; then pass exact ordered items as confirm. Protects default prompt/gateway.",json!({"kind":kind,"reference":string,"recursive":{"type":"boolean","default":false},"confirm":{"type":"array","items":string}}),&["kind","reference"]),
            schema("principal_create","Register external user/service or agent identity. Does not start inference.",json!({"id":string,"name":string,"kind":{"enum":["user","agent","bot","service","other"]}}),&["id","name","kind"]),
            schema("agent_bind","Host an Agent principal using an agent definition. Binding affects future unprepared deliveries. allow_mcp also requires worker --allow-mcp. max_depth bounds automated reply chains; default policy is mentioned.",json!({"principal":string,"agent":string,"working_directory":string,"policy":{"enum":["all","mentioned","manual"]},"max_steps":{"type":"integer","minimum":1,"maximum":100},"max_depth":{"type":"integer","minimum":1,"maximum":8},"allow_mcp":{"type":"boolean","default":false}}),&["principal","agent","working_directory","max_steps","max_depth"]),
            schema("agent_unbind","Stop hosting a participant; existing records remain.",json!({"principal":string}),&["principal"]),
            schema("binding_list","List hosted agent bindings.",json!({}),&[]),
        ]);
    }
    tools
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Kind {
    kind: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    kind: String,
    reference: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Write {
    kind: String,
    reference: String,
    create: bool,
    data: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delete {
    kind: String,
    reference: String,
    #[serde(default)]
    recursive: bool,
    confirm: Option<Vec<String>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Principal {
    id: String,
    name: String,
    kind: PrincipalKind,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    principal: PrincipalId,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    title: String,
    members: Vec<PrincipalId>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Conversation {
    conversation: ConversationId,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    conversation: ConversationId,
    principal: PrincipalId,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    conversation: ConversationId,
    policy: crate::ResponsePolicy,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    conversation: ConversationId,
    before: Option<String>,
    #[serde(default = "limit")]
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inbox {
    #[serde(default = "yes")]
    pending_only: bool,
    #[serde(default = "limit_u32")]
    limit: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ack {
    id: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageIdInput {
    message_id: agt_im::MessageId,
}
fn limit() -> usize {
    50
}
fn limit_u32() -> u32 {
    50
}
fn yes() -> bool {
    true
}
