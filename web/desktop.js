import { storage } from './storage.js';
import { cache, el, apiFetch, describeError, createLoadGuard, setBusy, sanitizeMessage, maybeLink } from './util.js';

const DESK_TEXT = {
  loading: 'よみこみちゅう…',
  doneStatus: '完了',
  errorStatus: 'エラー',
  error: (detail) => `リンク一覧を読み込めませんでした。（${detail}）`,
  empty: 'まだリンクがありません。ミラーしているサイトはまだ無いようです。',
  marqueeEmpty: 'まだ何もミラーしていません…また今度見てね！',
  marqueeLatest: (date, title) => `最新更新情報: ${date} ${title}`,
  iconMismatchDesc: 'NIP-05 不一致: このドメインの nostr.json に "_" が無いか、別の鍵を指しています',
  iconErrorDesc: 'NIP-05 確認失敗: nostr.json を取得・解釈できませんでした',
  iconNewDesc: '7日以内に更新されました',
  iconUpDesc: '30日以内に更新されました',
  iconDefaultDesc: '最近の更新はありません',
  openSite: 'サイトを開く',
  openGateway: 'ゲートウェイで開く',
  maximize: '最大化',
  restore: '元に戻す (縮小)',
};

const deskEls = {
  page: document.getElementById('desk-page'),
  status: document.getElementById('desk-page-status'),
  list: document.getElementById('desk-link-list'),
  marquee: document.getElementById('desk-marquee-text'),
  counter: document.getElementById('desk-counter'),
  clock: document.getElementById('desk-clock'),
  statusText: document.getElementById('desk-status-text'),
  reloadBtn: document.getElementById('desk-reload'),
};

const NEW_DAYS = 7;
const UP_DAYS = 30;
const SECONDS_PER_DAY = 86400;

function collectStoredSites(data) {
  const list = [];
  for (const acct of data.accounts || []) {
    for (const site of acct.sites || []) {
      if (site.stored) list.push(site);
    }
  }
  if (data.unfollowed && Array.isArray(data.unfollowed.accounts)) {
    for (const acct of data.unfollowed.accounts) {
      for (const site of acct.sites || []) {
        if (site.stored) list.push(site);
      }
    }
  }
  list.sort((a, b) => (b.created_at || 0) - (a.created_at || 0));
  return list;
}

function formatRetroDate(sec) {
  if (sec == null) return '----.--.--';
  const d = new Date(sec * 1000);
  const yyyy = d.getFullYear();
  const mm = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  return `${yyyy}.${mm}.${dd}`;
}

function statusIcon(site) {
  if (site.nip05 === 'mismatch') {
    return el('span', { class: 'desk-status-icon', 'data-status': 'ng', title: DESK_TEXT.iconMismatchDesc, 'aria-label': DESK_TEXT.iconMismatchDesc }, '✕');
  }
  if (site.nip05 === 'error') {
    return el('span', { class: 'desk-status-icon', 'data-status': 'err', title: DESK_TEXT.iconErrorDesc, 'aria-label': DESK_TEXT.iconErrorDesc }, '?');
  }
  const recognized = site.nip05 == null || site.nip05 === 'verified' || site.nip05 === 'not_applicable';
  if (!recognized) {
    return el('span', { class: 'desk-status-icon', 'data-status': 'err', title: DESK_TEXT.iconErrorDesc, 'aria-label': DESK_TEXT.iconErrorDesc }, '?');
  }
  const ageDays = site.created_at == null ? Infinity : (Date.now() / 1000 - site.created_at) / SECONDS_PER_DAY;
  if (ageDays <= NEW_DAYS) {
    return el('span', { class: 'desk-status-icon', 'data-status': 'new', title: DESK_TEXT.iconNewDesc, 'aria-label': DESK_TEXT.iconNewDesc }, 'NEW');
  }
  if (ageDays <= UP_DAYS) {
    return el('span', { class: 'desk-status-icon', 'data-status': 'up', title: DESK_TEXT.iconUpDesc, 'aria-label': DESK_TEXT.iconUpDesc }, 'UP');
  }
  return el('span', { class: 'desk-status-icon', 'data-status': 'default', title: DESK_TEXT.iconDefaultDesc, 'aria-label': DESK_TEXT.iconDefaultDesc }, '★');
}

function buildLinkRow(site) {
  const li = el('li', { class: 'desk-link-row' });
  li.append(statusIcon(site));
  li.append(el('span', { class: 'desk-link-date' }, formatRetroDate(site.created_at)));

  const cleanTitle = sanitizeMessage(site.title, 120);
  const titleText = cleanTitle || site.d;
  const primaryHref = site.gateway_url || site.url || null;
  let titleNode;
  if (primaryHref) {
    titleNode = maybeLink(primaryHref, titleText);
    titleNode.title = site.gateway_url ? DESK_TEXT.openGateway : DESK_TEXT.openSite;
  } else {
    titleNode = el('span', {}, titleText);
  }
  titleNode.classList.add('desk-link-title');
  li.append(titleNode);

  if (cleanTitle) {
    li.append(el('span', { class: 'desk-link-d' }, `(${site.d})`));
  }

  if (site.gateway_url && site.url) {
    const honke = maybeLink(site.url, '[本家]');
    honke.classList.add('desk-link-honke');
    honke.setAttribute('aria-label', `${titleText} — ${DESK_TEXT.openSite}`);
    li.append(honke);
  }

  const msg = sanitizeMessage(site.message);
  if (msg) {
    li.append(el('p', { class: 'desk-link-message' }, `「${msg}」`));
  }

  return li;
}

function renderPageStatus(kind, message) {
  if (!message) {
    deskEls.status.removeAttribute('data-kind');
    deskEls.status.textContent = '';
    return;
  }
  deskEls.status.dataset.kind = kind;
  deskEls.status.textContent = message;
}

function renderMarquee(sites) {
  if (sites.length === 0) {
    deskEls.marquee.textContent = DESK_TEXT.marqueeEmpty;
    return;
  }
  const top = sites[0];
  const label = sanitizeMessage(top.title, 60) || top.d;
  deskEls.marquee.textContent = DESK_TEXT.marqueeLatest(formatRetroDate(top.created_at), label);
}

function renderCounter(sites) {
  const visits = Number.parseInt(storage.get('swing:desktop:visits', '1'), 10) || 1;
  const base = 1000 + sites.length * 37;
  deskEls.counter.textContent = String(base + visits).padStart(6, '0');
}

function bumpVisitCounter() {
  const n = (Number.parseInt(storage.get('swing:desktop:visits', '0'), 10) || 0) + 1;
  storage.set('swing:desktop:visits', String(n));
}

function updateClock() {
  if (!deskEls.clock) return;
  const now = new Date();
  const hh = String(now.getHours()).padStart(2, '0');
  const mm = String(now.getMinutes()).padStart(2, '0');
  deskEls.clock.textContent = `${hh}:${mm}`;
}

const desktopLoadGuard = createLoadGuard();

/*
 * Window manager for the "SWING Explorer" chrome window. Geometry lives only
 * in this module's memory (no localStorage) and is recomputed from the
 * `.desk-screen` container's current size, so it survives sidebar
 * collapse/expand and browser resize without needing to persist anything.
 */
const MIN_W = 320;
const MIN_H = 240;
const SCREEN_PAD = 18;
const ICON_COLUMN = 108;
const TITLEBAR_H = 26;
const MIN_VISIBLE_TITLEBAR = 60;
const UNMAXIMIZE_DRAG_THRESHOLD = 4;

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

function applyWindowGeometry() {
  winEls.win.classList.toggle('is-maximized', winState.maximized);
  if (winState.maximized) {
    winEls.win.style.left = '0px';
    winEls.win.style.top = '0px';
    winEls.win.style.width = '100%';
    winEls.win.style.height = '100%';
    return;
  }
  winEls.win.style.left = `${winState.geom.x}px`;
  winEls.win.style.top = `${winState.geom.y}px`;
  winEls.win.style.width = `${winState.geom.w}px`;
  winEls.win.style.height = `${winState.geom.h}px`;
}

function updateMaxGlyph() {
  const restore = winState.maximized;
  winEls.glyphMax.classList.toggle('desk-glyph-max', !restore);
  winEls.glyphMax.classList.toggle('desk-glyph-restore', restore);
  const label = restore ? DESK_TEXT.restore : DESK_TEXT.maximize;
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

export const DesktopView = {
  init() {
    if (deskEls.reloadBtn) {
      deskEls.reloadBtn.addEventListener('click', () => this.load(true, deskEls.reloadBtn));
    }
    bumpVisitCounter();
    updateClock();
    setInterval(updateClock, 30000);
    wireWindowChrome();
  },
  onShow() {
    if (cache.sites) this.render();
    else this.load();
    requestAnimationFrame(() => reflowWindow());
  },
  async load(force, reloadBtn) {
    if (!force && cache.sites) return this.render();
    const gen = desktopLoadGuard.start();
    const firstLoad = !cache.sites;
    if (reloadBtn) setBusy(reloadBtn, true);
    if (firstLoad) {
      renderPageStatus('loading', DESK_TEXT.loading);
      if (deskEls.statusText) deskEls.statusText.textContent = DESK_TEXT.loading;
    }
    let data;
    try {
      data = await apiFetch('/api/sites');
    } catch (err) {
      if (!desktopLoadGuard.isCurrent(gen)) return;
      cache.sites = null;
      renderPageStatus('error', DESK_TEXT.error(describeError(err)));
      if (deskEls.list) deskEls.list.replaceChildren();
      if (deskEls.statusText) deskEls.statusText.textContent = DESK_TEXT.errorStatus;
      return;
    } finally {
      if (reloadBtn) setBusy(reloadBtn, false);
    }
    if (!desktopLoadGuard.isCurrent(gen)) return;
    cache.sites = data;
    this.render();
  },
  render() {
    const data = cache.sites;
    if (!data) return;
    const sites = collectStoredSites(data);

    if (sites.length === 0) {
      renderPageStatus('empty', DESK_TEXT.empty);
    } else {
      renderPageStatus(null, null);
    }
    if (deskEls.list) {
      deskEls.list.replaceChildren(...sites.map(buildLinkRow));
    }
    renderMarquee(sites);
    renderCounter(sites);
    if (deskEls.statusText) deskEls.statusText.textContent = DESK_TEXT.doneStatus;
  },
};
