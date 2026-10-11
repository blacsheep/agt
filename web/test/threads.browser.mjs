import { createConsole, cliRunner } from '../server.mjs';
import { fileURLToPath } from 'node:url';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
const home = await mkdtemp('/tmp/agt-threads-browser-');
const profile = await mkdtemp('/tmp/agt-threads-chrome-');
const run = cliRunner(fileURLToPath(new URL('../../target/debug/agt', import.meta.url)), home);
let server, chrome, socket;
try {
  await run(['init', home]);
  await run(['im', 'principal', 'create', 'operator']);
  await run(['im', 'principal', 'create', 'tech-lead', '--kind', 'agent']);
  await run(['im', 'principal', 'create', 'designer']);
  await run(['im', 'owner', 'ensure', 'operator']);
  for (const [name,args] of [
    ['resource_write',{kind:'provider',reference:'fixture',create:true,data:{base_url:'http://127.0.0.1:1/v1',api_key:'fixture'}}],
    ['resource_write',{kind:'model',reference:'fixture',create:true,data:{provider:'fixture',model:'fixture'}}],
    ['resource_write',{kind:'agent',reference:'tech-lead',create:true,data:{model:'fixture',description:'负责拆分任务'}}],
    ['agent_bind',{principal:'tech-lead',agent:'tech-lead',policy:'mentioned',working_directory:home}],
  ]) await run(['tool','call','--principal','operator','--manager',name,'--args',JSON.stringify(args)]);
  const shared = await run(['tool','call','--principal','operator','group_create','--args',JSON.stringify({title:'共同的群',members:['tech-lead']})]);
  const ask = await run(['tool','call','--principal','operator','message_send','--args',JSON.stringify({conversation:shared.id,body:'把 MCP 写好并挂上去',request_key:'ask-1'})]);
  // Final replies are linked to their Run (older replies through the run-result request key).
  await run(['tool','call','--principal','tech-lead','message_send','--args',JSON.stringify({conversation:shared.id,body:'已完成，三个 MCP 都通过了 probe',request_key:'run-result:reply-run'})]);
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
  const m1 = ask.message.id;
  await evaluate(`selectConversation(${JSON.stringify(shared.id)})`);
  await wait('document.querySelectorAll("#messages .message").length === 2');
  const tools = n => Array.from({length:n},(_,i)=>({id:'call:'+i,kind:i===3?'mcp':'framework',tool_name:i===3?'audit_query':'framework_resource_write',label:i===9?'<img src=x onerror=window.injected=1>':undefined,status:i===3?'failed':'completed',started_at:1000*i,updated_at:1000*i+400}));
  const runs = [
    {id:'reply-run',agent:'default/tech-lead',principal:'tech-lead',trigger_message:m1,state:'completed',silent:false,tool_failures:1,engine:'native',started_at:0,ended_at:134000,items:tools(10)},
    {id:'silent-run',agent:'default/tech-lead',principal:'tech-lead',trigger_message:m1,state:'completed',silent:true,tool_failures:0,engine:'native',started_at:1,ended_at:9000,items:tools(2)},
    {id:'failed-run',agent:'default/tech-lead',principal:'tech-lead',trigger_message:m1,state:'failed',silent:false,tool_failures:0,engine:'codex',started_at:2,ended_at:3000,items:[]},
    {id:'quiet-run',agent:'default/tech-lead',principal:'tech-lead',trigger_message:m1,state:'completed',silent:true,tool_failures:0,engine:'native',started_at:3,ended_at:4000,items:[]},
  ];
  await evaluate(`window.realApi = api; api = async (path, data) => path.startsWith('message-runs?') ? {runs:${JSON.stringify(runs)}} : path === 'tool-detail' ? {fields:{output:{text:'detail for '+data.activity,truncated:false}}} : realApi(path, data); AgtRunThreads.load()`);
  await wait('document.querySelectorAll(".run-thread").length === 3');
  // The reply carries its own trace; silent and failed runs hang under the message that triggered them.
  const reply = await evaluate(`document.querySelector('[data-thread-run="reply-run"]').closest('.message').querySelector('.bubble').textContent`);
  assert.match(reply, /三个 MCP/);
  assert.equal(await evaluate(`document.querySelector('[data-thread-anchor="${m1}"]').querySelectorAll('.run-thread').length`), 2);
  assert.match(await evaluate(`document.querySelector('[data-thread-run="reply-run"] summary').textContent`), /10 次工具调用 · 1 项失败.*2 分 14 秒/);
  assert.match(await evaluate(`document.querySelector('[data-thread-run="silent-run"] summary').textContent`), /处理了这条消息，没有发言 · 2 次工具调用/);
  assert.match(await evaluate(`document.querySelector('[data-thread-run="failed-run"] summary').textContent`), /处理失败/);
  assert.equal(await evaluate(`!!document.querySelector('[data-thread-run="quiet-run"]')`), false);
  assert.equal(await evaluate(`document.querySelector('[data-thread-run="reply-run"]').open`), false);
  // Inline: the latest eight entries; the expansion survives a transcript re-render.
  await click('[data-thread-run="reply-run"] > summary');
  assert.equal(await evaluate(`document.querySelectorAll('[data-thread-run="reply-run"] .run-thread-list li[data-status]').length`), 8);
  assert.match(await evaluate(`document.querySelector('[data-thread-run="reply-run"] .run-thread-skip').textContent`), /更早的 2 项/);
  await evaluate('renderMessages()');
  assert.equal(await evaluate(`document.querySelector('[data-thread-run="reply-run"]').open`), true);
  assert.equal(await evaluate('window.injected === undefined && !document.querySelector("#messages img")'), true);
  await cdp('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
  if (process.env.AGT_DESIGN_CAPTURE) { const shot = await cdp('Page.captureScreenshot', { format: 'png' }); await writeFile(process.env.AGT_DESIGN_CAPTURE + '/threads.png', Buffer.from(shot.data, 'base64')); }
  // The full timeline opens in the side panel with lazily loaded, scoped tool details.
  await click('[data-thread-run="reply-run"] [data-run-panel]');
  await wait('!document.querySelector("#command-panel").classList.contains("hidden")');
  assert.equal(await evaluate('document.querySelector("#command-panel").dataset.mode'), 'run');
  assert.equal(await evaluate('document.querySelector("#command-panel-kind").textContent'), '执行记录');
  assert.equal(await evaluate('document.querySelectorAll("#command-panel-nav button").length'), 0);
  assert.equal(await evaluate('document.querySelectorAll("#run-panel-tools .tool-card").length'), 10);
  await evaluate('document.querySelector(\'#run-panel-tools [data-activity="call:3"]\').open = true');
  await wait(`document.querySelector('#run-panel-tools [data-activity="call:3"] .tool-payload pre')?.textContent === 'detail for call:3'`);
  if (process.env.AGT_DESIGN_CAPTURE) { const shot = await cdp('Page.captureScreenshot', { format: 'png' }); await writeFile(process.env.AGT_DESIGN_CAPTURE + '/threads-panel.png', Buffer.from(shot.data, 'base64')); }
  await click('#run-panel-locate');
  assert.equal(await evaluate(`document.querySelector('[data-thread-run="reply-run"]').closest('.message').classList.contains('flash')`), true);
  await click('#close-command-panel');
  assert.equal(await evaluate('document.activeElement === document.querySelector(\'[data-thread-run="reply-run"] > summary\')'), true);
  // Above the composer only live work remains, one collapsed line per Agent; nothing to dismiss.
  await evaluate(`state.scheduler.online = true; state.runs = [{id:'reply-run',agent:'default/tech-lead',conversation:${JSON.stringify(shared.id)},state:'completed',tool_failures:0,progress:null,tool_activity:{principal:'tech-lead',items:tools(3),approvals:[]}}]; renderLiveReplies();`.replace('tools(3)', JSON.stringify(tools(3))));
  assert.equal(await evaluate('document.querySelectorAll(".live-reply").length'), 0);
  await evaluate(`state.runs[0].state = 'running'; state.runs[0].progress = {phase:'tool_running',started_at:Date.now(),preview:''}; renderLiveReplies();`);
  assert.equal(await evaluate('document.querySelectorAll(".live-reply").length'), 1);
  assert.equal(await evaluate('document.querySelector(".live-reply").open'), false);
  assert.match(await evaluate('document.querySelector(".live-phase").textContent'), /3 项工具/);
  // Phone width: threads stay inside the transcript.
  await cdp('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });
  assert.equal(await evaluate('[...document.querySelectorAll(".run-thread")].every(t => t.getBoundingClientRect().right <= innerWidth + 0.5)'), true);
  assert.deepEqual(errors, []);
  console.log('Run threads browser passed: reply/trigger anchors, inline list, persistence, side panel details, focus, live bar, mobile.');
} finally {
  socket?.close(); chrome?.kill('SIGTERM'); run.close(); server?.closeAllConnections(); server?.close();
  await new Promise(ok => setTimeout(ok, 500));
  await rm(home, { recursive: true, force: true }); await rm(profile, { recursive: true, force: true });
}
