export const FOCUSABLE = 'button, input, [tabindex]:not([tabindex="-1"])';

/* Not `[href]`: that also matches the decorative SVG `<use href>` icon references. */
export function focusableIn(root, { links = false } = {}) {
  const selector = links ? `a[href], ${FOCUSABLE}` : FOCUSABLE;
  return Array.from(root.querySelectorAll(selector)).filter((node) => !node.disabled && node.offsetParent !== null);
}

export function tabAcrossEdge(ev, list, active, edges = list) {
  if (ev.key !== 'Tab' || edges.length === 0) return;
  const edge = ev.shiftKey ? edges[0] : edges[edges.length - 1];
  if (active !== edge) return;
  const i = list.indexOf(edge);
  ev.preventDefault();
  list[(i + (ev.shiftKey ? -1 : 1) + list.length) % list.length].focus();
}

/* Keyed on `disabled` so an ordinary blur (a desktop/overlay click) is left alone. */
export function keepFocusOnDisable(root) {
  root.addEventListener('focusout', (ev) => {
    if (!(ev.target instanceof HTMLElement) || !ev.target.disabled) return;
    requestAnimationFrame(() => {
      if (!root.hidden && !root.contains(document.activeElement)) root.focus();
    });
  });
}

/* Disabling the just-clicked control drops focus to body. */
export function refocusIfDropped(root, fallback, fn) {
  const hadFocus = root.contains(document.activeElement);
  fn();
  if (hadFocus && !root.contains(document.activeElement)) fallback.focus();
}

const frames = [];

export function registerFrame(frame) {
  frames.push(frame);
}

export function isModalOpen() {
  return frames.some((frame) => frame.modal && !frame.root.hidden);
}

/* Win95-faithful default: the window starts active on boot, before anything has real focus. */
let hasHadRealFocus = false;
let syncScheduled = false;

export function scheduleActiveSync() {
  if (syncScheduled) return;
  syncScheduled = true;
  requestAnimationFrame(() => {
    syncScheduled = false;
    const focused = document.activeElement;
    const meaningful = focused && focused !== document.body && focused !== document.documentElement;
    if (meaningful) hasHadRealFocus = true;
    else if (!hasHadRealFocus) return;
    const openModal = frames.find((frame) => frame.modal && !frame.root.hidden);
    for (const frame of frames) {
      if (frame.frozen && frame.frozen()) continue;
      const active = openModal ? frame === openModal && frame.root.contains(focused) : frame.root.contains(focused);
      frame.titlebar.classList.toggle('is-inactive', !active);
    }
  });
}
