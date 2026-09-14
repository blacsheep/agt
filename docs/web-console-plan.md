# Local web console architecture

Current implementation · 2026-09-08. Operator instructions are in
[web/README.md](../web/README.md); scheduling behavior is defined in
[scheduler-plan.md](scheduler-plan.md).

## Process and responsibility boundaries

The frontend is static HTML/CSS/JavaScript without a build step. A dependency-free
Node server serves assets and adapts an allowlisted HTTP API to Rust CLI commands.
Resource, permission, messaging and execution logic remain in Rust.

Web startup prepares the selected home/operator and launches a Rust watch
coordinator. It becomes standby if another coordinator owns the home. Only one
coordinator is active per home; it dispatches up to three tasks by default, with
one active task per principal across conversations. Automatic message execution
uses this coordinator.

Node does not own a serial reply queue or a ten-batch loop. Messages, bindings,
Runs and scheduler state persist in their respective SQLite databases. Closing
an owning Web process interrupts its scheduler; another standby may take over.
Previously authorized waiting work can resume on restart, unlike ambiguous
response/tool execution, which requires inspection.

## HTTP workflow

- Successful message_send with auto_reply authorizes the exact returned message
  through schedule start --message. Retried duplicate sends repeat idempotent
  authorization, covering failure between message creation and scheduling.
- Reply controls check operator membership and call schedule start/resume/pause/end.
  Start/resume require model-cost confirmation; mounted MCP needs both binding
  and discussion authorization.
- Activity combines scheduler status with redacted Run metadata. Run inputs,
  outputs and raw error payloads are not returned by this list endpoint.
  When a selected conversation is supplied, activity additionally returns an allowlisted
  bounded draft for its active scheduled Runs, checking viewer membership, task version
  and discussion state. Child Runs and invalid/terminal tasks never expose drafts.
- Personal hide/show changes only the operator’s conversation entry. Leave retains
  readable membership-window history; dissolve is owner-only and terminal.
  These operations use the fixed operator. Legacy soft-delete/reactivate remains
  available for compatibility, but the Web remove-entry control never calls it.
- Startup persists the human workspace owner. Agent-created groups include that
  owner automatically; the creator becomes admin. Agent observation is a separate
  read-only page. See [conversation-lifecycle.md](conversation-lifecycle.md).

## UI behavior

Send-triggered replies are enabled by default with a cost notice. Human requests
can run concurrently across principals. Unmentioned peer messages may be evaluated
for participate/silent/wait; explicit peer mentions bypass that decision. Policy and
reply allowance remain hard gates. Every new message resets a five-second
quiet window before new work. Decisions can overlap in one conversation; autonomous
replies wait in durable ready_to_reply state and start one at a time. New authorized
human input resets the 100-reply allowance per principal. Model-call counts are
diagnostic only. Timed/quiet waits and ready replies occupy no execution thread.

Simple mode groups status by conversation and principal. A compact conversation
summary accompanies expandable member rows separating current execution state
from the latest recorded model participation decision. Public decision explanations
are previewed in two lines; details show the full explanation, time and wake
condition. Keyed member nodes preserve expansion and keyboard focus across polls.
Tablet and mobile layouts retain all returned members. Old decisions are labeled
when new messages or reevaluation make them historical; missing records are not
inferred from silent execution results. Browser-persisted debug_mode additionally shows
reason codes, wake times, claim/reply-count information and Run call diagnostics. It
changes presentation, not authorization or logging policy; no private model
reasoning is exposed.

Navigation defaults to a compact icon rail and can expand. Conversation and
activity panes have independent persistent collapse controls outside the panes;
hiding them retains DOM state and does not affect scheduling. Agent secondary
actions and recent execution history use inline disclosure. The workspace uses
shared cool-neutral/blue tokens, compact card controls and responsive column layouts.
The visual direction references office chat applications: a pale navigation rail, white
panels, blue selection and actions, and a lightly tinted message area.

Users first select an identity, then its conversation list, then a session.
Only the startup operator can write; other identities are read-only observation.
Delete belongs to list rows, with confirmation and undo. Pause allows current
replies to finish; end stops subsequent replies/reminders. Already dispatched
external MCP requests cannot be guaranteed to stop.

## Security and supported scope

IPv4 loopback only, exact Host/Origin validation, per-launch token, JSON request
limits, allowlisted operations and argument-array CLI execution (no shell).
This is a trusted host administration console, not multi-user authentication or
a public tenant-isolated service. The token permits resource management and
observation of private conversations; never expose it to untrusted clients.

The UI supports resource/Agent creation, readable resource details, confirmed
non-cascading resource deletion, binding updates/unbinding, private/group chat and
recoverable conversation deletion. Resource deletion reuses the service's exact
plan confirmation and reference/hosting protections. Direct Provider keys travel
through a per-child environment, never CLI arguments; resource reads stay redacted.
Agent editing updates model/Prompt references, descriptions and Skill/MCP mounts.
Creation and editing expose an opt-in resource_management builtin group. The runtime
uses the Agent snapshot to grant resource-only capabilities to its Participant
identity; it does not start another MCP process or require external MCP approval.
Resource schemas and operations are shared with manager CLI/MCP entrypoints.
Membership changes and archiving remain CLI operations. There is no game-specific layer or workflow DSL.

## Verification record

The 2026-09-07 implementation passed 261 Rust tests, seven Web API/integration
tests, formatting and Clippy. Browser checks with a local mock model verified
concurrent replies (two overlapping requests), model wait/silence, presence,
debug persistence, pause, delete/undo, read-only views and mobile layout.
No paid model calls were made for that implementation verification. Historical
manual-worker acceptance results do not establish new scheduler live-model quality.

## Streaming model progress

`agt-runtime/model_stream.rs` assembles Chat SSE deltas or a Responses completed response;
only validated complete assistant messages enter the tool loop. `ModelClient::complete_stream`
has a default implementation for nonstreaming clients. Retry attempts reset the preview;
stream termination errors retry the model request without replaying tools.

`progress.rs` maintains one bounded snapshot on the Run record, throttled to 300ms for
text deltas and cleared by Run finalization. This requires no SQLite migration. Snapshots
are operational state, not durable memory or public IM messages. They contain only a phase,
attempt, timestamps, task/version attribution and the latest 4 KiB of assistant text;
reasoning events become a phase only. Compaction/participation model output is not previewed.

Node remains a CLI adapter; the browser polls activity every 1.5s and renders drafts in a
height-limited, independently collapsible area beneath history. There is no Node SSE broker.
Provider JSON fallback and blocked-read cancellation limitations are documented in the Web guide.

## Contact search and conversation menu

Contact search reads the existing bootstrap principal directory (name/id); conversation search
filters the selected list scope. Clicking a contact reuses `direct_create`, personal `show`,
and the existing archived-chat restore flow. No model calls or new message sends are involved.
Only the fixed operator can start chats; Agent observation stays read-only.

Conversation context menus are shared by right-click, Shift+F10 and the visible ellipsis button.
They expose open, info and confirmed personal hide, with keyboard focus restoration and viewport
clamping. Group departure/dissolution remains in the existing group drawer. Menus never bypass
service permissions. Existing API and Rust contracts are unchanged by this UI update.

## Agent entry organization

The trusted host console exposes `/api/agent-entries` for confirmed batches of 1–100
conversation IDs and only hide/show actions. The target must be an Agent principal;
operations reuse its participant-scoped conversation_hide/show service calls without
manager authority. Each item reports success/failure. The observation page adds explicit
selection mode and a hidden-entry filter; this does not enable impersonated messaging,
group lifecycle changes, execution control or transcript deletion.
