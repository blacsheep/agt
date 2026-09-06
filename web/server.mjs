import http from 'node:http';
import { execFile } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { readFile, realpath, access } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const readTools = new Set(['conversation_list', 'member_list', 'message_history', 'message_status', 'message_inbox']);
const writeTools = new Set(['direct_create', 'group_create', 'message_send', 'member_add', 'member_remove', 'resource_write', 'principal_create', 'agent_bind', 'agent_unbind']);
const kinds = ['provider', 'model', 'prompt', 'skill', 'mcp', 'agent'];
const assets = new Map([['/', ['index.html', 'text/html']], ['/app.js', ['app.js', 'text/javascript']], ['/style.css', ['style.css', 'text/css']]]);

export class ApiError extends Error {
  constructor(status, code, message, field) { super(message); Object.assign(this, { status, code, field }); }
}

export function cliRunner(binary, home) {
  const children = new Set();
  const run = (args, { worker = false } = {}) => new Promise((resolveResult, reject) => {
    const child = execFile(binary, ['--home', home, '--output', 'json', ...args], {
      timeout: worker ? 0 : 30_000, maxBuffer: 8 * 1024 * 1024, windowsHide: true,
    }, (error, stdout) => {
      children.delete(child);
      let value;
      try { value = JSON.parse(stdout); } catch { /* Never echo raw stderr/config data. */ }
      if (error) {
        const safe = value?.error;
        reject(new ApiError(400, typeof safe?.code === 'string' ? safe.code : 'cli_failed',
          typeof safe?.message === 'string' ? safe.message : '操作未完成。请检查资源配置、权限或命令超时；重试前先刷新当前状态。', safe?.field));
      } else if (value === undefined) reject(new ApiError(502, 'invalid_response', 'CLI 返回了无法解析的响应。'));
      else resolveResult(value);
    });
    children.add(child);
  });
  run.close = () => { for (const child of children) child.kill('SIGTERM'); };
  return run;
}

function object(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new ApiError(400, 'invalid_arguments', '请求必须是 JSON 对象。');
  return value;
}
function str(value) {
  if (typeof value !== 'string' || !value.trim() || value.length > 256) throw new ApiError(400, 'invalid_arguments', '标识符必须是非空字符串。');
  return value;
}
async function body(req) {
  if (req.headers['content-type']?.split(';')[0] !== 'application/json') throw new ApiError(415, 'content_type', '需要 application/json。');
  let size = 0; const chunks = [];
  for await (const chunk of req) {
    size += chunk.length;
    if (size > 512 * 1024) throw new ApiError(413, 'too_large', '请求超过 512 KiB。');
    chunks.push(chunk);
  }
  try { return object(JSON.parse(Buffer.concat(chunks).toString())); }
  catch (e) { if (e instanceof ApiError) throw e; throw new ApiError(400, 'invalid_json', 'JSON 格式不正确。'); }
}

export function createConsole({ run, home, principal = 'web-operator', token = randomBytes(32).toString('hex') }) {
  let job = { state: 'idle', outcomes: [] };
  const tool = (name, args = {}, viewer = principal) => run(['tool', 'call', '--principal', viewer,
    ...(viewer === principal ? ['--manager'] : []), name, '--args', JSON.stringify(args)]);
  const server = http.createServer(async (req, res) => {
    const send = (status, value) => { res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8' }); res.end(JSON.stringify(value)); };
    res.setHeader('Cache-Control', 'no-store');
    res.setHeader('X-Content-Type-Options', 'nosniff');
    res.setHeader('Referrer-Policy', 'no-referrer');
    res.setHeader('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'");
    try {
      const host = `127.0.0.1:${server.address().port}`;
      if (req.headers.host !== host || (req.headers.origin && req.headers.origin !== `http://${host}`)) throw new ApiError(403, 'origin_denied', '仅允许从本机控制台访问。');
      const url = new URL(req.url, `http://${host}`);
      if (!url.pathname.startsWith('/api/')) {
        const asset = assets.get(url.pathname);
        if (req.method !== 'GET' || !asset) throw new ApiError(404, 'not_found', '页面不存在。');
        res.writeHead(200, { 'Content-Type': `${asset[1]}; charset=utf-8` });
        res.end(await readFile(resolve(here, 'public', asset[0]))); return;
      }
      if (req.headers['x-agt-token'] !== token) throw new ApiError(401, 'unauthorized', '请使用启动终端提供的完整链接打开控制台。');
      if (req.method === 'GET' && url.pathname === '/api/bootstrap') {
        const [principals, bindings, ...resources] = await Promise.all([
          tool('principal_list'), tool('binding_list'), ...kinds.map(kind => tool('resource_list', { kind })),
        ]);
        send(200, { home, principal, principals, bindings, resources: Object.fromEntries(kinds.map((k, i) => [k, resources[i]])) });
      } else if (req.method === 'GET' && url.pathname === '/api/activity') {
        const runs = await run(['run', 'list', '--limit', '50']);
        // Run input/output stays on the host; the list only needs operational metadata.
        send(200, { job, runs: runs.map(r => ({ id: r.id, agent: r.request.agent, state: r.state, model_steps: r.model_steps, tool_failures: r.tool_failures, error: r.error ? '执行失败，请查看调用记录或在主机运行 agt run info。' : null })) });
      } else if (req.method === 'POST' && url.pathname === '/api/read') {
        const data = await body(req); const name = str(data.name);
        if (!readTools.has(name)) throw new ApiError(403, 'tool_denied', '观察接口仅允许读取会话。');
        send(200, await tool(name, object(data.args ?? {}), data.viewer ? str(data.viewer) : principal));
      } else if (req.method === 'POST' && url.pathname === '/api/tool') {
        const data = await body(req); const name = str(data.name);
        if (!writeTools.has(name)) throw new ApiError(403, 'tool_denied', '该操作未开放。');
        if ('viewer' in data || 'principal' in data) throw new ApiError(403, 'identity_fixed', '写入身份由启动者固定。');
        send(200, await tool(name, object(data.args ?? {})));
      } else if (req.method === 'POST' && url.pathname === '/api/calls') {
        const data = await body(req);
        const after = data.after ?? 0;
        if (!Number.isSafeInteger(after) || after < 0) throw new ApiError(400, 'invalid_cursor', '调用游标无效。');
        send(200, await run(['run', 'calls', str(data.id), '--after', String(after), '--limit', '100']));
      } else if (req.method === 'POST' && url.pathname === '/api/worker') {
        const data = await body(req);
        if (data.confirm !== true) throw new ApiError(400, 'confirmation_required', '请确认付费模型执行。');
        if (job.state === 'running') throw new ApiError(409, 'worker_busy', '已有一批任务在执行，请勿重复启动。');
        const limit = data.limit ?? 1;
        if (!Number.isInteger(limit) || limit < 1 || limit > 10 || (data.allow_mcp !== undefined && typeof data.allow_mcp !== 'boolean')) throw new ApiError(400, 'invalid_arguments', '每位 Agent 每批投递数应为 1–10。');
        job = { state: 'running', started_at: Date.now(), outcomes: [] };
        const current = job;
        run(['worker', '--limit', String(limit), ...(data.allow_mcp ? ['--allow-mcp'] : [])], { worker: true })
          .then(outcomes => Object.assign(current, { state: 'completed', outcomes, finished_at: Date.now() }))
          .catch(error => Object.assign(current, { state: 'failed', error: error.message, finished_at: Date.now() }));
        send(202, job);
      } else throw new ApiError(404, 'not_found', '接口不存在。');
    } catch (error) {
      if (!res.headersSent) send(error.status ?? 500, { error: { code: error.code ?? 'internal_error', message: error instanceof ApiError ? error.message : '本地服务异常，请查看终端并刷新。', field: error.field } });
      else res.end();
    }
  });
  server.requestTimeout = 35_000;
  return { server, token };
}

export async function start(argv = process.argv.slice(2)) {
  const options = { home: resolve(homedir(), '.agt'), binary: resolve(here, '../target/debug/agt'), principal: 'web-operator', port: 4317 };
  for (let i = 0; i < argv.length; i += 2) {
    const key = { '--home': 'home', '--binary': 'binary', '--principal': 'principal', '--port': 'port' }[argv[i]];
    if (!key || !argv[i + 1]) throw new Error('用法：node web/server.mjs [--home DIR] [--binary PATH] [--principal ID] [--port 4317]');
    options[key] = key === 'port' ? Number(argv[i + 1]) : argv[i + 1];
  }
  if (!Number.isInteger(options.port) || options.port < 0 || options.port > 65535) throw new Error('端口无效');
  str(options.principal);
  options.binary = resolve(options.binary); options.home = resolve(options.home);
  await access(options.binary);
  const run = cliRunner(options.binary, options.home);
  try {
    // Explicit server startup prepares the home and a dedicated human operator only.
    const principals = await run(['im', 'principal', 'list']);
    const found = principals.find(p => p.id === options.principal);
    if (found && found.kind !== 'user') throw new Error('操作身份已被非用户占用，请使用 --principal 指定新身份。');
    if (!found) await run(['im', 'principal', 'create', options.principal, '--name', '本地操作员', '--kind', 'user']);
    options.home = await realpath(options.home);
    const { server, token } = createConsole({ ...options, run });
    await new Promise((ok, fail) => { server.once('error', fail); server.listen(options.port, '127.0.0.1', ok); });
    console.log(`\nAGT Studio · 本地控制台\n资源目录：${options.home}\n操作身份：${options.principal}\n打开：http://127.0.0.1:${server.address().port}/#${token}\n仅本机访问。此链接拥有主机管理权限，请勿分享。按 Ctrl+C 退出。\n`);
    const stop = () => { run.close(); server.close(); server.closeAllConnections(); };
    process.once('SIGINT', stop); process.once('SIGTERM', stop);
    return { server, run };
  } catch (error) { run.close(); throw error; }
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  start().catch(error => { console.error(error.message); process.exitCode = 1; });
}
