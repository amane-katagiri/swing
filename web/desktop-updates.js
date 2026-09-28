import { storage } from './storage.js';
import { apiFetch, cache, isHttpUrl } from './util.js';
import { kindWanted, readNotifySettings } from './notify-settings.js';

export const SEEN_KEY = 'swing:desktop:seen';
export const SEEN_PUBLISHED_KEY = 'swing:desktop:seen-published';
export const SEEN_REPLICAS_KEY = 'swing:desktop:seen-replicas';
export const REPLICA_REPORTERS_KEY = 'swing:desktop:replica-reporters';

const EVENT_KINDS = { stored: 'sites-stored', published: 'published', replica: 'replicas-added' };
const SEEN_KEYS = { stored: SEEN_KEY, published: SEEN_PUBLISHED_KEY, replica: SEEN_REPLICAS_KEY };

export function isNoticeEvent(event) {
  return Object.values(EVENT_KINDS).includes(event.kind);
}

function readNumber(key) {
  const n = Number.parseInt(storage.get(key, ''), 10);
  return Number.isFinite(n) ? n : null;
}

function readSeen(kind) {
  return readNumber(SEEN_KEYS[kind]);
}

function writeSeen(kind, value) {
  storage.set(SEEN_KEYS[kind], String(value));
}

function* storedSites(data) {
  const groups = [data.accounts || []];
  if (data.unfollowed && Array.isArray(data.unfollowed.accounts)) groups.push(data.unfollowed.accounts);
  for (const accounts of groups) {
    for (const acct of accounts) {
      for (const site of acct.sites || []) {
        if (site.stored) yield { acct, site };
      }
    }
  }
}

function newestStoredAt(data) {
  let newest = null;
  for (const { site } of storedSites(data)) {
    if (site.stored_at != null && (newest == null || site.stored_at > newest)) newest = site.stored_at;
  }
  return newest;
}

function siteHref(site) {
  return site.gateway_url || site.url || null;
}

export function collectNotices(data, since) {
  const notices = [];
  for (const { acct, site } of storedSites(data)) {
    const href = siteHref(site);
    if (!isHttpUrl(href) || site.stored_at == null || site.stored_at <= since) continue;
    notices.push({ kind: 'stored', key: `stored\u0000${acct.pubkey}\u0000${site.d}\u0000${site.stored_at}`, at: site.stored_at, pubkey: acct.pubkey, npub: acct.npub, site, href, storedAt: site.stored_at });
  }
  notices.sort((a, b) => a.at - b.at);
  return notices;
}

function publishedNotice(site) {
  return { kind: 'published', key: `published\u0000${site.d}\u0000${site.created_at}`, at: site.created_at, site, href: siteHref(site) };
}

export function collectPublished(data, since) {
  return (data.sites || [])
    .filter((site) => site.created_at != null && site.created_at > since)
    .map(publishedNotice)
    .sort((a, b) => a.at - b.at);
}

function latestReporters(site, author) {
  return (site.reporters || []).filter((r) => r.latest && r.pubkey !== author);
}

function readReporterBook() {
  const raw = storage.get(REPLICA_REPORTERS_KEY, null);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

/* Each reporter keeps the report cursor at which it first appeared, so that additions announced but never acknowledged come back after a reload. */
export function diffReporters(data, book, since, latest) {
  const author = (data.authors || [])[0];
  const next = {};
  const added = [];
  if (!author) return { next, added };
  for (const site of author.sites || []) {
    const before = (book && book[site.d]) || {};
    const entry = {};
    const fresh = [];
    for (const r of latestReporters(site, author.pubkey)) {
      const known = Number.isFinite(before[r.pubkey]) ? before[r.pubkey] : null;
      entry[r.pubkey] = known ?? latest;
      if (known == null || known > since) fresh.push({ pubkey: r.pubkey, npub: r.npub, tier: r.tier });
    }
    if (Object.keys(entry).length > 0) next[site.d] = entry;
    if (fresh.length > 0) {
      const at = Math.max(...fresh.map((r) => entry[r.pubkey]));
      added.push({ d: site.d, reporters: fresh, replicas: site.replicas, unverified: site.unverified, at });
    }
  }
  return { next, added, author };
}

function replicaNotice(add, published) {
  const site = { d: add.d, title: published ? published.title : null };
  return {
    kind: 'replica',
    key: `replica\u0000${add.d}`,
    at: add.at,
    site,
    href: published ? siteHref(published) : null,
    added: add.reporters,
    replicas: add.replicas,
    unverified: add.unverified,
  };
}

/* `announced` runs ahead of the persisted seen value so that notices nobody acknowledged come back after a reload. */
export function createUpdateWatcher({ currentSites, reloadSites, isActive }) {
  const listeners = new Set();
  const announced = { stored: readSeen('stored'), published: readSeen('published'), replica: readSeen('replica') };
  let settings = readNotifySettings();
  let timer = null;
  let intervalMs = 60000;
  let started = false;
  let inFlight = false;
  let failing = false;

  function emit(event) {
    for (const fn of listeners) {
      try {
        fn(event);
      } catch (err) {
        console.error(err);
      }
    }
  }

  function persistSeen(kind, value) {
    const seen = readSeen(kind);
    if (seen == null || value > seen) writeSeen(kind, value);
  }

  function acknowledge(notices) {
    const newest = {};
    for (const n of notices) {
      if (newest[n.kind] == null || n.at > newest[n.kind]) newest[n.kind] = n.at;
    }
    for (const [kind, at] of Object.entries(newest)) persistSeen(kind, at);
  }

  /* No mascot will bring up a kind whose mascot side is off, so reading it here keeps reloads from fetching and announcing it again. */
  function announce(kind, notices, upTo) {
    if (notices.length > 0) emit({ kind: EVENT_KINDS[kind], notices });
    if (!settings.mascot[kind]) persistSeen(kind, upTo);
  }

  function markAnnounced(kind, value) {
    if (announced[kind] == null || value > announced[kind]) announced[kind] = value;
  }

  function skipTo(kind, latest) {
    if (latest == null) return;
    persistSeen(kind, latest);
    markAnnounced(kind, latest);
  }

  async function checkStored(latest) {
    if (readSeen('stored') == null) {
      writeSeen('stored', latest ?? 0);
      announced.stored = latest ?? 0;
      return;
    }
    if (!kindWanted(settings, 'stored')) return skipTo('stored', latest);
    if (latest == null || latest <= announced.stored) return;
    const data = await sitesCovering(latest);
    if (!data) return;
    const notices = collectNotices(data, announced.stored);
    markAnnounced('stored', latest);
    announce('stored', notices, latest);
  }

  async function checkPublished(latest) {
    if (latest == null) return;
    if (readSeen('published') == null) {
      writeSeen('published', latest);
      markAnnounced('published', latest);
      return;
    }
    if (!kindWanted(settings, 'published')) return skipTo('published', latest);
    if (announced.published != null && latest <= announced.published) return;
    const data = await reloadPublishSites();
    const notices = collectPublished(data, announced.published ?? readSeen('published'));
    markAnnounced('published', latest);
    announce('published', notices, latest);
  }

  async function checkReplicas(latest) {
    if (latest == null) return;
    if (!kindWanted(settings, 'replica')) {
      storage.remove(REPLICA_REPORTERS_KEY);
      storage.remove(SEEN_REPLICAS_KEY);
      announced.replica = null;
      return;
    }
    const book = readReporterBook();
    const first = book == null || readSeen('replica') == null;
    if (!first && announced.replica != null && latest <= announced.replica) return;
    if (first && latest === 0) {
      storage.set(REPLICA_REPORTERS_KEY, '{}');
      writeSeen('replica', 0);
      announced.replica = 0;
      return;
    }
    const data = await apiFetch('/api/replicas');
    const since = first ? latest : (announced.replica ?? readSeen('replica'));
    const { next, added, author } = diffReporters(data, first ? null : book, since, latest);
    if (author) cache.replicasByKey.set(author.pubkey, data);
    storage.set(REPLICA_REPORTERS_KEY, JSON.stringify(next));
    if (first) {
      writeSeen('replica', latest);
      announced.replica = latest;
      return;
    }
    markAnnounced('replica', latest);
    const published = added.length > 0 ? await publishedSitesFor(added.map((a) => a.d)) : new Map();
    const notices = added.map((a) => replicaNotice(a, published.get(a.d))).sort((a, b) => a.at - b.at);
    announce('replica', notices, latest);
  }

  async function reloadPublishSites() {
    const data = await apiFetch('/api/publish/sites');
    cache.publishSites = data;
    return data;
  }

  async function publishedSitesFor(ds) {
    let data = cache.publishSites;
    if (!data || !ds.every((d) => data.sites.some((s) => s.d === d))) {
      try {
        data = await reloadPublishSites();
      } catch {
        data = data || { sites: [] };
      }
    }
    return new Map(data.sites.map((s) => [s.d, s]));
  }

  async function check() {
    if (intervalMs == null || inFlight || !isActive()) return;
    inFlight = true;
    settings = readNotifySettings();
    try {
      const activity = await apiFetch('/api/activity');
      await checkStored(activity.latest_stored_at);
      await checkPublished(activity.latest_published_at);
      await checkReplicas(activity.latest_replica_report_at);
      if (failing) {
        failing = false;
        emit({ kind: 'recovered' });
      }
    } catch (err) {
      if (!failing) {
        failing = true;
        emit({ kind: 'fetch-error', error: err });
      }
    } finally {
      inFlight = false;
    }
  }

  async function sitesCovering(latest) {
    const cached = currentSites();
    if (cached && (newestStoredAt(cached) ?? -1) >= latest) return cached;
    return reloadSites();
  }

  function schedule() {
    clearTimeout(timer);
    timer = null;
    if (intervalMs == null) return;
    timer = setTimeout(async () => {
      await check();
      schedule();
    }, intervalMs);
  }

  function onPublished(body) {
    if (!body || typeof body.site !== 'string' || !Number.isFinite(body.created_at)) return;
    const at = body.created_at;
    if (readSeen('published') == null) writeSeen('published', at - 1);
    markAnnounced('published', at);
    settings = readNotifySettings();
    if (!kindWanted(settings, 'published')) {
      persistSeen('published', at);
      return;
    }
    const site = { d: body.site, url: body.url ?? null, title: body.title ?? null, message: body.message ?? null, cid: body.cid, size: body.size, created_at: at, gateway_url: body.gateway_url ?? null };
    announce('published', [publishedNotice(site)], at);
  }

  return {
    start() {
      document.addEventListener('visibilitychange', () => {
        if (!document.hidden) check();
      });
      document.addEventListener('swing:published', (ev) => onPublished(ev.detail));
      started = true;
      schedule();
    },
    setInterval(ms) {
      intervalMs = ms;
      if (started) schedule();
    },
    checkNow() {
      return check();
    },
    subscribe(fn) {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    acknowledge,
    isAcknowledged(notice) {
      const seen = readSeen(notice.kind);
      return seen != null && notice.at <= seen;
    },
  };
}
