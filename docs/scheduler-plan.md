# Persistent discussion scheduler

Current implementation · 2026-09-08. See [operator acceptance](framework-acceptance.md)
and [document index](README.md).

The scheduler owns discussion rounds, independent input tasks, waits, leases and reply
allowances in runtime/scheduler.db. IM stays generic. Explicit runs retain their
own execution rules; automatic message execution uses only the scheduler.

## Module boundaries

`scheduler.rs` is the coordinator and preserves the existing public entry points.
Its tick order is explicit: heartbeat, poll active jobs, reconcile work, scan inputs,
expire discussions, reconcile deliveries, then dispatch ready tasks. Each stage
retains its original reads, writes and transaction boundaries.

| Module | Responsibility |
| --- | --- |
| `scheduler/lease.rs` | Coordinator ownership, independent heartbeat and release |
| `scheduler/controls.rs` | Human authorization, delete/dissolve and context reset |
| `scheduler/ingress.rs` | Convert IM deliveries into durable scheduler tasks |
| `scheduler/admission.rs` | Recheck binding, policy, discussion, snapshot and waits |
| `scheduler/guard.rs` | Transaction fences and per-attempt model accounting |
| `scheduler/execution.rs` | Shared input envelope, Run request and execution orchestration |
| `scheduler/backend.rs` | Native/Harness policy, context, participation and model-specific checks |
| `scheduler/decision.rs` | Participation inference and silent/wait/ready outcomes |
| `scheduler/completion.rs` | Run settlement, fenced publication and report obligations |

The claimed-task path is `admission → context → decision or Run → completion`.
A participation decision still releases its slot before reply admission; it does
not fall through into execution. Model and external calls remain outside the
short fence transactions. Publication still checks the claim before sending.
`ScheduleStore` retains persistence, atomic claims and counter transitions;
this split does not introduce new task states, schema changes or engine protocols.

## Engine preparation boundary

Admission resolves an `EngineKind` using the same agent configuration lookup as
Runtime dispatch. Native configuration is still resolved only after admission.
`scheduler/backend.rs` owns the differences below; it does not run either protocol.

| Concern | Native | Harness |
| --- | --- | --- |
| Nonmanual response policy | Agent peers use participation; human policy is preserved | Direct/mentioned messages |
| Participation and work triage | Native restricted model calls | Framework admission, no native inference |
| Context | Token-budgeted history and compaction; exclude trigger messages from history | Visible history including triggers for existing Harness rendering |
| Budget and mounted MCP admission | Existing native model budget and binding/discussion checks | Existing Harness validation during execution |

Shared code still creates the input envelope and Run key, binds the scheduled task,
and settles the returned Run. Error handling is unchanged: native context failures
settle the task at preparation; Harness context errors propagate to the existing
execution boundary. `engine/dispatch.rs` selects execution only after Run claim;
Runtime retains common replay and terminal-state persistence. Cancellation,
checkpoint compatibility and external protocol facts are not normalized away.

## Task transitions

`schedule_state.rs` owns the coupled record changes for claim/deferral, human wake,
lease recovery, participation, accepted inputs and Run settlement. Its operations
mutate only a `ScheduledTask`; authorization, usage charges/refunds, acceptance
receipts and database writes stay in their existing transactions. Task state strings
and serialized records are unchanged. Local one-field outcomes and error formatting
remain at the caller; there is no generic `set_state` API or new transition engine.

| Operation | Preserved contract |
| --- | --- |
| Claim | Set evaluating/responding, owner, version and usage epoch together |
| Defer a claim | Preserve owner/version until the store validates and refunds the old claim |
| Human wake | Clear quiet/reason/owner, increment version, retain the decision audit |
| Resume blocked work | Preserve claim and wait fields; only reset state/due/reason |
| Invalidate ready snapshot | Clear owner/reason and increment version; preserve quiet settings |
| Recover expired lease | Retry only evaluating; responding becomes interrupted |
| Scheduled participation | Persist decision; participate becomes ready, never runs inline |
| Input participation wait | Retain decision audit, release input owner; remove receipt in caller transaction |
| Accept running input | Record owner, version, Run and mode before execution |
| Complete an input turn | Release input owner, retain its Run and diagnostic reason |
| Settle parent | Only outstanding accepted inputs inherit its outcome; completed inputs remain final |
| Finish reply | Preserve report obligations and silence semantics separately from explicit work |

## Messages and running input queues

Human messages require explicit automatic-reply authorization. The first creates
a discussion; subsequent newly authorized human messages in the same unfinished
discussion reset all per-principal reply counters and renew its 30-minute expiry.
Message authorization is durable and idempotent: replaying an authorization or
resuming a discussion never resets counters. Ended discussions remain ended;
a new human message can create a new discussion.

Each pending message remains an independent task; there is no fixed five-second
quiet window or automatic burst merge. Explicit model-requested timed/quiet waits
remain supported. Human authorization wakes pending waits and reply-limited work.
Model-visible history uses persisted summaries and token-budgeted recent messages;
see [context management](context-management.md).

Native ordinary chat accepts same-principal, same-conversation, same-discussion
inputs at safe boundaries. Human messages default to steering; Agent messages
default to follow-up. `input_mode` can explicitly select either mode without
changing authorization. Steering enters between complete tool batches or before
a final response is committed; follow-up enters after the current turn completes.
Retries retain their request snapshot and do not consume inputs. Work tasks keep
their task_input/await protocol; Harness processes later inputs in separate Runs.
See [native agent loop](native-agent-loop.md) for queue and recovery contracts.

## Separate decision and reply admission

One coordinator per home dispatches up to three threads by default; concurrency
is configurable in 1–16. Different principals may evaluate participation in the
same conversation concurrently, including while a peer is replying. Decisions
use one restricted participate/silent/wait function call without connecting MCP.

A participation decision persists `ready_to_reply`, its safe reason code and the
history sequence it considered. It releases the thread and does not yet consume
a reply. All three participation actions also persist a separate `last_decision`
with the decision timestamp, considered message sequence and optional
`public_reason` (at most 240 Unicode characters). It survives scheduler state
changes and restart. Old task JSON defaults to no recorded decision. This is a
short user-facing explanation, not the model's private decision transcript, which
is not persisted or replayed.
Formal reply input includes the decision reason. Ready replies have queue priority
over evaluations. Before reply admission, new history invalidates the decision
and sends the task back to evaluation without a fixed quiet window.

At most one autonomous reply starts at a time in a conversation, and it waits for
existing responders. Human requests and explicitly mentioned peers retain direct
execution and may reply in parallel across identities, without a fixed quiet window.
Response policy, active membership, message deletion and expiry remain execution gates.
MCP availability follows Agent mounts; individual tool calls still require approval. Unmentioned nonmanual peer messages use participation.

One principal has at most one evaluating, ready or responding task across all
conversations. A ready task reserves the identity but not a thread. Multiple
principals bound to one Agent definition can execute concurrently.

## Limits and controls

Each principal may begin 100 replies per discussion between new authorized human
inputs. A reply is charged at execution admission; decisions, waiting and ready
queue time do not count. Pre-execution deferral refunds
that admission, without subtracting from a newer human turn's counters. Failed or
interrupted execution attempts retain their charge. Agent messages and cross-chat
peer sends inherit their origin discussion and never reset the allowance.

There is no discussion-wide model-call limit or per-principal decision-call limit.
Calls remain diagnostic counters. Single-Run `max_steps`, binding response policy
and the 30-minute discussion expiry remain in effect. Automated message depth is
not limited. Bindings have no max_depth field. Causal depth remains in messages
and Run requests for diagnostics.
Timed waits accept 1–300 seconds; model quiet waits accept 1–30 seconds and a maximum
wait of 1–300 seconds, with quiet interval no greater than maximum wait.

Pause stops new work; in-flight replies may finish. Resume renews expiry and can
approve mounted MCP, but does not renew the 100-reply allowance. A new authorized
human message resets it. End/delete cancels pending/ready/waiting work. Controls
apply at the originating conversation, including peer tasks elsewhere.

## Persistence and recovery

Short SQLite transactions reserve work and counters; none spans a model or MCP
request. Independent coordinator heartbeats and versioned claims fence stale work.
Expired evaluating work can retry; responding work becomes interrupted and is
never automatically replayed. Ready and waiting tasks survive restart.

Scheduler databases use schema v17, adding durable run_inputs acceptance receipts. Unsupported schema versions are rejected. Human authorization is deduplicated
in human_messages, and tasks persist counter epochs and reply admission state.

Local framework operations and final publication recheck discussion/claim state.
Already-issued external MCP requests cannot be recalled. A publication failure is
interrupted and requires inspection; a Run result is not an automatic replay plan.
Web launches a watch coordinator; takeover after an abnormal exit may wait for the 15-second lease.

Storage: im/im.db holds messages/deliveries; runtime/bindings.db holds identities
and policies; runtime/runs.db holds formal Runs and call diagnostics;
runtime/scheduler.db holds discussions, task decisions, waits and reply counters.
An evaluation/ready/wait may have no Run. `schedule status` includes reply usage;
Web debug mode shows counts, safe reasons and states, never model reasoning.

CLI: `schedule start <conversation> [--message <id>]`,
`schedule run --watch [--concurrency 3]`, `schedule status`,
`schedule pause|end <conversation>`, `schedule resume <conversation>`,
and `schedule delete <conversation> --actor <principal>`.
Start with --message authorizes that human message idempotently; without it, start
authorizes pending human deliveries and resumes. Non-watch execution drains only
currently ready work; use watch to wait through quiet windows and reminders.

Verification covers immediate independent inputs, steering/follow-up boundaries, concurrent decisions
and direct replies, autonomous reply exclusion, stale decisions, 100-reply reset
and idempotency, waits/restart, unsupported-schema rejection and pause/end/publication fencing.
Tests use local fixtures, not paid model requests.

Group dissolution uses `schedule dissolve <conversation> --actor <principal>`,
with owner-only IM authorization and terminal read-only history. Personal entry
hide/show never changes scheduling. End also ends discussions with tasks targeting
the selected conversation, including work originating elsewhere. IM history and
entry schema migration is described in [conversation-lifecycle.md](conversation-lifecycle.md).

Each logical model step (including participation decisions) retries transport/HTTP,
invalid JSON, explicit API errors and empty responses up to 10 total attempts.
Backoff is 250ms exponential, capped at 4s, checked for cancellation every 50ms.
Every hosted attempt passes ScheduledModel admission/fencing and increments actual
call accounting. Retries preserve the transcript and never replay completed tools;
Run model_steps remain logical steps, with request_attempts in the model call audit.
Refusals, truncation, configuration and tool failures do not trigger this retry loop.

Logical model steps are no longer capped, including explicit delegated child runs.
Legacy max_steps fields/options are accepted but ignored. model_steps remains an
observational counter. Discussion reply allowance/expiry, cancellation, context
size guards and the 10-attempt connection retry ceiling remain separate controls.

Within one runtime Run, successful framework_message_send operations with identical
conversation, body, normalized mentions and reply target reuse the first successful
request key even when the model emits new tool-call IDs/keys. Each invocation still
passes service validation, membership checks and scheduler fencing; the IM store
returns its existing message with duplicate=true. Failures are not cached. This is
not cross-Run text deduplication, and distinct recipients/reply targets remain distinct.

Accepted inputs are charged separately and recorded before execution. Pending input
state is `input` with its consuming Run ID. Completed turns settle their inputs;
a later failure does not overwrite completed inputs. On failure or lease takeover,
remaining accepted inputs settle with the parent and are never automatically
replayed. Unaccepted inputs remain queued. Follow-up starts a new send-deduplication
scope; steering preserves the current scope.
