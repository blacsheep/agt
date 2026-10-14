/* Live executions above the composer, one collapsible line per run. Settled runs are shown as message threads (run-threads.js). */
window.AgtExecutionView = (() => {
  const phases={preparing:$t('准备上下文与工具'),waiting:$t('等待模型输出'),reasoning:$t('模型处理中'),generating:$t('正在生成'),tool_pending:$t('准备工具调用'),tool_running:$t('正在执行工具'),compacting:$t('正在压缩上下文'),retrying:$t('连接异常，正在重试'),input_required:$t('执行器需要交互')};
  let scope=null,timer=null;
  const set=(node,selector,value)=>{const target=node.querySelector(selector);if(target.textContent!==value)target.textContent=value;};
  function summary(run, items, approvals) {
    if(approvals.length)return approvals.some(a=>a.state==='awaiting_human')?$t('等待人工审批'):$t('模型审核中');
    const active=items.filter(a=>a.status==='running');
    if(active.length>1)return $t('正在执行 {0} 项操作 · {1}', [active.length, active.slice(0,2).map(AgtToolActivity.title).join($t('、'))]);
    if(active.length)return AgtToolActivity.title(active[0]);
    if(run.progress?.phase==='tool_running'&&items.length)return $t('整理工具结果');
    return phases[run.progress?.phase]||$t('正在处理');
  }
  function render({container,runs,conversation,viewer,online,visible,nameOf,onApproval,loadToolDetail}) {
    clearTimeout(timer);
    const nextScope=JSON.stringify([viewer,conversation]);
    if(scope!==nextScope){container.replaceChildren();scope=nextScope;}
    // Only live executions live here; settled ones move into the message thread they belong to.
    const selected=visible?runs.filter(r=>r.conversation===conversation&&online&&r.state==='running'&&r.progress):[];
    const previous=new Map([...container.children].map(node=>[node.dataset.run,node]));
    for(const [index,run] of selected.entries()) {
      const p=run.state==='running'?run.progress:null, activity=run.tool_activity;
      const items=activity?.items||p?.activities||[], approvals=activity?.approvals||[];
      let node=previous.get(run.id);previous.delete(run.id);
      if(!node){
        node=document.createElement('details');node.className='live-reply';node.dataset.run=run.id;
        node.innerHTML='<summary><span class="run-indicator" aria-hidden="true"></span><strong></strong><span class="live-phase"></span><small></small></summary><div class="live-body"><div class="run-approval-actions"></div><div class="live-preview md"></div><p class="live-caption"></p><p class="run-progress-note"></p><div class="harness-activity"></div><p class="run-history-note"></p></div>';
        node.querySelector('summary').addEventListener('click',()=>{node.dataset.disclosure='user';});
      }
      // One line per Agent by default; only a pending approval needs attention unasked.
      if(!node.dataset.disclosure)node.open=approvals.length>0;
      node.dataset.state=run.state;
      set(node,':scope > summary > strong',activity?.principal||p?.principal?nameOf(activity?.principal||p?.principal):run.agent.split('/').at(-1));
      set(node,'.live-phase',summary(run,items,approvals)+(items.length?` ${$t('· {0} 项工具', [items.length])}`:''));
      set(node,'.run-indicator',run.state==='running'?(approvals.length?'?':'◌'):run.state==='completed'?'✓':'!');
      const starts=items.map(a=>a.started_at).filter(Number.isFinite),ends=items.map(a=>a.updated_at).filter(Number.isFinite);
      const started=activity?.started_at??p?.started_at??(starts.length?Math.min(...starts):null);
      const ended=activity?.ended_at??(ends.length?Math.max(...ends):null);
      const clock=node.querySelector(':scope > summary > small');clock.dataset.started=started??'';clock.dataset.ended=ended??'';clock.dataset.status=run.state;clock.dataset.attempt=p?.attempt||0;
      clock.textContent=AgtToolActivity.duration({started_at:started,updated_at:ended,status:run.state==='running'?'running':'completed'})+(p?.attempt>1?` ${$t('· 第 {0}/10 次尝试', [p.attempt])}`:'');
      const preview=node.querySelector('.live-preview'), following=!node.isConnected||preview.scrollHeight-preview.scrollTop-preview.clientHeight<30;
      const source=p?.preview||'';if(preview.dataset.source!==source){preview.dataset.source=source;preview.innerHTML=source?window.AgtMarkdown.render(source):'';}preview.hidden=!source;
      set(node,'.live-caption',p?.preview?$t('生成预览 · 完成后发布正式消息'):p?.phase==='retrying'?$t('本次预览已清除，正在重新请求。'):'');
      const age=p?.protocol_at?Math.max(0,Math.floor((Date.now()-p.protocol_at)/1000)):0;
      set(node,'.run-progress-note',activity?.approvals_unavailable?$t('审批状态暂时无法读取。'):age>30?$t('暂未收到新进度，执行结果尚未确认。'):'');
      const actions=node.querySelector('.run-approval-actions');
      const key=JSON.stringify(approvals);
      if(actions.dataset.key!==key){actions.dataset.key=key;actions.replaceChildren();for(const a of approvals){const button=document.createElement('button');button.type='button';button.className='secondary';button.textContent=a.source==='harness'?$t('查看引擎审批'):$t('查看工具权限');button.onclick=()=>onApproval(a.id);actions.append(button);}}
      AgtToolActivity.update(node.querySelector('.harness-activity'),items,{key:JSON.stringify([viewer,conversation,run.id]),load:id=>loadToolDetail(run.id,id)});
      set(node,'.run-history-note',items.length>=40?$t('显示最近 40 项公开工具记录。'):'');
      if(container.children[index]!==node){const focused=node.contains(document.activeElement)?document.activeElement:null;container.insertBefore(node,container.children[index]||null);focused?.focus({preventScroll:true});}
      if(following)preview.scrollTop=preview.scrollHeight;
    }
    previous.forEach(node=>node.remove());container.classList.toggle('hidden',!selected.length);
    function tick(){
      if(!document.hidden){
        AgtToolActivity.tick(container);
        container.querySelectorAll(':scope > details[data-state="running"] > summary > small').forEach(clock=>{
          clock.textContent=AgtToolActivity.duration({started_at:clock.dataset.started===''?null:Number(clock.dataset.started),status:'running'})+(Number(clock.dataset.attempt)>1?` ${$t('· 第 {0}/10 次尝试', [clock.dataset.attempt])}`:'');
        });
      }
      if(container.querySelector(':scope > details[data-state="running"]'))timer=setTimeout(tick,1000);
    }
    if(selected.some(r=>r.state==='running'))timer=setTimeout(tick,1000);
  }
  return {render};
})();
