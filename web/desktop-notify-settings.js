import { createCombobox } from './desktop-combobox.js';
import { NOTICE_KINDS, blockedReason, checkIntervalMs, readCheckInterval, readNotifySettings, requestBrowserPermission, unavailableReason, writeCheckInterval, writeNotifySettings } from './notify-settings.js';

const REASONS = {
  insecure: 'このページは安全な接続（https:// か localhost）で開かれていないため、ブラウザの通知を使えません。',
  unsupported: 'このブラウザはページからの通知に対応していません。',
  denied: 'ブラウザの設定で、このページからの通知がブロックされています。サイトの設定で通知を許可してから、もう一度オンにしてください。',
  default: '通知の許可が得られませんでした。もう一度オンにすると、許可を求めます。',
};

function kindInputs(prefix) {
  return Object.fromEntries(NOTICE_KINDS.map((kind) => [kind, document.getElementById(`${prefix}-${kind}`)]));
}

const els = {
  comboField: document.getElementById('desk-nt-interval-field'),
  comboList: document.getElementById('desk-nt-interval-list'),
  mascot: kindInputs('desk-nt-mascot'),
  browser: document.getElementById('desk-nt-browser'),
  browserKinds: kindInputs('desk-nt-browser'),
  reason: document.getElementById('desk-nt-browser-reason'),
  storageError: document.getElementById('desk-nt-storage-error'),
};

let saved = null;
let pending = null;
let updates = null;
let notifyChanged = () => {};

function isDirty() {
  return JSON.stringify(pending) !== JSON.stringify(saved);
}

function applyInterval(interval) {
  updates.setInterval(checkIntervalMs(interval));
}

function loadSaved() {
  return { interval: readCheckInterval(), notify: readNotifySettings() };
}

function showReason(reason) {
  els.reason.hidden = !reason;
  els.reason.textContent = reason ? REASONS[reason] : '';
}

const intervalCombo = createCombobox({
  field: els.comboField,
  list: els.comboList,
  onChange: (value) => setPending({ ...pending, interval: value === 'off' ? null : Number(value) }),
});

function syncForm() {
  intervalCombo.close();
  intervalCombo.setValue(pending.interval == null ? 'off' : String(pending.interval));
  const { mascot, browser } = pending.notify;
  const unavailable = unavailableReason() != null;
  els.browser.checked = browser.enabled && !unavailable;
  els.browser.disabled = unavailable;
  for (const kind of NOTICE_KINDS) {
    els.mascot[kind].checked = mascot[kind];
    els.browserKinds[kind].checked = browser[kind];
    els.browserKinds[kind].disabled = !els.browser.checked;
  }
}

function setPending(next) {
  pending = next;
  syncForm();
  notifyChanged();
}

function setNotify(section, patch) {
  const notify = { ...pending.notify, [section]: { ...pending.notify[section], ...patch } };
  setPending({ ...pending, notify });
}

function onBrowserChange() {
  if (!els.browser.checked) {
    showReason(null);
    setNotify('browser', { enabled: false });
    return;
  }
  requestBrowserPermission().then((reason) => {
    showReason(reason);
    if (reason) els.browser.checked = false;
    else setNotify('browser', { enabled: true });
  });
}

function showStorageError(show) {
  els.storageError.hidden = !show;
  els.storageError.textContent = show ? '保存できませんでした。ブラウザの保存容量が足りないようです。' : '';
}

export const NotifySettingsPage = {
  id: 'notify',
  init({ changed, dialog, updates: watcher }) {
    notifyChanged = changed;
    updates = watcher;
    saved = loadSaved();
    pending = structuredClone(saved);
    els.browser.addEventListener('change', onBrowserChange);
    for (const kind of NOTICE_KINDS) {
      els.mascot[kind].addEventListener('change', () => setNotify('mascot', { [kind]: els.mascot[kind].checked }));
      els.browserKinds[kind].addEventListener('change', () => setNotify('browser', { [kind]: els.browserKinds[kind].checked }));
    }
    /* A titlebar drag doesn't reliably fire a `click` on the field/list, so the combobox's own outside-click close can miss it. */
    dialog.querySelector('.desk-titlebar').addEventListener('pointerdown', () => intervalCombo.close());
    applyInterval(saved.interval);
  },
  open() {
    const previous = saved;
    saved = loadSaved();
    if (saved.interval !== previous.interval) applyInterval(saved.interval);
    pending = structuredClone(saved);
    showStorageError(false);
    showReason(blockedReason(pending.notify.browser.enabled));
    syncForm();
  },
  isDirty,
  save() {
    const { interval, notify } = pending;
    if (!writeCheckInterval(interval) || !writeNotifySettings(notify)) {
      showStorageError(true);
      return false;
    }
    showStorageError(false);
    if (interval !== saved.interval) applyInterval(interval);
    saved = structuredClone(pending);
    setPending(structuredClone(saved));
    return true;
  },
  discard() {
    pending = structuredClone(saved);
  },
  onKey(ev) {
    if (!intervalCombo.isOpen()) return false;
    if (ev.key === 'Tab') {
      intervalCombo.close();
      return false;
    }
    intervalCombo.handleKey(ev);
    return true;
  },
};
