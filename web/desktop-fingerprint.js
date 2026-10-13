import { EMOJI_SPRITE, fingerprintIds, emojiChar } from './ui.js';

const SIZE = 16;
const SVG_NS = 'http://www.w3.org/2000/svg';
const ALPHA_CUT = 128;

let spritePromise = null;
const pixelCache = new Map();

function loadSprite() {
  if (!spritePromise) {
    spritePromise = fetch(EMOJI_SPRITE)
      .then((res) => (res.ok ? res.text() : Promise.reject(new Error(`HTTP ${res.status}`))))
      .then((text) => new DOMParser().parseFromString(text, 'image/svg+xml'));
    spritePromise.catch(() => { spritePromise = null; });
  }
  return spritePromise;
}

function loadImage(src) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = reject;
    img.src = src;
  });
}

async function rasterize(id) {
  const symbol = (await loadSprite()).getElementById(`e-${id}`);
  if (!symbol) throw new Error(`missing emoji ${id}`);
  const body = [...symbol.childNodes].map((n) => new XMLSerializer().serializeToString(n)).join('');
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 36 36" width="36" height="36">${body}</svg>`;
  const img = await loadImage(`data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`);
  const canvas = document.createElement('canvas');
  canvas.width = SIZE;
  canvas.height = SIZE;
  const ctx = canvas.getContext('2d');
  ctx.imageSmoothingQuality = 'high';
  ctx.drawImage(img, 0, 0, SIZE, SIZE);
  const data = ctx.getImageData(0, 0, SIZE, SIZE);
  for (let i = 3; i < data.data.length; i += 4) data.data[i] = data.data[i] < ALPHA_CUT ? 0 : 255;
  ctx.putImageData(data, 0, 0);
  return canvas.toDataURL('image/png');
}

function pixelEmoji(id) {
  if (!pixelCache.has(id)) {
    const p = rasterize(id);
    p.catch(() => pixelCache.delete(id));
    pixelCache.set(id, p);
  }
  return pixelCache.get(id);
}

function vectorEmoji(id) {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('aria-hidden', 'true');
  svg.setAttribute('width', SIZE);
  svg.setAttribute('height', SIZE);
  const use = document.createElementNS(SVG_NS, 'use');
  use.setAttribute('href', `${EMOJI_SPRITE}#e-${id}`);
  svg.append(use);
  return svg;
}

export function deskFingerprint(ids, title) {
  const shown = fingerprintIds(ids);
  if (shown.length === 0) return null;
  const chars = shown.map(emojiChar).join(' ');
  const wrap = document.createElement('span');
  wrap.className = 'desk-fingerprint';
  wrap.setAttribute('role', 'img');
  wrap.setAttribute('aria-label', chars);
  if (title) wrap.title = title(chars);
  for (const id of shown) {
    const img = document.createElement('img');
    img.alt = '';
    img.width = SIZE;
    img.height = SIZE;
    img.decoding = 'async';
    pixelEmoji(id).then((src) => { img.src = src; }, () => img.replaceWith(vectorEmoji(id)));
    wrap.append(img);
  }
  return wrap;
}
