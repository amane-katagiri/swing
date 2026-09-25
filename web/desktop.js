import { storage } from './storage.js';
import { cache, el, apiFetch, describeError, createLoadGuard, setBusy, sanitizeMessage, sanitizeDisplayText, maybeLink } from './util.js';
import { DesktopWindow } from './desktop-window.js';
import { DesktopSettings } from './desktop-settings.js';
import { tabAcrossEdge, focusableIn, isModalOpen, scheduleActiveSync } from './desktop-focus.js';

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
};

const deskEls = {
  frame: document.getElementById('desk-page-frame'),
  clock: document.getElementById('desk-clock'),
  statusText: document.getElementById('desk-status-text'),
  reloadBtn: document.getElementById('desk-reload'),
  screen: document.getElementById('desk-screen'),
  icons: Array.from(document.querySelectorAll('.desk-icon')),
};

const LAUNCHERS = [
  { icon: document.getElementById('desk-icon-explorer'), open: () => DesktopWindow.open() },
  { icon: document.getElementById('desk-icon-control-panel'), open: () => DesktopSettings.open() },
];

function selectIcon(icon) {
  for (const other of deskEls.icons) other.classList.toggle('is-selected', other === icon);
}

function wireIconLaunchers() {
  for (const icon of deskEls.icons) {
    icon.addEventListener('click', (ev) => {
      ev.stopPropagation();
      selectIcon(icon);
    });
  }
  if (deskEls.screen) deskEls.screen.addEventListener('click', () => selectIcon(null));

  for (const { icon, open } of LAUNCHERS) {
    icon.addEventListener('dblclick', (ev) => {
      ev.stopPropagation();
      open();
    });
    icon.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter' || ev.key === ' ') {
        ev.preventDefault();
        selectIcon(icon);
        open();
      }
    });
  }

  document.addEventListener('focusin', (ev) => {
    if (ev.target instanceof Element && ev.target.closest('.desk-window')) selectIcon(null);
  });
}

/* Page markup is replaceable, so every element here is optional and looked up again whenever the frame document changes. */
const pageEls = { status: null, list: null, marquee: null, counter: null };

function pageDocument() {
  try {
    return deskEls.frame ? deskEls.frame.contentDocument : null;
  } catch {
    return null;
  }
}

function capturePageEls() {
  const doc = pageDocument();
  pageEls.status = doc ? doc.getElementById('desk-page-status') : null;
  pageEls.list = doc ? doc.getElementById('desk-link-list') : null;
  pageEls.marquee = doc ? doc.getElementById('desk-marquee-text') : null;
  pageEls.counter = doc ? doc.getElementById('desk-counter') : null;
  return doc;
}

const FRAME_CHROME_CSS = '/desktop-frame.css';
const FRAME_CHROME_ID = 'desk-frame-chrome';
let frameChromeReady = Promise.resolve();

const DESK_FONT = '12px PixelMplus12';
const DESK_FONT_BOLD = '700 12px PixelMplus12';
const REVEAL_TIMEOUT = 1500;

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
  const cleanD = sanitizeDisplayText(site.d);
  const titleText = cleanTitle || cleanD;
  const primaryHref = site.gateway_url || site.url || null;
  let titleNode;
  if (primaryHref) {
    titleNode = maybeLink(primaryHref, titleText);
    titleNode.title = site.gateway_url ? DESK_TEXT.openGateway : DESK_TEXT.openSite;
  } else {
    titleNode = el('span', {}, titleText);
  }
  titleNode.classList.add('desk-link-title');
  titleNode.setAttribute('dir', 'auto');
  li.append(titleNode);

  if (cleanTitle) {
    li.append(el('span', { class: 'desk-link-d', dir: 'auto' }, `(${cleanD})`));
  }

  if (site.gateway_url && site.url) {
    const honke = maybeLink(site.url, '[本家]');
    honke.classList.add('desk-link-honke');
    honke.setAttribute('aria-label', `${titleText} — ${DESK_TEXT.openSite}`);
    li.append(honke);
  }

  const msg = sanitizeMessage(site.message);
  if (msg) {
    li.append(el('p', { class: 'desk-link-message', dir: 'auto' }, `「${msg}」`));
  }

  return li;
}

function renderPageStatus(kind, message) {
  if (!pageEls.status) return;
  if (!message) {
    pageEls.status.removeAttribute('data-kind');
    pageEls.status.textContent = '';
    return;
  }
  pageEls.status.dataset.kind = kind;
  pageEls.status.textContent = message;
}

function renderMarquee(sites) {
  if (!pageEls.marquee) return;
  if (sites.length === 0) {
    pageEls.marquee.textContent = DESK_TEXT.marqueeEmpty;
    return;
  }
  const top = sites[0];
  const label = sanitizeMessage(top.title, 60) || sanitizeDisplayText(top.d);
  pageEls.marquee.textContent = DESK_TEXT.marqueeLatest(formatRetroDate(top.created_at), label);
}

function renderCounter(sites) {
  if (!pageEls.counter) return;
  const visits = Number.parseInt(storage.get('swing:desktop:visits', '1'), 10) || 1;
  const base = 1000 + sites.length * 37;
  pageEls.counter.textContent = String(base + visits).padStart(6, '0');
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

/* A parent document can't style a child iframe's scrollbar via CSS. */
function injectFrameChrome(doc) {
  if (!doc || !doc.head) return Promise.resolve();
  if (doc.getElementById(FRAME_CHROME_ID)) return Promise.resolve();
  return new Promise((resolve) => {
    const link = doc.createElement('link');
    link.id = FRAME_CHROME_ID;
    link.rel = 'stylesheet';
    link.href = FRAME_CHROME_CSS;
    link.addEventListener('load', () => resolve(), { once: true });
    link.addEventListener('error', () => resolve(), { once: true });
    doc.head.prepend(link);
  });
}

function wirePageFrame() {
  if (!deskEls.frame) return;
  const onLoad = () => {
    const doc = capturePageEls();
    frameChromeReady = injectFrameChrome(doc);
    if (doc) {
      doc.addEventListener('click', () => selectIcon(null));
      doc.addEventListener('keydown', onFrameKeydown, true);
    }
    scheduleActiveSync();
    if (cache.sites) DesktopView.render();
  };
  deskEls.frame.addEventListener('load', onLoad);
  const doc = pageDocument();
  if (doc && doc.readyState === 'complete') onLoad();
}

function outerDesktopFocusable() {
  return focusableIn(document.getElementById('view-desktop'));
}

function innerDesktopFocusable() {
  const doc = pageDocument();
  if (!doc || !doc.body) return [];
  return focusableIn(doc, { links: true });
}

function combinedDesktopFocusable() {
  const outer = outerDesktopFocusable();
  const inner = innerDesktopFocusable();
  if (!deskEls.frame || inner.length === 0) return outer;
  const before = [];
  const after = [];
  for (const node of outer) {
    (deskEls.frame.compareDocumentPosition(node) & Node.DOCUMENT_POSITION_PRECEDING ? before : after).push(node);
  }
  return [...before, ...inner, ...after];
}

function desktopTrapActive() {
  return document.body.dataset.view === 'desktop' && !isModalOpen();
}

function escapeDesktopToSideNav(ev) {
  const link = document.querySelector('a[data-route="desktop"]');
  if (!link) return;
  ev.preventDefault();
  link.focus();
}

function onDesktopKeydown(ev) {
  if (!desktopTrapActive()) return;
  const view = document.getElementById('view-desktop');
  if (!view.contains(document.activeElement)) return;
  if (ev.key === 'Escape') {
    escapeDesktopToSideNav(ev);
    return;
  }
  if (ev.key !== 'Tab') return;
  tabAcrossEdge(ev, combinedDesktopFocusable(), document.activeElement);
}

/* A separate listener because same-origin iframes still don't bubble keydown to the parent. */
function onFrameKeydown(ev) {
  if (!desktopTrapActive()) return;
  const doc = pageDocument();
  if (!doc) return;
  if (ev.key === 'Escape') {
    escapeDesktopToSideNav(ev);
    return;
  }
  if (ev.key !== 'Tab') return;
  const inner = innerDesktopFocusable();
  if (inner.length === 0) return;
  tabAcrossEdge(ev, combinedDesktopFocusable(), doc.activeElement, inner);
}

function pageFrameLoaded() {
  return new Promise((resolve) => {
    if (!deskEls.frame) {
      resolve();
      return;
    }
    const doc = pageDocument();
    if (doc && doc.readyState === 'complete' && doc.URL !== 'about:blank') {
      resolve();
      return;
    }
    deskEls.frame.addEventListener('load', () => resolve(), { once: true });
  });
}

/* The bundled pixel font is only requested once something using it is painted, so it is asked for up front instead. */
function loadDeskFont(doc) {
  if (!doc || !doc.fonts) return Promise.resolve();
  return Promise.all([doc.fonts.load(DESK_FONT), doc.fonts.load(DESK_FONT_BOLD)]).catch(() => {});
}

function warmDeskFonts() {
  loadDeskFont(document);
  pageFrameLoaded().then(() => loadDeskFont(pageDocument()));
}

function afterNextFrames() {
  return new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  });
}

/* Waits for the first real show (not startup) because Firefox gives a `display: none` iframe no layout, so its stylesheet/font only take effect once shown. */
let revealed = false;
function revealWindowWhenReady() {
  if (revealed) return;
  revealed = true;
  const ready = pageFrameLoaded()
    .then(() => Promise.all([frameChromeReady, loadDeskFont(document), loadDeskFont(pageDocument())]))
    .then(afterNextFrames)
    .catch(() => {});
  const fallback = new Promise((resolve) => setTimeout(resolve, REVEAL_TIMEOUT));
  Promise.race([ready, fallback]).then(DesktopWindow.reveal, DesktopWindow.reveal);
}

export const DesktopView = {
  init() {
    wirePageFrame();
    wireIconLaunchers();
    warmDeskFonts();
    if (deskEls.reloadBtn) {
      deskEls.reloadBtn.addEventListener('click', () => {
        this.load(true, deskEls.reloadBtn);
        DesktopWindow.focusWindow();
      });
    }
    bumpVisitCounter();
    updateClock();
    setInterval(updateClock, 30000);
    DesktopWindow.init();
    DesktopSettings.init();
    document.addEventListener('keydown', onDesktopKeydown, true);
    document.addEventListener('focusin', scheduleActiveSync);
    document.addEventListener('focusout', scheduleActiveSync);
    window.addEventListener('blur', scheduleActiveSync);
    window.addEventListener('focus', scheduleActiveSync);
    document.getElementById('view-desktop').addEventListener('pointerdown', scheduleActiveSync);
  },
  onShow() {
    revealWindowWhenReady();
    DesktopSettings.boot();
    if (cache.sites) this.render();
    else this.load();
    requestAnimationFrame(() => DesktopWindow.reflow());
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
      if (pageEls.list) pageEls.list.replaceChildren();
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
    if (pageEls.list) {
      pageEls.list.replaceChildren(...sites.map(buildLinkRow));
    }
    renderMarquee(sites);
    renderCounter(sites);
    if (deskEls.statusText) deskEls.statusText.textContent = DESK_TEXT.doneStatus;
  },
};
