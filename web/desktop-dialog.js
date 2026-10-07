import { focusableIn, keepFocusOnDisable, refocusIfDropped, registerFrame, scheduleActiveSync, tabAcrossEdge } from './desktop-focus.js';
import { clampPosition, screenSize, trackPointer } from './desktop-drag.js';
import { toDeskPx } from './desktop-scale.js';

const FLASH_TOGGLES = 6;
const FLASH_INTERVAL_MS = 90;
const STORAGE_ERROR_TEXT = '保存できませんでした。ブラウザの保存容量が足りないようです。';

export function showStorageError(node, show) {
  node.hidden = !show;
  node.textContent = show ? STORAGE_ERROR_TEXT : '';
}

export function createDialog({ root, returnFocus, onOpen, onOk, onCancel, onApply, onKey }) {
  const titlebar = root.querySelector('.desk-titlebar');
  const screen = document.getElementById('desk-screen');
  const overlay = document.getElementById('desk-dialog-overlay');
  const okBtn = root.querySelector('[data-dialog-action="ok"]');

  let userPositioned = false;
  let geom = { x: 0, y: 0 };
  let flashTimer = null;
  let flashCount = 0;

  function setPos(x, y) {
    geom = { x, y };
    root.style.left = `${Math.round(x)}px`;
    root.style.top = `${Math.round(y)}px`;
  }

  function center() {
    setPos(0, 0);
    const size = screenSize(screen);
    const w = root.offsetWidth;
    const h = root.offsetHeight;
    const pos = clampPosition((size.width - w) / 2, (size.height - h) / 2, w, size);
    setPos(pos.x, pos.y);
  }

  function place() {
    if (!userPositioned) {
      center();
      return;
    }
    const size = screenSize(screen);
    const w = root.offsetWidth;
    const pos = clampPosition(geom.x, geom.y, w, size);
    setPos(pos.x, pos.y);
  }

  function reflow() {
    if (root.hidden) return;
    const size = screenSize(screen);
    if (size.width === 0 || size.height === 0) return;
    place();
  }

  function stopFlash() {
    if (flashTimer) {
      clearInterval(flashTimer);
      flashTimer = null;
    }
    titlebar.classList.remove('is-inactive');
    if (!root.hidden && !root.contains(document.activeElement)) root.focus();
  }

  function flash() {
    stopFlash();
    flashCount = 0;
    flashTimer = setInterval(() => {
      flashCount += 1;
      titlebar.classList.toggle('is-inactive');
      if (flashCount >= FLASH_TOGGLES) stopFlash();
    }, FLASH_INTERVAL_MS);
  }

  function ok() {
    if (!onOk || onOk()) close();
  }

  function cancel() {
    if (onCancel) onCancel();
    close();
  }

  function apply() {
    if (!onApply) return;
    refocusIfDropped(root, okBtn, onApply);
  }

  /* Runs before Tab/Escape/Enter so a page's own popup (e.g. a combobox) can claim the keystroke first. */
  function onKeydown(ev) {
    if (!root.contains(document.activeElement)) return;
    if (onKey && onKey(ev)) return;
    if (ev.key === 'Escape') {
      ev.preventDefault();
      cancel();
      return;
    }
    if (ev.key === 'Tab') {
      tabAcrossEdge(ev, focusableIn(root), document.activeElement);
      return;
    }
    if (ev.key === 'Enter') {
      if (ev.target instanceof HTMLElement && ev.target.tagName === 'BUTTON') return;
      ev.preventDefault();
      ok();
    }
  }

  function open() {
    overlay.hidden = false;
    root.hidden = false;
    if (onOpen) onOpen();
    place();
    root.focus();
    document.addEventListener('keydown', onKeydown, true);
    scheduleActiveSync();
  }

  function close() {
    overlay.hidden = true;
    root.hidden = true;
    stopFlash();
    document.removeEventListener('keydown', onKeydown, true);
    if (returnFocus) returnFocus.focus();
    scheduleActiveSync();
  }

  const actions = { ok, cancel, close: cancel, apply };
  for (const btn of root.querySelectorAll('[data-dialog-action]')) {
    const action = actions[btn.dataset.dialogAction];
    if (action) btn.addEventListener('click', action);
  }

  overlay.addEventListener('click', flash);
  keepFocusOnDisable(root);

  titlebar.addEventListener('pointerdown', (ev) => {
    if (ev.target.closest('.desk-tbtn')) return;
    if (ev.button !== 0) return;
    ev.preventDefault();
    if (!root.contains(document.activeElement)) root.focus();
    const start = {
      startX: ev.clientX,
      startY: ev.clientY,
      origX: parseFloat(root.style.left) || 0,
      origY: parseFloat(root.style.top) || 0,
    };
    trackPointer(titlebar, ev, (mv) => {
      userPositioned = true;
      const size = screenSize(screen);
      const w = root.offsetWidth;
      const pos = clampPosition(
        start.origX + toDeskPx(mv.clientX - start.startX),
        start.origY + toDeskPx(mv.clientY - start.startY),
        w,
        size,
      );
      setPos(pos.x, pos.y);
    });
  });

  new ResizeObserver(() => reflow()).observe(screen);
  registerFrame({ root, titlebar, modal: true, frozen: () => flashTimer !== null });

  return { open, close };
}
