import { sanitizeDisplayText, siteTitle } from './util.js';
import { trackPointer } from './desktop-drag.js';
import { toDeskPx, frameToViewport } from './desktop-scale.js';
import { isModalOpen } from './desktop-focus.js';
import { loadPacks } from './desktop-mascot-pack.js';
import { createSprite } from './desktop-mascot-sprite.js';
import { between, createBehavior } from './desktop-mascot-behavior.js';
import { createBalloon } from './desktop-mascot-balloon.js';
import { isNoticeEvent } from './desktop-updates.js';
import { readNotifySettings } from './notify-settings.js';

const DEFAULT_LINES = {
  greet: ['こんにちは！'],
  'site-stored': ['{title} が更新されました'],
  'sites-stored-many': ['新しい更新が {count} 件あります'],
  'site-published': ['{title} を公開しました'],
  'sites-published-many': ['{count} 件のサイトを公開しました'],
  'replica-added': ['{title} をミラーしてくれる人が増えました'],
  'replicas-added-many': ['{count} 件のサイトをミラーしてくれる人が増えました'],
  'fetch-error': ['サーバにつながらないようです…'],
  recovered: ['サーバにつながりました'],
  idle: ['ひまですね…'],
  click: ['なにかご用ですか？'],
};

const NOTICE_LINES = {
  stored: ['site-stored', 'sites-stored-many'],
  published: ['site-published', 'sites-published-many'],
  replica: ['replica-added', 'replicas-added-many'],
};
const MAX_LINKS = 5;
const TITLE_MAX = 60;
const LINE_CLOSE_MS = 4000;
const DRAG_THRESHOLD = 3;
const IDLE_LINE_EVERY = [180000, 420000];
const CROWD_RATIO = 0.25;
const REDUCED = '(prefers-reduced-motion: reduce)';
const CLICK_SWALLOW_MS = 500;

const els = {
  container: document.getElementById('desk-mascots'),
  live: document.getElementById('desk-mascot-live'),
  fallbackFocus: document.getElementById('desk-icon-explorer'),
  frame: document.getElementById('desk-page-frame'),
};

let allPacks = [];
const instances = [];
const byNode = new Map();
let active = [];
let settings = { packs: ['yureko'], walk: true, chatter: true };
let resolveLoaded;
const loadedPromise = new Promise((resolve) => {
  resolveLoaded = resolve;
});
let updates = null;
let queued = [];
let deferred = [];
let loaded = false;
let greeted = false;
let roundRobin = 0;
let raf = null;
let last = 0;
let nextIdleLine = 0;
const bounds = { width: 0, ground: 0 };
let pointer = null;
let hot = null;
let dragging = null;
let blockTouch = false;
const reducedQuery = window.matchMedia(REDUCED);

function pickLine(pack, kind) {
  const list = pack.lines[kind] || DEFAULT_LINES[kind];
  return list[Math.floor(Math.random() * list.length)];
}

function fill(template, vars) {
  return template.replace(/\{(title|d|count)\}/g, (m, key) => (vars[key] == null ? m : String(vars[key])));
}

function isViewActive() {
  return document.body.dataset.view === 'desktop' && !document.hidden;
}

function combine(prev, next) {
  if (!prev || next.kind !== 'replica') return next;
  const reporters = new Map([...prev.added, ...next.added].map((r) => [r.pubkey, r]));
  return { ...(next.at >= prev.at ? next : prev), at: Math.max(prev.at, next.at), added: Array.from(reporters.values()) };
}

function mergeNotices(...lists) {
  const byKey = new Map();
  for (const list of lists) for (const n of list) byKey.set(n.key, combine(byKey.get(n.key), n));
  return Array.from(byKey.values()).sort((a, b) => a.at - b.at);
}

function unacknowledged(notices) {
  return notices.filter((n) => !updates.isAcknowledged(n));
}

function forMascot(notices) {
  const { mascot } = readNotifySettings();
  return notices.filter((n) => mascot[n.kind]);
}

function showingNotices(inst) {
  return inst.balloon.open && Array.isArray(inst.balloon.content.notices);
}

function announce(text) {
  if (!els.live) return;
  els.live.textContent = '';
  requestAnimationFrame(() => {
    els.live.textContent = text;
  });
}

function say(inst, kind, now = performance.now()) {
  if (!inst || showingNotices(inst) || inst.behavior.held) return;
  inst.balloon.show({ text: pickLine(inst.pack, kind), closable: false, autoCloseMs: LINE_CLOSE_MS }, now, { instant: reducedQuery.matches });
  inst.behavior.setTalking(true, now);
}

function noticeLine(pack, kind, group) {
  const [one, many] = NOTICE_LINES[kind];
  if (group.length > 1) return fill(pickLine(pack, many), { count: group.length });
  const n = group[0];
  return fill(pickLine(pack, one), { title: siteTitle(n.site, TITLE_MAX), d: sanitizeDisplayText(n.site.d, TITLE_MAX), count: 1 });
}

function showNotices(inst, notices, now) {
  const acknowledge = () => updates.acknowledge(notices);
  const toLink = (n) => ({ label: siteTitle(n.site, TITLE_MAX), href: n.href, onOpen: acknowledge });
  const text = Object.keys(NOTICE_LINES)
    .map((kind) => notices.filter((n) => n.kind === kind))
    .filter((group) => group.length > 0)
    .map((group) => noticeLine(inst.pack, group[0].kind, group))
    .join('\n');
  const newestFirst = [...notices].reverse();
  const content = { text, links: newestFirst.slice(0, MAX_LINKS).map(toLink), more: Math.max(0, notices.length - MAX_LINKS) };
  inst.balloon.show({ ...content, closable: true, autoCloseMs: null, notices }, now, { instant: reducedQuery.matches });
  inst.behavior.setTalking(true, now);
  announce([content.text, ...content.links.map((l) => l.label)].join('、'));
}

function holdsNotices(inst) {
  return showingNotices(inst) || inst.pending.length > 0;
}

function noticeTarget(candidates) {
  return candidates.find(holdsNotices) || candidates[0];
}

function deliver(inst, notices, now) {
  if (inst.behavior.held) {
    inst.pending = mergeNotices(inst.pending, notices);
    return;
  }
  const merged = mergeNotices(showingNotices(inst) ? inst.balloon.content.notices : [], inst.pending, notices);
  inst.pending = [];
  if (merged.length === 0) return;
  inst.behavior.surprise(now);
  showNotices(inst, merged, now);
}

function distribute(notices) {
  const fresh = unacknowledged(notices);
  if (fresh.length === 0 || active.length === 0) return;
  const now = performance.now();
  let target = active.find(holdsNotices);
  if (!target) {
    target = active[roundRobin % active.length];
    roundRobin += 1;
  }
  for (const inst of active) if (inst !== target) inst.behavior.surprise(now);
  deliver(target, fresh, now);
}

function flushDeferred() {
  if (!loaded || !isViewActive() || deferred.length === 0) return;
  const notices = forMascot(deferred);
  deferred = [];
  distribute(notices);
}

function onUpdate(event) {
  if (!loaded) {
    queued.push(event);
    return;
  }
  if (isNoticeEvent(event)) {
    const notices = forMascot(event.notices);
    if (isViewActive()) distribute(notices);
    else deferred = mergeNotices(deferred, notices);
  } else if ((event.kind === 'fetch-error' || event.kind === 'recovered') && isViewActive() && active.length > 0) {
    say(active[roundRobin % active.length], event.kind, performance.now());
  }
}

function onBalloonClose(inst, content, reason, hadFocus) {
  inst.behavior.setTalking(false, performance.now());
  if (reason === 'button' && Array.isArray(content.notices)) updates.acknowledge(content.notices);
  if (hadFocus && els.fallbackFocus) els.fallbackFocus.focus();
}

function toLocal(clientX, clientY) {
  const rect = els.container.getBoundingClientRect();
  return { x: toDeskPx(clientX - rect.left), y: toDeskPx(clientY - rect.top) };
}

function inside(node, x, y) {
  const r = node.getBoundingClientRect();
  return x >= r.left && x < r.right && y >= r.top && y < r.bottom;
}

function mascotAt(clientX, clientY) {
  if (!isViewActive() || els.container.hidden) return null;
  const p = toLocal(clientX, clientY);
  const nodes = els.container.children;
  for (let i = nodes.length - 1; i >= 0; i -= 1) {
    const node = nodes[i];
    if (node.hidden) continue;
    const inst = byNode.get(node);
    if (inst) {
      if (inst.sprite.hits(p.x, p.y)) return inst;
    } else if (node.classList.contains('desk-balloon') && inside(node, clientX, clientY)) {
      return null;
    }
  }
  return null;
}

function setHot(inst) {
  if (hot === inst) return;
  if (hot) hot.sprite.el.classList.remove('is-hot');
  hot = inst;
  if (hot) hot.sprite.el.classList.add('is-hot');
}

function updateHover() {
  if (dragging) return;
  setHot(pointer ? mascotAt(pointer.x, pointer.y) : null);
}

function onPointerMove(ev, point) {
  if (ev.pointerType === 'touch') return;
  pointer = point;
  updateHover();
}

function swallowNextClick(doc) {
  const swallow = (e) => {
    e.preventDefault();
    e.stopPropagation();
  };
  doc.addEventListener('click', swallow, { capture: true, once: true });
  return () => setTimeout(() => doc.removeEventListener('click', swallow, { capture: true }), CLICK_SWALLOW_MS);
}

function trackInFrame(doc, downEv, onMove, onEnd) {
  document.body.classList.add('desk-no-select');
  const mine = (e) => e.pointerId === downEv.pointerId;
  const move = (e) => {
    if (mine(e)) onMove(e);
  };
  const end = (e) => {
    if (!mine(e)) return;
    doc.removeEventListener('pointermove', move, true);
    doc.removeEventListener('pointerup', end, true);
    doc.removeEventListener('pointercancel', end, true);
    document.body.classList.remove('desk-no-select');
    onEnd(e);
  };
  doc.addEventListener('pointermove', move, true);
  doc.addEventListener('pointerup', end, true);
  doc.addEventListener('pointercancel', end, true);
}

function startInteraction(inst, ev, source) {
  const node = inst.sprite.el;
  const toPoint = (e) => {
    const c = source.toClient(e);
    return toLocal(c.x, c.y);
  };
  const start = toPoint(ev);
  const offset = { x: start.x - inst.behavior.x, y: start.y - inst.behavior.y };
  let moved = false;
  setHot(inst);
  dragging = inst;
  const onMove = (mv) => {
    const p = toPoint(mv);
    if (!moved) {
      if (Math.hypot(p.x - start.x, p.y - start.y) < DRAG_THRESHOLD) return;
      moved = true;
      inst.behavior.grab(performance.now());
    }
    inst.behavior.dragTo(p.x - offset.x, p.y - offset.y);
  };
  const onEnd = (e) => {
    dragging = null;
    if (source.done) source.done();
    if (e.pointerType !== 'touch') pointer = source.toClient(e);
    updateHover();
    const now = performance.now();
    if (moved) {
      inst.behavior.release(now);
      if (inst.pending.length > 0) deliver(inst, [], now);
      return;
    }
    onMascotClick(inst, now);
  };
  if (source.doc === document) trackPointer(node, ev, onMove, onEnd);
  else trackInFrame(source.doc, ev, onMove, onEnd);
}

const topSource = { doc: document, toClient: (e) => ({ x: e.clientX, y: e.clientY }) };

function frameSource(doc) {
  return { doc, toClient: (e) => frameToViewport(els.frame, e.clientX, e.clientY) };
}

function wirePointer(inst) {
  const node = inst.sprite.el;
  node.addEventListener('pointerdown', (ev) => {
    if (ev.button !== 0 || dragging) return;
    ev.preventDefault();
    startInteraction(inst, ev, topSource);
  });
  node.addEventListener('click', (ev) => ev.stopPropagation());
}

/* Touch has no hover to switch the mascot on beforehand, so the press that lands on what is underneath is taken over here. */
function takeOverPress(ev, source) {
  if (ev.button !== 0 || dragging || isModalOpen()) return;
  if (source.doc === document && (!(ev.target instanceof Element) || !ev.target.closest('#desk-screen') || ev.target.closest('#desk-mascots'))) return;
  const c = source.toClient(ev);
  const inst = mascotAt(c.x, c.y);
  if (!inst) return;
  ev.preventDefault();
  ev.stopPropagation();
  if (ev.pointerType === 'touch') blockTouch = true;
  const release = swallowNextClick(source.doc);
  startInteraction(inst, ev, { ...source, done: release });
}

function onTouchStart(ev) {
  if (!blockTouch) return;
  blockTouch = false;
  ev.preventDefault();
}

function watchDocument(doc, source) {
  doc.addEventListener('pointermove', (ev) => onPointerMove(ev, source.toClient(ev)), { capture: true, passive: true });
  doc.addEventListener('pointerdown', (ev) => takeOverPress(ev, source), true);
  doc.addEventListener('touchstart', onTouchStart, { capture: true, passive: false });
}

function onMascotClick(inst, now) {
  inst.behavior.wake(now);
  if (inst.pending.length > 0) {
    deliver(inst, [], now);
    return;
  }
  if (showingNotices(inst)) return;
  say(inst, 'click', now);
}

function applyWalk() {
  const now = performance.now();
  for (const inst of instances) inst.behavior.setWalk(settings.walk, now);
}

function removeInstance(inst) {
  const carried = mergeNotices(showingNotices(inst) ? inst.balloon.content.notices : [], inst.pending);
  inst.pending = [];
  inst.balloon.close('hidden');
  if (hot === inst) setHot(null);
  byNode.delete(inst.sprite.el);
  inst.sprite.el.remove();
  inst.balloon.el.remove();
  return carried;
}

function selectedPacks() {
  if (settings.packs == null) return allPacks;
  return allPacks.filter((pack) => settings.packs.includes(pack.id));
}

function reconcile() {
  const want = selectedPacks();
  const now = performance.now();
  let carried = [];
  for (const inst of instances) {
    if (!want.includes(inst.pack)) carried = mergeNotices(carried, removeInstance(inst));
  }
  const next = want.map((pack) => instances.find((inst) => inst.pack === pack) || createInstance(pack, now));
  instances.splice(0, instances.length, ...next);
  els.container.hidden = instances.length === 0;
  applyReduced();
  applyWalk();
  measure();
  updateActive();
  if (carried.length > 0 && active.length > 0) deliver(noticeTarget(active), carried, now);
  startLoop();
}

function createInstance(pack, now) {
  const inst = { pack, pending: [] };
  inst.sprite = createSprite(pack);
  inst.behavior = createBehavior(pack, { now });
  inst.balloon = createBalloon(els.container, {
    onClose: (content, reason, hadFocus) => onBalloonClose(inst, content, reason, hadFocus),
  });
  els.container.append(inst.sprite.el);
  byNode.set(inst.sprite.el, inst);
  wirePointer(inst);
  return inst;
}

function applyReduced() {
  for (const inst of instances) {
    inst.sprite.setStill(reducedQuery.matches);
    inst.behavior.setReduced(reducedQuery.matches);
  }
}

function fitting() {
  if (bounds.width === 0) return instances.slice();
  const budget = bounds.width * CROWD_RATIO;
  const out = [];
  let used = 0;
  for (const inst of instances) {
    used += inst.pack.frame.width * inst.pack.scale;
    if (out.length > 0 && used > budget) break;
    out.push(inst);
  }
  return out;
}

function updateActive() {
  const now = performance.now();
  const next = fitting();
  for (const inst of instances) {
    const on = next.includes(inst);
    inst.sprite.el.hidden = !on;
    if (on || !inst.balloon.open) continue;
    const carried = showingNotices(inst) ? inst.balloon.content.notices : [];
    const pending = mergeNotices(carried, inst.pending);
    inst.pending = [];
    inst.balloon.close('hidden');
    if (pending.length > 0 && next.length > 0) {
      const heir = noticeTarget(next);
      heir.pending = mergeNotices(heir.pending, pending);
    }
  }
  active = next;
  for (const inst of active) if (inst.pending.length > 0) deliver(inst, [], now);
}

function measure() {
  const width = els.container.clientWidth;
  const ground = els.container.clientHeight;
  if (width === 0 || ground === 0) return;
  const widthChanged = width !== bounds.width;
  bounds.width = width;
  bounds.ground = ground;
  const now = performance.now();
  for (const inst of instances) {
    inst.behavior.setBounds(width, ground, now);
    inst.balloon.remeasure();
  }
  if (widthChanged) updateActive();
}

function maybeIdleLine(now) {
  if (now < nextIdleLine) return;
  nextIdleLine = now + between(IDLE_LINE_EVERY);
  if (!settings.chatter) return;
  const idle = active.filter((inst) => !inst.balloon.open && inst.behavior.mode === 'idle');
  if (idle.length > 0) say(idle[Math.floor(Math.random() * idle.length)], 'idle', now);
}

function step(inst, now, dt) {
  inst.sprite.play(inst.behavior.animation, now);
  const moved = inst.sprite.advance(now);
  inst.behavior.update(now, dt, moved);
  inst.sprite.setSpeaking(inst.balloon.typing(now), now);
  const { x, y, facing } = inst.behavior;
  inst.sprite.place(x, y, facing);
  inst.balloon.tick(now, inst.sprite.box(x, y, facing), bounds);
}

function frame(now) {
  raf = null;
  if (!isViewActive() || instances.length === 0) return;
  if (bounds.width === 0) measure();
  if (bounds.width > 0) {
    const dt = now - last;
    for (const inst of active) step(inst, now, dt);
    updateHover();
    maybeIdleLine(now);
  }
  last = now;
  raf = requestAnimationFrame(frame);
}

function startLoop() {
  if (raf != null || !isViewActive() || instances.length === 0) return;
  last = performance.now();
  for (const inst of instances) inst.sprite.resume(last);
  raf = requestAnimationFrame(frame);
}

function greetOnce() {
  if (greeted || !loaded || !isViewActive() || active.length === 0) return;
  greeted = true;
  if (settings.chatter) say(active[0], 'greet');
}

async function load() {
  allPacks = await loadPacks();
  const now = performance.now();
  for (const pack of selectedPacks()) instances.push(createInstance(pack, now));
  els.container.hidden = instances.length === 0;
  applyReduced();
  applyWalk();
  measure();
  updateActive();
  nextIdleLine = now + between(IDLE_LINE_EVERY);
  loaded = true;
  resolveLoaded();
  const pendingEvents = queued;
  queued = [];
  for (const event of pendingEvents) onUpdate(event);
  greetOnce();
  flushDeferred();
  startLoop();
}

function onVisibility() {
  flushDeferred();
  startLoop();
}

export const DesktopMascots = {
  init({ updates: watcher }) {
    if (!els.container) return;
    updates = watcher;
    updates.subscribe(onUpdate);
    new ResizeObserver(measure).observe(els.container);
    reducedQuery.addEventListener('change', applyReduced);
    document.addEventListener('visibilitychange', onVisibility);
    watchDocument(document, topSource);
    load();
  },
  watchFrame(doc) {
    if (!els.container || !els.frame) return;
    watchDocument(doc, frameSource(doc));
  },
  onShow() {
    if (!els.container) return;
    greetOnce();
    flushDeferred();
    startLoop();
  },
  showing() {
    if (!els.container || !isViewActive()) return false;
    if (!loaded) return settings.packs == null || settings.packs.length > 0;
    return active.length > 0;
  },
  packs() {
    return allPacks;
  },
  whenLoaded() {
    return loadedPromise;
  },
  applySettings(next) {
    settings = { packs: next.packs == null ? null : [...next.packs], walk: next.walk, chatter: next.chatter };
    if (loaded) reconcile();
  },
};
