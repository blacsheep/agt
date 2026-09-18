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
  assert.match(await evaluate('savedSummary.textContent'), /用户操作/);
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
    {kind:'framework',tool_name:'framework_harness_models',label:'查询可用模型',status:'running',started_at:Date.now()-2000,updated_at:Date.now()},
    {kind:'mcp',tool_name:'search',status:'running',started_at:Date.now()-1000,updated_at:Date.now()},
    {kind:'framework',tool_name:'framework_harness_session_configure',summary:'会话模型已更新',status:'completed',started_at:Date.now()-5000,updated_at:Date.now()-4000}
  ]; renderActivity();`);
  assert.match(await evaluate('document.querySelector(".live-phase").textContent'), /正在执行 2 项操作/);
  await click('.live-reply summary');
  assert.match(await evaluate('document.querySelector(".harness-activity").textContent'), /framework_harness_models/);
  assert.match(await evaluate('document.querySelector(".harness-activity").textContent'), /会话模型已更新/);
  assert.match(await evaluate('activityTimeline([{kind:"framework",status:"unknown",started_at:1,updated_at:2}])'), /状态待确认/);

  await evaluate('window.liveSummary = document.querySelector(".live-reply summary"); liveSummary.focus(); renderActivity();');
  assert.equal(await evaluate('document.activeElement === liveSummary && liveSummary.parentElement.open'), true);
  for (const width of [1280, 768, 390, 360]) {
    await cdp('Emulation.setDeviceMetricsOverride', {width, height:900, deviceScaleFactor:1, mobile:width<761});
    await evaluate('liveSummary.parentElement.open = true');
    assert.equal(await evaluate('document.documentElement.scrollWidth <= innerWidth'), true, `live overflow at ${width}`);
    assert.equal(await evaluate('document.querySelector("#live-replies").getBoundingClientRect().height <= 261'), true);
  }
  if (process.env.AGT_DESIGN_CAPTURE) {
    await cdp('Emulation.setDeviceMetricsOverride', {width:1280,height:900,deviceScaleFactor:1,mobile:false});
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
  assert.deepEqual(errors, []);
  console.log('Presence browser passed: 14 members, source separation, public reason escaping/clamp, stable expansion/focus, stale decisions, pause/offline, legacy records, conversation switch, 6 viewport sizes.');
} finally {
  socket?.close(); chrome?.kill('SIGTERM'); run.close(); server?.closeAllConnections(); server?.close();
  await new Promise(ok => setTimeout(ok, 500));
  await rm(home, { recursive: true, force: true }); await rm(profile, { recursive: true, force: true });
}