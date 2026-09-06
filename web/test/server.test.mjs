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

test('worker requires paid confirmation, permits only one active finite batch', async t => {
  let finish; const seen = [];
  const { request } = await fixture(t, args => {
    seen.push(args);
    if (args[0] === 'worker') return new Promise(ok => { finish = ok; });
    return Promise.resolve([]);
  });
  assert.equal((await request('/api/worker', { limit: 1 })).status, 400);
  assert.equal((await request('/api/worker', { limit: 100, confirm: true })).status, 400);
  assert.equal((await request('/api/worker', { limit: 1, confirm: true })).status, 202);
  assert.equal((await request('/api/worker', { limit: 1, confirm: true })).status, 409);
  assert.deepEqual(seen, [['worker', '--limit', '1']]);
  finish([{ state: 'skipped' }]);
  await new Promise(ok => setImmediate(ok));
  const activity = await (await request('/api/activity')).json();
  assert.equal(activity.job.state, 'completed');
  assert.equal(activity.job.outcomes.length, 1);
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

test('real CLI: resources, binding, chat, observation, safe errors, empty worker', async t => {
  const home = await mkdtemp(resolve(tmpdir(), 'agt-web-test-'));
  const binary = resolve(import.meta.dirname, '../../target/debug/agt');
  const run = cliRunner(binary, home);
  t.after(async () => { run.close(); await rm(home, { recursive: true, force: true }); });
  await run(['init', home]);
  await run(['im', 'principal', 'create', 'operator']);
  await run(['provider', 'create', 'fixture', '--base-url', 'http://127.0.0.1:1/v1', '--api-key', 'fixture-secret']);
  const { request } = await fixture(t, run);
  const call = async (name, args) => {
    const response = await request('/api/tool', { name, args });
    const body = await response.json(); assert.equal(response.status, 200, JSON.stringify(body)); return body;
  };
  await call('resource_write', { kind: 'model', reference: 'test', create: true, data: { provider: 'fixture', model: 'test', params: { api_format: 'chat-completions', tool_call: true } } });
  await call('resource_write', { kind: 'prompt', reference: 'role', create: true, data: { text: 'test role' } });
  await call('resource_write', { kind: 'skill', reference: 'default/@team/review', create: true, data: { description: 'Review facts' } });
  await call('resource_write', { kind: 'agent', reference: 'researcher', create: true, data: { model: 'test', prompt: 'role', skills: ['default/@team/review'], mcps: [] } });
  await call('principal_create', { id: 'researcher', name: 'Researcher', kind: 'agent' });
  await call('agent_bind', { principal: 'researcher', agent: 'researcher', working_directory: home, policy: 'manual', max_steps: 2, max_depth: 2 });
  const group = await call('group_create', { title: '<script>alert(1)</script>', members: ['researcher'] });
  const args = { conversation: group.id, body: 'hello', mentions: ['researcher'], request_key: 'stable' };
  assert.equal((await call('message_send', args)).duplicate, false);
  assert.equal((await call('message_send', args)).duplicate, true);
  const history = await (await request('/api/read', { name: 'message_history', viewer: 'researcher', args: { conversation: group.id } })).json();
  assert.equal(history.messages.length, 1);
  await call('principal_create', { id: 'outsider', name: 'Outsider', kind: 'agent' });
  const denied = await request('/api/read', { name: 'message_history', viewer: 'outsider', args: { conversation: group.id } });
  assert.equal(denied.status, 400);
  const bootstrap = await (await request('/api/bootstrap')).json();
  assert.equal(bootstrap.bindings.length, 1);
  assert.equal(bootstrap.resources.skill.items[0].skill, '@team/review');
  assert.ok(!JSON.stringify(bootstrap).includes('fixture-secret'));
  const error = await request('/api/tool', { name: 'message_send', args: { ...args, reply_to: 'missing' } });
  assert.equal(error.status, 400);
  assert.equal((await error.json()).error.field, 'reply_to');
  await request('/api/worker', { confirm: true, limit: 1 }); // manual policy: no paid call
  let activity;
  for (let i = 0; i < 100; i++) {
    activity = await (await request('/api/activity')).json();
    if (activity.job.state !== 'running') break;
    await new Promise(ok => setTimeout(ok, 20));
  }
  assert.equal(activity.job.state, 'completed');
  assert.equal(activity.runs.length, 0);
  assert.equal(activity.job.outcomes[0].state, 'skipped');
});
