const $ = selector => document.querySelector(selector);
const $$ = selector => [...document.querySelectorAll(selector)];
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const labels = { provider: 'Provider', model: '模型', prompt: 'Prompt', skill: 'Skills', mcp: 'MCP', harness: 'Harness', agent: 'Agent' };
const stateNames = { completed: '已完成', running: '执行中', failed: '失败', canceled: '已取消', succeeded: '成功', started: '结果待确认', idle: '等待执行', skipped: '已跳过', pending: '待执行', processing: '处理中' };
let token = location.hash.slice(1) || sessionStorage.getItem('agt-token');
if (location.hash) { sessionStorage.setItem('agt-token', token); history.replaceState(null, '', location.pathname); }
const state = { boot: null, viewer: null, conversation: null, conversations: [], members: [], messages: [], contextResets: [], cursor: null, resourceKind: 'model', runs: [], scheduler: { online: false, discussions: [], tasks: [] }, polling: false, sending: false, pendingSend: null, modalBusy: false, generation: 0, page: 'chat' };
const directNames = new Map();
let debugMode = localStorage.getItem('agt-debug-mode') === 'true';
let choosingIdentity = false, selectedObserver = null;
let messageRequest = 0, conversationRequest = 0, modalGeneration = 0, commandGeneration = 0;
let commandTarget = null;

const layoutPreferences = {
  nav: localStorage.getItem('agt-layout-nav') !== 'false',
  conversations: localStorage.getItem('agt-layout-conversations') === 'true',
  activity: localStorage.getItem('agt-layout-activity') === 'true',
};
function renderLayout() {
  for (const [name, collapsed] of Object.entries(layoutPreferences)) {
    document.body.classList.toggle(`${name}-collapsed`, collapsed);
    const button = $(`#toggle-${name}`);
    const label = { nav: '导航', conversations: '会话列表', activity: '成员状态' }[name];
    button.setAttribute('aria-expanded', String(!collapsed));
    button.setAttribute('aria-label', `${collapsed ? '展开' : '收起'}${label}`);
    button.title = `${collapsed ? '展开' : '收起'}${label}`;
  }
}
for (const name of Object.keys(layoutPreferences)) {
  $(`#toggle-${name}`).onclick = () => {
    if (name === 'conversations') document.body.classList.remove('mobile-chat-open');
    layoutPreferences[name] = !layoutPreferences[name];
    localStorage.setItem(`agt-layout-${name}`, String(layoutPreferences[name]));
    renderLayout();
  };
}
renderLayout();

async function api(path, data) {
  const response = await fetch(`/api/${path}`, { method: data === undefined ? 'GET' : 'POST', headers: { 'X-AGT-Token': token || '', ...(data === undefined ? {} : { 'Content-Type': 'application/json' }) }, ...(data === undefined ? {} : { body: JSON.stringify(data) }) });
  const value = await response.json();
  if (!response.ok) { const error = new Error(value.error?.message || '请求失败'); error.code = value.error?.code; error.field = value.error?.field; throw error; }
  return value;
}
const tool = (name, args = {}) => api('tool', { name, args });
const read = (name, args = {}, viewer = state.viewer) => api('read', { name, args, viewer });
const items = kind => { const resource = state.boot?.resources[kind]; return Array.isArray(resource) ? resource : resource?.items || []; };
const ref = item => `${item.category || 'default'}/${item.skill || item.name || item.package || ''}`;
const principalOf = id => state.boot?.principals.find(p => p.id === id);
const nameOf = id => { const p = principalOf(id); return `${p?.name || id}${p?.deleted ? '（已删除）' : ''}`; };
const canContact = p => !p.deleted && (p.kind !== 'agent' || state.boot.bindings.some(b => b.principal === p.id && items('agent').some(a => ref(a) === b.agent)));
const dateTime = value => new Date(value).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
const empty = (title, description) => `<div class="empty compact"><h2>${esc(title)}</h2><p>${esc(description)}</p></div>`;
function showNotice(id, message, { error = false, action = null } = {}) {
  const notice = $(id);
  notice.dismiss?.();
  notice.listeners?.abort();
  const listeners = new AbortController(); notice.listeners = listeners;
  let timer, closed = false;
  const dismiss = () => {
    if (closed || notice.dismiss !== dismiss) return;
    closed = true; clearTimeout(timer);
    if (notice.contains(document.activeElement)) $('#chat-commands').focus();
    notice.classList.add('hidden'); notice.replaceChildren();
  };
  const pause = () => clearTimeout(timer);
  const resume = () => {
    pause();
    if (!closed && !error && !notice.matches(':hover') && !notice.contains(document.activeElement)) timer = setTimeout(() => { if (notice.matches(':hover') || notice.contains(document.activeElement)) return; dismiss(); }, action ? 10000 : 5000);
  };
  notice.dismiss = dismiss;
  notice.className = `notice${error ? ' error' : ''}`;
  const text = document.createElement('span'); text.className = 'notice-text'; text.textContent = message;
  const content = document.createElement('div'); content.className = 'notice-content'; content.append(text);
  if (action) content.append(action(dismiss));
  const close = document.createElement('button'); close.type = 'button'; close.className = 'notice-close'; close.textContent = '×';
  close.setAttribute('aria-label', '关闭提示'); close.title = '关闭提示'; close.onclick = dismiss;
  notice.append(content, close);
  notice.onmouseenter = pause; notice.onmouseleave = resume;
  notice.addEventListener('focusin', pause, { signal: listeners.signal });
  notice.addEventListener('focusout', () => queueMicrotask(resume), { signal: listeners.signal });
  notice.onkeydown = event => { if (event.key === 'Escape') { event.stopPropagation(); dismiss(); } };
  resume();
}
function notify(message, error = false) { showNotice('#notice', message, { error }); }
function errorText(error) {
  if (error.field === 'data.api_key_env') return '密钥传递失败，请重新输入 API Key 后重试。';
  return debugMode ? `${error.message}${error.field ? ` · 字段：${error.field}` : ''}${error.code ? ` [${error.code}]` : ''}` : error.message; }
function modalError(error) { $('#modal-error').textContent = errorText(error); $('#modal-error').classList.remove('hidden'); }
function options(values, selected = '') { return values.map(([value, label]) => `<option value="${esc(value)}" ${value === selected ? 'selected' : ''}>${esc(label)}</option>`).join(''); }
function field(label, content, hint = '') { return `<label class="field"><span>${esc(label)}</span>${content}${hint ? `<small>${esc(hint)}</small>` : ''}</label>`; }
function checks(values, name, selected = []) { return `<div class="check-list">${values.map(([value, label]) => `<label><input type="checkbox" name="${esc(name)}" value="${esc(value)}" ${selected.includes(value) ? 'checked' : ''}>${esc(label)}</label>`).join('') || '<small>暂无可用资源</small>'}</div>`; }
function dialog(title, content) { modalGeneration++; state.modalBusy = false; $('#modal-title').textContent = title; $('#modal-body').innerHTML = content; $('#modal-error').classList.add('hidden'); if (!$('#modal').open) $('#modal').showModal(); }
function closeModal() { if (!state.modalBusy) $('#modal').close(); }
function formHandler(handler) {
  $('#modal-form').addEventListener('submit', async event => {
    event.preventDefault(); if (state.modalBusy) return;
    const form = event.currentTarget; const button = form.querySelector('[type=submit]');
    state.modalBusy = true; button.disabled = true; $('#modal-error').classList.add('hidden');
    try { await handler(new FormData(form)); }
    catch (error) { modalError(error); }
    finally { state.modalBusy = false; button.disabled = false; }
  });
}
function currentEntry() { return state.conversations.find(x => x.conversation.id === state.conversation?.id); }
function isObserver() { return state.page === 'observer' || state.viewer !== state.boot?.principal; }
function canParticipate() { return !isObserver() && state.conversation?.state === 'active' && currentEntry()?.left_message_seq == null; }
function setupReadOnly() {
  ensureEvents();
  $('#chat-commands').disabled = !canParticipate();
  const observing = isObserver(), left = currentEntry()?.left_message_seq != null, dissolved = currentEntry()?.dissolved;
  $('#observe-banner').textContent = observing ? `只读查看 · ${nameOf(state.viewer)}${left ? ' · 已退出群聊，仅显示可见历史' : ''}` : dissolved ? '群聊已解散 · 历史记录只读' : left ? '你已退出群聊 · 仅可查看退出前有权访问的历史' : '会话已归档 · 历史记录只读';
  $('#observe-banner').classList.toggle('hidden', !state.conversation || !observing && !left && state.conversation.state === 'active');
  $('#composer').classList.toggle('hidden', !canParticipate());
  renderReplyStatus();
}
async function bootstrap() {
  const boot = await api('bootstrap'); state.boot = boot; directNames.clear();
  if (!state.viewer || !boot.principals.some(p => p.id === state.viewer && !p.deleted)) state.viewer = boot.principal;
  renderIdentities();
  $('#home-path').textContent = boot.home; $('#home-path').title = boot.home;
  $('#operator-name').textContent = nameOf(boot.principal);
  $('#agent-count').textContent = items('agent').length;
  renderAgents(); renderResources(); setupReadOnly();
}
function renderIdentities() {
  if (!state.boot) return;
  const observing = state.page === 'observer';
  $('#identity-directory').classList.toggle('hidden', !observing || !choosingIdentity);
  $('#observer-workspace').classList.toggle('hidden', choosingIdentity);
  $('#change-identity').classList.toggle('hidden', !observing || choosingIdentity);
  $('#new-chat').classList.toggle('hidden', observing);
  $('#observer-caption').textContent = choosingIdentity ? '选择 Agent，只读查看它有权访问的对话。' : `当前身份：${nameOf(state.viewer)} · 只读，不代替 Agent 发言`;
  const query = $('#agent-search').value.toLowerCase();
  $('#identities').innerHTML = state.boot.principals.filter(p => !p.deleted && p.kind === 'agent' && `${p.name} ${p.id}`.toLowerCase().includes(query)).map(p => `<button class="identity-button" data-identity="${esc(p.id)}" ${state.sending ? 'disabled' : ''}><span class="avatar">${esc((p.name || p.id).slice(0, 1))}</span><span><strong>${esc(p.name || p.id)}</strong><small>${esc(p.id)} · ${canContact(p) ? '查看会话 ↗' : '未绑定 · 查看历史 ↗'}</small></span></button>`).join('') || empty('暂无匹配的 Agent', '在 Agent 团队中创建角色并配置聊天身份。');
  $('#selected-identity').innerHTML = `<strong>${esc(nameOf(state.viewer))}</strong><small>${observing ? 'Agent 会话 · 只读' : '我的会话'}</small>`;
  $$('[data-identity]').forEach(button => button.onclick = async () => {
    if (state.sending) return;
    state.viewer = button.dataset.identity; selectedObserver = state.viewer; choosingIdentity = false;
    state.conversations = []; $('#search').value = ''; clearConversation(); renderIdentities(); renderConversations();
    try { await conversations(); await activity(); } catch (e) { notify(errorText(e), true); }
  });
}
async function conversations() {
  const request = ++conversationRequest;
  const viewer = state.viewer;
  if (state.page === 'observer' && choosingIdentity) return;
  const scope = $('#conversation-scope').value;
  const list = await read('conversation_list', {include_hidden: scope !== 'recent', groups_only: scope === 'groups'});
  if (state.viewer !== viewer || request !== conversationRequest) return;
  await Promise.all(list.filter(x => x.conversation.kind === 'direct' && !directNames.has(`${viewer}:${x.conversation.id}`)).slice(0, 20).map(async x => {
    const members = await read('member_list', { conversation: x.conversation.id }, viewer);
    directNames.set(`${viewer}:${x.conversation.id}`, members.filter(m => m.principal_id !== viewer && m.left_message_seq === null).map(m => nameOf(m.principal_id)).join('、'));
  }));
  if (state.viewer !== viewer || request !== conversationRequest) return;
  state.conversations = list;
  if (state.conversation && !list.some(x => x.conversation.id === state.conversation.id)) clearConversation();
  else if (state.conversation) { state.conversation = list.find(x => x.conversation.id === state.conversation.id).conversation; renderChatHead(); setupReadOnly(); }
  renderConversations();
}
function conversationName(c) {
  return c.title || directNames.get(`${state.viewer}:${c.id}`) || `私聊 · ${c.id.slice(0, 8)}`;
}
let entryOrganizer = {viewer:null, active:false, selected:new Set()};
function organizeEntries(action) {
  const agent = state.viewer, ids = [...entryOrganizer.selected];
  if (!ids.length || !state.boot.principals.some(p => p.id === agent && p.kind === 'agent') || state.page !== 'observer') return;
  const verb = action === 'hide' ? '移除' : '恢复';
  dialog(`${verb} Agent 的会话入口？`, `<form id="modal-form"><p>为「${esc(nameOf(agent))}」${verb} ${ids.length} 个会话入口。</p><p class="form-note">聊天记录、成员关系和上下文保留，Agent 任务不会停止。移除后可从“已移除”恢复；仍在会话中的 Agent 收到新消息时，入口会自动重新出现。</p><div class="form-actions"><button type="submit" class="primary">确认${verb}</button></div></form>`);
  formHandler(async () => {
    const result = await api('agent-entries',{agent,action,conversations:ids,confirm:true});
    const failed = result.results.filter(r=>!r.ok);
    if (state.viewer === agent) {
      entryOrganizer.selected = new Set(failed.map(r=>r.conversation));
      state.generation++; conversationRequest++;
    }
    $('#modal').close();
    notify(`${verb}成功 ${result.results.length-failed.length} 个${failed.length ? `，失败 ${failed.length} 个，请刷新检查后重试。` : '。'}`, !!failed.length);
    await conversations();
  });
}
$('#organize-entries').onclick = () => {entryOrganizer.active=!entryOrganizer.active;entryOrganizer.selected.clear();renderConversations();};
$('#select-entries').onchange = event => {entryOrganizer.selected = new Set(event.target.checked ? $$('#conversations [data-entry]').map(b=>b.dataset.entry).slice(0,100) : []);renderConversations();};
$('#hide-entries').onclick = () => organizeEntries('hide');
$('#restore-entries').onclick = () => organizeEntries('show');
function recentConversationEntries(entries) {
  if ($('#conversation-scope').value === 'hidden') return entries.filter(e=>e.hidden);
  if ($('#conversation-scope').value !== 'recent') return entries;
  const dissolved = entries.filter(e => e.dissolved && e.conversation.kind === 'group').sort((a,b) =>
    (b.last_viewed_at ?? b.conversation.updated_at ?? 0) - (a.last_viewed_at ?? a.conversation.updated_at ?? 0)
    || a.conversation.id.localeCompare(b.conversation.id));
  const retained = new Set(dissolved.slice(0,20).map(e => e.conversation.id));
  return entries.filter(e => !e.dissolved || e.conversation.kind !== 'group' || retained.has(e.conversation.id));
}
function renderConversations() {
  const focusedEntry = document.activeElement?.dataset.entry;
  $('#clear-search').classList.toggle('hidden', !$('#search').value);
  const query = $('#search').value.trim().toLowerCase();
  const entries = recentConversationEntries(state.conversations);
  const list = entries.filter(x => conversationName(x.conversation).toLowerCase().includes(query));
  $('#conversation-count').textContent = entries.length;
  const omitted = $('#conversation-scope').value === 'recent' ? state.conversations.length - entries.length : 0;
  $('#history-window').classList.toggle('hidden', !omitted);
  $('#history-window-text').textContent = `已解散群聊仅展示最近使用的 20 个，另 ${omitted} 个可在全部历史找回。`;
  if (entryOrganizer.viewer !== state.viewer) entryOrganizer = {viewer:state.viewer,active:false,selected:new Set()};
  const canOrganize = state.page === 'observer' && state.boot.principals.some(p=>p.id === state.viewer && p.kind === 'agent');
  const organizing = canOrganize && entryOrganizer.active;
  const available = new Set(list.map(e=>e.conversation.id));
  entryOrganizer.selected = new Set([...entryOrganizer.selected].filter(id=>available.has(id)));
  $('#entry-organizer').classList.toggle('hidden',!canOrganize);
  $('#entry-selection').classList.toggle('hidden',!organizing);
  $('#organize-entries').textContent = organizing ? '完成整理' : '整理会话';
  $('#organize-entries').setAttribute('aria-expanded',String(organizing));
  $('#entry-selected-count').textContent = `已选 ${entryOrganizer.selected.size} 个 · 每次最多 100 个`;
  $('#select-entries').checked = !!list.length && entryOrganizer.selected.size === Math.min(100,list.length);
  $('#select-entries').indeterminate = entryOrganizer.selected.size > 0 && !$('#select-entries').checked;
  $('#hide-entries').disabled = !entryOrganizer.selected.size;
  $('#restore-entries').disabled = !entryOrganizer.selected.size;
  const people = query && !isObserver() ? state.boot.principals.filter(p => p.id !== state.boot.principal && canContact(p) && `${p.name || ''} ${p.id}`.toLowerCase().includes(query)) : [];
  const contacts = people.length ? `<div class="search-section">联系人 · ${people.length}</div>${people.map(p => `<button class="contact-result" data-contact="${esc(p.id)}" ${state.sending ? 'disabled' : ''}><span class="avatar contact-avatar">${esc([...(p.name || p.id)][0])}</span><span><strong>${esc(p.name || p.id)}</strong><small>${p.kind === 'agent' ? 'Agent' : '用户'} · ${esc(p.id)}</small></span><span class="contact-action">私聊 ↗</span></button>`).join('')}` : '';
  $('#conversations').innerHTML = contacts + (query && list.length ? '<div class="search-section">当前列表的会话</div>' : '') + (list.map(({ conversation: c, member_count, left_message_seq, dissolved, hidden }) => `<div class="conversation-row">${organizing ? `<input class="entry-check" type="checkbox" data-entry="${esc(c.id)}" aria-label="选择 ${esc(conversationName(c))}" ${entryOrganizer.selected.has(c.id) ? 'checked' : ''}>` : ''}<button class="conversation-item ${state.conversation?.id === c.id ? 'selected' : ''}" data-conv="${esc(c.id)}"><span class="avatar">${c.kind === 'group' ? '▧' : '↗'}</span><span><strong>${esc(conversationName(c))}</strong><small>${c.kind === 'group' ? '群聊' : '私聊'} · ${member_count} 位参与者${dissolved ? ' · 已解散' : left_message_seq != null ? ' · 已退出' : c.state !== 'active' ? ' · 已归档' : hidden ? ' · 已移除入口' : ''}</small></span></button><button class="conversation-menu-button" data-menu="${esc(c.id)}" aria-label="${esc(conversationName(c))} 的会话操作" aria-haspopup="menu" ${state.sending ? 'disabled' : ''}>⋯</button></div>`).join('') || (contacts ? '' : empty(query ? '没有匹配结果' : '暂无会话', query ? '搜索用户、Agent 名称或会话名称' : '搜索联系人，开始一次对话')));
  $$('[data-conv]').forEach(button => button.onclick = () => selectConversation(button.dataset.conv).catch(e => notify(errorText(e), true)));
  $$('#conversations [data-entry]').forEach(input => input.onchange = () => { if(input.checked && entryOrganizer.selected.size < 100) entryOrganizer.selected.add(input.dataset.entry); else entryOrganizer.selected.delete(input.dataset.entry); renderConversations(); });
  if (focusedEntry) $$('#conversations [data-entry]').find(input=>input.dataset.entry === focusedEntry)?.focus({preventScroll:true});
  $$('[data-contact]').forEach(button => button.onclick = () => openContact(button.dataset.contact));
  $$('#conversations [data-menu]').forEach(button => button.onclick = event => { const rect = button.getBoundingClientRect(); showConversationMenu(button.dataset.menu, rect.left, rect.bottom + 4, event.currentTarget); });
  $$('#conversations [data-conv]').forEach(button => {
    button.oncontextmenu = event => { event.preventDefault(); showConversationMenu(button.dataset.conv, event.clientX, event.clientY, button); };
    button.onkeydown = event => { if (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey) { event.preventDefault(); const rect = button.getBoundingClientRect(); showConversationMenu(button.dataset.conv, rect.left, rect.bottom, button); } };
  });
}
async function openContact(principal) {
  if (state.sending || isObserver()) return;
  state.sending = true; renderConversations();
  let conversation;
  try {
    conversation = await tool('direct_create', {principal});
    if (conversation.state === 'active') await api('conversation', {action:'show',conversation:conversation.id});
  } catch (error) { notify(errorText(error), true); return; }
  finally { state.sending = false; renderConversations(); }
  if (!conversation) return;
  if (conversation.state !== 'active') { showRestoreChat(conversation); return; }
  $('#search').value = ''; $('#conversation-scope').value = 'recent';
  try { await conversations(); await selectConversation(conversation.id); }
  catch (error) { notify(errorText(error), true); }
}
let menuTarget = null;
function closeConversationMenu(restoreFocus = false) {
  const menu = $('#conversation-menu'); menu.classList.add('hidden');
  if (restoreFocus && menuTarget) {
    const target = menuTarget.node.isConnected ? menuTarget.node : $$('#conversations [data-conv]').find(b => b.dataset.conv === menuTarget.id);
    target?.focus({preventScroll:true});
  }
  menuTarget = null;
}
function showConversationMenu(id, x, y, trigger) {
  if (state.sending) return;
  const entry = state.conversations.find(e => e.conversation.id === id); if (!entry) return;
  const menu = $('#conversation-menu'); menuTarget = {id, node:trigger};
  menu.innerHTML = `<div class="context-menu-title">${esc(conversationName(entry.conversation))}</div><button role="menuitem" data-action="open">打开会话<span>↗</span></button><button role="menuitem" data-action="info">${entry.conversation.kind === 'group' ? '群信息' : '会话信息'}<span>ⓘ</span></button>${!isObserver() ? '<div class="context-menu-divider"></div><button role="menuitem" data-action="hide">从列表移除<span>−</span></button>' : ''}`;
  menu.classList.remove('hidden');
  menu.style.left = Math.max(8, Math.min(x, innerWidth - menu.offsetWidth - 8)) + 'px';
  menu.style.top = Math.max(8, Math.min(y, innerHeight - menu.offsetHeight - 8)) + 'px';
  menu.querySelector('[role=menuitem]').focus({preventScroll:true});
  menu.querySelectorAll('[data-action]').forEach(button => button.onclick = async () => {
    const action = button.dataset.action, viewer = state.viewer, page = state.page;
    closeConversationMenu(true);
    if (state.sending) return;
    if (action === 'hide') { showDeleteChat(entry.conversation); return; }
    try {
      await selectConversation(id);
      if (action === 'info' && state.conversation?.id === id && state.viewer === viewer && state.page === page) showGroupInfo();
    } catch (error) { notify(errorText(error), true); }
  });
}
$('#conversation-menu').onkeydown = event => {
  const buttons = [...$('#conversation-menu').querySelectorAll('[role=menuitem]')];
  const index = buttons.indexOf(document.activeElement);
  if (event.key === 'Escape') { event.preventDefault(); closeConversationMenu(true); }
  else if (event.key === 'Tab') closeConversationMenu(true);
  else if (['ArrowDown','ArrowUp','Home','End'].includes(event.key)) {
    event.preventDefault();
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length-1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
    buttons[next].focus();
  }
};
document.addEventListener('pointerdown', event => { if (!event.target.closest('#conversation-menu')) closeConversationMenu(); });
window.addEventListener('resize', () => closeConversationMenu());
document.addEventListener('scroll', event => { if (!event.target.closest?.('#conversation-menu')) closeConversationMenu(); }, true);
function clearConversation() { closeCommandPanel(); document.body.classList.remove('mobile-chat-open'); $('#latest-message').classList.add('hidden'); closeConversationMenu(); $('#group-drawer').close(); state.conversation = null; state.members = []; state.messages = []; state.contextResets = []; state.cursor = null; state.generation++; renderLiveReplies(); renderMessages(); renderChatHead(); setupReadOnly(); }
async function selectConversation(id) {
  closeCommandPanel();
  if (state.sending) { notify('消息正在发送，请稍后切换会话。'); return; }
  const viewer = state.viewer, page = state.page, generation = ++state.generation;
  const entry = state.conversations.find(x => x.conversation.id === id);
  if (entry?.hidden && !isObserver()) await api('conversation', {action:'show',conversation:id});
  if (generation !== state.generation || viewer !== state.viewer || page !== state.page) return;
  state.conversation = entry?.conversation;
  if (!state.conversation) return;
  document.body.classList.toggle('mobile-chat-open', matchMedia('(max-width: 760px)').matches);
  $('#latest-message').classList.add('hidden');
  state.messages = []; state.contextResets = []; state.members = []; state.cursor = null;
  state.runs = state.runs.map(r => ({...r, progress: null, tool_activity: null})); renderLiveReplies();
  $('#message-body').value = ''; state.pendingSend = null; renderMentions();
  renderConversations(); renderChatHead(); setupReadOnly();
  $('#messages').innerHTML = empty('正在读取…', '消息保存在本地');
  const members = await read('member_list', { conversation: id }, viewer);
  if (generation !== state.generation) return;
  state.members = members;
  renderChatHead(); renderMentions(); setupReadOnly(); await messages(true);
  if (generation !== state.generation) return;
  if (entry.dissolved && !isObserver()) {
    try {
      const visit = await api('conversation', {action:'visit',conversation:id});
      if (generation !== state.generation) return;
      // Polling may have replaced the list while history was loading.
      const current = state.conversations.find(e => e.conversation.id === id);
      if (current) current.last_viewed_at = visit.last_viewed_at;
      conversationRequest++; renderConversations();
    } catch (e) { if (generation === state.generation) notify('历史已打开，但最近查看顺序未保存：' + errorText(e), true); }
  }
  if (generation === state.generation) await activity();
}
function renderChatHead() {
  const c = state.conversation;
  if (!c) { $('#chat-head').innerHTML = '<strong>选择一个会话</strong>'; $('#group-info').disabled = true; renderReplyStatus(); return; }
  const title = c.kind === 'direct' && state.members.length ? state.members.map(m => nameOf(m.principal_id)).join(' ↔ ') : conversationName(c);
  const entry = currentEntry();
  const status = entry?.dissolved ? '已解散' : entry?.left_message_seq != null ? '已退出' : c.kind === 'group' ? '群聊' : '私聊';
  $('#chat-head').innerHTML = `<strong>${esc(title)}</strong><small>${esc(status)}${state.members.length ? ' · ' + state.members.filter(m => m.left_message_seq == null).map(m => esc(nameOf(m.principal_id))).join(' · ') : ''}</small>`;
  $('#group-info').disabled = false;
  $('#group-info').textContent = c.kind === 'group' ? '群信息' : '会话信息';
  renderReplyStatus();
}
function showDeleteChat(c) {
  dialog('从会话列表移除？', `<form id="modal-form"><p>仅移除你自己的「${esc(conversationName(c))}」入口。不会退出群聊、删除消息或停止 Agent。</p><p class="form-note">仍在群内时，新消息会让会话重新出现；也可从“我的群聊”或“全部历史”重新打开。</p><div class="form-actions"><button id="cancel-delete" class="quiet" type="button">取消</button><button class="primary" type="submit">移除入口</button></div></form>`);
  $('#cancel-delete').onclick = closeModal;
  formHandler(async () => {
    await api('conversation', { action: 'hide', conversation: c.id });
    $('#modal').close(); if (state.conversation?.id === c.id) clearConversation();
    await conversations();
    showNotice('#deleted-notice', '已移除个人会话入口，群聊与历史记录保留。', { action: dismiss => {
      const undo = document.createElement('button'); undo.type = 'button'; undo.className = 'quiet'; undo.textContent = '撤销移除';
      undo.onclick = async () => { undo.disabled = true; try { await api('conversation', {action:'show',conversation:c.id}); dismiss(); await conversations(); await selectConversation(c.id); } catch(e) {undo.disabled=false;notify(errorText(e),true);} };
      return undo;
    } });
  });
}
function canResetContext(c) {
  return c && !isObserver() && currentEntry()?.left_message_seq == null && c.state === 'active'
    && (c.kind === 'direct' || c.owner_id === state.boot.principal || state.members.some(m => m.principal_id === state.boot.principal && m.left_message_seq == null && ['owner', 'admin'].includes(m.role)));
}
function showResetContext(c, fromComposer = false) {
  if (state.sending) return;
  if (!canResetContext(c)) { notify('私聊参与者或群主、管理员才能重置上下文。', true); return; }
  closeMentions(); $('#group-drawer').close();
  dialog('从新的上下文开始？', '<form id="modal-form"><p>停止涉及本会话的 Agent 工作，清除本会话所有 Agent 的会话摘要。下一条新消息将作为新的上下文起点。</p><p class="form-note">聊天记录保留，Agent 仍可通过历史查询工具主动查阅旧消息。已经发出的外部工具操作不会撤销。</p><div class="form-actions"><button class="primary" type="submit">停止并重置上下文</button></div></form>');
  formHandler(async () => {
    await api('conversation', {action:'reset', conversation:c.id, confirm:true});
    if(state.conversation?.id===c.id) await messages(true);
    $('#modal').close();
    if (state.conversation?.id === c.id) {
      state.generation++;
      state.runs = state.runs.map(r => r.conversation === c.id ? {...r, progress:null,tool_activity:null} : r); renderLiveReplies();
      if (fromComposer && ['new','clear'].includes(AgtCommands.parse($('#message-body').value)?.name)) {
        $('#message-body').value = ''; state.pendingSend = null; renderMentions(); $('#message-body').focus();
      }
    }
    notify('上下文已重置，聊天记录保留。发送新消息开始讨论。');
    await activity().catch(() => notify('上下文已重置，但状态同步失败，请刷新页面。', true));
  });
}
function showGroupInfo() {
  const c = state.conversation; if (!c) return;
  const entry = currentEntry(), observer = isObserver(), left = entry?.left_message_seq != null;
  const owns = c.owner_id === state.boot.principal;
  const canReset = canResetContext(c);
  $('#group-title').textContent = conversationName(c);
  $('#group-content').innerHTML = `<p class="form-note">${entry?.dissolved ? '群聊已解散，历史记录保留。' : left ? '已退出，仅保留过去可见的消息。' : c.kind === 'group' ? '群成员与个人会话入口分别管理。' : '管理自己的会话入口。'}</p><div class="group-members">${state.members.map(m => `<div class="group-member"><span class="avatar">${esc([...nameOf(m.principal_id)][0])}</span><span><strong>${esc(nameOf(m.principal_id))}</strong><small>${m.left_message_seq != null ? '已退出' : ({owner:'群主',admin:'管理员',member:'成员'}[m.role] || '成员')}</small></span></div>`).join('')}</div>${observer ? '<p class="form-note">只读观察。请回到“会话空间”管理你自己的群聊。</p>' : `<div class="group-controls"><button id="hide-entry" class="secondary">从会话列表移除</button>${c.kind === 'group' && c.state === 'active' && !left ? owns ? '<p class="form-note">你是群主。负责人群需保留负责人；如不再需要协作，可解散群聊。</p><button id="dissolve-group" class="danger-button">解散群聊</button>' : '<button id="leave-group" class="danger-button">退出群聊，保留历史</button>' : ''}</div>`}`;
  if (canReset) {
    $('#group-content').insertAdjacentHTML('beforeend', '<section class="context-controls" aria-label="模型上下文"><div><strong>模型上下文</strong><p class="form-note">长对话会自动生成摘要。需要换个话题时，可让本会话的 Agent 从新的起点开始。</p></div><button id="reset-context" class="secondary">重置上下文…</button></section>');
    $('#reset-context').onclick = () => showResetContext(c);
  }
  for (const [id, action] of [['leave-group', 'leave'], ['dissolve-group', 'dissolve']]) {
    const button = $('#' + id); if (button) button.onclick = () => { $('#group-drawer').close(); confirmGroupAction(c, action); };
  }
  if ($('#hide-entry')) $('#hide-entry').onclick = () => { $('#group-drawer').close(); showDeleteChat(c); };
  $('#group-drawer').showModal();
}
function confirmGroupAction(c, action) {
  const leaving = action === 'leave';
  dialog(leaving ? '退出群聊？' : '解散群聊？', `<form id="modal-form"><p>${leaving ? '你将不再接收新消息或参与讨论，其他成员和 Agent 会继续协作。' : '所有成员将不能继续发消息，本群后续 Agent 回复与等待将停止；此操作不能撤销。'}</p><p class="form-note">会话入口和有权查看的历史记录保留。${leaving ? '' : '已经发出的外部请求可能仍会完成。'}</p><div class="form-actions"><button type="submit" class="danger-button">${leaving ? '确认退出' : '确认解散'}</button></div></form>`);
  formHandler(async () => {
    await api('conversation', {action,conversation:c.id,confirm:true}); $('#modal').close();
    $('#conversation-scope').value = 'recent'; await conversations(); await selectConversation(c.id);
  });
}
async function restoreChat(id) {
  if (state.sending) throw new Error('消息正在发送，请稍后恢复会话。');
  await api('conversation', { action: 'reactivate', conversation: id });
  switchPage('chat'); clearConversation();
  await conversations(); await selectConversation(id);
}
function showRestoreChat(c) {
  dialog('恢复已有私聊', `<form id="modal-form"><p>与这位参与者的私聊已删除或归档。恢复后可继续聊天，之前的消息会保留。</p><div class="form-actions"><button class="primary" type="submit">恢复并打开</button></div></form>`);
  formHandler(async () => { await restoreChat(c.id); $('#modal').close(); });
}
function discussionTasks(id) { return state.scheduler.tasks.filter(t => t.conversation === id); }
function agentStates(tasks) {
  const rank = t => {
    const round = state.scheduler.discussions.find(d => d.id === t.discussion);
    if (round?.state === 'ended') return 2;
    return ['evaluating', 'responding', 'input'].includes(t.state) ? 0 : ['waiting', 'queued', 'ready_to_reply'].includes(t.state) ? 1 : 2;
  };
  const latest = new Map();
  for (const task of tasks) {
    const key = JSON.stringify([task.conversation, task.principal]);
    const previous = latest.get(key);
    if (!previous || rank(task) < rank(previous)) latest.set(key, task);
  }
  return [...latest.values()];
}
const decisionReasons = { relevant_expertise: '有相关信息可以补充', correction_needed: '需要纠正已有信息', direct_question: '有待回答的问题', already_answered: '问题已有回答', not_relevant: '当前话题无需参与', observe_discussion: '等待其他成员补充' };
const decisionActions = { participate: '参与发言', silent: '本次不发言', wait: '等待观察' };
function participationView(t) {
  const round = state.scheduler.discussions.find(d => d.id === t.discussion);
  const active = ['queued', 'evaluating', 'ready_to_reply', 'responding', 'input', 'waiting', 'blocked', 'limited'].includes(t.state);
  if (!state.scheduler.online && active) return { label: '状态待确认', tone: 'attention', source: '服务连接', detail: '回复服务未连接，以下为最近保存的状态。' };
  if (active && ['paused', 'ended', 'limited'].includes(round?.state)) {
    const running = ['evaluating', 'responding', 'input'].includes(t.state);
    if (round.state === 'ended') return {label:'正在停止',tone:'attention',source:'会话控制',detail:'停止已请求，等待执行器确认。'};
    const expired = round.expires_at <= Date.now();
    if (running) return {label:'继续执行中',tone:'active',source:'框架调度',detail:round.state === 'paused' ? '当前执行可完成；后续任务等待恢复调度。' : '当前执行可完成；后续任务等待授权续期。'};
    return {label:round.state === 'paused' ? '已暂停' : expired ? '等待授权续期' : '本轮已达上限',tone:'muted',source:'会话控制',detail:round.state === 'paused' ? '恢复回复后继续调度。' : expired ? '讨论授权已到期，任务仍保留在队列。' : '本轮回复额度已用完。'};
  }
  const views = {
    queued: ['等待调度', 'muted', '框架调度', '输入已排队，等待可用处理机会。'],
    evaluating: [t.last_decision ? '重新判断中' : '参与判断中', 'active', '模型判断', '正在判断是否需要参与本次讨论。'],
    ready_to_reply: ['等待发言机会', 'active', '框架调度', '参与判断已完成，等待回复机会。'],
    input: ['已接入当前执行', 'active', '输入队列', '此消息已由当前执行接纳，等待处理结果。'],
    responding: ['正在回复', 'active', '执行中', '正在生成回复或执行工具。'],
    waiting: ['观察中', 'waiting', t.last_decision ? '模型自主' : '等待安排', t.quiet ? '讨论安静后重新判断；有新消息时可能顺延。' : `预计 ${dateTime(t.due_at)} 后重新判断。`],
    silent: [t.last_decision?.action === 'silent' ? '本次不发言' : '无追加回复', 'muted', t.last_decision?.action === 'silent' ? '模型自主' : '处理结果', t.last_decision?.action === 'silent' ? '模型判断本次无需参与。' : '本次处理没有追加公开回复；可能已通过工具发送消息。'],
    skipped: ['无需处理', 'muted', '框架规则', '响应策略或消息状态不要求回复。'],
    completed: ['已完成回复', 'done', '处理结果', '本次处理已结束，可在聊天中查看回复。'],
    failed: ['处理失败', 'attention', '执行异常', '本次处理未完成，请检查配置或连接；调试模式可查看错误。'],
    outcome_unknown: ['结果待核对', 'attention', '执行异常', '无法确认执行结果，请核对原始执行和文件改动。'],
    interrupted: ['回复已中断', 'attention', '执行异常', '回复中断，请查看运行记录。'],
    blocked: ['需要工具授权', 'attention', '框架规则', '外部工具尚未获得所需的执行授权。'],
    limited: ['本轮已达上限', 'attention', '框架规则', '已达到回复上限，发送新消息可重置。'],
    canceled: ['已取消', 'muted', '处理结果', '本次任务已取消。'],
  };
  const [label, tone, source, detail] = views[t.state] || ['等待处理', 'muted', '状态记录', '暂无详细状态说明。'];
  return { label, tone, source, detail };
}
function presenceMarkup(t) {
  const view = participationView(t), decision = t.last_decision;
  const reason = decisionReasons[decision?.reason_code] || '未记录具体原因';
  const conversation = !state.conversation ? state.conversations.find(c => c.conversation.id === t.conversation)?.conversation : null;
  const preview = view.source === '模型自主' ? decision?.public_reason?.trim() || reason : view.detail;
  const summary = `<span class="presence-person"><span class="presence-avatar" aria-hidden="true">${esc([...nameOf(t.principal)][0])}</span><span><strong>${esc(nameOf(t.principal))}</strong>${conversation ? `<small>${esc(conversationName(conversation))}</small>` : ''}</span></span><span class="presence-state" data-tone="${view.tone}"><span class="presence-dot" aria-hidden="true"></span>${esc(view.label)}</span><span class="presence-preview">${esc(preview)}</span><span class="presence-footer"><span>${esc(view.source)}</span><span class="presence-toggle"><span class="when-closed">查看说明</span><span class="when-open">收起说明</span><span aria-hidden="true">⌄</span></span></span>`;
  const stale = decision && (['queued', 'evaluating'].includes(t.state) || decision.through_seq < t.through_seq || (state.conversation?.id === t.conversation && decision.through_seq < state.conversation.last_message_seq));
  const wake = decision?.wake;
  const wakeText = wake?.type === 'after' ? `${wake.seconds} 秒后重新判断` : wake?.type === 'conversation_quiet' ? `讨论安静 ${wake.quiet_seconds} 秒后重新判断，最长观察 ${wake.max_wait_seconds} 秒` : '';
  const body = `<div class="presence-section"><span class="presence-label">当前状态 · ${esc(view.source)}</span><p>${esc(view.detail)}</p></div><div class="presence-section"><span class="presence-label">最近模型决定</span>${decision ? `<div class="decision-heading"><strong>${esc(decisionActions[decision.action] || '已判断')}</strong><time>${esc(dateTime(decision.decided_at))}</time></div><p class="decision-category">${esc(reason)}</p>${decision.public_reason?.trim() ? `<blockquote>${esc(decision.public_reason)}</blockquote>` : '<p class="presence-note">未提供额外说明</p>'}${wakeText ? `<p class="presence-note">当时的等待条件：${esc(wakeText)}。</p>` : ''}${stale ? '<p class="presence-note">这是之前的判断；有新消息或正在重新评估，请以当前状态为准。</p>' : ''}` : '<p class="presence-note">暂无独立参与判断记录。直接回复可能跳过此判断；旧记录也未保存这些信息。</p>'}</div>${debugMode ? `<pre class="presence-debug">${esc(JSON.stringify({ state: t.state, reason: t.reason, task: t.id, through_seq: t.through_seq, decision_through_seq: decision?.through_seq, due_at: t.due_at, version: t.version }, null, 2))}</pre>` : ''}`;
  return { summary, body };
}
function renderPresence(tasks) {
  const box = $('#agent-presence');
  const rows = debugMode ? tasks : agentStates(tasks);
  const previous = new Map([...box.children].map(node => [node.dataset.key, node]));
  rows.forEach((task, index) => {
    const key = JSON.stringify([task.conversation, task.principal, debugMode ? task.id : '']);
    let node = previous.get(key);
    previous.delete(key);
    if (!node) {
      node = document.createElement('details'); node.className = 'presence-card'; node.dataset.key = key;
      node.innerHTML = '<summary></summary><div class="presence-body"></div>';
    }
    const content = presenceMarkup(task);
    const summary = node.querySelector('summary'), body = node.querySelector('.presence-body');
    if (node._summary !== content.summary) { summary.innerHTML = content.summary; node._summary = content.summary; }
    if (node._body !== content.body) { body.innerHTML = content.body; node._body = content.body; }
    if (box.children[index] !== node) {
      const focused = document.activeElement === summary;
      box.insertBefore(node, box.children[index] || null);
      if (focused) summary.focus({ preventScroll: true });
    }
  });
  previous.forEach(node => node.remove());
  if (!rows.length && !box.children.length) box.innerHTML = '<p class="presence-empty">暂无参与状态<br><span>发送消息后，在这里查看成员的回应与等待。</span></p>';
}
function participationSummary(tasks) {
  const groups = new Map();
  for (const task of tasks) {
    const label = participationView(task).label;
    groups.set(label, (groups.get(label) || 0) + 1);
  }
  return [...groups].map(([label, count]) => `${count} 位${label}`).join(' · ');
}
function renderReplyStatus() {
  const id = state.conversation?.id;
  const own = canParticipate();
  const tasks = agentStates(discussionTasks(id));
  const rounds = state.scheduler.discussions.filter(d => d.conversation === id);
  const running = tasks.some(t => ['evaluating', 'responding', 'input'].includes(t.state));
  const current = rounds[0];
  $('#execute').disabled = !id || !own || state.conversation.state !== 'active' || running;
  $('#reply-status').classList.toggle('hidden', !id || isObserver() || currentEntry()?.left_message_seq != null || (!current && !tasks.length && state.scheduler.online));
  $('#reply-status').classList.toggle('problem', !state.scheduler.online || tasks.some(t => ['failed', 'blocked', 'limited', 'interrupted', 'outcome_unknown'].includes(t.state)));
  const pending = tasks.filter(t => ['evaluating', 'responding', 'ready_to_reply', 'waiting', 'queued'].includes(t.state));
  $('#reply-detail').textContent = !state.scheduler.online ? '回复服务未连接 · 请重新启动服务；未完成的提醒会保留' : current?.state === 'paused' ? (running ? '正在暂停 · 当前回复结束后停止' : '已暂停 · 可点击「继续回复」') : current?.state === 'ended' ? '本轮讨论已结束 · 发送新消息可开始新一轮' : current?.state === 'limited' ? (running ? '当前执行继续 · 后续任务等待续期' : '等待授权或额度续期 · 可点击「继续回复」') : pending.length ? participationSummary(tasks) : tasks.some(t => ['failed', 'blocked', 'interrupted', 'outcome_unknown'].includes(t.state)) ? '部分回复未完成 · 请查看成员状态' : '本轮暂时没有新的发言';
  $('#pause-reply').classList.toggle('hidden', !own || !rounds.some(d => d.state === 'active'));
  $('#pause-reply').disabled = false;
  $('#end-reply').classList.toggle('hidden', isObserver());
  $('#execute').classList.toggle('hidden', isObserver());
  $('#end-reply').disabled = !own || !rounds.some(d => d.state !== 'ended') && !pending.length;
  $('#end-reply').textContent = current?.state === 'ended' && !pending.length ? '本轮已停止' : state.conversation?.kind === 'direct' ? '停止 Agent' : '停止本群 Agent';
  $$('#conversations [data-menu]').forEach(button => { button.disabled = state.sending; });
}
let mentionOptions = [], mentionIndex = 0;
function mentionMembers() {
  return state.members.filter(m => m.principal_id !== state.boot?.principal && !principalOf(m.principal_id)?.deleted && m.left_message_seq == null);
}
function mentionToken(id) {
  return id !== 'all' && /^[\p{L}\p{N}_-]+$/u.test(id) ? `@${id}` : `@[${JSON.stringify(id)}]`;
}
function mentionedPrincipals(body) {
  const members = mentionMembers();
  const tokens = new Set([...body.matchAll(/(?:^|[\s（(【\[])@([^\s，。！？、：；,!?;:()（）【】\[\]]+)/gu)].map(m => m[1]));
  const explicit = new Set();
  body.replace(/(?:^|[\s（(【\[])@\[("(?:[^"\\]|\\.)*")\]/gu, (_, id) => { try { explicit.add(JSON.parse(id)); } catch { /* Incomplete literal is plain text. */ } return ''; });
  if (tokens.has('all')) return members.map(m => m.principal_id);
  return members.filter(m => tokens.has(m.principal_id) || explicit.has(m.principal_id)).map(m => m.principal_id);
}
function mentionQuery() {
  const input = $('#message-body');
  if (input.selectionStart !== input.selectionEnd) return null;
  const before = input.value.slice(0, input.selectionStart);
  const match = before.match(/(?:^|[\s（(【\[])@([^\s@，。！？、：；,!?;:()（）【】\[\]]*)$/u);
  return match ? { start: before.lastIndexOf('@'), end: input.selectionStart, text: match[1] } : null;
}
function closeMentions() {
  $('#mention-menu').classList.add('hidden');
  $('#message-body').setAttribute('aria-expanded', 'false');
  $('#message-body').removeAttribute('aria-activedescendant');
  mentionOptions = [];
}
function renderMentions() {
  resizeComposer();
  closeMentions();
  const ids = mentionedPrincipals($('#message-body').value);
  $('#mention-summary').textContent = ids.length ? `已提及 ${ids.length} 位成员` : '输入 @ 选择成员 · @all 提及所有人';
}
function showMentionOptions() {
  renderMentions();
  const query = mentionQuery();
  if (!query || state.sending || state.viewer !== state.boot?.principal) return;
  const candidates = [{ id: 'all', all: true, name: '所有人', description: '当前会话的所有其他成员' }, ...mentionMembers().map(m => ({ id: m.principal_id, name: nameOf(m.principal_id), description: m.principal_id }))];
  mentionOptions = candidates.filter(m => `${m.id} ${m.name}`.toLowerCase().includes(query.text.toLowerCase()));
  if (!mentionOptions.length) return;
  mentionIndex = 0;
  $('#mention-menu').innerHTML = mentionOptions.map((m, i) => `<button type="button" role="option" id="mention-option-${i}" aria-selected="${i === 0}" data-mention-index="${i}" tabindex="-1"><span class="mention-symbol">@</span><span><strong>${esc(m.name)}</strong><small>${esc(m.description)}</small></span></button>`).join('');
  $('#mention-menu').classList.remove('hidden');
  $('#message-body').setAttribute('aria-expanded', 'true');
  $('#message-body').setAttribute('aria-activedescendant', 'mention-option-0');
  $$('#mention-menu button').forEach(b => { b.onmousedown = e => e.preventDefault(); b.onclick = () => insertMention(Number(b.dataset.mentionIndex)); });
}
function insertMention(index) {
  const query = mentionQuery(), option = mentionOptions[index];
  if (!query || !option) return;
  const input = $('#message-body');
  input.setRangeText(`${option.all ? '@all' : mentionToken(option.id)} `, query.start, query.end, 'end');
  input.focus(); renderMentions();
}
async function messages(force = false, older = false) {
  const c = state.conversation; if (!c) return;
  const request = ++messageRequest;
  const generation = state.generation;
  const page = await read('message_history', { conversation: c.id, limit: 50, ...(older ? { before: state.cursor } : {}) });
  if (generation !== state.generation || request !== messageRequest) return;
  const previousResets=JSON.stringify(state.contextResets);
  const resetMap=new Map((older || state.messages.length>50 ? state.contextResets : []).map(r=>[r.id,r]));
  (page.context_resets || []).forEach(r=>resetMap.set(r.id,r));
  state.contextResets=[...resetMap.values()].sort((a,b)=>a.through_seq-b.through_seq || a.id-b.id);
  const oldIds = state.messages.map(m => `${m.id}:${m.deleted_at}`).join('|');
  if (older) { state.messages = [...page.messages, ...state.messages.filter(m => !page.messages.some(n => n.id === m.id))]; state.cursor = page.next_cursor; }
  else if (state.messages.length > 50) {
    const known = new Map(state.messages.map(m => [m.id, m])); page.messages.forEach(m => known.set(m.id, m));
    state.messages = [...known.values()].sort((a, b) => a.seq - b.seq);
  } else { state.messages = page.messages; state.cursor = page.next_cursor; }
  const box = $('#messages'); const nearBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 100;
  if (force || older || previousResets !== JSON.stringify(state.contextResets) || oldIds !== state.messages.map(m => `${m.id}:${m.deleted_at}`).join('|')) {
    const previousHeight = box.scrollHeight; const top = box.scrollTop;
    renderMessages();
    if (older) box.scrollTop = box.scrollHeight - previousHeight + top;
    else if (nearBottom || force) box.scrollTop = box.scrollHeight;
  }
  $('#older').classList.toggle('hidden', !state.cursor);
  updateLatestMessage();
}
function renderMessages() {
  if (!state.conversation) { $('#messages').innerHTML = empty('给协作一个起点', '新建私聊或群聊，邀请你的 Agent 团队。'); return; }
  const timeline=[...state.messages.map(m=>({seq:m.seq, message:m})), ...(state.contextResets || []).map(r=>({seq:r.through_seq,reset:r}))].sort((a,b)=>a.seq-b.seq || (a.reset?1:0)-(b.reset?1:0) || (a.reset?.id||0)-(b.reset?.id||0));
  $('#messages').innerHTML = timeline.map(entry => {
    if(entry.reset) return `<div class="context-divider" role="separator" aria-label="上下文已重置"><span><strong>新上下文</strong><time>${esc(new Date(entry.reset.created_at).toLocaleString())}</time><small>从这里重新开始 · 聊天记录保留</small></span></div>`;
    const m=entry.message;
    const own = m.sender_id === state.boot.principal;
    return `<article class="message ${own ? 'own' : ''}"><div class="avatar ${own ? 'user' : ''}">${esc(own ? '你' : nameOf(m.sender_id).slice(0, 1).toUpperCase())}</div><div class="message-content"><div class="message-meta"><span>${esc(nameOf(m.sender_id))}</span><time>${dateTime(m.created_at)}</time></div><div class="bubble">${esc(m.deleted_at ? '此消息已删除' : m.body)}</div><div class="message-tools"><span>${(m.metadata?.mentions || []).map(p => `@${esc(nameOf(p))}`).join(' ')}</span><button data-status="${esc(m.id)}">${debugMode ? '查看投递 ↗' : '回复情况'}</button></div></div></article>`;
  }).join('') || empty('会话已就绪', '发送第一条消息，邀请 Agent 回应。');
  $$('[data-status]').forEach(button => button.onclick = () => showStatus(button.dataset.status));
}
async function showStatus(id) {
  dialog(debugMode ? '消息投递' : '回复情况', '<p class="form-note">正在查看各位成员的回复情况…</p>');
  const generation = modalGeneration;
  try {
    const rows = await read('message_status', { message_id: id });
    if (generation !== modalGeneration || !$('#modal').open) return;
    $('#modal-body').innerHTML = rows.map(r => `<div class="call-row"><strong>${esc(nameOf(r.principal))}</strong> · ${esc(debugMode ? (stateNames[r.state] || r.state) : ({ completed: '已处理', failed: '暂时无法回复', skipped: '本条消息无需回复', pending: '等待回复', processing: '正在处理' }[r.state] || '等待处理'))}${r.error ? `<p class="form-note">${esc(debugMode ? r.error : friendlyFailure(r.error, r.state))}</p>` : ''}${r.run_id ? `<button class="quiet" data-run="${esc(r.run_id)}">${debugMode ? '查看运行 ↗' : '查看说明'}</button>` : ''}</div>`).join('') || '<p>这条消息没有需要回复的成员。</p>';
    $$('[data-run]').forEach(b => b.onclick = () => showCalls(b.dataset.run));
  } catch (e) { if (generation === modalGeneration && $('#modal').open) modalError(e); }
}
function friendlyFailure(error, status) {
  if (status === 'skipped') return '可能没有被邀请回复，或当前响应策略不需要回复。';
  if (/configuration|cannot load|supports.*only/i.test(error || '')) return '模型或角色配置还未就绪，请检查 Agent 团队中的模型和运行设置，然后发送新消息。';
  if (/401|403/.test(error || '')) return '模型服务未授权，请检查 Provider 的密钥和访问权限。';
  if (/429/.test(error || '')) return '模型服务繁忙或额度不足，请稍后再试并检查额度。';
  if (/timeout|timed out|HTTP request/i.test(error || '')) return '暂时连接不到模型服务，请检查网络和服务地址，稍后发送新消息。';
  return '请检查模型与角色设置；也可开启调试模式查看详细原因。失败消息不会自动重跑。';
}
async function activity() {
  const generation = state.generation;
  try {
    const query = state.conversation ? '?' + new URLSearchParams({conversation: state.conversation.id, viewer: state.viewer || state.boot.principal}) : '';
    const data = await api('activity' + query); if (generation !== state.generation) return;
    state.runs = data.runs; state.scheduler = data.scheduler; window.AgtInteractions?.statusChanged(); renderReplyStatus(); renderActivity(); window.AgtTasks?.refresh();
  } catch (error) {
    if (generation === state.generation) { state.runs=state.runs.map(r=>({...r,progress:null,tool_activity:null})); state.scheduler.online = false; window.AgtInteractions?.statusChanged(); renderReplyStatus(); renderActivity(); }
    throw error;
  }
}
function runLabel(r) {
  return ({ completed: r.tool_failures ? '回复结束，部分操作未完成' : '本次回复已结束', running: '正在准备回复', failed: '暂时无法回复', canceled: '回复已取消' })[r.state] || '等待处理';
}
function activityTitle(a) { return AgtToolActivity.title(a); }
function activityTimeline(activities) { return AgtToolActivity.timeline(activities); }
function renderLiveReplies() {
  const viewer=state.viewer, conversation=state.conversation?.id;
  AgtExecutionView.render({container:$('#live-replies'),runs:state.runs,conversation:state.conversation?.id,
    viewer:state.viewer,online:state.scheduler.online,
    visible:state.conversation?.state==='active' && currentEntry()?.left_message_seq==null,
    nameOf,onApproval:id=>AgtApprovals.open(id),
    loadToolDetail:(id,activity)=>api('tool-detail',{id,activity,viewer,conversation})});
}
function renderActivity() {
  renderLiveReplies();
  $('#activity-title').textContent = debugMode ? '运行观察' : '成员状态';
  $('#activity-hint').textContent = debugMode ? '点击查看调用' : '点击查看回复状态';
  const tasks = state.conversation ? discussionTasks(state.conversation.id) : state.scheduler.tasks.filter(t => state.conversations.some(c => c.conversation.id === t.conversation));
  const rounds = state.scheduler.discussions.filter(d => state.conversation ? d.conversation === state.conversation.id : state.conversations.some(c => c.conversation.id === d.conversation));
  $('#scheduler-state').textContent = state.scheduler.online ? '调度服务在线' : '调度服务离线';
  $('.execution-top').classList.toggle('offline', !state.scheduler.online);
  $('#scheduler-detail').textContent = debugMode ? rounds.slice(0, 3).map(d => d.state + ' · 模型调用 ' + d.calls + ' 次 · 每位 Agent 回复上限 ' + (d.max_replies ?? '不限')).join('；') || '尚未授权讨论轮次' : '当前进度与模型决定，分开查看。';
  renderPresence(tasks);
  const runs = debugMode ? state.runs : state.runs.filter(r => state.conversation ? r.conversation === state.conversation.id : state.conversations.some(c => c.conversation.id === r.conversation));
  $('#run-count').textContent = runs.length; $('#failure-count').textContent = runs.reduce((n,r) => n + r.tool_failures,0);
  $('#runs').innerHTML = runs.slice(0,15).map(r => '<button class="run-item" data-run="' + esc(r.id) + '"><span class="run-dot ' + esc(r.state) + '"></span><span><strong>' + esc(debugMode ? r.agent : r.agent.split('/').at(-1)) + '</strong><small>' + esc(debugMode ? (stateNames[r.state] + ' · ' + r.model_steps + ' 次模型调用 · ' + r.tool_failures + ' 次工具失败') : runLabel(r)) + '</small></span></button>').join('') || empty('还没有回复动态', '选择会话，发送一条消息开始聊天。');
  $$('#runs [data-run]').forEach(b => b.onclick = () => showCalls(b.dataset.run));
}
async function showCalls(id) {
  const run = state.runs.find(r => r.id === id);
  if (!debugMode) {
    if (!run) { dialog('回复说明', '<p class="run-summary">这条较早的记录不在最近动态中，可开启调试模式查看详情。</p>'); return; }
    const description = run.state === 'completed' ? (run.tool_failures ? 'Agent 已结束回复，但部分操作没有完成。请查看聊天内容确认结果。' : 'Agent 已完成本次处理。回复内容可在对应会话中查看；如果它选择不发言，聊天中不会新增消息。') : run.state === 'running' ? 'Agent 正在处理消息，完成后会显示在会话中。' : friendlyFailure(run.error);
    dialog('回复说明', `<div class="run-summary"><h3>${esc(run.agent.split('/').at(-1))}</h3><p>${esc(runLabel(run))}</p><p>${esc(description)}</p><div id="completed-tool-activity"></div></div>`);
    const generation = modalGeneration;
    let rows;
    try { rows = await api('calls', {id, after:0}); } catch(e) { if (generation === modalGeneration) modalError(e); return; }
    if (generation === modalGeneration && $('#completed-tool-activity')) $('#completed-tool-activity').innerHTML = activityTimeline(rows.filter(c=>c.tool_name).slice(-40).map(c=>({kind:c.kind==='framework'||c.tool_name.startsWith('framework_')?'framework':'mcp',tool_name:c.tool_name,label:c.public_label,summary:c.public_summary,status:({started:run.state==='running'?'running':'unknown',succeeded:'completed',failed:'failed',canceled:'canceled'})[c.state],started_at:c.started_at,updated_at:c.finished_at||c.started_at})));
    return;
  }
  dialog('调用记录', `<p class="form-note">${esc(id)}<br>仅保存参数形状与安全错误，不保存原始参数、模型推理或工具返回正文。</p><div id="calls-list"></div><button id="more-calls" class="quiet">加载调用</button>`);
  const generation = modalGeneration;
  let after = 0;
  async function load() {
    const button = $('#more-calls'); if (!button) return; button.disabled = true;
    try {
      const rows = await api('calls', { id, after });
      if (generation !== modalGeneration || !$('#modal').open) return;
      $('#calls-list').insertAdjacentHTML('beforeend', rows.map(r => `<details class="call-row"><summary>${esc(r.kind === 'model' ? '模型调用' : r.kind === 'compaction' ? '上下文压缩' : r.tool_name)} · ${esc(stateNames[r.state])} <small>${dateTime(r.started_at)}</small></summary>${r.error ? `<p class="error-text">${esc(r.error.code)} · ${esc(r.error.message)}${r.error.field ? ` · ${esc(r.error.field)}` : ''}</p>` : ''}<pre>${esc(JSON.stringify(r.argument_summary, null, 2))}</pre></details>`).join('') || '<p class="form-note">没有更多调用记录；旧版本 Run 不会补造日志。</p>');
      after = rows.at(-1)?.id || after; button.classList.toggle('hidden', rows.length < 100); button.textContent = '加载更多调用';
    } catch (e) { if (generation === modalGeneration && $('#modal').open) modalError(e); } finally { button.disabled = false; }
  }
  $('#more-calls').onclick = load; await load();
}
function showReplyControls() {
  const conversation = state.conversation?.id; if (!conversation) return;
  dialog('继续这个会话的回复', `<form id="modal-form"><p>处理当前会话尚未完成的消息，会调用模型并可能产生费用。</p><p class="form-note">消息独立排队；同会话的新消息会在工具执行完成或当前回答结束后继续处理。不同 Agent 可并行判断，自主接话依次开始；明确请求可并行回复。每位 Agent 的回复额度由全局运行设置决定，新的用户消息获准自动回复后计数归零；继续讨论不会重置计数。失败或中断的操作不会自动重跑。</p><div class="form-actions"><button class="primary" type="submit">开始回复</button></div></form>`);
  formHandler(async () => { await api('reply', { conversation, action: 'start', confirm: true }); $('#modal').close(); await activity(); });
}
function showNewChat() {
  const people = state.boot.principals.filter(p => p.id !== state.boot.principal && canContact(p));
  dialog('新建会话', `<form id="modal-form"><div class="form-note">以「${esc(nameOf(state.boot.principal))}」创建会话。仅列出已绑定现有资源的 Agent；已删除或未绑定身份不参与新会话。停用的 Agent 需启用后才能自动回应。</div>${field('会话类型', '<select name="kind"><option value="group">群聊 · 多方讨论</option><option value="direct">私聊 · 一对一</option></select>')}${field('群聊名称', '<input name="title" placeholder="例如：产品方案讨论" maxlength="120">')}<section class="participant-picker" aria-label="选择参与者"><label class="field"><span>参与者</span><input id="participant-search" type="search" placeholder="搜索名称或 ID" autocomplete="off"></label><div id="participant-selected" class="participant-selected"></div><div id="participant-count" class="participant-count" aria-live="polite"></div><div id="participant-results" class="participant-results"></div><div class="participant-pages"><button id="participant-prev" type="button" class="secondary">上一页</button><small id="participant-page"></small><button id="participant-next" type="button" class="secondary">下一页</button></div></section><div class="form-actions"><button class="primary" type="submit">创建会话</button></div></form>`);
  const kindSelect = $('#modal-form [name=kind]'), title = $('#modal-form [name=title]');
  const selected = new Set();
  let page = 0;
  const search = $('#participant-search');
  const renderPeople = () => {
    const query = search.value.trim().toLowerCase(), limit = query ? 10 : 8;
    const matches = people.filter(p => `${p.name || ''} ${p.id}`.toLowerCase().includes(query));
    const pages = Math.max(1, Math.ceil(matches.length / limit));
    page = Math.min(page, pages - 1);
    $('#participant-count').textContent = `${query ? '搜索结果' : '联系人'} · ${matches.length} 位 · 已选 ${selected.size} 位`;
    $('#participant-selected').innerHTML = [...selected].map(id => `<button type="button" class="participant-chip" data-remove-person="${esc(id)}" aria-label="移除 ${esc(nameOf(id))}">${esc(nameOf(id))}<span aria-hidden="true">×</span></button>`).join('');
    $('#participant-results').innerHTML = matches.slice(page * limit, (page + 1) * limit).map(p => `<label class="participant-row"><span class="participant-avatar" aria-hidden="true">${esc((p.name || p.id).slice(0, 1))}</span><span class="participant-identity"><strong>${esc(p.name || p.id)}</strong><small>${esc(p.id)}</small></span><input type="${kindSelect.value === 'direct' ? 'radio' : 'checkbox'}" name="candidate" value="${esc(p.id)}" ${selected.has(p.id) ? 'checked' : ''}></label>`).join('') || '<p class="participant-empty">没有匹配的联系人，试试其他名称或 ID。</p>';
    $('#participant-prev').disabled = page === 0;
    $('#participant-next').disabled = page + 1 >= pages;
    $('#participant-page').textContent = `${page + 1} / ${pages}`;
    $('.participant-pages').hidden = pages === 1;
  };
  search.oninput = () => { page = 0; renderPeople(); };
  $('#participant-prev').onclick = () => { page--; renderPeople(); };
  $('#participant-next').onclick = () => { page++; renderPeople(); };
  $('#participant-results').onchange = event => {
    const input = event.target;
    if (kindSelect.value === 'direct') selected.clear();
    if (input.checked) selected.add(input.value); else selected.delete(input.value);
    renderPeople();
    [...$('#participant-results').querySelectorAll('input')].find(item => item.value === input.value)?.focus();
  };
  $('#participant-selected').onclick = event => {
    const button = event.target.closest('[data-remove-person]');
    if (button) { selected.delete(button.dataset.removePerson); renderPeople(); search.focus(); }
  };
  const updateKind = () => {
    const group = kindSelect.value === 'group';
    if (!group && selected.size > 1) { const first = selected.values().next().value; selected.clear(); selected.add(first); }
    renderPeople();
    title.closest('.field').classList.toggle('hidden', !group);
    title.disabled = !group; title.required = group;
  };
  kindSelect.onchange = updateKind; updateKind();
  formHandler(async data => {
    const members = [...selected]; const kind = data.get('kind');
    if (!members.length || (kind === 'direct' && members.length !== 1)) throw new Error('请选择参与者；私聊只能选择一人。');
    if (kind === 'group' && !data.get('title').trim()) throw new Error('请填写群聊名称。');
    const c = await tool(kind === 'group' ? 'group_create' : 'direct_create', kind === 'group' ? { title: data.get('title').trim(), members } : { principal: members[0] });
    if (c.state !== 'active') { showRestoreChat(c); return; }
    switchPage('chat'); await api('conversation', {action:'show',conversation:c.id}); await conversations(); await selectConversation(c.id); $('#modal').close();
  });
}
const selectedAgents = new Set();
let agentBatchBusy = false;
function renderAgentSelection() {
  const agents = items('agent');
  for (const id of selectedAgents) if (!agents.some(a => ref(a) === id)) selectedAgents.delete(id);
  $('#agent-selection-count').textContent = `已选 ${selectedAgents.size} 位`;
  const all = $('#select-all-agents');
  all.checked = agents.length > 0 && selectedAgents.size === agents.length;
  all.indeterminate = selectedAgents.size > 0 && selectedAgents.size < agents.length;
  all.disabled = agentBatchBusy || !agents.length;
  for (const id of ['enable-agents','disable-agents']) $('#'+id).disabled = agentBatchBusy || !selectedAgents.size;
}
function changeAgentEnabled(references, enabled) {
  const names = [...references]; if (!names.length || agentBatchBusy) return;
  dialog(enabled ? '启用 Agent' : '停用 Agent', `<form id="modal-form"><p>${enabled ? '启用后会按响应策略处理消息，已有待处理消息可能触发模型调用。未配置的 Agent 使用默认运行设置：被提及时回应、执行额度遵循全局设置、不允许 MCP。' : '停止这些 Agent 在所有会话中的自动工作，并取消后续任务。身份、历史和运行设置保留；已发出的外部请求可能仍会完成。'}</p><p class="form-note">同一 Agent 的所有聊天身份都会${enabled ? '启用' : '停用'}。</p><ul>${names.map(n=>`<li>${esc(n)}</li>`).join('')}</ul><div class="form-actions"><button class="primary" type="submit">确认${enabled ? '启用' : '停用'} ${names.length} 位 Agent</button></div></form>`);
  formHandler(async () => {
    agentBatchBusy = true; renderAgents();
    const failures = []; let success = 0;
    try {
      for (const agent of names) {
        try {
          const bindings = state.boot.bindings.filter(b=>b.agent === agent);
          if (!bindings.length && enabled) {
            const principal = agent.split('/').at(-1);
            if (state.boot.principals.some(p=>p.id === principal)) throw new Error('同名身份已存在，请先在运行设置中确认身份，避免覆盖其他 Agent。');
            await ensurePrincipal(principal,principal);
            await tool('agent_bind',{principal,agent,working_directory:state.boot.home,policy:'mentioned'});
          } else for (const b of bindings) await tool('agent_set_enabled',{principal:b.principal,enabled});
          success++; selectedAgents.delete(agent);
        } catch(e) { failures.push(`${agent}：${errorText(e)}`); }
      }
      await bootstrap();
      $('#modal').close();
      if (failures.length) dialog('批量操作结果', `<p>成功 ${success} 位，未完成 ${failures.length} 位。成功的操作已生效；未完成项可能有部分身份已更新，请检查后重试。</p><ul>${failures.map(f=>`<li>${esc(f)}</li>`).join('')}</ul>`);
      else notify(`已${enabled ? '启用' : '停用'} ${success} 位 Agent。`);
    } finally { agentBatchBusy=false; renderAgents(); }
  });
}
function renderAgents() {
  $('#agents').innerHTML = items('agent').map(a => {
    const bindings = state.boot.bindings.filter(b => b.agent === ref(a));
    const binding = bindings[0], enabled = bindings.some(b => b.enabled !== false);
    const status = enabled ? bindings.some(b=>b.enabled === false) ? '部分启用' : '已启用' : binding ? '已停用' : '未启用';
    const model = a.harness ? `Codex · ${ref(a.harness)}` : a.model ? `${a.model.category || 'default'}/${a.model.name}` : '未配置模型';
    return `<article class="agent-card"><label class="agent-select"><input type="checkbox" data-select-agent="${esc(ref(a))}" ${selectedAgents.has(ref(a)) ? 'checked' : ''} ${agentBatchBusy ? 'disabled' : ''}> 选择 ${esc(a.name)}</label><div class="card-head"><div class="avatar">${esc(a.name.slice(0, 1).toUpperCase())}</div><div><h3>${esc(a.name)}</h3><small>${esc(model)}</small></div><span class="binding-indicator ${enabled ? 'bound' : ''}" title="${status}">${status}</span></div><p class="agent-description">${esc(a.description || '尚未填写角色说明。')}</p><div class="agent-capabilities"><span class="tag">${a.skills?.length || 0} Skills · ${a.mcps?.length || 0} MCP</span>${a.builtin_tools?.includes('resource_management') ? '<span class="tag good">内置资源管理</span>' : ''}</div><div class="card-actions"><button class="secondary" data-toggle-agent="${esc(ref(a))}" data-enable="${!enabled}" ${agentBatchBusy ? 'disabled' : ''}>${enabled ? '停用' : '启用'}</button><button class="secondary" data-edit-agent="${esc(ref(a))}">编辑角色</button>${binding ? `<button class="quiet" data-direct="${esc(binding.principal)}">发起私聊 ↗</button>` : ''}</div><details class="card-options"><summary>配置与管理</summary><div><button class="secondary" data-bind="${esc(ref(a))}">运行设置</button>${resourceActions('agent', a)}</div></details></article>`;
  }).join('') || empty('创建你的第一位 Agent', '先在资源库配置模型，再为 Agent 指定角色与能力。');
  renderAgentSelection();
  $$('[data-select-agent]').forEach(b=>b.onchange=()=>{ b.checked ? selectedAgents.add(b.dataset.selectAgent) : selectedAgents.delete(b.dataset.selectAgent); renderAgentSelection(); });
  $$('[data-toggle-agent]').forEach(b=>b.onclick=()=>changeAgentEnabled([b.dataset.toggleAgent],b.dataset.enable === 'true'));
  bindResourceActions($('#agents'));
  $$('[data-edit-agent]').forEach(b => b.onclick = () => showAgentEdit(b.dataset.editAgent));
  $$('[data-bind]').forEach(b => b.onclick = () => showBinding(b.dataset.bind));
  $$('[data-direct]').forEach(b => b.onclick = async () => { try { const c = await tool('direct_create', { principal: b.dataset.direct }); if (c.state !== 'active') { showRestoreChat(c); return; } switchPage('chat'); await api('conversation', {action:'show',conversation:c.id}); await conversations(); await selectConversation(c.id); } catch (e) { notify(errorText(e), true); } });
}
function builtinFields(selected = []) {
  const groups=[['resource_management','资源管理','创建和修改资源、绑定 Agent 并启用或停用。不自动获得本机命令权限。'],['task_collaboration','任务协作','委派可追踪任务、处理结果、安排依赖和并行策略。显式任务支持检查点等待。'],['local_execution','本机命令 · 原生 Agent','允许执行 Bash，另需在运行设置开启本机执行。命令具有服务账号权限，工作目录不是沙箱；Codex 使用自己的执行权限。']];
  return `<fieldset class="builtin-tools"><legend>框架内置工具</legend>${groups.map(([key,title,description])=>`<label><input type="checkbox" name="builtin_tools" value="${key}" ${selected.includes(key)?'checked':''}><span><strong>${title}</strong><small>${description}</small></span></label>`).join('')}<p class="form-note">Memory 默认可用：Agent 可查询自己的历史、保存带来源的主题摘要；全局运行设置可关闭。</p></fieldset>`;
}
async function showAgentEdit(reference) {
  dialog(`编辑 Agent · ${reference}`, '<p class="form-note">正在读取角色配置…</p>');
  const generation = modalGeneration;
  try {
    const agent = await api('resource', { action: 'read', kind: 'agent', reference });
    if (generation !== modalGeneration || !$('#modal').open) return;
    const choices = (kind, selected) => {
      const available = items(kind).map(item => [ref(item), ref(item)]);
      for (const value of selected) if (!available.some(([id]) => id === value)) available.push([value, `${value}（当前引用，列表不可用）`]);
      return available;
    };
    const prompt = ref(agent.prompt), mcps = agent.mcps.map(ref);
    $('#modal-body').innerHTML = `<form id="modal-form"><p class="form-note">修改会在下一次运行加载，已开始的运行继续使用原配置。聊天身份和响应策略请在“运行设置”中修改。</p>${field('资源名称', `<input value="${esc(reference)}" readonly>`)}${field('显示说明', `<textarea name="description">${esc(agent.description)}</textarea>`)}${executionFields(agent)}${field('Prompt 资源', `<select name="prompt" required>${options(choices('prompt', [prompt]), prompt)}</select>`, '这里切换角色引用的 Prompt，不修改共享正文。需要新指令时，可先在资源库新建 Prompt 再选择。')}${field('Skills', checks(choices('skill', agent.skills), 'skills', agent.skills))}${field('MCP', checks(choices('mcp', mcps), 'mcps', mcps))}${builtinFields(agent.builtin_tools)}<div class="form-actions"><button id="cancel-agent-edit" class="secondary" type="button">取消</button><button class="primary" type="submit">保存修改</button></div></form>`;
    bindExecutionFields();
    $('#cancel-agent-edit').onclick = closeModal;
    formHandler(async data => {
      await tool('resource_write', { kind: 'agent', reference, create: false, expected_revision: agent.revision, data: { description: data.get('description'), ...executionData(data), prompt: data.get('prompt'), skills: data.getAll('skills'), mcps: data.getAll('mcps'), builtin_tools: data.getAll('builtin_tools') } });
      $('#modal').close(); await bootstrap(); notify(`${reference} 已更新，将在下一次运行生效。`);
    });
  } catch (error) { if (generation === modalGeneration && $('#modal').open) { $('#modal-body').innerHTML = ''; modalError(error); } }
}
function bindingFields(binding = {}) {
  return `<p class="form-note">模型步数默认不限，可在全局运行设置中调整；可随时停止讨论或停用 Agent。</p><div class="form-grid">${field('响应策略', `<select name="policy">${options([['mentioned', '私聊 / 群内被提及'], ['all', '所有消息'], ['manual', '不自动回应']], binding.policy || 'mentioned')}</select>`)}</div><div class="form-grid">${field('执行工作目录', `<input name="cwd" value="${esc(binding.working_directory || state.boot.home)}" required>`)}</div>`;
}
const bindingData = (data, principal, agent) => ({ principal, agent, working_directory: data.get('cwd'), policy: data.get('policy') });
function showBinding(agent) {
  const binding = state.boot.bindings.find(b => b.agent === agent);
  dialog('运行设置', `<form id="modal-form"><p class="form-note">${esc(agent)} · 修改运行设置不会自动启用已停用的 Agent。修改影响后续执行，已开始的调用可能继续完成。</p>${field('Agent 身份 ID', `<input name="principal" value="${esc(binding?.principal || agent.split('/').at(-1))}" required ${binding ? 'readonly' : ''}>`, '身份不存在时会创建 Agent 身份；已有绑定会更新。')}${bindingFields(binding)}<div class="form-actions">${binding ? '<button id="unbind" type="button" class="quiet">解除运行配置</button>' : ''}<button class="primary" type="submit">保存设置</button></div></form>`);
  const definition = items('agent').find(a=>ref(a)===agent);
  if (definition?.harness) {
    const harness = items('harness').find(e=>ref(e)===ref(definition.harness));
    const cwd = $('#modal-form [name=cwd]'); cwd.readOnly=true; cwd.value=harness?.working_directory || state.boot.home;
    const policy = $('#modal-form [name=policy]'); policy.querySelector('[value=all]').disabled=true; if(policy.value==='all')policy.value='mentioned';
    cwd.closest('.field')?.insertAdjacentHTML('beforeend','<small>由Harness管理；外部 Agent 仅响应私聊和群内 @。</small>');
  }
  formHandler(async data => { const principal = data.get('principal').trim(); await ensurePrincipal(principal, principal); await tool('agent_bind', bindingData(data, principal, agent)); $('#modal').close(); await bootstrap(); notify('运行设置已保存。'); });
  if (binding) $('#unbind').onclick = async () => { if (!confirm('解除运行配置会停用此身份并移除运行设置，身份和消息保留。以后需重新配置才能启用。确认？')) return; try { await tool('agent_unbind', { principal: binding.principal }); $('#modal').close(); await bootstrap(); } catch (e) { modalError(e); } };
}
async function ensurePrincipal(id, name) {
  const exists = state.boot.principals.find(p => p.id === id);
  if (exists?.deleted) throw new Error('这个身份已删除，ID 保留用于历史记录。请使用新的身份 ID。');
  if (exists && exists.kind !== 'agent') throw new Error('这个身份已被非 Agent 参与者占用，请换一个 ID。');
  if (!exists) { await tool('principal_create', { id, name, kind: 'agent' }); state.boot.principals.push({ id, name, kind: 'agent' }); }
}
function showAgent() {
  dialog('创建一个新角色', `<form id="modal-form"><div class="form-note">只创建你命名的新资源。支持 Chat Completions 和 Responses，按模型配置调用对应接口。</div><div class="form-grid">${field('资源名称 / 身份 ID', '<input name="name" placeholder="如 researcher" pattern="[A-Za-z0-9][A-Za-z0-9_-]*" maxlength="60" required>')}${field('显示说明', '<input name="description" placeholder="如：负责事实核验与独立分析" maxlength="200">')}</div>${executionFields()}${field('角色 Prompt', '<textarea name="prompt" required placeholder="你负责什么？如何与同伴沟通？何时应当停止或保持沉默？"></textarea>')}${field('Skills（可选）', checks(items('skill').map(s => [ref(s), s.name || s.package]), 'skills'))}${field('MCP（可选）', checks(items('mcp').map(m => [ref(m), m.name]), 'mcps'))}${builtinFields()}${bindingFields()}<div id="setup-progress" class="progress"></div><div class="form-actions"><button class="primary" type="submit">创建并启用</button></div></form>`);
  bindExecutionFields();
  const done = new Set(); let lockedName = null;
  formHandler(async data => {
    const name = data.get('name').trim(); const prompt = `${name}-role`;
    if (lockedName && name !== lockedName) throw new Error('已有部分创建步骤完成。请保持名称重试，或关闭后创建另一个角色。');
    if (!lockedName && (items('agent').some(a => ref(a) === `default/${name}`) || items('prompt').some(p => ref(p) === `default/${prompt}`) || state.boot.principals.some(p => p.id === name))) throw new Error('资源或身份名称已存在。请换一个名称，原配置不会被覆盖。');
    lockedName = name;
    const step = async (key, action) => {
      if (!done.has(key)) { await action(); done.add(key); }
      const locked = ['name', ...(done.has('Prompt') ? ['prompt'] : []), ...(done.has('Agent') ? ['description', 'execution', 'harness', 'model', 'skills', 'mcps', 'builtin_tools'] : [])];
      // Preserve fields in FormData while preventing edits to already-created resources.
      $$('#modal-form input, #modal-form textarea, #modal-form select').forEach(input => {
        if (locked.includes(input.name)) {
          if (input.tagName === 'SELECT' || input.type === 'checkbox') {
            if (!input.disabled) {
              if (input.type !== 'checkbox' || input.checked) { const hidden = document.createElement('input'); hidden.type = 'hidden'; hidden.name = input.name; hidden.value = input.value; input.after(hidden); }
              input.disabled = true;
            }
          } else input.readOnly = true;
        }
      });
      $('#setup-progress').textContent = [...done].join(' → ') + ' ✓ · 已创建的资源字段已锁定；可修正后续步骤后重试。';
    };
    await step('Prompt', () => tool('resource_write', { kind: 'prompt', reference: prompt, create: true, data: { text: data.get('prompt') } }));
    await step('Agent', () => tool('resource_write', { kind: 'agent', reference: name, create: true, data: { ...executionData(data), prompt, description: data.get('description'), skills: data.getAll('skills'), mcps: data.getAll('mcps'), builtin_tools: data.getAll('builtin_tools') } }));
    await step('身份', () => ensurePrincipal(name, name));
    await step('启用', () => tool('agent_bind', bindingData(data, name, name)));
    $('#modal').close(); await bootstrap(); notify(`${name} 已就绪。新建会话邀请它，再发送任务。`);
  });
}
function resourceActions(kind, resource) {
  const reference = esc(ref(resource));
  return `${kind === 'harness' ? `<button class="secondary" data-harness-edit="${reference}">编辑</button><button class="secondary" data-harness-sessions="${reference}">执行会话</button><button class="secondary" data-harness-test="${reference}">检查连接</button>` : ''}<button class="secondary" data-resource-read="${reference}" data-resource-type="${kind}">查看</button><button class="quiet error-text" data-resource-delete="${reference}" data-resource-type="${kind}" ${kind === 'prompt' && ref(resource) === 'default/default' ? 'disabled title="默认 Prompt 是保留资源"' : ''}>删除</button>`;
}
function bindResourceActions(container) {
  container.querySelectorAll('[data-harness-sessions]').forEach(b=>b.onclick=()=>showHarnessResourceSessions(b.dataset.harnessSessions));
  container.querySelectorAll('[data-harness-edit]').forEach(b => b.onclick = () => showHarnessEdit(b.dataset.harnessEdit));
  container.querySelectorAll('[data-harness-test]').forEach(b => b.onclick = async () => {
    b.disabled = true; b.textContent = '检查中…';
    try {
      const result = await api('harness-test', {reference:b.dataset.harnessTest});
      notify(result.authenticated ? 'Codex 已连接，登录信息可用。未发起模型请求。' : result.requires_openai_auth === false ? 'Codex 已连接，使用外部模型认证配置。未验证模型请求。' : 'Codex 已连接，请先在服务器上登录 Codex。');
    } catch (e) { notify(errorText(e)); }
    finally { b.disabled = false; b.textContent = '检查连接'; }
  });
  container.querySelectorAll('[data-resource-read]').forEach(b => b.onclick = () => showResourceDetail(b.dataset.resourceType, b.dataset.resourceRead));
  container.querySelectorAll('[data-resource-delete]').forEach(b => b.onclick = () => showResourceDelete(b.dataset.resourceType, b.dataset.resourceDelete));
}
async function showResourceDetail(kind, reference) {
  dialog(`${labels[kind]} · ${reference}`, '<p class="form-note">正在读取资源…</p>');
  const generation = modalGeneration;
  try {
    const value = await api('resource', { action: 'read', kind, reference });
    if (generation !== modalGeneration || !$('#modal').open) return;
    const r = kind === 'prompt' ? value.prompt : kind === 'skill' ? value.skill : value;
    const pairs = [['资源', reference]];
    if (r.description) pairs.push(['说明', r.description]);
    if (kind === 'provider') pairs.push(['Base URL', r.base_url], ['密钥', r.api_key_configured ? '已配置' : '未配置']);
    if (kind === 'model') pairs.push(['Provider', ref(r.provider)], ['模型名称', r.model], ['API 协议', r.api_format], ['推理强度', r.params.reasoning_effort || '未指定'], ['温度', r.params.reasoning_effort ? '指定推理强度时不发送' : r.params.temperature], ['输出上限', r.params.max_tokens ?? '未指定'], ['上下文窗口', `${r.capabilities.context_window ?? 272000} tokens`], ['超时', `${r.params.timeout} 秒`], ['工具调用', r.capabilities.tool_call ? '开启' : '关闭']);
    if (kind === 'harness') pairs.push(['执行引擎', 'Codex'], ['可执行文件', r.executable], ['工作目录', r.working_directory], ['默认模型', r.model || '使用 Codex 默认配置'], ['默认思考档位', r.reasoning_effort || '沿用默认'], ['文件权限', r.sandbox === 'workspace_write' ? '工作区可写' : '只读'], ['超时', `${r.timeout_seconds} 秒`]);
    if (kind === 'agent') pairs.push([r.harness ? 'Harness' : '模型', ref(r.harness || r.model)], ['Prompt', ref(r.prompt)], ['Skills', r.skills.join('、') || '无'], ['MCP', r.mcps.map(ref).join('、') || '无'], ['内置工具', (r.builtin_tools||[]).map(k=>({resource_management:'资源管理',task_collaboration:'任务协作',local_execution:'本机命令'})[k]||k).join('、') || '无']);
    if (kind === 'mcp') pairs.push(['连接方式', r.transport], ['目标', r.target || '未指定']);
    const details = `<dl class="resource-details">${pairs.map(([label, text]) => `<dt>${esc(label)}</dt><dd>${esc(text)}</dd>`).join('')}</dl>`;
    const promptCharacters = kind === 'prompt' ? Array.from(value.content) : [];
    const previewLimit = 2000;
    const hasPreview = promptCharacters.length > previewLimit;
    const promptContent = kind === 'prompt' ? `<section class="prompt-reader" aria-label="Prompt 正文"><div class="prompt-reader-head"><h3>提示词正文</h3><span id="prompt-reader-mode" class="prompt-reader-mode">${hasPreview ? '预览' : '全文'}</span></div><pre id="prompt-content" class="resource-content">${esc(hasPreview ? promptCharacters.slice(0, previewLimit).join('') : value.content)}</pre><div class="prompt-reader-footer"><span id="prompt-character-count" role="status">${hasPreview ? `已展示 2,000 / 共 ${promptCharacters.length.toLocaleString('zh-CN')} 字符` : `共 ${promptCharacters.length.toLocaleString('zh-CN')} 字符`}</span>${hasPreview ? '<button id="toggle-prompt" class="secondary" type="button" aria-expanded="false" aria-controls="prompt-content">展开全文 <span aria-hidden="true">↓</span></button>' : ''}</div></section>` : '';
    const content = kind === 'prompt' ? promptContent : typeof value.content === 'string' ? `<h3>完整正文</h3><pre class="resource-content">${esc(value.content)}</pre>` : kind === 'mcp' ? `<h3>配置（敏感字段已隐藏）</h3><pre class="resource-content">${esc(JSON.stringify(r.config, null, 2))}</pre>` : '';
    $('#modal-body').innerHTML = `${details}${content}${r.path ? `<details class="resource-location"><summary>文件位置</summary><p>${esc(r.path)}</p></details>` : ''}`;
    if (hasPreview) $('#toggle-prompt').onclick = () => {
      const button = $('#toggle-prompt');
      const expanded = button.getAttribute('aria-expanded') !== 'true';
      $('#prompt-content').textContent = expanded ? value.content : promptCharacters.slice(0, previewLimit).join('');
      $('#prompt-reader-mode').textContent = expanded ? '全文' : '预览';
      $('#prompt-character-count').textContent = expanded ? `已展示全部 ${promptCharacters.length.toLocaleString('zh-CN')} 字符` : `已展示 2,000 / 共 ${promptCharacters.length.toLocaleString('zh-CN')} 字符`;
      button.setAttribute('aria-expanded', String(expanded));
      button.innerHTML = expanded ? '收起 <span aria-hidden="true">↑</span>' : '展开全文 <span aria-hidden="true">↓</span>';
      if (!expanded) $('.prompt-reader').scrollIntoView({ block: 'start' });
    };
  } catch (error) { if (generation === modalGeneration && $('#modal').open) { $('#modal-body').innerHTML = ''; modalError(error); } }
}
async function showResourceDelete(kind, reference) {
  dialog(`删除 ${labels[kind]} · ${reference}`, '<p class="form-note">正在检查资源引用与删除条件…</p>');
  const generation = modalGeneration;
  try {
    const plan = await api('resource', { action: 'plan', kind, reference });
    if (generation !== modalGeneration || !$('#modal').open) return;
    $('#modal-body').innerHTML = `<form id="modal-form"><p class="form-note">确认删除以下资源？删除后无法在 Web 撤销。此操作不会删除其他共享资源或聊天历史。</p><ul>${plan.items.map(item => `<li>${esc(item)}</li>`).join('')}</ul><div class="form-actions"><button id="cancel-resource-delete" class="secondary" type="button">取消</button><button class="primary" type="submit">确认删除</button></div></form>`;
    $('#cancel-resource-delete').onclick = closeModal;
    formHandler(async () => { await api('resource', { action: 'delete', kind, reference, confirm: plan.items }); $('#modal').close(); await bootstrap(); notify(`${labels[kind]} ${reference} 已删除。`); });
  } catch (error) { if (generation === modalGeneration && $('#modal').open) { $('#modal-body').innerHTML = ''; modalError(error); } }
}
function renderResources() {
  $('#resource-tabs').innerHTML = Object.entries(labels).filter(([k]) => k !== 'agent').map(([k, label]) => `<button data-kind="${k}" class="${k === state.resourceKind ? 'active' : ''}">${label} <small>${items(k).length}</small></button>`).join('');
  $$('[data-kind]').forEach(b => b.onclick = () => { state.resourceKind = b.dataset.kind; renderResources(); });
  const kind = state.resourceKind;
  const rows = items(kind);
  $('#resources').innerHTML = (kind === 'model' ? '<div class="resource-info warning">支持 Chat Completions 和 Responses；请选择 Provider 实际支持的接口。</div>' : '') + (rows.map(r => `<div class="resource-row"><div class="resource-name"><strong>${esc(r.skill || r.name || r.package)}</strong><p>${esc(r.category || 'default')}</p></div><p class="resource-description">${esc((kind === 'harness' ? `Codex · ${r.working_directory}` : r.model) || r.base_url || r.description || (kind === 'prompt' ? '点击查看完整 Prompt 正文' : r.target || '点击查看资源详情'))}${r.api_format || r.api_key_configured !== undefined ? `<br><span class="tag good">${esc(r.api_format || (r.api_key_configured ? '密钥已配置' : '未配置密钥'))}</span>` : ''}</p><div class="resource-actions">${resourceActions(kind, r)}</div></div>`).join('') || empty('资源库还是空的', '创建一个资源，或启动时指定已有的 --home。'));
  bindResourceActions($('#resources'));
  const diagnostics = state.boot.resources[kind]?.diagnostics;
  if (diagnostics?.length) $('#resources').insertAdjacentHTML('beforeend', `<p class="resource-info warning">有 ${diagnostics.length} 个资源读取异常。请在终端检查 agt ${esc(kind)} list；未显示的配置可能不可用。</p>`);
}
function showResource() {
  dialog('新建资源', `<form id="modal-form">${field('资源类型', `<select id="resource-kind" name="kind">${options(Object.entries(labels).filter(([k]) => k !== 'agent'), state.resourceKind)}</select>`)}${field('名称', '<input name="name" pattern="[A-Za-z0-9][A-Za-z0-9_-]*" maxlength="60" placeholder="新的资源名称" required>')}<div id="resource-fields"></div><div class="form-actions"><button class="primary" type="submit">创建资源</button></div></form>`);
  function render() {
    const kind = $('#resource-kind').value;
    const name = $('#modal-form input[name=name]');
    name.pattern = kind === 'skill' ? '[a-z0-9]+(-[a-z0-9]+)*' : '[A-Za-z0-9][A-Za-z0-9_-]*';
    name.title = kind === 'skill' ? '使用小写字母、数字和单个连字符，例如 code-review' : '使用字母、数字、下划线或连字符';
    name.placeholder = kind === 'skill' ? '例如 code-review（小写，单个连字符）' : '新的资源名称';
    $('#resource-fields').innerHTML = kind === 'harness' ? harnessFields() : kind === 'model' ? `<div class="model-provider-row">${field('Provider', `<select name="provider" required><option value="">选择 Provider…</option>${options(items('provider').map(p => [ref(p), ref(p)]))}</select>`)}<button id="load-provider-models" type="button" class="secondary" disabled>获取模型列表</button></div><p id="provider-models-status" class="model-list-status" role="status">选择 Provider 后可读取模型列表，也可直接填写模型名称。</p><div id="provider-model-options" class="hidden">${field('可用模型', '<select id="provider-model-list"><option value="">选择模型…</option></select>')}</div>${field('Provider 侧模型名称', '<input name="model" placeholder="从列表选择或直接输入模型名称" required>')}<div class="form-grid">${field('API 协议', '<select name="api_format"><option value="chat-completions">Chat Completions</option><option value="responses">Responses</option></select>')}${field('推理强度', '<select name="reasoning"><option value="">不指定 · 使用模型默认值</option><option value="none">None · 关闭推理</option><option value="minimal">Minimal · 最低</option><option value="low">Low · 低</option><option value="medium">Medium · 中</option><option value="high">High · 高</option><option value="xhigh">XHigh · 超高</option><option value="max">Max · 最高</option><option value="others">Others · 其他（自定义）</option></select>')}<div id="custom-reasoning-field" class="hidden">${field('自定义推理档位', '<input name="reasoning_custom" placeholder="填写 Provider 支持的参数值" disabled>', '按 Provider 文档填写，将原样发送（去除首尾空格）。')}</div>${field('上下文窗口（tokens）', '<input type="number" name="context_window" value="272000" min="4096" max="2000000" required>', '按模型文档填写。用于输入预算与自动压缩，不限制输出长度。')}${field('请求超时（秒）', '<input type="number" name="timeout" value="90" min="1" max="600" required>')}</div><p class="form-note">新别名启用工具调用，不指定输出 token 上限。推理档位因模型而异，请选择模型支持的值；“不指定”不发送此参数，None 则显式请求关闭推理。模型列表不代表协议支持情况，请选择 Provider 实际支持的 API 协议。</p>` : kind === 'provider' ? `${field('Base URL', '<input name="url" type="url" placeholder="https://example.com/v1" required>')}${field('API Key', '<input name="key" type="password" autocomplete="new-password" required>', '保存在本机 Provider 配置中，创建后不会显示密钥。')}` : kind === 'prompt' ? field('Prompt 正文', '<textarea name="text" required placeholder="写下角色指令…"></textarea>') : kind === 'skill' ? field('Skill 描述', '<textarea name="description" required placeholder="描述这个 Skill 的职责。创建后可用 CLI 完善 SKILL.md。"></textarea>') : field('单服务器 mcpServers JSON', '<textarea name="json" required placeholder=\'{"mcpServers":{"你的资源名称":{"command":"..."}}}\'></textarea>', '内部服务器名必须与资源名一致。仅挂载可信工具；这里不会执行。');
    if (kind === 'model') {
      const reasoning = $('#modal-form [name=reasoning]'), custom = $('#modal-form [name=reasoning_custom]');
      reasoning.onchange = () => {
        const other = reasoning.value === 'others';
        $('#custom-reasoning-field').classList.toggle('hidden', !other);
        custom.disabled = !other; custom.required = other;
        if (other) custom.focus();
      };
      const provider = $('#modal-form [name=provider]'), model = $('#modal-form [name=model]');
      const button = $('#load-provider-models'), list = $('#provider-model-list');
      const container = $('#provider-model-options'), status = $('#provider-models-status');
      const generation = modalGeneration;
      let request = 0;
      provider.onchange = () => {
        request++; button.disabled = !provider.value; button.textContent = '获取模型列表';
        model.value = ''; list.innerHTML = '<option value="">选择模型…</option>'; container.classList.add('hidden');
        status.textContent = '可读取所选 Provider 的模型列表，也可直接填写模型名称。';
      };
      list.onchange = () => { if (list.value) model.value = list.value; };
      model.oninput = () => { list.value = model.value; };
      button.onclick = async () => {
        const current = ++request;
        const active = () => current === request && generation === modalGeneration && provider.isConnected && $('#modal').open;
        button.disabled = true; button.textContent = '读取中…'; status.textContent = '正在向 Provider 获取模型列表…';
        try {
          const result = await api('provider-models', { provider: provider.value });
          if (!active()) return;
          const ids = [...new Set(result.items.map(item => item.id))];
          list.innerHTML = '<option value="">选择模型…</option>' + options(ids.map(id => [id, id]), model.value);
          container.classList.toggle('hidden', !ids.length);
          status.textContent = ids.length ? `已读取 ${ids.length} 个模型，请从列表选择或直接填写。` : 'Provider 返回了空列表，请直接填写模型名称。';
        } catch (error) {
          if (!active()) return;
          list.innerHTML = '<option value="">选择模型…</option>'; container.classList.add('hidden');
          status.textContent = errorText(error);
        } finally { if (active()) { button.disabled = false; button.textContent = '获取模型列表'; } }
      };
    }
  }
  $('#resource-kind').onchange = render; render();
  formHandler(async data => {
    const kind = data.get('kind'); let value;
    const effort = data.get('reasoning') === 'others' ? (data.get('reasoning_custom') || '').trim() : data.get('reasoning');
    if (kind === 'model' && data.get('reasoning') === 'others' && !effort) throw new Error('请填写自定义推理档位。');
    if (kind === 'model') value = { provider: data.get('provider'), model: data.get('model'), params: { api_format: data.get('api_format'), tool_call: true, context_window: Number(data.get('context_window')), timeout: Number(data.get('timeout')), ...(effort ? { reasoning_effort: effort } : {}) } };
    else if (kind === 'harness') value = harnessData(data);
    else if (kind === 'provider') value = { base_url: data.get('url').trim(), api_key: data.get('key').trim() };
    else if (kind === 'prompt') value = { text: data.get('text') };
    else if (kind === 'skill') value = { description: data.get('description') };
    else { try { value = JSON.parse(data.get('json')); } catch { throw new Error('MCP 配置不是有效 JSON。'); } if (!value?.mcpServers || Object.keys(value.mcpServers).length !== 1 || !Object.hasOwn(value.mcpServers, data.get('name').trim())) throw new Error('MCP 配置必须包含一个服务器，且服务器名与资源名称一致。'); }
    await tool('resource_write', { kind, reference: data.get('name').trim(), create: true, data: value });
    state.resourceKind = kind; $('#modal').close(); await bootstrap(); notify('资源已创建，原有配置未改动。');
  });
}
function switchPage(page) {
  closeCommandPanel();
  if (state.sending) { notify('消息发送中，请稍后切换页面。'); return; }
  closeConversationMenu();
  document.body.classList.toggle('workspace-page', ['chat','observer'].includes(page));
  document.body.classList.remove('mobile-chat-open');
  const previous = state.page; state.page = page;
  $$('.page').forEach(p => p.classList.toggle('hidden', p.id !== `page-${page}`));
  $$('.nav').forEach(b => { b.classList.toggle('active', b.dataset.page === page); if (b.dataset.page === page) b.setAttribute('aria-current', 'page'); else b.removeAttribute('aria-current'); });
  $('#page-label').textContent = { chat: '会话空间', observer: 'Agent 会话', agents: 'Agent 团队', resources: '资源库', settings: '运行设置', interactions: '交互观察' }[page];
  if (previous === 'interactions') window.AgtInteractions?.close();
  if (page === 'interactions') window.AgtInteractions?.open();
  if (page === 'settings') window.AgtSettings?.open();
  if (['chat', 'observer'].includes(page)) {
    const observer = page === 'observer';
    const viewer = observer ? selectedObserver || state.boot.principal : state.boot.principal;
    const changed = viewer !== state.viewer || (observer && previous !== 'observer');
    state.viewer = viewer; choosingIdentity = observer && !selectedObserver;
    $(observer ? '#observer-workspace' : '#chat-workspace').append($('.chat-layout'));
    $(observer ? '#observer-tools' : '#page-chat .section-heading').append($('#workspace-tools'));
    if (changed) { state.conversations = []; $('#search').value=''; $('#conversation-scope').value='recent'; clearConversation(); renderConversations(); }
    renderIdentities(); setupReadOnly();
    if (!choosingIdentity) conversations().then(activity).catch(e => notify(errorText(e),true));
  }
}
function help() { dialog('从这里开始你的实验', `<div class="guide"><p>这是通用的多 Agent 工作台。讨论、审查、博弈，都是同一套会话能力的用法。</p><ol><li><strong>准备模型。</strong>资源库会读取启动时的 home。支持已有的 Chat Completions / Responses 模型，请确认配置与 Provider 接口一致。</li><li><strong>创建角色。</strong>在 Agent 团队中设置角色 Prompt、模型、挂载资源和响应策略。</li><li><strong>建立会话。</strong>私聊选一人，群聊选多人。输入 @ 选择需要回应的成员，@all 提及所有人。</li><li><strong>发送与回复。</strong>默认发送后自动回复，会调用模型。可暂停后续回复，或关闭自动回复只保存消息；需要时点击「继续回复」。</li><li><strong>观察与推进。</strong>进入“Agent 会话”选择 Agent，只读查看其对话；“会话空间”始终使用你自己的身份。点击消息投递或运行条目检查错误。</li></ol><h3>几个重要边界</h3><p>界面是可信主机管理工具。观察身份不是多用户登录。阶段仍由你组织，没有游戏专属逻辑。自动消息不限制深度；每位 Agent 的回复额度由全局运行设置决定，新用户输入可重置。关闭服务可能中断调用；不要盲目重放结果不确定的操作。</p><h3>资源目录</h3><p>${esc(state.boot.home)}</p></div>`); }

$('#close-modal').onclick = closeModal;
$('#modal').addEventListener('cancel', e => { if (state.modalBusy) e.preventDefault(); });
$$('[data-page]').forEach(b => b.onclick = () => switchPage(b.dataset.page));
$('#select-all-agents').onchange = e => { selectedAgents.clear(); if (e.target.checked) items('agent').forEach(a=>selectedAgents.add(ref(a))); renderAgents(); };
$('#enable-agents').onclick = () => changeAgentEnabled(selectedAgents,true);
$('#disable-agents').onclick = () => changeAgentEnabled(selectedAgents,false);
$('#help').onclick = help; $('#new-agent').onclick = showAgent; $('#new-resource').onclick = showResource;
$('#new-chat').onclick = showNewChat; $('#empty-new-chat').onclick = showNewChat; $('#execute').onclick = showReplyControls;
$('#auto-reply').onchange = () => { $('#compose-hint').textContent = $('#auto-reply').checked ? '会调用模型 · Ctrl / ⌘ + Enter 发送' : '仅保存消息 · 可稍后点击「继续回复」'; };
$('#pause-reply').onclick = async () => {
  const conversation = state.conversation?.id; if (!conversation) return;
  $('#pause-reply').disabled = true;
  try { await api('reply', { action: 'pause', conversation }); await activity(); }
  catch (e) { notify(errorText(e), true); $('#pause-reply').disabled = false; }
};
$('#end-reply').onclick = () => {
  const conversation = state.conversation?.id; if (!conversation || !canParticipate()) return;
  dialog('停止本轮 Agent？', '<form id="modal-form"><p>结束涉及本群的讨论，取消后续回复和提醒。同一讨论在其他群的任务也会停止。已发送的消息保留，已发出的外部工具请求可能仍会完成。</p><div class="form-actions"><button class="primary" type="submit">结束讨论</button></div></form>');
  formHandler(async () => { await api('reply', { action: 'end', conversation }); $('#modal').close(); await activity(); });
};
$('#search').oninput = () => { closeConversationMenu(); renderConversations(); };
$('#clear-search').onclick = () => { $('#search').value=''; renderConversations(); $('#search').focus(); };
$('#back-to-conversations').onclick = () => { document.body.classList.remove('mobile-chat-open'); layoutPreferences.conversations=false; renderLayout(); $('#search').focus({preventScroll:true}); };
document.addEventListener('keydown', event => {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k' && !event.isComposing && !document.querySelector('dialog[open]') && ['chat','observer'].includes(state.page) && !choosingIdentity) {
    event.preventDefault(); document.body.classList.remove('mobile-chat-open'); layoutPreferences.conversations=false; renderLayout(); $('#search').focus(); $('#search').select();
  }
});
function resizeComposer() {
  const input = $('#message-body');
  if (!input || !input.getClientRects().length) return;
  input.style.height = 'auto'; input.style.height = Math.min(152, Math.max(56,input.scrollHeight)) + 'px';
}
function updateLatestMessage() {
  const box = $('#messages');
  $('#latest-message').classList.toggle('hidden', !state.conversation || box.scrollHeight-box.scrollTop-box.clientHeight < 140);
}
$('#messages').addEventListener('scroll', updateLatestMessage);
$('#latest-message').onclick = () => { $('#messages').scrollTop=$('#messages').scrollHeight; updateLatestMessage(); };

$('#change-identity').onclick = () => { if (state.sending) return; selectedObserver=null; choosingIdentity=true; state.conversations=[]; clearConversation(); renderIdentities(); };
$('#agent-search').oninput = renderIdentities;
$('#conversation-scope').onchange = () => { conversationRequest++; conversations().then(activity).catch(e => notify(errorText(e),true)); };
$('#show-all-history').onclick = () => { $('#conversation-scope').value = 'all'; $('#search').value = ''; $('#conversation-scope').onchange(); };
$('#group-info').onclick = showGroupInfo;
$('#close-group').onclick = () => $('#group-drawer').close();
$('#debug-mode').checked = debugMode; document.body.classList.toggle('debug-mode', debugMode);
$('#debug-mode').onchange = () => { debugMode = $('#debug-mode').checked; localStorage.setItem('agt-debug-mode', String(debugMode)); document.body.classList.toggle('debug-mode', debugMode); if ($('#modal').open && !state.modalBusy) $('#modal').close(); renderMessages(); renderReplyStatus(); renderActivity(); };
$('#older').onclick = () => messages(false, true).catch(e => notify(errorText(e), true));
$('#refresh').onclick = async () => { try { await bootstrap(); await conversations(); await messages(true); await activity(); notify('已同步本地状态。'); } catch (e) { notify(errorText(e), true); } };
$('#chat-commands').onclick = () => { if(!$('#command-panel').classList.contains('hidden')) closeCommandPanel(true); else if(canParticipate()) showChatCommand({name:'help',args:''}); };
$('#composer').onsubmit = async event => {
  event.preventDefault(); if (state.sending || !canParticipate()) return;
  let body = $('#message-body').value.trim(); if (!body) return;
  const command = AgtCommands.parse(body);
  if (command?.type === 'command') { await showChatCommand(command); return; }
  const literalMessage = command?.type === 'literal';
  if (literalMessage) body = command.text;
  const args = { conversation: state.conversation.id, body, mentions: mentionedPrincipals(body) };
  closeMentions();
  const autoReply = $('#auto-reply').checked;
  const signature = JSON.stringify(args);
  if (state.pendingSend?.signature !== signature) state.pendingSend = { signature, key: crypto.randomUUID() };
  state.sending = true; $('#send').disabled = true; renderIdentities(); $('#message-body').disabled = true;
  $('#send').textContent = '发送中…'; renderReplyStatus();
  try {
    await api('tool', { name: 'message_send', args: { ...args, request_key: state.pendingSend.key }, auto_reply: autoReply, literal_message: literalMessage });
    $('#message-body').value = ''; state.pendingSend = null; renderMentions();
    notify(autoReply ? '消息已发送，已安排 Agent 回复。' : '消息已保存，点击「继续回复」可让 Agent 回应。');
    await messages(true); await conversations(); await activity();
  } catch (e) { notify(errorText(e), true); }
  finally { state.sending = false; $('#send').disabled = false; $('#send').textContent = '发送消息 ↑'; renderIdentities(); $('#message-body').disabled = false; renderReplyStatus(); }
};
$('#message-body').oninput = event => { resizeComposer(); if (!event.isComposing) showMentionOptions(); };
$('#message-body').oncompositionend = showMentionOptions;
$('#message-body').onclick = showMentionOptions;
$('#message-body').onblur = closeMentions;
$('#message-body').onkeydown = event => {
  if (event.isComposing) return;
  if (event.key === 'Enter' && !event.shiftKey && AgtCommands.parse($('#message-body').value)?.type === 'command') { event.preventDefault(); $('#composer').requestSubmit(); return; }
  if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) { event.preventDefault(); closeMentions(); $('#composer').requestSubmit(); return; }
  if (!mentionOptions.length) return;
  if (event.key === 'Escape') { event.preventDefault(); closeMentions(); }
  else if (event.key === 'Enter' || event.key === 'Tab') { event.preventDefault(); insertMention(mentionIndex); }
  else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault(); mentionIndex = (mentionIndex + (event.key === 'ArrowDown' ? 1 : -1) + mentionOptions.length) % mentionOptions.length;
    $$('#mention-menu button').forEach((b, i) => b.setAttribute('aria-selected', String(i === mentionIndex)));
    const selected = $(`#mention-option-${mentionIndex}`); selected.scrollIntoView({ block: 'nearest' });
    $('#message-body').setAttribute('aria-activedescendant', selected.id);
  } else if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) closeMentions();
};
let eventSubscription = null, refreshTimer = null, refreshPending = false, directoryPending = false;
function queueRefresh(directory = false) {
  refreshPending = true; directoryPending ||= directory;
  if (refreshTimer || document.hidden) return;
  refreshTimer = setTimeout(async () => {
    refreshTimer = null;
    if (state.polling) { queueRefresh(); return; }
    refreshPending = false;
    state.polling = true;
    try {
      if (directoryPending) { directoryPending = false; await bootstrap(); }
      await activity();
      if (state.page === 'interactions') await window.AgtInteractions?.refresh();
      if (['chat', 'observer'].includes(state.page)) { await conversations(); await messages(); }
    } catch (e) { $('#connection').textContent = '同步失败'; if (e.code === 'unauthorized') notify(errorText(e), true); }
    finally { state.polling = false; if (refreshPending) queueRefresh(); }
  }, 300);
}
function ensureEvents() {
  if (!state.boot || !state.viewer || eventSubscription?.viewer === state.viewer) return;
  eventSubscription?.controller.abort();
  const subscription = { viewer: state.viewer, controller: new AbortController(), cursor: null };
  eventSubscription = subscription;
  consumeEvents(subscription);
}
async function consumeEvents(subscription) {
  const { controller, viewer } = subscription;
  let retry = 1000;
  while (!controller.signal.aborted) {
    try {
      const query = new URLSearchParams({ viewer });
      if (subscription.cursor) query.set('after', subscription.cursor);
      const response = await fetch(`/api/events?${query}`, { headers: { 'X-AGT-Token': token || '' }, signal: controller.signal });
      if (!response.ok) { if ([401,403].includes(response.status)) { $('#connection').textContent = '订阅权限已失效'; return; } throw new Error('stream unavailable'); }
      const reader = response.body.getReader(), decoder = new TextDecoder();
      let buffer = '';
      try {
        while (!controller.signal.aborted) {
          const { done, value } = await reader.read(); if (done) break;
          buffer += decoder.decode(value, { stream: true });
          if (buffer.length > 256 * 1024) throw new Error('stream too large');
          let boundary;
          while ((boundary = buffer.indexOf('\n\n')) >= 0) {
            const frame = buffer.slice(0,boundary); buffer = buffer.slice(boundary + 2);
            const data = frame.split('\n').find(line => line.startsWith('data: '));
            if (!data) continue;
            const page = JSON.parse(data.slice(6));
            if (eventSubscription !== subscription || state.viewer !== viewer) return;
            subscription.cursor = page.cursor;
            $('#connection').textContent = '本地已连接'; $('#connection').title = '实时事件订阅已连接'; retry = 1000;
            if (page.event === 'ready' || page.event === 'resync_required' || (typeof page.scheduler_online === 'boolean' && page.scheduler_online !== state.scheduler.online)) queueRefresh(true);
            else if (page.changes?.length) queueRefresh(page.changes.some(change => !change.conversation));
          }
        }
      } finally { await reader.cancel().catch(() => {}); }
      if (!controller.signal.aborted) throw new Error('stream ended');
    } catch {
      if (controller.signal.aborted) return;
      $('#connection').textContent = '正在重连';
      await new Promise(resolve => {
        const finish = () => { clearTimeout(timer); controller.signal.removeEventListener('abort', finish); resolve(); };
        const timer = setTimeout(finish, retry);
        controller.signal.addEventListener('abort', finish, { once: true });
      });
      retry = Math.min(retry * 2, 15000);
    }
  }
}
document.addEventListener('visibilitychange', () => { if (!document.hidden) queueRefresh(true); });
async function init() {
  try { await bootstrap(); await conversations(); await activity(); setupReadOnly(); }
  catch (e) { $('#connection').textContent = '未连接'; notify(errorText(e), true); $('#execute').disabled = true; }
}
init();

function executionFields(agent = {}) {
  const model = agent.model ? ref(agent.model) : '';
  const harness = agent.harness ? ref(agent.harness) : '';
  const harnesses = items('harness').map(e => [ref(e), `${e.name} · Codex`]);
  if (harness && !harnesses.some(([id]) => id === harness)) harnesses.push([harness, `${harness}（连接不可用）`]);
  return `<section class="execution-settings"><div class="form-grid">${field('执行方式', `<select name="execution">${options([['native','AGT 内置'],['external','外部 Harness']], harness ? 'external' : 'native')}</select>`)}<div data-execution-native>${field('模型', `<select name="model" required><option value="">选择模型…</option>${options(items('model').map(m=>[ref(m),`${m.name} · ${m.model}`]),model)}</select>`)}</div><div data-execution-external class="hidden">${field('Harness', `<select name="harness"><option value="">选择 Harness…</option>${options(harnesses,harness)}</select>`, harnesses.length ? '复用连接配置，每位 Agent 的会话上下文独立。' : '请先在资源库创建 Harness。')}</div></div><p data-execution-external class="form-note hidden">Codex 管理工具循环和上下文压缩；仅响应私聊和群内 @。支持角色 Prompt、Skills 正文和内置资源工具，暂不挂载 AGT MCP。工作目录由Harness决定。</p></section>`;
}
function executionData(data) { return data.get('execution') === 'external' ? {harness:data.get('harness')} : {model:data.get('model')}; }
function bindExecutionFields() {
  const selector = $('#modal-form [name=execution]');
  if (!selector) return;
  const update = () => {
    const external = selector.value === 'external';
    $$('[data-execution-native]').forEach(e=>e.classList.toggle('hidden',external));
    $$('[data-execution-external]').forEach(e=>e.classList.toggle('hidden',!external));
    for (const [name,active] of [['model',!external],['harness',external]]) {
      const input = $(`#modal-form [name=${name}]`); input.disabled=!active; input.required=active;
    }
    $$('#modal-form [name=mcps]').forEach(input=>{input.disabled=external;});
    const cwd = $('#modal-form [name=cwd]');
    if (cwd) {
      cwd.readOnly=external;
      if (external) cwd.value=items('harness').find(e=>ref(e)===$('#modal-form [name=harness]').value)?.working_directory || state.boot.home;
    }
    const policy = $('#modal-form [name=policy]');
    if (policy) { policy.querySelector('[value=all]').disabled=external; if(external && policy.value==='all') policy.value='mentioned'; }
  };
  selector.onchange=update; $('#modal-form [name=harness]').onchange=update; update();
}
function harnessFields(value = {}) {
  return `<section class="harness-intro"><span class="tag">Codex</span><p>连接服务器上已安装的 Codex。每个 Harness 独立配置项目和默认模型；已有会话保存自己的配置快照。</p></section>${field('说明', `<textarea name="harness_description" maxlength="4096">${esc(value.description||'')}</textarea>`)}${field('可执行文件', `<input name="executable" value="${esc(value.executable || '')}" placeholder="/usr/local/bin/codex" required>`, '服务器上的绝对路径，可运行 command -v codex 查询。')}${field('项目工作目录', `<input name="working_directory" value="${esc(value.working_directory || '')}" placeholder="/home/user/projects/demo" required>`, '同一目录中的 Agent 会共享文件；并行开发建议使用不同目录或 worktree。')}<div class="form-grid">${field('文件权限', `<select name="sandbox">${options([['read_only','只读 · 分析与审查'],['workspace_write','工作区可写 · 开发与修改']],value.sandbox || 'read_only')}</select>`)}${field('任务超时（秒）', `<input type="number" name="timeout_seconds" min="10" max="86400" value="${value.timeout_seconds || 1800}" required>`)}</div><details class="harness-options"><summary>模型与认证说明</summary>${field('默认思考档位（可选）', `<input name="external_effort" value="${esc(value.reasoning_effort||'')}" placeholder="留空沿用 Codex 默认">`)}${field('默认模型（可选）', `<input name="external_model" value="${esc(value.model || '')}" placeholder="留空使用 Codex 默认模型">`)}<p class="form-note">认证沿用服务器运行账号的 Codex 配置，请先完成 codex login。环境按已选文件权限执行，不自动批准权限升级；暂不支持交互授权，遇到此类请求会停止并说明原因。/new 重置工作上下文，不删除 Codex 自身的长期记忆或项目文件。</p></details>`;
}
function harnessData(data) { return {engine:'codex',description:data.get('harness_description').trim(),reasoning_effort:data.get('external_effort').trim()||null,executable:data.get('executable').trim(),working_directory:data.get('working_directory').trim(),sandbox:data.get('sandbox'),model:data.get('external_model').trim() || null,timeout_seconds:Number(data.get('timeout_seconds'))}; }
async function showHarnessEdit(reference) {
  dialog(`编辑Harness · ${reference}`,'<p class="form-note">正在读取配置…</p>');
  const generation=modalGeneration;
  try {
    const value=await api('resource',{action:'read',kind:'harness',reference});
    if(generation!==modalGeneration || !$('#modal').open)return;
    $('#modal-body').innerHTML=`<form id="modal-form">${harnessFields(value)}<p class="form-note">修改只影响新建执行会话。已有会话保留项目目录、模型和权限快照；可在聊天中用 /model 修改会话模型，或 /new 使用新默认值。</p><div class="form-actions"><button id="harness-default-models" class="secondary" type="button">读取可用模型</button><button id="harness-sessions" class="secondary" type="button">查看执行会话</button><button class="primary" type="submit">保存 Harness</button></div></form>`;
    $('#harness-sessions').onclick=()=>showHarnessResourceSessions(reference);
    $('#harness-default-models').onclick=async()=>{
      const button=$('#harness-default-models');button.disabled=true;
      try {
        const catalog=await api('harness',{operation:'resource-models',reference});
        if(generation!==modalGeneration || !$('#modal').open)return;
        if(!catalog.data.length)throw new Error('empty model catalog');
        const previous=$('[name=external_model]');
        const selected=previous.value;
        previous.outerHTML=`<select name="external_model"><option value="">沿用 Codex 默认</option>${options(catalog.data.map(m=>[m.model,m.displayName||m.model]),selected)}</select>`;
        const select=$('[name=external_model]');
        const update=()=>{const model=catalog.data.find(m=>m.model===select.value);const old=$('[name=external_effort]');old.outerHTML=`<select name="external_effort"><option value="">沿用默认档位</option>${model?options(model.supportedReasoningEfforts.map(e=>[e.reasoningEffort,e.reasoningEffort]),old.value):''}</select>`;};
        select.onchange=update;update();select.closest('details').open=true;
        notify('已读取已保存 Harness 连接的模型列表。');
      }catch(e){if(generation===modalGeneration)notify(errorText(e),true);}finally{if(generation===modalGeneration)button.disabled=false;}
    };
    formHandler(async data=>{await tool('resource_write',{kind:'harness',reference,create:false,expected_revision:value.revision,data:harnessData(data)});$('#modal').close();await bootstrap();notify('Harness已更新。');});
  } catch(e) {if(generation===modalGeneration)modalError(e);}
}

function codexCommandTargets() {
  return state.boot.bindings.filter(b=>{
    if(principalOf(b.principal)?.deleted || !state.members.some(m=>m.principal_id===b.principal && m.left_message_seq==null)) return false;
    const agent=items('agent').find(a=>ref(a)===b.agent);
    return agent?.harness && items('harness').some(h=>ref(h)===ref(agent.harness) && h.engine==='codex');
  });
}
function availableChatCommands() {
  const codex=codexCommandTargets().length>0;
  return AgtCommands.items.filter(c=>c.name!=='clear' && (!c.requires || codex)).map(c=>{
    if(c.name==='model') return {...c,label:codex?'Codex · 选择模型':'查看模型',description:codex?'选择 Codex Agent，修改它在当前聊天中的模型与档位。':'查看各 Agent 的配置模型与最近执行模型；此处不修改内置 Agent 模型。'};
    return c.requires ? {...c,label:'Codex · '+c.label} : c;
  });
}
function showUnavailableCommand(kind) {
  commandPanel('当前会话不支持 /'+kind,`<p>此命令用于 Codex 执行会话，当前会话没有可操作的 Codex Agent。</p><p class="form-note">内置 Agent 暂未接入此命令。可以查看模型和运行状态，或重置上下文。</p><button id="available-commands" class="secondary" type="button">查看可用命令</button><p class="form-note">草稿已保留，本次未发送消息或执行请求。</p>`);
  $('#available-commands').onclick=()=>showChatCommand({name:'help',args:''});
}
const commandPanelHome = $('#command-panel').parentElement;
function closeCommandPanel(restoreFocus = false) {
  commandGeneration++;
  commandTarget = null;
  $('#command-panel').classList.add('hidden');
  document.body.classList.remove('command-panel-open');
  $('#chat-commands').setAttribute('aria-expanded','false');
  $('#command-panel-body').innerHTML='';
  if (restoreFocus) $($('#command-panel').dataset.mode === 'approvals' ? '#ix-approvals' : $('#command-panel').dataset.mode === 'tasks' ? '#chat-tasks' : '#chat-commands').focus();
  commandPanelHome.append($('#command-panel'));
}
function commandPanel(title, content, kind = 'help') {
  (kind === 'approvals' ? document.body : commandPanelHome).append($('#command-panel'));
  $('#command-panel').dataset.mode = kind;
  commandGeneration++;
  const opening = $('#command-panel').classList.contains('hidden');
  $('#command-panel').classList.remove('hidden');
  document.body.classList.add('command-panel-open');
  $('#chat-commands').setAttribute('aria-expanded','true');
  $('#command-panel-title').textContent=title;
  $('#command-panel-context').textContent=kind==='approvals' ? '当前观察身份可见的审批' : state.conversation ? conversationName(state.conversation) : '';
  $('#command-panel-body').innerHTML=content;
  $('#command-panel-error').classList.add('hidden');
  $('#command-panel-nav').innerHTML=(kind==='approvals'?[]:availableChatCommands()).map(c=>`<button type="button" data-panel-command="${c.name}" aria-current="${kind===c.name?'page':'false'}" title="${esc(c.description)}">${esc(c.label)}</button>`).join('');
  $$('#command-panel-nav button').forEach(b=>b.onclick=()=>showChatCommand({name:b.dataset.panelCommand,args:''}));
  if(opening) $('#close-command-panel').focus({preventScroll:true});
  else $('#command-panel-nav [aria-current=page]')?.focus({preventScroll:true});
}
function commandPanelError(error) {
  $('#command-panel-error').textContent=errorText(error);
  $('#command-panel-error').classList.remove('hidden');
}
$('#close-command-panel').onclick=()=>closeCommandPanel(true);
$('#command-panel').addEventListener('keydown',event=>{
  if(event.key==='Escape' && !event.isComposing){event.preventDefault();closeCommandPanel(true);}
});
function showCommandAction(kind) {
  const reset=['new','clear'].includes(kind), c=state.conversation;
  const allowed=reset?canResetContext(c):canParticipate() && !$('#end-reply').disabled;
  commandPanel(reset?'重置上下文':'停止 Agent',`<p>${reset?'停止涉及本会话的 Agent 工作，清除会话摘要。下一条消息将从新的上下文开始。':'结束涉及本会话的讨论，取消后续回复和提醒。同一讨论在其他群的任务也会停止。'}</p><p class="form-note">聊天记录保留，已发出的外部工具操作不会撤销。${reset?'Agent 仍可通过历史查询工具查阅旧消息。':''}</p>${allowed?'':'<p class="form-note">当前没有可执行的操作，请检查会话权限和运行状态。</p>'}<button id="command-confirm" class="primary" type="button" ${allowed?'':'disabled'}>${reset?'确认停止并重置':'确认结束讨论'}</button><p id="command-action-feedback" role="status"></p>`,reset?'new':'stop');
  const generation=commandGeneration;
  $('#command-confirm').onclick=async()=>{
    const button=$('#command-confirm');button.disabled=true;
    $('#command-panel-error').classList.add('hidden');
    try {
      if(reset) await api('conversation',{action:'reset',conversation:c.id,confirm:true});
      else await api('reply',{action:'end',conversation:c.id});
      if(generation!==commandGeneration)return;
      if(reset) await messages(true);
      $('#command-action-feedback').textContent=reset?'上下文已重置，可以发送新消息。':'已结束本轮讨论。';
      if(reset){
        state.generation++;
        state.runs=state.runs.map(r=>r.conversation===c.id?{...r,progress:null,tool_activity:null}:r);renderLiveReplies();
      }
      const parsed=AgtCommands.parse($('#message-body').value);
      if(parsed?.type==='command' && [kind,reset?'clear':'stop'].includes(parsed.name)){$('#message-body').value='';state.pendingSend=null;renderMentions();resizeComposer();}
      await activity();
    }catch(e){if(generation===commandGeneration){commandPanelError(e);button.disabled=false;}}
  };
}
async function showChatCommand(command) {
  if(!state.conversation || !canParticipate()) return;
  const supported = AgtCommands.items.find(c=>c.name===command.name);
  if (supported?.requires && !codexCommandTargets().length) { showUnavailableCommand(command.name); return; }
  if (supported && !command.args && ['new','clear'].includes(command.name)) { showCommandAction(command.name); return; }
  if (supported && !command.args && command.name==='stop') { showCommandAction('stop'); return; }
  if (supported && !command.args && ['model','status','compact','rename','skills','mcp','permissions'].includes(command.name)) { await showHarnessCommand(command.name); return; }
  const heading = command.name==='help' && !command.args ? '会话命令' : supported ? '此命令暂不支持参数' : '此命令尚未接入 Web';
  commandPanel(heading, `<p class="form-note">以下只列出当前会话可用的操作。重置和停止作用于整个会话；标注 Codex 的操作仅作用于所选 Codex Agent。</p><div class="chat-command-list">${availableChatCommands().map(c=>`<button type="button" class="chat-command" data-chat-command="${c.name}"><code>/${esc(c.name)}</code><span><strong>${esc(c.label)}</strong><small>${esc(c.description)}</small></span></button>`).join('')}</div><details class="harness-options"><summary>其他 Codex 命令如何处理？</summary><p class="form-note">/review、/diff、/resume、/fork 和交互审批尚未映射。请在对应 Codex 客户端操作，或用自然语言提出任务。带参数的命令目前不会执行；群内命令不支持 @ 定向控制。</p></details><div class="form-actions"><button id="command-as-text" class="secondary" type="button" ${AgtCommands.parse($('#message-body').value)?.type==='command'?'':'disabled'}>改为普通文本草稿</button></div><p class="form-note">要讨论命令本身，可输入 //model 等双斜杠文本，再点击发送。</p>`);
  $$('[data-chat-command]').forEach(button=>button.onclick=()=>{ showChatCommand({name:button.dataset.chatCommand,args:''}); });
  $('#command-as-text').onclick=()=>{ $('#message-body').value='/'+$('#message-body').value.trim(); closeCommandPanel(); $('#message-body').focus(); resizeComposer(); };
}
async function showChatRuntime(kind) {
  const conversation=state.conversation.id;
  commandPanel(kind==='model' ? '本会话的模型' : '本会话的运行状态','<p class="form-note">正在读取配置和运行记录，不调用模型…</p>',kind);
  const generation=commandGeneration;
  try {
    const result=await api('chat-status',{conversation});
    if(generation!==commandGeneration || $('#command-panel').classList.contains('hidden'))return;
    $('#command-panel-body').innerHTML=`<p class="form-note">按 Agent 分别显示。最近执行模型来自后端返回值，可能与修改后的配置不同；没有记录时不推测。</p>${result.agents.map(a=>`<section class="command-agent"><div class="card-head"><strong>${esc(nameOf(a.principal))}</strong><span class="tag">${a.engine==='native'?'AGT 内置':esc(a.engine)}</span></div><dl class="resource-details"><dt>启用状态</dt><dd>${a.enabled?'已启用':'已停用'}</dd><dt>配置模型</dt><dd>${esc(a.configured_model || '沿用外部引擎默认配置')}</dd><dt>最近执行模型</dt><dd>${esc(a.last_run?.reported_model || '后端尚未报告')}</dd><dt>最近执行</dt><dd>${esc(a.last_run ? stateNames[a.last_run.state] || a.last_run.state : '暂无记录')}</dd>${a.harness?`<dt>Harness</dt><dd>${esc(a.harness)}</dd>`:''}</dl>${a.harness?`<button class="secondary" type="button" data-command-harness="${esc(a.harness)}">编辑共享环境…</button><p class="form-note">修改默认配置只影响新会话；已有会话保留配置快照。</p>`:''}</section>`).join('') || '<p class="form-note">本会话暂无已配置运行设置的 Agent。</p>'}`;
    $$('[data-command-harness]').forEach(b=>b.onclick=()=>showHarnessEdit(b.dataset.commandHarness));
  } catch(e) {if(generation===commandGeneration && !$('#command-panel').classList.contains('hidden')){$('#command-panel-body').innerHTML='';commandPanelError(e);}}
}

async function showHarnessCommand(kind) {
  if(!state.conversation || !canParticipate()) return;
  const conversation=state.conversation.id;
  const targets=codexCommandTargets();
  if (!targets.length) { if(kind==='model'||kind==='status') return showChatRuntime(kind); showUnavailableCommand(kind); return; }
  const mixed=state.boot.bindings.some(b=>state.members.some(m=>m.principal_id===b.principal && m.left_message_seq==null) && !targets.some(t=>t.principal===b.principal));
  if(kind==='status' && mixed) return showChatRuntime(kind);
  const titles={model:'选择会话模型',status:'执行会话',compact:'压缩上下文',rename:'会话命名',skills:'Codex Skills',mcp:'Codex MCP',permissions:'会话权限'};
  commandPanel(titles[kind] || '执行会话',`${mixed?'<p class="form-note">此操作仅作用于 Codex Agent，不影响群内其他 Agent。</p><button id="all-agent-models" class="secondary" type="button">查看所有 Agent 的模型与状态</button>':''}${field('Codex 操作对象',`<select id="harness-target">${options(targets.map(b=>[b.principal,nameOf(b.principal)]),commandTarget)}</select>`)}<div id="harness-detail"><p class="form-note">正在读取执行会话…</p></div>`,kind);
  if(mixed) $('#all-agent-models').onclick=()=>showChatRuntime('model');
  const generation=commandGeneration;
  let version=0;
  const load=async()=>{
    const request=++version, target=$('#harness-target').value;
    commandTarget=target;
    const active=()=>generation===commandGeneration && !$('#command-panel').classList.contains('hidden') && request===version;
    const call=(operation,extra={})=>api('harness',{operation,conversation,target,...extra});
    $('#harness-detail').innerHTML='<p class="form-note">正在读取执行会话…</p>';
    try {
      const session=await call('current');
      if(!active())return;
      const operate=(operation,extra={})=>call(operation,{session:session.id,...extra});
      const executionLabels={preparing:'准备中',running:'执行中',canceling:'正在停止',reconciling:'正在恢复',succeeded:'已完成',failed:'已失败 · 可继续',canceled:'已确认停止 · 可继续',interrupted:'引擎已中断 · 可继续',needs_review:'需要处理'};
      const canCancelExecution=['preparing','running','canceling'].includes(session.execution?.state);
      const executionLabel=executionLabels[session.execution?.state] || (session.busy?'暂不可用':'就绪');
      const info=`<div class="harness-session-summary"><strong>${esc(session.title||'未命名执行会话')}</strong><span class="tag">${esc(executionLabel)}</span><details class="session-metadata"><summary>连接详情</summary><dl class="resource-details"><dt>Harness</dt><dd>${esc(session.harness)}</dd><dt>项目目录</dt><dd>${esc(session.config.working_directory)}</dd><dt>Session ID</dt><dd>${esc(session.session_id||'首次执行时由 Codex 生成')}</dd></dl></details></div>`;
      $('#harness-detail').innerHTML=info+'<div class="form-actions"><button id="harness-status-refresh" class="secondary" type="button">刷新状态</button></div>';
      $('#harness-status-refresh').onclick=load;
      if(session.execution?.state==='needs_review') $('#harness-detail').insertAdjacentHTML('beforeend','<p class="form-note" role="status">上次执行结果无法确认。请检查原 Codex 线程和项目改动，确认旧执行已停止后使用 /new 开始新上下文。不会自动重做旧任务。</p><button id="harness-new-context" class="secondary" type="button">开始新上下文…</button>');
      if($('#harness-new-context')) $('#harness-new-context').onclick=()=>showCommandAction('new');
      if(kind==='model') {
        $('#harness-detail').insertAdjacentHTML('beforeend','<p id="harness-loading" class="form-note">正在读取 Codex 可用模型…</p>');
        const catalog=await operate('models'); if(!active())return;
        $('#harness-loading').remove();
        const models=catalog.data;
        if(!models.length){$('#harness-detail').insertAdjacentHTML('beforeend','<p class="form-note">当前连接没有返回可选模型。请检查 Codex 登录与服务配置。</p>');return;}
        const selected=models.some(m=>m.model===session.config.model)?session.config.model:(models.find(m=>m.isDefault)||models[0]).model;
        $('#harness-detail').insertAdjacentHTML('beforeend',`<form id="harness-model-form">${field('模型',`<select name="model">${options(models.map(m=>[m.model,m.displayName||m.model]),selected)}</select>`)}<p id="harness-model-description" class="form-note"></p>${field('思考档位','<select name="effort"></select>')}<p class="form-note">当前设置：${esc(session.config.model||'Codex 默认')} · ${esc(session.config.reasoning_effort||'默认档位')}。保存仅影响此 Agent 在本聊天中的后续执行，保留 Session ID。</p><div class="form-actions"><button class="primary" type="submit" ${session.busy?'disabled':''}>应用到本会话</button></div><p id="harness-feedback" role="status" class="form-note"></p></form>`);
        const model=$('#harness-model-form [name=model]'),effort=$('#harness-model-form [name=effort]');
        const update=()=>{const m=models.find(m=>m.model===model.value);$('#harness-model-description').textContent=m.description||'';effort.innerHTML=options(m.supportedReasoningEfforts.map(e=>[e.reasoningEffort,e.reasoningEffort]),m.model===session.config.model?session.config.reasoning_effort||m.defaultReasoningEffort:m.defaultReasoningEffort);};
        model.onchange=update;update();
        $('#harness-model-form').onsubmit=async e=>{e.preventDefault();const button=e.currentTarget.querySelector('button');button.disabled=true;try{await operate('configure',{model:model.value,effort:effort.value});if(active()){$('#harness-feedback').textContent='已保存，下次执行使用新设置。';}}catch(e){if(active())$('#harness-feedback').textContent=errorText(e);}finally{if(active())button.disabled=false;}};
      } else if(kind==='rename') {
        $('#harness-detail').insertAdjacentHTML('beforeend',`<form id="harness-rename-form">${field('会话名称',`<input name="title" value="${esc(session.title)}" maxlength="80" required>`,'同步到 Codex 的会话名称；不会修改聊天名称或 Session ID。')}<div class="form-actions"><button class="primary" ${session.busy?'disabled':''}>保存名称</button></div><p id="harness-feedback" role="status" class="form-note"></p></form>`);
        $('#harness-rename-form').onsubmit=async e=>{e.preventDefault();const b=e.currentTarget.querySelector('button');b.disabled=true;try{await operate('rename',{title:e.currentTarget.elements.title.value});if(active())await load();}catch(e){if(active()){$('#harness-feedback').textContent=errorText(e);b.disabled=false;}}};
      } else if(kind==='compact') {
        $('#harness-detail').insertAdjacentHTML('beforeend',`<p class="form-note">由 Codex 压缩当前上下文，可能产生模型费用。保留聊天记录和 Session ID。确认结束后可继续使用；结果无法确认时会显示「需要处理」。</p><div class="form-actions"><button id="harness-compact" class="primary" ${session.busy||!session.session_id?'disabled':''}>开始压缩</button><button id="harness-cancel" class="secondary" ${canCancelExecution?'':'disabled'}>取消执行</button></div><p id="harness-feedback" role="status" class="form-note"></p>`);
        $('#harness-cancel').onclick=async()=>{try{await operate('cancel');if(active())$('#harness-feedback').textContent='已请求取消，请检查状态后重置上下文。';}catch(e){if(active())$('#harness-feedback').textContent=errorText(e);}};
        $('#harness-compact').onclick=async()=>{$('#harness-compact').disabled=true;$('#harness-cancel').disabled=false;$('#harness-feedback').textContent='正在压缩，请等待 Codex 完成…';try{await operate('compact');if(active()){$('#harness-feedback').textContent='上下文压缩已完成。';$('#harness-compact').disabled=false;}}catch(e){if(active())$('#harness-feedback').textContent=errorText(e);}finally{if(active())$('#harness-cancel').disabled=true;}};
      } else if(kind==='skills'||kind==='mcp') {
        const data=await operate(kind);if(!active())return;
        const rows=kind==='skills'?data.data.flatMap(d=>d.skills||[]):data.data;
        $('#harness-detail').insertAdjacentHTML('beforeend',`<p class="form-note">来自此 Harness 项目和运行账户的 Codex 配置。</p><div class="harness-capability-list">${rows.map(r=>`<section><strong>${esc(r.name)}</strong><p>${esc(kind==='skills'?r.description||'':`认证：${r.authStatus||'未报告'} · 工具：${Object.keys(r.tools||{}).length}`)}</p></section>`).join('')||'<p class="form-note">未发现可用项目。</p>'}</div>`);
        if(kind==='skills'&&data.data.some(d=>d.errors?.length))$('#harness-detail').insertAdjacentHTML('beforeend','<p class="form-note">部分 Skills 读取失败，请检查 Codex 项目配置。</p>');
      } else {
        $('#harness-detail').insertAdjacentHTML('beforeend',`<dl class="resource-details"><dt>模型设置</dt><dd>${esc(session.config.model||'Codex 默认')}</dd><dt>思考档位</dt><dd>${esc(session.config.reasoning_effort||'默认')}</dd><dt>文件权限</dt><dd>${session.config.sandbox==='read_only'?'只读':'工作区可写'}</dd><dt>权限升级</dt><dd>不自动批准</dd></dl><p class="form-note">这是此会话保存的配置快照。修改 Harness 默认配置只影响新会话。</p>`);
      }
    }catch(e){if(active())$('#harness-detail').innerHTML=`<p class="form-note" role="alert">${esc(errorText(e))}</p>`;}
  };
  $('#harness-target').onchange=load;await load();
}
async function showHarnessResourceSessions(reference) {
  dialog('Harness 执行会话','<p class="form-note">正在读取关联会话…</p>');
  const generation=modalGeneration;
  try{
    const records=await api('harness',{operation:'resource-sessions',reference});
    if(generation!==modalGeneration||!$('#modal').open)return;
    $('#modal-body').innerHTML=`<p class="form-note">${esc(reference)} · 仅展示你仍在其中的聊天。旧记录保留各自配置。</p><div class="harness-capability-list">${records.map(s=>`<section><strong>${esc(s.title||nameOf(s.principal))}</strong><p>${esc(s.config.model||'Codex 默认')} · ${esc(s.config.working_directory)}</p><p class="session-id">${esc(s.session_id||'尚未执行')}</p><button class="secondary" data-harness-chat="${esc(s.conversation)}">打开聊天</button></section>`).join('')||'<p class="form-note">尚无可见执行会话。在聊天中使用 /model 或发送消息后会建立记录。</p>'}</div>`;
    $$('[data-harness-chat]').forEach(b=>b.onclick=async()=>{closeModal();switchPage('chat');await conversations();await selectConversation(b.dataset.harnessChat);});
  }catch(e){if(generation===modalGeneration)modalError(e);}
}
