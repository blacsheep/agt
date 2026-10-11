/* Settled executions as message threads: a collapsed line under the reply a Run published,
   or under the message that triggered it when the Run stayed silent or failed. */
window.AgtRunThreads = (() => {
  let runs = [], scope = '', request = 0;
  const open = new Set();
  const endings = { failed: '处理失败', canceled: '已取消', interrupted: '执行中断', outcome_unknown: '结果待确认', suspended: '已挂起' };
  const symbols = { completed: '✓', failed: '!', canceled: '−', unsupported: '!', unknown: '?' };
  const runOf = m => m.metadata?.run_id || /^tool:run-result:(.+)$/.exec(m.client_message_id || '')?.[1];
  const scopeOf = () => JSON.stringify([state.viewer, state.conversation?.id]);
  const count = r => `${r.items.length >= 40 ? '40+' : r.items.length} 次工具调用`;
  const failures = r => r.items.filter(a => a.status === 'failed').length || r.tool_failures;
  const spent = r => AgtToolActivity.duration({ started_at: r.started_at, updated_at: r.ended_at, status: 'completed' });

  async function load() {
    const key = scopeOf(), conversation = state.conversation?.id, mine = ++request;
    if (!conversation) { runs = []; scope = key; return; }
    try {
      const data = await api('message-runs?' + new URLSearchParams({ conversation, viewer: state.viewer || state.boot.principal }));
      if (mine !== request || key !== scopeOf()) return;
      runs = data.runs; scope = key; render(true);
    } catch { if (mine === request && key !== scope) runs = []; }
  }
  function reset() { request++; runs = []; scope = ''; open.clear(); }

  // anchor message id -> [{run, mode}]; a Run without public trace or failure stays out of the transcript.
  function anchors() {
    const result = new Map();
    if (scope !== scopeOf()) return result;
    const sent = new Map(), loaded = new Set(state.messages.map(m => m.id));
    for (const m of state.messages) { const id = runOf(m); if (id) sent.set(id, m.id); }
    for (const run of runs) {
      const reply = sent.get(run.id), failed = run.state !== 'completed';
      let anchor, mode;
      if (reply) { anchor = reply; mode = 'reply'; if (!run.items.length && !failures(run)) continue; }
      else if (loaded.has(run.trigger_message)) { anchor = run.trigger_message; mode = 'trigger'; if (!run.items.length && !failed) continue; }
      else continue;
      if (!result.has(anchor)) result.set(anchor, []);
      result.get(anchor).push({ run, mode });
    }
    for (const list of result.values()) list.sort((a, b) => (a.run.started_at || 0) - (b.run.started_at || 0));
    return result;
  }
  function line({ run, mode }) {
    const fail = failures(run), name = esc(nameOf(run.principal));
    const what = mode === 'reply' ? count(run)
      : run.state !== 'completed' ? `<strong>${name}</strong> ${endings[run.state] || '处理未完成'}${run.items.length ? ' · ' + count(run) : ''}`
      : `<strong>${name}</strong> ${run.silent ? '处理了这条消息，没有发言' : '没有发布回复'} · ${count(run)}`;
    const recent = run.items.slice(-8);
    return `<details class="run-thread" data-thread-run="${esc(run.id)}" data-state="${esc(run.state)}"${open.has(run.id) ? ' open' : ''}>
      <summary><span class="run-thread-icon" aria-hidden="true">⚙</span><span class="run-thread-text">${what}${fail ? ` · <em>${fail} 项失败</em>` : ''}</span><small>${esc(spent(run))}</small></summary>
      ${recent.length ? `<ol class="run-thread-list">${run.items.length > recent.length ? `<li class="run-thread-skip">… 更早的 ${run.items.length - recent.length} 项</li>` : ''}${recent.map(a => `<li data-status="${esc(a.status)}"><span aria-hidden="true">${symbols[a.status] || '?'}</span><span>${esc(AgtToolRenderers.describe(a).title)}</span><small>${esc(AgtToolActivity.duration(a))}</small></li>`).join('')}</ol>` : ''}
      <button type="button" class="run-thread-more" data-run-panel="${esc(run.id)}">查看完整执行记录 →</button>
    </details>`;
  }
  function render(preserveBottom = false) {
    const box = $('#messages'), bottom = preserveBottom && box.scrollHeight - box.scrollTop - box.clientHeight < 100;
    const map = anchors();
    $$('#messages [data-thread-anchor]').forEach(slot => {
      const html = (map.get(slot.dataset.threadAnchor) || []).map(line).join('');
      if (slot.dataset.html === html) return;
      slot.dataset.html = html; slot.innerHTML = html;
      slot.querySelectorAll('details').forEach(d => d.addEventListener('toggle', () => { d.open ? open.add(d.dataset.threadRun) : open.delete(d.dataset.threadRun); }));
      slot.querySelectorAll('[data-run-panel]').forEach(b => b.onclick = () => panel(b.dataset.runPanel));
    });
    if (bottom) box.scrollTop = box.scrollHeight;
  }

  // Full timeline in the side panel; each tool card loads its private payload lazily and scoped.
  function panel(id) {
    const run = runs.find(r => r.id === id); if (!run) return;
    const viewer = state.viewer, conversation = state.conversation.id, anchor = [...anchors()].find(([, list]) => list.some(e => e.run.id === id))?.[0];
    const status = run.state === 'completed' ? (run.silent ? '已结束 · 未发言' : '已结束 · 已回复') : endings[run.state] || run.state;
    commandPanel(`${nameOf(run.principal)} 的执行记录`, `<div class="run-panel">
      <dl class="run-panel-facts"><dt>状态</dt><dd>${esc(status)}</dd><dt>开始</dt><dd>${Number.isFinite(run.started_at) ? esc(new Date(run.started_at).toLocaleString('zh-CN')) : '未记录'}</dd><dt>耗时</dt><dd>${esc(spent(run))}</dd><dt>工具</dt><dd>${esc(count(run))}${failures(run) ? ` · ${failures(run)} 项失败` : ''}</dd><dt>引擎</dt><dd>${run.engine === 'codex' ? 'Codex' : '内置'}</dd></dl>
      ${anchor ? '<button type="button" class="secondary" id="run-panel-locate">定位到消息</button>' : ''}
      <div id="run-panel-tools" class="harness-activity"></div>
      <p class="form-note">${run.items.length >= 40 ? '只保留最近 40 项公开工具记录。' : ''}执行结束不代表所有操作均成功；展开单项可查看参数与返回结果。</p>
    </div>`, 'run');
    $('#command-panel').dataset.run = id;
    if (!run.items.length) $('#run-panel-tools').innerHTML = '<p class="form-note">这次执行没有调用工具。</p>';
    else AgtToolActivity.update($('#run-panel-tools'), run.items, { key: JSON.stringify([viewer, conversation, run.id]), load: activity => api('tool-detail', { id: run.id, activity, viewer, conversation }) });
    $('#run-panel-locate')?.addEventListener('click', () => {
      const target = document.querySelector(`#messages [data-thread-anchor="${CSS.escape(anchor)}"]`)?.closest('.message');
      if (!target) return;
      target.scrollIntoView({ block: 'center', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
      target.classList.remove('flash'); void target.offsetWidth; target.classList.add('flash');
    });
  }
  return { load, reset, render, panel };
})();
