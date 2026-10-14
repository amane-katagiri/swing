import './setup.mjs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { collectNotices, diffReporters, isNoticeEvent } from '../desktop-updates.js';

test('isNoticeEvent accepts only notice events', () => {
  const cases = [
    ['sites-stored', true],
    ['published', true],
    ['replicas-added', true],
    ['recovered', false],
    ['fetch-error', false],
    ['stored', false],
    [undefined, false],
  ];
  for (const [kind, want] of cases) assert.equal(isNoticeEvent({ kind }), want, String(kind));
});

function site(d, storedAt, extra = {}) {
  return { d, stored: true, stored_at: storedAt, url: `https://${d}/`, ...extra };
}

test('collectNotices lists stored sites newer than the cursor, oldest first', () => {
  const data = {
    accounts: [
      {
        pubkey: 'pk-a',
        npub: 'npub-a',
        sites: [
          site('new.example', 30),
          site('old.example', 10),
          site('cursor.example', 20),
          site('pending.example', 40, { stored: false }),
          site('unknown-time.example', null),
          site('gateway.example', 25, { gateway_url: 'http://127.0.0.1:8080/ipfs/x/', url: 'https://gateway.example/' }),
          site('no-link.example', 26, { url: null }),
          site('unsafe-link.example', 27, { url: 'javascript:alert(1)' }),
        ],
      },
      { pubkey: 'pk-b', npub: 'npub-b' },
    ],
    unfollowed: { accounts: [{ pubkey: 'pk-c', npub: 'npub-c', sites: [site('unfollowed.example', 22)] }] },
  };
  const notices = collectNotices(data, 20);
  assert.deepEqual(
    notices.map((n) => [n.site.d, n.at, n.href, n.pubkey, n.npub]),
    [
      ['unfollowed.example', 22, 'https://unfollowed.example/', 'pk-c', 'npub-c'],
      ['gateway.example', 25, 'http://127.0.0.1:8080/ipfs/x/', 'pk-a', 'npub-a'],
      ['new.example', 30, 'https://new.example/', 'pk-a', 'npub-a'],
    ],
  );
  assert.equal(notices[2].kind, 'stored');
  assert.equal(notices[2].storedAt, 30);
  assert.equal(notices[2].key, 'stored\u0000pk-a\u0000new.example\u000030');
});

test('collectNotices tolerates missing groups', () => {
  assert.deepEqual(collectNotices({}, 0), []);
  assert.deepEqual(collectNotices({ accounts: [], unfollowed: {} }, 0), []);
});

const AUTHOR = 'pk-author';

function reporter(pubkey, tier = 'chosen', latest = true) {
  return { pubkey, npub: `npub-${pubkey}`, tier, latest };
}

function replicas(sites) {
  return { authors: [{ pubkey: AUTHOR, sites }] };
}

test('diffReporters returns nothing without an author', () => {
  assert.deepEqual(diffReporters({}, null, 0, 5), { next: {}, added: [] });
  assert.deepEqual(diffReporters({ authors: [] }, {}, 0, 5), { next: {}, added: [] });
});

test('diffReporters counts only trusted, current reporters other than the author', () => {
  const data = replicas([
    {
      d: 'site.example',
      replicas: 3,
      unverified: 1,
      reporters: [reporter('pk-chosen'), reporter('pk-own', 'author'), reporter('pk-other', 'other'), reporter('pk-old', 'chosen', false), reporter(AUTHOR, 'author')],
    },
    { d: 'lonely.example', reporters: [reporter('pk-other', 'other')] },
    { d: 'bare.example' },
  ]);
  const { next, added, author } = diffReporters(data, null, 100, 100);
  assert.equal(author.pubkey, AUTHOR);
  assert.deepEqual(next, { 'site.example': { 'pk-chosen': 100, 'pk-own': 100 } });
  assert.deepEqual(added, [
    {
      d: 'site.example',
      reporters: [
        { pubkey: 'pk-chosen', npub: 'npub-pk-chosen', tier: 'chosen' },
        { pubkey: 'pk-own', npub: 'npub-pk-own', tier: 'author' },
      ],
      replicas: 3,
      unverified: 1,
      at: 100,
    },
  ]);
});

test('diffReporters keeps the cursor at which each reporter first appeared', () => {
  const data = replicas([{ d: 'site.example', reporters: [reporter('pk-known'), reporter('pk-unacked'), reporter('pk-new')] }]);
  const book = { 'site.example': { 'pk-known': 10, 'pk-unacked': 15, 'pk-gone': 12 }, 'removed.example': { 'pk-x': 1 } };
  const { next, added } = diffReporters(data, book, 12, 20);
  assert.deepEqual(next, { 'site.example': { 'pk-known': 10, 'pk-unacked': 15, 'pk-new': 20 } });
  assert.equal(added.length, 1);
  assert.deepEqual(added[0].reporters.map((r) => r.pubkey), ['pk-unacked', 'pk-new']);
  assert.equal(added[0].at, 20);
});

test('diffReporters reports nothing when every reporter is already acknowledged', () => {
  const data = replicas([{ d: 'site.example', reporters: [reporter('pk-a')] }]);
  const { next, added } = diffReporters(data, { 'site.example': { 'pk-a': 5 } }, 5, 9);
  assert.deepEqual(next, { 'site.example': { 'pk-a': 5 } });
  assert.deepEqual(added, []);
});

test('diffReporters treats a reporter that left and came back as new', () => {
  const data = replicas([{ d: 'site.example', reporters: [reporter('pk-a')] }]);
  const left = diffReporters(replicas([{ d: 'site.example', reporters: [] }]), { 'site.example': { 'pk-a': 5 } }, 5, 7);
  assert.deepEqual(left.next, {});
  const back = diffReporters(data, left.next, 7, 9);
  assert.deepEqual(back.next, { 'site.example': { 'pk-a': 9 } });
  assert.equal(back.added[0].at, 9);
});

test('diffReporters ignores cursors in the book that are not numbers', () => {
  const data = replicas([{ d: 'site.example', reporters: [reporter('pk-a'), reporter('pk-b')] }]);
  const { next, added } = diffReporters(data, { 'site.example': { 'pk-a': '3', 'pk-b': null } }, 5, 9);
  assert.deepEqual(next, { 'site.example': { 'pk-a': 9, 'pk-b': 9 } });
  assert.deepEqual(added[0].reporters.map((r) => r.pubkey), ['pk-a', 'pk-b']);
});
