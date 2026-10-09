/* Workspace policy editor. Descriptions and defaults come from the backend contract. */
window.AgtSettings = (() => {
  let snapshot, generation=0, dirty=false;
  const names={
    'permissions.harness_mode':'Harness 权限处理模式','permissions.mode':'Native 权限审核策略','permissions.reviewer_model':'审核模型','permissions.review_timeout_seconds':'模型审核时限（秒）','permissions.request_timeout_seconds':'审批总时限（秒）',
    'scheduler.concurrency':'同时执行任务数', 'scheduler.parallel_work':'允许同身份并行工作',
    'scheduler.replies_per_agent':'每位 Agent 的讨论回复额度','scheduler.discussion_seconds':'讨论授权有效期（秒）',
    'scheduler.model_steps':'单次原生运行的模型步数','memory.enabled':'历史记忆检索',
    'execution.enabled':'允许本机命令','execution.timeout_seconds':'本机命令时限（秒）','execution.output_bytes':'命令输出上限（字节）'
  };
  const sections=[['permissions','Native 工具权限','仅管理 AGT 自身工具；Codex 使用引擎原生审批。'],['scheduler','调度与容量','控制同时运行多少工作，以及一轮授权可以持续多久。'],['memory','跨会话记忆','按固定身份检索可见历史；主题摘要始终保留原消息依据。'],['execution','本机执行','与 Agent 的 local_execution 能力共同生效。宿主机权限不是项目目录沙箱。']];
  const valueAt=(config,key)=>{const [group,name]=key.split('.');return config[group][name];};
  function reviewerOptions(value) {
    const current=value && (value.includes('/')?value:'default/'+value);
    const models=items('model').map(ref);
    return `<option value="">未配置，转人工</option>${current&&!models.includes(current)?`<option value="${esc(current)}" selected disabled>${esc(current)}（模型不可用，请重新选择）</option>`:''}${models.map(model=>`<option value="${esc(model)}" ${model===current?'selected':''}>${esc(model)}</option>`).join('')}`;
  }
  function reviewStatus() {
    const mode=$('[data-setting="permissions.mode"]')?.value;
    const reviewer=$('[data-setting="permissions.reviewer_model"]')?.value;
    $('#permission-review-status').textContent=mode==='model_review'&&!reviewer
      ?'尚未选择审核模型：Native 请求将全部转人工。请选择下方审核模型并保存；对当前等待和后续请求生效。Codex 使用独立的 Harness 处理模式。'
      :'本策略仅用于 Native 工具调用。Codex 使用独立的 Harness 处理模式。保存后对当前等待及后续请求生效。';
  }
  function render(schema) {
    $('#runtime-settings').innerHTML=`<form id="runtime-config-form"><p class="settings-intro">工作空间默认策略 · CLI 临时覆盖不显示在这里。未填写的配置采用后端默认值。</p><p id="permission-review-status" class="form-note" role="status"></p>${sections.map(([group,title,description])=>`<section class="settings-section"><header><h2>${title}</h2><p>${description}</p></header>${schema.fields.filter(f=>f.key.startsWith(group+'.')).map(f=>{
      const value=valueAt(snapshot,f.key), reviewer=f.key==='permissions.reviewer_model', harness=f.key==='permissions.harness_mode', mode=harness||f.key==='permissions.mode', boolean=typeof value==='boolean', unlimited=!reviewer&&!mode&&!f.key.startsWith('permissions.')&&(f.default===null||(!boolean&&f.key!=='execution.output_bytes'));
      return `<div class="setting-row"><div class="setting-copy"><label for="setting-${f.key}">${esc(names[f.key]||f.key)}</label><p>${esc(f.description)}</p><small>生效：${esc(f.effective)} · 默认：${esc(f.default===null?(reviewer?'未配置':'不限'):typeof f.default==='boolean'?(f.default?'开启':'关闭'):f.default)}</small><details><summary>配置键</summary><code>${esc(f.key)}</code></details></div><div class="setting-control">${mode?`<select id="setting-${f.key}" data-setting="${f.key}">${harness?'':`<option value="model_review" ${value==='model_review'?'selected':''}>模型审核，异常转人工</option>`}<option value="manual" ${value==='manual'?'selected':''}>人工审批</option><option value="auto_allow" ${value==='auto_allow'?'selected':''}>自动放行</option></select>`:reviewer?`<select id="setting-${f.key}" data-setting="${f.key}">${reviewerOptions(value)}</select>`:boolean?`<label class="setting-switch"><input id="setting-${f.key}" data-setting="${f.key}" type="checkbox" ${value?'checked':''}> 开启</label>`:`<input id="setting-${f.key}" data-setting="${f.key}" type="number" min="${f.key==='execution.output_bytes'?1024:1}" step="1" value="${value??''}" ${value===null?'disabled':''} required aria-label="${esc(names[f.key])}">${unlimited?`<label><input data-unlimited="${f.key}" type="checkbox" ${value===null?'checked':''}> 不限</label>`:''}`}</div></div>`;
    }).join('')}</section>`).join('')}<div class="settings-save"><p id="settings-feedback" role="status">修改后保存；调度相关设置建议统一重启服务。</p><button class="primary" type="submit">保存设置</button></div></form>`;
    $$('[data-unlimited]').forEach(box=>box.onchange=()=>{const input=document.getElementById('setting-'+box.dataset.unlimited);input.disabled=box.checked;if(!box.checked&&!input.value)input.value=String(schema.fields.find(f=>f.key===box.dataset.unlimited).default??1);});
    reviewStatus();
    $('#runtime-config-form').onchange=reviewStatus;
    $('#runtime-config-form').oninput=()=>{reviewStatus();dirty=true;$('#settings-feedback').textContent='有未保存修改';};
    $('#runtime-config-form').onsubmit=async e=>{
      e.preventDefault();const button=e.currentTarget.querySelector('[type=submit]');button.disabled=true;
      const request=++generation;
      try {
        const config=structuredClone(snapshot);
        $$('[data-setting]').forEach(input=>{const [group,key]=input.dataset.setting.split('.');config[group][key]=['permissions.mode','permissions.harness_mode'].includes(input.dataset.setting)?input.value:input.dataset.setting==='permissions.reviewer_model'?(input.value||null):input.type==='checkbox'?input.checked:input.disabled?null:Number(input.value);});
        const saved=await api('config',{config,expected:snapshot});
        if(request!==generation)return;
        snapshot=saved;dirty=false;$('#settings-feedback').textContent='已保存。请按各项生效说明操作；现有讨论到期时间未被自动延长。';
      }catch(e){if(request===generation)$('#settings-feedback').textContent=errorText(e)+' 如果其他窗口修改过配置，请重新读取。';}
      finally{if(request===generation)button.disabled=false;}
    };
  }
  async function open(force=false) {
    if(snapshot&&!force){
      const select=$('[data-setting="permissions.reviewer_model"]');
      if(select){select.innerHTML=reviewerOptions(select.value);reviewStatus();}
      return;
    }
    const request=++generation;$('#runtime-settings').innerHTML='<p class="form-note">正在读取工作空间设置…</p>';
    try {const {config,schema}=await api('config');if(request!==generation)return;snapshot=config;dirty=false;render(schema);}
    catch(e){if(request===generation)$('#runtime-settings').innerHTML=`<p class="error-text">${esc(errorText(e))}</p>`;}
  }
  $('#reload-settings').onclick=()=>{if(dirty){$('#settings-feedback').textContent='重新读取会丢弃未保存修改。';dialog('重新读取运行设置？','<p>将放弃本页未保存的修改，读取服务器最新配置。</p><button id="confirm-settings-reload" class="primary" type="button">放弃修改并读取</button>');$('#confirm-settings-reload').onclick=()=>{closeModal();open(true);};}else open(true);};
  return {open};
})();
