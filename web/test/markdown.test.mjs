import test from 'node:test';
import assert from 'node:assert/strict';
import '../public/markdown.js';
const { render } = globalThis.AgtMarkdown;

test('markdown escapes raw HTML before converting', () => {
  assert.equal(render('<img src=x onerror=alert(1)>'), '<p>&lt;img src=x onerror=alert(1)&gt;</p>');
  assert.equal(render('**<b>x</b>**'), '<p><strong>&lt;b&gt;x&lt;/b&gt;</strong></p>');
});

test('links allow only http(s) and cannot break out of attributes', () => {
  assert.equal(render('[a](javascript:alert(1))'), '<p>[a](javascript:alert(1))</p>');
  assert.match(render('[文档](https://example.com/a?x=1&y=2)'), /<a href="https:\/\/example\.com\/a\?x=1&amp;y=2" target="_blank" rel="noopener noreferrer">文档<\/a>/);
  assert.equal(render('https://e.com/"onmouseover="x'), '<p><a href="https://e.com/&quot;onmouseover=&quot;x" target="_blank" rel="noopener noreferrer">https://e.com/&quot;onmouseover=&quot;x</a></p>');
  assert.doesNotMatch(render('see https://e.com/*a*b'), /<em>/);
});

test('blocks: code, lists, quotes, headings', () => {
  assert.equal(render('```js\nconst a = 1 < 2;\n**x**\n```'), '<pre><code data-lang="js">const a = 1 &lt; 2;\n**x**</code></pre>');
  assert.equal(render('- 一\n- `二`\n\n1. a\n2. b'), '<ul><li>一</li><li><code>二</code></li></ul><ol><li>a</li><li>b</li></ol>');
  assert.equal(render('> 引用\n正文'), '<blockquote>引用</blockquote><p>正文</p>');
  assert.equal(render('## 标题\n第一行\n第二行'), '<p class="md-heading">标题</p><p>第一行<br>第二行</p>');
});

test('plain chat text keeps line breaks and stray symbols', () => {
  assert.equal(render('2*3*4 = 24\n价格 $5'), '<p>2<em>3</em>4 = 24<br>价格 $5</p>');
});
