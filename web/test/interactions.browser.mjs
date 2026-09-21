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
  await wait('document.querySelectorAll(".ix-row").length === 2');
  assert.equal(await evaluate('window.injected'), undefined);
  await click('[data-ix-message="team-1"]');
  await wait('document.querySelector("#ix-execution-body").textContent.includes("等待测试结果")');
  assert.equal(await evaluate('document.querySelector("#ix-execution-body").textContent.includes("secret")'),false);
  assert.equal(await evaluate('document.querySelector(".ix-message details").open'),false);
  await click('.ix-message summary');
  assert.equal(await evaluate('document.querySelector(".ix-message details").open'),true);
  await evaluate('AgtInteractions.refresh()');
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
  assert.deepEqual(errors,[]);
  console.log('Interaction browser passed: scoped reads, filters, pagination, disclosure preservation, XSS, identity race and five viewport widths.');
} finally {
  socket?.close(); chrome?.kill('SIGTERM'); run.close(); server?.closeAllConnections(); server?.close();
  await new Promise(ok => setTimeout(ok, 500));
  await rm(home, { recursive: true, force: true }); await rm(profile, { recursive: true, force: true });
}
