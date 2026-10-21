import { cliRunner, startConsole } from './support/console.mjs';
import { fileURLToPath } from 'node:url';
import { mkdtemp, rm } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { join } from 'node:path';
import assert from 'node:assert/strict';
const home = await mkdtemp('/tmp/agt-update-browser-');
const profile = await mkdtemp('/tmp/agt-update-chrome-');
const run = cliRunner(fileURLToPath(new URL('../../target/debug/agt', import.meta.url)), home);
// Stands in for GitHub: the latest release redirects to its tag page.
const github = createServer((req, res) => {
  if (req.url === '/releases/latest') res.writeHead(302, { Location: `http://127.0.0.1:${github.address().port}/releases/tag/v99.0.0` });
  else res.writeHead(404);
  res.end();
});
await new Promise(ok => github.listen(0, '127.0.0.1', ok));
let server, chrome, socket;
try {
  await run(['init', home]);
  await run(['im', 'principal', 'create', 'operator']);
  server = await startConsole({ run, principal: 'operator', token: 'browser-test-token', env: {
    AGT_NO_UPDATE_CHECK: '', AGT_UPDATE_URL: `http://127.0.0.1:${github.address().port}/releases/latest`,
    AGT_UPDATE_CACHE: join(home, 'update-check.json'), NO_PROXY: '127.0.0.1', no_proxy: '127.0.0.1',
  } });
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

  // The newer release shows as a button at the bottom of the rail; nothing else happens.
  await wait('!document.querySelector("#update-button").classList.contains("hidden")');
  assert.equal(await evaluate('document.querySelector("#update-label").textContent'), '新版本 99.0.0');
  assert.equal(await evaluate('document.querySelector("#modal").open'), false);
  // Clicking it explains how to upgrade the CLI; `agt web` cannot replace itself.
  await click('#update-button');
  await wait('document.querySelector("#modal").open');
  assert.equal(await evaluate('document.querySelector("#modal-title").textContent'), '有新版本');
  assert.equal(await evaluate('document.querySelector("#modal-body .update-command").textContent'), 'agt upgrade');
  assert.match(await evaluate('document.querySelector("#modal-body a").href'), /\/releases\/tag\/v99\.0\.0$/);
  await click('#update-later');
  await wait('!document.querySelector("#modal").open');
  // An icon with the rail collapsed, labelled with the rail expanded.
  for (const collapsed of [true, false]) {
    if (await evaluate('document.body.classList.contains("nav-collapsed")') !== collapsed) await click('#toggle-nav');
    assert.equal(await evaluate('document.querySelector("#update-button").getBoundingClientRect().width > 0'), true);
    assert.equal(await evaluate('getComputedStyle(document.querySelector("#update-label")).display') === 'none', collapsed);
  }
  assert.deepEqual(errors, []);
  console.log('Update browser passed: new-version button, CLI upgrade dialog, collapsed rail.');
} finally {
  socket?.close(); chrome?.kill('SIGTERM'); await run.close(); await server?.close(); github.close();
  await new Promise(ok => setTimeout(ok, 500));
  await rm(home, { recursive: true, force: true }); await rm(profile, { recursive: true, force: true });
}
