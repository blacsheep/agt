/* Read-only projection of identity-scoped messages and current execution snapshots. */
(() => {
  const batchSize = 6;
  const recent = new Map();
  const graph = createInteractionGraph((id, node) => {
    if(id?.kind==='task'){selected=null;selectedTask=id.id;render();showTask(id.id,node.taskIds);return;}
    selectedTask=null;
    selected = id; detailSignature = ''; render();
    if (id) showDetail();
    else {
      ++detailEpoch;
      $('#ix-detail').innerHTML = `<div class="ix-detail-heading"><span class="eyebrow">NETWORK DETAIL</span><h2>${esc(node.label)}</h2><p>${esc(node.status)}</p></div><section class="ix-message"><p class="ix-body">${esc(node.reason || '当前筛选范围内没有相关消息。')}</p>${node.approvalIds?.length?'<button id="ix-node-approvals" class="secondary">查看权限审批</button>':''}</section>`;
      $('#ix-node-approvals')?.addEventListener('click',()=>AgtApprovals.open(node.approvalIds[0]));
    }
  });
  let epoch = 0, viewer, entries = [], loaded = new Map(), selected = null;
  let workTasks=[], approvals=[], taskPages=1, tasksMore=false, selectedTask=null;
  let busy = false, refreshPending = false, detailEpoch = 0, detailSignature = '', status = '';
  const stamp = value => new Date(value).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });
  const active = generation => generation === epoch && state.page === 'interactions';
  const title = item => item.conversation.title || (item.conversation.kind === 'direct' ? '私聊' : '未命名群聊');
  const option = (value, text) => `<option value="${esc(value)}">${esc(text)}</option>`;
  function selectOptions(id, html) {
    const node = $(id), value = node.value;
    if (node.innerHTML !== html) { node.innerHTML = html; node.value = value; if (node.selectedIndex < 0) node.selectedIndex = 0; }
  }
  function rows() {
    return [...loaded.values()].flatMap(item => item.messages.map(message => ({ item, message })))
      .sort((a, b) => b.message.created_at - a.message.created_at || b.message.seq - a.message.seq);
  }
  function target(item, message) {
    if (item.conversation.kind !== 'direct') return `群聊 · ${title(item)}`;
    const peers = item.members.filter(m => m.principal_id !== message.sender_id);
    return peers.length === 1 ? nameOf(peers[0].principal_id) : '私聊（对象未确认）';
  }
  function render() {
    const all = rows();
    selectOptions('#ix-agent', option('', '所有参与者') + [...new Set([...all.map(x => x.message.sender_id),...workTasks.flatMap(t=>[t.request.requester,t.request.assignee])])].map(id => option(id, nameOf(id))).join(''));
    selectOptions('#ix-conversation', option('', '已载入的会话') + entries.map(x => option(x.conversation.id, title(x))).join(''));
    const query = $('#ix-query').value.trim().toLowerCase(), agent = $('#ix-agent').value, conversation = $('#ix-conversation').value, kind = $('#ix-kind').value;
    const filtered = all.filter(({ item, message: m }) => kind!=='task' && (!agent || m.sender_id === agent) && (!conversation || item.conversation.id === conversation)
      && (!kind || (principalOf(m.sender_id)?.kind === 'agent' ? 'agent' : 'human') === kind)
      && (!query || `${m.body} ${nameOf(m.sender_id)} ${title(item)}`.toLowerCase().includes(query)));
    const work=workTasks.filter(t=>(!agent||[t.request.requester,t.request.assignee].includes(agent))&&(!conversation||t.request.report_to===conversation)&&(!kind||kind==='task')&&(!query||`${t.request.input} ${nameOf(t.request.requester)} ${nameOf(t.request.assignee)}`.toLowerCase().includes(query)));
    const visibleApprovals=approvals.filter(a=>(!conversation||a.conversation===conversation)&&(!agent||a.principal===agent)&&!query&&!kind);
    graph.update(filtered, state.scheduler.tasks || [], state.scheduler.online, recent, selectedTask ? {kind:'task',id:selectedTask} : selected, work, visibleApprovals);
    $('#ix-tasks-more').classList.toggle('hidden',!tasksMore);
    $('#ix-tasks-more').disabled=busy;
    $('#ix-approvals').textContent='权限审批'+(approvals.some(AgtApprovals.pending)?' · 待处理':'');
    $('#ix-count').textContent = `${filtered.length} 条消息 / ${work.length} 项任务 · 已载入 ${all.length} 条消息`;
    $('#ix-status').textContent = status || `已载入 ${loaded.size}/${entries.length} 个会话，每个会话初次读取最近 30 条；执行状态为当前快照。`;
    $('#ix-refresh').disabled = busy;
    $('#ix-more').classList.toggle('hidden', loaded.size >= entries.length);
    $('#ix-more').disabled = busy;
    const current = loaded.get(conversation);
    $('#ix-earlier').classList.toggle('hidden', !current?.cursor);
    $('#ix-earlier').disabled = busy;
    const markup = work.map(t=>`<button class="ix-row" data-ix-task="${esc(t.id)}"><span><strong>任务委派 · ${esc(nameOf(t.request.requester))} → ${esc(nameOf(t.request.assignee))}</strong><span class="ix-preview">${esc(t.request.input.slice(0,180))}</span><small>${esc(t.execution_status||t.state)}</small></span></button>`).join('') + filtered.map(({ item, message: m }) => `<button class="ix-row ${selected === m.id ? 'selected' : ''}" data-ix-message="${esc(m.id)}" aria-pressed="${selected === m.id}">
      <span class="ix-dot" aria-hidden="true">${esc(nameOf(m.sender_id).slice(0, 1))}</span>
      <span class="ix-row-main"><span class="ix-direction"><strong>${esc(nameOf(m.sender_id))}</strong><span aria-hidden="true"> → </span><span>${esc(target(item, m))}</span></span>
      <span class="ix-preview">${esc(m.deleted_at ? '消息已删除' : m.body)}</span><span class="ix-meta">${esc(title(item))} · #${m.seq}</span></span>
      <time>${esc(stamp(m.created_at))}</time><span class="ix-hover">${esc((m.deleted_at ? '消息已删除' : m.body || '空消息').slice(0, 600))}</span></button>`).join('') || empty(busy ? '正在读取交互' : '没有匹配的消息', '可切换观察身份、清除筛选，或载入更多会话。');
    // Do not replace focused rows when an invalidation returns identical data.
    if ($('#ix-list').innerHTML !== markup) {
      const focused = document.activeElement?.dataset.ixMessage;
      $('#ix-list').innerHTML = markup;
      $$('#ix-list [data-ix-task]').forEach(b=>b.onclick=()=>{selected=null;selectedTask=b.dataset.ixTask;render();showTask(selectedTask);});
      $$('#ix-list [data-ix-message]').forEach(button => button.onclick = () => {
        selectedTask=null; selected = button.dataset.ixMessage; detailSignature = ''; render(); showDetail();
        if (matchMedia('(max-width:1200px)').matches) $('#ix-detail').scrollIntoView({ block: 'start' });
      });
      if (focused) $$('#ix-list [data-ix-message]').find(b => b.dataset.ixMessage === focused)?.focus({ preventScroll: true });
    }
    if (!selected && !selectedTask) $('#ix-detail').innerHTML = '<div class="ix-detail-empty"><span aria-hidden="true">↗</span><h2>从一个节点开始</h2><p>点击 Agent 聚焦它的协作关系，点击连线查看最近交流。</p><p>悬停预览 · 拖动布局 · 滚轮缩放</p><p>执行光环来自当前运行状态；沿线流动表示刚收到新消息。</p></div>';
  }
  async function fetchConversation(entry, generation, earlier = false, live = false) {
    const id = entry.conversation.id, existing = loaded.get(id);
    const [history, members] = await Promise.all([
      read('message_history', { conversation: id, limit: 30, ...(earlier ? { before: existing.cursor } : {}) }, viewer),
      existing ? Promise.resolve(existing.members) : read('member_list', { conversation: id }, viewer)
    ]);
    if (!active(generation)) return;
    const messages = new Map((existing?.messages || []).map(m => [m.id, m]));
    const through = Math.max(0, ...(existing?.messages || []).map(m => m.seq));
    history.messages.forEach(m => {
      if (live && existing && m.seq > through && !messages.has(m.id)) recent.set(m.id, Date.now() + 6500);
      messages.set(m.id, m);
    });
    for (const [id, until] of recent) if (until < Date.now()) recent.delete(id);
    loaded.set(id, { ...entry, members, messages: [...messages.values()], cursor: earlier || !existing ? history.next_cursor : existing.cursor });
  }
  async function load(mode = 'initial') {
    if (state.page !== 'interactions') return;
    if (busy) { if (mode === 'refresh') refreshPending = true; return; }
    busy = true; const generation = epoch; status = '';  render();
    try {
      if (mode === 'initial' || mode === 'refresh' || mode==='tasks') {
        const next=new Map();let before;
        try {
          for(let page=0;page<taskPages;page++) {
            const rows=await read('task_list',{limit:100,...(before?{before}:{})},viewer);
            if(!active(generation))return;
            rows.forEach(t=>next.set(t.id,t));tasksMore=rows.length===100;before=rows.at(-1)?.list_cursor;
            if(!tasksMore||!before)break;
          }
          workTasks=[...next.values()];
        } catch(e) {if(!active(generation))return;workTasks=[];status='任务读取失败：'+e.message;}
        try {
          const nextApprovals=await api('approvals?'+new URLSearchParams({viewer}));
          if(!active(generation))return;approvals=nextApprovals;
        } catch(e) {if(!active(generation))return;approvals=[];status+=' 审批读取失败：'+e.message;}
        if(selectedTask&&!workTasks.some(t=>t.id===selectedTask)){selectedTask=null;detailSignature='';++detailEpoch;}
      }
      if (mode === 'initial' || mode === 'refresh') {
        const result = await read('conversation_list', { include_hidden: true }, viewer);
        if (!active(generation)) return;
        entries = result.sort((a, b) => b.conversation.updated_at - a.conversation.updated_at);
        const ids = new Set(entries.map(x => x.conversation.id));
        for (const id of loaded.keys()) if (!ids.has(id)) loaded.delete(id);
        if (!rows().some(x => x.message.id === selected)) { selected = null; ++detailEpoch; }
      }
      const requested = $('#ix-conversation').value;
      const batch = mode === 'earlier' ? entries.filter(x => x.conversation.id === requested)
        : mode === 'refresh' ? entries.filter((x, i) => loaded.has(x.conversation.id) || i < batchSize)
        : requested ? entries.filter(x => x.conversation.id === requested)
        : entries.filter(x => !loaded.has(x.conversation.id)).slice(0, batchSize);
      // Small batches bound concurrent CLI reads on large workspaces.
      let failures = 0;
      for (let i = 0; i < batch.length && active(generation); i += 3) {
        const results = await Promise.allSettled(batch.slice(i, i + 3).map(entry => fetchConversation(entry, generation, mode === 'earlier', mode === 'refresh')));
        failures += results.filter(r => r.status === 'rejected').length;
      }
      if (!active(generation)) return;
      if(failures)status=`${failures} 个会话读取失败，可点击刷新重试；当前记录可能不完整。`;
    } catch (e) { if (active(generation)) status = `读取失败：${e.message}`; }
    finally { if (active(generation)) { busy = false; render(); if (selected) showDetail(); if(selectedTask)showTask(selectedTask); AgtApprovals.refresh(); if (refreshPending) { refreshPending = false; load('refresh'); } } }
  }
  async function showDetail() {
    const row = rows().find(x => x.message.id === selected);
    if (!row) return;
    const generation = epoch, request = ++detailEpoch, { item, message: m } = row;
    const signature = JSON.stringify(m);
    if (signature !== detailSignature) {
      detailSignature = signature;
      $('#ix-detail').innerHTML = `<div class="ix-detail-heading"><span class="eyebrow">MESSAGE DETAIL</span><h2>${esc(nameOf(m.sender_id))}</h2><p>→ ${esc(target(item, m))}</p><small>${esc(stamp(m.created_at))} · #${m.seq}</small></div>
        <section class="ix-message"><h3>交流内容</h3><p class="ix-body">${esc((m.deleted_at ? '消息已删除' : m.body || '').slice(0, 160))}</p>${!m.deleted_at && m.body?.length > 160 ? `<details><summary>展开剩余 ${m.body.length - 160} 字</summary><p class="ix-body">${esc(m.body.slice(160))}</p></details>` : ''}</section>
        <section class="ix-execution"><h3>当前执行与公开理由</h3><p class="ix-note">这是此会话的近期状态快照，不代表这条消息发送时的状态，也不是模型内部思维链。</p><div id="ix-execution-body">读取中…</div></section>`;
    }
    try {
      const data = await api('activity?' + new URLSearchParams({ conversation: item.conversation.id, viewer }));
      if (!active(generation) || request !== detailEpoch) return;
      // Global activity responses must be restricted to the authorized selected conversation.
      const tasks = (data.scheduler.tasks || []).filter(t => t.conversation === item.conversation.id).sort((a, b) => b.due_at - a.due_at);
      const latest = [...new Map(tasks.map(t => t.principal).map(id => [id, tasks.find(t => t.principal === id)])).values()];
      const html = latest.map(t => {
        const decision = t.last_decision;
        const progress = data.runs.find(r => r.id === t.run_id && r.conversation === item.conversation.id)?.progress;
        return `<article class="ix-state"><div><strong>${esc(nameOf(t.principal))}</strong><span>${esc(stateNames[t.state] || ({waiting:'等待中',input:'已接入当前执行',responding:'回复中',queued:'排队中',silent:'不发言',evaluating:'判断中'})[t.state] || t.state)}</span></div>
          <p class="ix-note">框架状态 · ${data.scheduler.online ? '调度在线' : '调度离线，状态待确认'}</p>
          ${decision ? `<p><b>模型公开决策</b> · ${esc(({participate:'参与',wait:'等待',silent:'不发言'})[decision.action] || decision.action)}</p><p class="ix-body">${esc(decision.public_reason || '未记录公开理由')}</p><small>${esc(stamp(decision.decided_at))} · 截至消息 #${decision.through_seq}</small>` : '<p class="ix-note">未记录模型公开决策</p>'}
          ${progress ? `<p>${esc(progressLabels[progress.phase] || progress.phase)}</p>${activityTimeline(progress.activities || [])}` : ''}</article>`;
      }).join('') || '<p class="ix-note">此会话暂无可展示的执行记录。</p>';
      const box = $('#ix-execution-body');
      if (box.innerHTML !== html) box.innerHTML = html;
    } catch (e) { if (active(generation) && request === detailEpoch) $('#ix-execution-body').textContent = `状态读取失败：${e.message}`; }
  }
  async function showTask(id,related=[id]) {
    const generation=epoch,request=++detailEpoch;
    try {
      const t=await read('task_get',{task_id:id},viewer);
      if(!active(generation)||request!==detailEpoch||selectedTask!==id)return;
      const relevant=approvals.filter(a=>a.work_id===id);
      $('#ix-detail').innerHTML=`<div class="ix-detail-heading"><span class="eyebrow">TASK DELEGATION</span><h2>${esc(nameOf(t.request.requester))} → ${esc(nameOf(t.request.assignee))}</h2><p>${esc(t.execution_status||t.state)}${t.execution_engine ? ' · 执行引擎：'+esc(t.execution_engine) : ''}</p></div><section class="ix-message"><pre class="ix-body">${esc(t.request.input)}</pre><p class="ix-note">任务 ${esc(t.id)} · Run ${esc(t.run_id||'尚未执行')}</p>${t.output?`<h3>结果</h3><pre class="ix-body">${esc(t.output)}</pre>`:''}${related.length>1?related.map(other=>`<button class="secondary" data-related-task="${esc(other)}">${esc(other.slice(0,8))}</button>`).join(''):''}<h3>权限审批</h3>${relevant.map(a=>`<button class="secondary" data-task-approval="${esc(a.id)}">${esc(a.source==='harness'?'引擎原生':'Native')} · ${esc(AgtApprovals.labels[a.state]||a.state)}</button>`).join('')||'<p>当前已载入范围无审批记录。</p>'}</section>`;
      $$('[data-related-task]').forEach(b=>b.onclick=()=>{selectedTask=b.dataset.relatedTask;showTask(selectedTask,related)});
      $$('[data-task-approval]').forEach(b=>b.onclick=()=>AgtApprovals.open(b.dataset.taskApproval));
    }catch(e){if(active(generation)&&request===detailEpoch)$('#ix-detail').textContent='任务读取失败：'+e.message;}
  }
  function reset() {
    graph.reset(); recent.clear(); workTasks=[];approvals=[];selectedTask=null;taskPages=1;tasksMore=false;
    ++epoch; ++detailEpoch; loaded = new Map(); entries = []; selected = null; detailSignature = ''; status = ''; busy = false; refreshPending = false;
    for (const id of ['#ix-query', '#ix-agent', '#ix-conversation', '#ix-kind']) $(id).value = '';
    render();
  }
  function changeViewer() {
    reset(); viewer = $('#ix-viewer').value; state.viewer = viewer;
    state.conversations = []; clearConversation(); AgtApprovals.refresh(); ensureEvents(); load();
  }
  $('#ix-viewer').onchange = changeViewer;
  $('#ix-approvals').onclick=()=>AgtApprovals.open();
  $('#ix-tasks-more').onclick=()=>{taskPages++;load('tasks');};
  $('#ix-query').oninput = render;
  $('#ix-agent').onchange = render;
  $('#ix-kind').onchange = render;
  $('#ix-conversation').onchange = () => { render(); if ($('#ix-conversation').value && !loaded.has($('#ix-conversation').value)) load('more'); };
  $('#ix-clear').onclick = () => { for (const id of ['#ix-query','#ix-agent','#ix-conversation','#ix-kind']) $(id).value = ''; render(); };
  $('#ix-refresh').onclick = () => load(loaded.size ? 'refresh' : 'initial');
  $('#ix-more').onclick = () => { $('#ix-conversation').value = ''; load('more'); };
  $('#ix-earlier').onclick = () => load('earlier');
  window.AgtInteractions = {
    open() {
      $('#ix-viewer').innerHTML = state.boot.principals.filter(p => !p.deleted && (p.kind === 'agent' || p.id === state.boot.principal)).map(p => option(p.id, nameOf(p.id))).join('');
      $('#ix-viewer').value = state.boot.principal; changeViewer();
    },
    close() { reset(); },
    statusChanged() { if (state.page === 'interactions') render(); },
    refresh() { return load(loaded.size ? 'refresh' : 'initial'); }
  };
})();
