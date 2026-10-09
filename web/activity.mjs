// Public activity projection. Raw requests, output, paths and approval payloads never leave this module.
const kinds = new Set(['command', 'files', 'mcp', 'framework', 'input']);
const statuses = new Set(['running', 'completed', 'failed', 'canceled', 'unsupported', 'unknown']);
const safeName = value => typeof value === 'string' && /^[a-zA-Z0-9_.:/-]{1,128}$/.test(value);
const finite = value => Number.isFinite(value) ? value : null;

export function publicActivities(items, settled = false) {
  const result = new Map();
  for (const [index, a] of (Array.isArray(items) ? items : []).slice(-40).entries()) {
    if (!a || !kinds.has(a.kind) || !statuses.has(a.status)) continue;
    // Runtime IDs are audit IDs or hashed engine IDs; legacy snapshots get a local fallback.
    const id = typeof a.id === 'string' && /^(call:\d+|[a-f0-9]{64})$/.test(a.id) ? a.id : `legacy:${index}:${finite(a.started_at) || 0}`;
    result.set(id, { id, kind: a.kind, status: settled && a.status === 'running' ? 'unknown' : a.status,
      started_at: finite(a.started_at), updated_at: finite(a.updated_at),
      ...(safeName(a.tool_name) ? { tool_name: a.tool_name } : {}),
      ...(typeof a.label === 'string' ? { label: a.label.slice(0, 80) } : {}),
      ...(typeof a.summary === 'string' ? { summary: a.summary.slice(0, 120) } : {}) });
  }
  return [...result.values()];
}

function liveProgress(record, scheduler, visible, conversation) {
  const p = record.progress, trigger = record.request.trigger;
  if (!visible || !scheduler.online || !p || record.state !== 'running' || record.parent_run_id || trigger?.conversation_id !== conversation) return null;
  const task = scheduler.tasks.find(t => t.id === p.task && t.version === p.task_version && t.state === 'responding' && t.principal === trigger.principal && t.conversation === conversation);
  const round = task && scheduler.discussions.find(d => d.id === task.discussion);
  if (!round || !['active', 'paused'].includes(round.state)) return null;
  return { phase: p.phase, attempt: p.attempt, preview: typeof p.preview === 'string' ? p.preview.slice(-4096) : '',
    started_at: p.started_at, updated_at: p.updated_at, principal: task.principal,
    ...(Number.isFinite(p.protocol_at) ? { protocol_at: p.protocol_at, local_checked_at: p.local_checked_at, meaningful_at: p.meaningful_at } : {}),
    activities: publicActivities(p.activities) };
}

export async function activitySnapshot({ run, tool, conversation, viewer }) {
  const [records, scheduler] = await Promise.all([run(['run', 'list', '--limit', '50']), run(['schedule', 'status'])]);
  const members = conversation ? await tool('member_list', { conversation }, viewer).catch(() => []) : [];
  const visible = members.some(m => m.principal_id === viewer && m.left_message_seq == null);
  const runs = records.map(r => {
    const progress = liveProgress(r, scheduler, visible, conversation);
    const scoped = visible && !r.parent_run_id && r.request.trigger?.conversation_id === conversation;
    const saved = r.execution?.activity;
    const items = progress?.activities || (scoped && r.state !== 'running' ? publicActivities(saved?.items, true) : []);
    return { id: r.id, agent: r.request.agent, conversation: r.request.trigger?.conversation_id, state: r.state,
      model_steps: r.model_steps, tool_failures: r.tool_failures,
      error: r.error ? '执行失败，请查看消息投递了解原因。' : null, progress,
      tool_activity: scoped && (progress || items.length) ? {
        principal: r.request.trigger.principal, engine: r.execution?.engine === 'codex' ? 'codex' : 'native', items,
        started_at: finite(progress?.started_at ?? saved?.started_at),
        updated_at: finite(progress?.updated_at ?? saved?.updated_at), ended_at: finite(saved?.ended_at), approvals: []
      } : null };
  });
  // Only ask for approvals when there is a visible live execution. The approval store rechecks scope.
  const live = runs.filter(r => r.progress);
  if (live.length) {
    try {
      const approvals = await run(['permission', 'list', '--principal', viewer]);
      for (const r of live) r.tool_activity.approvals = approvals
        .filter(a => a.run_id === r.id && a.conversation === conversation && a.delivery === 'not_sent' && ['reviewing', 'awaiting_human'].includes(a.state))
        .map(a => ({ id: a.id, source: a.source, state: a.state }));
    } catch {
      for (const r of live) r.tool_activity.approvals_unavailable = true;
    }
  }
  return { scheduler, runs };
}
