import './setup.mjs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { sanitizeDisplayText } from '../util.js';

const vectors = JSON.parse(readFileSync(new URL('./unsafe-unicode.json', import.meta.url), 'utf8'));

function classify(c, out) {
  if (out === `a${c}b`) return 'kept';
  if (out === 'ab') return 'removed';
  if (out === 'a b') return 'space';
  return `unexpected ${JSON.stringify(out)}`;
}

test('sanitizeDisplayText classifies each code point as the shared vectors say', () => {
  for (const entry of vectors.chars) {
    const c = String.fromCodePoint(Number.parseInt(entry.cp, 16));
    assert.equal(classify(c, sanitizeDisplayText(`a${c}b`)), entry.want, `U+${entry.cp}`);
  }
});

test('sanitizeDisplayText cleans and truncates texts as the shared vectors say', () => {
  for (const entry of vectors.texts) {
    assert.equal(sanitizeDisplayText(entry.input, entry.max), entry.want, entry.name);
  }
});
