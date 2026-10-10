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
  // Owners are added to Agent-created groups, so a direct chat is the conversation the operator cannot see.
  const hidden = await run(['tool','call','--principal','tech-lead','direct_create','--args',JSON.stringify({principal:'designer'})]);
  await run(['tool','call','--principal','tech-lead','message_send','--args',JSON.stringify({conversation:hidden.id,body:'<img src=x onerror=window.injected=1>私下讨论',request_key:'hidden-1'})]);
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
  await click('[data-page=agents]');
  await wait('document.querySelector("[data-agent-detail]")');
  const reference = await evaluate('document.querySelector("[data-agent-detail]").dataset.agentDetail');
  await click(`[data-agent-detail="${reference}"]`);
  await wait('document.querySelector("#agent-detail").open');
  assert.equal(await evaluate('document.querySelector("#agent-detail-title").textContent'), 'tech-lead');
  assert.match(await evaluate('document.querySelector(".agent-hero").textContent'), /负责拆分任务/);
  // Conversations come from the Agent's perspective; the operator's own ones open in chat, others read-only.
  await click('[data-detail-tab=chats]');
  await wait('document.querySelectorAll("[data-detail-chat]").length === 2');
  assert.equal(await evaluate(`document.querySelector('[data-detail-chat="${shared.id}"]').dataset.shared`), 'true');
  assert.equal(await evaluate(`document.querySelector('[data-detail-chat="${hidden.id}"]').dataset.shared`), 'false');
  assert.equal(await evaluate('window.injected === undefined && !document.querySelector("#agent-detail img")'), true);
  // Arrow keys move between tabs and keep focus on the tab list.
  await evaluate('document.querySelector("[data-detail-tab=chats]").focus(); document.querySelector("[role=tablist]").dispatchEvent(new KeyboardEvent("keydown",{key:"ArrowRight",bubbles:true}))');
  assert.equal(await evaluate('document.activeElement.dataset.detailTab'), 'runs');
  // Live status refreshes in place without losing the open tab.
  await evaluate(`state.scheduler = {online:true,discussions:[],tasks:[{id:'t',principal:'tech-lead',conversation:${JSON.stringify(shared.id)},discussion:'d',state:'responding'}],usage:[]};
    state.runs = [{id:'r1',agent:${JSON.stringify(reference)},conversation:${JSON.stringify(shared.id)},state:'completed',model_steps:3,tool_failures:0,started_at:Date.now()-60000,ended_at:Date.now()-30000}]; renderActivity();`);
  assert.equal(await evaluate('document.querySelector("#agent-detail").classList.contains("working")'), true);
  assert.equal(await evaluate('document.querySelector("[data-detail-tab=runs]").getAttribute("aria-selected")'), 'true');
  assert.equal(await evaluate('document.querySelectorAll(".agent-runs [data-detail-run]").length'), 1);
  assert.equal(await evaluate('document.activeElement.dataset.detailTab'), 'runs');
  await click('[data-detail-tab=overview]');
  assert.match(await evaluate('document.querySelector(".agent-now").textContent'), /正在回复.*共同的群/);
  // Reduced motion removes the slide-in, so geometry is measured at rest.
  await cdp('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
  assert.equal(await evaluate('document.querySelector("#agent-detail").getAnimations({subtree:true}).filter(a => a.playState === "running").length'), 0);
  for (const width of [1280, 390]) {
    await cdp('Emulation.setDeviceMetricsOverride', { width, height: 900, deviceScaleFactor: 1, mobile: width < 761 });
    assert.equal(await evaluate('document.querySelector("#agent-detail").getBoundingClientRect().right <= innerWidth + 0.5'), true, `sheet overflow at ${width}`);
  }
  await cdp('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
  // A conversation the operator is not in opens read-only from the Agent's perspective.
  await click('[data-detail-tab=chats]');
  await click(`[data-detail-chat="${hidden.id}"]`);
  await wait(`state.page === 'observer' && state.viewer === 'tech-lead' && state.conversation?.id === ${JSON.stringify(hidden.id)}`);
  assert.equal(await evaluate('document.querySelector("#agent-detail").open'), false);
  // The command palette reaches the same drawer.
  await evaluate('AgtPalette.open(); const p = document.querySelector("#palette-input"); p.value = "tech-lead 的详情"; p.dispatchEvent(new Event("input"));');
  await evaluate('document.querySelector("#palette-input").dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true}))');
  await wait('state.page === "agents" && document.querySelector("#agent-detail").open');
  await click('[data-detail-close]');
  assert.equal(await evaluate('document.querySelector("#agent-detail").open'), false);
  assert.deepEqual(errors, []);
  console.log('Agent detail browser passed: profile, perspective conversations, tabs, live refresh, read-only routing, palette, mobile.');
} finally {
  socket?.close(); chrome?.kill('SIGTERM'); run.close(); server?.closeAllConnections(); server?.close();
  await new Promise(ok => setTimeout(ok, 500));
  await rm(home, { recursive: true, force: true }); await rm(profile, { recursive: true, force: true });
}
