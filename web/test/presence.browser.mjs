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
  chrome = spawn(process.env.CHROME_BINARY || '/opt/google/chrome/chrome', ['--headless=new', '--no-sandbox', '--disable-dev-shm-usage', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
  const endpoint = await new Promise((ok, fail) => { let log = ''; chrome.stderr.on('data', chunk => { log += chunk; const match = log.match(/DevTools listening on (ws:\/\/\S+)/); if (match) ok(match[1]); }); chrome.on('error', fail); });
  socket = new WebSocket(endpoint);
  await new Promise(ok => socket.addEventListener('open', ok, { once: true }));
  let sequence = 0; const pending = new Map(); const errors = [];
  socket.addEventListener('message', event => { const value = JSON.parse(event.data); if (value.id) { const p = pending.get(value.id); pending.delete(value.id); value.error ? p.reject(value.error) : p.resolve(value.result); } else if (value.method === 'Runtime.exceptionThrown') errors.push(value.params.exceptionDetails.text); });
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
    const chat = { id: 'sample-chat', kind: 'group', state: 'active', title: '示例协作', last_message_seq: 3 };
    state.conversation = chat;
    state.conversations = [{ conversation: chat }];
    state.boot.principals.push(...Array.from({length: 14}, (_, i) => ({id: 'sample-' + i, name: i === 0 ? 'tech_lead_长名称展示检查' : '成员 ' + i, kind: 'agent'})));
    state.scheduler = { online: true, discussions: [{id: 'round', conversation: chat.id, state: 'active'}], tasks: Array.from({length: 14}, (_, i) => ({id: 'task-' + i, principal: 'sample-' + i, conversation: chat.id, discussion: 'round', state: i === 0 ? 'waiting' : i === 1 ? 'ready_to_reply' : 'silent', through_seq: 3, due_at: Date.now() + 30000, quiet: false, last_decision: {action: i === 0 ? 'wait' : i === 1 ? 'participate' : 'silent', reason_code: i === 0 ? 'observe_discussion' : 'already_answered', public_reason: i === 0 ? '示例说明：<script>window.injected = true</script>' + '等待其他成员补充评估。'.repeat(15) : '', decided_at: Date.now(), through_seq: 3, ...(i === 0 ? {wake: {type: 'after', seconds: 30}} : {})}})), usage: [] };
    state.runs = [];
    renderReplyStatus(); renderActivity();
  `);
  assert.equal(await evaluate('document.querySelectorAll(".presence-card").length'), 14);
  assert.equal(await evaluate('window.injected === undefined'), true);
  assert.match(await evaluate('document.querySelector("#reply-detail").textContent'), /1 位观察中/);
  assert.match(await evaluate('document.querySelector(".presence-card").textContent'), /模型自主/);
  assert.equal(await evaluate('getComputedStyle(document.querySelector(".presence-preview")).webkitLineClamp'), '2');
  await click('.presence-card > summary');
  await evaluate('window.savedSummary = document.querySelector(".presence-card > summary"); savedSummary.focus(); renderActivity(); renderActivity();');
  assert.equal(await evaluate('savedSummary === document.activeElement && savedSummary.parentElement.open'), true);
  await evaluate('state.scheduler.tasks[0].state = "evaluating"; renderActivity();');
  assert.equal(await evaluate('savedSummary === document.activeElement && savedSummary.parentElement.open'), true);
  assert.match(await evaluate('savedSummary.parentElement.textContent'), /重新判断中/);
  assert.match(await evaluate('savedSummary.parentElement.textContent'), /这是之前的判断/);
  await evaluate('state.scheduler.tasks[0].state = "queued"; state.scheduler.discussions[0].state = "paused"; renderActivity();');
  assert.match(await evaluate('savedSummary.textContent'), /会话控制/);
  assert.match(await evaluate('savedSummary.textContent'), /已暂停/);
  await evaluate('state.scheduler.online = false; renderActivity();');
  assert.match(await evaluate('savedSummary.textContent'), /状态待确认/);
  assert.equal(await evaluate('document.querySelector(".execution-top").classList.contains("offline")'), true);
  await evaluate('state.scheduler.online = true; state.scheduler.discussions[0].state = "active"; state.scheduler.tasks[0].state = "waiting"; state.scheduler.tasks[1].state = "silent"; renderActivity();');
  assert.match(await evaluate('document.querySelectorAll(".presence-state")[1].textContent'), /无追加回复/);
  assert.match(await evaluate('document.querySelectorAll(".presence-body")[1].textContent'), /参与发言/);
  for (const width of [1440, 1280, 1024, 768, 390, 360]) {
    await cdp('Emulation.setDeviceMetricsOverride', {width, height: 900, deviceScaleFactor: 1, mobile: width < 761});
    assert.equal(await evaluate('document.documentElement.scrollWidth <= window.innerWidth'), true, `overflow at ${width}`);
    assert.notEqual(await evaluate('getComputedStyle(document.querySelector(".activity-pane")).display'), 'none');
    assert.equal(await evaluate('document.querySelector(".presence-card").scrollWidth <= document.querySelector(".presence-card").clientWidth + 1'), true, `card overflow at ${width}`);
  }
  await evaluate('(async () => { window.savedApi = api; api = async () => { throw new Error("fixture offline"); }; await activity().catch(() => {}); api = savedApi; })()');
  assert.equal(await evaluate('state.scheduler.online'), false);
  assert.match(await evaluate('savedSummary.textContent'), /状态待确认/);
  await evaluate('state.scheduler.online = true;');
  await evaluate('state.scheduler.tasks[0].last_decision = null; renderActivity();');
  assert.match(await evaluate('savedSummary.parentElement.textContent'), /暂无独立参与判断记录/);
  assert.doesNotMatch(await evaluate('savedSummary.textContent'), /模型自主/);
  await evaluate('state.conversation = {id: "other", state: "active"}; renderActivity();');
  assert.equal(await evaluate('document.querySelectorAll(".presence-card").length'), 0);
  assert.doesNotMatch(await evaluate('document.querySelector("#agent-presence").textContent'), /tech_lead/);
  await evaluate(`
    state.conversation = {id:'sample-chat', state:'active'};
    state.scheduler.online = true;
    state.runs = [{id:'live', agent:'default/a', state:'running', conversation:'sample-chat', progress:{principal:'sample-0', phase:'generating', attempt:1, started_at:Date.now()-9000, preview:'<script>window.streamInjected=true</script>' + '长正文'.repeat(2000)}}];
    renderActivity();
  `);
  assert.equal(await evaluate('document.querySelectorAll(".live-reply").length'), 1);
  assert.equal(await evaluate('window.streamInjected === undefined && !document.querySelector(".live-preview script")'), true);
  assert.match(await evaluate('document.querySelector(".live-phase").textContent'), /正在生成/);
  assert.equal(await evaluate('document.querySelector(".live-reply").open'), false);
  await evaluate(`state.runs[0].progress.activities = [
    {id:'call:1',kind:'framework',tool_name:'framework_harness_models',label:'查询可用模型',status:'running',started_at:Date.now()-2000,updated_at:Date.now()},
    {id:'call:2',kind:'mcp',tool_name:'search',status:'running',started_at:Date.now()-1000,updated_at:Date.now()},
    {id:'call:3',kind:'framework',tool_name:'framework_harness_session_configure',summary:'会话模型已更新',status:'completed',started_at:Date.now()-5000,updated_at:Date.now()-4000}
  ]; renderActivity();`);
  assert.match(await evaluate('document.querySelector(".live-phase").textContent'), /正在执行 2 项操作/);
  // Live work is one collapsed line per Agent until the user expands it.
  assert.equal(await evaluate('document.querySelector(".live-reply").open'),false);
  await click('.live-reply > summary');
  assert.match(await evaluate('document.querySelector(".harness-activity").textContent'), /framework_harness_models/);
  assert.match(await evaluate('document.querySelector(".harness-activity").textContent'), /会话模型已更新/);
  assert.match(await evaluate('activityTimeline([{kind:"framework",status:"unknown",started_at:1,updated_at:2}])'), /结果待确认/);

  await evaluate(`
    window.detailReads=[];window.detailApi=api;window.detailFailure=false;window.detailDelay=false;
    api=async(path,data)=>{
      if(path!=='tool-detail')return detailApi(path,data);
      detailReads.push(data);
      if(detailDelay)return new Promise(resolve=>window.resolveDetail=resolve);
      if(detailFailure){detailFailure=false;throw new Error('fixture detail unavailable');}
      return {fields:{command:{text:'cargo test --offline',truncated:false},arguments:{text:'{"path":"src/a.rs"}',truncated:false},aggregatedOutput:{text:'<script>window.rawInjected=true</script>\\n'+('tool output '.repeat(100)),truncated:true},changes:{text:'--- a/src/a.rs\\n+++ b/src/a.rs\\n-old\\n+new',truncated:false}}};
    };
  `);
  assert.equal(await evaluate('detailReads.length'),0);
  await click('[data-activity="call:2"] > summary');
  await wait(`document.querySelector('[data-activity="call:2"] .tool-payload pre')`);
  assert.match(await evaluate(`document.querySelector('[data-activity="call:2"] .tool-payload').textContent`),/cargo test --offline/);
  assert.match(await evaluate(`document.querySelector('[data-activity="call:2"] .tool-payload').textContent`),/已截断|\+new/);
  assert.equal(await evaluate('window.rawInjected===undefined && !document.querySelector(".tool-payload script")'),true);
  assert.equal(await evaluate('detailReads[0].conversation'), 'sample-chat');
  assert.equal(await evaluate('detailReads[0].id'), 'live');
  await evaluate('renderActivity();renderActivity()');
  assert.equal(await evaluate('detailReads.length'),1);
  await evaluate(`window.savedTool=document.querySelector('[data-activity="call:2"]');savedTool.querySelector('summary').focus();state.runs[0].progress.activities.reverse();state.runs[0].progress.activities.find(a=>a.id==='call:2').status='failed';renderActivity();`);
  assert.equal(await evaluate(`savedTool===document.querySelector('[data-activity="call:2"]') && savedTool.open && savedTool.contains(document.activeElement)`),true);
  assert.match(await evaluate(`savedTool.querySelector('.tool-status').textContent`),/失败/);
  assert.equal(await evaluate('document.querySelectorAll(".tool-card").length'),3);
  await wait('detailReads.length===2 && document.querySelector(".tool-payload pre")');
  await evaluate('detailFailure=true');
  await click('[data-activity="call:1"] > summary');
  await wait(`document.querySelector('[data-activity="call:1"] .tool-payload button')`);
  await click('[data-activity="call:1"] .tool-payload button');
  await wait(`document.querySelector('[data-activity="call:1"] .tool-payload pre')`);
  await evaluate('window.liveSummary = document.querySelector(".live-reply summary"); liveSummary.focus(); renderActivity();');
  assert.equal(await evaluate('document.activeElement === liveSummary && liveSummary.parentElement.open'), true);
  for (const width of [1280, 768, 390, 360]) {
    await cdp('Emulation.setDeviceMetricsOverride', {width, height:900, deviceScaleFactor:1, mobile:width<761});
    await evaluate('liveSummary.parentElement.open = true');
    assert.equal(await evaluate('document.documentElement.scrollWidth <= innerWidth'), true, `live overflow at ${width}`);
    assert.equal(await evaluate('document.querySelector("#live-replies").getBoundingClientRect().height <= 301'), true);
  }
  if (process.env.AGT_DESIGN_CAPTURE) {
    await cdp('Emulation.setDeviceMetricsOverride', {width:1280,height:900,deviceScaleFactor:1,mobile:false});
    await evaluate(`document.querySelector('[data-activity="call:2"] .tool-payload').scrollIntoView({block:'start'})`);
    const shot = await cdp('Page.captureScreenshot', {format:'png'});
    await writeFile('/tmp/agt-stream-preview.png', Buffer.from(shot.data, 'base64'));
  }
  await evaluate('state.runs[0].progress = {...state.runs[0].progress, preview:"", phase:"retrying", attempt:2}; renderActivity();');
  assert.equal(await evaluate('document.querySelector(".live-preview").textContent'), '');
  assert.match(await evaluate('document.querySelector(".live-reply").textContent'), /第 2\/10 次尝试/);
  await evaluate('state.runs[0].state = "completed"; renderActivity();');
  assert.equal(await evaluate('document.querySelectorAll(".live-reply").length'), 0);
  await evaluate('state.runs[0].state = "running"; renderActivity(); state.conversation = {id:"elsewhere",state:"active"}; renderActivity();');
  assert.equal(await evaluate('document.querySelectorAll(".live-reply").length'), 0);
  // Settled runs leave the live area entirely; their records live in message threads (run-threads.js).
  await evaluate(`
    state.conversation={id:'sample-chat',state:'active'};
    const retainedToolItems=state.runs[0].progress.activities.map(a=>({...a,status:a.id==='call:1'?'completed':a.status}));
    state.runs=[{id:'live',agent:'default/a',conversation:'sample-chat',state:'completed',progress:null,tool_activity:{principal:'sample-0',engine:'native',items:retainedToolItems,started_at:Date.now()-10000,ended_at:Date.now(),approvals:[]}}];
    renderActivity();
  `);
  assert.equal(await evaluate('document.querySelectorAll(".live-reply").length'),0);
  await evaluate(`state.scheduler.online=true;state.runs[0].state='running';state.runs[0].progress={phase:'tool_running',started_at:Date.now(),preview:''};state.runs[0].tool_activity.approvals=[{id:'permission',source:'native',state:'awaiting_human'}];renderActivity();`);
  assert.match(await evaluate('document.querySelector(".live-phase").textContent'),/等待人工审批/);
  // A pending approval opens the line by itself: it needs the user.
  assert.equal(await evaluate('document.querySelector(".live-reply").open'),true);
  assert.equal(await evaluate('document.querySelector(".run-approval-actions button").textContent'),'查看工具权限');
  await evaluate(`window.openedApproval=null;window.savedApprovals=AgtApprovals;window.AgtApprovals={open:id=>openedApproval=id};document.querySelector('.run-approval-actions button').click();window.AgtApprovals=savedApprovals;`);
  assert.equal(await evaluate('openedApproval'),'permission');
  await evaluate(`state.runs[0].tool_activity.approvals=[];state.runs[0].tool_activity.items=[{id:'call:9',kind:'files',status:'running',started_at:Date.now()-2000,updated_at:Date.now(),summary:'<img src=x onerror="window.toolInjected=true">'}];renderActivity();`);
  assert.equal(await evaluate('window.toolInjected===undefined && !document.querySelector(".tool-card img")'),true);
  assert.equal(await evaluate('document.querySelectorAll(".run-approval-actions button").length'),0);
  if(process.env.AGT_DESIGN_CAPTURE){
    await evaluate(`state.runs[0].tool_activity.items[0].summary='等待引擎报告文件更新结果';state.runs[0].tool_activity.items.push({id:'call:10',kind:'framework',tool_name:'framework_harness_models',status:'completed',started_at:Date.now()-3000,updated_at:Date.now()-1000,summary:'找到 3 个模型'});document.querySelector('.live-reply').open=true;renderActivity();`);
    await cdp('Emulation.setDeviceMetricsOverride',{width:1280,height:900,deviceScaleFactor:1,mobile:false});
    const shot=await cdp('Page.captureScreenshot',{format:'png'});await writeFile('/tmp/agt-tool-cards.png',Buffer.from(shot.data,'base64'));
  }
  await evaluate(`detailDelay=true;document.querySelector('[data-activity="call:9"]').open=true`);
  await wait('typeof resolveDetail==="function"');
  await evaluate('state.viewer="sample-1";clearConversation();resolveDetail({fields:{output:{text:"stale-private-detail",truncated:false}}});');
  assert.doesNotMatch(await evaluate('document.querySelector("#live-replies").textContent'),/stale-private-detail/);
  assert.equal(await evaluate('document.querySelectorAll(".live-reply").length'),0);
  // A failed refresh also clears retained facts whose membership can no longer be verified.
  await evaluate(`state.conversation={id:'sample-chat',state:'active'};state.runs[0].state='completed';state.runs[0].progress=null;renderActivity();`);
  await evaluate('(async()=>{const original=api;api=async()=>{throw new Error("revoked")};try{await activity()}catch{}finally{api=original}})()');
  assert.equal(await evaluate('document.querySelectorAll(".live-reply").length'),0);
  // Nothing to dismiss: terminal runs never appear here, resumed work always does.
  await evaluate(`state.viewer='operator';state.conversation={id:'sample-chat',state:'active'};state.scheduler.online=true;
    state.runs=[{id:'resumed',agent:'default/a',conversation:'sample-chat',state:'canceled',tool_activity:{principal:'a',items:[{id:'call:99',kind:'command',status:'unknown'}]}}];renderActivity();`);
  assert.equal(await evaluate('document.querySelectorAll(".live-reply").length'),0);
  await evaluate(`state.runs[0].state='running';state.runs[0].progress={phase:'waiting',principal:'a'};renderActivity()`);
  assert.equal(await evaluate('document.querySelectorAll(".live-reply").length'),1);
  assert.equal(await evaluate('!document.querySelector(".run-dismiss")'),true);
  assert.deepEqual(errors, []);
  console.log('Execution browser passed: keyed tool cards, live-only bar, approval actions, XSS, stable expansion/focus, identity cleanup and responsive widths.');
} finally {
  socket?.close(); chrome?.kill('SIGTERM'); run.close(); server?.closeAllConnections(); server?.close();
  await new Promise(ok => setTimeout(ok, 500));
  await rm(home, { recursive: true, force: true }); await rm(profile, { recursive: true, force: true });
}