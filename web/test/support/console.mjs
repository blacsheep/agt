// Test support: drive `agt web` (the Rust console) from Node tests.
//
// startConsole() runs `target/debug/agt web`. With a real cliRunner it uses
// that CLI and home directly. With a fake `run(args, options)` function, the
// console's CLI is fake-cli.mjs, which hands each call back to `run` here, so
// a test can script every CLI answer and record every command.
import http from 'node:http';
import { execFile, spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

export const agtBinary = fileURLToPath(new URL('../../../target/debug/agt', import.meta.url));
const fakeCli = fileURLToPath(new URL('./fake-cli.mjs', import.meta.url));

export class ApiError extends Error {
  constructor(status, code, message, field) { super(message); Object.assign(this, { status, code, field }); }
}

// The CLI as the console calls it, for preparing homes and for tests that
// talk to it directly.
export function cliRunner(binary, home) {
  const children = new Set();
  const run = (args, { env = {} } = {}) => new Promise((resolveResult, reject) => {
    const child = execFile(binary, ['--home', home, '--output', 'json', ...args], {
      env: { ...process.env, ...env }, timeout: 30_000, maxBuffer: 8 * 1024 * 1024, windowsHide: true,
    }, (error, stdout) => {
      children.delete(child);
      let value;
      try { value = JSON.parse(stdout); } catch { /* Never echo raw stderr/config data. */ }
      if (error) {
        const safe = value?.error;
        reject(new ApiError(400, typeof safe?.code === 'string' ? safe.code : 'cli_failed', typeof safe?.message === 'string' ? safe.message : 'cli failed', safe?.field));
      } else if (value === undefined) reject(new ApiError(502, 'invalid_response', 'invalid response'));
      else resolveResult(value);
    });
    children.add(child);
  });
  run.real = { binary, home };
  run.startScheduler = async () => {
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
    throw new Error('scheduler did not start');
  };
  run.watchEvents = ({ principal, after }, onPage, onEnd) => {
    const child = spawn(binary, ['--home', home, 'events', 'watch', '--principal', principal, ...(after ? ['--after', after] : [])], { stdio: ['ignore', 'pipe', 'ignore'], windowsHide: true });
    children.add(child);
    let buffer = '', ended = false;
    const finish = () => { if (ended) return; ended = true; children.delete(child); onEnd(); };
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => {
      buffer += chunk;
      let boundary;
      while ((boundary = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, boundary); buffer = buffer.slice(boundary + 1);
        try { onPage(JSON.parse(line)); } catch { child.kill('SIGTERM'); finish(); return; }
      }
    });
    child.once('error', finish); child.once('exit', finish);
    return () => { child.kill('SIGTERM'); finish(); };
  };
  run.close = () => Promise.all([...children].map(child => new Promise(done => {
    if (child.exitCode !== null || child.signalCode !== null) { done(); return; }
    child.once('exit', done); child.once('error', done);
    child.kill('SIGTERM');
  })));
  return run;
}

// Answers fake-cli.mjs: one POST per CLI call, or a JSON-lines stream for `events watch`.
async function bridge(run) {
  const server = http.createServer(async (req, res) => {
    let text = ''; for await (const chunk of req) text += chunk;
    const request = JSON.parse(text);
    if (req.url === '/watch') {
      if (!run.watchEvents) { res.writeHead(500); res.end(); return; }
      res.writeHead(200, { 'Content-Type': 'application/x-ndjson' });
      let stopped = false;
      const stop = run.watchEvents(request.after ? { principal: request.principal, after: request.after } : { principal: request.principal },
        page => res.write(JSON.stringify(page) + '\n'), () => res.end());
      // The console ends the subscription by stopping fake-cli, which closes this request.
      res.once('close', () => { if (!stopped) { stopped = true; stop?.(); } });
      return;
    }
    let reply;
    try { reply = { ok: true, value: await run(request.args, { env: request.env }) }; }
    catch (error) { reply = { ok: false, error: typeof error?.code === 'string' && error instanceof ApiError ? { code: error.code, message: error.message, field: error.field } : null }; }
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(reply));
  });
  await new Promise(ok => server.listen(0, '127.0.0.1', ok));
  return { url: `http://127.0.0.1:${server.address().port}`, close: () => { server.closeAllConnections(); server.close(); } };
}

// Starts the console as `principal`. Returns its base URL, token and close().
// It does not look for new releases unless `env` points it at a test server.
export async function startConsole({ run, principal = 'operator', token = randomBytes(32).toString('hex'), env = {} }) {
  let home = run?.real?.home, scratch, relay;
  const args = ['web', '--principal', principal, '--port', '0', '--token', token, '--no-setup'];
  if (!run?.real) {
    scratch = home = await mkdtemp(resolve(tmpdir(), 'agt-web-fake-'));
    relay = await bridge(run);
    args.push('--binary', fakeCli);
  }
  const child = spawn(agtBinary, ['--home', home, ...args], {
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, LANG: 'C', LC_ALL: 'C', AGT_NO_UPDATE_CHECK: '1', ...(relay ? { AGT_TEST_BRIDGE: relay.url } : {}), ...env },
  });
  const exited = new Promise(done => child.once('exit', done));
  const url = await new Promise((ok, fail) => {
    let output = '', errors = '';
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => { output += chunk; const match = output.match(/Open:\s+(http:\/\/127\.0\.0\.1:\d+)\//); if (match) ok(match[1]); });
    child.stderr.on('data', chunk => { errors += chunk; });
    child.once('exit', code => fail(new Error(`agt web exited (${code}): ${errors}`)));
  });
  return {
    url, token,
    close: async () => {
      child.kill('SIGTERM'); await exited;
      relay?.close();
      if (scratch) await rm(scratch, { recursive: true, force: true });
    },
  };
}
