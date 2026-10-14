/* Console language. Code wraps every piece of console text in $t(); the key is
 * the Chinese source text (gettext style), so Chinese needs no catalog and
 * other languages map that text to a translation in locales/<lang>.js.
 * Static HTML marks text with data-i18n and attributes with data-i18n-attr.
 *
 * Messages use a small ICU subset: {0} inserts an argument, and
 * {0, plural, one {# item} other {# items}} picks a form, with # for the number. */
(() => {
  const STORAGE = 'agt-lang';
  const stored = (() => { try { return localStorage.getItem(STORAGE); } catch { return null; } })();
  const lang = stored === 'zh' || stored === 'en' ? stored : /^zh\b/i.test(navigator.language || '') ? 'zh' : 'en';
  const catalogs = {};
  const missing = new Set();

  // Finds the matching closing brace, so nested plural forms work.
  function closing(text, open) {
    for (let depth = 0, i = open; i < text.length; i++) {
      if (text[i] === '{') depth++;
      else if (text[i] === '}' && --depth === 0) return i;
    }
    return -1;
  }
  function format(message, args) {
    let out = '';
    for (let i = 0; i < message.length;) {
      const open = message.indexOf('{', i);
      if (open < 0) { out += message.slice(i); break; }
      const end = closing(message, open);
      if (end < 0) { out += message.slice(i); break; }
      out += message.slice(i, open);
      const [name, kind, ...rest] = message.slice(open + 1, end).split(',');
      const value = args[name.trim()];
      if (kind?.trim() === 'plural') {
        const forms = rest.join(',');
        const pick = form => { const at = forms.search(new RegExp(`(^|\\s)${form}\\s*\\{`)); return at < 0 ? null : forms.slice(forms.indexOf('{', at) + 1, closing(forms, forms.indexOf('{', at))); };
        const form = (Number(value) === 1 ? pick('one') : null) ?? pick('other') ?? '';
        out += format(form.replace(/#/g, String(value)), args);
      } else {
        out += value === undefined ? message.slice(open, end + 1) : String(value);
      }
      i = end + 1;
    }
    return out;
  }
  function t(key, args) {
    let message = key;
    if (lang !== 'zh') {
      message = catalogs[lang]?.[key];
      if (message === undefined) { if (/[\u4e00-\u9fff]/.test(key)) missing.add(key); message = key; }
    }
    return args ? format(message, args) : message;
  }
  function apply(root = document) {
    for (const el of root.querySelectorAll('[data-i18n]')) el.textContent = t(el.dataset.i18n || el.textContent.trim());
    for (const el of root.querySelectorAll('[data-i18n-attr]')) {
      for (const attr of el.dataset.i18nAttr.split(/\s+/)) {
        const value = el.getAttribute(attr);
        if (value) el.setAttribute(attr, t(value));
      }
    }
  }

  window.AgtI18n = {
    lang,
    /** Locale for dates and numbers. */
    locale: lang === 'zh' ? 'zh-CN' : 'en-US',
    /** Registers translations for a language: { 'Chinese source text': 'translation' }. */
    add(language, table) { Object.assign(catalogs[language] ||= {}, table); },
    t,
    apply,
    set(next) { try { localStorage.setItem(STORAGE, next); } catch { /* private mode: this page only */ } location.reload(); },
    /** Keys looked up in this page that have no translation. */
    missing: () => [...missing],
  };
  window.$t = t;

  const start = () => {
    document.documentElement.lang = AgtI18n.locale;
    if (lang !== 'zh') apply();
    const picker = document.getElementById('lang-select');
    if (picker) { picker.value = lang; picker.onchange = () => AgtI18n.set(picker.value); }
  };
  // 'interactive' comes after parsing but before deferred scripts run, so
  // static text is translated before the app starts changing it.
  if (document.readyState === 'loading') document.addEventListener('readystatechange', start, { once: true }); else start();
})();
