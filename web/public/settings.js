/* Workspace policy editor. Descriptions and defaults come from the backend contract. */
window.AgtSettings = (() => {
  let snapshot, generation=0, dirty=false;
  const names={
    'permissions.harness_mode':$t('Harness 权限处理模式'),'permissions.mode':$t('Native 权限审核策略'),'permissions.reviewer_model':$t('审核模型'),'permissions.review_timeout_seconds':$t('模型审核时限（秒）'),'permissions.request_timeout_seconds':$t('审批总时限（秒）'),
    'scheduler.concurrency':$t('同时执行任务数'), 'scheduler.parallel_work':$t('允许同身份并行工作'),
    'scheduler.replies_per_agent':$t('每位 Agent 的讨论回复额度'),'scheduler.discussion_seconds':$t('讨论授权有效期（秒）'),
    'scheduler.model_steps':$t('单次原生运行的模型步数'),'scheduler.timezone':$t('时区'),'memory.enabled':$t('历史记忆检索'),
    'execution.enabled':$t('允许本机命令'),'execution.timeout_seconds':$t('本机命令时限（秒）'),'execution.output_bytes':$t('命令输出上限（字节）')
  };
  const sections=[['permissions',$t('工具权限审批'),$t('Native Agent 调用工具前的审核策略，以及 Codex 原生审批请求的处理方式。')],['scheduler',$t('调度与容量'),$t('控制同时运行多少工作，以及一轮授权可以持续多久。')],['memory',$t('跨会话记忆'),$t('按固定身份检索可见历史；主题摘要始终保留原消息依据。')],['execution',$t('本机执行'),$t('与 Agent 的 local_execution 能力共同生效。宿主机权限不是项目目录沙箱。')]];
  const valueAt=(config,key)=>{const [group,name]=key.split('.');return config[group][name];};
  function reviewerOptions(value) {
    const current=value && (value.includes('/')?value:'default/'+value);
    const models=items('model').map(ref);
    return `<option value="">${$t('未配置，转人工')}</option>${current&&!models.includes(current)?`<option value="${esc(current)}" selected disabled>${$t('{0}（模型不可用，请重新选择）', [esc(current)])}</option>`:''}${models.map(model=>`<option value="${esc(model)}" ${model===current?'selected':''}>${esc(model)}</option>`).join('')}`;
  }
  function reviewStatus() {
    const mode=$('[data-setting="permissions.mode"]')?.value;
    const reviewer=$('[data-setting="permissions.reviewer_model"]')?.value;
    $('#permission-review-status').textContent=mode==='model_review'&&!reviewer
      ?$t('尚未选择审核模型：Native 请求将全部转人工。请选择下方审核模型并保存；对当前等待和后续请求生效。Codex 使用独立的 Harness 处理模式。')
      :'';
  }
  // Highlight rows whose value differs from the loaded snapshot, and the save bar while anything is unsaved.
  function readValue(input) {
    const key=input.dataset.setting;
    if(['permissions.mode','permissions.harness_mode'].includes(key)) return input.value;
    if(key==='permissions.reviewer_model') return input.value||null;
    if(key==='scheduler.timezone') return input.value.trim()||null;
    if(input.type==='checkbox') return input.checked;
    return input.disabled?null:Number(input.value);
  }
  function markChanges() {
    let changed=0;
    $$('#runtime-config-form .setting-row').forEach(row=>{
      const input=row.querySelector('[data-setting]');
      const differs=!!input && readValue(input)!==valueAt(snapshot,input.dataset.setting);
      row.classList.toggle('changed',differs); if(differs) changed++;
    });
    dirty=changed>0;
    $('.settings-save')?.classList.toggle('dirty',dirty);
    $('#settings-feedback').textContent=dirty?$t('{0} 项未保存', [changed]):$t('没有未保存的修改');
  }
  function render(schema) {
    $('#runtime-settings').innerHTML=`<div class="settings-layout"><nav class="settings-nav" aria-label="${$t('设置分区')}">${sections.map(([group,title])=>`<button type="button" data-settings-jump="${group}">${title}</button>`).join('')}</nav><form id="runtime-config-form"><p class="settings-intro">${$t('工作空间默认策略；CLI 的临时覆盖不显示在这里，未填写的项采用默认值。')}</p><p id="permission-review-status" class="settings-warning" role="status"></p>${sections.map(([group,title,description])=>`<section class="settings-section" id="settings-${group}"><header><h2>${title}</h2><p>${description}</p></header>${schema.fields.filter(f=>f.key.startsWith(group+'.')).map(f=>{
      const value=valueAt(snapshot,f.key), reviewer=f.key==='permissions.reviewer_model', harness=f.key==='permissions.harness_mode', mode=harness||f.key==='permissions.mode', boolean=typeof value==='boolean', zone=f.key==='scheduler.timezone', unlimited=!zone&&!reviewer&&!mode&&!f.key.startsWith('permissions.')&&(f.default===null||(!boolean&&f.key!=='execution.output_bytes'));
      return `<div class="setting-row"><div class="setting-copy"><label for="setting-${f.key}">${esc(names[f.key]||f.key)}</label><p>${esc($t(f.description))}</p><small>${$t('生效：{0} · 默认：{1}', [esc($t(f.effective)), esc(f.default===null?(reviewer?$t('未配置'):zone?$t('本机时区'):$t('不限')):typeof f.default==='boolean'?(f.default?$t('开启'):$t('关闭')):f.default)])}</small><details><summary>${$t('配置键')}</summary><code>${esc(f.key)}</code></details></div><div class="setting-control">${mode?`<select id="setting-${f.key}" data-setting="${f.key}">${harness?'':`<option value="model_review" ${value==='model_review'?'selected':''}>${$t('模型审核，异常转人工')}</option>`}<option value="manual" ${value==='manual'?'selected':''}>${$t('人工审批')}</option><option value="auto_allow" ${value==='auto_allow'?'selected':''}>${$t('自动放行')}</option></select>`:reviewer?`<select id="setting-${f.key}" data-setting="${f.key}">${reviewerOptions(value)}</select>`:zone?`<input id="setting-${f.key}" data-setting="${f.key}" type="text" value="${esc(value??'')}" placeholder="${$t('留空使用本机时区，例如 Asia/Shanghai')}" spellcheck="false" aria-label="${esc(names[f.key])}">`:boolean?`<label class="setting-switch"><input id="setting-${f.key}" data-setting="${f.key}" type="checkbox" ${value?'checked':''}> ${$t('开启')}</label>`:`<input id="setting-${f.key}" data-setting="${f.key}" type="number" min="${f.key==='execution.output_bytes'?1024:1}" step="1" value="${value??''}" ${value===null?'disabled':''} required aria-label="${esc(names[f.key])}">${unlimited?`<label><input data-unlimited="${f.key}" type="checkbox" ${value===null?'checked':''}> ${$t('不限')}</label>`:''}`}</div></div>`;
    }).join('')}</section>`).join('')}<div class="settings-save"><p id="settings-feedback" role="status">${$t('没有未保存的修改')}</p><small>${$t('调度相关设置建议保存后统一重启服务。')}</small><button class="primary" type="submit">${$t('保存设置')}</button></div></form></div>`;
    $$('[data-settings-jump]').forEach(button=>button.onclick=()=>document.getElementById('settings-'+button.dataset.settingsJump).scrollIntoView({behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'auto':'smooth',block:'start'}));
    const observer=new IntersectionObserver(entries=>{for(const entry of entries) if(entry.isIntersecting) $$('[data-settings-jump]').forEach(b=>b.setAttribute('aria-current',String('settings-'+b.dataset.settingsJump===entry.target.id)));},{rootMargin:'-20% 0px -70% 0px'});
    $$('.settings-section').forEach(section=>observer.observe(section));
    $$('[data-unlimited]').forEach(box=>box.onchange=()=>{const input=document.getElementById('setting-'+box.dataset.unlimited);input.disabled=box.checked;if(!box.checked&&!input.value)input.value=String(schema.fields.find(f=>f.key===box.dataset.unlimited).default??1);markChanges();});
    reviewStatus();
    $('#runtime-config-form').onchange=()=>{reviewStatus();markChanges();};
    $('#runtime-config-form').oninput=()=>{reviewStatus();markChanges();};
    $('#runtime-config-form').onsubmit=async e=>{
      e.preventDefault();const button=e.currentTarget.querySelector('[type=submit]');button.disabled=true;
      const request=++generation;
      try {
        const config=structuredClone(snapshot);
        $$('[data-setting]').forEach(input=>{const [group,key]=input.dataset.setting.split('.');config[group][key]=readValue(input);});
        const saved=await api('config',{config,expected:snapshot});
        if(request!==generation)return;
        snapshot=saved;markChanges();$('#settings-feedback').textContent=$t('已保存。请按各项生效说明操作；现有讨论到期时间未被自动延长。');
      }catch(e){if(request===generation)$('#settings-feedback').textContent=errorText(e)+$t(' 如果其他窗口修改过配置，请重新读取。');}
      finally{if(request===generation)button.disabled=false;}
    };
  }
  async function open(force=false) {
    if(snapshot&&!force){
      const select=$('[data-setting="permissions.reviewer_model"]');
      if(select){select.innerHTML=reviewerOptions(select.value);reviewStatus();}
      return;
    }
    const request=++generation;$('#runtime-settings').innerHTML=`<p class="form-note">${$t('正在读取工作空间设置…')}</p>`;
    try {const {config,schema}=await api('config');if(request!==generation)return;snapshot=config;dirty=false;render(schema);}
    catch(e){if(request===generation)$('#runtime-settings').innerHTML=`<p class="error-text">${esc(errorText(e))}</p>`;}
  }
  $('#reload-settings').onclick=()=>{if(dirty){$('#settings-feedback').textContent=$t('重新读取会丢弃未保存修改。');dialog($t('重新读取运行设置？'),`<p>${$t('将放弃本页未保存的修改，读取服务器最新配置。')}</p><button id="confirm-settings-reload" class="primary" type="button">${$t('放弃修改并读取')}</button>`);$('#confirm-settings-reload').onclick=()=>{closeModal();open(true);};}else open(true);};
  return {open};
})();
