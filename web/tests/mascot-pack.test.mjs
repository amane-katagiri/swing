import './setup.mjs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { normalizeManifest } from '../desktop-mascot-pack.js';

const MASCOTS = new URL('../mascots/', import.meta.url);
const BUNDLED = ['mochi', 'neko', 'yureko'];
const MAX_SHEET_SIDE = 4096;
const MAX_SHEET_PIXELS = 2048 * 2048;
const MAX_SPRITE_BYTES = 1024 * 1024;
const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

function pngSize(bytes) {
  assert.ok(bytes.subarray(0, 8).equals(PNG_SIGNATURE), 'PNG signature');
  assert.equal(bytes.toString('latin1', 12, 16), 'IHDR');
  return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) };
}

function usedIndices(pack) {
  const all = [...Object.values(pack.animations), ...Object.values(pack.overlays)];
  return all.flatMap((a) => a.frames.map((f) => f.index));
}

function minimal(extra = {}) {
  return { format: 1, sprite: 'sprite.png', frame: { width: 48, height: 48 }, animations: { idle: { frames: [0] } }, ...extra };
}

test('the bundled packs are the ones the server embeds', () => {
  const dirs = readdirSync(MASCOTS, { withFileTypes: true }).filter((d) => d.isDirectory()).map((d) => d.name).sort();
  assert.deepEqual(dirs, BUNDLED);
  const server = readFileSync(new URL('../../src/dashboard/mascots/mod.rs', import.meta.url), 'utf8');
  for (const id of BUNDLED) assert.ok(server.includes(`web/mascots/${id}/manifest.json`), id);
});

for (const id of BUNDLED) {
  test(`bundled pack ${id} passes validation and fits its sprite sheet`, () => {
    const manifest = JSON.parse(readFileSync(new URL(`${id}/manifest.json`, MASCOTS), 'utf8'));
    const pack = normalizeManifest(manifest);
    assert.notEqual(pack.name, '');
    assert.match(pack.sprite, /^[^/\\.][^/\\]*\.png$/i);
    const bytes = readFileSync(new URL(`${id}/${pack.sprite}`, MASCOTS));
    assert.ok(bytes.length <= MAX_SPRITE_BYTES, 'sprite size');
    const { width, height } = pngSize(bytes);
    assert.ok(width <= MAX_SHEET_SIDE && height <= MAX_SHEET_SIDE && width * height <= MAX_SHEET_PIXELS, `${width}x${height}`);
    const cols = Math.floor(width / pack.frame.width);
    const rows = Math.floor(height / pack.frame.height);
    for (const index of usedIndices(pack)) assert.ok(index < cols * rows, `frame ${index} outside the ${cols}x${rows} sheet`);
  });

  test(`bundled pack ${id} uses only the placeholders each line kind fills in`, () => {
    const manifest = JSON.parse(readFileSync(new URL(`${id}/manifest.json`, MASCOTS), 'utf8'));
    const single = ['title', 'd', 'count'];
    const allowed = {
      'site-stored': single,
      'site-published': single,
      'replica-added': single,
      'sites-stored-many': ['count'],
      'sites-published-many': ['count'],
      'replicas-added-many': ['count'],
    };
    for (const [kind, lines] of Object.entries(normalizeManifest(manifest).lines)) {
      for (const line of lines) {
        for (const m of line.matchAll(/\{(\w+)\}/g)) assert.ok((allowed[kind] || []).includes(m[1]), `${kind}: ${line}`);
      }
    }
  });
}

test('normalizeManifest fills in the documented defaults', () => {
  const pack = normalizeManifest(minimal({ frame: { width: 33, height: 20 } }));
  assert.equal(pack.name, '');
  assert.equal(pack.scale, 1);
  assert.deepEqual(pack.anchor, { x: 16, y: 20 });
  assert.deepEqual(pack.balloon, { x: 16.5, y: 0 });
  assert.equal(pack.facing, 'right');
  assert.equal(pack.speed, 24);
  assert.equal(pack.walkMoves, false);
  assert.deepEqual(pack.animations.idle, { frames: [{ index: 0, ms: 250, dx: 0, dy: 0 }], loop: true, flip: true });
  for (const name of ['walk', 'talk', 'sleep', 'surprise', 'drag', 'fall']) assert.equal(pack.animations[name], pack.animations.idle, name);
  assert.deepEqual(pack.overlays, {});
  assert.deepEqual(pack.lines, {});
});

test('normalizeManifest reads frames, overlays and lines', () => {
  const pack = normalizeManifest(
    minimal({
      name: `  ${'名'.repeat(45)}\u202e`,
      animations: {
        idle: { frames: [0, 1], fps: 2 },
        walk: { frames: [{ index: 2, ms: 120, dx: -2, dy: 1 }], loop: false },
        sleep: { frames: [3], flip: false },
      },
      overlays: { blink: { frames: [4] }, mouth: { frames: [5], on: ['talk', 'idle'] }, wink: 'ignored' },
      lines: { greet: 'hi', idle: ['a', 3, '', 'x'.repeat(250)], unknown: ['dropped'] },
    }),
  );
  assert.equal(pack.name, `${'名'.repeat(40)}…`);
  assert.deepEqual(pack.animations.idle.frames.map((f) => f.ms), [500, 500]);
  assert.deepEqual(pack.animations.walk, { frames: [{ index: 2, ms: 120, dx: -2, dy: 1 }], loop: false, flip: true });
  assert.equal(pack.animations.sleep.flip, false);
  assert.equal(pack.walkMoves, true);
  assert.deepEqual([...pack.overlays.blink.on], ['idle', 'walk', 'talk']);
  assert.deepEqual(pack.overlays.blink.interval, [2500, 6000]);
  assert.deepEqual([...pack.overlays.mouth.on], ['talk', 'idle']);
  assert.equal(pack.overlays.mouth.interval, undefined);
  assert.deepEqual(Object.keys(pack.overlays).sort(), ['blink', 'mouth']);
  assert.deepEqual(pack.lines, { greet: ['hi'], idle: ['a', `${'x'.repeat(200)}…`] });
});

test('normalizeManifest rejects values outside the pack format', () => {
  const idle = (spec) => ({ animations: { idle: { frames: [0], ...spec } } });
  const cases = [
    ['not an object', null],
    ['array', []],
    ['format 2', minimal({ format: 2 })],
    ['format as a string', minimal({ format: '1' })],
    ['empty sprite', minimal({ sprite: '' })],
    ['no frame', minimal({ frame: undefined })],
    ['frame too small', minimal({ frame: { width: 7, height: 48 } })],
    ['frame too large', minimal({ frame: { width: 48, height: 129 } })],
    ['fractional frame', minimal({ frame: { width: 47.5, height: 48 } })],
    ['scale 0', minimal({ scale: 0 })],
    ['scale 5', minimal({ scale: 5 })],
    ['fractional scale', minimal({ scale: 1.5 })],
    ['anchor outside', minimal({ anchor: { x: 49, y: 0 } })],
    ['negative anchor', minimal({ anchor: { x: 0, y: -1 } })],
    ['fractional anchor', minimal({ anchor: { x: 1.5, y: 0 } })],
    ['balloon outside', minimal({ balloon: { x: 0, y: 49 } })],
    ['balloon not an object', minimal({ balloon: 3 })],
    ['facing up', minimal({ facing: 'up' })],
    ['speed 0', minimal({ speed: 0 })],
    ['speed above 1000', minimal({ speed: 1001 })],
    ['speed as a string', minimal({ speed: '24' })],
    ['no animations', minimal({ animations: undefined })],
    ['no idle', minimal({ animations: { walk: { frames: [0] } } })],
    ['empty frames', minimal(idle({ frames: [] }))],
    ['too many frames', minimal(idle({ frames: new Array(257).fill(0) }))],
    ['negative index', minimal(idle({ frames: [-1] }))],
    ['fractional index', minimal(idle({ frames: [1.5] }))],
    ['fps 0', minimal(idle({ fps: 0 }))],
    ['fps above 60', minimal(idle({ fps: 61 }))],
    ['frame shorter than 16 ms', minimal(idle({ frames: [{ index: 0, ms: 15 }] }))],
    ['frame longer than 60 s', minimal(idle({ frames: [{ index: 0, ms: 60001 }] }))],
    ['low fps gives frames longer than 60 s', minimal(idle({ fps: 0.01 }))],
    ['dx wider than the frame', minimal(idle({ frames: [{ index: 0, dx: 49 }] }))],
    ['dy taller than the frame', minimal(idle({ frames: [{ index: 0, dy: -49 }] }))],
    ['loop as a string', minimal(idle({ loop: 'yes' }))],
    ['flip as a number', minimal(idle({ flip: 0 }))],
    ['bad walk', minimal({ animations: { idle: { frames: [0] }, walk: { frames: 'x' } } })],
    ['overlays not an object', minimal({ overlays: [] })],
    ['blink on not an array', minimal({ overlays: { blink: { frames: [0], on: 'idle' } } })],
    ['blink interval reversed', minimal({ overlays: { blink: { frames: [0], interval: [3000, 2000] } } })],
    ['blink interval too short', minimal({ overlays: { blink: { frames: [0], interval: [99, 2000] } } })],
    ['blink interval with one value', minimal({ overlays: { blink: { frames: [0], interval: [2000] } } })],
    ['mouth loop not a boolean', minimal({ overlays: { mouth: { frames: [0], loop: 1 } } })],
    ['lines not an object', minimal({ lines: ['hi'] })],
  ];
  for (const [name, manifest] of cases) assert.throws(() => normalizeManifest(manifest), (err) => err.constructor.name === 'PackError', name);
});

test('normalizeManifest accepts the edges of each range', () => {
  const cases = [
    ['smallest frame', minimal({ frame: { width: 8, height: 8 }, anchor: { x: 8, y: 8 } })],
    ['largest frame and scale', minimal({ frame: { width: 128, height: 128 }, scale: 4 })],
    ['anchor on the corner', minimal({ anchor: { x: 0, y: 0 } })],
    ['speed 1000', minimal({ speed: 1000 })],
    ['facing left', minimal({ facing: 'left' })],
    ['fps 60', minimal({ animations: { idle: { frames: [0], fps: 60 } } })],
    ['256 frames', minimal({ animations: { idle: { frames: new Array(256).fill(0) } } })],
    ['dx equal to the frame', minimal({ animations: { idle: { frames: [{ index: 0, dx: 48, dy: -48 }] } } })],
    ['equal blink interval', minimal({ overlays: { blink: { frames: [0], interval: [100, 100] } } })],
    ['unknown keys', minimal({ extra: true, animations: { idle: { frames: [0] }, dance: 'ignored' } })],
  ];
  for (const [name, manifest] of cases) assert.doesNotThrow(() => normalizeManifest(manifest), name);
});
