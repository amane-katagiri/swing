const view = document.getElementById('view-desktop');
const frame = document.getElementById('desk-page-frame');
let snapped = 1;
let scale = 1;
let frameExempt = false;

export function toDeskPx(viewportPx) {
  return viewportPx / scale;
}

export function frameToViewport(frameEl, x, y) {
  const rect = frameEl.getBoundingClientRect();
  const win = frameEl.contentWindow;
  const ratio = win && win.innerWidth > 0 ? rect.width / win.innerWidth : 1;
  return { x: rect.left + x * ratio, y: rect.top + y * ratio };
}

function zoomValue(z) {
  return Math.abs(z - 1) < 1e-6 ? '' : String(z);
}

/* The epsilon absorbs ratios such as 1.9999999 that some browsers report at 200%. */
function applyScale() {
  const ratio = window.devicePixelRatio || 1;
  snapped = Math.max(1, Math.floor(ratio + 1e-6));
  scale = snapped / ratio;
  view.style.zoom = zoomValue(scale);
  scaleFrame();
  alignOrigins();
}

function frameRoot() {
  try {
    return frame && frame.contentDocument ? frame.contentDocument.documentElement : null;
  } catch {
    return null;
  }
}

/* Firefox draws an iframe under an ancestor zoom off the pixel grid and WebKit shrinks its viewport by that zoom, so there the frame cancels it and zooms its own document. */
function scaleFrame() {
  const root = frameRoot();
  if (!root) return;
  frame.style.zoom = '';
  root.style.zoom = '';
  frame.getBoundingClientRect();
  const win = frame.contentWindow;
  const offGrid = Math.abs((win.devicePixelRatio || 1) - snapped) > 1e-3;
  const shrunk = root.clientHeight < (win.innerHeight * (1 + scale)) / 2;
  frameExempt = offGrid || shrunk;
  if (!frameExempt) return;
  frame.style.zoom = zoomValue(1 / scale);
  root.style.zoom = zoomValue(scale);
}

/* An integer ratio alone still blurs when the box starts mid-pixel, as the sidebar width or the text above the frame can leave it. */
function alignOrigin(el, devicePerPx) {
  el.style.left = '';
  el.style.top = '';
  const rect = el.getBoundingClientRect();
  if (rect.width === 0 && rect.height === 0) return;
  const ratio = window.devicePixelRatio || 1;
  const offset = (v) => {
    const device = v * ratio;
    return (Math.round(device) - device) / devicePerPx;
  };
  const dx = offset(rect.left);
  const dy = offset(rect.top);
  if (Math.abs(dx) > 1e-3) el.style.left = `${dx}px`;
  if (Math.abs(dy) > 1e-3) el.style.top = `${dy}px`;
}

function alignOrigins() {
  alignOrigin(view, snapped);
  if (frame) alignOrigin(frame, frameExempt ? window.devicePixelRatio || 1 : snapped);
}

function watchRatio() {
  const query = window.matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
  query.addEventListener(
    'change',
    () => {
      applyScale();
      watchRatio();
    },
    { once: true },
  );
}

export function initDeskScale() {
  applyScale();
  watchRatio();
  window.addEventListener('resize', applyScale);
  const observer = new ResizeObserver(() => {
    scaleFrame();
    alignOrigins();
  });
  observer.observe(view);
  if (frame) {
    observer.observe(frame);
    frame.addEventListener('load', () => {
      scaleFrame();
      alignOrigins();
    });
  }
}
