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
  for (const [name,args] of [
    ['resource_write',{kind:'provider',reference:'fixture',create:true,data:{base_url:'http://127.0.0.1:1/v1',api_key:'fixture'}}],
    ['resource_write',{kind:'model',reference:'fixture',create:true,data:{provider:'fixture',model:'fixture'}}],
    ['resource_write',{kind:'agent',reference:'tech-lead',create:true,data:{model:'fixture'}}],
    ['agent_bind',{principal:'tech-lead',agent:'tech-lead',policy:'manual',working_directory:home}],
  ]) await run(['tool','call','--principal','operator','--manager',name,'--args',JSON.stringify(args)]);
  ({ server } = createConsole({ run, home, principal: 'operator', token: 'browser-test-token' }));
  await new Promise(ok => server.listen(0, '127.0.0.1', ok));
  const url = `http://127.0.0.1:${server.address().port}`;
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

  await evaluate('state.polling=true;');
  await evaluate(`document.querySelector('#search').value='tech'; document.querySelector('#search').dispatchEvent(new Event('input'));`);
  assert.equal(await evaluate('document.querySelectorAll("[data-contact]").length'),1);
  await click('[data-contact="tech-lead"]');
  await wait('state.conversation?.kind === "direct" && state.members.length === 2');
  const direct = await evaluate('state.conversation.id');
  assert.equal(await evaluate('document.querySelector("#search").value'), '');
  await run(['im','conversation','hide',direct,'--actor','operator']);
  await evaluate('conversations()');
  await evaluate(`document.querySelector('#search').value='tech-lead'; document.querySelector('#search').dispatchEvent(new Event('input'));`);
  await click('[data-contact="tech-lead"]');
  await wait(`state.conversation?.id === ${JSON.stringify(direct)} && !state.sending`);
  assert.equal(await evaluate('state.conversations.filter(e=>e.conversation.kind === "direct").length'),1);
  // Archived private chats use the existing explicit restore flow.
  await run(['im','conversation','archive',direct,'--actor','operator']);
  await evaluate('conversations()');
  await evaluate(`document.querySelector('#search').value='tech'; document.querySelector('#search').dispatchEvent(new Event('input'));`);
  await click('[data-contact="tech-lead"]');
  await wait('document.querySelector("#modal").open');
  assert.match(await evaluate('document.querySelector("#modal-title").textContent'),/恢复/);
  await click('#modal-form button[type=submit]');
  await wait('!document.querySelector("#modal").open && state.conversation?.state === "active"');
  await evaluate(`document.querySelector('#search').value=''; renderConversations();`);
  const group = await run(['tool','call','--principal','operator','--manager','group_create','--args',JSON.stringify({title:'产品设计 · 协作讨论',members:['tech-lead','designer']})]);
  for (const [principal,body] of [['operator','我们把日常协作放在这里。搜索联系人可以直接打开私聊，会话操作也可以右键完成。'],['tech-lead','收到。我会先检查接口和边界，保留现有会话记录与权限。'],['designer','视觉使用浅灰和白色区分层级，蓝色标记选中状态与主要操作，让消息成为界面的重点。']]) {
    await run(['tool','call','--principal',principal,'message_send','--args',JSON.stringify({conversation:group.id,body,request_key:principal})]);
  }
  await evaluate(`conversations()`);
  await click(`[data-conv="${group.id}"]`);
  await wait('state.messages.length===3');
  await evaluate(`document.querySelector('[data-conv="${group.id}"]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:innerWidth-2,clientY:innerHeight-2}));`);
  assert.equal(await evaluate('document.querySelectorAll("#conversation-menu [role=menuitem]").length'),3);
  assert.equal(await evaluate('document.activeElement.dataset.action'), 'open');
  assert.equal(await evaluate('document.querySelector("#conversation-menu").getBoundingClientRect().right <= innerWidth && document.querySelector("#conversation-menu").getBoundingClientRect().bottom <= innerHeight'), true);
  await evaluate(`document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'End',bubbles:true}));`);
  assert.equal(await evaluate('document.activeElement.dataset.action'), 'hide');
  await evaluate('renderConversations()');
  await evaluate(`document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));`);
  assert.equal(await evaluate('document.querySelector("#conversation-menu").classList.contains("hidden")'),true);
  assert.equal(await evaluate('document.activeElement.dataset.conv'),group.id);
  await click(`[data-menu="${group.id}"]`);
  await click('#conversation-menu [data-action=hide]');
  assert.match(await evaluate('document.querySelector("#modal-body").textContent'),/不会退出群聊/);
  await evaluate('closeModal()');
  await click(`[data-menu="${group.id}"]`);
  await click('#conversation-menu [data-action=info]');
  await wait('document.querySelector("#group-drawer").open');
  await click('#close-group');
  // Contacts are unavailable in read-only observation; menus cannot mutate entries.
  await evaluate(`state.viewer='tech-lead'; state.page='observer'; document.querySelector('#search').value='designer'; renderConversations();`);
  assert.equal(await evaluate('document.querySelectorAll("[data-contact]").length'),0);
  await evaluate(`document.querySelector('#search').value=''; renderConversations();`);
  await click(`[data-menu="${group.id}"]`);
  assert.equal(await evaluate('document.querySelector("#conversation-menu [data-action=hide]")'),null);
  await evaluate(`closeConversationMenu(); state.viewer='operator'; state.page='chat'; renderConversations();`);
  for (const width of [1440,1280,1024,768,390,360]) {
    await cdp('Emulation.setDeviceMetricsOverride',{width,height:960,deviceScaleFactor:1,mobile:width<761});
    assert.equal(await evaluate('document.documentElement.scrollWidth <= innerWidth'),true,`overflow at ${width}`);
    await click(`[data-menu="${group.id}"]`);
    assert.equal(await evaluate('document.querySelector("#conversation-menu").getBoundingClientRect().right <= innerWidth'),true);
    await evaluate('closeConversationMenu()');
  }
  await cdp('Emulation.setDeviceMetricsOverride',{width:1440,height:960,deviceScaleFactor:1,mobile:false});
  await evaluate('scrollTo(0,0)');
  if (process.env.AGT_DESIGN_CAPTURE) {
    const shot=await cdp('Page.captureScreenshot',{format:'png'});
    await writeFile('/tmp/agt-collaboration-desktop.png',Buffer.from(shot.data,'base64'));
    await click(`[data-menu="${group.id}"]`);
    const menuShot=await cdp('Page.captureScreenshot',{format:'png'});
    await writeFile('/tmp/agt-collaboration-menu.png',Buffer.from(menuShot.data,'base64'));
  }
  await evaluate(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'k',ctrlKey:true,bubbles:true,cancelable:true}));`);
  assert.equal(await evaluate('document.querySelector("#palette").open && document.activeElement.id'),'palette-input');
  await evaluate(`const p=document.querySelector('#palette-input'); p.value='产品设计'; p.dispatchEvent(new Event('input'));`);
  assert.match(await evaluate('document.querySelector("#palette-list [aria-selected=true]").textContent'),/产品设计/);
  await evaluate(`document.querySelector('#palette-input').value='<img src=x onerror=window.paletteInjected=1>'; document.querySelector('#palette-input').dispatchEvent(new Event('input'));`);
  assert.equal(await evaluate('window.paletteInjected===undefined && !document.querySelector("#palette-list img")'),true);
  await evaluate(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'k',ctrlKey:true,bubbles:true,cancelable:true}));`);
  assert.equal(await evaluate('document.querySelector("#palette").open'),false);
  await evaluate('layoutPreferences.conversations=false; renderLayout(); document.querySelector("#search").focus();');
  await evaluate(`document.querySelector('#search').value='tech'; renderConversations();`);
  await click('#clear-search');
  assert.equal(await evaluate('document.querySelector("#search").value'),'');
  await evaluate(`document.querySelector('#message-body').value=('line'+String.fromCharCode(10)).repeat(40); document.querySelector('#message-body').dispatchEvent(new Event('input'));`);
  assert.equal(await evaluate('document.querySelector("#message-body").getBoundingClientRect().height <= 153'),true);
  await evaluate(`document.querySelector('#message-body').value=''; document.querySelector('#message-body').dispatchEvent(new Event('input'));`);
  assert.equal(await evaluate('document.querySelector("#message-body").getBoundingClientRect().height <= 70'),true);
  await evaluate(`window.realMessages=state.messages; state.messages=Array.from({length:40},(_,i)=>({...realMessages[0],id:'long-'+i,body:'阅读历史 '+i})); renderMessages(); document.querySelector('#messages').scrollTop=0; updateLatestMessage();`);
  assert.equal(await evaluate('document.querySelector("#latest-message").classList.contains("hidden")'),false);
  await click('#latest-message');
  await wait('document.querySelector("#latest-message").classList.contains("hidden")');
  await evaluate('state.messages=realMessages; renderMessages();');
  await cdp('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
  await evaluate(`selectConversation(${JSON.stringify(group.id)})`);
  assert.equal(await evaluate('getComputedStyle(document.querySelector("#conversation-pane")).display'),'none');
  assert.notEqual(await evaluate('getComputedStyle(document.querySelector("#back-to-conversations")).display'),'none');
  if (process.env.AGT_DESIGN_CAPTURE) {
    await evaluate('scrollTo(0,0)');
    const mobile=await cdp('Page.captureScreenshot',{format:'png'});
    await writeFile('/tmp/agt-collaboration-mobile.png',Buffer.from(mobile.data,'base64'));
  }
  await click('#back-to-conversations');
  assert.notEqual(await evaluate('getComputedStyle(document.querySelector("#conversation-pane")).display'),'none');
  assert.equal(await evaluate('document.documentElement.scrollWidth <= innerWidth'),true);
  await cdp('Emulation.setDeviceMetricsOverride',{width:1440,height:960,deviceScaleFactor:1,mobile:false});
  // Error paths keep the search in place, and user-controlled names stay text.
  await evaluate(`closeConversationMenu(); window.realApi=api; api=async()=>{throw new Error('fixture unavailable');}; state.boot.principals.push({id:'unsafe',name:'<img src=x onerror="window.injected=1">',kind:'user'}); document.querySelector('#search').value='unsafe'; renderConversations();`);
  assert.equal(await evaluate('document.querySelector(".contact-result img")'),null);
  await click('[data-contact=unsafe]');
  await wait('!state.sending');
  assert.match(await evaluate('document.querySelector("#notice").textContent'),/fixture unavailable/);
  assert.equal(await evaluate('document.querySelector("#search").value'),'unsafe');
  await evaluate('api=realApi');
  await evaluate(`(async()=>{
    state.viewer='tech-lead'; state.page='observer'; selectedObserver='tech-lead'; choosingIdentity=false;
    document.querySelector('#search').value=''; document.querySelector('#conversation-scope').value='recent';
    await conversations(); renderConversations();
  })()`);
  await click('#organize-entries');
  assert.equal(await evaluate('document.querySelectorAll("[data-entry]").length'),2);
  await click('#select-entries');
  await click('#hide-entries');
  assert.match(await evaluate('document.querySelector("#modal-body").textContent'),/tech-lead/);
  assert.match(await evaluate('document.querySelector("#modal-body").textContent'),/任务不会停止/);
  await click('#modal-form button[type=submit]');
  await wait('!state.modalBusy && !document.querySelector("#modal").open && state.conversations.length===0');
  await evaluate(`document.querySelector('#conversation-scope').value='hidden'; conversations();`);
  assert.equal(await evaluate('document.querySelectorAll("[data-entry]").length'),2);
  await click('#select-entries');
  await click('#restore-entries');
  await click('#modal-form button[type=submit]');
  await wait('!state.modalBusy && !document.querySelector("#modal").open && document.querySelectorAll("[data-entry]").length===0');
  await evaluate(`document.querySelector('#conversation-scope').value='recent'; conversations();`);
  assert.equal(await evaluate('document.querySelectorAll("[data-entry]").length'),2);
  const kept=await run(['tool','call','--principal','tech-lead','message_history','--args',JSON.stringify({conversation:group.id})]);
  assert.equal(kept.messages.length,3);
  await run(['tool','call','--principal','operator','--manager','agent_unbind','--args',JSON.stringify({principal:'tech-lead'})]);
  await run(['tool','call','--principal','operator','--manager','resource_delete','--args',JSON.stringify({kind:'agent',reference:'tech-lead',confirm:['agent/default/tech-lead']})]);
  await evaluate(`(async()=>{ state.page='chat'; state.viewer='operator'; choosingIdentity=false; await bootstrap(); await conversations(); await selectConversation('${group.id}'); })()`);
  assert.match(await evaluate('document.querySelector("#messages").textContent'),/tech-lead（已删除）/);
  await evaluate(`document.querySelector('#search').value='tech-lead'; renderConversations();`);
  assert.equal(await evaluate('document.querySelectorAll("[data-contact]").length'),0);
  await evaluate('showNewChat()');
  assert.equal(await evaluate('document.querySelector("[name=candidate][value=tech-lead]")'),null);
  await evaluate('closeModal()');
  assert.deepEqual(errors,[]);
  console.log('Collaboration browser passed: real direct create/reuse/hidden restore/archive restore, group menu, keyboard/focus, observer restrictions, escaped search, failure recovery, 6 widths.');
} finally {
  socket?.close(); chrome?.kill('SIGTERM'); run.close(); server?.closeAllConnections(); server?.close();
  await new Promise(ok => setTimeout(ok, 500));
  await rm(home, { recursive: true, force: true }); await rm(profile, { recursive: true, force: true });
}
