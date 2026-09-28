import { storage } from './storage.js';

export const NOTIFY_SETTINGS_KEY = 'swing:desktop:notify';
export const MASCOT_SETTINGS_KEY = 'swing:desktop:mascot';
export const NOTICE_KINDS = ['stored', 'published', 'replica'];
export const SHOW_SELF_IN_WEBRING = 'swing:show-self-in-webring';
export const CHECK_INTERVALS = [60, 300, 900, 1800];
const DEFAULT_CHECK_INTERVAL = 60;

export function readMascotSettings() {
  const raw = storage.get(MASCOT_SETTINGS_KEY, null);
  if (!raw) return {};
  try {
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

/* The check interval shares this key with the mascot settings, so each writer rewrites only its own fields. */
export function writeMascotSettings(fields) {
  return storage.trySet(MASCOT_SETTINGS_KEY, JSON.stringify({ ...readMascotSettings(), ...fields }));
}

export function readCheckInterval() {
  const { interval } = readMascotSettings();
  return interval === null || CHECK_INTERVALS.includes(interval) ? interval : DEFAULT_CHECK_INTERVAL;
}

export function writeCheckInterval(interval) {
  return writeMascotSettings({ interval });
}

export function checkIntervalMs(interval) {
  return interval == null ? null : interval * 1000;
}

function allKinds(on) {
  return Object.fromEntries(NOTICE_KINDS.map((kind) => [kind, on]));
}

export function defaultNotifySettings() {
  return { mascot: allKinds(true), browser: { enabled: false, ...allKinds(true) } };
}

function copyBooleans(from, fields, into) {
  if (!from || typeof from !== 'object') return;
  for (const field of fields) {
    if (typeof from[field] === 'boolean') into[field] = from[field];
  }
}

export function readNotifySettings() {
  const out = defaultNotifySettings();
  let parsed;
  try {
    parsed = JSON.parse(storage.get(NOTIFY_SETTINGS_KEY, 'null'));
  } catch {
    return out;
  }
  if (!parsed || typeof parsed !== 'object') return out;
  copyBooleans(parsed.mascot, NOTICE_KINDS, out.mascot);
  copyBooleans(parsed.browser, ['enabled', ...NOTICE_KINDS], out.browser);
  return out;
}

export function writeNotifySettings(settings) {
  return storage.trySet(NOTIFY_SETTINGS_KEY, JSON.stringify(settings));
}

export function notificationSupport() {
  if (!window.isSecureContext) return 'insecure';
  if (typeof Notification !== 'function') return 'unsupported';
  return Notification.permission;
}

export function unavailableReason() {
  const support = notificationSupport();
  return support === 'insecure' || support === 'unsupported' ? support : null;
}

export function blockedReason(enabled) {
  const unavailable = unavailableReason();
  if (unavailable) return unavailable;
  const support = notificationSupport();
  return enabled && support !== 'granted' ? support : null;
}

export function requestBrowserPermission() {
  const support = notificationSupport();
  if (support === 'granted') return Promise.resolve(null);
  if (support !== 'default') return Promise.resolve(support);
  return Notification.requestPermission().then(
    (result) => (result === 'granted' ? null : result),
    () => 'default',
  );
}

export function browserNotifyReady(settings = readNotifySettings()) {
  return settings.browser.enabled && NOTICE_KINDS.some((kind) => settings.browser[kind]) && notificationSupport() === 'granted';
}

export function kindWanted(settings, kind) {
  return settings.mascot[kind] || (settings.browser[kind] && browserNotifyReady(settings));
}
