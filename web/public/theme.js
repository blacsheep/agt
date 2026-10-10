// Loaded synchronously in <head> so the theme is set before first paint (no light flash in dark mode).
(function () {
  const media = window.matchMedia('(prefers-color-scheme: dark)');
  const preference = () => {
    try { return localStorage.getItem('agt-theme') || 'system'; } catch { return 'system'; }
  };
  const apply = () => {
    const pref = preference();
    document.documentElement.dataset.theme = pref === 'system' ? (media.matches ? 'dark' : 'light') : pref;
    document.documentElement.dataset.themePreference = pref;
  };
  apply();
  media.addEventListener('change', apply);
  window.AgtTheme = {
    get: preference,
    set(pref) {
      try { localStorage.setItem('agt-theme', pref); } catch { /* private mode: keep for this page only */ }
      apply();
    },
  };
})();
