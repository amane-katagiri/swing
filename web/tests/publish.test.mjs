import './setup.mjs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { computeRelativePath, findNewFiles, groupByFolder } from '../publish.js';

const file = (name, webkitRelativePath = '') => ({ name, webkitRelativePath });

test('computeRelativePath drops the chosen folder from folder uploads', () => {
  const cases = [
    ['single file', file('index.html'), 'index.html'],
    ['file at the top of the folder', file('index.html', 'site/index.html'), 'index.html'],
    ['nested file', file('style.css', 'site/assets/css/style.css'), 'assets/css/style.css'],
    ['folder named like a file', file('a.html', 'site.v2/a.html'), 'a.html'],
    ['spaces and Japanese', file('ページ 1.html', 'マイ サイト/日記/ページ 1.html'), '日記/ページ 1.html'],
    ['relative path without a slash', file('x', 'loose.txt'), 'loose.txt'],
    ['dotfile', file('.nojekyll', 'site/.nojekyll'), '.nojekyll'],
  ];
  for (const [name, f, want] of cases) assert.equal(computeRelativePath(f), want, name);
});

test('findNewFiles lists files not in the previous version', () => {
  const previous = { files: ['index.html', 'assets/a.css'] };
  const files = [file('index.html', 'site/index.html'), file('a.css', 'site/assets/a.css'), file('b.css', 'site/assets/b.css'), file('new.html', 'site/new.html')];
  assert.deepEqual(findNewFiles(previous, files), ['assets/b.css', 'new.html']);
  assert.deepEqual(findNewFiles({ files: [] }, [file('one.html')]), ['one.html']);
});

test('groupByFolder groups paths by folder in code unit order', () => {
  assert.deepEqual(groupByFolder([]), []);
  assert.deepEqual(groupByFolder(['b/z.css', 'index.html', 'a/b/c.js', 'b/Y.css', 'B/x', 'about.html', 'a/b.js']), [
    ['', ['about.html', 'index.html']],
    ['B', ['x']],
    ['a', ['b.js']],
    ['a/b', ['c.js']],
    ['b', ['Y.css', 'z.css']],
  ]);
});
