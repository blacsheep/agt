import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import vm from 'node:vm';

const web = join(dirname(fileURLToPath(import.meta.url)), '..');
const repo = join(web, '..');
const IDEOGRAPH = /[一-鿿]/;
const scripts = readdirSync(join(web, 'public')).filter(f => f.endsWith('.js') && f !== 'i18n.js');
const read = file => readFileSync(join(web, file), 'utf8');
const unquote = s => s.replace(/\\'/g, "'").replace(/\\n/g, '\n').replace(/\\\\/g, '\\');

// Loads i18n.js and a catalog in a sandbox with the given browser language.
function load(language, stored = null) {
  const context = {
    navigator: { language },
    localStorage: { getItem: () => stored, setItem() {} },
    document: { readyState: 'complete', documentElement: {}, getElementById: () => null, querySelectorAll: () => [] },
    location: { reload() {} },
  };
  context.window = context;
  vm.createContext(context);
  vm.runInContext(read('public/i18n.js'), context);
  vm.runInContext(read('public/locales/en.js'), context);
  return context;
}

// Every catalog key the console uses: $t() calls, marked HTML, server messages
// and the Rust strings the console shows (settings descriptions, tool labels).
function usedKeys() {
  const keys = new Set();
  for (const file of scripts) for (const m of read(`public/${file}`).matchAll(/\$t\(\s*'((?:[^'\\]|\\.)*)'/g)) keys.add(unquote(m[1]));
  const html = read('public/index.html');
  for (const m of html.matchAll(/<[a-z0-9]+[^>]*\sdata-i18n(?:="([^"]*)")?(?=[\s>])[^>]*>([^<]*)</g)) keys.add((m[1] || m[2]).trim());
  for (const m of html.matchAll(/<[^>]*data-i18n-attr="([^"]*)"[^>]*>/g))
    for (const attr of m[1].split(' ')) { const value = m[0].match(new RegExp(`\\s${attr}="([^"]*)"`)); if (value) keys.add(value[1]); }
  for (const m of read('server.mjs').matchAll(/ApiError\(\d+,\s*'[^']*',\s*'((?:[^'\\]|\\.)*)'/g)) keys.add(unquote(m[1]));
  for (const file of ['crates/agt-config/src/runtime.rs', 'crates/agt-runtime/src/audit.rs'])
    for (const m of readFileSync(join(repo, file), 'utf8').matchAll(/"((?:[^"\\]|\\.)*)"/g)) if (IDEOGRAPH.test(m[1])) keys.add(m[1]);
  return [...keys].filter(k => IDEOGRAPH.test(k));
}

test('every console text has an English translation', () => {
  const { AgtI18n } = load('en-US');
  const english = new Map(usedKeys().map(k => [k, AgtI18n.t(k)]));
  const missing = [...english].filter(([k, v]) => v === k).map(([k]) => k);
  assert.deepEqual(missing, [], `add these keys to web/public/locales/en.js:\n${missing.join('\n')}`);
});

test('console scripts keep Chinese text inside $t()', () => {
  const stray = [];
  for (const file of scripts.filter(f => !f.startsWith('locales'))) {
    const source = read(`public/${file}`)
      .replace(/\$t\(\s*'(?:[^'\\]|\\.)*'/g, '$t(KEY')
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/(^|[^:'"`\\])\/\/[^\n]*/g, '$1');
    source.split('\n').forEach((line, i) => { if (IDEOGRAPH.test(line)) stray.push(`${file}:${i + 1}: ${line.trim().slice(0, 120)}`); });
  }
  assert.deepEqual(stray, [], 'wrap this text in $t():\n' + stray.join('\n'));
});

test('index.html marks its Chinese text for translation', () => {
  const html = read('public/index.html')
    .replace(/<([a-z0-9]+)[^>]*\sdata-i18n(?:="[^"]*")?(?=[\s>])[^>]*>[^<]*</g, '<')
    .replace(/<[^>]*data-i18n-attr="[^"]*"[^>]*>/g, tag => tag.replace(/\s(title|aria-label|placeholder|alt)="[^"]*"/g, ''))
    .replace(/<select id="lang-select"[\s\S]*?<\/select>/, '');
  const stray = html.split('\n').filter(line => IDEOGRAPH.test(line)).map(line => line.trim().slice(0, 120));
  assert.deepEqual(stray, [], 'mark this text with data-i18n or data-i18n-attr:\n' + stray.join('\n'));
});

test('$t fills arguments and plural forms, and Chinese needs no catalog', () => {
  const en = load('en-US').AgtI18n;
  assert.equal(en.lang, 'en');
  assert.equal(en.t('{0} 条消息', [1]), '1 message');
  assert.equal(en.t('{0} 条消息', [3]), '3 messages');
  assert.equal(en.t('为「{0}」移除 {1} 个会话入口。', ['Lead', 2]), 'Remove 2 entries for "Lead".');
  en.add('en', { '{0} 条消息在 {1}': '{0, plural, one {# message in {1}} other {# messages in {1}}}' });
  assert.equal(en.t('{0} 条消息在 {1}', [2, '#general']), '2 messages in #general', 'arguments keep their own #');
  assert.equal(en.t('不存在的键'), '不存在的键');
  assert.deepEqual([...en.missing()], ['不存在的键']);

  const zh = load('zh-CN').AgtI18n;
  assert.equal(zh.lang, 'zh');
  assert.equal(zh.t('{0} 条消息', [3]), '3 条消息');
  assert.equal(zh.t('发送消息 ↑'), '发送消息 ↑');
  assert.equal(load('zh-CN', 'en').AgtI18n.lang, 'en', 'a stored choice wins over the browser language');
});
