import { storage } from './storage.js';

export const STARTUP_KEY = 'swing:desktop:startup';

export function startupView() {
  return storage.get(STARTUP_KEY, '0') === '1' ? 'desktop' : 'sites';
}

const els = {
  startup: document.getElementById('desk-sys-startup'),
  storageError: document.getElementById('desk-sys-storage-error'),
};

let saved = false;
let pending = false;
let notifyChanged = () => {};

function showStorageError(show) {
  els.storageError.hidden = !show;
  els.storageError.textContent = show ? '保存できませんでした。ブラウザの保存容量が足りないようです。' : '';
}

export const SystemSettingsPage = {
  id: 'system',
  init({ changed }) {
    notifyChanged = changed;
    saved = startupView() === 'desktop';
    pending = saved;
    els.startup.addEventListener('change', () => {
      pending = els.startup.checked;
      notifyChanged();
    });
  },
  open() {
    pending = saved;
    els.startup.checked = pending;
    showStorageError(false);
  },
  isDirty() {
    return pending !== saved;
  },
  save() {
    if (!storage.trySet(STARTUP_KEY, pending ? '1' : '0')) {
      showStorageError(true);
      return false;
    }
    showStorageError(false);
    saved = pending;
    notifyChanged();
    return true;
  },
  discard() {
    pending = saved;
  },
};
