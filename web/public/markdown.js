// Minimal Markdown for chat bubbles. All text is HTML-escaped first; only a small, known set of
// constructs is converted afterwards, so model output cannot inject markup. No inline styles (CSP).
(function (root) {
  const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

  function inline(text) {
    // text is already escaped. Code spans and links become placeholders first so emphasis rules
    // never touch their contents or attribute values.
    const held = [];
    const hold = html => `\u0000${held.push(html) - 1}\u0000`;
    const link = (url, label) => hold(`<a href="${url}" target="_blank" rel="noopener noreferrer">${label}</a>`);
    let out = text
      .replace(/`([^`\n]+)`/g, (_, code) => hold(`<code>${code}</code>`))
      .replace(/\[([^\]\n]+)\]\((https?:\/\/[^\s)]+)\)/g, (_, label, url) => link(url, label))
      .replace(/(^|[\s(（])(https?:\/\/[^\s<)）\u0000]+)/g, (_, lead, url) => lead + link(url, url));
    out = out
      .replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>')
      .replace(/(^|[^*])\*([^*\s][^*\n]*?)\*(?!\*)/g, '$1<em>$2</em>')
      .replace(/~~([^~\n]+)~~/g, '<del>$1</del>');
    return out.replace(/\u0000(\d+)\u0000/g, (_, i) => held[i]);
  }

  function render(source) {
    const lines = esc(String(source ?? '').replace(/\u0000/g, '')).replace(/\r\n?/g, '\n').split('\n');
    const html = [];
    let paragraph = [], list = null, quote = [];
    const flushParagraph = () => { if (paragraph.length) html.push(`<p>${paragraph.map(inline).join('<br>')}</p>`); paragraph = []; };
    const flushList = () => { if (list) html.push(`<${list.tag}>${list.items.map(item => `<li>${inline(item)}</li>`).join('')}</${list.tag}>`); list = null; };
    const flushQuote = () => { if (quote.length) html.push(`<blockquote>${quote.map(inline).join('<br>')}</blockquote>`); quote = []; };
    const flushAll = () => { flushParagraph(); flushList(); flushQuote(); };
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      const fence = line.match(/^\s*```\s*([\w+-]*)\s*$/);
      if (fence) {
        flushAll();
        const body = [];
        while (++i < lines.length && !/^\s*```\s*$/.test(lines[i])) body.push(lines[i]);
        html.push(`<pre><code${fence[1] ? ` data-lang="${fence[1]}"` : ''}>${body.join('\n')}</code></pre>`);
        continue;
      }
      const heading = line.match(/^(#{1,4})\s+(.*)$/);
      const bullet = line.match(/^\s*[-*+]\s+(.*)$/);
      const ordered = line.match(/^\s*\d+[.)]\s+(.*)$/);
      const quoted = line.match(/^&gt;\s?(.*)$/);
      if (!line.trim()) { flushAll(); continue; }
      if (heading) { flushAll(); html.push(`<p class="md-heading">${inline(heading[2])}</p>`); continue; }
      if (/^\s*([-*_])\s*\1\s*\1[\s\-*_]*$/.test(line)) { flushAll(); html.push('<hr>'); continue; }
      if (bullet || ordered) {
        flushParagraph(); flushQuote();
        const tag = bullet ? 'ul' : 'ol';
        if (list && list.tag !== tag) flushList();
        if (!list) list = { tag, items: [] };
        list.items.push((bullet || ordered)[1]);
        continue;
      }
      if (quoted) { flushParagraph(); flushList(); quote.push(quoted[1]); continue; }
      if (list && /^\s{2,}\S/.test(line)) { list.items[list.items.length - 1] += ' ' + line.trim(); continue; }
      flushList(); flushQuote();
      paragraph.push(line);
    }
    flushAll();
    return html.join('');
  }

  root.AgtMarkdown = { render };
})(typeof window !== 'undefined' ? window : globalThis);
