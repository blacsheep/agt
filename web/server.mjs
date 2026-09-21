import http from 'node:http';
import './public/commands.js';
import { execFile, spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { readFile, realpath, access } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const readTools = new Set(['conversation_list', 'member_list', 'message_history', 'message_status', 'message_inbox', 'task_get', 'task_list', 'task_attention']);
const writeTools = new Set(['direct_create', 'group_create', 'message_send', 'member_add', 'member_remove', 'resource_write', 'principal_create', 'agent_bind', 'agent_unbind', 'agent_set_enabled', 'task_cancel', 'task_reply', 'task_reprioritize', 'task_defer', 'task_resolve', 'task_retry', 'task_report']);
const kinds = ['provider', 'model', 'prompt', 'skill', 'mcp', 'agent', 'harness'];
const assets = new Map([['/interaction-graph.js', ['interaction-graph.js', 'text/javascript']], ['/interactions.js', ['interactions.js', 'text/javascript']], ['/', ['index.html', 'text/html']], ['/app.js', ['app.js', 'text/javascript']], ['/tasks.js', ['tasks.js', 'text/javascript']], ['/settings.js', ['settings.js', 'text/javascript']], ['/commands.js', ['commands.js', 'text/javascript']], ['/style.css', ['style.css', 'text/css']]]);

export class ApiError extends Error {
  constructor(status, code, message, field) { super(message); Object.assign(this, { status, code, field }); }
}

export function cliRunner(binary, home) {
  const children = new Set();
  const run = (args, { env = {} } = {}) => new Promise((resolveResult, reject) => {
    const child = execFile(binary, ['--home', home, '--output', 'json', ...args], {
      env: { ...process.env, ...env }, timeout: args[0] === 'harness' && args[1] === 'session' && args[2] === 'compact' ? 86_400_000 : 30_000, maxBuffer: 8 * 1024 * 1024, windowsHide: true,
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
  run.startScheduler = async () => {
    // A standby coordinator takes over when an old process exits or its lease
    // expires; a stale online snapshot must not prevent startup after a crash.
    const child = spawn(binary, ['--home', home, '--output', 'json', 'schedule', 'run', '--watch'], { stdio: 'ignore', windowsHide: true });
    children.add(child);
    let stopped = false;
    child.once('error', () => { stopped = true; children.delete(child); });
    child.once('exit', () => { stopped = true; children.delete(child); });
    for (let attempt = 0; attempt < 50; attempt++) {
      if ((await run(['schedule', 'status'])).online) return;
      if (stopped) break;
      await new Promise(ok => setTimeout(ok, 100));
    }
    child.kill('SIGTERM');
    throw new ApiError(503, 'scheduler_offline', '回复服务未能启动，请检查调度进程状态和数据目录权限。');
  };
  run.watchEvents = ({ principal, after }, onPage, onEnd) => {
    const child = spawn(binary, ['--home', home, 'events', 'watch', '--principal', principal, ...(after ? ['--after', after] : [])], { stdio: ['ignore', 'pipe', 'ignore'], windowsHide: true });
    children.add(child);
    let buffer = '', ended = false;
    const finish = () => { if (ended) return; ended = true; children.delete(child); onEnd(); };
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => {
      buffer += chunk;
      if (buffer.length > 256 * 1024) { child.kill('SIGTERM'); finish(); return; }
      let boundary;
      while ((boundary = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, boundary); buffer = buffer.slice(boundary + 1);
        try { const page = JSON.parse(line); if (!['ready','change','resync_required'].includes(page.event) || typeof page.cursor !== 'string' || !Array.isArray(page.changes)) throw new Error(); onPage(page); }
        catch { child.kill('SIGTERM'); finish(); return; }
      }
    });
    child.once('error', finish); child.once('exit', finish);
    return () => { child.kill('SIGTERM'); finish(); };
  };
  run.close = () => Promise.all([...children].map(child => new Promise(resolve => {
    if (child.exitCode !== null || child.signalCode !== null) { resolve(); return; }
    child.once('exit', resolve); child.once('error', resolve);
    child.kill('SIGTERM');
  })));
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
  const changingConversations = new Set();
  const streams = new Set();

  const tool = (name, args = {}, viewer = principal) => run(['tool', 'call', '--principal', viewer,
    ...(viewer === principal ? ['--manager'] : []), name, '--args', JSON.stringify(args)]);
  const member = conversation => tool('member_list', { conversation });
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
      if (req.method === 'GET' && url.pathname === '/api/events') {
        if (!run.watchEvents) throw new ApiError(503, 'events_unavailable', '事件订阅未就绪，请更新 CLI 后重启服务。');
        if (streams.size >= 8) throw new ApiError(429, 'too_many_streams', '订阅连接过多，请关闭多余窗口。');
        const viewer = str(url.searchParams.get('viewer') || principal);
        const after = url.searchParams.get('after');
        if (after !== null) str(after);
        const identities = await tool('principal_list');
        if (!identities.some(p => p.id === viewer && !p.deleted && (viewer === principal || p.kind === 'agent'))) throw new ApiError(403, 'viewer_denied', '观察身份不存在或不可用。');
        if (streams.size >= 8) throw new ApiError(429, 'too_many_streams', '订阅连接过多，请关闭多余窗口。');
        res.writeHead(200, { 'Content-Type': 'text/event-stream; charset=utf-8', 'X-Accel-Buffering': 'no', Connection: 'keep-alive' });
        res.flushHeaders();
        streams.add(res);
        let stop;
        const cleanup = () => { streams.delete(res); stop?.(); };
        res.once('close', cleanup);
        stop = run.watchEvents({ principal: viewer, after }, page => {
          // Rust checks membership on every delivery. No raw stdout/stderr reaches the client.
          if (res.destroyed || !res.write(`event: ${page.event}\ndata: ${JSON.stringify(page)}\n\n`)) res.destroy();
        }, () => res.end());
        if (res.destroyed) cleanup();
      } else if (url.pathname === '/api/config' && req.method === 'GET') {
        const [config, schema] = await Promise.all([run(['config','show']),run(['config','describe'])]);
        send(200,{config,schema});
      } else if (url.pathname === '/api/config' && req.method === 'POST') {
        const data=await body(req);
        if (Object.keys(data).some(k=>!['config','expected'].includes(k))) throw new ApiError(400,'invalid_config','配置请求包含未知字段。');
        send(200,await run(['config','update','--value',JSON.stringify(object(data.config)),'--expected',JSON.stringify(object(data.expected))]));
      } else if (req.method === 'GET' && url.pathname === '/api/bootstrap') {
        const [principals, bindings, ...resources] = await Promise.all([
          tool('principal_list', { include_deleted: true }), tool('binding_list'), ...kinds.map(kind => tool('resource_list', { kind })),
        ]);
        send(200, { home, principal, principals, bindings, resources: Object.fromEntries(kinds.map((k, i) => [k, resources[i]])) });
      } else if (req.method === 'GET' && url.pathname === '/api/activity') {
        const [runs, scheduler] = await Promise.all([run(['run', 'list', '--limit', '50']), run(['schedule', 'status'])]);
        // Drafts are only returned for the selected, currently visible conversation.
        const conversation = url.searchParams.get('conversation');
        const viewer = url.searchParams.get('viewer') || principal;
        let visible = false;
        if (conversation && scheduler.online) {
          const members = await tool('member_list', { conversation }, viewer).catch(() => []);
          visible = members.some(m => m.principal_id === viewer && m.left_message_seq == null);
        }
        const progress = r => {
          const p = r.progress, trigger = r.request.trigger;
          if (!visible || !p || r.state !== 'running' || r.parent_run_id || trigger?.conversation_id !== conversation) return null;
          const task = scheduler.tasks.find(t => t.id === p.task && t.version === p.task_version && t.state === 'responding' && t.principal === trigger.principal && t.conversation === conversation);
          const round = task && scheduler.discussions.find(d => d.id === task.discussion);
          if (!round || !['active', 'paused'].includes(round.state)) return null;
          return { phase: p.phase, attempt: p.attempt, preview: typeof p.preview === 'string' ? p.preview.slice(-4096) : '', started_at: p.started_at, updated_at: p.updated_at, principal: task.principal,
            ...(Number.isFinite(p.protocol_at) ? { protocol_at: p.protocol_at, local_checked_at: p.local_checked_at, meaningful_at: p.meaningful_at,
              } : {}),
            activities: (Array.isArray(p.activities) ? p.activities : []).slice(-40).filter(a => ['command','files','mcp','framework','input'].includes(a.kind) && ['running','completed','failed','canceled','unsupported','unknown'].includes(a.status)).map(a => ({kind:a.kind,status:a.status,started_at:a.started_at,updated_at:a.updated_at,
              ...(typeof a.tool_name==='string' && /^[a-zA-Z0-9_.:/-]{1,128}$/.test(a.tool_name) ? {tool_name:a.tool_name}:{}),
              ...(typeof a.label==='string'?{label:a.label.slice(0,80)}:{}),...(typeof a.summary==='string'?{summary:a.summary.slice(0,120)}:{})})) };
        };
        // Never serialize private Run input, final output, tool arguments or summaries.
        send(200, { scheduler, runs: runs.map(r => ({ id: r.id, agent: r.request.agent, conversation: r.request.trigger?.conversation_id, state: r.state, model_steps: r.model_steps, tool_failures: r.tool_failures, error: r.error ? '执行失败，请查看消息投递了解原因。' : null, progress: progress(r) })) });
      } else if (req.method === 'POST' && url.pathname === '/api/harness') {
        const data = await body(req), operation = str(data.operation);
        if ('viewer' in data || 'actor' in data || 'principal' in data) throw new ApiError(403, 'identity_fixed', '操作身份由启动者固定。');
        if (operation === 'capabilities') { send(200, await run(['harness','capabilities'])); return; }
        if (operation === 'resource-models') { send(200,await tool('harness_models',{reference:str(data.reference)})); return; }
        if (operation === 'resource-sessions') {
          const reference = str(data.reference);
          const records = await run(['harness','session','list']);
          const visible = [];
          for (const session of records.filter(s=>s.harness===reference && s.conversation)) {
            const members = await member(session.conversation).catch(()=>[]);
            if (members.some(m=>m.principal_id===principal && m.left_message_seq==null)) visible.push(session);
          }
          send(200,visible); return;
        }
        const conversation = str(data.conversation), target = str(data.target);
        const members = await member(conversation);
        if (![principal,target].every(id=>members.some(m=>m.principal_id===id && m.left_message_seq==null))) throw new ApiError(403,'not_member','仅当前会话成员可管理该 Agent 的执行会话。');
        const session = await run(['harness','session','current','--principal',target,'--conversation',conversation]);
        if (operation === 'current') { send(200,session); return; }
        if (str(data.session) !== session.id) throw new ApiError(409,'session_changed','执行会话已变化，请刷新后重试。');
        if (operation === 'models' || operation === 'configure') {
          send(200,await tool(operation === 'models' ? 'harness_session_models' : 'harness_session_configure',{
            conversation,target,session:session.id,...(operation === 'configure' ? {model:str(data.model),...(data.effort?{effort:str(data.effort)}:{})}:{})
          })); return;
        }
        const args = ['harness','session'];
        if (['skills','mcp','compact','cancel'].includes(operation)) args.push(operation,session.id);
        else if (operation === 'rename') args.push('rename',session.id,str(data.title));
        else throw new ApiError(400,'unsupported_operation','尚未接入此 Harness 操作。');
        send(200,await run(args));
      } else if (req.method === 'POST' && url.pathname === '/api/chat-status') {
        const data = await body(req), conversation = str(data.conversation);
        if ('viewer' in data || 'principal' in data || 'actor' in data) throw new ApiError(403, 'identity_fixed', '查看身份由启动者固定。');
        const members = await member(conversation);
        if (!members.some(m => m.principal_id === principal && m.left_message_seq == null)) throw new ApiError(403, 'not_member', '仅当前会话成员可查看运行信息。');
        const [bindings, runs] = await Promise.all([tool('binding_list'), run(['run','list','--limit','200'])]);
        const agents = await Promise.all(bindings.filter(b => members.some(m => m.principal_id === b.principal && m.left_message_seq == null)).map(async b => {
          const definition = await tool('resource_read', {kind:'agent',reference:b.agent});
          const harness = definition.harness && `${definition.harness.category}/${definition.harness.name}`;
          const config = await tool('resource_read', {kind:harness ? 'harness' : 'model',reference:harness || `${definition.model.category}/${definition.model.name}`});
          const last = runs.find(r => r.request.trigger?.conversation_id === conversation && r.request.trigger?.principal === b.principal && r.request.agent === b.agent);
          return {principal:b.principal,agent:b.agent,enabled:b.enabled !== false,engine:harness ? config.engine : 'native',harness:harness || null,configured_model:config.model || null,
            last_run:last ? {id:last.id,state:last.state,reported_model:typeof last.execution?.model === 'string' ? last.execution.model : null,harness:last.execution?.harness || null} : null};
        }));
        send(200,{conversation,agents});
      } else if (req.method === 'POST' && url.pathname === '/api/read') {
        const data = await body(req); const name = str(data.name);
        if (!readTools.has(name)) throw new ApiError(403, 'tool_denied', '观察接口仅允许读取会话。');
        const args=object(data.args ?? {}), viewer=data.viewer ? str(data.viewer) : principal;
        const result=await tool(name,args,viewer);
        if(name==='message_history' && Array.isArray(result.messages)) {
          const first=result.messages[0]?.seq ?? 0;
          const last=result.messages.at(-1)?.seq ?? 0;
          result.context_resets=await run(['schedule','reset-history',str(args.conversation),'--actor',viewer,'--from',String(result.next_cursor ? Math.max(0,first-1) : 0),'--through',String(last)]);
        }
        send(200,result);
      } else if (req.method === 'POST' && url.pathname === '/api/tool') {
        const data = await body(req); const name = str(data.name);
        if (!writeTools.has(name)) throw new ApiError(403, 'tool_denied', '该操作未开放。');
        if ('viewer' in data || 'principal' in data) throw new ApiError(403, 'identity_fixed', '写入身份由启动者固定。');
        if (data.auto_reply !== undefined && typeof data.auto_reply !== 'boolean') throw new ApiError(400, 'invalid_arguments', '自动回复必须为布尔值。');
        const args = object(data.args ?? {});
        if (data.literal_message !== undefined && typeof data.literal_message !== 'boolean') throw new ApiError(400, 'invalid_arguments', '普通文本标记必须为布尔值。');
        if (name === 'message_send' && data.literal_message !== true && AgtCommands.parse(args.body)?.type === 'command') throw new ApiError(400, 'chat_command_required', '这是控制命令，请通过 Web 命令入口执行；如需讨论命令本身，请明确按普通文本发送。');
        if (name === 'resource_write' && args.kind === 'provider' && 'api_key_env' in object(args.data)) throw new ApiError(400, 'invalid_arguments', '请直接输入 API Key，Web 不支持环境变量方式。', 'data.api_key');
        let result;
        if (name === 'resource_write' && args.kind === 'provider' && args.data && typeof args.data === 'object' && 'api_key' in args.data) {
          const { api_key, ...config } = object(args.data);
          if (typeof api_key !== 'string' || !api_key.trim()) throw new ApiError(400, 'invalid_arguments', '请填写非空的 API Key。', 'data.api_key');
          const key = `AGT_WEB_CREDENTIAL_${randomBytes(16).toString('hex')}`;
          result = await run(['tool', 'call', '--principal', principal, '--manager', name, '--args', JSON.stringify({ ...args, data: { ...config, api_key_env: key } })], { env: { [key]: api_key } });
        } else result = await tool(name, args);
        if (name === 'message_send' && data.auto_reply === true) await run(['schedule', 'start', str(args.conversation), '--message', str(result.message.id)]);
        send(200, result);
      } else if (req.method === 'POST' && url.pathname === '/api/harness-test') {
        const data = await body(req);
        if ('viewer' in data || 'principal' in data) throw new ApiError(403, 'identity_fixed', '连接测试仅供控制台管理员使用。');
        send(200, await run(['harness', 'resource', 'test', str(data.reference)]));
      } else if (req.method === 'POST' && url.pathname === '/api/provider-models') {
        const data = await body(req);
        const provider = str(data.provider);
        try { send(200, await tool('provider_models', { reference: provider })); }
        catch { throw new ApiError(502, 'provider_models_failed', '无法获取模型列表。请检查 Provider 地址、密钥和连接，或直接填写模型名称。'); }
      } else if (req.method === 'POST' && url.pathname === '/api/resource') {
        const data = await body(req);
        if ('viewer' in data || 'principal' in data) throw new ApiError(403, 'identity_fixed', '资源操作身份由启动者固定。');
        if (!kinds.includes(data.kind) || !['read', 'plan', 'delete'].includes(data.action)) throw new ApiError(400, 'invalid_arguments', '未知资源类型或操作。');
        if ('recursive' in data) throw new ApiError(400, 'invalid_arguments', 'Web 仅支持删除单个资源，请先解除引用。');
        const args = { kind: data.kind, reference: str(data.reference) };
        if (data.action !== 'read') args.recursive = false;
        if (data.action === 'delete') {
          if (!Array.isArray(data.confirm) || data.confirm.length !== 1 || typeof data.confirm[0] !== 'string') throw new ApiError(400, 'confirmation_required', '请先获取删除计划并确认。');
          args.confirm = data.confirm;
        }
        try { send(200, await tool(data.action === 'read' ? 'resource_read' : 'resource_delete', args)); }
        catch (error) {
          if (data.action !== 'read' && error.code === 'operation_rejected') throw new ApiError(400, error.code, '无法删除：请先解除资源引用或在运行设置中解除 Agent 的运行配置。默认 Prompt 不可删除；资源配置损坏或引用缺失也会阻止删除。刷新状态后重试。');
          throw error;
        }
      } else if (req.method === 'POST' && url.pathname === '/api/agent-entries') {
        const data = await body(req), agent = str(data.agent);
        if (data.confirm !== true || !['hide','show'].includes(data.action)) throw new ApiError(400,'invalid_arguments','请确认移除或恢复会话入口。');
        if (!Array.isArray(data.conversations) || !data.conversations.length || data.conversations.length > 100) throw new ApiError(400,'invalid_arguments','每次请选择 1–100 个会话。');
        const ids = [...new Set(data.conversations.map(str))];
        const principals = await tool('principal_list');
        if (!principals.some(p => p.id === agent && p.kind === 'agent')) throw new ApiError(403,'agent_required','仅可整理 Agent 的会话入口。');
        const results = [];
        for (const conversation of ids) {
          try {
            await tool(data.action === 'hide' ? 'conversation_hide' : 'conversation_show', {conversation}, agent);
            results.push({conversation,ok:true});
          } catch { results.push({conversation,ok:false,error:'无法整理该入口，请刷新后检查成员关系。'}); }
        }
        send(200,{results});
      } else if (req.method === 'POST' && url.pathname === '/api/conversation') {
        const data = await body(req); const conversation = str(data.conversation);
        if (!['hide', 'show', 'visit', 'leave', 'dissolve', 'delete', 'reactivate', 'reset'].includes(data.action)) throw new ApiError(400, 'invalid_arguments', '未知会话操作。');
        if (['delete', 'leave', 'dissolve', 'reset'].includes(data.action) && data.confirm !== true) throw new ApiError(400, 'confirmation_required', '请确认会话操作。');
        if (changingConversations.has(conversation)) throw new ApiError(409, 'conversation_busy', '会话操作进行中，请稍后重试。');
        changingConversations.add(conversation);
        try {
          await member(conversation);
          if ('viewer' in data || 'principal' in data || 'actor' in data) throw new ApiError(403, 'identity_fixed', '操作身份由启动者固定。');
          const commands = {
            reset: ['schedule', 'reset', conversation, '--actor', principal],
            hide: ['im', 'conversation', 'hide', conversation, '--actor', principal],
            show: ['im', 'conversation', 'unhide', conversation, '--actor', principal],
            visit: ['im', 'conversation', 'visit', conversation, '--actor', principal],
            leave: ['im', 'group', 'members', 'leave', conversation, principal],
            dissolve: ['schedule', 'dissolve', conversation, '--actor', principal],
            delete: ['schedule', 'delete', conversation, '--actor', principal],
            reactivate: ['im', 'conversation', 'reactivate', conversation, '--actor', principal],
          };
          const result = await run(commands[data.action]);
          send(200, result);
        } finally {
          changingConversations.delete(conversation);
        }
      } else if (req.method === 'POST' && url.pathname === '/api/reply') {
        const data = await body(req); const conversation = str(data.conversation);
        if (!['start', 'pause', 'end', 'resume'].includes(data.action)) throw new ApiError(400, 'invalid_arguments', '未知回复操作。');
        const members = await member(conversation);
        if (!members.some(m => m.principal_id === principal && m.left_message_seq === null)) throw new ApiError(403, 'not_member', '退出群聊后不能控制讨论。');
        const paid = ['start', 'resume'].includes(data.action);
        if (paid && (data.confirm !== true || (data.allow_mcp !== undefined && typeof data.allow_mcp !== 'boolean'))) throw new ApiError(400, 'confirmation_required', '请确认调用模型。');
        const scheduler = await run(['schedule', data.action, conversation, ...(paid && data.allow_mcp ? ['--allow-mcp'] : [])]);
        send(202, { scheduler });
      } else if (req.method === 'POST' && url.pathname === '/api/calls') {
        const data = await body(req);
        const after = data.after ?? 0;
        if (!Number.isSafeInteger(after) || after < 0) throw new ApiError(400, 'invalid_cursor', '调用游标无效。');
        send(200, await run(['run', 'calls', str(data.id), '--after', String(after), '--limit', '100']));
      } else throw new ApiError(404, 'not_found', '接口不存在。');
    } catch (error) {
      if (!res.headersSent) send(error.status ?? 500, { error: { code: error.code ?? 'internal_error', message: error instanceof ApiError ? error.message : '本地服务异常，请查看终端并刷新。', field: error.field } });
      else res.end();
    }
  });
  server.on('close', () => { for (const response of streams) response.destroy(); });
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
    const owner = await run(['im', 'owner', 'ensure', options.principal]);
    if (owner !== options.principal) throw new Error('此工作空间的负责人是 ' + owner + '，请使用 --principal 指定该身份启动。');
    await run.startScheduler();
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
