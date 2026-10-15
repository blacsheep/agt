import { cliRunner, startConsole } from './support/console.mjs';
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
  await run(['im','owner','ensure','operator']);
  const call = (name,args) => run(['tool','call','--principal','operator','--manager',name,'--args',JSON.stringify(args)]);
  await run(['provider','create','fixture','--base-url','http://127.0.0.1:1/v1','--api-key','mock']);
  await run(['model','create','fixture','--provider','fixture','--model','mock']);
  await run(['agent','create','lead','--model','fixture']);
  await call('principal_create',{id:'lead',name:'Lead',kind:'agent'});
  await call('agent_bind',{principal:'lead',agent:'lead',working_directory:home,policy:'manual'});
  const chat = await call('direct_create',{principal:'lead'});
  const message = await call('message_send',{conversation:chat.id,body:'实现任务面板 <script>不可执行</script>',request_key:'source'});
  await run(['schedule','start',chat.id,'--message',message.message.id]);
  const task = await call('task_request',{assignee:'lead',conversation:chat.id,input:'检查事件重连并补充验证',request_key:'task',source_message:message.message.id});
  server = await startConsole({ run, principal: 'operator', token: 'browser-test-token' });
  const url = server.url;
  chrome = spawn(process.env.CHROME_BINARY || '/opt/google/chrome/chrome', ['--headless=new', '--accept-lang=zh-CN', '--no-sandbox', '--disable-dev-shm-usage', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
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
  await click(`[data-conv="${chat.id}"]`);
  await click('#chat-tasks');
  await wait('document.querySelectorAll(".work-row").length === 1');
  assert.match(await evaluate('document.querySelector("#task-count").textContent'), /1 项/);
  await click('.work-row');
  await wait('document.querySelector("[data-task-action=cancel]") !== null');
  assert.match(await evaluate('document.querySelector(".work-detail").textContent'), /待处理/);
  for (const width of [1280,768,390,360]) {
    await cdp('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:width<761});
    assert.equal(await evaluate('document.documentElement.scrollWidth <= innerWidth'),true,'task panel overflow '+width);
  }
  await click('[data-task-action=cancel]');
  await evaluate('document.querySelector(".task-action-form textarea").value="用户取消测试任务"; document.querySelector(".task-action-form").requestSubmit();');
  await wait('document.querySelector(".work-state.canceled") !== null');
  assert.equal((await call('task_get',{task_id:task.id})).state,'canceled');
  assert.match(await evaluate('document.querySelector(".task-termination").textContent'), /任务被主动取消/);
  assert.match(await evaluate('document.querySelector(".task-termination").textContent'), /尚未开始执行/);
  assert.equal(await evaluate('document.querySelector(".work-detail").textContent.includes("正在确认执行状态")'),false);
  await evaluate(`window.actualTaskApi=api;window.taskFixture=null;api=async (...args)=>args[0]==='read' && args[1]?.name==='task_get' && window.taskFixture ? window.taskFixture : window.actualTaskApi(...args);`);
  const savedTask = await call('task_get',{task_id:task.id});
  for (const [status,code,confirmation] of [['interrupted','engine_interrupted','engine_interrupted'],['outcome_unknown','connection_lost','unconfirmed'],['canceling','task_canceled','unconfirmed']]) {
    const fixture = {...savedTask,state:status==='canceling'?'running':status,execution_status:status,cancellation_requested:status==='canceling'?'stop':null,termination:{code,source:'harness',observed_at:Date.now()-2000,interrupt_requested_at:Date.now()-1000,confirmed_at:confirmation==='unconfirmed'?null:Date.now(),confirmation,side_effects:'may_have_changes',actor:null},notification_blocked_reason:'discussion_expired'};
    await evaluate(`window.taskFixture=${JSON.stringify(fixture)}`);
    await click('#task-refresh');
    await wait(`document.querySelector('.work-state.${status}') !== null`);
    assert.match(await evaluate('document.querySelector(".work-detail").textContent'), /讨论授权已到期/);
    assert.match(await evaluate('document.querySelector(".task-termination").textContent'), /可能已有文件/);
    assert.match(await evaluate('document.querySelector(".task-timeline").textContent'), /向引擎请求停止/);
    for (const width of [1280,390,360]) {
      await cdp('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:width<761});
      assert.equal(await evaluate('document.documentElement.scrollWidth <= innerWidth'),true,'termination panel overflow '+width);
    }
  }
  await evaluate('api=window.actualTaskApi;window.taskFixture=null');
  await click('#task-refresh');
  await wait('document.querySelector(".work-state.canceled") !== null');
  await click('[data-task-action=report]');
  await evaluate('document.querySelector(".task-action-form textarea").value="任务已按用户要求取消，未启动模型。";document.querySelector(".task-action-form").requestSubmit();');
  await wait('document.querySelector(".task-report").textContent.includes("待发布")');
  await run(['schedule','run']);
  await click('#task-refresh');
  await wait('document.querySelector(".task-report").textContent.includes("已汇报")');

  await evaluate('state.page="observer"; state.viewer="lead"; setupReadOnly(); AgtTasks.open();');
  await wait('document.querySelector("#task-filter") !== null');
  await evaluate('document.querySelector("#task-filter").value="all";document.querySelector("#task-filter").dispatchEvent(new Event("change"));');
  await wait('document.querySelectorAll(".work-row").length === 1');
  await click('.work-row');
  await wait('document.querySelector(".work-detail") !== null');
  assert.equal(await evaluate('document.querySelectorAll("[data-task-action]").length'),0);
  assert.deepEqual(errors, []);
  console.log('Task panel browser passed: real ledger query, cancellation, observer read-only, responsive widths.');
} finally {
  socket?.close(); chrome?.kill('SIGTERM'); await run.close(); await server?.close();
  await new Promise(ok => setTimeout(ok, 500));
  await rm(home, { recursive: true, force: true }); await rm(profile, { recursive: true, force: true });
}
