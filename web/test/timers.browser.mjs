import { cliRunner, startConsole } from './support/console.mjs';
import { fileURLToPath } from 'node:url';
import { mkdtemp, rm } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
const home = await mkdtemp('/tmp/agt-timers-browser-');
const profile = await mkdtemp('/tmp/agt-timers-chrome-');
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
  const chat = await call('group_create',{title:'data',members:['lead']});
  await call('timer_create',{conversation:chat.id,text:'每月发数据',name:'月度数据',cron:'0 9 1 * *',mentions:['lead']});
  server = await startConsole({ run, principal: 'operator', token: 'browser-test-token' });
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
  await cdp('Page.navigate', { url: `${server.url}/#browser-test-token` });
  const evaluate = async expression => { const value = await cdp('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }); if (value.exceptionDetails) throw new Error(JSON.stringify(value.exceptionDetails)); return value.result.value; };
  const wait = async expression => { for (let i = 0; i < 150; i++) { if (await evaluate(expression)) return; await new Promise(ok => setTimeout(ok, 100)); } throw new Error(`Timed out: ${expression}`); };
  const click = selector => evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);
  await wait('document.querySelector("#connection")?.textContent === "本地已连接"');
  await click(`[data-conv="${chat.id}"]`);
  await wait(`state.conversation?.id === ${JSON.stringify(chat.id)} && state.members.length === 2`);

  // `/timers` lists the existing timer.
  await evaluate('showChatCommand({name:"timers",args:""})');
  await wait('document.querySelectorAll("#timer-list .work-row").length === 1');
  assert.match(await evaluate('document.querySelector("#timer-list").textContent'), /月度数据/);
  assert.match(await evaluate('document.querySelector("#timer-list").textContent'), /0 9 1 \* \*/);
  assert.match(await evaluate('document.querySelector("#timer-count").textContent'), /1 个等待触发/);

  // Create a one-off timer from the form, in the browser's local time.
  await evaluate(`(() => {
    document.querySelector('.timer-create').open = true;
    const form = document.querySelector('#timer-form');
    form.text.value = '提醒：交周报';
    const at = new Date(Date.now() + 3600_000);
    const pad = n => String(n).padStart(2, '0');
    form.at.value = at.getFullYear()+'-'+pad(at.getMonth()+1)+'-'+pad(at.getDate())+'T'+pad(at.getHours())+':'+pad(at.getMinutes());
    form.querySelector('[name=mentions][value=lead]').checked = true;
    form.minutes.value = '90';
    form.requestSubmit();
  })()`);
  await wait('document.querySelectorAll("#timer-list .work-row").length === 2');
  const created = (await call('timer_list',{conversation:chat.id})).timers.find(t => t.text === '提醒：交周报');
  assert.ok(created, 'the form created a timer');
  assert.deepEqual(created.mentions, ['lead']);
  assert.equal(created.discussion_seconds, 5400);
  assert.ok(created.next_in_seconds > 3000 && created.next_in_seconds <= 3600, String(created.next_in_seconds));

  for (const width of [1280,768,390,360]) {
    await cdp('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:width<761});
    assert.equal(await evaluate('document.documentElement.scrollWidth <= innerWidth'),true,'timer panel overflow '+width);
  }
  await cdp('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });

  // Cancel it again.
  await click(`[data-timer-cancel="${created.id}"]`);
  await wait('document.querySelectorAll("#timer-list .work-state.canceled").length === 1');
  assert.equal((await call('timer_list',{conversation:chat.id})).timers.length, 1);
  assert.deepEqual(errors, []);
  console.log('Timers browser passed: list, create from the form, cancel, responsive widths.');
} finally {
  socket?.close(); chrome?.kill('SIGTERM'); await run.close(); await server?.close();
  await new Promise(ok => setTimeout(ok, 500));
  await rm(home, { recursive: true, force: true }); await rm(profile, { recursive: true, force: true });
}
