import './setup.mjs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { MESSAGES } from '../i18n.js';

const WEB = new URL('../', import.meta.url);
const sources = readdirSync(WEB)
  .filter((name) => name.endsWith('.js'))
  .map((name) => ({ name, text: readFileSync(new URL(name, WEB), 'utf8') }));

const KEY_MAPS = [
  ['publish.js', 'NIP05_ERROR_KEYS', ['']],
  ['publish.js', 'LINK_KIND_KEYS', ['']],
  ['publish.js', 'NIP05_BLOCKED_KEYS', ['']],
  ['settings-notify.js', 'REASON_KEYS', ['']],
  ['setup.js', 'KEY_CHOICE_HINTS', ['']],
  ['ui.js', 'STORED_LABELS', ['']],
  ['desktop-notify.js', 'TEXT_KEYS', ['']],
  ['sites.js', 'NIP05_KEYS', ['nip05Label', 'nip05Desc']],
];

function firstArgument(text, start) {
  let depth = 0;
  let quote = null;
  for (let i = start; i < text.length; i += 1) {
    const c = text[i];
    if (quote) {
      if (c === '\\') i += 1;
      else if (c === quote) quote = null;
    } else if (c === "'" || c === '"' || c === '`') {
      quote = c;
    } else if (c === '(' || c === '[' || c === '{') {
      depth += 1;
    } else if (c === ')' || c === ']' || c === '}') {
      if (depth === 0) return text.slice(start, i);
      depth -= 1;
    } else if (c === ',' && depth === 0) {
      return text.slice(start, i);
    }
  }
  throw new Error(`unterminated call at ${start}`);
}

function literalKeys(arg) {
  const keys = [];
  for (const m of arg.matchAll(/(===|!==|==|!=)?\s*(['"])([^'"\\]*)\2/g)) {
    if (!m[1]) keys.push(m[3]);
  }
  return keys;
}

function usedKeys() {
  const used = [];
  for (const { name, text } of sources) {
    for (const m of text.matchAll(/(?<![\w.$])t\(/g)) {
      for (const key of literalKeys(firstArgument(text, m.index + 2))) used.push([name, key]);
    }
  }
  return used;
}

function mapKeys() {
  const used = [];
  for (const [file, constName, prefixes] of KEY_MAPS) {
    const text = sources.find((s) => s.name === file).text;
    const start = text.indexOf(`const ${constName} = {`);
    assert.notEqual(start, -1, `${file}: ${constName}`);
    const body = text.slice(start, text.indexOf('}', start));
    const values = [...body.matchAll(/'([^']*)'/g)].map((m) => m[1]);
    assert.ok(values.length > 0, `${file}: ${constName}`);
    for (const v of values) for (const p of prefixes) used.push([`${file} ${constName}`, `${p}${v}`]);
  }
  return used;
}

function placeholders(str) {
  return [...new Set([...str.matchAll(/\{(\w+)\}/g)].map((m) => m[1]))].sort();
}

test('en and ja have the same keys', () => {
  const en = Object.keys(MESSAGES.en).sort();
  const ja = Object.keys(MESSAGES.ja).sort();
  assert.deepEqual(en.filter((k) => !Object.hasOwn(MESSAGES.ja, k)), [], 'missing in ja');
  assert.deepEqual(ja.filter((k) => !Object.hasOwn(MESSAGES.en, k)), [], 'missing in en');
  assert.deepEqual(Object.keys(MESSAGES).sort(), ['en', 'ja']);
});

test('every message is a non-empty string', () => {
  for (const [lang, dict] of Object.entries(MESSAGES)) {
    for (const [key, value] of Object.entries(dict)) {
      assert.equal(typeof value, 'string', `${lang}.${key}`);
      assert.notEqual(value.trim(), '', `${lang}.${key}`);
    }
  }
});

test('placeholders match between languages', () => {
  for (const key of Object.keys(MESSAGES.en)) {
    assert.deepEqual(placeholders(MESSAGES.ja[key] ?? ''), placeholders(MESSAGES.en[key]), key);
  }
});

test('every literal key passed to t() exists', () => {
  const used = usedKeys();
  assert.ok(used.some(([, key]) => key === 'copied'));
  assert.ok(used.some(([, key]) => key === 'sizeOver'));
  assert.ok(!used.some(([, key]) => key === 'over'));
  for (const [file, key] of used) assert.ok(Object.hasOwn(MESSAGES.en, key), `${file}: ${key}`);
});

test('every key in the lookup tables exists', () => {
  for (const [where, key] of mapKeys()) assert.ok(Object.hasOwn(MESSAGES.en, key), `${where}: ${key}`);
});
