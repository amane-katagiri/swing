import { storage } from './storage.js';
import { t } from './i18n.js';
import { cache, el, apiFetch, setStatus, clearStatus, describeError, createLoadGuard } from './util.js';

const settingsEls = {
  status: document.getElementById('settings-config-status'),
  content: document.getElementById('settings-config-content'),
  themeSelect: document.getElementById('theme-select'),
  langSelect: document.getElementById('lang-select'),
  userCssInput: document.getElementById('user-css-input'),
  userCssApply: document.getElementById('user-css-apply'),
  userCssReset: document.getElementById('user-css-reset'),
};

function formatRawConfigValue(v) {
  if (Array.isArray(v)) return v.length ? v.join(', ') : '–';
  if (v == null) return '–';
  return String(v);
}

function buildConfigValueCell(item) {
  if (item.display != null) {
    return el('span', {}, [
      document.createTextNode(`${item.display} `),
      el('span', { class: 'swing-hint swing-mono' }, formatRawConfigValue(item.value)),
    ]);
  }
  return document.createTextNode(formatRawConfigValue(item.value));
}

export function renderConfig(config) {
  settingsEls.content.replaceChildren();
  settingsEls.content.append(
    el('p', { class: 'swing-hint' }, config.config_path ? t('configFile', { path: config.config_path }) : t('configEnvOnly')),
  );
  for (const section of config.sections) {
    const sec = el('div', { class: 'swing-config-section' }, el('h3', {}, section.name));
    const table = el('table', { class: 'swing-table' });
    const headers = [t('tableKey'), t('tableValue'), t('tableEnv')];
    table.append(el('thead', {}, el('tr', {}, headers.map((h) => el('th', {}, h)))));
    const tbody = el('tbody');
    for (const item of section.items) {
      const tr = el('tr');
      tr.append(el('td', { class: 'swing-mono' }, item.key || '–'));
      tr.append(el('td', {}, buildConfigValueCell(item)));
      tr.append(el('td', { class: 'swing-mono' }, item.env || '–'));
      tbody.append(tr);
    }
    table.append(tbody);
    sec.append(table);
    settingsEls.content.append(sec);
  }
}

function applyTheme(theme) {
  const root = document.documentElement;
  if (theme === 'light' || theme === 'dark') root.setAttribute('data-theme', theme);
  else root.removeAttribute('data-theme');
}

const settingsLoadGuard = createLoadGuard();

export const SettingsView = {
  init() {
    const savedTheme = storage.get('swing:theme', 'auto');
    applyTheme(savedTheme);
    settingsEls.themeSelect.value = savedTheme;
    settingsEls.themeSelect.addEventListener('change', () => {
      storage.set('swing:theme', settingsEls.themeSelect.value);
      applyTheme(settingsEls.themeSelect.value);
    });

    settingsEls.langSelect.value = storage.get('swing:lang', 'auto');
    settingsEls.langSelect.addEventListener('change', () => {
      storage.set('swing:lang', settingsEls.langSelect.value);
      document.dispatchEvent(new CustomEvent('swing:langchange'));
    });

    const userCssEl = document.getElementById('user-css');
    const savedCss = storage.get('swing:user-css', '');
    userCssEl.textContent = savedCss;
    settingsEls.userCssInput.value = savedCss;
    settingsEls.userCssApply.addEventListener('click', () => {
      storage.set('swing:user-css', settingsEls.userCssInput.value);
      userCssEl.textContent = settingsEls.userCssInput.value;
    });
    settingsEls.userCssReset.addEventListener('click', () => {
      settingsEls.userCssInput.value = '';
      storage.remove('swing:user-css');
      userCssEl.textContent = '';
    });
  },
  onShow() {
    if (cache.config) renderConfig(cache.config);
    else this.load();
  },
  async load(force) {
    const gen = settingsLoadGuard.start();
    setStatus(settingsEls.status, 'loading', t('loadingConfig'));
    try {
      const data = force || !cache.config ? await apiFetch('/api/config') : cache.config;
      if (!settingsLoadGuard.isCurrent(gen)) return;
      cache.config = data;
      clearStatus(settingsEls.status);
      renderConfig(cache.config);
    } catch (err) {
      if (!settingsLoadGuard.isCurrent(gen)) return;
      setStatus(settingsEls.status, 'error', describeError(err));
    }
  },
};
