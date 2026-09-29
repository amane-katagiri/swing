import { storage } from './storage.js';
import { t, currentLang } from './i18n.js';
import { cache, el, apiFetch, setStatus, clearStatus, describeError, createLoadGuard, setBusy } from './util.js';
import { loadStats } from './stats.js';
import { BrowserNotifySettings } from './settings-notify.js';

const settingsEls = {
  notWritableNotice: document.getElementById('settings-not-writable-notice'),
  restartNotice: document.getElementById('settings-restart-notice'),
  status: document.getElementById('settings-config-status'),
  content: document.getElementById('settings-config-content'),
  themeSelect: document.getElementById('theme-select'),
  langSelect: document.getElementById('lang-select'),
  userCssInput: document.getElementById('user-css-input'),
  userCssApply: document.getElementById('user-css-apply'),
  userCssReset: document.getElementById('user-css-reset'),
  processStop: document.getElementById('process-stop'),
  processRestart: document.getElementById('process-restart'),
  processStatus: document.getElementById('process-status'),
};

const sectionFields = new Map();

function formatRawConfigValue(v) {
  if (Array.isArray(v)) return v.length ? v.join(', ') : '–';
  if (v == null) return '–';
  return String(v);
}

function descriptionText(item) {
  if (!item.description) return '';
  const lang = currentLang();
  return item.description[lang] || item.description.en || '';
}

function appendDescriptionHint(td, item) {
  const text = descriptionText(item);
  if (text) td.append(el('div', { class: 'swing-hint' }, text));
}

function appendReadOnlyValue(td, item) {
  if (item.display != null) {
    td.append(document.createTextNode(`${item.display} `), el('span', { class: 'swing-hint swing-mono' }, formatRawConfigValue(item.value)));
  } else {
    td.append(document.createTextNode(formatRawConfigValue(item.value)));
  }
  // raw, not kind, marks the whitelist: kind is now on every catalog entry
  if (item.source === 'env' && item.raw != null) {
    td.append(el('span', { class: 'swing-hint' }, ` — ${t('configLockedByEnv')}`));
  }
}

function buildBoolControl(item) {
  const select = el('select', { class: 'swing-input' }, [el('option', { value: 'true' }, 'true'), el('option', { value: 'false' }, 'false')]);
  select.value = String(item.raw) === 'true' ? 'true' : 'false';
  return { input: select, getValue: () => select.value };
}

function buildModeControl(item) {
  const select = el(
    'select',
    { class: 'swing-input' },
    (item.options || []).map((opt) => el('option', { value: opt }, opt)),
  );
  select.value = item.raw != null ? String(item.raw) : '';
  return { input: select, getValue: () => select.value };
}

function buildListControl(item) {
  const textarea = el('textarea', { class: 'swing-input', rows: '4' });
  textarea.value = Array.isArray(item.raw) ? item.raw.join('\n') : '';
  return {
    input: textarea,
    getValue: () =>
      textarea.value
        .split('\n')
        .map((s) => s.trim())
        .filter((s) => s.length > 0),
  };
}

function buildTextControl(item) {
  const input = el('input', { type: 'text', class: 'swing-input' });
  input.value = item.raw != null ? String(item.raw) : '';
  return { input, getValue: () => input.value.trim() };
}

function buildEditableControl(item) {
  if (item.kind === 'bool') return buildBoolControl(item);
  if (item.kind === 'mode') return buildModeControl(item);
  if (item.kind === 'list') return buildListControl(item);
  return buildTextControl(item);
}

function rawEquals(a, b, kind) {
  if (kind === 'list') {
    const x = Array.isArray(a) ? a : [];
    const y = Array.isArray(b) ? b : [];
    return x.length === y.length && x.every((v, i) => v === y[i]);
  }
  return String(a) === String(b);
}

function sectionStatusId(name) {
  return `settings-section-status-${name}`;
}

function setSectionDisabled(container, disabled) {
  container.querySelectorAll('input, select, textarea, button').forEach((field) => {
    field.disabled = disabled;
  });
}

async function saveSection(name, container, saveBtn) {
  const fields = sectionFields.get(name) || [];
  const items = {};
  for (const f of fields) {
    const value = f.getValue();
    if (!rawEquals(value, f.raw, f.kind)) items[`${name}.${f.key}`] = value;
  }
  const statusEl = document.getElementById(sectionStatusId(name));
  if (Object.keys(items).length === 0) {
    if (statusEl) clearStatus(statusEl);
    return;
  }
  setSectionDisabled(container, true);
  setBusy(saveBtn, true);
  if (statusEl) clearStatus(statusEl);
  try {
    const data = await apiFetch('/api/config', { method: 'PUT', body: JSON.stringify({ items }) });
    cache.config = data;
    renderConfig(data);
    const newStatusEl = document.getElementById(sectionStatusId(name));
    if (newStatusEl) setStatus(newStatusEl, 'ok', t('configSaved'));
  } catch (err) {
    setSectionDisabled(container, false);
    setBusy(saveBtn, false);
    if (statusEl) setStatus(statusEl, 'error', describeError(err));
  }
}

function buildSection(section, canEdit) {
  const sec = el('div', { class: 'swing-config-section' }, el('h3', {}, section.name));
  const table = el('table', { class: 'swing-table' });
  const headers = [t('tableKey'), t('tableValue'), t('tableEnv')];
  table.append(el('thead', {}, el('tr', {}, headers.map((h) => el('th', {}, h)))));
  const tbody = el('tbody');
  const fields = [];
  for (const item of section.items) {
    const tr = el('tr');
    tr.append(el('td', { class: 'swing-mono' }, item.key || '–'));
    const valueTd = el('td');
    if (item.editable && canEdit) {
      const control = buildEditableControl(item);
      valueTd.append(control.input);
      fields.push({ key: item.key, kind: item.kind, raw: item.raw, getValue: control.getValue });
    } else {
      appendReadOnlyValue(valueTd, item);
    }
    if (item.raw != null) appendDescriptionHint(valueTd, item);
    tr.append(valueTd);
    tr.append(el('td', { class: 'swing-mono' }, item.env || '–'));
    tbody.append(tr);
  }
  table.append(tbody);
  sec.append(table);

  if (fields.length) {
    sectionFields.set(section.name, fields);
    const saveBtn = el('button', { type: 'button', class: 'swing-btn swing-btn-accent' }, t('configSaveBtn'));
    saveBtn.addEventListener('click', () => saveSection(section.name, sec, saveBtn));
    sec.append(el('div', { class: 'swing-config-actions' }, saveBtn));
    sec.append(el('div', { class: 'swing-status', id: sectionStatusId(section.name), 'aria-live': 'polite' }));
  }
  return sec;
}

export function renderConfig(config) {
  sectionFields.clear();
  settingsEls.content.replaceChildren();
  const canEdit = config.writable !== false;
  const pathText = config.config_path ? t('configFile', { path: config.config_path }) : t('configEnvOnly');
  const willBeCreated = config.config_path && config.config_exists === false ? t('configWillBeCreated') : '';
  settingsEls.content.append(el('p', { class: 'swing-hint' }, `${pathText}${willBeCreated}`));
  for (const section of config.sections) {
    settingsEls.content.append(buildSection(section, canEdit));
  }
  if (!canEdit) setStatus(settingsEls.notWritableNotice, 'warn', t('configNotWritable'));
  else clearStatus(settingsEls.notWritableNotice);
  if (config.restart_required) setStatus(settingsEls.restartNotice, 'warn', t('configRestartRequiredNotice'));
  else clearStatus(settingsEls.restartNotice);
}

function applyTheme(theme) {
  const root = document.documentElement;
  if (theme === 'light' || theme === 'dark') root.setAttribute('data-theme', theme);
  else root.removeAttribute('data-theme');
}

const settingsLoadGuard = createLoadGuard();

export const SettingsView = {
  init({ updates }) {
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

    BrowserNotifySettings.init({ updates });

    async function runProcessAction(button, confirmKey, path, okKey) {
      if (!window.confirm(t(confirmKey))) return;
      clearStatus(settingsEls.processStatus);
      setBusy(button, true);
      try {
        await apiFetch(path, { method: 'POST' });
        setStatus(settingsEls.processStatus, 'ok', t(okKey));
      } catch (err) {
        setStatus(settingsEls.processStatus, 'error', describeError(err));
      } finally {
        setBusy(button, false);
      }
    }
    settingsEls.processStop.addEventListener('click', () =>
      runProcessAction(settingsEls.processStop, 'processStopConfirm', '/api/shutdown', 'processStopResult'));
    settingsEls.processRestart.addEventListener('click', () =>
      runProcessAction(settingsEls.processRestart, 'processRestartConfirm', '/api/restart', 'processRestartResult'));
  },
  onShow() {
    if (cache.config) renderConfig(cache.config);
    else this.load();
    BrowserNotifySettings.onShow();
    loadStats();
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
