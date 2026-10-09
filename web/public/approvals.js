/* Shared human inbox; the source engine retains ownership of each decision. */
window.AgtApprovals = (() => {
  let generation=0, selected=null, records=[], more=false, panelViewer=null, modePanel=null;
  const labels={reviewing:'Native 模型审核中',awaiting_human:'等待人工',approved:'已批准',denied:'已拒绝',canceled:'已取消',expired:'已过期'};
  const delivery={not_sent:'决定尚未应用',sending:'正在回传',sent:'已回传引擎',resolved:'审批处理已结束',invalidated:'原请求已失效',unknown:'回传结果待核对'};
  const origins={model_review:'模型审核',auto_allow:'自动放行',human:'人工审批',local_admin:'本机管理员'};
  const originLabel=a=>a.decision_origin ? `决定来源：${origins[a.decision_origin]||a.decision_origin}` : a.decision!=null?'决定来源：旧记录未注明':'';
  const deliveryLabel=a=>a.delivery==='not_sent' ? (a.decision!=null?'等待执行器应用，无需再次审批':'尚未作出决定') : delivery[a.delivery]||a.delivery;
  const active=()=>$('#command-panel').dataset.mode==='approvals' && !$('#command-panel').classList.contains('hidden');
  const canApprove=()=>!!state.boot?.principal && state.viewer===state.boot.principal;
  function renderBody(html) {
    const body=$('#command-panel-body'), fresh=!modePanel;
    if(fresh){modePanel=document.createElement('section');modePanel.className='permission-mode';}
    const focus=modePanel.contains(document.activeElement)?document.activeElement:null;
    body.innerHTML=html;body.prepend(modePanel);focus?.focus({preventScroll:true});
    if(fresh)AgtPermissionMode.mount(modePanel,canApprove());
  }
  function policyHint(a) {
    if(a.source==='harness')return '这是 Codex / Harness 原生权限请求，按 Harness 处理模式回传决定，不经过 Native 审核模型。';
    const policy=a.request.policy;
    if(policy?.mode==='manual')return '此请求最初使用人工审批策略；当前未决请求会跟随处理模式变更。';
    if(policy?.mode==='model_review'&&!policy.reviewer_model)return '此 Run 启用了模型审核，但未指定审核模型，因此转人工。可在上方选择审核模型并保存。';
    if(policy?.mode==='model_review'&&!a.request.reviewer_fingerprint)return '此 Run 的审核模型未能加载，因此转人工。请检查审核模型及其 Provider 配置。';
    if(policy?.mode==='model_review')return '此请求使用模型审核。若需要人工介入，上方会显示转人工原因；已批准的请求无需再次审批。';
    return '审核策略只约束本次工具调用，不替代框架身份和资源权限。';
  }
  const pending=item=>['reviewing','awaiting_human'].includes(item.state) && item.delivery==='not_sent';
  const query=(viewer,id)=>api('approvals?'+new URLSearchParams({viewer,...(id?{id}:{})}));
  async function open(id=null) {
    selected=id;records=[];generation++;panelViewer=state.viewer;modePanel=null;
    commandPanel('权限审批','<p>正在读取…</p>','approvals');
    if(id) await detail(id);else await list();
  }
  async function list(append=false) {
    selected=null;const request=++generation,viewer=state.viewer;
    try {
      const params=new URLSearchParams({viewer});if(append&&records.length)params.set('before',records.at(-1).id);
      const rows=await api('approvals?'+params);
      if(!active()||request!==generation||viewer!==state.viewer)return;
      records=[...new Map((append?[...records,...rows]:rows).map(x=>[x.id,x])).values()];more=rows.length===100;
      renderBody(`<button id="approval-refresh" class="secondary">刷新</button><p class="form-note">${canApprove()?'以控制台操作者身份审批。':'只读观察。'}下方保留审批记录；后续请求按上方保存的模式处理。</p>${records.map(a=>`<button class="work-row" data-approval="${esc(a.id)}"><strong>${esc(a.source==='harness'?'引擎原生审批':'Native 工具权限')}</strong><p>${esc(nameOf(a.principal||'本机运行'))} · ${esc(labels[a.state]||a.state)}</p><small>${esc(deliveryLabel(a))}</small><small>${esc(originLabel(a))}</small></button>`).join('')||'<p>当前身份没有可见审批。</p>'}${more?'<button id="approval-more" class="secondary">载入更早审批</button>':''}`);
      $('#approval-refresh').onclick=()=>list();$('#approval-more')?.addEventListener('click',()=>list(true));
      $$('[data-approval]').forEach(b=>b.onclick=()=>detail(b.dataset.approval));
    }catch(e){if(active()&&request===generation)commandPanelError(e)}
  }
  async function detail(id) {
    selected=id;const request=++generation,viewer=state.viewer;
    try {
      const a=await query(viewer,id);
      if(!active()||request!==generation||viewer!==state.viewer)return;
      const choices=a.request.options||[];
      renderBody(`<button id="approval-back" class="secondary">‹ 审批列表</button><button id="approval-refresh" class="secondary">刷新</button><h3>${esc(a.source==='harness'?'引擎原生审批':'Native 工具权限')}</h3><p>${esc(labels[a.state]||a.state)} · ${esc(deliveryLabel(a))}</p><p>${esc(originLabel(a))}</p><p>${esc(a.reason)}</p><p class="form-note">${esc(policyHint(a))}</p><pre class="ix-body">${esc(JSON.stringify(a.source==='harness'?{...a.request.params,...(a.request.item_context?{item_context:a.request.item_context}:{})}:{tool:a.request.tool,arguments:a.request.arguments},null,2))}</pre><small>截止 ${esc(new Date(a.expires_at).toLocaleString())} · Run ${esc(a.run_id)}</small>${canApprove()&&pending(a)?`<form id="approval-form"><label>决定<select name="choice" required><option value="" disabled selected>请选择决定</option>${choices.map((o,i)=>`<option value="${i}">${esc(o.label)}</option>`).join('')}</select></label><label>说明<textarea name="reason" maxlength="2000"></textarea></label><button class="primary">${a.state==='reviewing'?'接管并提交人工决定':'提交决定'}</button><p id="approval-feedback" role="status"></p></form>`:''}`);
      $('#approval-back').onclick=()=>list();$('#approval-refresh').onclick=()=>detail(id);
      const form=$('#approval-form');if(form)form.onsubmit=async e=>{
        e.preventDefault();if(!canApprove()||viewer!==state.viewer)return;
        const data=new FormData(form),button=form.querySelector('button');button.disabled=true;
        try {await api('approvals',{id:a.id,expected_revision:a.revision,decision:choices[Number(data.get('choice'))].value,reason:String(data.get('reason')||'')});if(active()&&viewer===state.viewer&&selected===id)await detail(id);}
        catch(e){if(active()&&request===generation){$('#approval-feedback').textContent=errorText(e);button.disabled=false;}}
      };
    }catch(e){if(active()&&request===generation)commandPanelError(e)}
  }
  // Keep unsubmitted human choices, but refresh settled/read-only details with new events.
  return {open,refresh(){if(active()){if(state.viewer!==panelViewer){generation++;panelViewer=state.viewer;records=[];selected=null;modePanel=null;$('#command-panel-body').textContent='正在读取…';list();}else if(!selected)list();else if(!$('#approval-form'))detail(selected);}},labels,pending};
})();
