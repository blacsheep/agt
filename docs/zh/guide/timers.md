# 定时任务

定时任务会在之后的某个时间往会话里发一条消息，可以只发一次，也可以按周期重复。到点发出的消息算作你发的：它会开启新一轮讨论，所以即使上一轮讨论早已过期，Agent 也会开始干活。两次触发之间不运行任何东西，也不轮询。

常见用法：

- "5 分钟后提醒我交周报。"
- "每月 1 号 9 点，让大家发一下数据。"

## 开启

Agent 通过内置工具 `task_collaboration` 设置定时任务，通常交给 lead：

```bash
agt agent update lead --add-builtin-tool task_collaboration
```

调度器必须在运行（Web 控制台会启动它）。

## 让 Agent 设置

直接用自然语言说。Agent 调用 `timer_create`，返回结果里有换算后的准确本地时间，它可以据此跟你确认。

| 你说 | Agent 设置 |
| --- | --- |
| 5 分钟后提醒我交周报 | `after_seconds: 300`、`text: "提醒：交周报"`、`mentions: [自己]` |
| 明天 9 点提醒我 | `at: "2026-10-17T09:00"` |
| 每月 1 号 9 点组织大家发数据 | `cron: "0 9 1 * *"`、`mentions: ["lead"]` |

## 到点时会发生什么

- 消息以你的名义出现。Agent 设置的定时任务以工作空间所有者的名义发送。Web 控制台会标注**定时任务 · 由 … 设置**，正文以 `[AGT timer <id>]` 开头。
- 它开启或续期讨论，和你亲自发消息完全一样：回复额度重新计算，讨论持续 `scheduler.discussion_seconds`。耗时长的任务可以给定时任务单独设置 `discussion_seconds`（1 分钟到 7 天），例如每月推数据需要两小时就设为 7200。
- 被提及的成员必须回复，其他成员按通常的[对话规则](./conversations.md)处理。
- 如果到点时 agt 没在运行，下次启动时补发一次，并注明迟到。错过的多次周期合并成这一次。

## 时间写法

| 写法 | 示例 | 说明 |
| --- | --- | --- |
| `after_seconds` | `300` | 10 秒到 366 天 |
| `at` | `2026-11-01T09:00` | 工作空间时区的本地时间，也可以带偏移，例如 `+08:00`。10 秒到 366 天之内。 |
| `cron` | `0 9 1 * *` | 五个字段：分、时、日、月、星期。支持 `L`（最后）、`#`（第几个星期几）和 `W`（最近的工作日）。两次触发至少间隔 10 分钟。 |

工作空间时区由 `scheduler.timezone` 指定，不设置时使用本机时区（见[调度器](../modules/scheduler.md#全部参数)）。单个定时任务可以用 `timezone` 指定自己的时区，并一直沿用创建时的时区。

Agent 也能看到时间：它的输入里有带偏移的本地时间 `now`、时区 `timezone`，每条新消息带 `sent_at`。

## 审批

`timer_create` 是写操作，按你的[权限模式](./permissions.md)处理：`manual` 下需要你在 Web 控制台批准，`model_review` 下由审核模型决定，`auto_allow` 下直接通过。你自己在 CLI 创建的定时任务立即生效。

## 查看与取消

在 Web 控制台的会话里输入 `/timers`，可以看到本会话的定时任务、下次触发时间和设置者。在这里也可以新建定时任务（选择一个时间只触发一次，或填写 cron 表达式），以及取消。

在 CLI 中：

```bash
agt tool call timer_list --principal me --manager
agt tool call timer_create --principal me --manager \
  --args '{"conversation":"<id>","text":"5 分钟后开站会","cron":"55 9 * * 1-5","mentions":["lead"]}'
agt tool call timer_cancel --principal me --manager --args '{"timer_id":"<id>"}'
```

- Agent 只能取消自己设置的定时任务。人可以取消所在会话里的任何定时任务。
- 会话被删除或解散，或者它代表的人退出会话时，定时任务自动结束。
- `/stop` 结束当前讨论，但不会取消定时任务，需要单独取消。

## 限制

| 限制 | 值 |
| --- | --- |
| 每个会话等待触发的定时任务 | 20 个 |
| 正文 | 4000 字符 |
| 提及 | 10 个 |
| Agent 可以在哪里设置 | 只能在它正在处理的会话 |
| 投递失败 | 每分钟重试，5 次后取消 |
