import { showStorageError } from './desktop-dialog.js';
import { storage } from './storage.js';
import { el } from './util.js';
import { refocusIfDropped } from './desktop-focus.js';
import { createCombobox } from './desktop-combobox.js';
import { DEFAULT_COLOR, quantizeColorHex, processImageFile } from './desktop-wallpaper-image.js';

const WALLPAPER_KEY = 'swing:desktop:wallpaper';
const DISPLAY_MODES = ['center', 'tile', 'contain', 'cover', 'stretch'];
const NATIVE_SIZE_MODES = new Set(['center', 'tile']);

const PALETTE = [
  '#000000', '#808080', '#800000', '#808000', '#008000', '#008080', '#000080', '#800080', '#808040', '#004040',
  '#004080', '#4000ff', '#c0c0c0', '#ff0000', '#ffff00', '#00ff00', '#00ffff', '#0000ff', '#ff00ff',
  '#ffff80', '#c0dcc0', '#a4a0a0', '#ffffff',
];

const els = {
  deskScreen: document.getElementById('desk-screen'),
  wallpaper: document.getElementById('desk-wallpaper'),
  preview: document.getElementById('desk-wp-preview'),
  swatches: document.getElementById('desk-wp-swatches'),
  customColor: document.getElementById('desk-wp-custom-color-input'),
  colorResetBtn: document.getElementById('desk-wp-color-reset'),
  browseBtn: document.getElementById('desk-wp-browse'),
  removeBtn: document.getElementById('desk-wp-remove'),
  fileInput: document.getElementById('desk-wp-file-input'),
  imageFilename: document.getElementById('desk-wp-image-filename'),
  imageSize: document.getElementById('desk-wp-image-size'),
  comboField: document.getElementById('desk-wp-display-field'),
  comboList: document.getElementById('desk-wp-display-list'),
  imageError: document.getElementById('desk-wp-image-error'),
  storageError: document.getElementById('desk-wp-storage-error'),
};

let saved = {};
let pending = {};
let notifyChanged = () => {};
let dialogEl = null;

function canonicalWallpaper(state) {
  const out = {};
  if (state.color) out.color = state.color;
  if (state.image) {
    out.image = {
      dataUrl: state.image.dataUrl,
      width: state.image.width,
      height: state.image.height,
      display: state.image.display,
      filename: state.image.filename || '',
    };
  }
  return out;
}

function normalizeWallpaper(parsed) {
  const result = {};
  if (!parsed || typeof parsed !== 'object') return result;
  if (typeof parsed.color === 'string' && /^#[0-9a-f]{6}$/i.test(parsed.color)) {
    result.color = parsed.color.toLowerCase();
  }
  const img = parsed.image;
  if (img && typeof img === 'object' && typeof img.dataUrl === 'string' && img.dataUrl.startsWith('data:image/')) {
    const width = Number(img.width);
    const height = Number(img.height);
    const display = DISPLAY_MODES.includes(img.display) ? img.display : 'center';
    if (Number.isFinite(width) && Number.isFinite(height) && width > 0 && height > 0) {
      result.image = { dataUrl: img.dataUrl, width, height, display, filename: typeof img.filename === 'string' ? img.filename : '' };
    }
  }
  return result;
}

function loadSavedWallpaper() {
  const raw = storage.get(WALLPAPER_KEY, null);
  if (!raw) return {};
  try {
    return normalizeWallpaper(JSON.parse(raw));
  } catch {
    return {};
  }
}

function saveWallpaper(state) {
  return storage.trySet(WALLPAPER_KEY, JSON.stringify(state));
}

function previewScale() {
  const screenRect = els.deskScreen.getBoundingClientRect();
  const previewRect = els.preview.getBoundingClientRect();
  if (!screenRect.width || !previewRect.width) return 1;
  return previewRect.width / screenRect.width;
}

function applyBackground(target, state, scale = 1) {
  target.style.backgroundColor = state.color || '';
  target.style.backgroundImage = '';
  target.style.backgroundRepeat = '';
  target.style.backgroundPosition = '';
  target.style.backgroundSize = '';
  target.removeAttribute('data-fit');

  const image = state.image;
  if (!image || !image.dataUrl) return;
  const display = image.display || 'center';
  target.style.backgroundImage = `url("${image.dataUrl}")`;

  if (NATIVE_SIZE_MODES.has(display)) {
    const w = Math.max(1, Math.round(image.width * scale));
    const h = Math.max(1, Math.round(image.height * scale));
    target.style.backgroundSize = `${w}px ${h}px`;
    target.style.backgroundRepeat = display === 'tile' ? 'repeat' : 'no-repeat';
    target.style.backgroundPosition = display === 'tile' ? 'top left' : 'center';
    target.setAttribute('data-fit', 'pixelated');
    return;
  }
  target.style.backgroundRepeat = 'no-repeat';
  target.style.backgroundPosition = 'center';
  if (display === 'contain') target.style.backgroundSize = 'contain';
  else if (display === 'cover') target.style.backgroundSize = 'cover';
  else target.style.backgroundSize = '100% 100%';
}

function updatePreview() {
  applyBackground(els.preview, pending, previewScale());
}

function isDirty() {
  return JSON.stringify(canonicalWallpaper(pending)) !== JSON.stringify(canonicalWallpaper(saved));
}

function showImageError(message) {
  if (!message) {
    els.imageError.hidden = true;
    els.imageError.textContent = '';
    return;
  }
  els.imageError.hidden = false;
  els.imageError.textContent = message;
}

function setImageInfo(image) {
  if (!image) {
    els.imageFilename.textContent = '画像が選択されていません';
    els.imageFilename.title = '';
    els.imageSize.textContent = '';
    return;
  }
  const name = image.filename || '(名前なし)';
  els.imageFilename.textContent = name;
  els.imageFilename.title = name;
  els.imageSize.textContent = `${image.width}×${image.height}・16 ビット`;
}

function highlightSwatch(hex) {
  const target = (hex || '').toLowerCase();
  for (const btn of els.swatches.children) {
    btn.classList.toggle('is-selected', btn.dataset.color === target);
  }
}

function buildSwatches() {
  const nodes = [];
  for (const hex of PALETTE) {
    const btn = el('button', {
      type: 'button',
      class: 'desk-wp-swatch',
      style: `background-color:${hex}`,
      'data-color': hex,
      title: hex,
      'aria-label': hex,
      onclick: () => pickColor(hex),
    });
    nodes.push(btn);
  }
  els.swatches.replaceChildren(...nodes);
}

function syncColorPanel() {
  const color = pending.color || DEFAULT_COLOR;
  els.customColor.value = color;
  highlightSwatch(pending.color || null);
}

function onDisplayChange(value) {
  if (!pending.image) return;
  setPending({ ...pending, image: { ...pending.image, display: value } });
}

const displayCombo = createCombobox({ field: els.comboField, list: els.comboList, onChange: onDisplayChange });

function syncImagePanel() {
  const image = pending.image || null;
  setImageInfo(image);
  els.removeBtn.disabled = !image;
  displayCombo.setDisabled(!image);
  displayCombo.close();
  displayCombo.setValue(image ? image.display : 'center');
}

function setPending(next) {
  pending = next;
  syncColorPanel();
  syncImagePanel();
  updatePreview();
  notifyChanged();
}

function pickColor(hex) {
  setPending({ ...pending, color: quantizeColorHex(hex) });
}

function resetColor() {
  const next = { ...pending };
  delete next.color;
  setPending(next);
}

function removeImage() {
  refocusIfDropped(dialogEl, els.browseBtn, () => {
    const next = { ...pending };
    delete next.image;
    setPending(next);
  });
}

async function onFileChosen() {
  const file = els.fileInput.files && els.fileInput.files[0];
  els.fileInput.value = '';
  if (!file) return;
  showImageError(null);
  els.imageFilename.textContent = 'よみこみちゅう…';
  els.imageFilename.title = '';
  els.imageSize.textContent = '';
  try {
    const { dataUrl, width, height } = await processImageFile(file);
    const display = (pending.image && pending.image.display) || 'center';
    setPending({ ...pending, image: { dataUrl, width, height, display, filename: file.name } });
  } catch (err) {
    showImageError(`画像を読み込めませんでした。（${err && err.message ? err.message : '不明なエラー'}）`);
    syncImagePanel();
  }
}

function syncFormFromState() {
  syncColorPanel();
  syncImagePanel();
  showImageError(null);
  showStorageError(els.storageError, false);
  updatePreview();
  notifyChanged();
}

function wireEvents() {
  els.customColor.addEventListener('input', () => pickColor(els.customColor.value));
  els.colorResetBtn.addEventListener('click', resetColor);
  els.browseBtn.addEventListener('click', () => els.fileInput.click());
  els.fileInput.addEventListener('change', onFileChosen);
  els.removeBtn.addEventListener('click', removeImage);
}

export const WallpaperPage = {
  id: 'background',
  init({ changed, dialog }) {
    notifyChanged = changed;
    dialogEl = dialog;
    saved = loadSavedWallpaper();
    pending = structuredClone(saved);
    buildSwatches();
    wireEvents();
    this.boot();
  },
  open() {
    pending = structuredClone(saved);
    syncFormFromState();
  },
  isDirty,
  save() {
    const canonical = canonicalWallpaper(pending);
    if (!saveWallpaper(canonical)) {
      showStorageError(els.storageError, true);
      return false;
    }
    showStorageError(els.storageError, false);
    saved = canonical;
    applyBackground(els.wallpaper, saved);
    setPending(structuredClone(canonical));
    return true;
  },
  discard() {
    pending = structuredClone(saved);
  },
  onKey(ev) {
    if (!displayCombo.isOpen()) return false;
    if (ev.key === 'Tab') {
      displayCombo.close();
      return false;
    }
    displayCombo.handleKey(ev);
    return true;
  },
  boot() {
    applyBackground(els.wallpaper, saved);
  },
};
