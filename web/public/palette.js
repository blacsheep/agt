/* Command palette (Ctrl/⌘+K): one keyboard entry to every page, conversation, contact and action. */
window.AgtPalette = (() => {
  const dialog = document.querySelector('#palette'), input = document.querySelector('#palette-input'), list = document.querySelector('#palette-list');
  let entries = [], index = 0;
  const pages = [['chat', '会话空间', '聊天、群聊与私聊'], ['agents', 'Agent 团队', '创建、编辑和启停 Agent'], ['observer', 'Agent 会话', '只读查看 Agent 的对话'], ['interactions', '交互观察', '协作网络与消息流'], ['resources', '资源库', '模型、Prompt、Skills、MCP'], ['settings', '运行设置', '调度、权限、记忆与本机执行']];
  const icon = { page: '◧', chat: '#', direct: '@', action: '›', agent: '✦' };

  function collect() {
    const out = [];
    const human = state.viewer === state.boot?.principal;
    const add = (group, kind, label, hint, run, extra = {}) => out.push({ group, kind, label, hint, run, keys: `${label} ${hint}`.toLowerCase(), ...extra });
    if (state.conversation && state.page === 'chat') {
      const c = state.conversation;
      if (human) add('当前会话', 'action', '查看协作任务', conversationName(c), () => $('#chat-tasks').click());
      add('当前会话', 'action', '会话控制与命令', '/help /model /status …', () => $('#chat-commands').click());
      if (c.kind === 'group') add('当前会话', 'action', '群信息与成员', conversationName(c), () => $('#group-info').click());
    }
    for (const { conversation: c, last_message, member_count } of state.conversations) {
      add('会话', 'chat', conversationName(c), last_message ? previewText(last_message, c, member_count) : `${c.kind === 'group' ? '群聊' : '私聊'} · ${member_count} 位参与者`,
        () => { switchPage('chat'); selectConversation(c.id); }, { tone: toneOf(c.id), initial: initialOf(conversationName(c)), square: c.kind === 'group' });
    }
    if (human) for (const p of state.boot.principals.filter(p => p.id !== state.boot.principal && canContact(p))) {
      add('联系人', 'direct', `与 ${p.name || p.id} 私聊`, p.kind === 'agent' ? `Agent · ${p.id}` : p.id, () => startDirect(p.id), { tone: toneOf(p.id), initial: initialOf(p.name || p.id) });
    }
    for (const [page, label, hint] of pages) add('页面', 'page', label, hint, () => switchPage(page));
    if (human) {
      add('操作', 'action', '新建会话', '群聊或私聊', () => { switchPage('chat'); $('#new-chat').click(); });
      add('操作', 'action', '创建 Agent', '新的角色配置', () => { switchPage('agents'); $('#new-agent').click(); });
      add('操作', 'action', '新建资源', 'Provider、模型、Prompt…', () => { switchPage('resources'); $('#new-resource').click(); });
    }
    const theme = AgtTheme.get(), next = { system: 'light', light: 'dark', dark: 'system' }[theme];
    const names = { system: '跟随系统', light: '浅色', dark: '深色' };
    add('操作', 'action', `外观：切换为${names[next]}`, `当前 ${names[theme]}`, () => { AgtTheme.set(next); $('#theme-select').value = next; });
    add('操作', 'action', $('#debug-mode').checked ? '关闭调试模式' : '打开调试模式', '显示调度与调用细节', () => $('#debug-mode').click());
    return out;
  }

  // Subsequence match; earlier and contiguous hits rank higher.
  function score(entry, query) {
    if (!query) return 1;
    let at = -1, total = 0, run = 0;
    for (const ch of query) {
      const next = entry.keys.indexOf(ch, at + 1);
      if (next < 0) return 0;
      run = next === at + 1 ? run + 1 : 0;
      total += 10 - Math.min(next, 9) + run * 3;
      at = next;
    }
    return total + (entry.label.toLowerCase().startsWith(query) ? 50 : 0);
  }

  function render() {
    const query = input.value.trim().toLowerCase();
    const groups = ['当前会话', '会话', '联系人', '页面', '操作'];
    entries = collect().map(e => ({ ...e, score: score(e, query) })).filter(e => e.score > 0);
    entries.sort((a, b) => query ? b.score - a.score : groups.indexOf(a.group) - groups.indexOf(b.group));
    if (!query) entries = groups.flatMap(g => entries.filter(e => e.group === g).slice(0, g === '会话' ? 6 : 8));
    else entries = entries.slice(0, 30);
    index = Math.min(index, Math.max(0, entries.length - 1));
    let last = '';
    list.innerHTML = entries.map((e, i) => {
      const head = !query && e.group !== last ? `<li class="palette-group" role="presentation">${esc(e.group)}</li>` : '';
      last = e.group;
      const mark = e.initial ? `<span class="avatar${e.square ? ' group' : ''}" data-tone="${e.tone}" aria-hidden="true">${esc(e.initial)}</span>` : `<span class="palette-icon" aria-hidden="true">${icon[e.kind]}</span>`;
      return `${head}<li role="option" id="palette-option-${i}" data-index="${i}" aria-selected="${i === index}">${mark}<span><strong>${esc(e.label)}</strong><small>${esc(e.hint)}</small></span>${query ? `<em>${esc(e.group)}</em>` : ''}</li>`;
    }).join('') || '<li class="palette-empty" role="presentation">没有匹配的结果</li>';
    input.setAttribute('aria-activedescendant', entries.length ? `palette-option-${index}` : '');
    list.querySelectorAll('[data-index]').forEach(li => {
      li.onmousemove = () => { if (index !== Number(li.dataset.index)) { index = Number(li.dataset.index); highlight(); } };
      li.onclick = () => choose(Number(li.dataset.index));
    });
  }
  function highlight() {
    list.querySelectorAll('[data-index]').forEach(li => li.setAttribute('aria-selected', String(Number(li.dataset.index) === index)));
    const current = document.getElementById(`palette-option-${index}`);
    current?.scrollIntoView({ block: 'nearest' });
    input.setAttribute('aria-activedescendant', current ? current.id : '');
  }
  function choose(i) {
    const entry = entries[i];
    if (!entry) return;
    close();
    entry.run();
  }
  function open() {
    if (dialog.open || choosingIdentity) return;
    input.value = ''; index = 0; render();
    dialog.showModal(); input.focus();
  }
  function close() { if (dialog.open) dialog.close(); }

  input.addEventListener('input', () => { index = 0; render(); });
  input.addEventListener('keydown', event => {
    if (event.isComposing) return;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      if (entries.length) { index = (index + (event.key === 'ArrowDown' ? 1 : -1) + entries.length) % entries.length; highlight(); }
    } else if (event.key === 'Enter') { event.preventDefault(); choose(index); }
  });
  dialog.addEventListener('click', event => { if (event.target === dialog) close(); });
  document.addEventListener('keydown', event => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k' && !event.isComposing) {
      if (dialog.open) { event.preventDefault(); close(); return; }
      if (document.querySelector('dialog[open]')) return;
      event.preventDefault(); open();
    }
  });
  document.querySelector('#open-palette')?.addEventListener('click', open);
  return { open, close };
})();
