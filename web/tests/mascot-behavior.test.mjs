import './setup.mjs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { normalizeManifest } from '../desktop-mascot-pack.js';
import { between, createBehavior } from '../desktop-mascot-behavior.js';

const WIDTH = 400;
const GROUND = 300;
const STEP = { dx: 0 };

function pack(extra = {}) {
  return normalizeManifest({
    format: 1,
    sprite: 'sprite.png',
    frame: { width: 48, height: 48 },
    anchor: { x: 24, y: 48 },
    speed: 20,
    animations: { idle: { frames: [0] }, sleep: { frames: [1], flip: false } },
    ...extra,
  });
}

function placed(r, p = pack()) {
  const b = createBehavior(p, { random: () => r, now: 0 });
  b.setBounds(WIDTH, GROUND, 0);
  return b;
}

function moveTo(b, x, t) {
  b.grab(t);
  b.dragTo(x, GROUND);
  b.release(t);
}

function run(b, from, to, stepMs = 50, step = STEP) {
  for (let t = from; t <= to; t += stepMs) b.update(t, stepMs, step);
}

test('between maps the random number onto the range', () => {
  assert.equal(between([10, 20], () => 0), 10);
  assert.equal(between([10, 20], () => 0.5), 15);
});

test('setBounds places the mascot on the ground within reach of both edges', () => {
  const left = placed(0);
  assert.deepEqual([left.x, left.y, left.facing, left.mode], [24, GROUND, 'left', 'idle']);
  const right = placed(0.99);
  assert.ok(Math.abs(right.x - (24 + 0.99 * (WIDTH - 48))) < 1e-9);
  assert.equal(right.facing, 'right');
  const narrow = createBehavior(pack({ scale: 2 }), { random: () => 0 });
  narrow.setBounds(60, GROUND, 0);
  assert.equal(narrow.x, 30);
});

test('setBounds keeps the mascot inside when the desktop shrinks', () => {
  const b = placed(0.99);
  b.setBounds(200, 150, 10);
  assert.deepEqual([b.x, b.y], [176, 150]);
});

test('an idle mascot walks toward a far target at its speed and stops there', () => {
  const b = placed(0.3);
  moveTo(b, 300, 0);
  assert.equal(b.mode, 'idle');
  const target = 24 + 0.3 * (WIDTH - 48);
  const decideAt = 1500 + 0.3 * 4500;
  b.update(decideAt - 1, 16, STEP);
  assert.equal(b.mode, 'idle');
  b.update(decideAt, 16, STEP);
  assert.deepEqual([b.mode, b.facing, b.animation], ['walk', 'left', 'walk']);
  b.update(decideAt + 1000, 1000, STEP);
  assert.equal(b.x, 300 - 2, 'a long frame moves at most 100 ms worth');
  b.update(decideAt + 1050, 50, STEP);
  assert.equal(b.x, 300 - 3);
  run(b, decideAt + 1100, decideAt + 20000);
  assert.equal(b.x, target);
  assert.equal(b.mode, 'idle');
});

test('a pack whose walk frames move uses dx instead of speed', () => {
  const moving = pack({ scale: 2, animations: { idle: { frames: [0] }, walk: { frames: [{ index: 1, dx: 3 }] } } });
  const b = placed(0.3, moving);
  moveTo(b, 60, 0);
  b.update(3000, 16, STEP);
  assert.deepEqual([b.mode, b.facing], ['walk', 'right']);
  b.update(3016, 16, { dx: -3 });
  assert.equal(b.x, 66);
  b.update(3032, 16, { dx: 0 });
  assert.equal(b.x, 66);
});

test('a near target is not worth a walk', () => {
  const b = placed(0.3);
  run(b, 0, 50000);
  assert.equal(b.mode, 'idle');
});

test('the mascot does not walk when walking is off or motion is reduced', () => {
  for (const [name, setup] of [
    ['walk off', (b) => b.setWalk(false, 0)],
    ['reduced motion', (b) => b.setReduced(true)],
  ]) {
    const b = placed(0.3);
    moveTo(b, 300, 0);
    setup(b);
    run(b, 0, 50000);
    assert.deepEqual([b.mode, b.x], ['idle', 300], name);
  }
});

test('turning walking off stops a walk', () => {
  const b = placed(0.3);
  moveTo(b, 300, 0);
  b.update(2850, 16, STEP);
  assert.equal(b.mode, 'walk');
  b.setWalk(false, 2900);
  assert.equal(b.mode, 'idle');
});

test('an idle mascot falls asleep at its next decision and wakes up after a while', () => {
  const b = placed(0.9);
  const sleepAt = 60000 + 0.9 * 60000;
  const idleWait = 1500 + 0.9 * 4500;
  run(b, 0, sleepAt - 50);
  assert.equal(b.mode, 'idle');
  let t = sleepAt - 50;
  while (b.mode === 'idle' && t < sleepAt + idleWait + 100) {
    t += 50;
    b.update(t, 50, STEP);
  }
  assert.equal(b.mode, 'sleep');
  assert.ok(t >= sleepAt && t <= sleepAt + idleWait, `slept at ${t}`);
  const wakeAt = t + 30000 + 0.9 * 60000;
  run(b, t + 50, wakeAt - 50);
  assert.equal(b.mode, 'sleep');
  b.update(wakeAt, 50, STEP);
  assert.equal(b.mode, 'idle');
});

test('interaction postpones sleep and wakes a sleeping mascot', () => {
  const b = placed(0.9);
  b.wake(100000);
  run(b, 0, 150000);
  assert.equal(b.mode, 'idle');
  b.update(100000 + 114000, 50, STEP);
  assert.equal(b.mode, 'sleep');
  b.wake(215000);
  assert.equal(b.mode, 'idle');
});

test('a sleep animation with flip off faces the way the sheet is drawn', () => {
  const b = placed(0);
  assert.equal(b.facing, 'left');
  b.update(60000, 16, STEP);
  assert.equal(b.mode, 'sleep');
  b.update(60016, 16, STEP);
  assert.equal(b.facing, 'right');
});

test('talking shows the talk animation, stops a walk and wakes the mascot', () => {
  const b = placed(0.3);
  moveTo(b, 300, 0);
  b.update(2850, 16, STEP);
  assert.equal(b.mode, 'walk');
  b.setTalking(true, 2900);
  assert.deepEqual([b.mode, b.animation], ['idle', 'talk']);
  run(b, 2900, 30000);
  assert.deepEqual([b.mode, b.x], ['idle', 300], 'no walk while talking');
  b.setTalking(false, 30000);
  assert.equal(b.animation, 'idle');

  const sleeping = placed(0);
  sleeping.update(60000, 16, STEP);
  assert.equal(sleeping.mode, 'sleep');
  sleeping.setTalking(true, 60100);
  assert.equal(sleeping.mode, 'idle');
});

test('surprise lasts 900 ms and does not interrupt a drag', () => {
  const b = placed(0.9);
  b.surprise(1000);
  assert.deepEqual([b.mode, b.animation], ['surprise', 'surprise']);
  b.setTalking(true, 1000);
  assert.equal(b.animation, 'surprise');
  b.update(1899, 16, STEP);
  assert.equal(b.mode, 'surprise');
  b.update(1900, 16, STEP);
  assert.equal(b.mode, 'idle');

  b.grab(2000);
  b.surprise(2000);
  assert.deepEqual([b.mode, b.held], ['drag', true]);
});

test('a mascot dropped in the air falls to the ground', () => {
  const b = placed(0.9);
  b.grab(0);
  b.dragTo(-100, 0);
  assert.deepEqual([b.x, b.y], [24, 48], 'kept inside and below the top');
  b.release(0);
  assert.equal(b.animation, 'fall');
  let t = 0;
  while (b.mode === 'fall' && t < 5000) {
    t += 16;
    b.update(t, 16, STEP);
  }
  assert.deepEqual([b.mode, b.y], ['idle', GROUND]);
  assert.ok(t > 400 && t < 700, `landed after ${t} ms`);
});

test('with reduced motion a dropped mascot lands at once', () => {
  const b = placed(0.9);
  b.setReduced(true);
  b.grab(0);
  b.dragTo(200, 100);
  b.release(0);
  assert.deepEqual([b.mode, b.y], ['idle', GROUND]);
});

test('release does nothing unless the mascot is held', () => {
  const b = placed(0.9);
  b.release(0);
  assert.equal(b.mode, 'idle');
});
