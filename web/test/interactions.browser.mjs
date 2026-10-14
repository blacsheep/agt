import { createConsole, cliRunner } from '../server.mjs';
import { fileURLToPath } from 'node:url';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
const home = await mkdtemp('/tmp/agt-resource-browser-');
const profile = await mkdtemp('/tmp/agt-resource-chrome-');
const run = cliRunner(fileURLToPath(new URL('../../target/debug/agt', import.meta.url)), home);
let server, chrome, socket;
try {
  await run(['init', home]);
  await run(['im', 'principal', 'create', 'operator']);
  ({ server } = createConsole({ run, home, principal: 'operator', token: 'browser-test-token' }));
  await new Promise(ok => server.listen(0, '127.0.0.1', ok));
  const url = `http://127.0.0.1:${server.address().port}`;
  chrome = spawn(process.env.CHROME_BINARY || '/opt/google/chrome/chrome', ['--headless=new', '--accept-lang=zh-CN', '--no-sandbox', '--disable-dev-shm-usage', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
  const endpoint = await new Promise((ok, fail) => { let log = ''; chrome.stderr.on('data', chunk => { log += chunk; const match = log.match(/DevTools listening on (ws:\/\/\S+)/); if (match) ok(match[1]); }); chrome.on('error', fail); });
  socket = new WebSocket(endpoint);
  await new Promise(ok => socket.addEventListener('open', ok, { once: true }));
  let sequence = 0; const pending = new Map(); const errors = [];
  socket.addEventListener('message', event => { const value = JSON.parse(event.data); if (value.id) { const p = pending.get(value.id); pending.delete(value.id); value.error ? p.reject(value.error) : p.resolve(value.result); } else if (value.method === 'Runtime.exceptionThrown') errors.push(value.params.exceptionDetails.exception?.description || value.params.exceptionDetails.text); });
  const command = (method, params = {}, sessionId) => new Promise((resolve, reject) => { const id = ++sequence; pending.set(id, { resolve, reject }); socket.send(JSON.stringify({ id, method, params, sessionId })); });
  const { targetId } = await command('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await command('Target.attachToTarget', { targetId, flatten: true });
  const cdp = (method, params) => command(method, params, sessionId);
  await cdp('Runtime.enable');
  await cdp('Page.enable');
  await cdp('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
  await cdp('Page.navigate', { url: `${url}/#browser-test-token` });
  const evaluate = async expression => { const value = await cdp('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }); if (value.exceptionDetails) throw new Error(JSON.stringify(value.exceptionDetails)); return value.result.value; };
  const wait = async expression => { for (let i = 0; i < 150; i++) { if (await evaluate(expression)) return; await new Promise(ok => setTimeout(ok, 100)); } throw new Error(`Timed out: ${expression}`); };
  const click = selector => evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);
  await wait('document.querySelector("#connection")?.textContent === "本地已连接"');


  await evaluate(`
    state.polling = true;
    state.boot.principals.push({id:'lead',name:'Lead',kind:'agent'},{id:'coder',name:'Coder',kind:'agent'});
    window.originalApi = api;
    window.ixReads = [];
    api = async (path, data) => {
      if (path === 'read') {
        ixReads.push(data);
        const id = data.viewer === 'coder' ? 'private' : 'team';
        if(data.name === 'conversation_list') return [{conversation:{id,kind:'direct',title:id,updated_at:1000}}];
        if(data.name === 'member_list') return [{principal_id:'lead'},{principal_id:'coder'}];
        if(data.name === 'message_history') return {messages: data.args.before ? [{id:'older',sender_id:'coder',seq:1,body:'earlier record',created_at:100}] : [{id:id+'-1',sender_id:'lead',seq:2,body:'<img src=x onerror="window.injected=true">'+'长内容'.repeat(300),created_at:1000},{id:id+'-2',sender_id:'coder',seq:3,body:'测试完成',created_at:2000}],next_cursor:data.args.before ? null : 2};
        throw new Error('unexpected read');
      }
      if(path.startsWith('activity')) return {scheduler:{online:true,tasks:[{id:'t',conversation:'team',principal:'lead',state:'waiting',due_at:1000,last_decision:{action:'wait',public_reason:'等待测试结果',decided_at:1100,through_seq:2}},{id:'secret',conversation:'other',principal:'secret',state:'running'}]},runs:[]};
      return originalApi(path,data);
    };
    switchPage('interactions');
  `);
  // The page opens in the global view: every identity's conversations, each read as a member.
  await wait('document.querySelector("#ix-viewer").value === "*" && document.querySelectorAll(".ix-row").length === 4 && document.querySelector("[data-ix-message=private-1]")');
  assert.equal(await evaluate('ixReads.some(r => r.name === "message_history" && r.args.conversation === "private" && r.viewer === "coder")'), true);
  await evaluate('document.querySelector("#ix-viewer").value="operator";document.querySelector("#ix-viewer").dispatchEvent(new Event("change"));');
  await wait('document.querySelectorAll(".ix-row").length === 2');
  assert.equal(await evaluate('window.injected'), undefined);
  assert.equal(await evaluate('document.querySelectorAll(".ix-node").length'),2);
  assert.equal(await evaluate('document.querySelectorAll(".ix-edge:not(.membership)").length'),2);
  assert.equal(await evaluate('document.querySelectorAll(".ix-packet").length'),0);
  await evaluate(`document.querySelector('[data-node="p:lead"]').dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));`);
  assert.equal(await evaluate('document.querySelectorAll(".ix-node.focused").length'),1);
  await click('#ix-unfocus');
  const beforeZoom = await evaluate('document.querySelector("#ix-zoom-level").textContent');
  await click('#ix-zoom-in');
  assert.notEqual(await evaluate('document.querySelector("#ix-zoom-level").textContent'),beforeZoom);
  await click('#ix-fit');
  const beforeMove = await evaluate(`document.querySelector('[data-node="p:lead"]').getAttribute('transform')`);
  await evaluate(`document.querySelector('[data-node="p:lead"]').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true}));`);
  assert.notEqual(await evaluate(`document.querySelector('[data-node="p:lead"]').getAttribute('transform')`),beforeMove);
  await evaluate('document.querySelector(".ix-events").open=true;');

  await click('[data-ix-message="team-1"]');
  await wait('document.querySelector("#ix-execution-body").textContent.includes("等待测试结果")');
  assert.equal(await evaluate('document.querySelector("#ix-execution-body").textContent.includes("secret")'),false);
  assert.equal(await evaluate('document.querySelector(".ix-message details").open'),false);
  await click('.ix-message summary');
  assert.equal(await evaluate('document.querySelector(".ix-message details").open'),true);
  await evaluate("state.page='interactions';AgtInteractions.refresh()");
  assert.equal(await evaluate('document.querySelector(".ix-message details").open'),true);
  await evaluate('document.querySelector("#ix-query").value="测试完成";document.querySelector("#ix-query").dispatchEvent(new Event("input"));');
  assert.equal(await evaluate('document.querySelectorAll(".ix-row").length'),1);
  await click('#ix-clear');
  await evaluate('document.querySelector("#ix-conversation").value="team";document.querySelector("#ix-conversation").dispatchEvent(new Event("change"));');
  await click('#ix-earlier');
  await wait('document.querySelectorAll(".ix-row").length === 3');
  for(const width of [1440,1024,768,390,360]){
    await cdp('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:width<600});
    assert.equal(await evaluate('document.documentElement.scrollWidth <= innerWidth'),true,'overflow at '+width);
  }
  await cdp('Emulation.setDeviceMetricsOverride',{width:1440,height:1000,deviceScaleFactor:1,mobile:false});
  const screenshot = await cdp('Page.captureScreenshot',{format:'png'});
  await writeFile('/tmp/agt-interactions.png',Buffer.from(screenshot.data,'base64'));
  await evaluate('document.querySelector("#ix-viewer").value="coder";document.querySelector("#ix-viewer").dispatchEvent(new Event("change"));');
  await wait('document.querySelectorAll(".ix-row").length === 2 && document.querySelector("[data-ix-message=private-1]")');
  assert.equal(await evaluate('document.querySelector("[data-ix-message=team-1]") === null'),true);
  assert.equal(await evaluate('document.querySelector("#ix-detail").textContent.includes("等待测试结果")'),false);
  // A request from the previous identity must never populate the new view.
  await evaluate(`
    window.currentApi = api;
    api = async (path,data) => {
      if(path==='read' && data.name==='message_history' && data.viewer==='coder') {
        await new Promise(resolve=>setTimeout(resolve,300));
        return {messages:[{id:'stale-secret',sender_id:'coder',seq:99,body:'stale secret',created_at:3000}],next_cursor:null};
      }
      return currentApi(path,data);
    };
    AgtInteractions.refresh();
  `);
  await evaluate('document.querySelector("#ix-viewer").value="operator";document.querySelector("#ix-viewer").dispatchEvent(new Event("change"));');
  await wait('document.querySelector("[data-ix-message=team-1]") !== null');
  await new Promise(resolve=>setTimeout(resolve,450));
  assert.equal(await evaluate('document.querySelector("[data-ix-message=stale-secret]") === null'),true);
  await evaluate(`
    api = async (path,data) => {
      if(path==='read' && data.name==='conversation_list') throw new Error('测试读取失败');
      return currentApi(path,data);
    };
    AgtInteractions.refresh();
  `);
  assert.match(await evaluate('document.querySelector("#ix-status").textContent'),/测试读取失败/);
  assert.equal(await evaluate('document.querySelector("#ix-refresh").disabled'),false);

  // Group conversations use explicit hubs; historical reads do not animate.
  await evaluate(`
    window.graphNewMessage = false;
    state.boot.principals.push({id:'reviewer',name:'Reviewer',kind:'agent'},{id:'researcher',name:'Researcher',kind:'agent'});
    state.scheduler = {online:true,tasks:[{id:'live',principal:'coder',conversation:'network',state:'responding',due_at:2000},{id:'hidden',principal:'outsider',conversation:'hidden',state:'responding',due_at:2000}]};
    api = async (path,data) => {
      if(path==='read'){
        if(data.name==='conversation_list')return [{conversation:{id:'network',kind:'group',title:'产品研发协作',updated_at:3000}}];
        if(data.name==='member_list')return ['lead','coder','reviewer','researcher'].map(principal_id=>({principal_id,left_message_seq:null}));
        if(data.name==='message_history')return {messages:[
          {id:'network-1',sender_id:'lead',seq:1,body:'请实现第一阶段功能，并完成测试。',created_at:1000},
          {id:'network-2',sender_id:'reviewer',seq:2,body:'我会核验接口边界和测试覆盖。',created_at:2000},
          ...(graphNewMessage?[{id:'network-3',sender_id:'coder',seq:3,body:'实现完成，开始运行测试。',created_at:3000}]:[])
        ],next_cursor:null};
      }
      return currentApi(path,data);
    };
    document.querySelector('#ix-conversation').value='';
    document.querySelector('.ix-events').open=false;
    AgtInteractions.refresh();
  `);
  assert.equal(await evaluate('document.querySelectorAll(".ix-node.group").length'),1);
  assert.equal(await evaluate('document.querySelectorAll(".ix-node").length'),5);
  assert.equal(await evaluate('document.querySelectorAll(".ix-edge.membership").length'),4);
  assert.equal(await evaluate('document.querySelectorAll(".ix-node.running").length'),1);
  assert.equal(await evaluate('document.querySelectorAll(".ix-packet").length'),0);
  await evaluate('graphNewMessage=true; AgtInteractions.refresh();');
  assert.equal(await evaluate('document.querySelectorAll(".ix-packet").length'),1);
  await click('#ix-layout');
  await evaluate('document.querySelector("#ix-graph").scrollIntoView({block:"center"})');
  const position = await evaluate(`(() => {const p=document.querySelector('[data-node="p:coder"]').getBoundingClientRect();return {x:p.x+p.width/2,y:p.y+25};})()`);
  const initialPosition = await evaluate(`document.querySelector('[data-node="p:coder"]').getAttribute('transform')`);
  await cdp('Input.dispatchMouseEvent',{type:'mousePressed',x:position.x,y:position.y,button:'left',clickCount:1});
  await cdp('Input.dispatchMouseEvent',{type:'mouseMoved',x:position.x+45,y:position.y+20,button:'left',buttons:1});
  await cdp('Input.dispatchMouseEvent',{type:'mouseReleased',x:position.x+45,y:position.y+20,button:'left',clickCount:1});
  assert.notEqual(await evaluate(`document.querySelector('[data-node="p:coder"]').getAttribute('transform')`),initialPosition);
  await evaluate('document.querySelector("#ix-graph").dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true}))');
  const networkScreenshot=await cdp('Page.captureScreenshot',{format:'png'});
  await writeFile('/tmp/agt-network.png',Buffer.from(networkScreenshot.data,'base64'));
  await evaluate('state.scheduler.online=false; AgtInteractions.refresh();');
  assert.equal(await evaluate('document.querySelectorAll(".ix-node.running").length'),0);
  await new Promise(resolve=>setTimeout(resolve,6600));
  assert.equal(await evaluate('document.querySelectorAll(".ix-packet").length'),0);


  await evaluate(`
    state.scheduler={online:true,tasks:[],discussions:[]};
    window.delegated={id:'work-only',list_cursor:1,state:'running',run_id:'run-only',execution_engine:'codex',request:{requester:'lead',assignee:'coder',report_to:'task-chat',input:'implement <img src=x onerror="window.injected=true">'}};
    window.approvalRecord={id:'approval-only',revision:1,source:'harness',principal:'coder',work_id:'work-only',run_id:'run-only',conversation:'task-chat',state:'awaiting_human',delivery:'not_sent',reason:'',expires_at:Date.now()+60000,request:{params:{command:'echo <script>unsafe</script>'},options:[{label:'批准本次',value:'accept'},{label:'拒绝本次',value:'decline'}]}};
    window.approvalWrites=[];
    state.boot.bindings.push({principal:'coder',agent:'default/coder'});
    state.boot.resources.agent=[{category:'default',name:'coder',harness:{category:'default',name:'codex'}}];
    state.boot.resources.harness=[{category:'default',name:'codex',engine:'codex'}];
    api=async(path,data)=>{
      if(path==='read'){
        if(data.name==='task_list')return data.viewer==='operator'?[delegated,delegated]:[];
        if(data.name==='task_get')return delegated;
        if(data.name==='conversation_list')return [];
        throw new Error('unexpected read '+data.name);
      }
      if(path==='approvals'&&data){approvalWrites.push(data);approvalRecord={...approvalRecord,revision:2,state:'denied',decision:data.decision};return approvalRecord;}
      if(path.startsWith('approvals?')){const p=new URLSearchParams(path.split('?')[1]);return p.has('id')?approvalRecord:p.get('viewer')==='operator'?[approvalRecord]:[];}
      if(path.startsWith('activity'))return {scheduler:{online:true,tasks:[]},runs:[]};
      return originalApi(path,data);
    };
    document.querySelector('#ix-viewer').value='operator';document.querySelector('#ix-viewer').dispatchEvent(new Event('change'));
  `);
  await wait('document.querySelectorAll(".ix-edge.delegation").length===1 && document.querySelectorAll("[data-ix-task]").length===1');
  assert.equal(await evaluate('document.querySelectorAll(".ix-node").length'),2);
  assert.match(await evaluate('document.querySelector("#ix-graph").textContent'),/codex/);
  await evaluate('document.querySelector(".ix-edge.delegation").dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true}))');
  await wait('document.querySelector("#ix-detail").textContent.includes("执行引擎：codex")');
  assert.equal(await evaluate('window.injected'),undefined);
  await click('[data-task-approval]');
  await wait('!!document.querySelector("#approval-form")');
  assert.equal(await evaluate('document.querySelector("#approval-form select").value'),'');
  await wait('!!document.querySelector(".permission-mode-form")');
  assert.equal(await evaluate('!!document.querySelector("#approval-all")'),false);
  assert.match(await evaluate('document.querySelector("#command-panel-body").textContent'),/不经过 Native 审核模型/);
  assert.equal(await evaluate('document.querySelector("#approval-form").checkValidity()'),false);
  assert.equal(await evaluate('document.querySelector("#approval-form").getBoundingClientRect().width>0'),true);
  for(const width of [1280,390,360]) {
    await cdp('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:width<600});
    assert.equal(await evaluate('document.documentElement.scrollWidth<=innerWidth'),true);
  }

  await evaluate('document.querySelector("#approval-form select").value="1";document.querySelector("#approval-form").requestSubmit()');
  await wait('approvalWrites.length===1 && !document.querySelector("#approval-form")');
  assert.equal(await evaluate('approvalWrites[0].decision'),'decline');
  assert.equal(await evaluate('approvalWrites[0].expected_revision'),1);
  await evaluate(`approvalRecord={...approvalRecord,state:'approved',decision:'accept',decision_origin:'model_review',delivery:'not_sent'};AgtApprovals.refresh()`);
  await wait('document.querySelector("#command-panel-body").textContent.includes("决定来源：模型审核")');
  assert.match(await evaluate('document.querySelector("#command-panel-body").textContent'),/无需再次审批/);
  await evaluate(`approvalRecord={...approvalRecord,delivery:'resolved'};AgtApprovals.refresh()`);
  await wait('document.querySelector("#command-panel-body").textContent.includes("审批处理已结束")');
  await evaluate(`approvalRecord={...approvalRecord,revision:3,state:'awaiting_human',delivery:'not_sent'};state.page='observer';AgtApprovals.open('approval-only')`);
  await wait('!!document.querySelector(".permission-mode-form")');
  await evaluate(`document.querySelector('.permission-mode-form [name="native"]').value='auto_allow';document.querySelector('.permission-mode-form [name="harness"]').value='auto_allow';document.querySelector('.permission-mode-form').requestSubmit()`);
  await wait('document.querySelector(".permission-mode-feedback").textContent.startsWith("已保存")');
  assert.equal(await evaluate('approvalWrites.length'),1);
  assert.equal((await run(['config','show'])).permissions.harness_mode,'auto_allow');
  await evaluate('AgtApprovals.open()');
  await wait(`document.querySelector('.permission-mode-form [name="native"]')?.value==="auto_allow"`);
  assert.equal(await evaluate(`document.querySelector('.permission-mode-form [name="harness"]').value`),'auto_allow');
  await evaluate("state.page='interactions'");
  await evaluate('AgtInteractions.refresh()');
  assert.equal(await evaluate('document.querySelectorAll(".ix-edge.delegation").length'),1);
  await evaluate('document.querySelector("#ix-viewer").value="coder";document.querySelector("#ix-viewer").dispatchEvent(new Event("change"))');
  await wait('document.querySelectorAll(".ix-node").length===0 && !document.querySelector("#command-panel-body").textContent.includes("unsafe")');
  assert.equal(await evaluate('document.querySelectorAll("#approval-form").length'),0);
  await evaluate('AgtApprovals.open()');
  await wait('!document.querySelector("#command-panel-body").textContent.includes("正在读取")');
  assert.equal(await evaluate('document.querySelectorAll("#approval-all").length'),0);
  await wait('!!document.querySelector(".permission-mode-form select")');
  assert.equal(await evaluate('document.querySelector(".permission-mode-form select").disabled'),true);

  assert.deepEqual(errors,[]);
  console.log('Interaction browser passed: scoped messages and delegation edges, Harness identity, approval decisions, observer isolation, XSS and responsive widths.');
} finally {
  socket?.close(); chrome?.kill('SIGTERM'); run.close(); server?.closeAllConnections(); server?.close();
  await new Promise(ok => setTimeout(ok, 500));
  await rm(home, { recursive: true, force: true }); await rm(profile, { recursive: true, force: true });
}
