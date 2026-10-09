/* Lazy payload loading, scoped to one mounted tool card. No shared private-data cache. */
window.AgtToolDetails = (() => {
  const states = new WeakMap();
  const titles = {command:'命令',cwd:'工作目录',arguments:'调用参数',output:'返回结果',aggregatedOutput:'终端输出',changes:'文件变更 / Diff',result:'返回结果',error:'错误',exitCode:'退出码'};
  function bind(node, activity, context) {
    if (!context) return;
    let state = states.get(node);
    if (!state) {
      const body = document.createElement('div');body.className='tool-payload';
      node.querySelector('.tool-detail').append(body);
      state={body,version:null,serial:0};states.set(node,state);
      node.addEventListener('toggle',()=>{if(node.open)load(node,state);});
    }
    state.context=context;state.activity=activity;
    const version=JSON.stringify([context.key,activity.status,activity.updated_at]);
    if(state.version!==version){state.version=version;state.serial++;state.loaded=false;state.pending=false;state.body.replaceChildren();}
    if(node.open)load(node,state);
  }
  async function load(node,state) {
    if(state.loaded||state.pending||!node.isConnected)return;
    state.pending=true;
    const serial=state.serial, context=state.context;
    state.body.textContent='正在读取工具详情…';
    try {
      const detail=await context.load(state.activity.id);
      if(serial!==state.serial||!node.isConnected)return;
      state.loaded=true;state.body.replaceChildren();
      const fields=Object.keys(titles).filter(key=>detail?.fields?.[key]).map(key=>[key,detail.fields[key]]);
      if(!fields.length){state.body.textContent='详情不可用：旧记录或执行器未提供原始内容。';return;}
      for(const [key,field] of fields){
        const section=document.createElement('section'),label=document.createElement('h4'),pre=document.createElement('pre');
        label.textContent=titles[key]+(field.truncated?' · 已截断（64 KiB）':'');
        pre.textContent=field.text===''?'（空）':field.text;pre.tabIndex=0;
        section.append(label,pre);state.body.append(section);
      }
      if(state.activity.status==='running'){
        const note=document.createElement('p');note.textContent='工具仍在执行，结果会在执行器报告后更新。';state.body.append(note);
      }
    } catch(error) {
      if(serial!==state.serial||!node.isConnected)return;
      state.body.replaceChildren();
      const message=document.createElement('p');message.textContent=error.message||'工具详情读取失败。';
      const retry=document.createElement('button');retry.type='button';retry.className='secondary';retry.textContent='重试';
      retry.onclick=()=>load(node,state);state.body.append(message,retry);
    } finally {if(serial===state.serial)state.pending=false;}
  }
  return {bind};
})();
