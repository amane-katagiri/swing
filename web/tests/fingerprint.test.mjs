import './setup.mjs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { emojiChar, fingerprintIds } from '../ui.js';

test('fingerprintIds keeps only emoji ids and cuts to the count', () => {
  const cases = [
    ['not an array', ['1f600'], []],
    ['valid ids', [['1f600', '2764', '1f3f3']], ['1f600', '2764', '1f3f3']],
    ['invalid ids are dropped', [['1F600', '27', '1f6000', '../x', 3, null, '1f380']], ['1f380']],
    ['count', [['1f600', '2764', '1f3f3'], 2], ['1f600', '2764']],
  ];
  for (const [name, args, want] of cases) assert.deepEqual(fingerprintIds(...args), want, name);
});

test('emojiChar adds the emoji presentation selector', () => {
  assert.equal(emojiChar('1f380'), '\u{1f380}\ufe0f');
  assert.equal(emojiChar('2764'), '\u2764\ufe0f');
});
