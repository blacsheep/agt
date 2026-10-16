/* Shared human inbox; the source engine retains ownership of each decision. */
window.AgtApprovals = (() => {
  let generation=0, selected=null, records=[], more=false, panelViewer=null, modePanel=null;
  const labels={reviewing:$t('Native 模型审核中'),awaiting_human:$t('等待人工'),approved:$t('已批准'),denied:$t('已拒绝'),canceled:$t('已取消'),expired:$t('已过期')};
  const delivery={not_sent:$t('决定尚未应用'),sending:$t('正在回传'),sent:$t('已回传引擎'),resolved:$t('审批处理已结束'),invalidated:$t('原请求已失效'),unknown:$t('回传结果待核对')};
  const origins={model_review:$t('模型审核'),auto_allow:$t('自动放行'),human:$t('人工审批'),local_admin:$t('本机管理员'),rule:$t('已保存的总是允许规则')};
  const choiceLabels={allow:$t('批准本次调用'),allow_always:$t('总是允许此 Agent 使用该工具'),deny:$t('拒绝本次调用'),cancel:$t('取消执行')};
  const rulesSection=rules=>rules.length?`<details class="permission-rules"><summary>${$t('总是允许的规则（{0}）', [rules.length])}</summary><p class="form-note">${$t('这些 Agent 调用对应工具时不再询问。移除后恢复正常审批。')}</p>${rules.map(r=>`<div class="work-row"><strong>${esc(r.agent)}</strong><p>${esc(r.tool)}</p><button type="button" class="secondary" data-rule-agent="${esc(r.agent)}" data-rule-tool="${esc(r.tool)}">${$t('移除')}</button></div>`).join('')}</details>`:'';
  const originLabel=a=>a.decision_origin ? $t('决定来源：{0}', [origins[a.decision_origin]||a.decision_origin]) : a.decision!=null?$t('决定来源：旧记录未注明'):'';
  const deliveryLabel=a=>a.delivery==='not_sent' ? (a.decision!=null?$t('等待执行器应用，无需再次审批'):$t('尚未作出决定')) : delivery[a.delivery]||a.delivery;
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
    if(a.source==='harness')return $t('这是 Codex / Harness 原生权限请求，按 Harness 处理模式回传决定，不经过 Native 审核模型。');
    const policy=a.request.policy;
    if(policy?.mode==='manual')return $t('此请求最初使用人工审批策略；当前未决请求会跟随处理模式变更。');
    if(policy?.mode==='model_review'&&!policy.reviewer_model)return $t('此 Run 启用了模型审核，但未指定审核模型，因此转人工。可在上方选择审核模型并保存。');
    if(policy?.mode==='model_review'&&!a.request.reviewer_fingerprint)return $t('此 Run 的审核模型未能加载，因此转人工。请检查审核模型及其 Provider 配置。');
    if(policy?.mode==='model_review')return $t('此请求使用模型审核。若需要人工介入，上方会显示转人工原因；已批准的请求无需再次审批。');
    return $t('审核策略只约束本次工具调用，不替代框架身份和资源权限。');
  }
  const pending=item=>['reviewing','awaiting_human'].includes(item.state) && item.delivery==='not_sent';
  const query=(viewer,id)=>api('approvals?'+new URLSearchParams({viewer,...(id?{id}:{})}));
  async function open(id=null) {
    selected=id;records=[];generation++;panelViewer=state.viewer;modePanel=null;
    commandPanel($t('权限审批'),`<p>${$t('正在读取…')}</p>`,'approvals');
    if(id) await detail(id);else await list();
  }
  async function list(append=false) {
    selected=null;const request=++generation,viewer=state.viewer;
    try {
      const params=new URLSearchParams({viewer});if(append&&records.length)params.set('before',records.at(-1).id);
      const [rows,rules]=await Promise.all([api('approvals?'+params),canApprove()?api('permission-rules'):[]]);
      if(!active()||request!==generation||viewer!==state.viewer)return;
      records=[...new Map((append?[...records,...rows]:rows).map(x=>[x.id,x])).values()];more=rows.length===100;
      renderBody(`<button id="approval-refresh" class="secondary">${$t('刷新')}</button><p class="form-note">${$t('{0}下方保留审批记录；后续请求按上方保存的模式处理。', [canApprove()?$t('以控制台操作者身份审批。'):$t('只读观察。')])}</p>${rulesSection(rules)}${records.map(a=>`<button class="work-row" data-approval="${esc(a.id)}"><strong>${esc(a.source==='harness'?$t('引擎原生审批'):$t('Native 工具权限'))}</strong><p>${esc(nameOf(a.principal||$t('本机运行')))} · ${esc(labels[a.state]||a.state)}</p><small>${esc(deliveryLabel(a))}</small><small>${esc(originLabel(a))}</small></button>`).join('')||`<p>${$t('当前身份没有可见审批。')}</p>`}${more?`<button id="approval-more" class="secondary">${$t('载入更早审批')}</button>`:''}`);
      $('#approval-refresh').onclick=()=>list();$('#approval-more')?.addEventListener('click',()=>list(true));
      $$('[data-approval]').forEach(b=>b.onclick=()=>detail(b.dataset.approval));
      $$('[data-rule-agent]').forEach(b=>b.onclick=async()=>{b.disabled=true;try{await api('permission-rules',{agent:b.dataset.ruleAgent,tool:b.dataset.ruleTool});await list();}catch(e){commandPanelError(e);b.disabled=false;}});
    }catch(e){if(active()&&request===generation)commandPanelError(e)}
  }
  async function detail(id) {
    selected=id;const request=++generation,viewer=state.viewer;
    try {
      const a=await query(viewer,id);
      if(!active()||request!==generation||viewer!==state.viewer)return;
      const choices=a.request.options||[];
      renderBody(`<button id="approval-back" class="secondary">${$t('‹ 审批列表')}</button><button id="approval-refresh" class="secondary">${$t('刷新')}</button><h3>${esc(a.source==='harness'?$t('引擎原生审批'):$t('Native 工具权限'))}</h3><p>${esc(labels[a.state]||a.state)} · ${esc(deliveryLabel(a))}</p><p>${esc(originLabel(a))}</p><p>${esc(a.reason)}</p><p class="form-note">${esc(policyHint(a))}</p><pre class="ix-body">${esc(JSON.stringify(a.source==='harness'?{...a.request.params,...(a.request.item_context?{item_context:a.request.item_context}:{})}:{tool:a.request.tool,arguments:a.request.arguments},null,2))}</pre><small>${$t('截止 {0} · Run {1}', [esc(new Date(a.expires_at).toLocaleString(AgtI18n.locale)), esc(a.run_id)])}</small>${canApprove()&&pending(a)?`<form id="approval-form"><label>${$t('决定')}<select name="choice" required><option value="" disabled selected>${$t('请选择决定')}</option>${choices.map((o,i)=>`<option value="${i}">${esc(a.source==='native'&&choiceLabels[o.value]||o.label)}</option>`).join('')}</select></label><label>${$t('说明')}<textarea name="reason" maxlength="2000"></textarea></label><button class="primary">${a.state==='reviewing'?$t('接管并提交人工决定'):$t('提交决定')}</button><p id="approval-feedback" role="status"></p></form>`:''}`);
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
  return {open,refresh(){if(active()){if(state.viewer!==panelViewer){generation++;panelViewer=state.viewer;records=[];selected=null;modePanel=null;$('#command-panel-body').textContent=$t('正在读取…');list();}else if(!selected)list();else if(!$('#approval-form'))detail(selected);}},labels,pending};
})();
