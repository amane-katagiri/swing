import { el } from './util.js';
import { opaqueAt } from './desktop-mascot-pack.js';

export function createPlayer(frames, loop) {
  const cycleMs = frames.reduce((sum, f) => sum + f.ms, 0);
  const cycleDx = frames.reduce((sum, f) => sum + f.dx, 0);
  let i = 0;
  let start = 0;
  let done = false;
  return {
    get frame() {
      return frames[i];
    },
    get done() {
      return done;
    },
    restart(now) {
      i = 0;
      start = now;
      done = false;
    },
    resume(now) {
      start = now;
    },
    advance(now) {
      const step = { dx: 0 };
      if (loop && now - start >= cycleMs) {
        const cycles = Math.floor((now - start) / cycleMs);
        start += cycles * cycleMs;
        step.dx = cycles * cycleDx;
      }
      for (let n = 0; n <= frames.length && !done && now - start >= frames[i].ms; n += 1) {
        start += frames[i].ms;
        if (i + 1 < frames.length) i += 1;
        else if (loop) i = 0;
        else {
          done = true;
          break;
        }
        step.dx += frames[i].dx;
      }
      return step;
    },
  };
}

function randomBetween(random, [lo, hi]) {
  return lo + random() * (hi - lo);
}

export function createSprite(pack, { random = Math.random } = {}) {
  const { width: fw, height: fh } = pack.frame;
  const s = pack.scale;
  const layer = (kind) => el('div', { class: `desk-mascot-layer desk-mascot-layer-${kind}` });
  const base = layer('base');
  const blink = pack.overlays.blink ? layer('blink') : null;
  const mouth = pack.overlays.mouth ? layer('mouth') : null;
  const flip = el('div', { class: 'desk-mascot-flip' }, [base, blink, mouth]);
  const root = el('div', { class: 'desk-mascot', 'data-pack': pack.id, 'aria-hidden': 'true' }, flip);
  root.style.width = `${fw * s}px`;
  root.style.height = `${fh * s}px`;
  for (const node of [base, blink, mouth]) {
    if (!node) continue;
    node.style.backgroundImage = `url("${pack.spriteUrl}")`;
    node.style.backgroundSize = `${pack.sheet.width * s}px ${pack.sheet.height * s}px`;
  }

  let animName = null;
  let player = null;
  let still = false;
  let speaking = false;
  let baseDy = 0;
  let blinkPlayer = null;
  let nextBlink = 0;
  const mouthPlayer = pack.overlays.mouth ? createPlayer(pack.overlays.mouth.frames, true) : null;
  const shown = new Map();
  const visible = new Map();
  const pos = { left: 0, top: 0, flipped: false };

  function show(node, frame) {
    visible.set(node, frame);
    if (!frame) {
      if (shown.get(node) !== null) {
        node.style.visibility = 'hidden';
        shown.set(node, null);
      }
      return;
    }
    const key = `${frame.index}:${frame.dy}`;
    if (shown.get(node) === key) return;
    shown.set(node, key);
    const col = frame.index % pack.sheet.cols;
    const row = Math.floor(frame.index / pack.sheet.cols);
    node.style.visibility = '';
    node.style.backgroundPosition = `${-col * fw * s}px ${-row * fh * s}px`;
    node.style.transform = frame.dy ? `translateY(${frame.dy * s}px)` : '';
  }

  function layerHits(frame, lx, ly) {
    if (!frame) return false;
    const col = pos.flipped ? fw - 1 - Math.floor(lx / s) : Math.floor(lx / s);
    const row = Math.floor((ly - frame.dy * s) / s);
    if (row < 0 || row >= fh) return false;
    const sx = (frame.index % pack.sheet.cols) * fw + col;
    const sy = Math.floor(frame.index / pack.sheet.cols) * fh + row;
    return opaqueAt(pack, sx, sy);
  }

  function scheduleBlink(now) {
    nextBlink = now + randomBetween(random, pack.overlays.blink.interval);
  }

  return {
    el: root,
    get animation() {
      return animName;
    },
    get done() {
      return player ? player.done : true;
    },
    play(name, now) {
      if (name === animName) return;
      animName = name;
      const anim = pack.animations[name] || pack.animations.idle;
      player = createPlayer(anim.frames, anim.loop);
      player.restart(now);
    },
    setStill(value) {
      still = value;
    },
    setSpeaking(value, now) {
      if (value && !speaking && mouthPlayer) mouthPlayer.restart(now);
      speaking = value;
    },
    resume(now) {
      if (player) player.resume(now);
      if (blinkPlayer) blinkPlayer.resume(now);
      if (mouthPlayer) mouthPlayer.resume(now);
      if (pack.overlays.blink) scheduleBlink(now);
    },
    advance(now) {
      if (!player) return { dx: 0 };
      if (still) {
        show(base, pack.animations.idle.frames[0]);
        baseDy = pack.animations.idle.frames[0].dy;
        if (blink) show(blink, null);
        if (mouth) show(mouth, null);
        return { dx: 0 };
      }
      const step = player.advance(now);
      show(base, player.frame);
      const dy = player.frame.dy;
      baseDy = dy;
      if (blink) {
        const on = pack.overlays.blink.on.has(animName);
        if (!blinkPlayer && on && now >= nextBlink) {
          blinkPlayer = createPlayer(pack.overlays.blink.frames, false);
          blinkPlayer.restart(now);
        }
        if (blinkPlayer) blinkPlayer.advance(now);
        if (blinkPlayer && (blinkPlayer.done || !on)) {
          blinkPlayer = null;
          scheduleBlink(now);
        }
        show(blink, blinkPlayer ? { ...blinkPlayer.frame, dy } : null);
      }
      if (mouth) {
        const on = speaking && pack.overlays.mouth.on.has(animName);
        if (on) mouthPlayer.advance(now);
        show(mouth, on ? { ...mouthPlayer.frame, dy } : null);
      }
      return step;
    },
    place(x, y, facing) {
      const flipped = facing !== pack.facing;
      const ax = flipped ? fw - pack.anchor.x : pack.anchor.x;
      pos.left = Math.round(x - ax * s);
      pos.top = Math.round(y - pack.anchor.y * s);
      pos.flipped = flipped;
      root.style.left = `${pos.left}px`;
      root.style.top = `${pos.top}px`;
      flip.classList.toggle('is-flipped', flipped);
    },
    hits(x, y) {
      const lx = x - pos.left;
      const ly = y - pos.top;
      if (root.hidden || lx < 0 || ly < 0 || lx >= fw * s || ly >= fh * s) return false;
      for (const frame of visible.values()) if (layerHits(frame, lx, ly)) return true;
      return false;
    },
    box(x, y, facing) {
      const flipped = facing !== pack.facing;
      const ax = flipped ? fw - pack.anchor.x : pack.anchor.x;
      const left = Math.round(x - ax * s);
      const top = Math.round(y - pack.anchor.y * s);
      const bx = flipped ? fw - pack.balloon.x : pack.balloon.x;
      return { left, top, right: left + fw * s, bottom: top + fh * s, pointX: Math.round(left + bx * s), pointY: Math.round(top + (pack.balloon.y + baseDy) * s) };
    },
  };
}
