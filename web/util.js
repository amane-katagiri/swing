import { storage } from './storage.js';
import { t, currentLang } from './i18n.js';

export const DEFAULT_STYLES = { sites: 'cards', webring: 'graph' };

export const cache = {
  overview: null,
  sites: null,
  status: null,
  mirror: null,
  webringByQuery: new Map(),
  replicasByKey: new Map(),
  config: null,
  publishSites: null,
};

export function createLoadGuard() {
  let gen = 0;
  return {
    start() {
      return ++gen;
    },
    isCurrent(g) {
      return g === gen;
    },
  };
}

export function el(tag, attrs, children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') node.className = v;
    else if (k.startsWith('on') && typeof v === 'function') node.addEventListener(k.slice(2), v);
    else if (v === true) node.setAttribute(k, '');
    else node.setAttribute(k, v);
  }
  for (const c of [].concat(children == null ? [] : children)) {
    if (c == null) continue;
    node.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return node;
}

export function clamp(v, lo, hi) {
  return Math.min(hi, Math.max(lo, v));
}

export function setStatus(container, kind, message) {
  container.dataset.kind = kind;
  container.textContent = message || '';
}

export function clearStatus(container) {
  container.removeAttribute('data-kind');
  container.textContent = '';
}

export function describeError(err) {
  if (!err) return t('unknownError');
  if (err.status === 0) return err.message || t('unreachable');
  if (err.status) return `${err.message} (HTTP ${err.status})`;
  return err.message || String(err);
}

export function formatBytes(n) {
  if (n == null) return '–';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${i === 0 ? v : v.toFixed(1)} ${units[i]}`;
}

export function formatSiteSize(site) {
  if (site.stored_size != null) return formatBytes(site.stored_size);
  if (site.size != null) return `(${formatBytes(site.size)})`;
  return '–';
}

export function formatTime(sec) {
  if (sec == null) return '–';
  const lang = currentLang();
  try {
    return new Intl.DateTimeFormat(lang === 'ja' ? 'ja-JP' : 'en-US', {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(sec * 1000));
  } catch {
    return new Date(sec * 1000).toLocaleString();
  }
}

export function stripControlChars(str) {
  return String(str).replace(/[\x00-\x1f\x7f]+/g, ' ').trim();
}

export function sanitizeMessage(str, max) {
  if (!str) return null;
  const limit = max || 200;
  const cleaned = stripControlChars(str);
  if (!cleaned) return null;
  return cleaned.length > limit ? `${cleaned.slice(0, limit)}…` : cleaned;
}

export function shortenMiddle(str, head, tail) {
  const h = head || 10;
  const t2 = tail || 6;
  if (!str || str.length <= h + t2 + 1) return str || '';
  return `${str.slice(0, h)}…${str.slice(-t2)}`;
}

export function maybeLink(url, text) {
  if (typeof url === 'string' && /^https?:\/\//i.test(url)) {
    return el('a', { href: url, target: '_blank', rel: 'noopener noreferrer' }, text || url);
  }
  return el('span', {}, text || url || '');
}

export function ensureBusyStructure(button) {
  if (button.querySelector('.swing-btn-label')) return;
  const label = el('span', { class: 'swing-btn-label' });
  while (button.firstChild) label.append(button.firstChild);
  const spinner = el('span', { class: 'swing-btn-spinner', 'aria-hidden': 'true' });
  button.append(label, spinner);
}

export function setBusy(button, busy) {
  if (!button) return;
  ensureBusyStructure(button);
  button.disabled = busy;
  if (busy) button.setAttribute('aria-busy', 'true');
  else button.removeAttribute('aria-busy');
  button.classList.toggle('is-busy', busy);
}

export async function copyWithFeedback(button, text) {
  ensureBusyStructure(button);
  const label = button.querySelector('.swing-btn-label');
  const original = label.textContent;
  clearTimeout(button._swingCopyTimer);
  let ok = true;
  try {
    await navigator.clipboard.writeText(text || '');
  } catch {
    ok = false;
  }
  label.textContent = ok ? t('copied') : t('copyFailed');
  button.dataset.copied = String(ok);
  button.dataset.copyFailed = String(!ok);
  button._swingCopyTimer = setTimeout(() => {
    label.textContent = original;
    delete button.dataset.copied;
    delete button.dataset.copyFailed;
  }, 1500);
}

export async function apiFetch(path, opts) {
  const options = opts || {};
  const method = (options.method || 'GET').toUpperCase();
  const headers = Object.assign({}, options.headers);
  if (method !== 'GET' && method !== 'HEAD') {
    headers['X-Swing-Dashboard'] = '1';
    headers['Content-Type'] = 'application/json';
  }
  let res;
  try {
    res = await fetch(path, Object.assign({}, options, { headers }));
  } catch {
    const err = new Error(t('unreachable'));
    err.status = 0;
    throw err;
  }
  const text = await res.text();
  let body = null;
  if (text) {
    try {
      body = JSON.parse(text);
    } catch {
      body = null;
    }
  }
  if (!res.ok) {
    const message = body && typeof body.error === 'string' ? body.error : `HTTP ${res.status}`;
    const err = new Error(message);
    err.status = res.status;
    err.body = body;
    throw err;
  }
  return body;
}

export function getStyle(view) {
  if (!(view in DEFAULT_STYLES)) return null;
  return storage.get(`swing:style:${view}`, DEFAULT_STYLES[view]);
}

export function setStyle(view, value) {
  storage.set(`swing:style:${view}`, value);
}

function wireButtonGroup(group, { valueAttr, get, set, onChange }) {
  const current = get();
  const buttons = group.querySelectorAll('button');
  for (const btn of buttons) {
    btn.setAttribute('aria-pressed', String(btn.dataset[valueAttr] === current));
    btn.addEventListener('click', () => {
      set(btn.dataset[valueAttr]);
      for (const b of buttons) b.setAttribute('aria-pressed', String(b === btn));
      onChange(btn.dataset[valueAttr]);
    });
  }
}

export function wireStyleSwitch(view, onChange) {
  const group = document.querySelector(`.swing-style-switch[data-target="${view}"]`);
  wireButtonGroup(group, {
    valueAttr: 'styleValue',
    get: () => getStyle(view),
    set: (v) => setStyle(view, v),
    onChange: (v) => {
      if (document.body.dataset.view === view) document.body.dataset.style = v;
      onChange(v);
    },
  });
}

export function wireSortSwitch(selector, storageKey, defaultValue, onChange) {
  const group = document.querySelector(selector);
  wireButtonGroup(group, {
    valueAttr: 'sortValue',
    get: () => storage.get(storageKey, defaultValue),
    set: (v) => storage.set(storageKey, v),
    onChange,
  });
}

export function setFormDisabled(form, disabled) {
  for (const field of form.elements) field.disabled = disabled;
}
