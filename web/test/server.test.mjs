import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import http from 'node:http';
import { createConsole, cliRunner } from '../server.mjs';

async function fixture(t, run) {
  const { server, token } = createConsole({ run, home: '/isolated', principal: 'operator' });
  await new Promise(ok => server.listen(0, '127.0.0.1', ok));
  t.after(() => { server.closeAllConnections(); server.close(); });
  const url = `http://127.0.0.1:${server.address().port}`;
  const request = (path, data, headers = {}) => fetch(`${url}${path}`, { method: data === undefined ? 'GET' : 'POST', headers: { 'X-AGT-Token': token, ...(data === undefined ? {} : { 'Content-Type': 'application/json' }), ...headers }, body: data === undefined ? undefined : JSON.stringify(data) });
  return { request, url };
}

test('loopback API requires token and matching origin, does not echo input errors', async t => {
  const seen = [];
  const { request, url } = await fixture(t, async args => { seen.push(args); return []; });
  assert.equal((await fetch(`${url}/api/bootstrap`)).status, 401);
  assert.equal((await request('/api/bootstrap', undefined, { Origin: 'https://evil.example' })).status, 403);
  const deniedHost = await new Promise(ok => http.get(`${url}/api/bootstrap`, { headers: { Host: 'evil.example' } }, res => { res.resume(); ok(res.statusCode); }));
  assert.equal(deniedHost, 403);
  assert.equal((await request('/api/tool', { name: 'resource_delete', args: {} })).status, 403);
  assert.equal((await request('/api/tool', { name: 'message_send', args: {}, viewer: 'victim' })).status, 403);
  assert.equal((await request('/api/read', { name: 'message_send', args: {} })).status, 403);
  assert.equal(seen.length, 0);
  assert.equal((await request('/api/tool', { name: 'group_create', args: { title: '$(touch /tmp/not-executed)', members: [] } })).status, 200);
  assert.deepEqual(seen[0].slice(0, 6), ['tool', 'call', '--principal', 'operator', '--manager', 'group_create']);
  assert.ok(seen[0].at(-1).includes('$(touch'));
  const page = await request('/');
  assert.match(page.headers.get('content-security-policy'), /frame-ancestors 'none'/);
  assert.doesNotMatch(await page.text(), /[a-f0-9]{64}/);
});

test('malformed inputs and internal failures do not expose host details', async t => {
  const { request } = await fixture(t, async () => { throw new Error('private-host-secret'); });
  assert.equal((await request('/api/tool', [])).status, 400);
  assert.equal((await request('/api/tool', { name: 'group_create', args: [] })).status, 400);
  assert.equal((await request('/api/calls', { id: 'run', after: -1 })).status, 400);
  const failed = await request('/api/bootstrap');
  assert.equal(failed.status, 500);
  assert.doesNotMatch(await failed.text(), /private-host-secret/);
});

async function settled(request, message) {
  for (let i = 0; i < 2000; i++) {
    const value = await (await request('/api/activity')).json();
    if (value.scheduler.tasks.length && (!message || value.scheduler.tasks.some(t => t.triggers?.includes(message))) && value.scheduler.tasks.every(t => !['queued', 'evaluating', 'ready_to_reply', 'responding'].includes(t.state))) return value;
    await new Promise(ok => setTimeout(ok, 20));
  }
  assert.fail('scheduler did not settle');
}

test('automatic reply authorizes exact message, including idempotent retries', async t => {
  const seen = []; let sends = 0;
  const { request } = await fixture(t, async args => {
    seen.push(args);
    if (args.includes('message_send')) return { duplicate: sends++ > 0, message: { id: 'message-one' } };
    return { online: true, discussions: [], tasks: [] };
  });
  const send = auto_reply => request('/api/tool', { name: 'message_send', args: { conversation: 'one', body: 'hello', request_key: 'stable' }, auto_reply });
  assert.equal((await send(false)).status, 200);
  assert.equal(seen.filter(a => a[0] === 'schedule').length, 0);
  assert.equal((await send(true)).status, 200);
  assert.equal((await send(true)).status, 200);
  assert.deepEqual(seen.filter(a => a[0] === 'schedule'), Array(2).fill(['schedule', 'start', 'one', '--message', 'message-one']));
});

test('discussion controls require membership and paid confirmation; mounted MCPs need no separate opt-in', async t => {
  const seen = [];
  const { request } = await fixture(t, async args => { seen.push(args); return args.includes('member_list') ? [{principal_id:'operator',left_message_seq:null}] : []; });
  assert.equal((await request('/api/reply', { action: 'start', conversation: 'one' })).status, 400);
  for (const action of ['start', 'resume', 'pause', 'end']) {
    assert.equal((await request('/api/reply', { action, conversation: 'one', confirm: true })).status, 202);
  }
  assert.deepEqual(seen.filter(a => a[0] === 'schedule'), [
    ['schedule', 'start', 'one'], ['schedule', 'resume', 'one'], ['schedule', 'pause', 'one'], ['schedule', 'end', 'one'],
  ]);
  assert.equal(seen.filter(a => a.includes('member_list')).length, 5);
});

test('activity represents simultaneous conversations and redacts run content', async t => {
  const scheduler = { online: true, discussions: [], tasks: [{ principal: 'a', conversation: 'one', state: 'responding', last_decision: { action: 'participate', reason_code: 'relevant_expertise', public_reason: '可以补充技术信息', decided_at: 1234, through_seq: 2 } }, { principal: 'b', conversation: 'two', state: 'responding' }] };
  const { request } = await fixture(t, async args => args[0] === 'schedule' ? scheduler : [{ id: 'run', request: { agent: 'a', input: 'private-input' }, output: 'private-output', error: 'private-error', state: 'failed', model_steps: 1, tool_failures: 0 }]);
  const activity = await (await request('/api/activity')).json();
  assert.deepEqual(activity.scheduler, scheduler);
  assert.doesNotMatch(JSON.stringify(activity), /private-input|private-output|private-error/);
});

test('deletion uses runtime-fenced operation and fixed operator identity', async t => {
  const seen = [];
  const { request } = await fixture(t, async args => { seen.push(args); return []; });
  assert.equal((await request('/api/conversation', { action: 'delete', conversation: 'one', confirm: true, actor: 'victim' })).status, 403);
  assert.equal((await request('/api/conversation', {action:'delete',conversation:'one',confirm:true})).status,200);
  assert.deepEqual(seen.at(-1), ['schedule', 'delete', 'one', '--actor', 'operator']);
});

test('real CLI: resources, binding, chat, observation, safe errors, persistent scheduler', async t => {
  const home = await mkdtemp(resolve(tmpdir(), 'agt-web-test-'));
  const binary = resolve(import.meta.dirname, '../../target/debug/agt');
  const run = cliRunner(binary, home);
  t.after(async () => { await run.close(); await rm(home, { recursive: true, force: true }); });
  await run(['init', home]);
  await run(['im', 'principal', 'create', 'operator']);
  await run(['provider', 'create', 'fixture', '--base-url', 'http://127.0.0.1:1/v1', '--api-key', 'fixture-secret']);
  const { request } = await fixture(t, run);
  const call = async (name, args) => {
    const response = await request('/api/tool', { name, args });
    const body = await response.json(); assert.equal(response.status, 200, JSON.stringify(body)); return body;
  };
  await call('resource_write', { kind: 'model', reference: 'test', create: true, data: { provider: 'fixture', model: 'test', params: { api_format: 'chat-completions', tool_call: true } } });
  await call('resource_write', { kind: 'model', reference: 'responses-test', create: true, data: { provider: 'fixture', model: 'test', params: { api_format: 'responses', tool_call: true } } });
  await call('resource_write', { kind: 'prompt', reference: 'role', create: true, data: { text: 'test role' } });
  await call('resource_write', { kind: 'skill', reference: 'default/@team/review', create: true, data: { description: 'Review facts' } });
  await call('resource_write', { kind: 'agent', reference: 'researcher', create: true, data: { model: 'test', prompt: 'role', skills: ['default/@team/review'], mcps: [] } });
  await call('principal_create', { id: 'researcher', name: 'Researcher', kind: 'agent' });
  await call('agent_bind', { principal: 'researcher', agent: 'researcher', working_directory: home, policy: 'manual', max_steps: 2 });
  await call('agent_set_enabled', {principal:'researcher',enabled:false});
  let activation = await (await request('/api/bootstrap')).json();
  assert.equal(activation.bindings[0].enabled,false);
  assert.equal(activation.bindings[0].max_steps,2);
  await call('agent_set_enabled', {principal:'researcher',enabled:true});
  activation = await (await request('/api/bootstrap')).json();
  assert.equal(activation.bindings[0].enabled,true);

  const group = await call('group_create', { title: '<script>alert(1)</script>', members: ['researcher'] });
  const args = { conversation: group.id, body: 'hello', mentions: ['researcher'], request_key: 'stable' };
  assert.equal((await call('message_send', args)).duplicate, false);
  assert.equal((await call('message_send', args)).duplicate, true);
  const history = await (await request('/api/read', { name: 'message_history', viewer: 'researcher', args: { conversation: group.id } })).json();
  assert.equal(history.messages.length, 1);
  await call('principal_create', { id: 'outsider', name: 'Outsider', kind: 'user' });
  const denied = await request('/api/read', { name: 'message_history', viewer: 'outsider', args: { conversation: group.id } });
  assert.equal(denied.status, 400);
  const bootstrap = await (await request('/api/bootstrap')).json();
  assert.equal(bootstrap.bindings.length, 1);
  assert.equal(bootstrap.resources.model.items.find(m => m.name === 'responses-test').api_format, 'responses');
  assert.equal(bootstrap.resources.skill.items[0].skill, '@team/review');
  assert.ok(!JSON.stringify(bootstrap).includes('fixture-secret'));
  const error = await request('/api/tool', { name: 'message_send', args: { ...args, reply_to: 'missing' } });
  assert.equal(error.status, 400);
  assert.equal((await error.json()).error.field, 'reply_to');
  await run.startScheduler();
  await request('/api/reply', { action: 'start', conversation: group.id, confirm: true }); // manual policy: no paid call
  const activity = await settled(request);
  assert.equal(activity.runs.length, 0);
  assert.equal(activity.scheduler.tasks[0].state, 'skipped');
  assert.equal((await request('/api/conversation', { action: 'delete', conversation: group.id })).status, 400);
  assert.equal((await request('/api/conversation', { action: 'delete', conversation: group.id, confirm: true })).status, 200);
  let list = await (await request('/api/read', { name: 'conversation_list' })).json();
  assert.ok(!list.some(x => x.conversation.id === group.id));
  assert.equal((await request('/api/conversation', { action: 'reactivate', conversation: group.id })).status, 200);
  list = await (await request('/api/read', { name: 'conversation_list' })).json();
  assert.ok(list.some(x => x.conversation.id === group.id));
  const restored = await (await request('/api/read', { name: 'message_history', args: { conversation: group.id } })).json();
  assert.equal(restored.messages.length, 1);
  // The web manager must not delete groups it only observes or merely belongs to.
  const other = await run(['tool', 'call', '--principal', 'outsider', 'group_create', '--args', JSON.stringify({ title: 'not owned', members: ['operator'] })]);
  assert.equal((await request('/api/conversation', { action: 'delete', conversation: other.id, confirm: true })).status, 400);
  const auto = await request('/api/tool', { name: 'message_send', args: { ...args, request_key: 'auto' }, auto_reply: true });
  assert.equal(auto.status, 200);
  const done = await settled(request, (await auto.json()).message.id);
  assert.ok(done.scheduler.tasks.every(t => t.state === 'skipped'));
  assert.equal(done.scheduler.tasks[0].conversation, group.id);
  assert.equal(done.runs.length, 0); // manual policy never invokes a model
});


test('provider key uses a private child environment, never command arguments', async t => {
  const seen = [];
  const { request } = await fixture(t, async (args, options) => { seen.push({ args, options }); return { api_key_configured: true }; });
  const write = data => request('/api/tool', { name: 'resource_write', args: { kind: 'provider', reference: 'fixture', create: true, data } });
  const result = await write({ base_url: 'https://example.com/v1', api_key: 'private-fixture-key' });
  assert.equal(result.status, 200);
  assert.doesNotMatch(await result.text(), /private-fixture-key/);
  assert.doesNotMatch(JSON.stringify(seen[0].args), /private-fixture-key/);
  const config = JSON.parse(seen[0].args.at(-1)).data;
  assert.equal(seen[0].options.env[config.api_key_env], 'private-fixture-key');
  assert.equal(process.env[config.api_key_env], undefined);
  for (const data of [{ api_key: '' }, { api_key_env: 'ENV' }, { api_key: 'key', api_key_env: 'ENV' }]) assert.equal((await write(data)).status, 400);
  assert.equal(seen.length, 1);
});

test('resource endpoint fixes identity, prevents cascading and requires a confirmation plan', async t => {
  const seen = [];
  const { request } = await fixture(t, async args => { seen.push(args); return { items: ['prompt/default/role'], requires_confirmation: true }; });
  const resource = { kind: 'prompt', reference: 'default/role' };
  for (const data of [{ action: 'delete' }, { action: 'delete', confirm: true }, { action: 'delete', confirm: [] }, { action: 'plan', recursive: true }, { action: 'read', kind: 'unknown' }]) assert.equal((await request('/api/resource', { ...resource, ...data })).status, 400);
  assert.equal((await request('/api/resource', { ...resource, action: 'read', viewer: 'victim' })).status, 403);
  assert.equal(seen.length, 0);
  await request('/api/resource', { ...resource, action: 'read' });
  assert.deepEqual(seen[0].slice(0, 6), ['tool', 'call', '--principal', 'operator', '--manager', 'resource_read']);
  await request('/api/resource', { ...resource, action: 'plan', confirm: ['ignored'] });
  assert.deepEqual(JSON.parse(seen[1].at(-1)), { ...resource, recursive: false });
  await request('/api/resource', { ...resource, action: 'delete', confirm: ['prompt/default/role'] });
  assert.deepEqual(JSON.parse(seen[2].at(-1)), { ...resource, recursive: false, confirm: ['prompt/default/role'] });
});

test('real CLI: all six Web resources create, readable details, protected deletion and safe credentials', async t => {
  const home = await mkdtemp(resolve(tmpdir(), 'agt-web-resources-'));
  const run = cliRunner(resolve(import.meta.dirname, '../../target/debug/agt'), home);
  t.after(async () => { await run.close(); await rm(home, { recursive: true, force: true }); });
  await run(['init', home]);
  await run(['im', 'principal', 'create', 'operator']);
  const { request } = await fixture(t, run);
  const checked = async (path, data, status = 200) => {
    const response = await request(path, data);
    const value = await response.json();
    assert.equal(response.status, status, JSON.stringify(value));
    assert.doesNotMatch(JSON.stringify(value), /fixture-private-secret/);
    return value;
  };
  const write = (kind, reference, data, status) => checked('/api/tool', { name: 'resource_write', args: { kind, reference, create: true, data } }, status);
  const resource = (action, kind, reference, extra = {}, status) => checked('/api/resource', { action, kind, reference, ...extra }, status);
  await write('provider', 'fixture', { base_url: 'http://127.0.0.1:1/v1', api_key: 'fixture-private-secret' });
  let error = await write('provider', 'environment', { base_url: 'http://127.0.0.1:1/v1', api_key_env: 'ENV' }, 400);
  assert.equal(error.error.field, 'data.api_key');
  await write('model', 'model', { provider: 'fixture', model: 'test', params: { api_format: 'responses' } });
  const text = '# Role\n\n<script>window.injected = true</script>\n中文正文';
  await write('prompt', 'role', { text });
  await write('skill', 'default/@team/review', { description: 'Review facts' });
  error = await write('skill', 'Review_bad', { description: 'invalid name' }, 400);
  assert.equal(error.error.field, 'reference');
  await write('mcp', 'docs', { mcpServers: { docs: { type: 'http', url: 'http://127.0.0.1:1/mcp', http_headers: { Authorization: 'fixture-private-secret' } } } });
  error = await write('mcp', 'mismatch', { mcpServers: { wrong: { command: 'unused' } } }, 400);
  assert.equal(error.error.field, 'data.mcpServers');
  const stalePromptPlan = await resource('plan', 'prompt', 'role');
  await write('agent', 'reviewer', { model: 'model', prompt: 'role', skills: ['default/@team/review'], mcps: ['docs'] });
  await resource('delete', 'prompt', 'role', { confirm: stalePromptPlan.items }, 400);
  assert.equal((await resource('read', 'prompt', 'role')).content, text);
  assert.match((await resource('read', 'skill', 'default/@team/review')).content, /Review facts/);
  assert.equal((await resource('read', 'provider', 'fixture')).api_key_configured, true);
  assert.equal((await resource('read', 'model', 'model')).api_format, 'responses');
  assert.ok((await resource('read', 'mcp', 'docs')).config);
  assert.equal((await resource('read', 'agent', 'reviewer')).name, 'reviewer');
  for (const [kind, reference] of [['provider', 'fixture'], ['model', 'model'], ['prompt', 'role'], ['skill', 'default/@team/review'], ['mcp', 'docs'], ['prompt', 'default/default']]) await resource('plan', kind, reference, {}, 400);
  await checked('/api/tool', { name: 'principal_create', args: { id: 'reviewer', name: 'Reviewer', kind: 'agent' } });
  await checked('/api/tool', { name: 'agent_bind', args: { principal: 'reviewer', agent: 'reviewer', working_directory: home, policy: 'manual', max_steps: 2 } });
  await resource('plan', 'agent', 'reviewer', {}, 400);
  await checked('/api/tool', { name: 'agent_unbind', args: { principal: 'reviewer' } });
  error = await resource('delete', 'agent', 'reviewer', { confirm: ['agent/default/wrong'] }, 400);
  assert.equal(error.error.code, 'confirmation_mismatch');
  for (const [kind, reference] of [['agent', 'reviewer'], ['model', 'model'], ['provider', 'fixture'], ['prompt', 'role'], ['skill', 'default/@team/review'], ['mcp', 'docs']]) {
    const plan = await resource('plan', kind, reference);
    assert.equal(plan.requires_confirmation, true);
    await resource('read', kind, reference); // planning leaves the resource intact
    assert.deepEqual((await resource('delete', kind, reference, { confirm: plan.items })).deleted, plan.items);
    await resource('read', kind, reference, {}, 400);
  }
  const boot = await checked('/api/bootstrap');
  assert.equal(boot.principals.find(p => p.id === 'reviewer').deleted, true);
  await checked('/api/tool', { name: 'direct_create', args: { principal: 'reviewer' } }, 400);
  await resource('read', 'prompt', 'default/default');
});

test('provider model discovery validates input and hides remote failures', async t => {
  const seen = [];
  const { request } = await fixture(t, async args => { seen.push(args); throw new Error('private-provider-response'); });
  assert.equal((await request('/api/provider-models', { provider: '' })).status, 400);
  assert.equal(seen.length, 0);
  const response = await request('/api/provider-models', { provider: '--home' });
  assert.equal(response.status, 502);
  assert.doesNotMatch(await response.text(), /private-provider-response/);
  assert.deepEqual(seen[0], ['tool', 'call', '--principal', 'operator', '--manager', 'provider_models', '--args', JSON.stringify({ reference: '--home' })]);
});

test('real CLI: discovers provider models with stored credentials and creates models without a token cap', async t => {
  const home = await mkdtemp(resolve(tmpdir(), 'agt-web-models-'));
  const run = cliRunner(resolve(import.meta.dirname, '../../target/debug/agt'), home);
  t.after(async () => { await run.close(); await rm(home, { recursive: true, force: true }); });
  const calls = [];
  let result = { data: [{ id: 'vendor/model-b' }, { id: 'model-a' }] };
  const provider = http.createServer((req, res) => {
    calls.push({ url: req.url, authorization: req.headers.authorization });
    res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(result));
  });
  await new Promise(ok => provider.listen(0, '127.0.0.1', ok));
  t.after(() => { provider.closeAllConnections(); provider.close(); });
  await run(['init', home]); await run(['im', 'principal', 'create', 'operator']);
  const { request } = await fixture(t, run);
  const write = async (kind, reference, data) => {
    const response = await request('/api/tool', { name: 'resource_write', args: { kind, reference, create: true, data } });
    const value = await response.json(); assert.equal(response.status, 200, JSON.stringify(value)); return value;
  };
  await write('provider', 'fixture', { base_url: `http://127.0.0.1:${provider.address().port}/v1`, api_key: 'model-list-fixture-key' });
  let response = await request('/api/provider-models', { provider: 'default/fixture' });
  assert.equal(response.status, 200);
  const models = await response.json();
  assert.deepEqual(models.items.map(m => m.id), ['model-a', 'vendor/model-b']);
  assert.deepEqual(calls, [{ url: '/v1/models', authorization: 'Bearer model-list-fixture-key' }]);
  assert.doesNotMatch(JSON.stringify(models), /model-list-fixture-key/);
  for (const api_format of ['responses', 'chat-completions']) {
    const model = await write('model', api_format, { provider: 'fixture', model: models.items[1].id, params: { api_format, tool_call: true, timeout: 90 } });
    assert.equal(model.model, 'vendor/model-b');
    assert.equal(model.params.max_tokens, null);
  }
  result = { data: [] };
  for (const effort of ['max', 'future-level']) {
    const model = await write('model', effort, {provider:'fixture',model:'test',params:{reasoning_effort:effort}});
    assert.equal(model.params.reasoning_effort,effort);
    const read = await request('/api/resource',{action:'read',kind:'model',reference:effort});
    assert.equal(read.status,200);
    assert.equal((await read.json()).params.reasoning_effort,effort);
  }
  response = await request('/api/provider-models', { provider: 'fixture' });
  assert.equal(response.status, 200); assert.deepEqual((await response.json()).items, []);
  result = { data: 'private-malformed-response' };
  response = await request('/api/provider-models', { provider: 'fixture' });
  assert.equal(response.status, 502); assert.doesNotMatch(await response.text(), /private-malformed-response/);
  assert.ok(calls.every(c => c.url === '/v1/models'));
});

test('real CLI: personal entries, supervised groups and retained departure history', async t => {
  const home = await mkdtemp(resolve(tmpdir(), 'agt-web-sessions-'));
  const run = cliRunner(resolve(import.meta.dirname, '../../target/debug/agt'), home);
  t.after(async () => { await run.close(); await rm(home, { recursive: true, force: true }); });
  await run(['init', home]);
  await run(['im', 'principal', 'create', 'operator']);
  await run(['im', 'principal', 'create', 'lead', '--kind', 'agent']);
  await run(['im', 'principal', 'create', 'other']);
  await run(['im', 'owner', 'ensure', 'operator']);
  const { request } = await fixture(t, run);
  const call = (actor, name, args) => run(['tool', 'call', name, '--principal', actor, '--args', JSON.stringify(args)]);
  const group = await call('lead', 'group_create', {title:'Supervised',members:[]});
  assert.equal(group.owner_id, 'operator');
  const mutate = async (action, conversation) => {
    const response = await request('/api/conversation', {action,conversation,confirm:true});
    assert.equal(response.status,200,await response.text());
  };
  await mutate('hide',group.id);
  assert.equal((await call('operator','conversation_list',{})).length,0);
  assert.equal((await call('lead','conversation_list',{})).length,1);
  await mutate('show',group.id);
  await mutate('dissolve',group.id);
  assert.equal((await call('operator','conversation_list',{}))[0].dissolved,true);
  const restored = await request('/api/conversation',{action:'reactivate',conversation:group.id});
  assert.notEqual(restored.status,200);
  const ordinary = await call('other','group_create',{title:'Other group',members:['operator']});
  await call('other','message_send',{conversation:ordinary.id,body:'before leaving',request_key:'before'});
  await mutate('leave',ordinary.id);
  await call('other','message_send',{conversation:ordinary.id,body:'after leaving',request_key:'after'});
  const history = await request('/api/read',{name:'message_history',args:{conversation:ordinary.id},viewer:'operator'});
  assert.equal(history.status,200);
  const text = JSON.stringify(await history.json());
  assert.ok(text.includes('before leaving')); assert.ok(!text.includes('after leaving'));
  const reply = await request('/api/reply',{action:'end',conversation:ordinary.id});
  assert.equal(reply.status,403);
});

test('context reset is confirmed and uses the fixed operator through runtime', async t => {
  const seen = [];
  const { request } = await fixture(t, async args => { seen.push(args); return []; });
  assert.equal((await request('/api/conversation', {action:'reset',conversation:'one'})).status,400);
  assert.equal((await request('/api/conversation', {action:'reset',conversation:'one',confirm:true,actor:'someone'})).status,403);
  assert.equal((await request('/api/conversation', {action:'reset',conversation:'one',confirm:true})).status,200);
  assert.deepEqual(seen.at(-1), ['schedule','reset','one','--actor','operator']);
});

test('live drafts are allowlisted and scoped to current membership and task version', async t => {
  const scheduler = { online: true, tasks: [{id:'task', version:3, state:'responding', principal:'a', conversation:'one', discussion:'round'}], discussions:[{id:'round', state:'active'}] };
  const progress = {task:'task', task_version:3, phase:'generating', preview:'visible draft', attempt:2, started_at:123, updated_at:456, reasoning:'PRIVATE', tool_arguments:'PRIVATE'};
  progress.activities=[{kind:'framework',status:'running',tool_name:'framework_harness_models',label:'查询可用模型',started_at:123,updated_at:456,arguments:'PRIVATE',output:'PRIVATE'},{kind:'mcp',status:'completed',tool_name:'invalid name PRIVATE',started_at:123,updated_at:456}];
  const live = { id:'run', request:{agent:'a', trigger:{conversation_id:'one',principal:'a'}, input:'PRIVATE'}, state:'running', progress, output:'PRIVATE' };
  let members = [{principal_id:'operator',left_message_seq:null}];
  const {request} = await fixture(t, async args => args[0] === 'schedule' ? scheduler : args.includes('member_list') ? members : [live]);
  const get = async query => (await (await request('/api/activity' + query)).json()).runs[0].progress;
  assert.equal(await get(''), null);
  assert.equal(await get('?conversation=two'), null);
  const draft = await get('?conversation=one');
  assert.equal(draft.preview, 'visible draft');
  assert.equal(draft.principal, 'a');
  assert.equal(draft.activities[0].tool_name,'framework_harness_models');
  assert.equal(draft.activities[1].tool_name,undefined);
  assert.doesNotMatch(JSON.stringify(draft), /PRIVATE|tool_arguments|reasoning/);
  progress.task_version = 2; assert.equal(await get('?conversation=one'), null);
  progress.task_version = 3;
  members[0].left_message_seq = 9; assert.equal(await get('?conversation=one'), null);
  members[0].left_message_seq = null;
  for (const state of ['ended', 'limited']) { scheduler.discussions[0].state = state; assert.equal(await get('?conversation=one'), null); }
  scheduler.discussions[0].state = 'active';
  scheduler.online = false; assert.equal(await get('?conversation=one'), null);
  scheduler.online = true;
  live.state = 'completed'; assert.equal(await get('?conversation=one'), null);
});

test('real scheduler streams a draft before publishing and hides it when stopped', {timeout: 30000}, async t => {
  const home = await mkdtemp(resolve(tmpdir(), 'agt-web-stream-'));
  const run = cliRunner(resolve(import.meta.dirname, '../../target/debug/agt'), home);
  let finish;
  const model = http.createServer(async (req, res) => {
    let body = ''; for await (const chunk of req) body += chunk;
    assert.equal(JSON.parse(body).stream, true);
    res.writeHead(200, {'Content-Type':'text/event-stream'});
    res.write('data: ' + JSON.stringify({choices:[{delta:{content:'draft'}}]}) + '\n\n');
    finish = () => res.end('data: ' + JSON.stringify({choices:[{delta:{content:' complete'},finish_reason:'stop'}]}) + '\n\ndata: [DONE]\n\n');
  });
  await new Promise(ok => model.listen(0, '127.0.0.1', ok));
  t.after(async () => { finish?.(); await run.close(); model.closeAllConnections(); model.close(); await rm(home, {recursive:true,force:true}); });
  await run(['init', home]);
  await run(['im', 'principal', 'create', 'operator']);
  await run(['provider','create','fixture','--base-url',`http://127.0.0.1:${model.address().port}/v1`,'--api-key','mock']);
  await run(['model','create','test','--provider','fixture','--model','test']);
  await run(['agent','create','a','--model','test']);
  const {request} = await fixture(t, run);
  const call = async (name, args) => {
    const response = await request('/api/tool',{name,args}); const data = await response.json();
    assert.equal(response.status,200,JSON.stringify(data)); return data;
  };
  await call('principal_create',{id:'a',name:'Agent',kind:'agent'});
  await call('agent_bind',{principal:'a',agent:'a',working_directory:home,policy:'mentioned'});
  const group = await call('group_create',{title:'Streaming',members:['a']});
  await run.startScheduler();
  for (const stop of [false,true]) {
    const sent = await request('/api/tool',{name:'message_send',args:{conversation:group.id,body:'hello',mentions:['a'],request_key:String(stop)},auto_reply:true});
    assert.equal(sent.status,200);
    let visible = false;
    for (let i=0;i<500;i++) {
      const activity = await (await request('/api/activity?conversation='+group.id)).json();
      if (activity.runs.some(r => r.progress?.preview === 'draft')) { visible=true; break; }
      await new Promise(ok => setTimeout(ok,20));
    }
    assert.ok(visible,'scheduled draft was not visible');
    const history = async () => (await (await request('/api/read',{name:'message_history',args:{conversation:group.id}})).json()).messages;
    const before = await history();
    assert.equal(before.filter(m => m.sender_id === 'a').length, stop ? 1 : 0);
    if (stop) {
      assert.equal((await request('/api/reply',{action:'end',conversation:group.id})).status,202);
      const stopped = await (await request('/api/activity?conversation='+group.id)).json();
      assert.ok(stopped.runs.every(r => !r.progress));
    }
    finish();
    await settled(request);
    const after = await history();
    assert.equal(after.filter(m => m.body === 'draft complete').length, 1);
  }
});


test('explicit history visits persist via CLI without changing messages or another identity', async t => {
  const home = await mkdtemp(resolve(tmpdir(),'agt-history-window-'));
  const run = cliRunner(resolve(import.meta.dirname,'../../target/debug/agt'), home);
  t.after(async()=>{await run.close();await rm(home,{recursive:true,force:true});});
  await run(['init',home]); await run(['im','principal','create','operator']); await run(['im','principal','create','other']);
  const {request} = await fixture(t,run);
  const call = async(name,args)=> (await (await request('/api/tool',{name,args})).json());
  const chat = await call('group_create',{title:'history',members:['other']});
  await call('message_send',{conversation:chat.id,body:'keep me',request_key:'one'});
  assert.equal((await request('/api/conversation',{action:'dissolve',conversation:chat.id,confirm:true})).status,200);
  const visit = await request('/api/conversation',{action:'visit',conversation:chat.id});
  assert.equal(visit.status,200); const time=(await visit.json()).last_viewed_at; assert.ok(time>0);
  assert.equal((await request('/api/conversation',{action:'visit',conversation:chat.id,viewer:'other'})).status,403);
  const list = async viewer => (await (await request('/api/read',{name:'conversation_list',viewer,args:{include_hidden:true}})).json());
  assert.equal((await list('operator'))[0].last_viewed_at,time);
  assert.equal((await list('operator'))[0].last_viewed_at,time);
  assert.equal((await list('other'))[0].last_viewed_at,null);
  const history = await (await request('/api/read',{name:'message_history',args:{conversation:chat.id}})).json();
  assert.equal(history.messages[0].body,'keep me');
});

test('Agent entry organization only permits explicit hide/show for Agent identities', async t => {
  const seen=[];
  const {request}=await fixture(t,async args=>{
    seen.push(args);
    if(args.includes('principal_list'))return [{id:'agent',kind:'agent'},{id:'operator',kind:'user'}];
    if(args.at(-1).includes('bad'))throw new Error('private');
    return {};
  });
  const send=data=>request('/api/agent-entries',data);
  assert.equal((await send({agent:'agent',action:'hide',conversations:['one']})).status,400);
  assert.equal((await send({agent:'agent',action:'delete',conversations:['one'],confirm:true})).status,400);
  assert.equal((await send({agent:'operator',action:'hide',conversations:['one'],confirm:true})).status,403);
  const result=await send({agent:'agent',action:'hide',conversations:['one','bad'],confirm:true});
  assert.equal(result.status,200);
  assert.deepEqual((await result.json()).results.map(r=>r.ok),[true,false]);
  const calls=seen.filter(a=>a.includes('conversation_hide'));
  assert.equal(calls.length,2);
  assert.deepEqual(calls[0].slice(0,5),['tool','call','--principal','agent','conversation_hide']);
  assert.equal(calls[0].includes('--manager'),false);
  assert.equal((await send({agent:'agent',action:'show',conversations:['one'],confirm:true})).status,200);
});

test('slash commands never reach message_send implicitly; literal messages are explicit', async t => {
  const seen=[];
  const {request}=await fixture(t,async args=>{seen.push(args);return {};});
  for(const body of ['/model','/model new-model','/compact','/review','/resume','/permissions','/new','/clear','/anything-future']) {
    const response=await request('/api/tool',{name:'message_send',args:{conversation:'c',body,request_key:body}});
    assert.equal(response.status,400,body);assert.equal((await response.json()).error.code,'chat_command_required');
  }
  assert.equal(seen.length,0);
  assert.equal((await request('/api/tool',{name:'message_send',literal_message:true,args:{conversation:'c',body:'/model',request_key:'literal'}})).status,200);
  assert.equal(JSON.parse(seen.at(-1).at(-1)).body,'/model');
  for(const body of ['/tmp/project','Explain /model','/model\nThis is quoted text']) assert.equal(AgtCommands.parse(body),null);
  assert.deepEqual(AgtCommands.parse('//model'),{type:'literal',text:'/model'});
});

test('chat model/status reads are membership scoped and expose only reported metadata',async t=>{
  const {request}=await fixture(t,async args=>{
    if(args.includes('member_list')) return [{principal_id:'operator',left_message_seq:null},{principal_id:'a',left_message_seq:null}];
    if(args.includes('binding_list')) return [{principal:'a',agent:'default/a',enabled:true},{principal:'other',agent:'default/other'}];
    if(args.includes('resource_read')) {const q=JSON.parse(args.at(-1));return q.kind==='agent'?{harness:{category:'default',name:'local'}}:{engine:'codex',model:null};}
    if(args[0]==='run')return [{id:'r',state:'completed',request:{agent:'default/a',trigger:{conversation_id:'chat',principal:'a'},input:'PRIVATE'},output:'PRIVATE',execution:{model:'reported-model',harness:'default/local',thread_id:'PRIVATE'}}];
    throw new Error('unexpected operation');
  });
  const response=await request('/api/chat-status',{conversation:'chat'});assert.equal(response.status,200);
  const data=await response.json();assert.equal(data.agents.length,1);assert.equal(data.agents[0].configured_model,null);assert.equal(data.agents[0].last_run.reported_model,'reported-model');assert.doesNotMatch(JSON.stringify(data),/PRIVATE/);
  assert.equal((await request('/api/chat-status',{conversation:'chat',viewer:'other'})).status,403);
});

test('Harness operations validate membership, current session and fixed identity',async t=>{
  const calls=[];
  const {request}=await fixture(t,async args=>{
    calls.push(args);
    if(args.includes('member_list'))return [{principal_id:'operator',left_message_seq:null},{principal_id:'a',left_message_seq:null}];
    if(args.includes('current'))return {id:'current-session',principal:'a',conversation:'chat'};
    return {data:[]};
  });
  const send=extra=>request('/api/harness',{conversation:'chat',target:'a',session:'current-session',operation:'models',...extra});
  assert.equal((await send({viewer:'other'})).status,403);
  assert.equal((await send({target:'not-member'})).status,403);
  assert.equal((await send({session:'someone-elses-session'})).status,409);
  assert.equal((await send({operation:'arbitrary-rpc'})).status,400);
  assert.equal(calls.filter(a=>a[0]==='harness' && a[2]==='models').length,0);
  assert.equal((await send({})).status,200);
  assert.equal(calls.at(-1)[5], 'harness_session_models');
  assert.deepEqual(JSON.parse(calls.at(-1).at(-1)),{conversation:'chat',target:'a',session:'current-session'});
  assert.equal((await send({operation:'configure',model:'fixture-model',effort:'xhigh'})).status,200);
  assert.equal(calls.at(-1)[5], 'harness_session_configure');
  assert.deepEqual(JSON.parse(calls.at(-1).at(-1)),{conversation:'chat',target:'a',session:'current-session',model:'fixture-model',effort:'xhigh'});
  for(const operation of ['compact','cancel','skills','mcp'])assert.equal((await send({operation})).status,200);
  assert.equal((await send({operation:'rename',title:'Review session'})).status,200);
});

test('event streams require a valid operator/agent viewer and release the subscription', async t => {
  let onPage, stopped = 0, options;
  const run = async () => [{ id: 'operator', kind: 'user' }, { id: 'agent', kind: 'agent' }, { id: 'other-human', kind: 'user' }, { id: 'deleted', kind: 'agent', deleted: true }];
  run.watchEvents = (selected, page) => { options = selected; onPage = page; return () => { stopped++; }; };
  const { request, url } = await fixture(t, run);
  assert.equal((await fetch(`${url}/api/events`)).status, 401);
  assert.equal((await request('/api/events?viewer=other-human')).status, 403);
  assert.equal((await request('/api/events?viewer=deleted')).status, 403);
  const response = await request('/api/events?viewer=agent&after=scope-cursor');
  assert.match(response.headers.get('content-type'), /text\/event-stream/);
  assert.deepEqual(options, { principal: 'agent', after: 'scope-cursor' });
  const reader = response.body.getReader();
  onPage({ event: 'change', cursor: 'next', changes: [{ conversation: 'visible', kind: 'work' }] });
  const { value } = await reader.read();
  assert.match(new TextDecoder().decode(value), /event: change/);
  await reader.cancel();
  for (let i = 0; i < 50 && !stopped; i++) await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(stopped, 1);
});

test('real JSONL watch delivers committed changes and replays a reconnect cursor', async t => {
  const home = await mkdtemp(resolve(tmpdir(), 'agt-events-'));
  const run = cliRunner(resolve(import.meta.dirname, '../../target/debug/agt'), home);
  t.after(async () => { await run.close(); await rm(home, { recursive: true, force: true }); });
  await run(['init', home]);
  await run(['im', 'principal', 'create', 'operator', '--name', 'Operator', '--kind', 'user']);
  const pages = [], errors = [];
  const stop = run.watchEvents({ principal: 'operator' }, page => pages.push(page), () => errors.push('ended'));
  t.after(stop);
  const until = async predicate => {
    for (let i = 0; i < 200; i++) { if (predicate()) return; if (errors.length) assert.fail('watch exited'); await new Promise(resolve => setTimeout(resolve, 20)); }
    assert.fail('watch did not deliver');
  };
  await until(() => pages.some(p => p.event === 'ready'));
  const cursor = pages.find(p => p.event === 'ready').cursor;
  await run(['im', 'principal', 'create', 'agent', '--name', 'Agent', '--kind', 'agent']);
  await until(() => pages.some(p => p.changes.some(c => c.kind === 'access')));
  const replay = [];
  const stopReplay = run.watchEvents({ principal: 'operator', after: cursor }, page => replay.push(page), () => {});
  t.after(stopReplay);
  await until(() => replay.some(p => p.changes.some(c => c.kind === 'access')));
});


test('task queries use observer identity while task controls fix the human operator', async t => {
  const seen = [];
  const {request} = await fixture(t, async args => {seen.push(args);return [];});
  assert.equal((await request('/api/read',{name:'task_list',viewer:'lead',args:{conversation:'chat',before:4}})).status,200);
  assert.deepEqual(seen[0].slice(0,5),['tool','call','--principal','lead','task_list']);
  assert.equal((await request('/api/tool',{name:'task_cancel',viewer:'lead',args:{}})).status,403);
  assert.equal((await request('/api/tool',{name:'task_reply',args:{task_id:'id',expected_revision:3,answer:'ok'}})).status,200);
  assert.deepEqual(seen[1].slice(0,6),['tool','call','--principal','operator','--manager','task_reply']);
});

test('workspace config uses validated CLI contract and rejects identity injection',async t=>{
  const calls=[];
  const {request}=await fixture(t,async args=>{calls.push(args);return args[1]==='describe'?{fields:[]}:{scheduler:{concurrency:3}};});
  const response=await request('/api/config');assert.equal(response.status,200);
  assert.deepEqual(calls.slice(0,2),[['config','show'],['config','describe']]);
  assert.equal((await request('/api/config',{config:{},expected:{},viewer:'a'})).status,400);
  assert.equal((await request('/api/config',{config:{scheduler:{concurrency:null}},expected:{scheduler:{concurrency:3}}})).status,200);
  assert.deepEqual(calls.at(-1),['config','update','--value','{"scheduler":{"concurrency":null}}','--expected','{"scheduler":{"concurrency":3}}']);
});
