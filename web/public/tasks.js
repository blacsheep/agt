/* Task workspace: compact lists first, explicit decisions in the side panel. */
window.AgtTasks = (() => {
  let records = [], selected = null, loading = false, more = false, request = 0;
  const labels = {queued:$t('待处理'),running:$t('执行中'),waiting_dependency:$t('等待子任务'),waiting_until:$t('定时等待'),waiting_input:$t('等待答复'),succeeded:$t('已完成'),failed:$t('失败'),canceled:$t('已取消'),interrupted:$t('已中断'),canceling:$t('正在停止'),outcome_unknown:$t('结果待核对')};
  const priorities = {urgent:$t('紧急'),high:$t('高'),normal:$t('普通'),low:$t('低')};
  const blockers = {task_report_required:$t('上次执行已结束，但尚未提交任务报告；待办仍保留'),discussion_expired:$t('讨论授权已到期，续期后继续处理'),discussion_paused:$t('讨论已暂停，恢复后继续处理'),disabled_or_unbound:$t('Agent 已停用或解除绑定'),authorization_changed:$t('会话或成员权限已变化'),identity_busy:$t('该身份正在处理串行任务'),resource_busy:$t('共享资源正被其他任务使用'),resource_outcome_unknown:$t('共享资源的前次结果需要核对'),waiting_input:$t('等待业务答复'),waiting_for_task:$t('等待依赖结果'),deferred_until:$t('等待指定时间'),dependency_outcome_unknown:$t('依赖结果待核对'),dependency_deadline_exceeded:$t('依赖等待已超时'),disabled:$t('Agent 已停用'),manual:$t('手动响应策略'),expired:$t('讨论授权已过期')};
  const stopReasons = {cancellation_requested:$t('收到停止请求'),task_canceled:$t('任务被主动取消'),discussion_stopped:$t('会话已停止'),context_reset:$t('会话上下文已重置'),authorization_revoked:$t('执行权限已撤销'),service_shutdown:$t('执行服务关闭'),scheduler_lease_lost:$t('调度器执行归属失效'),execution_owner_lost:$t('执行线程或归属丢失'),harness_lease_lost:$t('Harness 执行租约失效'),harness_timeout:$t('Harness 执行超时'),connection_lost:$t('Codex 连接断开'),engine_failed:$t('引擎明确返回失败'),engine_interrupted:$t('引擎确认中断'),protocol_error:$t('引擎协议或交互错误'),result_missing:$t('引擎结束但未提供有效结果'),result_persistence_failed:$t('执行结果未能可靠保存')};
  const confirmations = {unconfirmed:$t('停止状态未确认'),not_submitted:$t('尚未提交引擎'),not_running:$t('未在运行'),runtime_stopped:$t('运行时已停止'),engine_interrupted:$t('引擎已确认停止'),engine_failed:$t('引擎已确认失败'),engine_rejected:$t('引擎拒绝执行'),engine_completed:$t('引擎已结束')};
  const sources = {scheduler:$t('框架调度器'),runtime:$t('运行时'),task_control:$t('任务控制'),harness:$t('Harness 适配器')};
  function terminationDetails(item) {
    const t = item.termination;
    if (!t) return '';
    const effects = item.resolution ? $t('已记录人工核对结论，原始执行事实保留。') : t.side_effects === 'not_started' ? $t('尚未开始执行，不存在本次执行产生的外部修改。') : $t('可能已有文件或外部系统改动；停止执行不会撤销这些改动。');
    return `<section class="task-termination" aria-label="${$t('执行终止原因')}"><div><strong>${esc(stopReasons[t.code] || t.code)}</strong><span>${esc(confirmations[t.confirmation] || t.confirmation)}</span></div><p>${esc(sources[t.source] || t.source)}${t.actor ? ` ${$t('· 操作者 {0}', [esc(nameOf(t.actor))])}` : ''}</p><p class="form-note">${esc(effects)}</p><details><summary>${$t('追踪信息')}</summary><dl><dt>${$t('原因码')}</dt><dd>${esc(t.code)}</dd>${Object.entries(item.execution_ids || {run_id:item.run_id}).filter(([,v])=>v).map(([key,value])=>`<dt>${esc(key)}</dt><dd>${esc(value)}</dd>`).join('')}</dl></details></section>`;
  }
  function terminationTimeline(item) {
    const events = (item.timeline || []).slice();
    const t = item.termination;
    if (t) {
      events.push({at:t.observed_at,label:$t('记录原因 · {0}', [stopReasons[t.code] || t.code])});
      if (t.interrupt_requested_at) events.push({at:t.interrupt_requested_at,label:$t('向引擎请求停止')});
      if (t.confirmed_at) events.push({at:t.confirmed_at,label:confirmations[t.confirmation] || t.confirmation});
    }
    return events.sort((a,b)=>a.at-b.at);
  }
  const active = () => !$('#command-panel').classList.contains('hidden') && $('#command-panel').dataset.mode === 'tasks';
  function shell() {
    commandPanel($t('协作任务'), `<div class="task-toolbar"><input id="task-search" class="search" placeholder="${$t('搜索任务、执行者…')}" aria-label="${$t('搜索任务')}"><div><select id="task-scope" aria-label="${$t('任务范围')}"><option value="current">${$t('当前会话')}</option><option value="all">${$t('全部可见任务')}</option>${isObserver() ? '' : `<option value="incidents">${$t('工作空间异常')}</option>`}</select><select id="task-filter" aria-label="${$t('任务状态')}"><option value="active">${$t('进行中')}</option><option value="attention">${$t('需要处理')}</option><option value="all">${$t('全部状态')}</option><option value="done">${$t('已结束')}</option></select></div></div><p id="task-count" class="form-note" role="status">${$t('正在读取…')}</p><div id="task-list"></div><button id="task-more" type="button" class="quiet hidden">${$t('加载更早任务')}</button>`, 'tasks');
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
      const reasons = {outcome_unknown:$t('结果待核对'),publication_blocked:$t('结果投递受阻'),report_blocked:$t('原会话汇报受阻'),report_overdue:$t('超过 5 分钟尚未汇报'),dependency_deadline_exceeded:$t('依赖等待已超时'),requester_unavailable:$t('请求者已失效')};
      const rows = records.filter(item => `${item.id} ${nameOf(item.assignee)} ${nameOf(item.requester)}`.toLowerCase().includes(search));
      $('#task-count').textContent = $t('{0} 项异常 · 仅显示最少元数据，不授予私聊访问权限', [rows.length]);
      $('#task-list').innerHTML = rows.map(item => `<button class="work-row" type="button" data-work="${esc(item.id)}" ${item.can_inspect ? '' : 'disabled'}><strong>${esc(item.reasons.map(r => reasons[r] || r).join(' · '))}</strong><p>${esc(nameOf(item.requester))} → ${esc(nameOf(item.assignee))}</p><small>${item.can_inspect ? $t('查看任务详情') : $t('无会话访问权限，请联系相关成员核查')} · ${esc(dateTime(item.updated_at))}</small></button>`).join('') || empty($t('暂无待处理异常'),$t('结果未知、汇报受阻及超时待办会在这里列出。'));
      $$('#task-list [data-work]:not([disabled])').forEach(button => button.onclick = () => detail(button.dataset.work));
      $('#task-more').classList.toggle('hidden',!more); return;
    }
    const rows = records.filter(item => {
      const ended = ['succeeded','failed','canceled','interrupted','outcome_unknown'].includes(item.state);
      const attention = item.state === 'waiting_input' || item.state === 'outcome_unknown' && !item.resolution || item.publication === 'blocked' || ['awaiting_report','blocked'].includes(item.report?.state) || item.requester_notification === 'pending' || item.blocked_reason === 'dependency_deadline_exceeded';
      return (filter === 'all' || filter === 'active' && !ended || filter === 'done' && ended || filter === 'attention' && attention) && `${item.request.input} ${nameOf(item.request.assignee)} ${nameOf(item.request.requester)} ${item.id}`.toLowerCase().includes(search);
    });
    $('#task-count').textContent = $t('{0} 项符合条件 · 已加载 {1} 项{2}', [rows.length, records.length, isObserver() ? $t(' · 只读观察') : '']);
    $('#task-list').innerHTML = rows.map(item => `<button type="button" class="work-row" data-work="${esc(item.id)}"><div><span class="work-state ${esc(item.execution_status || item.state)}">${esc(labels[item.execution_status || item.state] || item.state)}</span><small>${esc(priorities[item.priority] || item.priority)}</small></div><strong>${esc(item.request.input.slice(0,180))}</strong><p>${esc(nameOf(item.request.requester))} → ${esc(nameOf(item.request.assignee))}</p><small>${esc(blockers[item.blocked_reason] || item.priority_reason || item.deferred?.reason || dateTime(item.updated_at))}</small></button>`).join('') || empty($t('这里暂时没有任务'),$t('普通聊天不会自动转成任务。可让挂载「任务协作」的 lead 委派工作，或调整筛选范围。'));
    $$('#task-list [data-work]').forEach(button => button.onclick = () => detail(button.dataset.work));
    $('#task-more').classList.toggle('hidden',!more);
  }
  async function detail(id) {
    selected = id; const generation = ++request, viewer = state.viewer;
    try {
      const item = await read('task_get',{task_id:id},viewer);
      if (!active() || generation !== request || viewer !== state.viewer) return;
      const ended = ['succeeded','failed','canceled','interrupted','outcome_unknown'].includes(item.state);
      const human = !isObserver();
      const canQueue = human && item.state === 'queued' && item.request.requester === state.boot.principal;
      const actions = [];
      if (human && item.report?.state === 'awaiting_report') actions.push(['report',$t('汇报到原会话')]);
      if (human && !ended && !item.cancellation_requested) actions.push(['cancel',$t('取消任务')]);
      if (human && item.state === 'waiting_input') actions.unshift(['reply',$t('回答问题')]);
      if (canQueue) actions.push(['priority',$t('调整优先级')],['defer',$t('延后执行')]);
      if (human && item.state === 'outcome_unknown' && !item.resolution) actions.push(['resolve',$t('记录核对结论')]);
      if (human && (['failed','canceled','interrupted'].includes(item.state) || item.resolution?.outcome === 'failed')) actions.push(['retry',$t('创建重试任务')]);
      $('#command-panel-body').innerHTML = `<div class="task-detail-tools"><button id="task-back" class="quiet" type="button">${$t('‹ 任务列表')}</button><button id="task-refresh" class="secondary" type="button">${$t('刷新状态')}</button></div><div class="work-detail"><div class="work-detail-state"><span class="work-state ${esc(item.execution_status || item.state)}">${esc(labels[item.execution_status || item.state] || item.state)}</span><small>${$t('{0}优先级', [esc(priorities[item.priority])])}</small></div><h3>${esc(nameOf(item.request.assignee))}</h3><p class="form-note">${$t('来自 {0} · {1}', [esc(nameOf(item.request.requester)), esc(dateTime(item.created_at))])}</p><details class="task-request" open><summary>${$t('任务要求')}</summary><pre>${esc(item.request.input)}</pre></details>${item.execution_policy ? `<p class="form-note">${$t('执行策略：{0}', [item.execution_policy.parallel?$t('允许并行（需全局启用）'):$t('身份串行')])}${item.execution_policy.exclusive_resources?.length?` ${$t('· 互斥资源：{0}', [esc(item.execution_policy.exclusive_resources.join($t('、')))])}`:''}</p>`:''}${item.blocked_reason ? `<p class="task-callout">${esc(blockers[item.blocked_reason] || item.blocked_reason)}</p>` : ''}${item.deferred?.reason ? `<p>${esc(item.deferred.reason)}</p>` : ''}${item.cancellation_requested && !ended ? `<p class="task-callout">${$t('取消已请求，正在确认执行状态；已发生的外部操作不会撤销。')}</p>` : ''}${terminationDetails(item)}${item.question ? `<div class="task-callout"><strong>${$t('需要补充信息')}</strong><p>${esc(item.question)}</p><small>${$t('截止 {0}', [esc(new Date(item.deadline).toLocaleString())])}</small></div>` : ''}${item.waiting_for_task ? `<p class="form-note">${$t('依赖任务：{0}', [esc(item.waiting_for_task)])}</p>` : ''}${item.output || item.error || item.reason ? `<details class="task-request" open><summary>${$t('结果与说明')}</summary><pre>${esc(item.output || item.error || item.reason)}</pre></details>` : ''}${item.resolution ? `<p class="task-callout">${$t('人工核对：{0}', [item.resolution.outcome === 'succeeded' ? $t('成功') : $t('失败')])}<br>${esc(item.resolution.evidence)}</p>` : ''}<div class="task-report"><span>${$t('结果投递')}</span><strong>${esc(({not_ready:$t('尚未就绪'),pending:$t('待投递'),published:$t('已送达'),blocked:$t('投递受阻')})[item.publication] || item.publication)}</strong><span>${$t('原会话汇报')}</span><strong>${esc(({not_ready:$t('尚未就绪'),awaiting_report:$t('等待汇报'),pending:$t('待发布'),published:$t('已汇报'),blocked:$t('汇报受阻')})[item.report?.state] || $t('尚未就绪'))}</strong><span>${$t('请求者处理')}</span><strong>${esc(({pending:$t('待处理'),handled:$t('已交接')})[item.requester_notification] || $t('尚无终态通知'))}</strong></div>${item.notification_blocked_reason ? `<p class="form-note">${$t('通知阻塞：{0}', [esc(blockers[item.notification_blocked_reason] || item.notification_blocked_reason)])}</p>` : ''}${item.triage ? `<p class="form-note">${$t('{0}：{1}', [item.triage.source === 'model' ? $t('模型分诊') : $t('框架回退'), esc(item.triage.reason)])}</p>` : ''}<div class="task-actions">${actions.map(([action,label]) => `<button type="button" class="${action === 'reply' ? 'primary' : 'secondary'}" data-task-action="${action}">${label}</button>`).join('')}</div><div id="task-action-form"></div><details><summary>${$t('任务时间线')}</summary><ol class="task-timeline">${terminationTimeline(item).map(e => `<li><span>${esc(e.label || labels[e.kind] || ({cancellation_requested:$t('请求取消'),requested:$t('创建任务'),created:$t('创建'),suspended:$t('保存检查点'),resumed:$t('恢复排队'),input_requested:$t('请求业务输入'),input_replied:$t('收到答复'),resolved:$t('人工核对'),reprioritized:$t('调整优先级'),deferred:$t('延后')})[e.kind] || e.kind)}</span><small>${esc(dateTime(e.at))}</small></li>`).join('')}</ol></details>${item.activity?.length ? `<details><summary>${$t('执行器活动')}</summary>${activityTimeline(item.activity)}</details>` : ''}<small class="task-id">${$t('{0} · 版本 {1}', [esc(item.id), item.revision])}</small></div>`;
      $('#task-back').onclick = open;
      $('#task-refresh').onclick = () => detail(id);
      $$('[data-task-action]').forEach(button => button.onclick = () => actionForm(item,button.dataset.taskAction));
    } catch (e) { if (active()) commandPanelError(e); }
  }
  function actionForm(item, action) {
    const isAnswer = action === 'reply';
    const names = {report:$t('确认发布汇报'),reply:$t('提交答复'),cancel:$t('确认取消'),priority:$t('保存优先级'),defer:$t('保存延后时间'),resolve:$t('保存核对结论'),retry:$t('确认创建重试')};
    const extra = action === 'priority' ? field($t('优先级'),`<select name="priority">${options(Object.entries(priorities),item.priority)}</select>`) : action === 'defer' ? field($t('最早执行时间'),'<input type="datetime-local" name="until" required>') : action === 'resolve' ? field($t('实际执行结果'),`<select name="outcome"><option value="failed">${$t('已核对失败')}</option><option value="succeeded">${$t('已核对成功')}</option></select>`) : '';
    $('#task-action-form').innerHTML = `<form class="task-action-form">${extra}${field(action === 'report' ? $t('汇报正文') : isAnswer ? $t('业务答复') : action === 'resolve' ? $t('核对依据') : $t('操作原因'),`<textarea name="text" rows="4" maxlength="${isAnswer ? 16000 : 2000}" required></textarea>`)}<p class="form-note">${action === 'report' ? $t('将以你的身份发布到原始会话；请确认汇报内容和接收范围。') : action === 'resolve' ? $t('请先核对外部执行和文件状态。该结论会留存，不会覆盖原始异常记录。') : action === 'retry' ? $t('会创建关联的新任务并调用模型；请确认此前的外部操作不会被重复执行。') : isAnswer ? $t('仅补充业务信息，不授予工具或沙箱权限。') : $t('提交时会重新检查权限及版本，状态变化时请先刷新。')}</p><button class="primary" type="submit">${names[action]}</button><span id="task-action-feedback" role="status"></span></form>`;
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
