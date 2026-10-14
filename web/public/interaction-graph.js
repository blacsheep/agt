/* SVG network renderer. Data and permissions remain owned by the observation page. */
window.createInteractionGraph = (onSelect) => {
  const svg = $('#ix-graph'), tip = $('#ix-graph-tip');
  let nodes = new Map(), edges = [], positions = new Map(), focus = null, chosen = null;
  let camera = { x: 0, y: 0, k: 1 }, drag = null, moved = false, expiry;
  const point = event => new DOMPoint(event.clientX, event.clientY).matrixTransform(svg.getScreenCTM().inverse());
  const world = p => ({ x: (p.x - camera.x) / camera.k, y: (p.y - camera.y) / camera.k });
  const short = (value, n = 18) => value.length > n ? value.slice(0, n) + '…' : value;
  const transform = () => {
    svg.querySelector('.ix-world')?.setAttribute('transform', `translate(${camera.x} ${camera.y}) scale(${camera.k})`);
    $('#ix-zoom-level').textContent = Math.round(camera.k * 100) + '%';
  };
  function fit() {
    if (!nodes.size) return;
    const coords = [...nodes.keys()].map(id => positions.get(id));
    const left = Math.min(...coords.map(p => p.x)) - 100, top = Math.min(...coords.map(p => p.y)) - 85;
    const width = Math.max(...coords.map(p => p.x)) + 100 - left, height = Math.max(...coords.map(p => p.y)) + 100 - top;
    const k = Math.min(1.5, 880 / width, 510 / height);
    camera = { k, x: 480 - (left + width / 2) * k, y: 295 - (top + height / 2) * k }; transform();
  }
  function layout() {
    positions.clear();
    const ordered = [...nodes.values()].sort((a,b) => b.degree - a.degree || a.id.localeCompare(b.id));
    ordered.forEach((n,i) => {
      if (!i) positions.set(n.id, { x: 480, y: 290 });
      else {
        const ring = Math.floor((i - 1) / 8) + 1, count = Math.min(8, ordered.length - 1 - (ring - 1) * 8);
        const angle = (i - 1) % 8 / count * Math.PI * 2 - Math.PI / 2;
        positions.set(n.id, { x: 480 + Math.cos(angle) * 235 * ring, y: 290 + Math.sin(angle) * 190 * ring });
      }
    });
  }
  function draw() {
    const activeElement = document.activeElement;
    const focusedKey = activeElement?.getAttribute('data-node');
    const focusedEdge = activeElement?.getAttribute('data-edge');
    const neighborhood = new Set([focus]);
    edges.forEach(e => { if (e.from === focus || e.to === focus) { neighborhood.add(e.from); neighborhood.add(e.to); } });
    const now = Date.now();
    svg.innerHTML = `<defs><linearGradient id="ix-ring-gradient" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#5b5ff0"/><stop offset=".5" stop-color="#8a4fe0"/><stop offset="1" stop-color="#22b8cf"/></linearGradient><marker id="ix-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M0 0 10 5 0 10z" fill="#7d96b8"/></marker></defs><g class="ix-world">
      ${edges.map((e,i) => {
        const a = positions.get(e.from), b = positions.get(e.to), dx = b.x-a.x, dy = b.y-a.y, length = Math.hypot(dx,dy) || 1;
        const x1=a.x+dx/length*36, y1=a.y+dy/length*36, x2=b.x-dx/length*40, y2=b.y-dy/length*40;
        const cx=(x1+x2)/2-dy*.12, cy=(y1+y2)/2+dx*.12, d=`M${x1} ${y1} Q${cx} ${cy} ${x2} ${y2}`;
        const lit = e.until > now, selected = chosen?.kind==='task' ? e.workTasks?.some(t=>t.id===chosen.id) : e.messages.some(m => m.id === chosen);
        return `<g class="ix-edge ${e.membership?'membership':''} ${e.workTasks?'delegation':''} ${lit?'fresh':''} ${selected?'chosen':''} ${focus && e.from!==focus && e.to!==focus?'dim':''}" data-edge="${i}" tabindex="0" role="button" aria-label="${esc($t('{0} → {1}，{2}', [nodes.get(e.from).label, nodes.get(e.to).label, e.membership?$t('成员关系'):e.workTasks?$t('{0} 项任务委派', [e.workTasks.length]):$t('{0} 条消息', [e.messages.length])]))}">
          <path class="ix-edge-hit" d="${d}"/><path class="ix-edge-line" d="${d}" ${e.membership?'':'marker-end="url(#ix-arrow)"'}/>
          ${lit ? `<circle class="ix-packet" r="4"><animateMotion dur="1.3s" repeatCount="indefinite" path="${d}"/></circle>` : ''}
          ${!e.membership ? `<text x="${cx}" y="${cy-7}" class="ix-edge-count">${e.workTasks?$t('委派 {0}', [e.workTasks.length]):e.messages.length}</text>` : ''}</g>`;
      }).join('')}
      ${[...nodes.values()].map(n => {
        const p=positions.get(n.id), lit=edges.some(e => e.until>now && (e.from===n.id || e.to===n.id));
        return `<g data-tone="${n.group?'':toneOf(n.id)}" class="ix-node ${n.group?'group':''} ${focus===n.id?'focused':''} ${focus&&!neighborhood.has(n.id)?'dim':''} ${n.running?'running':''} ${n.tone||''} ${lit?'fresh':''}" data-node="${esc(n.id)}" transform="translate(${p.x} ${p.y})" tabindex="0" role="button" aria-label="${esc($t('{0}，{1}', [n.label, n.status]))}">
          <circle class="ix-node-halo" r="43" pathLength="300"/>${n.group?'<rect class="ix-node-face" x="-27" y="-27" width="54" height="54" rx="12" transform="rotate(45)"/>':'<circle class="ix-node-face" r="31"/>'}
          <text class="ix-node-initial" text-anchor="middle" y="6">${esc(n.group?'#':n.label.slice(0,2))}</text>
          <circle class="ix-node-state" cx="25" cy="22" r="6"/>
          <text class="ix-node-label" text-anchor="middle" y="59">${esc(short(n.label))}</text><text class="ix-node-caption" text-anchor="middle" y="77">${esc((n.engine?n.engine+' · ':'')+n.status)}</text></g>`;
      }).join('')}</g>`;
    transform();
    // Re-rendering restarts CSS animations; phase-lock running rings to the wall clock so they keep turning smoothly.
    svg.querySelectorAll('.ix-node.running .ix-node-halo').forEach(halo => { halo.style.animationDelay = `-${now % 2400}ms`; });
    if (focusedKey) [...svg.querySelectorAll('[data-node]')].find(n=>n.dataset.node===focusedKey)?.focus({preventScroll:true});
    else if (focusedEdge != null) svg.querySelector(`[data-edge="${focusedEdge}"]`)?.focus({preventScroll:true});
    $('#ix-graph-count').textContent = $t('{0} 个节点 · {1} 条通信路径', [nodes.size, edges.filter(e=>!e.membership).length]);
    $('#ix-graph-empty').classList.toggle('hidden', nodes.size > 0);
    $('#ix-graph-empty').textContent = $t('当前范围暂无交互 · 展开筛选切换身份或载入更多会话');
    $('#ix-unfocus').disabled = !focus;
  }
  function showTip(element) {
    if (!element) { tip.classList.add('hidden'); return; }
    const node = nodes.get(element.dataset.node), edge = edges[Number(element.dataset.edge)];
    const relevant = node ? edges.filter(e=>e.from===node.id || e.to===node.id).flatMap(e=>e.messages) : edge?.messages || [];
    const latest = relevant.sort((a,b)=>b.created_at-a.created_at)[0];
    tip.textContent = [node ? node.label+' · '+node.status : nodes.get(edge.from).label+' → '+nodes.get(edge.to).label,
      node?.reason, edge?.workTasks ? $t('{0} 项任务委派，点击查看任务', [edge.workTasks.length]) : latest ? (latest.deleted_at?$t('消息已删除'):latest.body).slice(0,220) : $t('当前范围暂无聊天消息'), $t('点击聚焦 · 拖动节点调整位置')].filter(Boolean).join('\n');
    tip.classList.remove('hidden');
  }
  function choose(element) {
    const node = nodes.get(element?.dataset.node), edge = edges[Number(element?.dataset.edge)];
    if (node) {
      focus = focus===node.id ? null : node.id;
      const latest = edges.filter(e=>e.from===node.id || e.to===node.id).flatMap(e=>e.messages).sort((a,b)=>b.created_at-a.created_at)[0];
      chosen=latest?.id || null; onSelect(chosen, node);
      draw();
    } else if (edge) {
      focus = edge.from;
      if(edge.workTasks){onSelect({kind:'task',id:edge.workTasks[0].id}, {taskIds:edge.workTasks.map(t=>t.id)});draw();return;}
      chosen=edge.messages[0]?.id || null; onSelect(chosen, {label:nodes.get(edge.from).label+' → '+nodes.get(edge.to).label,status:$t('成员关系'),reason:$t('这条虚线表示群聊成员关系，不表示已发送或已读消息。')});
      draw();
    }
  }
  const hit = event => event.target.closest('[data-node], [data-edge]');
  svg.onpointerover = e => { if(!drag) showTip(hit(e)); };
  svg.onpointerleave = () => tip.classList.add('hidden');
  svg.onfocusin = e => { if(!drag) showTip(hit(e)); };
  svg.onfocusout = () => tip.classList.add('hidden');
  svg.onpointerdown = e => {
    if(e.button!==0 || drag) return;
    const element=hit(e), p=point(e), id=element?.dataset.node;
    drag={pointer:e.pointerId, id, element, start:p, position:id?{...positions.get(id)}:{...camera}};
    moved=false; svg.setPointerCapture(e.pointerId); tip.classList.add('hidden');
  };
  svg.onpointermove = e => {
    if(!drag || e.pointerId!==drag.pointer) return;
    const p=point(e), dx=p.x-drag.start.x, dy=p.y-drag.start.y;
    moved ||= Math.hypot(dx,dy)>4;
    if(!moved) return;
    if(drag.id) { positions.set(drag.id,{x:drag.position.x+dx/camera.k,y:drag.position.y+dy/camera.k}); draw(); }
    else { camera.x=drag.position.x+dx; camera.y=drag.position.y+dy; transform(); }
  };
  svg.onpointerup = e => {
    if(!drag || e.pointerId!==drag.pointer) return;
    const element=drag.element; drag=null; svg.releasePointerCapture(e.pointerId);
    if(!moved) { if(element) choose(element); else { focus=null; draw(); } }
  };
  svg.onpointercancel = () => { drag=null; };
  svg.onkeydown = e => {
    if(e.key==='Enter' || e.key===' ') { if(hit(e)) {e.preventDefault(); choose(hit(e));} }
    if(e.key==='Escape') { focus=null; draw(); tip.classList.add('hidden'); }
    if(['ArrowLeft','ArrowRight','ArrowUp','ArrowDown'].includes(e.key)) {
      e.preventDefault(); const id=hit(e)?.dataset.node, dx=({ArrowLeft:-20,ArrowRight:20})[e.key]||0, dy=({ArrowUp:-20,ArrowDown:20})[e.key]||0;
      if(id) { const p=positions.get(id); p.x+=dx; p.y+=dy; draw(); } else {camera.x+=dx;camera.y+=dy;transform();}
    }
  };
  function zoom(factor, anchor={x:480,y:310}) {
    const p=world(anchor), k=Math.max(.2,Math.min(3,camera.k*factor));
    camera={k,x:anchor.x-p.x*k,y:anchor.y-p.y*k}; transform(); tip.classList.add('hidden');
  }
  svg.addEventListener('wheel',e=>{e.preventDefault();zoom(Math.exp(-e.deltaY*.001),point(e));},{passive:false});
  $('#ix-zoom-in').onclick=()=>zoom(1.2); $('#ix-zoom-out').onclick=()=>zoom(1/1.2);
  $('#ix-fit').onclick=fit; $('#ix-layout').onclick=()=>{layout();draw();fit();};
  $('#ix-unfocus').onclick=()=>{focus=null;draw();};
  return {
    update(rows, tasks, online, recent, selected, workTasks = [], approvals = []) {
      chosen=selected;
      const next=new Map(), paths=new Map();
      const add=(id,label,group=false)=>{if(!next.has(id))next.set(id,{id,label,group,degree:0,status:group?$t('群聊'):$t('暂无执行'),running:false});return id;};
      for(const {item,message:m} of rows) {
        const from=add('p:'+m.sender_id,nameOf(m.sender_id));
        let to;
        if(item.conversation.kind==='direct') {
          const peers=item.members.filter(x=>x.principal_id!==m.sender_id);
          to=peers.length===1?add('p:'+peers[0].principal_id,nameOf(peers[0].principal_id)):add('c:'+item.conversation.id,$t('私聊 · 对象未确认'),true);
        } else {
          to=add('c:'+item.conversation.id,item.conversation.title||$t('群聊'),true);
          for(const member of item.members.filter(x=>x.left_message_seq==null)) {
            const memberId=add('p:'+member.principal_id,nameOf(member.principal_id)), key=JSON.stringify([to,memberId]);
            if(!paths.has(key))paths.set(key,{from:to,to:memberId,messages:[],membership:true,until:0});
          }
        }
        const key=JSON.stringify([from,to,item.conversation.id]);
        if(!paths.has(key))paths.set(key,{from,to,messages:[],until:0});
        const edge=paths.get(key);edge.messages.push(m);edge.until=Math.max(edge.until,recent.get(m.id)||0);
      }
      for(const task of workTasks) {
        const from=add('p:'+task.request.requester,nameOf(task.request.requester));
        const to=add('p:'+task.request.assignee,nameOf(task.request.assignee));
        const key=JSON.stringify(['work',from,to]);
        if(!paths.has(key))paths.set(key,{from,to,messages:[],workTasks:[],until:0});
        paths.get(key).workTasks.push(task);
      }
      for(const a of approvals) if(a.principal) add('p:'+a.principal,nameOf(a.principal));
      nodes=next;edges=[...paths.values()];
      edges.forEach(e=>{nodes.get(e.from).degree++;nodes.get(e.to).degree++;});
      const allowed=new Set([...rows.map(r=>r.item.conversation.id),...workTasks.map(t=>t.request.report_to),...approvals.map(a=>a.conversation)]);
      const activeStates=new Set(['running','responding','evaluating']);
      const ordered=tasks.filter(t=>allowed.has(t.conversation)).sort((a,b)=>Number(activeStates.has(b.state))-Number(activeStates.has(a.state)) || b.due_at-a.due_at);
      const seen=new Set();
      for(const t of ordered) {
        const n=nodes.get('p:'+t.principal);
        if(!n || seen.has(n.id))continue;seen.add(n.id);
        n.running=online && activeStates.has(t.state);
        n.tone=online ? ({waiting:'waiting',queued:'waiting',failed:'failed'})[t.state] || '' : '';
        n.status=!online?$t('状态待确认'):({running:$t('执行中'),input:$t('已接入当前执行'),responding:$t('回复中'),evaluating:$t('判断中'),waiting:$t('等待中'),queued:$t('排队中'),ready_to_reply:$t('等待回复'),blocked:$t('受阻'),limited:$t('已达限制'),failed:$t('执行失败'),canceled:$t('已取消'),completed:$t('已完成'),silent:$t('暂不发言')})[t.state]||t.state;
        n.reason=t.last_decision?.public_reason ? $t('公开理由：{0}', [t.last_decision.public_reason]) : '';
      }
      for(const n of nodes.values()) {
        if(n.group)continue;
        const principal=n.id.slice(2), binding=state.boot?.bindings?.find(b=>b.principal===principal);
        const agent=binding && items('agent').find(a=>ref(a)===binding.agent);
        const harness=agent?.harness && items('harness').find(h=>ref(h)===ref(agent.harness));
        n.engine=harness?.engine || (agent?.harness?'Harness':'');
        const work=workTasks.filter(t=>t.request.assignee===principal && !['succeeded','failed','canceled','interrupted','outcome_unknown'].includes(t.state));
        if(work.length){n.status=$t('{0} 项活动任务', [work.length]);n.running=online&&work.some(t=>t.state==='running');}
        n.approvalIds=approvals.filter(a=>a.principal===principal && ['reviewing','awaiting_human'].includes(a.state)&&a.delivery==='not_sent').map(a=>a.id);
        if(n.approvalIds.length){n.status=$t('{0} 项权限审核', [n.approvalIds.length]);n.tone='waiting';}
      }
      const initial=positions.size===0;
      if(initial)layout();
      else [...nodes.keys()].filter(id=>!positions.has(id)).forEach((id,i)=>positions.set(id,{x:180+i*160,y:80}));
      if(focus&&!nodes.has(focus))focus=null;
      draw();if(initial)fit();
      clearTimeout(expiry);
      const remaining=edges.map(e=>e.until-Date.now()).filter(x=>x>0);
      if(remaining.length)expiry=setTimeout(draw,Math.max(...remaining)+30);
    },
    reset() {clearTimeout(expiry);nodes.clear();edges=[];positions.clear();focus=null;chosen=null;drag=null;camera={x:0,y:0,k:1};tip.classList.add('hidden');draw();}
  };
};
