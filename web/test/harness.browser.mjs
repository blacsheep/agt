import { createConsole, cliRunner } from '../server.mjs';
import { fileURLToPath } from 'node:url';
import { mkdtemp, rm, writeFile, readFile, chmod } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
const home = await mkdtemp('/tmp/agt-resource-browser-');
const profile = await mkdtemp('/tmp/agt-resource-chrome-');
const run = cliRunner(fileURLToPath(new URL('../../target/debug/agt', import.meta.url)), home);
let server, chrome, socket;
try {
  await run(['init', home]);
  const executable = `${home}/codex-fixture`;
  await writeFile(executable, await readFile(new URL('../../crates/agt-harness/tests/fixtures/codex.py', import.meta.url)));
  await chmod(executable, 0o700);
  await run(['im', 'principal', 'create', 'operator']);
  await run(['im', 'principal', 'create', 'tech-lead', '--kind', 'agent']);
  await run(['im', 'principal', 'create', 'designer']);
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

  await evaluate('state.polling=true; switchPage("resources"); state.resourceKind="environment"; showResource();');
  await evaluate(`document.querySelector('[name=name]').value='local'; document.querySelector('[name=executable]').value=${JSON.stringify(executable)}; document.querySelector('[name=working_directory]').value=${JSON.stringify(home)};`);
  for (const width of [1280,390]) {
    await cdp('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:width<761});
    assert.equal(await evaluate('document.documentElement.scrollWidth<=innerWidth'),true);
    assert.equal(await evaluate('document.querySelector("#modal").scrollWidth<=document.querySelector("#modal").clientWidth+1'),true);
    if(process.env.AGT_DESIGN_CAPTURE){const shot=await cdp('Page.captureScreenshot',{format:'png'});await writeFile(`/tmp/agt-harness-environment-${width}.png`,Buffer.from(shot.data,'base64'));}
  }
  await click('#modal-form button[type=submit]');
  await wait('!state.modalBusy && !document.querySelector("#modal").open && items("environment").length===1');
  await evaluate('showAgent()');
  await evaluate(`document.querySelector('[name=name]').value='external'; document.querySelector('[name=prompt]').value='You review code.';document.querySelector('[name=execution]').value='external';document.querySelector('[name=execution]').dispatchEvent(new Event('change'));document.querySelector('[name=environment]').value='default/local';document.querySelector('[name=environment]').dispatchEvent(new Event('change'));`);
  assert.equal(await evaluate('document.querySelector("[name=model]").disabled'),true);
  assert.equal(await evaluate('document.querySelector("[name=environment]").required'),true);
  assert.equal(await evaluate('document.querySelector("[name=cwd]").readOnly'),true);
  assert.equal(await evaluate('document.querySelector("[name=allow_mcp]").disabled'),true);
  await click('#modal-form button[type=submit]');
  await wait('!state.modalBusy && !document.querySelector("#modal").open && items("agent").length===1');
  assert.equal(await evaluate('items("agent")[0].model'),null);
  assert.equal(await evaluate('state.boot.bindings.find(b=>b.agent==="default/external").enabled'),true);
  await evaluate('showAgentEdit("default/external")');
  await wait('document.querySelector("#modal [name=environment]")?.value==="default/local"');
  await evaluate('document.querySelector("[name=description]").value="Review specialist"');
  await click('#modal-form button[type=submit]');
  await wait('!state.modalBusy && !document.querySelector("#modal").open');
  assert.equal(await evaluate('items("agent")[0].description'),'Review specialist');
  await evaluate('showEnvironmentEdit("default/local")');
  await wait('document.querySelector("[name=timeout_seconds]")');
  await evaluate('document.querySelector("[name=timeout_seconds]").value="120"');
  await click('#modal-form button[type=submit]');
  await wait('!state.modalBusy && !document.querySelector("#modal").open');
  assert.equal(await evaluate('items("environment")[0].timeout_seconds'),120);
  const chat=await run(['tool','call','--principal','operator','--manager','direct_create','--args',JSON.stringify({principal:'external'})]);
  await evaluate('switchPage("chat");');
  await evaluate('conversations()');
  await evaluate(`selectConversation(${JSON.stringify(chat.id)})`);
  await wait('state.conversation?.id=== '+JSON.stringify(chat.id));
  await evaluate('document.querySelector("#auto-reply").checked=false;');
  for(const text of ['/model','/status','/compact','/review','/permissions','/model future','/']) {
    await evaluate(`document.querySelector('#message-body').value=${JSON.stringify(text)}; document.querySelector('#composer').requestSubmit();`);
    await wait('document.querySelector("#modal").open && !document.querySelector("#modal-body").textContent.includes("正在读取")');
    assert.equal(await evaluate('state.messages.length'),0);
    await evaluate('closeModal()');
  }
  for(const width of [1280,390]) {
    await cdp('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:width<761});
    await evaluate('showChatCommand({name:"help",args:""})');
    assert.equal(await evaluate('document.querySelector("#modal").scrollWidth<=document.querySelector("#modal").clientWidth+1'),true);
    if(process.env.AGT_DESIGN_CAPTURE){const shot=await cdp('Page.captureScreenshot',{format:'png'});await writeFile(`/tmp/agt-chat-commands-${width}.png`,Buffer.from(shot.data,'base64'));}
    await evaluate('closeModal()');
  }
  await evaluate(`document.querySelector('#message-body').value='//model'; document.querySelector('#composer').requestSubmit();`);
  await wait('state.messages.length===1 && !state.sending');
  assert.equal(await evaluate('state.messages[0].body'),'/model');
  await evaluate(`document.querySelector('#message-body').value='/clear'; document.querySelector('#composer').requestSubmit();`);
  await wait('document.querySelector("#modal").open');
  assert.match(await evaluate('document.querySelector("#modal-title").textContent'),/上下文/);
  await evaluate('closeModal()');
  assert.deepEqual(errors,[]);
  console.log('Harness browser passed: create/edit environment and external Agent without Model, enabled binding, execution controls, desktop/mobile overflow.');
} finally {
  socket?.close(); chrome?.kill('SIGTERM'); run.close(); server?.closeAllConnections(); server?.close();
  await new Promise(ok=>setTimeout(ok,500));
  await rm(home,{recursive:true,force:true}); await rm(profile,{recursive:true,force:true});
}
