import { createDialog } from './desktop-dialog.js';
import { apiFetch, cache, describeError } from './util.js';
import { parseMirrorKeys, MAX_MIRROR_KEYS } from './ui.js';

const TEXT = {
  adding: 'ミラーに追加しています…',
  noChange: '入力した鍵はすべてミラー済みです。',
  tooMany: (n) => `一度に追加できるのは ${MAX_MIRROR_KEYS} 件までです（${n} 件入力されています）。`,
  error: (detail) => `ミラーに追加できませんでした。（${detail}）`,
  added: (n) => `${n} 件をミラーに追加しました`,
  alreadyMirrored: (n) => `${n} 件はミラー済み`,
  relays: (ok, total) => `リレー ${ok}/${total} 件に送信`,
};

const dialogRoot = document.getElementById('desk-dialog-mirror-add');

const els = {
  button: document.getElementById('desk-mirror-add'),
  input: document.getElementById('desk-mirror-add-keys'),
  status: document.getElementById('desk-mirror-add-status'),
  okBtn: dialogRoot.querySelector('[data-dialog-action="ok"]'),
};

let busy = false;
let onAdded = async () => {};

function showStatus(kind, text) {
  els.status.hidden = !text;
  els.status.textContent = text || '';
  if (kind) els.status.dataset.kind = kind;
  else delete els.status.dataset.kind;
}

function refreshOk() {
  els.okBtn.disabled = busy || !els.input.value.trim();
}

function setBusyState(next) {
  busy = next;
  els.input.readOnly = next;
  refreshOk();
}

function summary(result) {
  const notes = [];
  if (result.unchanged.length > 0) notes.push(TEXT.alreadyMirrored(result.unchanged.length));
  if (result.published) notes.push(TEXT.relays(result.relays.filter((r) => r.ok).length, result.relays.length));
  const head = TEXT.added(result.changed.length);
  return notes.length > 0 ? `${head}（${notes.join('、')}）` : head;
}

async function submit() {
  const keys = parseMirrorKeys(els.input.value);
  if (keys.length === 0) return;
  if (keys.length > MAX_MIRROR_KEYS) {
    showStatus('error', TEXT.tooMany(keys.length));
    return;
  }
  setBusyState(true);
  showStatus('loading', TEXT.adding);
  let result;
  try {
    result = await apiFetch('/api/mirror/add', { method: 'POST', body: JSON.stringify({ keys }) });
  } catch (err) {
    showStatus('error', TEXT.error(describeError(err)));
    return;
  } finally {
    setBusyState(false);
  }
  if (result.changed.length === 0) {
    showStatus(null, TEXT.noChange);
    return;
  }
  cache.mirror = null;
  if (!dialogRoot.hidden) dialog.close();
  await onAdded(summary(result));
}

const dialog = createDialog({
  root: dialogRoot,
  returnFocus: els.button,
  onOpen() {
    if (busy) return;
    els.input.value = '';
    showStatus(null, '');
    refreshOk();
  },
  onOk() {
    if (!busy && !els.okBtn.disabled) submit();
    return false;
  },
});

export const DesktopMirrorAdd = {
  init({ added }) {
    onAdded = added;
    els.input.addEventListener('input', refreshOk);
    els.button.addEventListener('click', () => {
      dialog.open();
      els.input.focus();
    });
  },
};
