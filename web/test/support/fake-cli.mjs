#!/usr/bin/env node
// Stands in for `agt` when a test scripts the CLI: forwards each call to the
// test process (AGT_TEST_BRIDGE) and prints its answer the way `agt` would.
const bridge = process.env.AGT_TEST_BRIDGE;
let args = process.argv.slice(2);
if (args[0] === '--home') args = args.slice(2);
if (args[0] === '--output' && args[1] === 'json') args = args.slice(2);
const post = (path, body) => fetch(`${bridge}${path}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });

if (args[0] === 'events' && args[1] === 'watch') {
  const option = name => { const i = args.indexOf(name); return i >= 0 ? args[i + 1] : undefined; };
  const response = await post('/watch', { principal: option('--principal'), after: option('--after') });
  for await (const chunk of response.body) process.stdout.write(chunk);
} else {
  // Only the credential the console hands to this one child, never the whole environment.
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => key.startsWith('AGT_WEB_CREDENTIAL_')));
  const reply = await (await post('/call', { args, env })).json();
  if (reply.ok) process.stdout.write(JSON.stringify(reply.value) ?? '');
  else {
    if (reply.error) process.stdout.write(JSON.stringify({ error: reply.error }));
    process.exitCode = 1;
  }
}
