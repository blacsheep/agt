/* Task workspace: compact lists first, explicit decisions in the side panel. */
window.AgtTasks = (() => {
  let records = [], selected = null, loading = false, more = false, request = 0;
  const labels = {queued:'待处理',running:'执行中',waiting_dependency:'等待子任务',waiting_until:'定时等待',waiting_input:'等待答复',succeeded:'已完成',failed:'失败',canceled:'已取消',outcome_unknown:'结果待核对'};
  const priorities = {urgent:'紧急',high:'高',normal:'普通',low:'低'};
  const blockers = {waiting_input:'等待业务答复',waiting_for_task:'等待依赖结果',deferred_until:'等待指定时间',dependency_outcome_unknown:'依赖结果待核对',dependency_deadline_exceeded:'依赖等待已超时',disabled:'Agent 已停用',manual:'手动响应策略',expired:'讨论授权已过期'};
  const active = () => !$('#command-panel').classList.contains('hidden') && $('#command-panel').dataset.mode === 'tasks';
  function shell() {
    commandPanel('协作任务', `<div class="task-toolbar"><input id="task-search" class="search" placeholder="搜索任务、执行者…" aria-label="搜索任务"><div><select id="task-scope" aria-label="任务范围"><option value="current">当前会话</option><option value="all">全部可见任务</option>${isObserver() ? '' : '<option value="incidents">工作空间异常</option>'}</select><select id="task-filter" aria-label="任务状态"><option value="active">进行中</option><option value="attention">需要处理</option><option value="all">全部状态</option><option value="done">已结束</option></select></div></div><p id="task-count" class="form-note" role="status">正在读取…</p><div id="task-list"></div><button id="task-more" type="button" class="quiet hidden">加载更早任务</button>`, 'tasks');
    $('#command-panel-nav').innerHTML = '';
    $('#task-search').oninput = render;
    $('#task-filter').onchange = render;
    $('#task-scope').onchange = () => load();
    $('#task-more').onclick = () => load(true);
    if (!state.conversation) $('#task-scope').value = 'all';
  }
  async function load(append = false) {
    if (!active() || selected) return;
    const generation = ++request, viewer = state.viewer, chat = state.conversation?.id;
    const args = {limit:100};
    if ($('#task-scope')?.value === 'current' && chat) args.conversation = chat;
    if (append && records.length) args.before = records.at(-1).list_cursor;
    loading = true;
    try {
      const incidentMode = $('#task-scope')?.value === 'incidents';
      const rows = await read(incidentMode ? 'task_attention' : 'task_list',args,viewer);
      if (!active() || selected || generation !== request || viewer !== state.viewer || chat !== state.conversation?.id) return;
      records = append ? [...records,...rows] : rows;
      more = rows.length === 100;
      render();
    } catch (e) { if (active() && generation === request) commandPanelError(e); }
    finally { if (generation === request) loading = false; }
  }
  function render() {
    if (!active() || selected || !$('#task-list')) return;
    const search = $('#task-search').value.trim().toLowerCase(), filter = $('#task-filter').value;
    const incidents = $('#task-scope').value === 'incidents';
    $('#task-filter').disabled = incidents;
    if (incidents) {
      const reasons = {outcome_unknown:'结果待核对',publication_blocked:'结果投递受阻',report_blocked:'原会话汇报受阻',report_overdue:'超过 5 分钟尚未汇报',dependency_deadline_exceeded:'依赖等待已超时',requester_unavailable:'请求者已失效'};
      const rows = records.filter(item => `${item.id} ${nameOf(item.assignee)} ${nameOf(item.requester)}`.toLowerCase().includes(search));
      $('#task-count').textContent = `${rows.length} 项异常 · 仅显示最少元数据，不授予私聊访问权限`;
      $('#task-list').innerHTML = rows.map(item => `<button class="work-row" type="button" data-work="${esc(item.id)}" ${item.can_inspect ? '' : 'disabled'}><strong>${esc(item.reasons.map(r => reasons[r] || r).join(' · '))}</strong><p>${esc(nameOf(item.requester))} → ${esc(nameOf(item.assignee))}</p><small>${item.can_inspect ? '查看任务详情' : '无会话访问权限，请联系相关成员核查'} · ${esc(dateTime(item.updated_at))}</small></button>`).join('') || empty('暂无待处理异常','结果未知、汇报受阻及超时待办会在这里列出。');
      $$('#task-list [data-work]:not([disabled])').forEach(button => button.onclick = () => detail(button.dataset.work));
      $('#task-more').classList.toggle('hidden',!more); return;
    }
    const rows = records.filter(item => {
      const ended = ['succeeded','failed','canceled','outcome_unknown'].includes(item.state);
      const attention = item.state === 'waiting_input' || item.state === 'outcome_unknown' && !item.resolution || item.publication === 'blocked' || ['awaiting_report','blocked'].includes(item.report?.state) || item.requester_notification === 'pending' || item.blocked_reason === 'dependency_deadline_exceeded';
      return (filter === 'all' || filter === 'active' && !ended || filter === 'done' && ended || filter === 'attention' && attention) && `${item.request.input} ${nameOf(item.request.assignee)} ${nameOf(item.request.requester)} ${item.id}`.toLowerCase().includes(search);
    });
    $('#task-count').textContent = `${rows.length} 项符合条件 · 已加载 ${records.length} 项${isObserver() ? ' · 只读观察' : ''}`;
    $('#task-list').innerHTML = rows.map(item => `<button type="button" class="work-row" data-work="${esc(item.id)}"><div><span class="work-state ${esc(item.state)}">${esc(labels[item.state] || item.state)}</span><small>${esc(priorities[item.priority] || item.priority)}</small></div><strong>${esc(item.request.input.slice(0,180))}</strong><p>${esc(nameOf(item.request.requester))} → ${esc(nameOf(item.request.assignee))}</p><small>${esc(blockers[item.blocked_reason] || item.priority_reason || item.deferred?.reason || dateTime(item.updated_at))}</small></button>`).join('') || empty('这里暂时没有任务','普通聊天不会自动转成任务。可让挂载「任务协作」的 lead 委派工作，或调整筛选范围。');
    $$('#task-list [data-work]').forEach(button => button.onclick = () => detail(button.dataset.work));
    $('#task-more').classList.toggle('hidden',!more);
  }
  async function detail(id) {
    selected = id; const generation = ++request, viewer = state.viewer;
    try {
      const item = await read('task_get',{task_id:id},viewer);
      if (!active() || generation !== request || viewer !== state.viewer) return;
      const ended = ['succeeded','failed','canceled','outcome_unknown'].includes(item.state);
      const human = !isObserver();
      const canQueue = human && item.state === 'queued' && item.request.requester === state.boot.principal;
      const actions = [];
      if (human && item.report?.state === 'awaiting_report') actions.push(['report','汇报到原会话']);
      if (human && !ended && !item.cancellation_requested) actions.push(['cancel','取消任务']);
      if (human && item.state === 'waiting_input') actions.unshift(['reply','回答问题']);
      if (canQueue) actions.push(['priority','调整优先级'],['defer','延后执行']);
      if (human && item.state === 'outcome_unknown' && !item.resolution) actions.push(['resolve','记录核对结论']);
      if (human && (['failed','canceled'].includes(item.state) || item.resolution?.outcome === 'failed')) actions.push(['retry','创建重试任务']);
      $('#command-panel-body').innerHTML = `<div class="task-detail-tools"><button id="task-back" class="quiet" type="button">‹ 任务列表</button><button id="task-refresh" class="secondary" type="button">刷新状态</button></div><div class="work-detail"><div class="work-detail-state"><span class="work-state ${esc(item.state)}">${esc(labels[item.state] || item.state)}</span><small>${esc(priorities[item.priority])}优先级</small></div><h3>${esc(nameOf(item.request.assignee))}</h3><p class="form-note">来自 ${esc(nameOf(item.request.requester))} · ${esc(dateTime(item.created_at))}</p><details class="task-request" open><summary>任务要求</summary><pre>${esc(item.request.input)}</pre></details>${item.blocked_reason ? `<p class="task-callout">${esc(blockers[item.blocked_reason] || item.blocked_reason)}</p>` : ''}${item.deferred?.reason ? `<p>${esc(item.deferred.reason)}</p>` : ''}${item.cancellation_requested ? '<p class="task-callout">取消已请求，正在确认执行状态；已发生的外部操作不会撤销。</p>' : ''}${item.question ? `<div class="task-callout"><strong>需要补充信息</strong><p>${esc(item.question)}</p><small>截止 ${esc(new Date(item.deadline).toLocaleString())}</small></div>` : ''}${item.waiting_for_task ? `<p class="form-note">依赖任务：${esc(item.waiting_for_task)}</p>` : ''}${item.output || item.error || item.reason ? `<details class="task-request" open><summary>结果与说明</summary><pre>${esc(item.output || item.error || item.reason)}</pre></details>` : ''}${item.resolution ? `<p class="task-callout">人工核对：${item.resolution.outcome === 'succeeded' ? '成功' : '失败'}<br>${esc(item.resolution.evidence)}</p>` : ''}<div class="task-report"><span>结果投递</span><strong>${esc(({not_ready:'尚未就绪',pending:'待投递',published:'已送达',blocked:'投递受阻'})[item.publication] || item.publication)}</strong><span>原会话汇报</span><strong>${esc(({not_ready:'尚未就绪',awaiting_report:'等待汇报',pending:'待发布',published:'已汇报',blocked:'汇报受阻'})[item.report?.state] || '尚未就绪')}</strong><span>请求者处理</span><strong>${esc(({pending:'待处理',handled:'已交接'})[item.requester_notification] || '尚无终态通知')}</strong></div>${item.notification_blocked_reason ? `<p class="form-note">通知阻塞：${esc(blockers[item.notification_blocked_reason] || item.notification_blocked_reason)}</p>` : ''}${item.triage ? `<p class="form-note">${item.triage.source === 'model' ? '模型分诊' : '框架回退'}：${esc(item.triage.reason)}</p>` : ''}<div class="task-actions">${actions.map(([action,label]) => `<button type="button" class="${action === 'reply' ? 'primary' : 'secondary'}" data-task-action="${action}">${label}</button>`).join('')}</div><div id="task-action-form"></div><details><summary>任务时间线</summary><ol class="task-timeline">${(item.timeline || []).slice().reverse().map(e => `<li><span>${esc(labels[e.kind] || ({created:'创建',suspended:'保存检查点',resumed:'恢复排队',input_requested:'请求业务输入',input_replied:'收到答复',resolved:'人工核对',reprioritized:'调整优先级',deferred:'延后'})[e.kind] || e.kind)}</span><small>${esc(dateTime(e.at))}</small></li>`).join('')}</ol></details>${item.activity?.length ? `<details><summary>执行器活动</summary><ol class="task-timeline">${item.activity.map(a => `<li><span>${esc(({command:'命令',files:'文件修改',framework:'框架工具',mcp:'MCP',input:'交互'})[a.kind] || a.kind)}</span><small>${esc(a.status)} · ${esc(dateTime(a.updated_at))}</small></li>`).join('')}</ol></details>` : ''}<small class="task-id">${esc(item.id)} · 版本 ${item.revision}</small></div>`;
      $('#task-back').onclick = open;
      $('#task-refresh').onclick = () => detail(id);
      $$('[data-task-action]').forEach(button => button.onclick = () => actionForm(item,button.dataset.taskAction));
    } catch (e) { if (active()) commandPanelError(e); }
  }
  function actionForm(item, action) {
    const isAnswer = action === 'reply';
    const names = {report:'确认发布汇报',reply:'提交答复',cancel:'确认取消',priority:'保存优先级',defer:'保存延后时间',resolve:'保存核对结论',retry:'确认创建重试'};
    const extra = action === 'priority' ? field('优先级',`<select name="priority">${options(Object.entries(priorities),item.priority)}</select>`) : action === 'defer' ? field('最早执行时间','<input type="datetime-local" name="until" required>') : action === 'resolve' ? field('实际执行结果','<select name="outcome"><option value="failed">已核对失败</option><option value="succeeded">已核对成功</option></select>') : '';
    $('#task-action-form').innerHTML = `<form class="task-action-form">${extra}${field(action === 'report' ? '汇报正文' : isAnswer ? '业务答复' : action === 'resolve' ? '核对依据' : '操作原因',`<textarea name="text" rows="4" maxlength="${isAnswer ? 16000 : 2000}" required></textarea>`)}<p class="form-note">${action === 'report' ? '将以你的身份发布到原始会话；请确认汇报内容和接收范围。' : action === 'resolve' ? '请先核对外部执行和文件状态。该结论会留存，不会覆盖原始异常记录。' : action === 'retry' ? '会创建关联的新任务并调用模型；请确认此前的外部操作不会被重复执行。' : isAnswer ? '仅补充业务信息，不授予工具或沙箱权限。' : '提交时会重新检查权限及版本，状态变化时请先刷新。'}</p><button class="primary" type="submit">${names[action]}</button><span id="task-action-feedback" role="status"></span></form>`;
    const form = $('#task-action-form form');
    form.querySelector('textarea').focus();
    const key = crypto.randomUUID();
    form.onsubmit = async event => {
      event.preventDefault(); if (isObserver()) return;
      const data = new FormData(form), button = form.querySelector('button'); button.disabled = true;
      const args = {task_id:item.id,expected_revision:item.revision};
      const toolName = ({report:'task_report',reply:'task_reply',cancel:'task_cancel',priority:'task_reprioritize',defer:'task_defer',resolve:'task_resolve',retry:'task_retry'})[action];
      if (isAnswer) Object.assign(args,{input_request_id:item.run_id,answer:data.get('text'),request_key:key});
      else if (action === 'resolve') Object.assign(args,{outcome:data.get('outcome'),evidence:data.get('text')});
      else if (action === 'report') args.body = data.get('text');
      else args.reason = data.get('text');
      if (action === 'priority') args.priority = data.get('priority');
      if (action === 'retry') args.request_key = key;
      if (action === 'defer') args.not_before = new Date(data.get('until')).getTime();
      try { const result = await tool(toolName,args); if (active()) await detail(action === 'retry' ? result.id : item.id); await activity(); }
      catch (e) { commandPanelError(e); button.disabled = false; }
    };
  }
  function open() { selected=null; request++; records=[]; shell(); load(); }
  return { open, refresh() { if (active() && !selected && !loading) load(); } };
})();
document.querySelector('#chat-tasks').onclick = () => AgtTasks.open();
