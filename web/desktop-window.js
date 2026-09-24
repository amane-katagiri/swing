/* Geometry is kept only in memory (not localStorage) and recomputed from `.desk-screen`'s current size, so it survives sidebar collapse/expand and resize without persisting anything. */
const MIN_W = 320;
const MIN_H = 240;
const SCREEN_PAD = 18;
const ICON_COLUMN = 108;
const TITLEBAR_H = 26;
const MIN_VISIBLE_TITLEBAR = 60;
const UNMAXIMIZE_DRAG_THRESHOLD = 4;

const WIN_TEXT = {
  maximize: '最大化',
  restore: '元に戻す (縮小)',
};

const winEls = {
  screen: document.getElementById('desk-screen'),
  win: document.getElementById('desk-window'),
  titlebar: document.getElementById('desk-titlebar'),
  titlebarText: document.getElementById('desk-titlebar-text'),
  btnMin: document.getElementById('desk-btn-min'),
  btnMax: document.getElementById('desk-btn-max'),
  glyphMax: document.getElementById('desk-glyph-max'),
  btnClose: document.getElementById('desk-btn-close'),
  taskbtn: document.getElementById('desk-taskbtn'),
  iconExplorer: document.getElementById('desk-icon-explorer'),
  icons: Array.from(document.querySelectorAll('.desk-icon')),
  resizeHandles: Array.from(document.querySelectorAll('.desk-resize')),
};

const winState = {
  geom: { x: ICON_COLUMN, y: SCREEN_PAD, w: 640, h: 480 },
  maximized: false,
  minimized: false,
  closed: false,
  initialized: false,
  userPositioned: false,
};

function isNarrowLayout() {
  return window.matchMedia('(max-width: 760px)').matches;
}

function clampNum(v, lo, hi) {
  return Math.min(hi, Math.max(lo, v));
}

function screenSize() {
  const rect = winEls.screen.getBoundingClientRect();
  return { width: rect.width, height: rect.height };
}

function defaultGeometry(size) {
  const maxAllowedW = Math.max(MIN_W, size.width - ICON_COLUMN * 2);
  const w = clampNum(Math.min(920, maxAllowedW), MIN_W, Math.max(MIN_W, size.width));
  const h = clampNum(size.height - SCREEN_PAD * 2, MIN_H, Math.max(MIN_H, size.height));
  const x = Math.max(0, (size.width - w) / 2);
  return { x, y: SCREEN_PAD, w, h };
}

function clampGeometry(g, size) {
  const w = clampNum(g.w, MIN_W, Math.max(MIN_W, size.width));
  const h = clampNum(g.h, MIN_H, Math.max(MIN_H, size.height));
  const x = clampNum(g.x, MIN_VISIBLE_TITLEBAR - w, Math.max(0, size.width - MIN_VISIBLE_TITLEBAR));
  const y = clampNum(g.y, 0, Math.max(0, size.height - TITLEBAR_H));
  return { x, y, w, h };
}

/* Snapped to whole pixels: a half-pixel edge blurs the bitmap font inside the window and in the page frame. */
function applyWindowGeometry() {
  winEls.win.classList.toggle('is-maximized', winState.maximized);
  if (winState.maximized) {
    const size = screenSize();
    winEls.win.style.left = '0px';
    winEls.win.style.top = '0px';
    winEls.win.style.width = `${Math.floor(size.width)}px`;
    winEls.win.style.height = `${Math.floor(size.height)}px`;
    return;
  }
  winEls.win.style.left = `${Math.round(winState.geom.x)}px`;
  winEls.win.style.top = `${Math.round(winState.geom.y)}px`;
  winEls.win.style.width = `${Math.round(winState.geom.w)}px`;
  winEls.win.style.height = `${Math.round(winState.geom.h)}px`;
}

function updateMaxGlyph() {
  const restore = winState.maximized;
  winEls.glyphMax.classList.toggle('desk-glyph-max', !restore);
  winEls.glyphMax.classList.toggle('desk-glyph-restore', restore);
  const label = restore ? WIN_TEXT.restore : WIN_TEXT.maximize;
  winEls.btnMax.setAttribute('aria-label', label);
  winEls.btnMax.title = label;
}

function renderWindow() {
  applyWindowGeometry();
  const shown = !winState.minimized && !winState.closed;
  winEls.win.hidden = !shown;
  winEls.taskbtn.hidden = winState.closed;
  winEls.taskbtn.classList.toggle('is-active', shown);
  winEls.taskbtn.setAttribute('aria-pressed', String(shown));
  updateMaxGlyph();
}

function ensureWindowInitialized() {
  if (winState.initialized) return;
  const size = screenSize();
  if (size.width === 0 || size.height === 0) return;
  winState.geom = clampGeometry(defaultGeometry(size), size);
  winState.initialized = true;
  renderWindow();
}

function reflowWindow() {
  if (!winState.initialized) {
    ensureWindowInitialized();
    return;
  }
  const size = screenSize();
  if (size.width === 0 || size.height === 0) return;
  const base = winState.userPositioned || winState.maximized ? winState.geom : defaultGeometry(size);
  winState.geom = clampGeometry(base, size);
  applyWindowGeometry();
}

function minimizeWindow() {
  winState.minimized = true;
  renderWindow();
}

function closeWindow() {
  winState.closed = true;
  winState.minimized = false;
  renderWindow();
}

function openWindowFromIcon() {
  if (winState.closed) {
    const size = screenSize();
    winState.geom = clampGeometry(defaultGeometry(size), size);
    winState.closed = false;
    winState.minimized = false;
    winState.initialized = true;
    winState.userPositioned = false;
  } else if (winState.minimized) {
    winState.minimized = false;
  }
  renderWindow();
}

function toggleMaximized() {
  if (isNarrowLayout()) return;
  winState.maximized = !winState.maximized;
  renderWindow();
}

function selectDesktopIcon(icon) {
  for (const other of winEls.icons) other.classList.toggle('is-selected', other === icon);
}

function wireWindowChrome() {
  winEls.btnMin.addEventListener('click', () => minimizeWindow());
  winEls.btnMax.addEventListener('click', () => toggleMaximized());
  winEls.btnClose.addEventListener('click', () => closeWindow());

  winEls.taskbtn.addEventListener('click', () => {
    if (winState.closed) return;
    winState.minimized = !winState.minimized;
    renderWindow();
  });

  winEls.titlebar.addEventListener('dblclick', (ev) => {
    if (ev.target.closest('.desk-tbtn')) return;
    toggleMaximized();
  });

  let drag = null;
  winEls.titlebar.addEventListener('pointerdown', (ev) => {
    if (isNarrowLayout()) return;
    if (ev.target.closest('.desk-tbtn')) return;
    if (ev.button !== 0) return;
    drag = {
      startX: ev.clientX,
      startY: ev.clientY,
      origX: winState.geom.x,
      origY: winState.geom.y,
      fromMaximized: winState.maximized,
    };
    document.body.classList.add('desk-no-select');
    ev.preventDefault();
    try {
      winEls.titlebar.setPointerCapture(ev.pointerId);
    } catch {
      /* no active pointer to capture (e.g. synthetic events) */
    }
  });
  winEls.titlebar.addEventListener('pointermove', (ev) => {
    if (!drag) return;
    const size = screenSize();
    if (drag.fromMaximized) {
      if (Math.hypot(ev.clientX - drag.startX, ev.clientY - drag.startY) < UNMAXIMIZE_DRAG_THRESHOLD) return;
      const rect = winEls.screen.getBoundingClientRect();
      const ratio = clampNum((drag.startX - rect.left) / Math.max(1, rect.width), 0, 1);
      drag.origX = drag.startX - rect.left - winState.geom.w * ratio;
      drag.origY = drag.startY - rect.top - TITLEBAR_H / 2;
      drag.fromMaximized = false;
      winState.maximized = false;
      renderWindow();
    }
    winState.userPositioned = true;
    winState.geom = clampGeometry(
      { ...winState.geom, x: drag.origX + (ev.clientX - drag.startX), y: drag.origY + (ev.clientY - drag.startY) },
      size,
    );
    applyWindowGeometry();
  });
  const endDrag = (ev) => {
    if (!drag) return;
    drag = null;
    document.body.classList.remove('desk-no-select');
    try {
      winEls.titlebar.releasePointerCapture(ev.pointerId);
    } catch {
      /* pointer capture already released */
    }
  };
  winEls.titlebar.addEventListener('pointerup', endDrag);
  winEls.titlebar.addEventListener('pointercancel', endDrag);

  for (const handle of winEls.resizeHandles) {
    const dir = handle.dataset.dir;
    handle.addEventListener('pointerdown', (ev) => {
      if (winState.maximized || isNarrowLayout()) return;
      if (ev.button !== 0) return;
      ev.preventDefault();
      const start = { ...winState.geom };
      const startX = ev.clientX;
      const startY = ev.clientY;
      document.body.classList.add('desk-no-select');
      try {
        handle.setPointerCapture(ev.pointerId);
      } catch {
        /* no active pointer to capture (e.g. synthetic events) */
      }

      const onMove = (mv) => {
        const dx = mv.clientX - startX;
        const dy = mv.clientY - startY;
        let { x, y, w, h } = start;
        if (dir.includes('e')) w = start.w + dx;
        if (dir.includes('s')) h = start.h + dy;
        if (dir.includes('w')) {
          w = start.w - dx;
          x = start.x + dx;
        }
        if (dir.includes('n')) {
          h = start.h - dy;
          y = start.y + dy;
        }
        if (w < MIN_W) {
          if (dir.includes('w')) x -= MIN_W - w;
          w = MIN_W;
        }
        if (h < MIN_H) {
          if (dir.includes('n')) y -= MIN_H - h;
          h = MIN_H;
        }
        winState.userPositioned = true;
        winState.geom = clampGeometry({ x, y, w, h }, screenSize());
        applyWindowGeometry();
      };
      const onUp = (up) => {
        handle.removeEventListener('pointermove', onMove);
        handle.removeEventListener('pointerup', onUp);
        handle.removeEventListener('pointercancel', onUp);
        document.body.classList.remove('desk-no-select');
        try {
          handle.releasePointerCapture(up.pointerId);
        } catch {
          /* pointer capture already released */
        }
      };
      handle.addEventListener('pointermove', onMove);
      handle.addEventListener('pointerup', onUp);
      handle.addEventListener('pointercancel', onUp);
    });
  }

  for (const icon of winEls.icons) {
    icon.addEventListener('click', (ev) => {
      ev.stopPropagation();
      selectDesktopIcon(icon);
    });
  }
  winEls.iconExplorer.addEventListener('dblclick', (ev) => {
    ev.stopPropagation();
    openWindowFromIcon();
  });
  winEls.iconExplorer.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter' || ev.key === ' ') {
      ev.preventDefault();
      selectDesktopIcon(winEls.iconExplorer);
      openWindowFromIcon();
    }
  });
  winEls.screen.addEventListener('click', () => selectDesktopIcon(null));

  new ResizeObserver(() => reflowWindow()).observe(winEls.screen);
}

export const DesktopWindow = {
  init() {
    wireWindowChrome();
  },
  reflow() {
    reflowWindow();
  },
  reveal() {
    winEls.win.classList.remove('is-loading');
  },
  deselectIcons() {
    selectDesktopIcon(null);
  },
};
