import { sanitizeDisplayText } from './util.js';

export const INDEX_URL = '/mascots/index.json';
export const ANIMATION_NAMES = ['idle', 'walk', 'talk', 'sleep', 'surprise', 'drag', 'fall'];
export const LINE_KINDS = ['greet', 'site-stored', 'sites-stored-many', 'fetch-error', 'recovered', 'idle', 'click'];

const OVERLAY_DEFAULTS = {
  blink: { on: ['idle', 'walk', 'talk'], interval: [2500, 6000] },
  mouth: { on: ['talk'] },
};
const DEFAULT_FPS = 4;
const DEFAULT_SPEED = 24;
const MIN_FRAME = 8;
const MAX_FRAME = 128;
const MAX_SCALE = 4;
const MAX_LINE = 200;
const MAX_FPS = 60;
const MIN_FRAME_MS = 16;
const MAX_FRAME_MS = 60000;
const MAX_FRAMES = 256;
const MAX_SPEED = 1000;
const MIN_BLINK_MS = 100;
const MAX_BLINK_MS = 600000;
export const MAX_SHEET_SIDE = 4096;
export const MAX_SHEET_PIXELS = 2048 * 2048;
const OPAQUE_ALPHA = 128;

class PackError extends Error {}

function fail(message) {
  throw new PackError(message);
}

function isObject(v) {
  return v != null && typeof v === 'object' && !Array.isArray(v);
}

function isInt(v, lo, hi) {
  return Number.isInteger(v) && v >= lo && v <= hi;
}

function isNum(v) {
  return typeof v === 'number' && Number.isFinite(v);
}

function isNumIn(v, lo, hi) {
  return isNum(v) && v >= lo && v <= hi;
}

function normalizeFrames(spec, label, frame) {
  if (!isObject(spec) || !Array.isArray(spec.frames) || spec.frames.length === 0) fail(`${label}: frames must be a non-empty array`);
  if (spec.frames.length > MAX_FRAMES) fail(`${label}: at most ${MAX_FRAMES} frames`);
  const fps = spec.fps == null ? DEFAULT_FPS : spec.fps;
  if (!isNum(fps) || fps <= 0 || fps > MAX_FPS) fail(`${label}: fps must be a number above 0 and at most ${MAX_FPS}`);
  const frames = spec.frames.map((f, i) => {
    const entry = Number.isInteger(f) ? { index: f } : f;
    if (!isObject(entry) || !Number.isInteger(entry.index) || entry.index < 0) fail(`${label}: frame ${i} has no valid index`);
    const ms = entry.ms == null ? 1000 / fps : entry.ms;
    if (!isNumIn(ms, MIN_FRAME_MS, MAX_FRAME_MS)) fail(`${label}: frame ${i} lasts ${ms} ms, outside ${MIN_FRAME_MS}-${MAX_FRAME_MS} ms`);
    const dx = entry.dx == null ? 0 : entry.dx;
    const dy = entry.dy == null ? 0 : entry.dy;
    if (!isNumIn(dx, -frame.width, frame.width) || !isNumIn(dy, -frame.height, frame.height)) {
      fail(`${label}: frame ${i} dx/dy must be numbers within the frame size`);
    }
    return { index: entry.index, ms, dx, dy };
  });
  const loop = spec.loop == null ? true : spec.loop;
  if (typeof loop !== 'boolean') fail(`${label}: loop must be a boolean`);
  return { frames, loop };
}

function normalizeOverlay(name, spec, frame) {
  const base = normalizeFrames(spec, `overlays.${name}`, frame);
  const defaults = OVERLAY_DEFAULTS[name];
  const on = spec.on == null ? defaults.on : spec.on;
  if (!Array.isArray(on) || !on.every((n) => typeof n === 'string')) fail(`overlays.${name}: on must be an array of animation names`);
  const overlay = { frames: base.frames, on: new Set(on) };
  if (name === 'blink') {
    const interval = spec.interval == null ? defaults.interval : spec.interval;
    if (!Array.isArray(interval) || interval.length !== 2 || !interval.every((v) => isNumIn(v, MIN_BLINK_MS, MAX_BLINK_MS)) || interval[0] > interval[1]) {
      fail(`overlays.blink: interval must be [min, max] in ms, each ${MIN_BLINK_MS}-${MAX_BLINK_MS}`);
    }
    overlay.interval = interval;
  }
  return overlay;
}

function normalizeLines(spec) {
  const lines = {};
  if (spec == null) return lines;
  if (!isObject(spec)) fail('lines must be an object');
  for (const kind of LINE_KINDS) {
    const v = spec[kind];
    if (v == null) continue;
    const list = (Array.isArray(v) ? v : [v]).filter((s) => typeof s === 'string').map((s) => sanitizeDisplayText(s, MAX_LINE)).filter(Boolean);
    if (list.length > 0) lines[kind] = list;
  }
  return lines;
}

export function normalizeManifest(m) {
  if (!isObject(m)) fail('manifest must be an object');
  if (m.format !== 1) fail(`unsupported format ${JSON.stringify(m.format)}`);
  if (typeof m.sprite !== 'string' || m.sprite === '') fail('sprite must be a file name');
  if (!isObject(m.frame) || !isInt(m.frame.width, MIN_FRAME, MAX_FRAME) || !isInt(m.frame.height, MIN_FRAME, MAX_FRAME)) {
    fail(`frame width/height must be integers ${MIN_FRAME}-${MAX_FRAME}`);
  }
  const width = m.frame.width;
  const height = m.frame.height;
  const scale = m.scale == null ? 1 : m.scale;
  if (!isInt(scale, 1, MAX_SCALE)) fail(`scale must be an integer 1-${MAX_SCALE}`);
  const anchor = m.anchor == null ? { x: Math.floor(width / 2), y: height } : m.anchor;
  if (!isObject(anchor) || !isInt(anchor.x, 0, width) || !isInt(anchor.y, 0, height)) fail('anchor must be integer x/y inside the frame');
  const balloon = m.balloon == null ? { x: width / 2, y: 0 } : m.balloon;
  if (m.balloon != null && (!isObject(balloon) || !isInt(balloon.x, 0, width) || !isInt(balloon.y, 0, height))) fail('balloon must be integer x/y inside the frame');
  const facing = m.facing == null ? 'right' : m.facing;
  if (facing !== 'right' && facing !== 'left') fail('facing must be "right" or "left"');
  const speed = m.speed == null ? DEFAULT_SPEED : m.speed;
  if (!isNum(speed) || speed <= 0 || speed > MAX_SPEED) fail(`speed must be a number above 0 and at most ${MAX_SPEED}`);

  if (!isObject(m.animations) || m.animations.idle == null) fail('animations.idle is required');
  const animations = {};
  for (const name of ANIMATION_NAMES) {
    const spec = m.animations[name];
    if (spec == null) continue;
    const anim = normalizeFrames(spec, `animations.${name}`, { width, height });
    const flip = spec.flip == null ? true : spec.flip;
    if (typeof flip !== 'boolean') fail(`animations.${name}: flip must be a boolean`);
    animations[name] = { ...anim, flip };
  }
  for (const name of ANIMATION_NAMES) {
    if (!animations[name]) animations[name] = animations.idle;
  }

  const overlays = {};
  if (m.overlays != null) {
    if (!isObject(m.overlays)) fail('overlays must be an object');
    for (const name of Object.keys(OVERLAY_DEFAULTS)) {
      if (m.overlays[name] != null) overlays[name] = normalizeOverlay(name, m.overlays[name], { width, height });
    }
  }

  const name = typeof m.name === 'string' ? sanitizeDisplayText(m.name, 40) : '';
  return {
    name,
    sprite: m.sprite,
    frame: { width, height },
    scale,
    anchor: { x: anchor.x, y: anchor.y },
    balloon: { x: balloon.x, y: balloon.y },
    facing,
    speed,
    animations,
    walkMoves: animations.walk.frames.some((f) => f.dx !== 0),
    overlays,
    lines: normalizeLines(m.lines),
  };
}

function maxFrameIndex(pack) {
  let max = 0;
  for (const anim of Object.values(pack.animations)) for (const f of anim.frames) max = Math.max(max, f.index);
  for (const ov of Object.values(pack.overlays)) for (const f of ov.frames) max = Math.max(max, f.index);
  return max;
}

function resolveInside(file, baseUrl) {
  const url = new URL(file, baseUrl);
  if (url.origin !== baseUrl.origin || !url.pathname.startsWith(baseUrl.pathname)) fail(`${file} is outside the pack`);
  return url;
}

function loadImage(url) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.decoding = 'async';
    img.addEventListener('load', () => resolve(img), { once: true });
    img.addEventListener('error', () => reject(new PackError(`sprite ${url.pathname} could not be loaded`)), { once: true });
    img.src = url.href;
  });
}

function readMask(img) {
  const width = img.naturalWidth;
  const height = img.naturalHeight;
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  if (!ctx) fail('sprite pixels could not be read');
  ctx.drawImage(img, 0, 0);
  let data;
  try {
    data = ctx.getImageData(0, 0, width, height).data;
  } catch {
    fail('sprite pixels could not be read');
  }
  const mask = new Uint8Array(Math.ceil((width * height) / 8));
  for (let i = 0; i < width * height; i += 1) {
    if (data[i * 4 + 3] >= OPAQUE_ALPHA) mask[i >> 3] |= 1 << (i & 7);
  }
  return mask;
}

export function opaqueAt(pack, x, y) {
  const i = y * pack.sheet.width + x;
  return (pack.mask[i >> 3] & (1 << (i & 7))) !== 0;
}

async function fetchJson(url) {
  const resp = await fetch(url, { credentials: 'same-origin' });
  if (!resp.ok) throw new PackError(`${url} returned ${resp.status}`);
  return resp.json();
}

async function loadPack(entry) {
  if (!isObject(entry) || typeof entry.id !== 'string' || typeof entry.base !== 'string') fail('index entry needs id and base');
  const baseUrl = new URL(entry.base.endsWith('/') ? entry.base : `${entry.base}/`, window.location.href);
  if (baseUrl.origin !== window.location.origin) fail(`${entry.base} is not same-origin`);
  const pack = normalizeManifest(await fetchJson(new URL('manifest.json', baseUrl)));
  const spriteUrl = resolveInside(pack.sprite, baseUrl);
  const img = await loadImage(spriteUrl);
  if (img.naturalWidth > MAX_SHEET_SIDE || img.naturalHeight > MAX_SHEET_SIDE || img.naturalWidth * img.naturalHeight > MAX_SHEET_PIXELS) {
    fail(`sprite is ${img.naturalWidth}x${img.naturalHeight}, over ${MAX_SHEET_SIDE} px a side or ${MAX_SHEET_PIXELS} pixels`);
  }
  const cols = Math.floor(img.naturalWidth / pack.frame.width);
  const rows = Math.floor(img.naturalHeight / pack.frame.height);
  if (maxFrameIndex(pack) >= cols * rows) fail(`frame index ${maxFrameIndex(pack)} is outside the ${cols}x${rows} sprite sheet`);
  return {
    ...pack,
    id: entry.id,
    spriteUrl: spriteUrl.href,
    sheet: { width: img.naturalWidth, height: img.naturalHeight, cols },
    mask: readMask(img),
  };
}

export async function loadPacks() {
  let index;
  try {
    index = await fetchJson(INDEX_URL);
  } catch (err) {
    console.warn('mascot index could not be loaded:', err);
    return [];
  }
  const entries = isObject(index) && Array.isArray(index.packs) ? index.packs : [];
  const results = await Promise.all(
    entries.map((entry) =>
      loadPack(entry).catch((err) => {
        console.warn(`mascot pack ${isObject(entry) ? entry.id : '?'} skipped:`, err instanceof Error ? err.message : err);
        return null;
      }),
    ),
  );
  return results.filter(Boolean);
}
