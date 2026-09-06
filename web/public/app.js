const $ = selector => document.querySelector(selector);
const $$ = selector => [...document.querySelectorAll(selector)];
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const labels = { provider: 'Provider', model: '模型', prompt: 'Prompt', skill: 'Skills', mcp: 'MCP', agent: 'Agent' };
const stateNames = { completed: '已完成', running: '执行中', failed: '失败', canceled: '已取消', succeeded: '成功', started: '结果待确认', idle: '等待执行', skipped: '已跳过', pending: '待执行', processing: '处理中' };
let token = location.hash.slice(1) || sessionStorage.getItem('agt-token');
if (location.hash) { sessionStorage.setItem('agt-token', token); history.replaceState(null, '', location.pathname); }
const state = { boot: null, viewer: null, conversation: null, conversations: [], members: [], messages: [], cursor: null, resourceKind: 'model', runs: [], job: {}, polling: false, sending: false, pendingSend: null, modalBusy: false, generation: 0, page: 'chat' };
const directNames = new Map();
let messageRequest = 0, conversationRequest = 0, modalGeneration = 0;

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
const nameOf = id => state.boot?.principals.find(p => p.id === id)?.name || id;
const dateTime = value => new Date(value).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
const empty = (title, description) => `<div class="empty compact"><h2>${esc(title)}</h2><p>${esc(description)}</p></div>`;
function notify(message, error = false) { $('#notice').textContent = message; $('#notice').className = `notice${error ? ' error' : ''}`; }
function errorText(error) { return `${error.message}${error.field ? ` · 字段：${error.field}` : ''}${error.code ? ` [${error.code}]` : ''}`; }
function modalError(error) { $('#modal-error').textContent = errorText(error); $('#modal-error').classList.remove('hidden'); }
function options(values, selected = '') { return values.map(([value, label]) => `<option value="${esc(value)}" ${value === selected ? 'selected' : ''}>${esc(label)}</option>`).join(''); }
function field(label, content, hint = '') { return `<label class="field"><span>${esc(label)}</span>${content}${hint ? `<small>${esc(hint)}</small>` : ''}</label>`; }
function checks(values, name) { return `<div class="check-list">${values.map(([value, label]) => `<label><input type="checkbox" name="${esc(name)}" value="${esc(value)}">${esc(label)}</label>`).join('') || '<small>暂无可用资源</small>'}</div>`; }
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
function setupReadOnly() {
  const observing = state.viewer !== state.boot?.principal;
  $('#observe-banner').classList.toggle('hidden', !observing);
  $('#composer').classList.toggle('hidden', !state.conversation || observing);
}
async function bootstrap() {
  const boot = await api('bootstrap'); state.boot = boot;
  if (!state.viewer || !boot.principals.some(p => p.id === state.viewer)) state.viewer = boot.principal;
  $('#viewer').innerHTML = options(boot.principals.map(p => [p.id, p.id === boot.principal ? '我 · 操作员' : `只读观察 · ${p.name || p.id}`]), state.viewer);
  $('#home-path').textContent = boot.home; $('#home-path').title = boot.home;
  $('#operator-name').textContent = nameOf(boot.principal);
  $('#agent-count').textContent = items('agent').length;
  renderAgents(); renderResources(); setupReadOnly();
}
async function conversations() {
  const request = ++conversationRequest;
  const viewer = state.viewer;
  const list = await read('conversation_list');
  if (state.viewer !== viewer || request !== conversationRequest) return;
  await Promise.all(list.filter(x => x.conversation.kind === 'direct' && !directNames.has(`${viewer}:${x.conversation.id}`)).slice(0, 20).map(async x => {
    const members = await read('member_list', { conversation: x.conversation.id }, viewer);
    directNames.set(`${viewer}:${x.conversation.id}`, members.filter(m => m.principal_id !== viewer && m.left_message_seq === null).map(m => nameOf(m.principal_id)).join('、'));
  }));
  if (state.viewer !== viewer || request !== conversationRequest) return;
  state.conversations = list;
  if (state.conversation && !list.some(x => x.conversation.id === state.conversation.id)) clearConversation();
  renderConversations();
}
function conversationName(c) {
  return c.title || directNames.get(`${state.viewer}:${c.id}`) || `私聊 · ${c.id.slice(0, 8)}`;
}
function renderConversations() {
  const query = $('#search').value.toLowerCase();
  const list = state.conversations.filter(x => conversationName(x.conversation).toLowerCase().includes(query));
  $('#conversation-count').textContent = state.conversations.length;
  $('#conversations').innerHTML = list.map(({ conversation: c, member_count }) => `<button class="conversation-item ${state.conversation?.id === c.id ? 'selected' : ''}" data-conv="${esc(c.id)}"><span class="avatar">${c.kind === 'group' ? '▧' : '↗'}</span><span><strong>${esc(conversationName(c))}</strong><small>${c.kind === 'group' ? '群聊' : '私聊'} · ${member_count} 位参与者${c.state !== 'active' ? ' · 已归档' : ''}</small></span></button>`).join('') || empty('暂无会话', query ? '试试其他关键词' : '点击「新建会话」开始');
  $$('[data-conv]').forEach(button => button.onclick = () => selectConversation(button.dataset.conv).catch(e => notify(errorText(e), true)));
}
function clearConversation() { state.conversation = null; state.members = []; state.messages = []; state.cursor = null; state.generation++; renderMessages(); renderChatHead(); setupReadOnly(); }
async function selectConversation(id) {
  if (state.sending) { notify('消息正在发送，请稍后切换会话。'); return; }
  state.conversation = state.conversations.find(x => x.conversation.id === id)?.conversation;
  if (!state.conversation) return;
  state.generation++; state.messages = []; state.members = []; state.cursor = null;
  $('#message-body').value = ''; state.pendingSend = null;
  renderConversations(); renderChatHead(); setupReadOnly();
  $('#messages').innerHTML = empty('正在读取…', '消息保存在本地');
  const generation = state.generation;
  const members = await read('member_list', { conversation: id });
  if (generation !== state.generation) return;
  state.members = members.filter(m => m.left_message_seq === null);
  renderChatHead(); renderMentions(); await messages(true);
}
function renderChatHead() {
  const c = state.conversation;
  if (!c) { $('#chat-head').innerHTML = '<strong>选择一个会话</strong><span class="muted">PRIVATE / GROUP</span>'; return; }
  const title = c.kind === 'direct' && state.members.length ? state.members.map(m => nameOf(m.principal_id)).join(' ↔ ') : conversationName(c);
  $('#chat-head').innerHTML = `<div><strong>${esc(title)}</strong><small>${state.members.map(m => esc(nameOf(m.principal_id))).join(' · ')}</small></div><span class="muted">${c.kind === 'group' ? 'GROUP' : 'PRIVATE'}</span>`;
}
function renderMentions() {
  const candidates = state.members.filter(m => m.principal_id !== state.boot.principal && state.boot.bindings.some(b => b.principal === m.principal_id));
  $('#mentions').innerHTML = candidates.map(m => `<label><input type="checkbox" value="${esc(m.principal_id)}" checked>${esc(nameOf(m.principal_id))}</label>`).join('') || '<span>暂无托管 Agent</span>';
}
async function messages(force = false, older = false) {
  const c = state.conversation; if (!c) return;
  const request = ++messageRequest;
  const generation = state.generation;
  const page = await read('message_history', { conversation: c.id, limit: 50, ...(older ? { before: state.cursor } : {}) });
  if (generation !== state.generation || request !== messageRequest) return;
  const oldIds = state.messages.map(m => `${m.id}:${m.deleted_at}`).join('|');
  if (older) { state.messages = [...page.messages, ...state.messages.filter(m => !page.messages.some(n => n.id === m.id))]; state.cursor = page.next_cursor; }
  else if (state.messages.length > 50) {
    const known = new Map(state.messages.map(m => [m.id, m])); page.messages.forEach(m => known.set(m.id, m));
    state.messages = [...known.values()].sort((a, b) => a.seq - b.seq);
  } else { state.messages = page.messages; state.cursor = page.next_cursor; }
  const box = $('#messages'); const nearBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 100;
  if (force || older || oldIds !== state.messages.map(m => `${m.id}:${m.deleted_at}`).join('|')) {
    const previousHeight = box.scrollHeight; const top = box.scrollTop;
    renderMessages();
    if (older) box.scrollTop = box.scrollHeight - previousHeight + top;
    else if (nearBottom || force) box.scrollTop = box.scrollHeight;
  }
  $('#older').classList.toggle('hidden', !state.cursor);
}
function renderMessages() {
  if (!state.conversation) { $('#messages').innerHTML = empty('给协作一个起点', '新建私聊或群聊，邀请你的 Agent 团队。'); return; }
  $('#messages').innerHTML = state.messages.map(m => {
    const own = m.sender_id === state.boot.principal;
    return `<article class="message ${own ? 'own' : ''}"><div class="avatar ${own ? 'user' : ''}">${esc(own ? '你' : nameOf(m.sender_id).slice(0, 1).toUpperCase())}</div><div class="message-content"><div class="message-meta"><span>${esc(nameOf(m.sender_id))}</span><time>${dateTime(m.created_at)}</time></div><div class="bubble">${esc(m.deleted_at ? '此消息已删除' : m.body)}</div><div class="message-tools"><span>${(m.metadata?.mentions || []).map(p => `@${esc(nameOf(p))}`).join(' ')}</span><button data-status="${esc(m.id)}">查看投递 ↗</button></div></div></article>`;
  }).join('') || empty('会话已就绪', '发送第一条消息，邀请 Agent 回应。');
  $$('[data-status]').forEach(button => button.onclick = () => showStatus(button.dataset.status));
}
async function showStatus(id) {
  dialog('消息投递', '<p class="form-note">正在读取各收件人的处理状态…</p>');
  const generation = modalGeneration;
  try {
    const rows = await read('message_status', { message_id: id });
    if (generation !== modalGeneration || !$('#modal').open) return;
    $('#modal-body').innerHTML = '<p class="form-note">投递完成不等于目标达成。可继续检查 Run 中的工具失败。</p>' + rows.map(r => `<div class="call-row"><strong>${esc(nameOf(r.principal))}</strong> · ${esc(stateNames[r.state] || r.state)}${r.error ? `<p class="error-text">${esc(r.error)}</p>` : ''}${r.run_id ? `<button class="quiet" data-run="${esc(r.run_id)}">查看运行 ↗</button>` : ''}</div>`).join('');
    $$('[data-run]').forEach(b => b.onclick = () => showCalls(b.dataset.run));
  } catch (e) { if (generation === modalGeneration && $('#modal').open) modalError(e); }
}
async function activity() {
  const data = await api('activity'); state.runs = data.runs; state.job = data.job;
  $('#execute').disabled = data.job.state === 'running';
  $('#execute').textContent = data.job.state === 'running' ? '执行中…' : '▷ 执行一批';
  $('#worker-state').textContent = stateNames[data.job.state] || data.job.state;
  $('#worker-detail').textContent = data.job.state === 'running' ? '正在处理当前批次。发送新消息不会自动追加下一批；请等待返回。' : data.job.state === 'failed' ? data.job.error : data.job.state === 'completed' ? `上批处理 ${data.job.outcomes.length} 条投递。可能包含跳过或失败；后续消息需要再次执行。` : '消息先进入收件箱。由你决定何时调用模型，保持实验可控。';
  $('#run-count').textContent = data.runs.length; $('#failure-count').textContent = data.runs.reduce((n, r) => n + r.tool_failures, 0);
  $('#runs').innerHTML = data.runs.slice(0, 15).map(r => `<button class="run-item" data-run="${esc(r.id)}"><span class="run-dot ${esc(r.state)}"></span><span><strong>${esc(r.agent)}</strong><small>${esc(stateNames[r.state])} · ${r.model_steps} 次模型调用${r.tool_failures ? ` · ${r.tool_failures} 次工具失败` : ''}</small></span></button>`).join('') || '<div class="empty compact"><p>还没有运行记录。<br>发送消息后，点击右上角执行。</p></div>';
  $$('#runs [data-run]').forEach(b => b.onclick = () => showCalls(b.dataset.run));
}
async function showCalls(id) {
  dialog('调用记录', `<p class="form-note">${esc(id)}<br>仅保存参数形状与安全错误，不保存原始参数、模型推理或工具返回正文。</p><div id="calls-list"></div><button id="more-calls" class="quiet">加载调用</button>`);
  const generation = modalGeneration;
  let after = 0;
  async function load() {
    const button = $('#more-calls'); if (!button) return; button.disabled = true;
    try {
      const rows = await api('calls', { id, after });
      if (generation !== modalGeneration || !$('#modal').open) return;
      $('#calls-list').insertAdjacentHTML('beforeend', rows.map(r => `<details class="call-row"><summary>${esc(r.kind === 'model' ? '模型调用' : r.tool_name)} · ${esc(stateNames[r.state])} <small>${dateTime(r.started_at)}</small></summary>${r.error ? `<p class="error-text">${esc(r.error.code)} · ${esc(r.error.message)}${r.error.field ? ` · ${esc(r.error.field)}` : ''}</p>` : ''}<pre>${esc(JSON.stringify(r.argument_summary, null, 2))}</pre></details>`).join('') || '<p class="form-note">没有更多调用记录；旧版本 Run 不会补造日志。</p>');
      after = rows.at(-1)?.id || after; button.classList.toggle('hidden', rows.length < 100); button.textContent = '加载更多调用';
    } catch (e) { if (generation === modalGeneration && $('#modal').open) modalError(e); } finally { button.disabled = false; }
  }
  $('#more-calls').onclick = load; await load();
}
function showWorker() {
  dialog('执行这一批任务', `<form id="modal-form"><div class="form-note danger-note">这会调用配置中的真实模型，可能产生费用。执行范围是此 home 下的所有托管 Agent，不仅是当前会话。一个 Agent 每次运行仍受自己的步数和深度限制。</div>${field('每位 Agent 本批最多处理的投递数', '<input name="limit" type="number" min="1" max="10" value="1" required>', '建议先从 1 开始。处理后可以查看结果，再执行下一批。')}<div class="check-list"><label><input name="mcp" type="checkbox">允许已获绑定授权的外部 MCP 执行</label></div><p class="form-note">本页不强杀执行中的工具。若关闭服务中断运行，可能留下结果不确定的 started/running，需要人工核验，不能盲目重试。</p><div class="form-actions"><button class="primary" type="submit">确认并执行 · 可能付费</button></div></form>`);
  formHandler(async data => { await api('worker', { limit: Number(data.get('limit')), allow_mcp: data.has('mcp'), confirm: true }); $('#modal').close(); await activity(); });
}
function showNewChat() {
  const people = state.boot.principals.filter(p => p.id !== state.boot.principal);
  dialog('新建会话', `<form id="modal-form"><div class="form-note">以「${esc(nameOf(state.boot.principal))}」创建会话。对方需要托管绑定才能由 worker 自动回应。</div>${field('会话类型', '<select name="kind"><option value="group">群聊 · 多方讨论</option><option value="direct">私聊 · 一对一</option></select>')}${field('群聊名称', '<input name="title" placeholder="例如：产品方案讨论" maxlength="120">')}${field('参与者', checks(people.map(p => [p.id, p.name || p.id]), 'members'), '私聊请选择一人；群聊至少选择一人。')}<div class="form-actions"><button class="primary" type="submit">创建会话</button></div></form>`);
  formHandler(async data => {
    const members = data.getAll('members'); const kind = data.get('kind');
    if (!members.length || (kind === 'direct' && members.length !== 1)) throw new Error('请选择参与者；私聊只能选择一人。');
    if (kind === 'group' && !data.get('title').trim()) throw new Error('请填写群聊名称。');
    const c = await tool(kind === 'group' ? 'group_create' : 'direct_create', kind === 'group' ? { title: data.get('title').trim(), members } : { principal: members[0] });
    state.viewer = state.boot.principal; $('#viewer').value = state.viewer; await conversations(); await selectConversation(c.id); switchPage('chat'); $('#modal').close();
  });
}
function renderAgents() {
  $('#agents').innerHTML = items('agent').map(a => {
    const binding = state.boot.bindings.find(b => b.agent === ref(a));
    const model = a.model ? `${a.model.category || 'default'}/${a.model.name}` : '未配置模型';
    return `<article class="agent-card"><div class="card-head"><div class="avatar">${esc(a.name.slice(0, 1).toUpperCase())}</div><div><h3>${esc(a.name)}</h3><small>${esc(model)}</small></div></div><p>${esc(a.description || '一个等待任务的独立角色。')}</p><span class="tag ${binding ? 'good' : ''}">${binding ? '已托管' : '未绑定'}</span><span class="tag">${a.skills?.length || 0} Skills · ${a.mcps?.length || 0} MCP</span>${binding ? `<span class="tag">深度 ${binding.max_depth} · ${binding.max_steps} 步</span>` : ''}<div class="card-actions"><button class="secondary" data-bind="${esc(ref(a))}">${binding ? '托管设置' : '配置托管'}</button>${binding ? `<button class="quiet" data-direct="${esc(binding.principal)}">发起私聊 ↗</button>` : ''}</div></article>`;
  }).join('') || empty('创建你的第一位 Agent', '先在资源库配置模型，再为 Agent 指定角色与能力。');
  $$('[data-bind]').forEach(b => b.onclick = () => showBinding(b.dataset.bind));
  $$('[data-direct]').forEach(b => b.onclick = async () => { try { const c = await tool('direct_create', { principal: b.dataset.direct }); state.viewer = state.boot.principal; $('#viewer').value = state.viewer; switchPage('chat'); await conversations(); await selectConversation(c.id); } catch (e) { notify(errorText(e), true); } });
}
function bindingFields(binding = {}) {
  return `<div class="form-grid">${field('响应策略', `<select name="policy">${options([['mentioned', '私聊 / 群内被提及'], ['all', '所有消息'], ['manual', '不自动回应']], binding.policy || 'mentioned')}</select>`)}${field('自动消息最大深度', `<input name="depth" type="number" value="${binding.max_depth || 3}" min="1" max="8" required>`)}</div><div class="form-grid">${field('每次运行最大模型步数', `<input name="steps" type="number" value="${binding.max_steps || 8}" min="1" max="100" required>`)}${field('执行工作目录', `<input name="cwd" value="${esc(binding.working_directory || state.boot.home)}" required>`)}</div><div class="check-list"><label><input type="checkbox" name="allow_mcp" ${binding.allow_mcp ? 'checked' : ''}>允许挂载 MCP（执行批次时仍需再次授权）</label></div>`;
}
const bindingData = (data, principal, agent) => ({ principal, agent, working_directory: data.get('cwd'), policy: data.get('policy'), max_steps: Number(data.get('steps')), max_depth: Number(data.get('depth')), allow_mcp: data.has('allow_mcp') });
function showBinding(agent) {
  const binding = state.boot.bindings.find(b => b.agent === agent);
  dialog('托管设置', `<form id="modal-form"><p class="form-note">${esc(agent)} · 修改只影响后续尚未准备的投递，不取消已经运行的调用。</p>${field('Agent 身份 ID', `<input name="principal" value="${esc(binding?.principal || agent.split('/').at(-1))}" required ${binding ? 'readonly' : ''}>`, '身份不存在时会创建 Agent 身份；已有绑定会更新。')}${bindingFields(binding)}<div class="form-actions">${binding ? '<button id="unbind" type="button" class="quiet">停止托管</button>' : ''}<button class="primary" type="submit">保存设置</button></div></form>`);
  formHandler(async data => { const principal = data.get('principal').trim(); await ensurePrincipal(principal, principal); await tool('agent_bind', bindingData(data, principal, agent)); $('#modal').close(); await bootstrap(); notify('托管设置已保存。'); });
  if (binding) $('#unbind').onclick = async () => { if (!confirm('停止托管不会删除 Agent 或消息，也不会取消正在运行的调用。确认？')) return; try { await tool('agent_unbind', { principal: binding.principal }); $('#modal').close(); await bootstrap(); } catch (e) { modalError(e); } };
}
async function ensurePrincipal(id, name) {
  const exists = state.boot.principals.find(p => p.id === id);
  if (exists && exists.kind !== 'agent') throw new Error('这个身份已被非 Agent 参与者占用，请换一个 ID。');
  if (!exists) { await tool('principal_create', { id, name, kind: 'agent' }); state.boot.principals.push({ id, name, kind: 'agent' }); }
}
function showAgent() {
  const models = items('model').filter(m => m.api_format === 'chat-completions');
  dialog('创建一个新角色', `<form id="modal-form"><div class="form-note">只创建你命名的新资源。现有模型中仅 Chat Completions 可用于当前运行时；Responses 模型请先创建兼容别名。</div><div class="form-grid">${field('资源名称 / 身份 ID', '<input name="name" placeholder="如 researcher" pattern="[A-Za-z0-9][A-Za-z0-9_-]*" maxlength="60" required>')}${field('显示说明', '<input name="description" placeholder="如：负责事实核验与独立分析" maxlength="200">')}</div>${field('模型', `<select name="model" required><option value="">选择模型…</option>${options(models.map(m => [ref(m), `${m.name} · ${m.model}`]))}</select>`, models.length ? '只挂载模型，不会立即调用。' : '暂无兼容模型。请先在资源库创建 Chat Completions 模型。')}${field('角色 Prompt', '<textarea name="prompt" required placeholder="你负责什么？如何与同伴沟通？何时应当停止或保持沉默？"></textarea>')}${field('Skills（可选）', checks(items('skill').map(s => [ref(s), s.name || s.package]), 'skills'))}${field('MCP（可选）', checks(items('mcp').map(m => [ref(m), m.name]), 'mcps'))}${bindingFields()}<div id="setup-progress" class="progress"></div><div class="form-actions"><button class="primary" type="submit">创建并托管</button></div></form>`);
  const done = new Set(); let lockedName = null;
  formHandler(async data => {
    const name = data.get('name').trim(); const prompt = `${name}-role`;
    if (lockedName && name !== lockedName) throw new Error('已有部分创建步骤完成。请保持名称重试，或关闭后创建另一个角色。');
    if (!lockedName && (items('agent').some(a => ref(a) === `default/${name}`) || items('prompt').some(p => ref(p) === `default/${prompt}`) || state.boot.principals.some(p => p.id === name))) throw new Error('资源或身份名称已存在。请换一个名称，原配置不会被覆盖。');
    lockedName = name;
    const step = async (key, action) => {
      if (!done.has(key)) { await action(); done.add(key); }
      const locked = ['name', ...(done.has('Prompt') ? ['prompt'] : []), ...(done.has('Agent') ? ['description', 'model', 'skills', 'mcps'] : [])];
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
    await step('Agent', () => tool('resource_write', { kind: 'agent', reference: name, create: true, data: { model: data.get('model'), prompt, description: data.get('description'), skills: data.getAll('skills'), mcps: data.getAll('mcps') } }));
    await step('身份', () => ensurePrincipal(name, name));
    await step('托管', () => tool('agent_bind', bindingData(data, name, name)));
    $('#modal').close(); await bootstrap(); notify(`${name} 已就绪。新建会话邀请它，再发送任务。`);
  });
}
function renderResources() {
  $('#resource-tabs').innerHTML = Object.entries(labels).filter(([k]) => k !== 'agent').map(([k, label]) => `<button data-kind="${k}" class="${k === state.resourceKind ? 'active' : ''}">${label} <small>${items(k).length}</small></button>`).join('');
  $$('[data-kind]').forEach(b => b.onclick = () => { state.resourceKind = b.dataset.kind; renderResources(); });
  const kind = state.resourceKind;
  const rows = items(kind);
  $('#resources').innerHTML = (kind === 'model' ? '<div class="resource-info warning">当前运行时支持 Chat Completions。Responses 配置可保留，但请创建兼容别名用于测试，不要覆盖原模型。</div>' : '') + (rows.map(r => `<div class="resource-row"><div><strong>${esc(r.name || r.package)}</strong><p>${esc(r.category || 'default')}</p></div><p>${esc(r.model || r.base_url || r.description || r.path || '可挂载资源')}</p><span class="tag ${r.api_format === 'responses' ? 'warn' : ''}">${esc(r.api_format || (r.api_key_configured !== undefined ? (r.api_key_configured ? '密钥已配置' : '未配置密钥') : kind.toUpperCase()))}</span></div>`).join('') || empty('资源库还是空的', '创建一个资源，或启动时指定已有的 --home。'));
  const diagnostics = state.boot.resources[kind]?.diagnostics;
  if (diagnostics?.length) $('#resources').insertAdjacentHTML('beforeend', `<p class="resource-info warning">有 ${diagnostics.length} 个资源读取异常。请在终端检查 agt ${esc(kind)} list；未显示的配置可能不可用。</p>`);
}
function showResource() {
  dialog('新建资源', `<form id="modal-form">${field('资源类型', `<select id="resource-kind" name="kind">${options(Object.entries(labels).filter(([k]) => k !== 'agent'), state.resourceKind)}</select>`)}${field('名称', '<input name="name" pattern="[A-Za-z0-9][A-Za-z0-9_-]*" maxlength="60" placeholder="新的资源名称" required>')}<div id="resource-fields"></div><div class="form-actions"><button class="primary" type="submit">创建资源</button></div></form>`);
  function render() {
    const kind = $('#resource-kind').value;
    $('#resource-fields').innerHTML = kind === 'model' ? `${field('Provider', `<select name="provider" required><option value="">选择 Provider…</option>${options(items('provider').map(p => [ref(p), p.name]))}</select>`)}${field('Provider 侧模型名称', '<input name="model" placeholder="例如 your-model-name" required>')}<div class="form-grid">${field('推理强度', '<select name="reasoning"><option value="low">Low · 低</option><option value="medium">Medium · 中</option><option value="high">High · 高</option><option value="">不指定</option></select>')}${field('请求超时（秒）', '<input type="number" name="timeout" value="90" min="1" max="600" required>')}</div><p class="form-note">新别名使用 Chat Completions，启用工具调用，输出上限 2000 token。Provider 需支持此模型的该接口。</p>` : kind === 'provider' ? `${field('Base URL', '<input name="url" type="url" placeholder="https://example.com/v1" required>')}${field('密钥环境变量名', '<input name="env" placeholder="MODEL_API_KEY" required>', '启动 Node 服务前设置该环境变量。页面不接收或显示密钥原文。')}` : kind === 'prompt' ? field('Prompt 正文', '<textarea name="text" required placeholder="写下角色指令…"></textarea>') : kind === 'skill' ? field('Skill 描述', '<textarea name="description" required placeholder="描述这个 Skill 的职责。创建后可用 CLI 完善 SKILL.md。"></textarea>') : field('单服务器 mcpServers JSON', '<textarea name="json" required placeholder=\'{"mcpServers":{"你的资源名称":{"command":"..."}}}\'></textarea>', '内部服务器名必须与资源名一致。仅挂载可信工具；这里不会执行。');
  }
  $('#resource-kind').onchange = render; render();
  formHandler(async data => {
    const kind = data.get('kind'); let value;
    if (kind === 'model') value = { provider: data.get('provider'), model: data.get('model'), params: { api_format: 'chat-completions', tool_call: true, max_tokens: 2000, timeout: Number(data.get('timeout')), ...(data.get('reasoning') ? { reasoning_effort: data.get('reasoning') } : {}) } };
    else if (kind === 'provider') value = { base_url: data.get('url'), api_key_env: data.get('env') };
    else if (kind === 'prompt') value = { text: data.get('text') };
    else if (kind === 'skill') value = { description: data.get('description') };
    else { try { value = JSON.parse(data.get('json')); } catch { throw new Error('MCP 配置不是有效 JSON。'); } }
    await tool('resource_write', { kind, reference: data.get('name').trim(), create: true, data: value });
    state.resourceKind = kind; $('#modal').close(); await bootstrap(); notify('资源已创建，原有配置未改动。');
  });
}
function switchPage(page) { state.page = page; $$('.page').forEach(p => p.classList.toggle('hidden', p.id !== `page-${page}`)); $$('.nav').forEach(b => b.classList.toggle('active', b.dataset.page === page)); $('#page-label').textContent = { chat: '会话空间', agents: 'Agent 团队', resources: '资源库' }[page]; }
function help() { dialog('从这里开始你的实验', `<div class="guide"><p>这是通用的多 Agent 工作台。讨论、审查、博弈，都是同一套会话能力的用法。</p><ol><li><strong>准备模型。</strong>资源库会读取启动时的 home。已有模型时，若模型是 Responses，请创建新的 Chat Completions 别名。</li><li><strong>创建角色。</strong>在 Agent 团队中设置角色 Prompt、模型、挂载资源和托管策略。</li><li><strong>建立会话。</strong>私聊选一人，群聊选多人。发送消息时勾选需要回应的 Agent。</li><li><strong>明确执行。</strong>点击「执行一批」，确认付费和处理范围。新产生的后续消息可能需要再执行一批。</li><li><strong>观察与推进。</strong>切换查看身份可只读观察 Agent 私聊；回到操作员身份推动下一阶段。点击消息投递或运行条目检查错误。</li></ol><h3>几个重要边界</h3><p>界面是可信主机管理工具。观察身份不是多用户登录。阶段仍由你组织，没有游戏专属逻辑。默认深度 3 不支持无限接话。关闭服务可能中断调用；不要盲目重放结果不确定的操作。</p><h3>资源目录</h3><p>${esc(state.boot.home)}</p></div>`); }

$('#close-modal').onclick = closeModal;
$('#modal').addEventListener('cancel', e => { if (state.modalBusy) e.preventDefault(); });
$$('[data-page]').forEach(b => b.onclick = () => switchPage(b.dataset.page));
$('#help').onclick = help; $('#new-agent').onclick = showAgent; $('#new-resource').onclick = showResource;
$('#new-chat').onclick = showNewChat; $('#empty-new-chat').onclick = showNewChat; $('#execute').onclick = showWorker;
$('#search').oninput = renderConversations;
$('#viewer').onchange = async () => { state.viewer = $('#viewer').value; clearConversation(); try { await conversations(); } catch (e) { notify(errorText(e), true); } };
$('#older').onclick = () => messages(false, true).catch(e => notify(errorText(e), true));
$('#refresh').onclick = async () => { try { await bootstrap(); await conversations(); await messages(true); await activity(); notify('已同步本地状态。'); } catch (e) { notify(errorText(e), true); } };
$('#composer').onsubmit = async event => {
  event.preventDefault(); if (state.sending || !state.conversation || state.viewer !== state.boot.principal) return;
  const body = $('#message-body').value.trim(); if (!body) return;
  const args = { conversation: state.conversation.id, body, mentions: $$('#mentions input:checked').map(i => i.value) };
  const signature = JSON.stringify(args);
  if (state.pendingSend?.signature !== signature) state.pendingSend = { signature, key: crypto.randomUUID() };
  state.sending = true; $('#send').disabled = true; $('#viewer').disabled = true; $('#message-body').disabled = true;
  try {
    await tool('message_send', { ...args, request_key: state.pendingSend.key });
    $('#message-body').value = ''; state.pendingSend = null;
    notify('消息已保存。准备好后点击「执行一批」让托管 Agent 回应。');
    await messages(true); await conversations();
  } catch (e) { notify(errorText(e), true); }
  finally { state.sending = false; $('#send').disabled = false; $('#viewer').disabled = false; $('#message-body').disabled = false; }
};
$('#message-body').onkeydown = event => { if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) { event.preventDefault(); $('#composer').requestSubmit(); } };
async function poll() {
  if (state.polling || !state.boot || document.hidden) return;
  state.polling = true;
  try { await activity(); if (state.page === 'chat') { await conversations(); await messages(); } $('#connection').textContent = '本地已连接'; }
  catch (e) { $('#connection').textContent = '连接中断'; if (e.code === 'unauthorized') notify(errorText(e), true); }
  finally { state.polling = false; }
}
async function init() {
  try { await bootstrap(); await conversations(); await activity(); setupReadOnly(); $('#connection').textContent = '本地已连接'; }
  catch (e) { $('#connection').textContent = '未连接'; notify(errorText(e), true); $('#execute').disabled = true; }
}
init(); setInterval(poll, 4000);
