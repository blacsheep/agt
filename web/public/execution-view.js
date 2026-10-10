/* One run summary with independently keyed tool cards. Payload fetching is delegated to the scoped tool-detail module. */
window.AgtExecutionView = (() => {
  const phases={preparing:'准备上下文与工具',waiting:'等待模型输出',reasoning:'模型处理中',generating:'正在生成',tool_pending:'准备工具调用',tool_running:'正在执行工具',compacting:'正在压缩上下文',retrying:'连接异常，正在重试',input_required:'执行器需要交互'};
  const endings={completed:'执行已结束',failed:'执行失败',canceled:'执行已取消',interrupted:'执行已中断',outcome_unknown:'执行结果待确认',suspended:'执行已挂起'};
  let scope=null,timer=null;
  const dismissedScopes=new Map();
  function dismissedFor(key) {
    if(!dismissedScopes.has(key)) {
      let ids=[];
      try {const saved=JSON.parse(localStorage.getItem('agt.dismissed-executions:'+key)||'[]');if(Array.isArray(saved))ids=saved.filter(id=>typeof id==='string');}catch{}
      dismissedScopes.set(key,new Set(ids));
    }
    return dismissedScopes.get(key);
  }
  const set=(node,selector,value)=>{const target=node.querySelector(selector);if(target.textContent!==value)target.textContent=value;};
  function summary(run, items, approvals) {
    if(run.state!=='running')return `${endings[run.state]||'执行已结束'} · ${items.length} 项工具记录`;
    if(approvals.length)return approvals.some(a=>a.state==='awaiting_human')?'等待人工审批':'模型审核中';
    const active=items.filter(a=>a.status==='running');
    if(active.length>1)return `正在执行 ${active.length} 项操作 · ${active.slice(0,2).map(AgtToolActivity.title).join('、')}`;
    if(active.length)return AgtToolActivity.title(active[0]);
    if(run.progress?.phase==='tool_running'&&items.length)return '整理工具结果';
    return phases[run.progress?.phase]||'正在处理';
  }
  function render({container,runs,conversation,viewer,online,visible,nameOf,onApproval,loadToolDetail}) {
    clearTimeout(timer);
    const nextScope=JSON.stringify([viewer,conversation]);
    if(scope!==nextScope){container.replaceChildren();scope=nextScope;}
    const dismissed=dismissedFor(nextScope);
    const selected=visible?runs.filter(r=>(r.state==='running'||!dismissed.has(r.id))&&r.conversation===conversation&&((online&&r.state==='running'&&r.progress)||(r.state!=='running'&&r.tool_activity?.items?.length))):[];
    const previous=new Map([...container.children].map(node=>[node.dataset.run,node]));
    for(const [index,run] of selected.entries()) {
      const p=run.state==='running'?run.progress:null, activity=run.tool_activity;
      const items=activity?.items||p?.activities||[], approvals=activity?.approvals||[];
      let node=previous.get(run.id);previous.delete(run.id);
      if(!node){
        node=document.createElement('details');node.className='live-reply';node.dataset.run=run.id;
        node.innerHTML='<summary><span class="run-indicator" aria-hidden="true"></span><strong></strong><span class="live-phase"></span><small></small><button type="button" class="run-dismiss" aria-label="关闭执行提示" title="关闭提示，执行记录仍保留">×</button></summary><div class="live-body"><div class="run-approval-actions"></div><div class="live-preview md"></div><p class="live-caption"></p><p class="run-progress-note"></p><div class="harness-activity"></div><p class="run-history-note"></p></div>';
        node.querySelector('summary').addEventListener('click',()=>{node.dataset.disclosure='user';});
      }
      if(!node.dataset.disclosure)node.open=!!(items.length||approvals.length)&&run.state==='running';
      node.dataset.state=run.state;
      const dismiss=node.querySelector('.run-dismiss');dismiss.hidden=run.state==='running';
      dismiss.onclick=event=>{
        event.preventDefault();event.stopPropagation();
        if(node.dataset.state==='running')return;
        dismissed.add(run.id);
        try {localStorage.setItem('agt.dismissed-executions:'+nextScope,JSON.stringify([...dismissed]));}catch{}
        const next=node.nextElementSibling||node.previousElementSibling;
        node.remove();container.classList.toggle('hidden',!container.children.length);
        next?.querySelector('summary')?.focus({preventScroll:true});
      };
      set(node,':scope > summary > strong',activity?.principal||p?.principal?nameOf(activity?.principal||p?.principal):run.agent.split('/').at(-1));
      set(node,'.live-phase',summary(run,items,approvals));
      set(node,'.run-indicator',run.state==='running'?(approvals.length?'?':'◌'):run.state==='completed'?'✓':'!');
      const starts=items.map(a=>a.started_at).filter(Number.isFinite),ends=items.map(a=>a.updated_at).filter(Number.isFinite);
      const started=activity?.started_at??p?.started_at??(starts.length?Math.min(...starts):null);
      const ended=activity?.ended_at??(ends.length?Math.max(...ends):null);
      const clock=node.querySelector(':scope > summary > small');clock.dataset.started=started??'';clock.dataset.ended=ended??'';clock.dataset.status=run.state;clock.dataset.attempt=p?.attempt||0;
      clock.textContent=AgtToolActivity.duration({started_at:started,updated_at:ended,status:run.state==='running'?'running':'completed'})+(p?.attempt>1?` · 第 ${p.attempt}/10 次尝试`:'');
      const preview=node.querySelector('.live-preview'), following=!node.isConnected||preview.scrollHeight-preview.scrollTop-preview.clientHeight<30;
      const source=p?.preview||'';if(preview.dataset.source!==source){preview.dataset.source=source;preview.innerHTML=source?window.AgtMarkdown.render(source):'';}preview.hidden=!source;
      set(node,'.live-caption',p?.preview?'生成预览 · 完成后发布正式消息':p?.phase==='retrying'?'本次预览已清除，正在重新请求。':'');
      const age=p?.protocol_at?Math.max(0,Math.floor((Date.now()-p.protocol_at)/1000)):0;
      set(node,'.run-progress-note',activity?.approvals_unavailable?'审批状态暂时无法读取。':age>30?'暂未收到新进度，执行结果尚未确认。':'');
      const actions=node.querySelector('.run-approval-actions');
      const key=JSON.stringify(approvals);
      if(actions.dataset.key!==key){actions.dataset.key=key;actions.replaceChildren();for(const a of approvals){const button=document.createElement('button');button.type='button';button.className='secondary';button.textContent=a.source==='harness'?'查看引擎审批':'查看工具权限';button.onclick=()=>onApproval(a.id);actions.append(button);}}
      AgtToolActivity.update(node.querySelector('.harness-activity'),items,{key:JSON.stringify([viewer,conversation,run.id]),load:id=>loadToolDetail(run.id,id)});
      set(node,'.run-history-note',items.length>=40?'显示最近 40 项公开工具记录。':run.state!=='running'?'工具记录已保留；执行结束不代表所有操作均成功。':'');
      if(container.children[index]!==node){const focused=node.contains(document.activeElement)?document.activeElement:null;container.insertBefore(node,container.children[index]||null);focused?.focus({preventScroll:true});}
      if(following)preview.scrollTop=preview.scrollHeight;
    }
    previous.forEach(node=>node.remove());container.classList.toggle('hidden',!selected.length);
    function tick(){
      if(!document.hidden){
        AgtToolActivity.tick(container);
        container.querySelectorAll(':scope > details[data-state="running"] > summary > small').forEach(clock=>{
          clock.textContent=AgtToolActivity.duration({started_at:clock.dataset.started===''?null:Number(clock.dataset.started),status:'running'})+(Number(clock.dataset.attempt)>1?` · 第 ${clock.dataset.attempt}/10 次尝试`:'');
        });
      }
      if(container.querySelector(':scope > details[data-state="running"]'))timer=setTimeout(tick,1000);
    }
    if(selected.some(r=>r.state==='running'))timer=setTimeout(tick,1000);
  }
  return {render};
})();
