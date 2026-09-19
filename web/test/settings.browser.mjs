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

  await click('[data-page="settings"]');
  await wait('!!document.querySelector("#runtime-config-form")');
  assert.match(await evaluate('document.querySelector("#runtime-settings").textContent'), /工作目录不是沙箱/);
  assert.equal(await evaluate('document.querySelectorAll("[data-setting]").length'),9);
  if(process.env.AGT_DESIGN_CAPTURE) {
    const shot=await cdp('Page.captureScreenshot',{format:'png'});
    await writeFile(process.env.AGT_DESIGN_CAPTURE,Buffer.from(shot.data,'base64'));
  }
  for(const width of [1280,768,390,360]) {
    await cdp('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:width<761});
    assert.equal(await evaluate('document.documentElement.scrollWidth<=innerWidth'),true,`settings overflow at ${width}`);
  }
  await evaluate(`document.querySelector('[data-unlimited="scheduler.replies_per_agent"]').click();document.querySelector('#runtime-config-form').requestSubmit();`);
  await wait('document.querySelector("#settings-feedback").textContent.startsWith("已保存")');
  assert.equal((await run(['config','show'])).scheduler.replies_per_agent,null);
  await evaluate(`document.querySelector('[data-setting="scheduler.concurrency"]').value='7';document.querySelector('[data-setting="scheduler.concurrency"]').dispatchEvent(new Event('input',{bubbles:true}));`);
  await click('#reload-settings');
  assert.equal(await evaluate('document.querySelector("#modal").open'),true);
  await click('#confirm-settings-reload');
  await wait(`document.querySelector('[data-setting="scheduler.concurrency"]')?.value === "3"`);
  await evaluate(`document.querySelector('#runtime-config-form').insertAdjacentHTML('beforeend', '<div id="builtin-preview">'+builtinFields(['task_collaboration','local_execution'])+'</div>')`);
  assert.equal(await evaluate('document.querySelectorAll("#builtin-preview input:checked").length'),2);
  assert.deepEqual(errors, []);
  console.log('Settings browser passed: backend descriptions, unlimited save, reload confirmation, capability controls, responsive widths.');
} finally {
  socket?.close(); chrome?.kill('SIGTERM'); await run.close(); server?.closeAllConnections(); server?.close();
  await new Promise(ok => setTimeout(ok, 500));
  await rm(home, { recursive: true, force: true }); await rm(profile, { recursive: true, force: true });
}
