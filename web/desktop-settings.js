import { storage } from './storage.js';
import { el } from './util.js';

const WALLPAPER_KEY = 'swing:desktop:wallpaper';
const DISPLAY_MODES = ['center', 'tile', 'contain', 'cover', 'stretch'];
const NATIVE_SIZE_MODES = new Set(['center', 'tile']);

const DOWNSCALE_STEPS = [1920, 1280, 1024, 800, 640, 512, 400, 320, 240];
const STORAGE_TARGET_CHARS = 2.5 * 1024 * 1024;

const PALETTE = [
  '#000000', '#808080', '#800000', '#808000', '#008000', '#008080', '#000080', '#800080', '#808040', '#004040',
  '#004080', '#4000ff', '#808080', '#c0c0c0', '#ff0000', '#ffff00', '#00ff00', '#00ffff', '#0000ff', '#ff00ff',
  '#ffff80', '#c0dcc0', '#a4a0a0', '#ffffff',
];

const FLASH_TOGGLES = 6;
const FLASH_INTERVAL_MS = 90;

const TITLEBAR_H = 26;
const MIN_VISIBLE_TITLEBAR = 60;

const settingsEls = {
  deskScreen: document.getElementById('desk-screen'),
  wallpaper: document.getElementById('desk-wallpaper'),
  icon: document.getElementById('desk-icon-control-panel'),
  overlay: document.getElementById('desk-dialog-overlay'),
  dialog: document.getElementById('desk-dialog-control-panel'),
  titlebar: document.getElementById('desk-dialog-titlebar'),
  closeBtn: document.getElementById('desk-dialog-close'),
  okBtn: document.getElementById('desk-dialog-ok'),
  cancelBtn: document.getElementById('desk-dialog-cancel'),
  applyBtn: document.getElementById('desk-dialog-apply'),
  tabButtons: Array.from(document.querySelectorAll('.desk-dialog-tab')),
  tabPanels: Array.from(document.querySelectorAll('.desk-tabpanel')),
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
  comboValue: document.getElementById('desk-wp-display-value'),
  comboList: document.getElementById('desk-wp-display-list'),
  comboOptions: Array.from(document.querySelectorAll('#desk-wp-display-list .desk-wp-combobox-option')),
  imageError: document.getElementById('desk-wp-image-error'),
  storageError: document.getElementById('desk-wp-storage-error'),
};

let savedState = {};
let pendingState = {};
let flashTimer = null;
let flashCount = 0;
let dragState = null;
let comboOpen = false;
let comboActiveIndex = 0;
let onActivationChange = () => {};

function clampNum(v, lo, hi) {
  return Math.min(hi, Math.max(lo, v));
}

function cloneWallpaper(state) {
  return JSON.parse(JSON.stringify(state));
}

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

function quantize5(v) {
  const level = Math.round((v / 255) * 31);
  return (level << 3) | (level >> 2);
}

function quantize6(v) {
  const level = Math.round((v / 255) * 63);
  return (level << 2) | (level >> 4);
}

function quantizeColorHex(hex) {
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex || '');
  if (!m) return '#008080';
  const r = quantize5(parseInt(m[1], 16));
  const g = quantize6(parseInt(m[2], 16));
  const b = quantize5(parseInt(m[3], 16));
  return `#${[r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('')}`;
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
  const screenRect = settingsEls.deskScreen.getBoundingClientRect();
  const previewRect = settingsEls.preview.getBoundingClientRect();
  if (!screenRect.width || !previewRect.width) return 1;
  return previewRect.width / screenRect.width;
}

function applyBackground(target, state, opts) {
  const isPreview = !!(opts && opts.preview);
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
    const scale = isPreview ? previewScale() : 1;
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
  applyBackground(settingsEls.preview, pendingState, { preview: true });
}

function isDirty() {
  return JSON.stringify(canonicalWallpaper(pendingState)) !== JSON.stringify(canonicalWallpaper(savedState));
}

function updateDirtyUI() {
  settingsEls.applyBtn.disabled = !isDirty();
}

function showStorageError(show) {
  settingsEls.storageError.hidden = !show;
  if (show) {
    settingsEls.storageError.textContent = '保存できませんでした。ブラウザの保存容量が足りないようです。';
  }
}

function showImageError(message) {
  if (!message) {
    settingsEls.imageError.hidden = true;
    settingsEls.imageError.textContent = '';
    return;
  }
  settingsEls.imageError.hidden = false;
  settingsEls.imageError.textContent = message;
}

function setImageInfo(image) {
  if (!image) {
    settingsEls.imageFilename.textContent = '画像が選択されていません';
    settingsEls.imageFilename.title = '';
    settingsEls.imageSize.textContent = '';
    return;
  }
  const name = image.filename || '(名前なし)';
  settingsEls.imageFilename.textContent = name;
  settingsEls.imageFilename.title = name;
  settingsEls.imageSize.textContent = `${image.width}×${image.height}・16 ビット`;
}

function highlightSwatch(hex) {
  const target = (hex || '').toLowerCase();
  for (const btn of settingsEls.swatches.children) {
    btn.classList.toggle('is-selected', btn.dataset.color === target);
  }
}

function buildSwatches() {
  const seen = new Set();
  const nodes = [];
  for (const hex of PALETTE) {
    if (seen.has(hex)) continue;
    seen.add(hex);
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
  settingsEls.swatches.replaceChildren(...nodes);
}

function pickColor(hex) {
  const q = quantizeColorHex(hex);
  pendingState = { ...pendingState, color: q };
  syncColorPanel();
  updatePreview();
  updateDirtyUI();
}

function resetColor() {
  const next = { ...pendingState };
  delete next.color;
  pendingState = next;
  syncColorPanel();
  updatePreview();
  updateDirtyUI();
}

function removeImage() {
  const next = { ...pendingState };
  delete next.image;
  pendingState = next;
  const hadFocus = settingsEls.dialog.contains(document.activeElement);
  syncImagePanel();
  /* Disabling the focused button drops focus to body. */
  if (hadFocus && !settingsEls.dialog.contains(document.activeElement)) settingsEls.browseBtn.focus();
  updatePreview();
  updateDirtyUI();
}

function syncColorPanel() {
  const color = pendingState.color || '#008080';
  settingsEls.customColor.value = color;
  highlightSwatch(pendingState.color || null);
}

function comboLabelFor(value) {
  const opt = settingsEls.comboOptions.find((o) => o.dataset.value === value);
  return opt ? opt.textContent : '';
}

function comboIndexOf(value) {
  const i = settingsEls.comboOptions.findIndex((o) => o.dataset.value === value);
  return i === -1 ? 0 : i;
}

function renderComboValue(value) {
  settingsEls.comboField.dataset.value = value;
  settingsEls.comboValue.textContent = comboLabelFor(value);
  for (const o of settingsEls.comboOptions) o.setAttribute('aria-selected', String(o.dataset.value === value));
}

function positionComboList() {
  const rect = settingsEls.comboField.getBoundingClientRect();
  settingsEls.comboList.style.left = `${Math.round(rect.left)}px`;
  settingsEls.comboList.style.top = `${Math.round(rect.bottom + 2)}px`;
  settingsEls.comboList.style.width = `${Math.round(rect.width)}px`;
}

function highlightComboIndex(index) {
  comboActiveIndex = clampNum(index, 0, settingsEls.comboOptions.length - 1);
  for (let i = 0; i < settingsEls.comboOptions.length; i += 1) {
    settingsEls.comboOptions[i].classList.toggle('is-active', i === comboActiveIndex);
  }
  const active = settingsEls.comboOptions[comboActiveIndex];
  settingsEls.comboField.setAttribute('aria-activedescendant', active.id);
  active.scrollIntoView({ block: 'nearest' });
}

function onComboOutsideClick(ev) {
  if (settingsEls.comboField.contains(ev.target) || settingsEls.comboList.contains(ev.target)) return;
  closeCombo();
}

function openCombo() {
  if (settingsEls.comboField.disabled || comboOpen) return;
  comboOpen = true;
  positionComboList();
  settingsEls.comboList.hidden = false;
  settingsEls.comboField.setAttribute('aria-expanded', 'true');
  highlightComboIndex(comboIndexOf(settingsEls.comboField.dataset.value || 'center'));
  document.addEventListener('click', onComboOutsideClick, true);
}

function closeCombo() {
  if (!comboOpen) return;
  comboOpen = false;
  settingsEls.comboList.hidden = true;
  settingsEls.comboField.setAttribute('aria-expanded', 'false');
  settingsEls.comboField.removeAttribute('aria-activedescendant');
  document.removeEventListener('click', onComboOutsideClick, true);
}

function onDisplayChange(value) {
  if (!pendingState.image) return;
  pendingState = { ...pendingState, image: { ...pendingState.image, display: value } };
  updatePreview();
  updateDirtyUI();
}

function commitCombo(value) {
  renderComboValue(value);
  closeCombo();
  onDisplayChange(value);
}

function handleComboKeydown(ev) {
  switch (ev.key) {
    case 'Escape':
      ev.preventDefault();
      ev.stopPropagation();
      closeCombo();
      return;
    case 'ArrowDown':
      ev.preventDefault();
      highlightComboIndex(comboActiveIndex + 1);
      return;
    case 'ArrowUp':
      ev.preventDefault();
      highlightComboIndex(comboActiveIndex - 1);
      return;
    case 'Home':
      ev.preventDefault();
      highlightComboIndex(0);
      return;
    case 'End':
      ev.preventDefault();
      highlightComboIndex(settingsEls.comboOptions.length - 1);
      return;
    case 'Enter':
    case ' ':
      ev.preventDefault();
      commitCombo(settingsEls.comboOptions[comboActiveIndex].dataset.value);
      return;
    default:
  }
}

function syncImagePanel() {
  const image = pendingState.image || null;
  setImageInfo(image);
  settingsEls.removeBtn.disabled = !image;
  settingsEls.comboField.disabled = !image;
  closeCombo();
  renderComboValue(image ? image.display : 'center');
}

function syncFormFromState() {
  syncColorPanel();
  syncImagePanel();
  showImageError(null);
  showStorageError(false);
  updatePreview();
  updateDirtyUI();
}

function decodeImageFile(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error('ファイルを読み込めませんでした'));
    reader.onload = () => {
      const img = new Image();
      img.onerror = () => reject(new Error('画像を解釈できませんでした'));
      img.onload = () => resolve(img);
      img.src = reader.result;
    };
    reader.readAsDataURL(file);
  });
}

/* Not composited with the background color so a later color change still shows through. */
function quantizeCanvas(ctx, w, h) {
  const imgData = ctx.getImageData(0, 0, w, h);
  const d = imgData.data;
  for (let i = 0; i < d.length; i += 4) {
    const a = d[i + 3] >= 128 ? 255 : 0;
    if (a === 0) {
      d[i] = 0;
      d[i + 1] = 0;
      d[i + 2] = 0;
    } else {
      d[i] = quantize5(d[i]);
      d[i + 1] = quantize6(d[i + 1]);
      d[i + 2] = quantize5(d[i + 2]);
    }
    d[i + 3] = a;
  }
  ctx.putImageData(imgData, 0, 0);
}

/* One large drawImage() reduction aliases even with imageSmoothingQuality "high". */
function drawScaledSmooth(ctx, img, dw, dh) {
  let src = img;
  let srcW = img.naturalWidth || img.width;
  let srcH = img.naturalHeight || img.height;
  while (srcW / 2 > dw && srcH / 2 > dh) {
    const nextW = Math.max(dw, Math.round(srcW / 2));
    const nextH = Math.max(dh, Math.round(srcH / 2));
    const step = document.createElement('canvas');
    step.width = nextW;
    step.height = nextH;
    const stepCtx = step.getContext('2d');
    stepCtx.imageSmoothingEnabled = true;
    stepCtx.imageSmoothingQuality = 'high';
    stepCtx.drawImage(src, 0, 0, srcW, srcH, 0, 0, nextW, nextH);
    src = step;
    srcW = nextW;
    srcH = nextH;
  }
  ctx.imageSmoothingEnabled = true;
  ctx.imageSmoothingQuality = 'high';
  ctx.drawImage(src, 0, 0, srcW, srcH, 0, 0, dw, dh);
}

function renderQuantizedPng(img, maxLong) {
  const natW = img.naturalWidth || img.width;
  const natH = img.naturalHeight || img.height;
  const scale = Math.min(1, maxLong / Math.max(natW, natH));
  const w = Math.max(1, Math.round(natW * scale));
  const h = Math.max(1, Math.round(natH * scale));
  const canvas = document.createElement('canvas');
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext('2d');
  drawScaledSmooth(ctx, img, w, h);
  quantizeCanvas(ctx, w, h);
  return { dataUrl: canvas.toDataURL('image/png'), width: w, height: h };
}

async function processImageFile(file) {
  const img = await decodeImageFile(file);
  const natMax = Math.max(img.naturalWidth || img.width, img.naturalHeight || img.height);
  const candidates = [natMax, ...DOWNSCALE_STEPS.filter((s) => s < natMax)];
  let result = null;
  for (const maxLong of candidates) {
    result = renderQuantizedPng(img, maxLong);
    if (result.dataUrl.length <= STORAGE_TARGET_CHARS) break;
  }
  return result;
}

async function onFileChosen() {
  const file = settingsEls.fileInput.files && settingsEls.fileInput.files[0];
  settingsEls.fileInput.value = '';
  if (!file) return;
  showImageError(null);
  settingsEls.imageFilename.textContent = 'よみこみちゅう…';
  settingsEls.imageFilename.title = '';
  settingsEls.imageSize.textContent = '';
  try {
    const { dataUrl, width, height } = await processImageFile(file);
    const display = (pendingState.image && pendingState.image.display) || 'center';
    pendingState = { ...pendingState, image: { dataUrl, width, height, display, filename: file.name } };
    syncImagePanel();
    updatePreview();
    updateDirtyUI();
  } catch (err) {
    showImageError(`画像を読み込めませんでした。（${err && err.message ? err.message : '不明なエラー'}）`);
    syncImagePanel();
  }
}

function stopFlash() {
  if (flashTimer) {
    clearInterval(flashTimer);
    flashTimer = null;
  }
  settingsEls.titlebar.classList.remove('is-inactive');
  /* The overlay click dropped focus to body. */
  if (!settingsEls.dialog.hidden && !settingsEls.dialog.contains(document.activeElement)) settingsEls.dialog.focus();
}

function flashTitlebar() {
  stopFlash();
  flashCount = 0;
  flashTimer = setInterval(() => {
    flashCount += 1;
    settingsEls.titlebar.classList.toggle('is-inactive');
    if (flashCount >= FLASH_TOGGLES) stopFlash();
  }, FLASH_INTERVAL_MS);
}

function getFocusable() {
  return Array.from(settingsEls.dialog.querySelectorAll('button, input, [tabindex]:not([tabindex="-1"])')).filter(
    (node) => !node.disabled && node.offsetParent !== null,
  );
}

/* Popup keys are handled here because this capture-phase listener runs before the combobox's own. */
function onDialogKeydown(ev) {
  if (comboOpen && ev.key !== 'Tab') {
    handleComboKeydown(ev);
    return;
  }
  if (comboOpen && ev.key === 'Tab') closeCombo();
  if (ev.key === 'Escape') {
    ev.preventDefault();
    cancelAndClose();
    return;
  }
  if (ev.key === 'Tab') {
    const focusable = getFocusable();
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (ev.shiftKey && document.activeElement === first) {
      ev.preventDefault();
      last.focus();
    } else if (!ev.shiftKey && document.activeElement === last) {
      ev.preventDefault();
      first.focus();
    }
    return;
  }
  if (ev.key === 'Enter') {
    if (ev.target instanceof HTMLElement && ev.target.tagName === 'BUTTON') return;
    ev.preventDefault();
    okAndClose();
  }
}

function screenSize() {
  const rect = settingsEls.deskScreen.getBoundingClientRect();
  return { width: rect.width, height: rect.height };
}

function clampDialogPos(x, y, w, size) {
  const cx = clampNum(x, MIN_VISIBLE_TITLEBAR - w, Math.max(0, size.width - MIN_VISIBLE_TITLEBAR));
  const cy = clampNum(y, 0, Math.max(0, size.height - TITLEBAR_H));
  return { x: cx, y: cy };
}

let dialogUserPositioned = false;
let dialogGeom = { x: 0, y: 0 };

function setDialogPos(x, y) {
  dialogGeom = { x, y };
  settingsEls.dialog.style.left = `${Math.round(x)}px`;
  settingsEls.dialog.style.top = `${Math.round(y)}px`;
}

function centerDialog() {
  setDialogPos(0, 0);
  const size = screenSize();
  const w = settingsEls.dialog.offsetWidth;
  const h = settingsEls.dialog.offsetHeight;
  const pos = clampDialogPos((size.width - w) / 2, (size.height - h) / 2, w, size);
  setDialogPos(pos.x, pos.y);
}

function placeDialog() {
  if (!dialogUserPositioned) {
    centerDialog();
    return;
  }
  const size = screenSize();
  const w = settingsEls.dialog.offsetWidth;
  const pos = clampDialogPos(dialogGeom.x, dialogGeom.y, w, size);
  setDialogPos(pos.x, pos.y);
}

function reflowDialog() {
  if (settingsEls.dialog.hidden) return;
  const size = screenSize();
  if (size.width === 0 || size.height === 0) return;
  placeDialog();
}

function openDialog() {
  pendingState = cloneWallpaper(savedState);
  settingsEls.overlay.hidden = false;
  settingsEls.dialog.hidden = false;
  syncFormFromState();
  placeDialog();
  settingsEls.dialog.focus();
  document.addEventListener('keydown', onDialogKeydown, true);
  onActivationChange();
}

function closeDialog() {
  closeCombo();
  settingsEls.overlay.hidden = true;
  settingsEls.dialog.hidden = true;
  stopFlash();
  document.removeEventListener('keydown', onDialogKeydown, true);
  settingsEls.icon.focus();
  onActivationChange();
}

function applyState(state) {
  savedState = state;
  applyBackground(settingsEls.wallpaper, savedState, { preview: false });
}

function tryApply() {
  const canonical = canonicalWallpaper(pendingState);
  const ok = saveWallpaper(canonical);
  if (!ok) {
    showStorageError(true);
    return false;
  }
  showStorageError(false);
  applyState(canonical);
  pendingState = cloneWallpaper(canonical);
  updateDirtyUI();
  return true;
}

function okAndClose() {
  if (isDirty() && !tryApply()) return;
  closeDialog();
}

function applyOnly() {
  const hadFocus = settingsEls.dialog.contains(document.activeElement);
  tryApply();
  /* Disabling the focused button drops focus to body. */
  if (hadFocus && !settingsEls.dialog.contains(document.activeElement)) settingsEls.okBtn.focus();
}

function cancelAndClose() {
  pendingState = cloneWallpaper(savedState);
  closeDialog();
}

function activateTab(id) {
  for (const btn of settingsEls.tabButtons) {
    const active = btn.dataset.tab === id;
    btn.classList.toggle('is-active', active);
    btn.setAttribute('aria-selected', String(active));
  }
  for (const panel of settingsEls.tabPanels) {
    panel.hidden = panel.id !== `desk-tabpanel-${id}`;
  }
}

function selectIcon(target) {
  for (const iconEl of document.querySelectorAll('.desk-icon')) iconEl.classList.toggle('is-selected', iconEl === target);
}

function wireDrag() {
  const endDrag = (ev) => {
    if (!dragState) return;
    dragState = null;
    document.body.classList.remove('desk-no-select');
    try {
      settingsEls.titlebar.releasePointerCapture(ev.pointerId);
    } catch {}
  };
  settingsEls.titlebar.addEventListener('pointerdown', (ev) => {
    if (ev.target.closest('.desk-tbtn')) return;
    if (ev.button !== 0) return;
    closeCombo();
    dragState = {
      startX: ev.clientX,
      startY: ev.clientY,
      origX: parseFloat(settingsEls.dialog.style.left) || 0,
      origY: parseFloat(settingsEls.dialog.style.top) || 0,
    };
    document.body.classList.add('desk-no-select');
    ev.preventDefault();
    try {
      settingsEls.titlebar.setPointerCapture(ev.pointerId);
    } catch {}
  });
  settingsEls.titlebar.addEventListener('pointermove', (ev) => {
    if (!dragState) return;
    dialogUserPositioned = true;
    const size = screenSize();
    const w = settingsEls.dialog.offsetWidth;
    const pos = clampDialogPos(dragState.origX + (ev.clientX - dragState.startX), dragState.origY + (ev.clientY - dragState.startY), w, size);
    setDialogPos(pos.x, pos.y);
  });
  settingsEls.titlebar.addEventListener('pointerup', endDrag);
  settingsEls.titlebar.addEventListener('pointercancel', endDrag);

  new ResizeObserver(() => reflowDialog()).observe(settingsEls.deskScreen);
}

function wireEvents() {
  settingsEls.icon.addEventListener('dblclick', (ev) => {
    ev.stopPropagation();
    openDialog();
  });
  settingsEls.icon.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter' || ev.key === ' ') {
      ev.preventDefault();
      selectIcon(settingsEls.icon);
      openDialog();
    }
  });

  settingsEls.overlay.addEventListener('click', () => flashTitlebar());
  settingsEls.dialog.addEventListener('focusin', () => selectIcon(null));
  /* Keyed on `disabled` so an ordinary blur such as an overlay click is left alone. */
  settingsEls.dialog.addEventListener('focusout', (ev) => {
    if (!(ev.target instanceof HTMLElement) || !ev.target.disabled) return;
    requestAnimationFrame(() => {
      if (!settingsEls.dialog.hidden && !settingsEls.dialog.contains(document.activeElement)) settingsEls.dialog.focus();
    });
  });

  settingsEls.closeBtn.addEventListener('click', cancelAndClose);
  settingsEls.cancelBtn.addEventListener('click', cancelAndClose);
  settingsEls.okBtn.addEventListener('click', okAndClose);
  settingsEls.applyBtn.addEventListener('click', applyOnly);

  for (const btn of settingsEls.tabButtons) {
    btn.addEventListener('click', () => activateTab(btn.dataset.tab));
  }

  settingsEls.customColor.addEventListener('input', () => pickColor(settingsEls.customColor.value));
  settingsEls.colorResetBtn.addEventListener('click', resetColor);

  settingsEls.browseBtn.addEventListener('click', () => settingsEls.fileInput.click());
  settingsEls.fileInput.addEventListener('change', onFileChosen);
  settingsEls.removeBtn.addEventListener('click', removeImage);

  settingsEls.comboField.addEventListener('click', () => {
    if (comboOpen) closeCombo();
    else openCombo();
  });
  settingsEls.comboField.addEventListener('keydown', (ev) => {
    if (comboOpen) return;
    if (ev.key === 'ArrowDown' || ev.key === 'ArrowUp') {
      ev.preventDefault();
      openCombo();
    }
  });
  for (const opt of settingsEls.comboOptions) {
    opt.addEventListener('click', () => commitCombo(opt.dataset.value));
    opt.addEventListener('mouseenter', () => highlightComboIndex(settingsEls.comboOptions.indexOf(opt)));
  }

  wireDrag();
}

export const DesktopSettings = {
  init(onChange) {
    onActivationChange = onChange || onActivationChange;
    savedState = loadSavedWallpaper();
    pendingState = cloneWallpaper(savedState);
    buildSwatches();
    wireEvents();
    applyBackground(settingsEls.wallpaper, savedState, { preview: false });
  },
  applyStoredWallpaper() {
    applyBackground(settingsEls.wallpaper, savedState, { preview: false });
  },
  isDialogOpen() {
    return !settingsEls.dialog.hidden;
  },
  syncActive() {
    if (flashTimer) return;
    settingsEls.titlebar.classList.toggle('is-inactive', !settingsEls.dialog.contains(document.activeElement));
  },
};
