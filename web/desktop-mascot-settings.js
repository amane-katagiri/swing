import { el } from './util.js';
import { DesktopMascots } from './desktop-mascot.js';
import { readMascotSettings, writeMascotSettings } from './notify-settings.js';

const DEFAULTS = { packs: ['yureko'], walk: true, chatter: true };
const THUMB_PX = 48;
const PREVIEW_MARGIN = 8;

const els = {
  preview: document.getElementById('desk-ms-preview'),
  list: document.getElementById('desk-ms-list'),
  listStatus: document.getElementById('desk-ms-list-status'),
  walk: document.getElementById('desk-ms-walk'),
  chatter: document.getElementById('desk-ms-chatter'),
  storageError: document.getElementById('desk-ms-storage-error'),
};

let saved = { ...DEFAULTS };
let pending = { ...DEFAULTS };
let selectedId = null;
let rows = [];
let notifyChanged = () => {};

function normalize(parsed) {
  const out = { ...DEFAULTS };
  if (!parsed || typeof parsed !== 'object') return out;
  if (parsed.packs === null) out.packs = null;
  else if (Array.isArray(parsed.packs)) out.packs = [...new Set(parsed.packs.filter((id) => typeof id === 'string'))];
  if (typeof parsed.walk === 'boolean') out.walk = parsed.walk;
  if (typeof parsed.chatter === 'boolean') out.chatter = parsed.chatter;
  return out;
}

function loadSaved() {
  return normalize(readMascotSettings());
}

function canonical(state) {
  const out = {};
  if (state.packs != null) out.packs = [...state.packs].sort();
  out.walk = state.walk;
  out.chatter = state.chatter;
  return out;
}

function isDirty() {
  return JSON.stringify(canonical(pending)) !== JSON.stringify(canonical(saved));
}

function isShown(state, id) {
  return state.packs == null || state.packs.includes(id);
}

function apply(state) {
  DesktopMascots.applySettings(state);
}

function stillFrame(pack, scale) {
  const frame = pack.animations.idle.frames[0];
  const { width: fw, height: fh } = pack.frame;
  const col = frame.index % pack.sheet.cols;
  const row = Math.floor(frame.index / pack.sheet.cols);
  const node = el('span', { class: 'desk-ms-still', 'aria-hidden': 'true' });
  node.style.width = `${fw * scale}px`;
  node.style.height = `${fh * scale}px`;
  node.style.backgroundImage = `url("${pack.spriteUrl}")`;
  node.style.backgroundSize = `${pack.sheet.width * scale}px ${pack.sheet.height * scale}px`;
  node.style.backgroundPosition = `${-col * fw * scale}px ${-row * fh * scale}px`;
  if (frame.dy) node.style.transform = `translateY(${frame.dy * scale}px)`;
  return node;
}

function fitScale(pack, width, height) {
  return Math.max(1, Math.min(Math.floor(width / pack.frame.width), Math.floor(height / pack.frame.height)));
}

function packLabel(pack) {
  return pack.name || pack.id;
}

function renderPreview() {
  const pack = DesktopMascots.packs().find((p) => p.id === selectedId);
  if (!pack) {
    els.preview.replaceChildren();
    return;
  }
  const w = els.preview.clientWidth - PREVIEW_MARGIN * 2;
  const h = els.preview.clientHeight - PREVIEW_MARGIN * 2;
  els.preview.replaceChildren(stillFrame(pack, w > 0 && h > 0 ? fitScale(pack, w, h) : 1));
}

function select(id) {
  selectedId = id;
  for (const row of rows) row.item.classList.toggle('is-selected', row.id === id);
  renderPreview();
}

function toggle(id, on) {
  const packs = DesktopMascots.packs();
  const shown = new Set(packs.filter((p) => isShown(pending, p.id)).map((p) => p.id));
  if (on) shown.add(id);
  else shown.delete(id);
  const next = packs.filter((p) => shown.has(p.id)).map((p) => p.id);
  setPending({ ...pending, packs: next.length === packs.length ? null : next });
}

function buildList() {
  const packs = DesktopMascots.packs();
  rows = packs.map((pack) => {
    const input = el('input', { type: 'checkbox', class: 'desk-check-input' });
    input.addEventListener('change', () => toggle(pack.id, input.checked));
    input.addEventListener('focus', () => select(pack.id));
    const thumb = el('span', { class: 'desk-ms-thumb' }, stillFrame(pack, fitScale(pack, THUMB_PX, THUMB_PX)));
    const label = el('span', { class: 'desk-check-label desk-ms-name', title: packLabel(pack) }, packLabel(pack));
    const item = el('li', { class: 'desk-ms-item' }, el('label', { class: 'desk-check desk-ms-check' }, [input, thumb, label]));
    item.addEventListener('pointerdown', () => select(pack.id));
    return { id: pack.id, item, input };
  });
  els.list.replaceChildren(...rows.map((row) => row.item));
  els.list.hidden = rows.length === 0;
  els.listStatus.hidden = rows.length > 0;
  els.listStatus.textContent = rows.length > 0 ? '' : 'マスコットがありません';
  if (!rows.some((row) => row.id === selectedId)) selectedId = rows.length > 0 ? rows[0].id : null;
  syncForm();
}

function syncForm() {
  for (const row of rows) row.input.checked = isShown(pending, row.id);
  els.walk.checked = pending.walk;
  els.chatter.checked = pending.chatter;
  select(selectedId);
}

function setPending(next) {
  pending = next;
  syncForm();
  notifyChanged();
}

function showStorageError(show) {
  els.storageError.hidden = !show;
  els.storageError.textContent = show ? '保存できませんでした。ブラウザの保存容量が足りないようです。' : '';
}

export const MascotSettingsPage = {
  id: 'mascot',
  init({ changed }) {
    notifyChanged = changed;
    saved = loadSaved();
    pending = { ...saved };
    els.list.hidden = true;
    els.walk.addEventListener('change', () => setPending({ ...pending, walk: els.walk.checked }));
    els.chatter.addEventListener('change', () => setPending({ ...pending, chatter: els.chatter.checked }));
    new ResizeObserver(renderPreview).observe(els.preview);
    DesktopMascots.whenLoaded().then(buildList);
    apply(saved);
  },
  open() {
    pending = { ...saved };
    showStorageError(false);
    syncForm();
  },
  isDirty,
  save() {
    const { packs, walk, chatter } = pending;
    if (!writeMascotSettings({ packs, walk, chatter })) {
      showStorageError(true);
      return false;
    }
    showStorageError(false);
    saved = { ...pending };
    apply(saved);
    setPending({ ...saved });
    return true;
  },
  discard() {
    pending = { ...saved };
  },
};
