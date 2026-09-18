use crate::{Actor, Binding, BindingStore, MessageInput, Messaging, Resources, Role, ToolError};
use agt_im::{ConversationId, PrincipalId, PrincipalInput, PrincipalKind};
use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Application operations shared by CLI, stdio MCP and hosted agent tools.
pub struct ToolService {
    home: PathBuf,
    messaging: Messaging,
    resource_management: bool,
    harness: Option<Box<dyn crate::HarnessControl>>,
    work: Option<Box<dyn crate::WorkControl>>,
}
impl ToolService {
    pub fn open(home: &Path, actor: Actor) -> Result<Self> {
        Ok(Self {
            home: home.to_owned(),
            messaging: Messaging::open(home, actor)?,
            resource_management: false,
            harness: None,
            work: None,
        })
    }
    pub fn schemas(&self) -> Vec<Value> {
        let mut result = schemas(self.messaging.actor.role, self.resource_management);
        if self.harness.is_some()
            && (self.resource_management || self.messaging.actor.role == Role::Manager)
        {
            result.extend(super::harness_tools::schemas());
        }
        if self.work.is_some() {
            let string = json!({"type":"string"});
            result.extend([
                schema("task_request", "Reliably enqueue an asynchronous task for another bound Agent. conversation must include both you and assignee; create a direct chat/group first if needed. Returns queued, not completion. Disabled/manual assignees remain queued. Do not also send the same request with message_send. The assignee's result or failure is delivered back here to you; report it to the original user. Hosted calls inherit the current discussion and permissions. Trusted human CLI calls must provide their already-authorized source_message. Priority defaults to normal; Agents can request low/normal/high, urgent requires human Manager. Use task_await only when it is listed by this execution host.", json!({"assignee":string,"conversation":string,"input":string,"request_key":string,"source_message":string,"priority":{"enum":["low","normal","high","urgent"]}}), &["assignee","conversation","input","request_key"]),
                schema("task_reprioritize", "Change a queued task's priority with expected_revision from task_get. Assignee may choose low/normal/high; requesting human Manager controls urgent. Does not preempt admitted/running work. Changes inherit the current discussion authorization.", json!({"task_id":string,"expected_revision":{"type":"integer"},"priority":{"enum":["low","normal","high","urgent"]},"reason":string}), &["task_id","expected_revision","priority","reason"]),
                schema("task_defer", "Defer a queued task only. Choose exactly one: not_before (future Unix milliseconds), or after_task plus deadline (future Unix milliseconds). Barrier target must have the same assignee and discussion; cycles are rejected. Success/failure/cancellation releases the barrier, unknown outcome does not. Expired unresolved barriers stay blocked for review. Does not suspend any running task or freeze the requester. Query task_get for the shared waiting condition and revision.", json!({"task_id":string,"expected_revision":{"type":"integer"},"reason":string,"not_before":{"type":"integer"},"after_task":string,"deadline":{"type":"integer"}}), &["task_id","expected_revision","reason"]),
                schema("task_cancel", "Cancel a task and its exclusively owned descendants. Queued/suspended work stops immediately; running work receives a cancellation request, and may become outcome_unknown if external effects cannot be confirmed. Terminal results are preserved. Requires current expected_revision and reason; hosted requester/assignee or human Manager in the shared conversation only.", json!({"task_id":string,"expected_revision":{"type":"integer"},"reason":string}), &["task_id","expected_revision","reason"]),
                schema("task_resolve", "Human Manager only: record evidence after inspecting an outcome_unknown execution. outcome is succeeded or failed. This does not execute tools or overwrite the original unknown record; it publishes the verification to the requester. Requires expected_revision and evidence. Never guess an external outcome.", json!({"task_id":string,"expected_revision":{"type":"integer"},"outcome":{"enum":["succeeded","failed"]},"evidence":string}), &["task_id","expected_revision","outcome","evidence"]),
                schema("task_retry", "Explicitly create a new task from failed/canceled work or an unknown result verified as failed. Preserves the old task and links retry_of. Requires expected_revision, new request_key and reason. Requester or human Manager only; discussion and membership authorization must remain valid. Never automatically retry unknown external side effects.", json!({"task_id":string,"expected_revision":{"type":"integer"},"request_key":string,"reason":string}), &["task_id","expected_revision","request_key","reason"]),
                schema("task_reply", "Answer a pending business input request. Requester or human Manager in the shared conversation only. input_request_id is the waiting Run ID from task_get; include current expected_revision and a unique request_key. Replays with the same answer are idempotent. Does not grant sandbox/MCP/approval permissions.",json!({"task_id":string,"input_request_id":string,"expected_revision":{"type":"integer"},"answer":string,"request_key":string}),&["task_id","input_request_id","expected_revision","answer","request_key"]),
                schema("task_report", "Persist an explicit report to the task's original conversation after reviewing its result. Use this instead of message_send for the final report so delivery survives restart and is tracked separately from reading the result. Hosted requester or human Manager only; fixed original destination and current authorization. Same body is idempotent; conflicting replacement rejected. Does not mark the task successful or approve execution.", json!({"task_id":string,"expected_revision":{"type":"integer"},"body":string}), &["task_id","expected_revision","body"]),
                schema("task_attention", "Workspace owner (human Manager) only: list minimal task incident metadata for unknown outcomes, blocked reports/publications, missing requester or expired dependencies. Does not expose private task input/results or grant membership. Supports limit and before cursor.", json!({"limit":{"type":"integer"},"before":{"type":"integer"}}), &[]),
                schema("task_get", "Read a task you requested or were assigned, including execution and publication state. Task ID alone grants no access.", json!({"task_id":string}), &["task_id"]),
                schema("task_list", "List your requested and assigned tasks, newest first. queued_only=true selects pending work to inspect before reprioritizing/defer. Optional conversation filters before pagination; use the last list_cursor as before for older tasks. Querying does not acknowledge notifications.", json!({"limit":{"type":"integer"},"queued_only":{"type":"boolean"},"conversation":string,"before":{"type":"integer"}}), &[]),
            ]);
        }
        if self.work.as_ref().is_some_and(|w| w.supports_await()) {
            result.push(schema("task_yield","Save this task and release the execution slot until future not_before (Unix milliseconds), allowing other queued work to run. Call alone and end the turn. The original discussion budget and permissions still apply.",json!({"not_before":{"type":"integer"},"reason":{"type":"string"}}),&["not_before","reason"]));
            result.push(schema("task_input","Save this task and ask its requester for business input, resuming on task_reply or deadline (future Unix milliseconds). Call alone and end the turn. This is not an execution approval or sandbox permission request.",json!({"question":{"type":"string"},"deadline":{"type":"integer"}}),&["question","deadline"]));
            result.push(schema("task_await", "Suspend this explicit work task until all 1..20 requested dependencies resolve, or deadline (Unix milliseconds). Call alone, never in a parallel tool batch. Runtime saves completed tool results and releases the execution slot; continuation receives the outcome, including failure, unknown outcome or timeout. Other tasks for this Agent can run while waiting. Does not cancel the dependency on timeout.", json!({"task_id":{"type":"string"},"task_ids":{"type":"array","items":{"type":"string"}},"deadline":{"type":"integer"}}), &["deadline"]));
        }
        result
    }
    pub fn with_work_control(mut self, work: Box<dyn crate::WorkControl>) -> Self {
        self.work = Some(work);
        self
    }
    pub fn with_harness_control(mut self, control: Box<dyn crate::HarnessControl>) -> Self {
        self.harness = Some(control);
        self
    }
    pub fn with_resource_management(mut self, enabled: bool) -> Self {
        self.resource_management = enabled;
        self
    }
    fn require_resource_management(&self) -> Result<()> {
        if self.resource_management {
            Ok(())
        } else {
            self.messaging.actor.require_manager()
        }
    }
    pub fn with_discussion(mut self, discussion: Option<String>) -> Self {
        self.messaging = self.messaging.with_discussion(discussion);
        self
    }
    pub fn call(&self, name: &str, args: Value) -> std::result::Result<Value, ToolError> {
        let schema = self
            .schemas()
            .into_iter()
            .find(|s| s["name"] == name)
            .ok_or_else(|| {
                ToolError::new(
                    "tool_unavailable",
                    "Tool is unknown or unavailable to your fixed role; use tools/list.",
                )
            })?;
        validate_arguments(&schema["inputSchema"], &args)?;
        self.call_inner(name, args).map_err(ToolError::classify)
    }

    fn call_inner(&self, name: &str, args: Value) -> Result<Value> {
        if name.starts_with("harness_") {
            self.require_resource_management()?;
            return self
                .harness
                .as_ref()
                .expect("schema checked")
                .call(name, args);
        }
        if matches!(
            name,
            "task_request"
                | "task_get"
                | "task_list"
                | "task_defer"
                | "task_reprioritize"
                | "task_await"
                | "task_yield"
                | "task_input"
                | "task_reply"
                | "task_report"
                | "task_attention"
                | "task_cancel"
                | "task_resolve"
                | "task_retry"
        ) {
            return self.work.as_ref().expect("schema checked").call(name, args);
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
                self.require_resource_management()?;
                let d: Write = serde_json::from_value(args)?;

                let mut value = resources.write(&d.kind, &d.reference, d.create, d.data)?;
                if d.kind == "agent" {
                    let (category, name) = crate::named_ref(&d.reference)?;
                    let reference = format!("{category}/{name}");
                    let bindings = BindingStore::open(&self.home)?
                        .list_with_state()?
                        .into_iter()
                        .filter(|b| b.binding.agent == reference)
                        .collect::<Vec<_>>();
                    let enabled = bindings.iter().any(|b| b.enabled);
                    let next_steps = if bindings.is_empty() {
                        vec![
                            "Configuration saved only; this Agent is NOT enabled. Inspect principal_list/binding_list, create a kind=agent identity if needed, then agent_bind(principal, agent).",
                            "Verify binding_list contains the intended Agent identity with enabled=true before reporting it ready or assigning work.",
                        ]
                    } else if !enabled {
                        vec![
                            "Running settings exist but all identities are disabled. Call agent_set_enabled(principal, enabled=true) for the intended identity, then verify binding_list.",
                        ]
                    } else {
                        vec![
                            "At least one identity is enabled. Check each intended identity in bindings before assigning work.",
                        ]
                    };
                    value["activation"] = json!({"configured":!bindings.is_empty(),"enabled":enabled,"bindings":bindings,"next_steps":next_steps});
                }
                value
            }
            "resource_delete" => {
                self.require_resource_management()?;
                let d: Delete = serde_json::from_value(args)?;
                resources.delete(&d.kind, &d.reference, d.recursive, d.confirm)?
            }
            "provider_models" => {
                let d: ProviderReference = serde_json::from_value(args)?;
                resources.provider_models(&d.reference)?
            }
            "resource_test" => {
                let d: Reference = serde_json::from_value(args)?;
                resources.test(&d.kind, &d.reference)?
            }
            "resource_category_list" => {
                let d: Kind = serde_json::from_value(args)?;
                serde_json::to_value(resources.categories(&d.kind)?.list()?)?
            }
            "resource_category_create" | "resource_category_delete" => {
                self.require_resource_management()?;
                let d: Category = serde_json::from_value(args)?;
                let category = agt_core::CategoryName::new(d.category)?;
                let categories = resources.categories(&d.kind)?;
                let path = if name == "resource_category_create" {
                    categories.create(&category)?
                } else {
                    categories.delete_empty(&category)?
                };
                json!({"category": category, "path": path})
            }
            "principal_create" => {
                self.require_resource_management()?;
                let d: Principal = serde_json::from_value(args)?;
                if m.actor.role != Role::Manager && d.kind != PrincipalKind::Agent {
                    bail!("resource management can create only Agent identities");
                }
                serde_json::to_value(m.im.create_principal(PrincipalInput {
                    id: PrincipalId::new(d.id),
                    kind: d.kind,
                    display_name: d.name,
                    metadata: json!({}),
                })?)?
            }
            "principal_list" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Directory {
                    #[serde(default)]
                    include_deleted: bool,
                }
                let d: Directory = serde_json::from_value(args)?;
                let principals = if d.include_deleted {
                    m.im.list_principals_including_deleted()?
                } else {
                    m.im.list_principals()?
                };
                json!(
                    principals.into_iter()
                        .map(|p| json!({"id":p.id,"name":p.display_name,"kind":p.kind,"deleted":p.deleted_at.is_some()}))
                        .collect::<Vec<_>>()
                )
            }
            "agent_bind" => {
                self.require_resource_management()?;
                let mut args = args;
                let fields = args.as_object_mut().expect("validated object");
                fields
                    .entry("working_directory")
                    .or_insert(json!(self.home));
                fields.entry("max_steps").or_insert(json!(0));
                let binding: Binding = serde_json::from_value(args)?;
                let store = BindingStore::open(&self.home)?;
                let binding = store.bind(binding)?;
                let status = store
                    .list_with_state()?
                    .into_iter()
                    .find(|b| b.binding.principal == binding.principal)
                    .ok_or_else(|| {
                        anyhow::anyhow!("binding changed concurrently; check binding_list")
                    })?;
                let mut value = serde_json::to_value(&status)?;
                value["next_step"] = json!(if status.enabled {
                    "Enabled. This identity can process authorized messages according to its policy."
                } else {
                    "Still disabled. Call agent_set_enabled(principal, enabled=true) before assigning work."
                });
                value
            }
            "agent_set_enabled" => {
                self.require_resource_management()?;
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Enable {
                    principal: PrincipalId,
                    enabled: bool,
                }
                let d: Enable = serde_json::from_value(args)?;
                BindingStore::open(&self.home)?.set_enabled(&d.principal, d.enabled)?;
                json!({"principal":d.principal,"enabled":d.enabled})
            }
            "agent_unbind" => {
                self.require_resource_management()?;
                let d: Target = serde_json::from_value(args)?;
                BindingStore::open(&self.home)?.unbind(&d.principal)?;
                json!({"unbound":d.principal})
            }
            "binding_list" => {
                self.require_resource_management()?;
                let _: Empty = serde_json::from_value(args)?;
                serde_json::to_value(BindingStore::open(&self.home)?.list_with_state()?)?
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
                let d: ConversationList = serde_json::from_value(args)?;
                serde_json::to_value(m.im.list_conversation_entries(
                    &m.actor.principal,
                    d.include_hidden,
                    d.groups_only,
                )?)?
            }
            "conversation_hide" | "conversation_show" => {
                let d: Conversation = serde_json::from_value(args)?;
                m.im.set_conversation_hidden(
                    &d.conversation,
                    &m.actor.principal,
                    name == "conversation_hide",
                )?;
                json!({"updated":true})
            }
            "group_dissolve" => {
                let d: Conversation = serde_json::from_value(args)?;
                m.im.dissolve_group(&d.conversation, &m.actor.principal)?;
                json!({"dissolved":true})
            }
            "member_list" => {
                let d: Conversation = serde_json::from_value(args)?;
                let own = m.im.get_member(&d.conversation, &m.actor.principal)?;
                serde_json::to_value(if own.left_message_seq.is_some() {
                    vec![own]
                } else {
                    m.im.list_members(&d.conversation)?
                })?
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
                m.im.visible_message(&d.message_id, &m.actor.principal)?;
                json!(m.im.delivery_status(&d.message_id)?.into_iter().map(|d|json!({"principal":d.principal_id,"state":d.state,"run_id":d.run_id,"error":d.error})).collect::<Vec<_>>())
            }
            _ => bail!("unknown tool"),
        })
    }
}

fn schema(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
}
fn schemas(role: Role, resource_management: bool) -> Vec<Value> {
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
            "List available participant identities. Set include_deleted to resolve historical names; deleted identities cannot join new conversations.",
            json!({"include_deleted":{"type":"boolean"}}),
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
            "Create a group. Agent creators must have a configured workspace owner, who is automatically included as group owner; the creating Agent is an admin. Messages broadcast to current members.",
            json!({"title":string,"members":{"type":"array","items":string}}),
            &["title", "members"],
        ),
        schema(
            "conversation_list",
            "List your personal conversation entries, including read-only departed/dissolved history. groups_only lists your current groups; include_hidden finds removed entries.",
            json!({"include_hidden":{"type":"boolean"},"groups_only":{"type":"boolean"}}),
            &[],
        ),
        schema(
            "conversation_hide",
            "Remove only your conversation entry; membership, messages and Agent execution are unchanged. New messages reveal the entry again for active members.",
            json!({"conversation":string}),
            &["conversation"],
        ),
        schema(
            "conversation_show",
            "Restore your personal conversation entry without rejoining or reactivating a group.",
            json!({"conversation":string}),
            &["conversation"],
        ),
        schema(
            "group_dissolve",
            "Permanently dissolve a group you own. Retains read-only history and stops further messages; cannot be restored.",
            json!({"conversation":string}),
            &["conversation"],
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
            "Send as your fixed identity. Reuse request_key only for identical content. Do not resend a successful message with a new key; identical sends within one runtime Run reuse the first successful key. A successful send to the triggering conversation suppresses automatic final publication. Use mentions to request group responses; sending does not synchronously wait for agents.",
            json!({"conversation":string,"body":string,"request_key":string,"mentions":{"type":"array","items":string},"reply_to":{"type":["string","null"],"description":"Existing message ID in this conversation. Omit or null for a new message; blank strings are treated as absent."}}),
            &["conversation", "body", "request_key"],
        ),
        schema(
            "message_history",
            "Read shared messages oldest-to-newest within a page. Pass next_cursor as before for older history.",
            json!({"conversation":string,"before":{"type":["string","null"],"description":"Use next_cursor from this conversation; omit/null/blank for latest page."},"limit":{"type":"integer","minimum":1,"maximum":200,"default":50}}),
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
    if role == Role::Manager || resource_management {
        let kind = json!({"enum":["provider","model","prompt","skill","mcp","agent","harness"]});
        tools.extend([
            schema("resource_list","List resource summaries. Secrets are redacted.",json!({"kind":kind}),&["kind"]),
            schema("resource_read","Read a resource by category/name (default category optional). Skills support @scope/package. Secrets are redacted.",json!({"kind":kind,"reference":string}),&["kind","reference"]),
            schema("resource_write","Create or update resource. data: provider create {base_url,api_key} (api_key_env alternative), update any field; model create {provider,model,params?: ModelConfigUpdate}, update ModelConfigUpdate fields directly. ModelConfigUpdate.context_window is the input token window (4096..2000000, default 272000) used by runtime auto compaction; it does not limit output tokens; prompt {text}; skill create {description}, update {text: full SKILL.md}; mcp single-server {mcpServers:{name:config}}; harness {engine: codex,executable: absolute path,working_directory: absolute path,sandbox: read_only|workspace_write,model?: string,reasoning_effort?: string,description?: string,timeout_seconds?: 10..86400}; Harness writes require resource_management or Manager. Creating a Harness saves connection configuration; it does not install Codex or create a container. Use harness_models to discover real models and reasoning levels; use harness_session_configure to change only one Agent/chat. Changing a resource default does not change existing sessions. agent {model OR harness,prompt?,skills?,mcps?,builtin_tools?: [resource_management, task_collaboration],description?}, Exactly one of model/harness required on create. External Agents execute only private messages or explicit mentions; their working directory comes from the harness. Agent arrays replace mounts (empty unmounts all); omitted fields unchanged. Categories must exist. Use exactly one of api_key/api_key_env; credentials are never returned. resource_management grants resource and Agent lifecycle management; task_collaboration separately grants asynchronous task requests, inspection, queue controls, cancellation, business replies and task_report during hosted runs; explicit Native/Codex work also exposes task_await/task_yield/task_input for checkpointed suspension. task_attention is restricted to the human workspace owner. Mount task_collaboration on the requesting lead; assignees only need a valid binding. Agent writes return a configuration summary plus activation status and next_steps. Do not report an Agent ready until its intended binding has enabled=true. Creating an Agent definition does not enable it: use principal_create(kind=agent), then agent_bind; use binding_list to inspect existing identities first.",json!({"kind":kind,"reference":string,"create":{"type":"boolean"},"data":{"type":"object"}}),&["kind","reference","create","data"]),
            schema("resource_delete","First omit confirm (or use null) to get dependency-aware deletion plan; then pass exact ordered items as confirm. Empty array does not authorize a nonempty plan. Protects the default prompt.",json!({"kind":kind,"reference":string,"recursive":{"type":"boolean","default":false},"confirm":{"type":["array","null"],"items":string}}),&["kind","reference"]),
            schema("provider_models", "Fetch model IDs using stored Provider credentials. Calls /models; does not run inference.", json!({"reference":string}), &["reference"]),
            schema("resource_test", "Validate Provider or MCP configuration only. Does not contact servers or execute MCP commands.", json!({"kind":{"enum":["provider","mcp"]},"reference":string}), &["kind","reference"]),
            schema("resource_category_list", "List categories in a resource domain.", json!({"kind":kind}), &["kind"]),
            schema("resource_category_create", "Create a category in a resource domain.", json!({"kind":kind,"category":string}), &["kind","category"]),
            schema("resource_category_delete", "Delete an empty non-default category. Nonempty and default categories are protected.", json!({"kind":kind,"category":string}), &["kind","category"]),
        ]);
    }
    if role == Role::Manager || resource_management {
        let principal_kinds = if role == Role::Manager {
            json!({"enum":["user","agent","bot","service","other"]})
        } else {
            json!({"enum":["agent"]})
        };
        tools.extend([
            schema("principal_create","Register a new identity (resource-managed Agents may create only kind=agent). Does not enable execution: create the Agent resource, register its identity, then agent_bind. Use principal_list before creating; never reuse another Agent identity accidentally.",json!({"id":string,"name":string,"kind":principal_kinds}),&["id","name","kind"]),
            schema("agent_bind","Configure an Agent identity to run an existing Agent definition. New bindings start enabled; updating an existing disabled binding preserves disabled state, so call agent_set_enabled(true) to resume. Defaults: workspace home directory, unlimited model steps, mentioned policy, MCP disabled. May trigger pending authorized messages. Check binding_list first: reusing a principal updates its existing binding. allow_mcp still requires discussion authorization.",json!({"principal":string,"agent":string,"working_directory":string,"policy":{"enum":["all","mentioned","manual"]},"max_steps":{"type":"integer","description":"Deprecated compatibility field; ignored. Model steps are unlimited."},"allow_mcp":{"type":"boolean","default":false}}),&["principal","agent"]),
            schema("agent_set_enabled","Enable or disable an existing binding across all conversations while preserving identity, history and running settings. Needs agent_bind first. Enabling may process pending authorized messages; disabling cancels further work, but cannot undo external requests. Disabling yourself stops your current hosted task.",json!({"principal":string,"enabled":{"type":"boolean"}}),&["principal","enabled"]),
            schema("agent_unbind","Remove an Agent identity’s running settings and disable execution; identity and messages remain. For ordinary stopping use agent_set_enabled(false) instead. Unbind before deleting a bound Agent resource. Deleting that resource then retires its last-bound identities; historical names/messages remain, but retired identities cannot be reused.",json!({"principal":string}),&["principal"]),
            schema("binding_list","List saved agent bindings, including their enabled state.",json!({}),&[]),
        ]);
    }
    tools
}

// Validate the small schema vocabulary used by our own tool envelopes. Nested
// resource documents remain validated by their typed resource parsers, not here.
fn validate_arguments(schema: &Value, args: &Value) -> std::result::Result<(), ToolError> {
    let args = args
        .as_object()
        .ok_or_else(|| ToolError::invalid(None, "Arguments must be a JSON object."))?;
    let properties = schema["properties"].as_object().expect("tool properties");
    if args.keys().any(|key| !properties.contains_key(key)) {
        return Err(ToolError::invalid(
            None,
            "Unknown argument; use only fields declared in the tool schema.",
        ));
    }
    for required in schema["required"].as_array().expect("required fields") {
        let field = required.as_str().expect("field name");
        if !args.contains_key(field) {
            return Err(ToolError::invalid(
                Some(field),
                "Required argument is missing.",
            ));
        }
    }
    // Field names come from our schema, never untrusted input.
    for (field, spec) in properties {
        if let Some(value) = args.get(field)
            && !matches_spec(spec, value)
        {
            return Err(ToolError::invalid(
                Some(field),
                "Argument must match the type, enum and bounds declared in the tool schema.",
            ));
        }
    }
    Ok(())
}

fn matches_spec(spec: &Value, value: &Value) -> bool {
    let type_matches = |kind: &str| match kind {
        "null" => value.is_null(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    };
    let valid_type = match &spec["type"] {
        Value::String(kind) => type_matches(kind),
        Value::Array(kinds) => kinds.iter().any(|k| k.as_str().is_some_and(type_matches)),
        Value::Null => true, // enum-only property
        _ => false,
    };
    valid_type
        && spec
            .get("enum")
            .is_none_or(|values| values.as_array().expect("enum").contains(value))
        && spec["minimum"]
            .as_f64()
            .is_none_or(|min| value.as_f64().is_some_and(|n| n >= min))
        && spec["maximum"]
            .as_f64()
            .is_none_or(|max| value.as_f64().is_some_and(|n| n <= max))
        && value
            .as_array()
            .is_none_or(|values| values.iter().all(|item| matches_spec(&spec["items"], item)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationList {
    #[serde(default)]
    include_hidden: bool,
    #[serde(default)]
    groups_only: bool,
}
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
struct ProviderReference {
    reference: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Category {
    kind: String,
    category: String,
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
