export const TITLEBAR_H = 26;
export const MIN_VISIBLE_TITLEBAR = 60;

export function clampNum(v, lo, hi) {
  return Math.min(hi, Math.max(lo, v));
}

export function screenSize(screen) {
  const rect = screen.getBoundingClientRect();
  return { width: rect.width, height: rect.height };
}

export function clampPosition(x, y, w, size) {
  const cx = clampNum(x, MIN_VISIBLE_TITLEBAR - w, Math.max(0, size.width - MIN_VISIBLE_TITLEBAR));
  const cy = clampNum(y, 0, Math.max(0, size.height - TITLEBAR_H));
  return { x: cx, y: cy };
}

/* No active pointer to capture/release for synthetic events. */
export function trackPointer(el, downEv, onMove, onEnd) {
  document.body.classList.add('desk-no-select');
  try {
    el.setPointerCapture(downEv.pointerId);
  } catch {}
  const move = (ev) => onMove(ev);
  const end = (ev) => {
    el.removeEventListener('pointermove', move);
    el.removeEventListener('pointerup', end);
    el.removeEventListener('pointercancel', end);
    document.body.classList.remove('desk-no-select');
    try {
      el.releasePointerCapture(ev.pointerId);
    } catch {}
    if (onEnd) onEnd(ev);
  };
  el.addEventListener('pointermove', move);
  el.addEventListener('pointerup', end);
  el.addEventListener('pointercancel', end);
}
