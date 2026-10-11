/* Agent profile drawer: who the Agent is, what it is doing now, where it works and what it can use. */
window.AgtAgentDetail = (() => {
  const sheet = document.querySelector('#agent-detail');
  let reference = null, tab = 'overview', chats = null, request = 0;
  const tabs = [['overview', '概览'], ['chats', '会话'], ['runs', '执行'], ['abilities', '能力']];
  const policies = { mentioned: '私聊或群内被 @ 时回应', all: '回应所有消息', manual: '不自动回应，需手动继续' };
  const builtins = { resource_management: ['资源管理', '创建和修改资源、绑定 Agent 并启停'], task_collaboration: ['任务协作', '委派可追踪任务、安排依赖与并行'], local_execution: ['本机命令', '可执行 Bash（需在运行设置开启本机执行）'] };
  const runStates = { running: '执行中', completed: '已完成', failed: '失败', canceled: '已取消' };

  const agent = () => items('agent').find(a => ref(a) === reference);
  const bindings = () => state.boot.bindings.filter(b => b.agent === reference);
  const principals = () => bindings().map(b => b.principal);
  const runs = () => state.runs.filter(r => r.agent === reference);

  function duration(r) {
    if (!r.started_at) return '';
    const seconds = Math.max(0, Math.round(((r.ended_at || Date.now()) - r.started_at) / 1000));
    return seconds < 60 ? `${seconds} 秒` : seconds < 3600 ? `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒` : `${Math.floor(seconds / 3600)} 小时 ${Math.floor(seconds % 3600 / 60)} 分`;
  }
  function chatName(c) { return c.kind === 'direct' && chats?.names.get(c.id) ? chats.names.get(c.id) : conversationName(c); }

  // Conversations are read from the Agent's own perspective, the same read-only scope as "Agent 会话".
  async function loadChats() {
    const generation = ++request, ids = principals();
    chats = null;
    try {
      const [mine, ...theirs] = await Promise.all([read('conversation_list', { include_hidden: true }, state.boot.principal), ...ids.map(id => read('conversation_list', { include_hidden: true }, id).then(list => list.map(e => ({ ...e, viewer: id }))))]);
      const seen = new Map();
      for (const entry of theirs.flat()) if (!seen.has(entry.conversation.id)) seen.set(entry.conversation.id, entry);
      const list = [...seen.values()].sort((a, b) => (b.last_message?.created_at || 0) - (a.last_message?.created_at || 0));
      const names = new Map();
      await Promise.all(list.filter(e => e.conversation.kind === 'direct').slice(0, 20).map(async e => {
        const members = await read('member_list', { conversation: e.conversation.id }, e.viewer).catch(() => []);
        names.set(e.conversation.id, members.filter(m => m.principal_id !== e.viewer && m.left_message_seq == null).map(m => nameOf(m.principal_id)).join('、') || '私聊');
      }));
      if (generation !== request) return;
      chats = { list, names, shared: new Set(mine.map(e => e.conversation.id)) };
    } catch (e) {
      if (generation === request) chats = { error: errorText(e), list: [], names: new Map(), shared: new Set() };
    }
    if (sheet.open) render();
  }

  function hero(a) {
    const enabled = bindings().some(b => b.enabled !== false), main = principals()[0], tone = toneOf(main || reference);
    const model = a.harness ? `Codex · ${ref(a.harness)}` : a.model ? `${a.model.category || 'default'}/${a.model.name}` : '未配置模型';
    return `<header class="agent-hero" data-tone="${tone}">
      <button type="button" class="agent-sheet-close" data-detail-close aria-label="关闭">×</button>
      <div class="agent-hero-avatar"><span class="avatar" data-tone="${tone}" aria-hidden="true">${esc(initialOf(a.name))}</span></div>
      <h2 id="agent-detail-title">${esc(main ? nameOf(main) : a.name)}</h2>
      <p class="agent-hero-meta"><span class="agent-hero-state ${enabled ? 'on' : ''}">${enabled ? '已启用' : bindings().length ? '已停用' : '未启用'}</span><span>模型 · ${esc(model)}</span><span>配置 · ${esc(reference)}</span></p>
      <p class="agent-hero-description">${esc(a.description || '尚未填写角色说明。')}</p>
      <div class="agent-hero-actions">
        ${main && enabled ? `<button type="button" class="primary" data-detail-direct="${esc(main)}">发起私聊</button>` : ''}
        ${main ? `<button type="button" class="secondary" data-detail-observe="${esc(main)}">以它的视角查看会话</button>` : ''}
        <button type="button" class="secondary" data-detail-edit>编辑角色</button>
        <button type="button" class="quiet" data-detail-toggle="${!enabled}">${enabled ? '停用' : '启用'}</button>
      </div>
    </header>`;
  }

  function now() {
    const work = principals().map(workingState).find(Boolean);
    const queued = state.scheduler.tasks.filter(t => principals().includes(t.principal) && ['queued', 'ready_to_reply', 'waiting'].includes(t.state)).length;
    if (!state.scheduler.online) return '<div class="agent-now idle"><span class="agent-now-dot"></span><div><strong>状态未知</strong><small>调度服务未连接，无法确认当前状态。</small></div></div>';
    return work ? `<div class="agent-now working"><span class="agent-now-dot"></span><div><strong>${esc(work.label)}</strong><small>${esc(work.conversation ? `在「${work.conversation}」` : '在一个你不可见的会话中')}${queued ? ` · 另有 ${queued} 项排队` : ''}</small></div></div>`
      : `<div class="agent-now idle"><span class="agent-now-dot"></span><div><strong>空闲</strong><small>${queued ? `${queued} 项等待处理` : '没有正在处理的消息'}</small></div></div>`;
  }

  function overview() {
    const list = runs(), done = list.filter(r => r.state !== 'running');
    const ok = done.filter(r => r.state === 'completed' && !r.tool_failures).length;
    const stats = [
      [chats ? chats.list.length : '…', '参与的会话'],
      [list.length, '最近执行'],
      [done.length ? `${Math.round(ok / done.length * 100)}%` : '—', '顺利完成'],
      [list.reduce((n, r) => n + (r.model_steps || 0), 0), '模型步数'],
    ];
    const strip = list.slice(0, 24).reverse();
    return `${now()}
      <div class="agent-stats">${stats.map(([value, label]) => `<div><strong>${esc(String(value))}</strong><small>${label}</small></div>`).join('')}</div>
      <section class="agent-section"><h3>最近执行 <small>最近 ${strip.length} 次 · 由旧到新</small></h3>${strip.length ? `<div class="run-strip" role="list">${strip.map(r => `<button type="button" role="listitem" class="run-tick ${esc(r.state)}${r.tool_failures ? ' partial' : ''}" data-detail-run="${esc(r.id)}" title="${esc(`${runStates[r.state] || r.state}${r.tool_failures ? ` · ${r.tool_failures} 个操作未完成` : ''}${r.started_at ? ' · ' + listTime(r.started_at) : ''}`)}"><span class="visually-hidden">${esc(runStates[r.state] || r.state)}</span></button>`).join('')}</div><p class="run-strip-legend"><span class="completed">完成</span><span class="partial">部分未完成</span><span class="failed">失败</span><span class="running">执行中</span></p>` : '<p class="agent-empty">最近还没有执行记录。</p>'}</section>
      <section class="agent-section"><h3>常在的会话</h3>${chatList(3)}</section>`;
  }

  function chatList(limit) {
    if (!chats) return '<p class="agent-empty">正在读取…</p>';
    if (chats.error) return `<p class="agent-empty">${esc(chats.error)}</p>`;
    const list = limit ? chats.list.slice(0, limit) : chats.list;
    if (!list.length) return '<p class="agent-empty">它还没有加入任何会话。</p>';
    return `<div class="agent-chats">${list.map(({ conversation: c, last_message, member_count, viewer }) => {
      const shared = chats.shared.has(c.id);
      return `<button type="button" data-detail-chat="${esc(c.id)}" data-viewer="${esc(viewer)}" data-shared="${shared}"><span class="avatar${c.kind === 'group' ? ' group' : ''}" data-tone="${toneOf(c.id)}" aria-hidden="true">${esc(initialOf(chatName(c)))}</span><span><strong>${esc(chatName(c))}</strong><small>${esc(last_message ? `${last_message.sender_id === viewer ? '它' : nameOf(last_message.sender_id)}：${last_message.body.replace(/\s+/g, ' ').replace(/[*`#>_~]/g, '')}` : `${c.kind === 'group' ? '群聊' : '私聊'} · ${member_count} 位参与者`)}</small></span><span class="agent-chat-side"><time>${esc(listTime(last_message?.created_at))}</time><em>${shared ? '我也在' : '只读'}</em></span></button>`;
    }).join('')}</div>`;
  }

  function runList() {
    const list = runs();
    if (!list.length) return '<p class="agent-empty">最近 50 次执行中没有它的记录。</p>';
    const names = new Map((chats?.list || []).map(e => [e.conversation.id, chatName(e.conversation)]));
    return `<div class="agent-runs">${list.map(r => `<button type="button" data-detail-run="${esc(r.id)}"><span class="run-dot ${esc(r.state)}"></span><span><strong>${esc(runStates[r.state] || r.state)}${r.tool_failures ? ` · ${r.tool_failures} 个操作未完成` : ''}</strong><small>${esc([names.get(r.conversation) || (r.conversation ? '其他会话' : '非会话触发'), r.model_steps ? `${r.model_steps} 次模型调用` : '', duration(r)].filter(Boolean).join(' · '))}</small></span><time>${esc(r.started_at ? listTime(r.started_at) : '')}</time></button>`).join('')}</div><p class="agent-empty">只显示全局最近 50 次执行中属于它的部分。点击查看调用记录。</p>`;
  }

  function abilities(a) {
    const chip = (kind, value) => `<button type="button" class="ability-chip" data-resource-read="${esc(ref(value))}" data-resource-type="${kind}">${esc(ref(value))}</button>`;
    const row = (title, body) => `<div class="ability-row"><dt>${title}</dt><dd>${body}</dd></div>`;
    const binding = bindings();
    return `<dl class="abilities">
      ${row('模型', a.harness ? chip('harness', a.harness) : a.model ? chip('model', a.model) : '<span class="agent-empty">未配置</span>')}
      ${row('Prompt', a.prompt ? chip('prompt', a.prompt) : '<span class="agent-empty">默认</span>')}
      ${row('Skills', a.skills?.length ? a.skills.map(s => chip('skill', s)).join('') : '<span class="agent-empty">无</span>')}
      ${row('MCP', a.mcps?.length ? a.mcps.map(m => chip('mcp', m)).join('') : '<span class="agent-empty">无</span>')}
      ${row('内置工具', `<ul class="builtin-list">${Object.entries(builtins).map(([key, [title, hint]]) => `<li class="${a.builtin_tools?.includes(key) ? 'on' : ''}"><strong>${title}</strong><small>${hint}</small></li>`).join('')}<li class="on"><strong>Memory</strong><small>查询历史、保存带来源的主题摘要（可在运行设置关闭）</small></li></ul>`)}
      ${row('聊天身份', binding.length ? binding.map(b => `<div class="binding-line"><strong>${esc(nameOf(b.principal))}</strong><small>${esc(policies[b.policy] || b.policy)}${b.enabled === false ? ' · 已停用' : ''}</small></div>`).join('') : '<span class="agent-empty">尚未绑定聊天身份，启用后才会出现在会话中。</span>')}
    </dl><div class="abilities-actions"><button type="button" class="secondary" data-bind="${esc(reference)}">运行设置</button><button type="button" class="secondary" data-resource-read="${esc(reference)}" data-resource-type="agent">查看配置文件</button></div>`;
  }

  function render() {
    const a = agent();
    if (!a) { close(); return; }
    const body = { overview, chats: () => `<section class="agent-section">${chatList()}</section>`, runs: runList, abilities: () => abilities(a) }[tab]();
    const scroll = sheet.querySelector('.agent-sheet-body')?.scrollTop || 0, outer = sheet.scrollTop;
    // Re-rendering replaces every node; remember which control had focus by its data attributes.
    const focused = sheet.contains(document.activeElement) ? [...document.activeElement.attributes].filter(x => x.name.startsWith('data-')).map(x => `[${x.name}="${CSS.escape(x.value)}"]`).join('') : '';
    sheet.dataset.tone = toneOf(principals()[0] || reference);
    sheet.classList.toggle('working', principals().some(workingState));
    sheet.style.setProperty('--spin-phase', `-${Date.now() % 2400}ms`);
    sheet.innerHTML = `${hero(a)}<nav class="agent-tabs" role="tablist">${tabs.map(([key, label]) => `<button type="button" role="tab" aria-selected="${key === tab}" data-detail-tab="${key}">${label}${key === 'chats' && chats?.list.length ? `<small>${chats.list.length}</small>` : key === 'runs' && runs().length ? `<small>${runs().length}</small>` : ''}</button>`).join('')}</nav><div class="agent-sheet-body" role="tabpanel">${body}</div>`;
    sheet.querySelector('.agent-sheet-body').scrollTop = scroll; sheet.scrollTop = outer;
    bind();
    if (focused) sheet.querySelector(focused)?.focus({ preventScroll: true });
  }

  function bind() {
    const on = (selector, handler) => sheet.querySelectorAll(selector).forEach(el => el.onclick = () => handler(el));
    on('[data-detail-close]', close);
    on('[data-detail-tab]', el => { tab = el.dataset.detailTab; render(); sheet.querySelector(`[data-detail-tab=${tab}]`).focus(); });
    on('[data-detail-direct]', el => { close(); startDirect(el.dataset.detailDirect); });
    on('[data-detail-observe]', el => { close(); observeAs(el.dataset.detailObserve); });
    on('[data-detail-edit]', () => showAgentEdit(reference));
    on('[data-detail-toggle]', el => changeAgentEnabled([reference], el.dataset.detailToggle === 'true'));
    on('[data-detail-run]', el => showCalls(el.dataset.detailRun));
    on('[data-detail-chat]', el => {
      close();
      if (el.dataset.shared === 'true') { switchPage('chat'); selectConversation(el.dataset.detailChat); }
      else observeAs(el.dataset.viewer, el.dataset.detailChat);
    });
    on('[data-bind]', el => showBinding(el.dataset.bind));
    bindResourceActions(sheet);
    sheet.querySelector('[role=tablist]').onkeydown = event => {
      if (!['ArrowLeft', 'ArrowRight'].includes(event.key)) return;
      const index = tabs.findIndex(([key]) => key === tab), next = tabs[(index + (event.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length][0];
      tab = next; render(); sheet.querySelector(`[data-detail-tab=${next}]`).focus();
    };
  }

  function open(agentRef, initialTab = 'overview') {
    if (!items('agent').some(a => ref(a) === agentRef)) return;
    reference = agentRef; tab = initialTab;
    render();
    if (!sheet.open) sheet.showModal();
    sheet.querySelector('[data-detail-close]').focus();
    loadChats();
  }
  function close() { if (sheet.open) sheet.close(); }
  sheet.addEventListener('click', event => { if (event.target === sheet) close(); });
  sheet.addEventListener('close', () => { request++; reference = null; });

  return {
    open, close,
    // Activity polling and config edits re-render in place; the open tab and scroll position are kept.
    refresh() { if (sheet.open && reference) render(); },
  };
})();
