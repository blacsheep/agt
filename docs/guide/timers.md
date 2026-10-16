# Timers

A timer posts a message to a conversation later, once or on a repeating schedule. When it fires, it counts as a message from you: it starts a new discussion, so agents get to work even if the last discussion expired long ago. Between firings nothing runs and nothing is polled.

Typical uses:

- "Remind me to send the weekly report in 5 minutes."
- "At 9:00 on the 1st of every month, have the team send the data."

## Enable it

Agents set timers with the `task_collaboration` built-in tools, usually on the lead:

```bash
agt agent update lead --add-builtin-tool task_collaboration
```

The scheduler must be running (the Web console starts it).

## Ask an agent

Say what you want in plain words. The agent calls `timer_create`, and the result tells it the exact local time it resolved, so it can confirm with you.

| You say | The agent sets |
| --- | --- |
| Remind me to send the weekly report in 5 minutes | `after_seconds: 300`, `text: "Reminder: send the weekly report"`, `mentions: [itself]` |
| Remind me at 9 tomorrow | `at: "2026-10-17T09:00"` |
| At 9:00 on the 1st of every month, organize sending the data | `cron: "0 9 1 * *"`, `mentions: ["lead"]` |

## When it fires

- The message appears in your name. When an agent set the timer, it speaks as the workspace owner. The Web console marks it **Timer · set by …**, and the text starts with `[AGT timer <id>]`.
- It starts or renews the discussion exactly as your own message would: reply counts reset and the discussion lasts `scheduler.discussion_seconds`. A timer can set its own `discussion_seconds` (1 minute to 7 days) for long jobs, such as 7200 for a two-hour monthly data run.
- Mentioned members must answer. Others follow the usual [conversation rules](./conversations.md).
- If agt was not running at that time, the timer fires once when it starts again and says it is late. Missed repeats collapse into that one delivery.

## Times

| Way | Example | Notes |
| --- | --- | --- |
| `after_seconds` | `300` | 10 seconds to 366 days |
| `at` | `2026-11-01T09:00` | Local time in the workspace time zone, or with an offset such as `+08:00`. 10 seconds to 366 days ahead. |
| `cron` | `0 9 1 * *` | Five fields: minute, hour, day of month, month, day of week. `L` (last), `#` (nth weekday) and `W` (nearest weekday) are allowed. At least 10 minutes apart. |

The workspace time zone is `scheduler.timezone`, or the host's zone when it is not set (see [Scheduler](../modules/scheduler.md#all-parameters)). A timer can name its own `timezone` and keeps the zone it was created with.

Agents also see the time: their input carries `now` in local time with its offset, the `timezone`, and `sent_at` on each new message.

## Approval

`timer_create` is a write, so it follows your [permission mode](./permissions.md): in `manual` you approve it in the Web console, in `model_review` the reviewer decides, and in `auto_allow` it goes through. Timers you create yourself from the CLI take effect at once.

## See and cancel timers

In the Web console, type `/timers` in a conversation to see its timers, when each fires next, and who set it. From there you can also create a timer (once at a time you pick, or with a cron expression) and cancel one.

From the CLI:

```bash
agt tool call timer_list --principal me --manager
agt tool call timer_create --principal me --manager \
  --args '{"conversation":"<id>","text":"Stand-up in 5 minutes","cron":"55 9 * * 1-5","mentions":["lead"]}'
agt tool call timer_cancel --principal me --manager --args '{"timer_id":"<id>"}'
```

- Agents can cancel only the timers they set. People can cancel any timer in their conversation.
- A timer ends by itself when its conversation is deleted or dissolved, or its human leaves.
- `/stop` ends the current discussion but keeps timers; cancel them separately.

## Limits

| Limit | Value |
| --- | --- |
| Active timers per conversation | 20 |
| Text | 4000 characters |
| Mentions | 10 |
| Where an agent can set one | Only the conversation it is working in |
| Failed delivery | Retried every minute; canceled after 5 attempts |
