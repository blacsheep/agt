/* Stable tool-call cards. Owns DOM identity/disclosure; renderers own tool-specific wording. */
window.AgtToolActivity = (() => {
  const labels = {running:$t('执行中'),completed:$t('成功'),failed:$t('失败'),canceled:$t('已取消'),unsupported:$t('不支持'),unknown:$t('结果待确认')};
  const symbols = {running:'◌',completed:'✓',failed:'!',canceled:'−',unsupported:'!',unknown:'?'};
  const text = (node, value) => { if(node.textContent!==value) node.textContent=value; };
  function duration(a, now=Date.now()) {
    if(!Number.isFinite(a.started_at))return $t('耗时未提供');
    const end=a.status==='running'?now:a.updated_at;
    if(!Number.isFinite(end))return $t('耗时未提供');
    const seconds=Math.max(0,(end-a.started_at)/1000);
    return seconds<60?$t('{0} 秒', [seconds.toFixed(1)]):$t('{0} 分 {1} 秒', [Math.floor(seconds/60), Math.floor(seconds%60)]);
  }
  function update(container, activities, context) {
    const previous=new Map([...container.children].map(node=>[node.dataset.activity,node]));
    const unique=new Map(activities.map((a,i)=>[a.id||`legacy:${i}:${a.started_at}`,a]));
    let index=0;
    for(const [id,a] of unique) {
      let node=previous.get(id);previous.delete(id);
      if(!node){
        node=document.createElement('details');node.className='tool-card';node.dataset.activity=id;
        node.innerHTML='<summary><span class="tool-symbol" aria-hidden="true"></span><span class="tool-heading"><strong></strong><span class="tool-category"></span><span class="tool-public-preview"></span></span><span class="tool-result"><span class="tool-status"></span><time></time></span></summary><div class="tool-detail"><code></code><p class="tool-summary"></p><p class="tool-note"></p></div>';
      }
      const view=AgtToolRenderers.describe(a);
      node.dataset.status=a.status;node.dataset.kind=a.kind;
      text(node.querySelector('.tool-symbol'),view.symbol);
      text(node.querySelector('strong'),view.title);
      text(node.querySelector('.tool-category'),view.category);
      const excerpt=node.querySelector('.tool-public-preview');text(excerpt,a.summary?$t(a.summary):'');excerpt.hidden=!a.summary;
      text(node.querySelector('.tool-status'),`${symbols[a.status]||'?'} ${labels[a.status]||$t('结果待确认')}`);
      const clock=node.querySelector('time');clock.dataset.started=a.started_at??'';clock.dataset.ended=a.updated_at??'';clock.dataset.status=a.status;text(clock,duration(a));
      const code=node.querySelector('code');text(code,a.tool_name||'');code.hidden=!a.tool_name;
      const summary=node.querySelector('.tool-summary');text(summary,a.summary?$t(a.summary):'');summary.hidden=!a.summary;
      text(node.querySelector('.tool-note'),view.note);
      if(container.children[index]!==node){const focused=node.contains(document.activeElement)?document.activeElement:null;container.insertBefore(node,container.children[index]||null);focused?.focus({preventScroll:true});}
      AgtToolDetails.bind(node,a,context);
      index++;
    }
    previous.forEach(node=>node.remove());
  }
  function tick(container) {
    container.querySelectorAll('time[data-status="running"]').forEach(node=>text(node,duration({status:'running',started_at:node.dataset.started===''?null:Number(node.dataset.started)})));
  }
  // Existing task/history views can reuse the presentation without owning live DOM state.
  function timeline(activities) {
    return `<ol class="task-timeline tool-activity-list">${activities.map(a=>{
      const view=AgtToolRenderers.describe(a);
      return `<li><div><span>${esc(view.title)}</span>${a.tool_name?`<code>${esc(a.tool_name)}</code>`:''}${a.summary?`<small>${esc($t(a.summary))}</small>`:''}</div><small>${esc(labels[a.status]||$t('状态待确认'))} · ${esc(duration(a))} · ${esc(view.category)}</small></li>`;
    }).join('')}</ol>`;
  }
  return {update,tick,timeline,title:a=>AgtToolRenderers.describe(a).title,duration};
})();
