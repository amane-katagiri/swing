import { storage } from './storage.js';
import { el } from './util.js';
import { createCombobox } from './desktop-combobox.js';
import { DesktopMascots } from './desktop-mascot.js';

export const MASCOT_SETTINGS_KEY = 'swing:desktop:mascot';
const INTERVALS = [60, 300, 900, 1800];
const DEFAULTS = { packs: null, interval: 60, walk: true, chatter: true };
const THUMB_PX = 48;
const PREVIEW_MARGIN = 8;

const els = {
  preview: document.getElementById('desk-ms-preview'),
  list: document.getElementById('desk-ms-list'),
  listStatus: document.getElementById('desk-ms-list-status'),
  comboField: document.getElementById('desk-ms-interval-field'),
  comboList: document.getElementById('desk-ms-interval-list'),
  walk: document.getElementById('desk-ms-walk'),
  chatter: document.getElementById('desk-ms-chatter'),
  storageError: document.getElementById('desk-ms-storage-error'),
};

let saved = { ...DEFAULTS };
let pending = { ...DEFAULTS };
let selectedId = null;
let rows = [];
let updates = null;
let notifyChanged = () => {};

function normalize(parsed) {
  const out = { ...DEFAULTS };
  if (!parsed || typeof parsed !== 'object') return out;
  if (Array.isArray(parsed.packs)) out.packs = [...new Set(parsed.packs.filter((id) => typeof id === 'string'))];
  if (parsed.interval === null || INTERVALS.includes(parsed.interval)) out.interval = parsed.interval;
  if (typeof parsed.walk === 'boolean') out.walk = parsed.walk;
  if (typeof parsed.chatter === 'boolean') out.chatter = parsed.chatter;
  return out;
}

function loadSaved() {
  const raw = storage.get(MASCOT_SETTINGS_KEY, null);
  if (!raw) return { ...DEFAULTS };
  try {
    return normalize(JSON.parse(raw));
  } catch {
    return { ...DEFAULTS };
  }
}

function canonical(state) {
  const out = {};
  if (state.packs != null) out.packs = [...state.packs].sort();
  out.interval = state.interval;
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

function intervalMs(state) {
  return state.interval == null ? null : state.interval * 1000;
}

function apply(state, previous) {
  if (!previous || previous.interval !== state.interval) updates.setInterval(intervalMs(state));
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

function onIntervalChange(value) {
  setPending({ ...pending, interval: value === 'off' ? null : Number(value) });
}

const intervalCombo = createCombobox({ field: els.comboField, list: els.comboList, onChange: onIntervalChange });

function syncForm() {
  for (const row of rows) row.input.checked = isShown(pending, row.id);
  intervalCombo.close();
  intervalCombo.setValue(pending.interval == null ? 'off' : String(pending.interval));
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
  init({ changed, dialog, updates: watcher }) {
    notifyChanged = changed;
    updates = watcher;
    saved = loadSaved();
    pending = { ...saved };
    els.list.hidden = true;
    els.walk.addEventListener('change', () => setPending({ ...pending, walk: els.walk.checked }));
    els.chatter.addEventListener('change', () => setPending({ ...pending, chatter: els.chatter.checked }));
    /* A titlebar drag doesn't reliably fire a `click` on the field/list, so the combobox's own outside-click close can miss it. */
    dialog.querySelector('.desk-titlebar').addEventListener('pointerdown', () => intervalCombo.close());
    new ResizeObserver(renderPreview).observe(els.preview);
    DesktopMascots.whenLoaded().then(buildList);
    apply(saved, null);
  },
  open() {
    pending = { ...saved };
    showStorageError(false);
    syncForm();
  },
  isDirty,
  save() {
    const { packs, interval, walk, chatter } = pending;
    const next = packs == null ? { interval, walk, chatter } : { packs, interval, walk, chatter };
    if (!storage.trySet(MASCOT_SETTINGS_KEY, JSON.stringify(next))) {
      showStorageError(true);
      return false;
    }
    showStorageError(false);
    const previous = saved;
    saved = { ...pending };
    apply(saved, previous);
    setPending({ ...saved });
    return true;
  },
  discard() {
    pending = { ...saved };
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
