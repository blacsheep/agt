/* The new-version button at the bottom of the rail. AGT Studio downloads and
 * installs the new version after the person confirms; `agt web` runs from the
 * CLI, so its dialog shows the command that upgrades it. Nothing changes
 * without a click. */
(() => {
  const tauri = window.__TAURI__;
  const SIX_HOURS = 6 * 60 * 60 * 1000;
  let status = null;

  async function load() {
    return tauri ? tauri.core.invoke('update_status') : api('update');
  }

  async function refresh() {
    try { status = await load(); } catch { status = null; }
    const button = $('#update-button');
    const available = !!status?.update_available;
    button.classList.toggle('hidden', !available);
    if (!available) return;
    const label = $t('新版本 {0}', [status.latest]);
    $('#update-label').textContent = label;
    button.title = label;
    button.setAttribute('aria-label', label);
  }

  // Agents answering now; restarting AGT Studio cuts their replies short.
  function working() {
    if (!state.scheduler?.online) return 0;
    return new Set(state.scheduler.tasks.filter(t => ['evaluating', 'responding', 'input'].includes(t.state)).map(t => t.principal)).size;
  }

  function open() {
    if (!status?.update_available) return;
    const release = `<p class="form-note">${$t('{0} 已发布，你现在用的是 {1}。', [esc(status.latest), esc(status.current)])} <a href="${esc(status.url)}" target="_blank" rel="noopener noreferrer">${$t('查看更新内容')}</a></p>`;
    if (!tauri) {
      dialog($t('有新版本'), `${release}<p class="form-note">${$t('在终端运行下面的命令升级，然后重启 agt web：')}</p><pre class="update-command"><code>agt upgrade</code></pre><div class="form-actions"><button id="update-copy" type="button" class="quiet">${$t('复制命令')}</button><button id="update-later" type="button" class="primary">${$t('知道了')}</button></div>`);
      $('#update-copy').onclick = async () => { try { await navigator.clipboard.writeText('agt upgrade'); notify($t('已复制')); } catch { notify($t('复制失败，请手动复制'), true); } };
      $('#update-later').onclick = closeModal;
      return;
    }
    const busy = working();
    const warning = busy ? $t('更新会重启 AGT Studio。{0} 位 Agent 正在工作，它们的回复会中断。', [busy]) : $t('更新会重启 AGT Studio。');
    dialog($t('有新版本'), `${release}<p class="form-note">${warning}</p><p id="update-progress" class="form-note hidden" aria-live="polite"></p><div class="form-actions"><button id="update-later" type="button" class="quiet">${$t('稍后')}</button><button id="update-now" type="button" class="primary">${$t('更新并重启')}</button></div>`);
    $('#update-later').onclick = closeModal;
    $('#update-now').onclick = install;
  }

  async function install() {
    const progress = $('#update-progress');
    $('#update-now').disabled = true; $('#update-later').disabled = true;
    $('#modal-error').classList.add('hidden');
    progress.classList.remove('hidden'); progress.textContent = $t('正在下载…');
    state.modalBusy = true;
    const channel = new tauri.core.Channel();
    channel.onmessage = ({downloaded, total}) => {
      const mb = n => (n / 1048576).toFixed(1);
      progress.textContent = total ? $t('正在下载 {0} / {1} MB', [mb(downloaded), mb(total)]) : $t('正在下载 {0} MB', [mb(downloaded)]);
    };
    try {
      await tauri.core.invoke('update_install', {channel});
      progress.textContent = $t('正在重启…');
    } catch (error) {
      state.modalBusy = false;
      progress.classList.add('hidden');
      $('#update-now').disabled = false; $('#update-later').disabled = false;
      modalError({message: String(error)});
    }
  }

  $('#update-button').onclick = open;
  if (tauri) {
    tauri.event.listen('agt-update', refresh);
    tauri.event.listen('agt-update-open', async () => { await refresh(); open(); });
  }
  refresh();
  setInterval(refresh, SIX_HOURS);
})();
