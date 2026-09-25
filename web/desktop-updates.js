import { storage } from './storage.js';
import { apiFetch, isHttpUrl } from './util.js';

export const SEEN_KEY = 'swing:desktop:seen';

function readSeen() {
  const n = Number.parseInt(storage.get(SEEN_KEY, ''), 10);
  return Number.isFinite(n) ? n : null;
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

export function collectNotices(data, since) {
  const notices = [];
  for (const { acct, site } of storedSites(data)) {
    const href = site.gateway_url || site.url;
    if (!isHttpUrl(href) || site.stored_at == null || site.stored_at <= since) continue;
    notices.push({ pubkey: acct.pubkey, npub: acct.npub, site, href, storedAt: site.stored_at });
  }
  notices.sort((a, b) => a.storedAt - b.storedAt);
  return notices;
}

/* `announced` runs ahead of the persisted `seen` so that notices nobody acknowledged come back after a reload. */
export function createUpdateWatcher({ currentSites, reloadSites, isActive }) {
  const listeners = new Set();
  let seen = readSeen();
  let announced = seen;
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

  function persistSeen(value) {
    seen = value;
    storage.set(SEEN_KEY, String(value));
  }

  async function check() {
    if (intervalMs == null || inFlight || !isActive()) return;
    inFlight = true;
    try {
      const { latest_stored_at: latest } = await apiFetch('/api/activity');
      if (failing) {
        failing = false;
        emit({ kind: 'recovered' });
      }
      if (seen == null) {
        persistSeen(latest ?? 0);
        announced = seen;
        return;
      }
      if (latest == null || latest <= announced) return;
      const data = await sitesCovering(latest);
      if (!data) return;
      const notices = collectNotices(data, announced);
      announced = latest;
      if (notices.length > 0) emit({ kind: 'sites-stored', notices });
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

  return {
    start() {
      document.addEventListener('visibilitychange', () => {
        if (!document.hidden) check();
      });
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
    acknowledge(storedAt) {
      if (seen == null || storedAt > seen) persistSeen(storedAt);
    },
  };
}
