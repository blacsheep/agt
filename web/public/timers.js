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
  async function open() {
    const chat = state.conversation?.id;
    commandPanel($t('定时任务'), `<p class="form-note">${$t('到点时以你的名义在本会话发出消息，并像你亲自发送一样开启新一轮讨论。Agent 设置时按工具权限审批；也可以用 CLI 创建。')}</p><p id="timer-count" class="form-note" role="status">${$t('正在读取…')}</p><div id="timer-list"></div>`, 'timers');
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
