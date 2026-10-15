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

  await evaluate('window.noticeBefore = document.querySelector("#chat-head").getBoundingClientRect().top; notify("消息已发送，已安排 Agent 回复。");');
  assert.equal(await evaluate('document.querySelector("#chat-head").getBoundingClientRect().top'), await evaluate('noticeBefore'));
  await click('#notice .notice-close');
  assert.equal(await evaluate('document.querySelector("#notice").classList.contains("hidden")'), true);
  await evaluate('notify("自动收起测试");');
  await wait('document.querySelector("#notice").classList.contains("hidden")');
  await evaluate('notify("请检查配置",true); document.querySelector("#notice .notice-close").focus();');
  await new Promise(resolve=>setTimeout(resolve,5200));
  assert.equal(await evaluate('document.querySelector("#notice").classList.contains("hidden")'), false);
  await evaluate('document.querySelector("#notice .notice-close").dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true}));');
  assert.equal(await evaluate('document.querySelector("#notice").classList.contains("hidden")'), true);
  await evaluate('notify("悬停阅读"); document.querySelector("#notice .notice-close").focus();');
  assert.equal(await evaluate('document.activeElement === document.querySelector("#notice .notice-close")'),true, await evaluate('document.activeElement.outerHTML'));
  await new Promise(resolve=>setTimeout(resolve,5200));
  assert.equal(await evaluate('document.querySelector("#notice").classList.contains("hidden")'), false);
  await click('#notice .notice-close');
  await evaluate(`window.undone=false; showNotice('#deleted-notice','已移除个人会话入口，群聊与历史记录保留。',{action:dismiss=>{const b=document.createElement('button'); b.textContent='撤销移除'; b.onclick=()=>{window.undone=true;dismiss();};return b;}});`);
  for(const width of [1280,390,360]) {
    await cdp('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:width<761});
    assert.equal(await evaluate('document.documentElement.scrollWidth<=innerWidth'),true);
    const bounds=await evaluate('(()=>{const r=document.querySelector("#deleted-notice .notice-close").getBoundingClientRect();return {left:r.left,right:r.right};})()');
    assert.ok(bounds.left>=0 && bounds.right<=width);
  }
  await click('#deleted-notice .notice-content button');
  assert.equal(await evaluate('undone && document.querySelector("#deleted-notice").classList.contains("hidden")'),true);
  await evaluate(`showNotice('#deleted-notice','旧提示',{action:dismiss=>{window.oldDismiss=dismiss;return document.createElement('button');}}); showNotice('#deleted-notice','新提示'); oldDismiss();`);
  assert.equal(await evaluate('document.querySelector("#deleted-notice .notice-text").textContent'),'新提示');
  await click('#deleted-notice .notice-close');
  assert.deepEqual(errors, []);
  console.log('Notice browser passed: dismiss, expiry, focus pause, persistent errors, undo, replacement safety, desktop/mobile layout.');
} finally {
  socket?.close(); chrome?.kill('SIGTERM'); await run.close(); await server?.close();
  await new Promise(ok => setTimeout(ok, 500));
  await rm(home, { recursive: true, force: true }); await rm(profile, { recursive: true, force: true });
}
