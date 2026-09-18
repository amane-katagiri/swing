try {
  if (localStorage.getItem('swing:nav:collapsed') === '1') document.body.dataset.nav = 'collapsed';
} catch {}
try {
  let lang = localStorage.getItem('swing:lang');
  if (lang !== 'en' && lang !== 'ja') lang = (navigator.language || '').toLowerCase().startsWith('ja') ? 'ja' : 'en';
  if (lang !== 'en') {
    document.documentElement.lang = lang;
    document.documentElement.dataset.i18nPending = '';
  }
} catch {}
