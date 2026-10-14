import './setup.mjs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { snapScale } from '../desktop-scale.js';

test('snapScale rounds the device pixel ratio down to a whole number of device pixels', () => {
  const cases = [
    ['100%', 1, 1, 1],
    ['125%', 1.25, 1, 0.8],
    ['150%', 1.5, 1, 1 / 1.5],
    ['200%', 2, 2, 1],
    ['200% reported just below', 1.9999999, 2, 2 / 1.9999999],
    ['250%', 2.5, 2, 0.8],
    ['300%', 3, 3, 1],
    ['zoomed out', 0.5, 1, 2],
    ['67%', 0.67, 1, 1 / 0.67],
  ];
  for (const [name, ratio, snapped, scale] of cases) {
    const got = snapScale(ratio);
    assert.equal(got.snapped, snapped, name);
    assert.ok(Math.abs(got.scale - scale) < 1e-12, `${name}: ${got.scale}`);
    assert.ok(Math.abs(got.scale * ratio - got.snapped) < 1e-9, `${name}: one CSS px is a whole number of device pixels`);
  }
});
