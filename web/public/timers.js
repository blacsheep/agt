/* Timers of the current conversation: what fires next, who set it, and cancel. */
window.AgtTimers = (() => {
  const states = {active:$t('等待触发'),done:$t('已触发'),canceled:$t('已取消')};
  const when = value => value ? new Date(value).toLocaleString(AgtI18n.locale, {dateStyle:'medium', timeStyle:'short'}) : '';
  function row(timer, human) {
    const schedule = timer.cron ? $t('周期：{0}（{1}）', [timer.cron, timer.timezone]) : $t('只触发一次');
    const next = timer.state === 'active' ? $t('下次：{0}', [when(timer.next_at)]) : timer.reason || (timer.last_fired_at ? $t('触发于 {0}', [when(timer.last_fired_at)]) : '');
    const cancel = human && timer.state === 'active' ? `<button type="button" class="secondary" data-timer-cancel="${esc(timer.id)}">${$t('取消')}</button>` : '';
    return `<div class="work-row"><div><span class="work-state ${esc(timer.state)}">${esc(states[timer.state] || timer.state)}</span><small>${esc(schedule)}</small></div><strong>${esc(timer.name || timer.text.slice(0,180))}</strong>${timer.name ? `<p>${esc(timer.text.slice(0,180))}</p>` : ''}<p>${$t('由 {0} 设置 · 以 {1} 的名义发送', [esc(nameOf(timer.creator)), esc(nameOf(timer.sender))])}</p><small>${esc(next)}${timer.fired ? $t(' · 已触发 {0} 次', [timer.fired]) : ''}</small>${cancel}</div>`;
  }
  function form() {
    const members = state.members.filter(m => m.left_message_seq == null && m.principal_id !== state.boot.principal);
    return `<details class="timer-create"><summary>${$t('新建定时任务')}</summary><form id="timer-form" class="task-action-form">${field($t('消息内容'), '<textarea name="text" rows="3" maxlength="4000" required></textarea>', $t('到点时以你的名义发出，写成单独看也能明白的指令。'))}${field($t('名称（可选）'), '<input name="name" maxlength="80">')}${field($t('时间'), `<select name="kind">${options([['once', $t('只触发一次')], ['cron', $t('按周期重复')]], 'once')}</select>`)}<div data-timer-kind="once">${field($t('触发时间'), '<input type="datetime-local" name="at">')}</div><div data-timer-kind="cron" class="hidden">${field($t('cron 表达式'), '<input name="cron" placeholder="0 9 1 * *" spellcheck="false">', $t('分 时 日 月 星期，例如每月 1 号 9 点是 0 9 1 * *；至少间隔 10 分钟。'))}</div>${members.length ? `<fieldset class="field"><legend>${$t('需要回复的成员')}</legend>${members.map(m => `<label><input type="checkbox" name="mentions" value="${esc(m.principal_id)}"> ${esc(nameOf(m.principal_id))}</label>`).join('')}</fieldset>` : ''}${field($t('讨论时长（分钟，可选）'), '<input type="number" name="minutes" min="1" max="10080" step="1">', $t('留空沿用工作空间设置；耗时长的任务可以调大。'))}<button class="primary" type="submit">${$t('创建')}</button></form></details>`;
  }
  function bindForm(chat) {
    const form = $('#timer-form');
    if (!form) return;
    form.kind.onchange = () => $$('[data-timer-kind]').forEach(el => el.classList.toggle('hidden', el.dataset.timerKind !== form.kind.value));
    form.onsubmit = async event => {
      event.preventDefault();
      const data = new FormData(form), button = form.querySelector('[type=submit]');
      const args = {conversation:chat, text:String(data.get('text')), mentions:data.getAll('mentions')};
      if (data.get('name')) args.name = String(data.get('name'));
      if (data.get('kind') === 'cron') args.cron = String(data.get('cron') || '');
      // The browser's local time, sent with its offset so the server needs no guess.
      else if (data.get('at')) args.at = new Date(String(data.get('at'))).toISOString();
      if (data.get('minutes')) args.discussion_seconds = Number(data.get('minutes')) * 60;
      button.disabled = true;
      try { await tool('timer_create', args); await open(); }
      catch (e) { commandPanelError(e); button.disabled = false; }
    };
  }
  async function open() {
    const chat = state.conversation?.id;
    commandPanel($t('定时任务'), `<p class="form-note">${$t('到点时以你的名义在本会话发出消息，并像你亲自发送一样开启新一轮讨论。Agent 设置时按工具权限审批；也可以用 CLI 创建。')}</p>${isObserver() ? '' : form()}<p id="timer-count" class="form-note" role="status">${$t('正在读取…')}</p><div id="timer-list"></div>`, 'timers');
    bindForm(chat);
    const generation = commandGeneration, viewer = state.viewer;
    try {
      const {timers} = await read('timer_list', {conversation:chat, include_finished:true}, viewer);
      if (generation !== commandGeneration || chat !== state.conversation?.id) return;
      const human = !isObserver();
      const active = timers.filter(t => t.state === 'active').length;
      $('#timer-count').textContent = $t('{0} 个等待触发 · 共 {1} 个', [active, timers.length]);
      $('#timer-list').innerHTML = timers.map(t => row(t, human)).join('') || empty($t('本会话没有定时任务'), $t('可以对 Agent 说“每月 1 号 9 点提醒大家发数据”，它会设置定时任务。'));
      $$('[data-timer-cancel]').forEach(button => button.onclick = async () => {
        button.disabled = true;
        try { await tool('timer_cancel', {timer_id:button.dataset.timerCancel, reason:$t('在 Web 上取消')}); await open(); }
        catch (e) { commandPanelError(e); button.disabled = false; }
      });
    } catch (e) { if (generation === commandGeneration) commandPanelError(e); }
  }
  return {open};
})();
