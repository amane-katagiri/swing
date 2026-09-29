import { storage } from './storage.js';
import { t, currentLang } from './i18n.js';

const DEFAULT_STYLES = { sites: 'cards', webring: 'graph' };

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

export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

const POLL_ATTEMPTS = 120;
const POLL_INTERVAL_MS = 1000;

export async function pollUntil(predicate) {
  for (let attempt = 0; attempt < POLL_ATTEMPTS; attempt += 1) {
    await sleep(POLL_INTERVAL_MS);
    try {
      if (await predicate()) return true;
    } catch {}
  }
  return false;
}

function randomNonce() {
  const bytes = crypto.getRandomValues(new Uint8Array(32));
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

// Whoever holds the port while swing is down would receive the session cookie, so probes go out without it.
export async function fetchInstance() {
  const res = await fetch('/api/identity', {
    method: 'POST',
    credentials: 'omit',
    headers: { 'X-Swing-Dashboard': '1', 'Content-Type': 'application/json' },
    body: JSON.stringify({ nonce: randomNonce() }),
  });
  if (!res.ok) return null;
  const body = parseApiBody(await res.text());
  return body && typeof body.instance === 'string' ? body.instance : null;
}

export async function dashboardAnswers() {
  try {
    return (await fetchInstance()) != null;
  } catch {
    return false;
  }
}

const MAX_BACKOFF_MS = 10 * 60 * 1000;

export function backoffDelay(intervalMs, failures) {
  return Math.max(intervalMs, Math.min(intervalMs * 2 ** Math.min(failures, 10), MAX_BACKOFF_MS));
}

export function waitForNewInstance(previous) {
  return pollUntil(async () => {
    const instance = await fetchInstance();
    return instance != null && instance !== previous;
  });
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
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let v = n;
  let i = 0;
  while ((i === 0 ? v >= 1024 : v >= 1023.95) && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${i === 0 ? v : v.toFixed(1).replace(/\.0$/, '')} ${units[i]}`;
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

function stripControlChars(str) {
  return String(str).replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029]+/g, ' ').trim();
}

// Other people's event text could use these to visually reorder or hide itself.
const UNSAFE_UNICODE_RE =
  /[\u00ad\u061c\u180e\u200b-\u200f\u202a-\u202e\u2060-\u2069\ufeff\ufff9-\ufffb\u{e0000}-\u{e007f}]/gu;

export function stripUnsafeUnicode(str) {
  return String(str == null ? '' : str).replace(UNSAFE_UNICODE_RE, '');
}

export function sanitizeDisplayText(str, max) {
  if (str == null) return '';
  const cleaned = stripUnsafeUnicode(stripControlChars(str));
  if (!max) return cleaned;
  return cleaned.length > max ? `${cleaned.slice(0, max)}…` : cleaned;
}

export function sanitizeMessage(str, max) {
  if (!str) return null;
  const cleaned = sanitizeDisplayText(str, max || 200);
  return cleaned || null;
}

export function siteTitle(site, max) {
  return sanitizeMessage(site.title, max) || sanitizeDisplayText(site.d, max);
}

export function shortenMiddle(str, head, tail) {
  const h = head || 10;
  const t2 = tail || 6;
  if (!str || str.length <= h + t2 + 1) return str || '';
  return `${str.slice(0, h)}…${str.slice(-t2)}`;
}

export function isHttpUrl(url) {
  return typeof url === 'string' && /^https?:\/\//i.test(url);
}

export function maybeLink(url, text) {
  if (isHttpUrl(url)) {
    return el('a', { href: url, target: '_blank', rel: 'noopener noreferrer' }, text || sanitizeDisplayText(url));
  }
  return el('span', {}, text || sanitizeDisplayText(url) || '');
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
  const headers = Object.assign({ 'X-Swing-Dashboard': '1' }, options.headers);
  if (method !== 'GET' && method !== 'HEAD') {
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
  const body = parseApiBody(await res.text());
  const err = apiResponseError(path, res.status, body);
  if (err) throw err;
  return body;
}

export function parseApiBody(text) {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

export function apiResponseError(path, status, body) {
  if (status === 401 && path !== '/api/login') {
    document.dispatchEvent(new CustomEvent('swing:unauthorized'));
  }
  if (status >= 200 && status < 300) return null;
  const message = body && typeof body.error === 'string' ? sanitizeDisplayText(body.error) : `HTTP ${status}`;
  const err = new Error(message);
  err.status = status;
  err.body = body;
  return err;
}

export function getStyle(view) {
  if (!(view in DEFAULT_STYLES)) return null;
  return storage.get(`swing:style:${view}`, DEFAULT_STYLES[view]);
}

function setStyle(view, value) {
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
