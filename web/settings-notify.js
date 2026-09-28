import { t } from './i18n.js';
import { setStatus, clearStatus } from './util.js';
import { NOTICE_KINDS, blockedReason, checkIntervalMs, readCheckInterval, readNotifySettings, requestBrowserPermission, unavailableReason, writeCheckInterval, writeNotifySettings } from './notify-settings.js';

const REASON_KEYS = {
  insecure: 'browserNotifyInsecure',
  unsupported: 'browserNotifyUnsupported',
  denied: 'browserNotifyDenied',
  default: 'browserNotifyDefault',
  storage: 'browserNotifySaveFailed',
};

const els = {
  interval: document.getElementById('settings-check-interval'),
  enabled: document.getElementById('settings-notify-enabled'),
  kinds: Object.fromEntries(NOTICE_KINDS.map((kind) => [kind, document.getElementById(`settings-notify-${kind}`)])),
  status: document.getElementById('settings-notify-status'),
};

let reason = null;
let updates = null;

function showReason(next) {
  reason = next;
  if (reason) setStatus(els.status, 'error', t(REASON_KEYS[reason]));
  else clearStatus(els.status);
}

function syncForm() {
  const interval = readCheckInterval();
  els.interval.value = interval == null ? 'off' : String(interval);
  const { browser } = readNotifySettings();
  const unavailable = unavailableReason() != null;
  els.enabled.checked = browser.enabled && !unavailable;
  els.enabled.disabled = unavailable;
  for (const kind of NOTICE_KINDS) {
    els.kinds[kind].checked = browser[kind];
    els.kinds[kind].disabled = !els.enabled.checked;
  }
}

function showSaved(saved) {
  if (!saved) showReason('storage');
  else if (reason === 'storage') showReason(null);
  syncForm();
}

function update(patch) {
  const settings = readNotifySettings();
  showSaved(writeNotifySettings({ ...settings, browser: { ...settings.browser, ...patch } }));
}

function onIntervalChange() {
  const interval = els.interval.value === 'off' ? null : Number(els.interval.value);
  const saved = writeCheckInterval(interval);
  if (saved) updates.setInterval(checkIntervalMs(interval));
  showSaved(saved);
}

function onEnabledChange() {
  if (!els.enabled.checked) {
    showReason(null);
    update({ enabled: false });
    return;
  }
  requestBrowserPermission().then((blocked) => {
    showReason(blocked);
    if (blocked) els.enabled.checked = false;
    else update({ enabled: true });
  });
}

export const BrowserNotifySettings = {
  init({ updates: watcher }) {
    updates = watcher;
    els.interval.addEventListener('change', onIntervalChange);
    els.enabled.addEventListener('change', onEnabledChange);
    for (const kind of NOTICE_KINDS) {
      els.kinds[kind].addEventListener('change', () => update({ [kind]: els.kinds[kind].checked }));
    }
  },
  onShow() {
    showReason(blockedReason(readNotifySettings().browser.enabled));
    syncForm();
  },
  render() {
    showReason(reason);
  },
};
