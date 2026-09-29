import { showStorageError } from './desktop-dialog.js';
import { storage } from './storage.js';

const STARTUP_KEY = 'swing:desktop:startup';

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
    showStorageError(els.storageError, false);
  },
  isDirty() {
    return pending !== saved;
  },
  save() {
    if (!storage.trySet(STARTUP_KEY, pending ? '1' : '0')) {
      showStorageError(els.storageError, true);
      return false;
    }
    showStorageError(els.storageError, false);
    saved = pending;
    notifyChanged();
    return true;
  },
  discard() {
    pending = saved;
  },
};
