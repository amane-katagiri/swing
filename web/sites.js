import { storage } from './storage.js';
import { t } from './i18n.js';
import {
  cache,
  el,
  apiFetch,
  setStatus,
  clearStatus,
  describeError,
  formatBytes,
  formatSiteSize,
  formatTime,
  shortenMiddle,
  stripControlChars,
  maybeLink,
  setBusy,
  getStyle,
  wireStyleSwitch,
  wireSortSwitch,
  createLoadGuard,
  sanitizeMessage,
} from './util.js';
import { copyButton, storedBadge, appendLinksAndMessage, renderMirrorOpResult, renderOpError, buildRemoveControl } from './ui.js';

const MAX_MIRROR_KEYS = 100;

const SITE_FIELD_DEFAULTS = { url: null, title: null, message: null, nip05: null, replicas: null, stored: null, stored_size: null, gateway_url: null };

function normalizeSite(site, contextDefaults) {
  return Object.assign({}, SITE_FIELD_DEFAULTS, contextDefaults || {}, site);
}

function matchesFilter(site, acct, filterVal, storedOnly) {
  if (storedOnly && !site.stored) return false;
  if (!filterVal) return true;
  const hay = [site.d, site.message, site.cid, site.url, acct.npub, acct.pubkey]
    .filter(Boolean)
    .join(' ')
    .toLowerCase();
  return hay.includes(filterVal);
}

function sortAccounts(accounts, mode) {
  if (mode === 'pubkey') return [...accounts].sort((a, b) => (a.npub < b.npub ? -1 : a.npub > b.npub ? 1 : 0));
  const bySite = mode === 'name'
    ? (a, b) => a.d.localeCompare(b.d)
    : (a, b) => (b.created_at || 0) - (a.created_at || 0);
  const withSites = [];
  const withoutSites = [];
  for (const acct of accounts) {
    if (acct.sites.length === 0) withoutSites.push(acct);
    else withSites.push(Object.assign({}, acct, { sites: [...acct.sites].sort(bySite) }));
  }
  withSites.sort((a, b) => bySite(a.sites[0], b.sites[0]));
  return [...withSites, ...withoutSites];
}

const NIP05_KEYS = { verified: 'Verified', mismatch: 'Mismatch', error: 'Error', not_applicable: 'NotApplicable' };

function nip05Badge(status, template) {
  const key = Object.hasOwn(NIP05_KEYS, status) ? NIP05_KEYS[status] : null;
  const label = key ? t(`nip05Label${key}`) : status;
  return el('span', {
    class: 'swing-badge',
    'data-nip05': status,
    title: key ? t(`nip05Desc${key}`) : null,
  }, template ? t(template, { status: label }) : label);
}

function buildSiteEntry(site) {
  const wrap = el('div', { class: 'swing-site', 'data-stored': String(!!site.stored) });
  const title = sanitizeMessage(site.title);
  const row = el('div', { class: 'swing-site-row' }, [
    el('span', { class: 'swing-site-name' }, site.d),
    title ? el('span', { class: 'swing-hint' }, title) : null,
  ]);
  const badges = el('div', { class: 'swing-site-badges' }, [
    storedBadge(site),
    site.nip05 ? nip05Badge(site.nip05, 'nip05Badge') : null,
    el('span', { class: 'swing-badge' }, t('replicasBadge', { n: site.replicas == null ? '–' : site.replicas })),
  ]);
  wrap.append(row, badges);

  const meta = el('div', { class: 'swing-site-meta' }, [
    el('div', { class: 'swing-site-meta-cid' }, [
      el('span', { class: 'swing-copyable' }, [
        document.createTextNode(`cid: ${shortenMiddle(site.cid, 10, 6)}`),
        copyButton(site.cid),
      ]),
    ]),
    el('div', { class: 'swing-site-meta-info' }, `${formatSiteSize(site)} · ${formatTime(site.created_at)}`),
  ]);
  wrap.append(meta);

  appendLinksAndMessage(wrap, site);

  return wrap;
}

function buildSiteTable(sites) {
  const table = el('table', { class: 'swing-table' });
  const headers = [t('tableSite'), t('tableStored'), 'NIP-05', t('tableReplicas'), 'CID', t('tableSize'), t('tableUpdated'), t('tableLinks')];
  table.append(el('thead', {}, el('tr', {}, headers.map((h) => el('th', {}, h)))));
  const tbody = el('tbody');
  for (const site of sites) {
    const tr = el('tr', { 'data-stored': String(!!site.stored) });
    tr.append(el('td', {}, site.d));
    tr.append(el('td', {}, storedBadge(site)));
    tr.append(el('td', {}, site.nip05 ? nip05Badge(site.nip05) : '–'));
    tr.append(el('td', {}, site.replicas == null ? '–' : String(site.replicas)));
    tr.append(
      el('td', { class: 'swing-mono' }, [
        document.createTextNode(`${shortenMiddle(site.cid, 8, 6)} `),
        copyButton(site.cid),
      ]),
    );
    tr.append(el('td', {}, formatSiteSize(site)));
    tr.append(el('td', {}, formatTime(site.created_at)));
    const linksTd = el('td', {});
    if (site.url) linksTd.append(maybeLink(site.url, t('openSite')));
    if (site.url && site.gateway_url) linksTd.append(document.createTextNode(' '));
    if (site.gateway_url) linksTd.append(maybeLink(site.gateway_url, t('openGateway')));
    if (!site.url && !site.gateway_url) linksTd.append(document.createTextNode('–'));
    tr.append(linksTd);
    tbody.append(tr);
  }
  table.append(tbody);
  return table;
}

const sitesEls = {
  status: document.getElementById('sites-status'),
  content: document.getElementById('sites-content'),
  unfollowedSection: document.getElementById('sites-unfollowed'),
  unfollowedNote: document.getElementById('unfollowed-note'),
  unfollowedContent: document.getElementById('unfollowed-content'),
  filterText: document.getElementById('sites-filter-text'),
  filterStored: document.getElementById('sites-filter-stored'),
  mirrorAddForm: document.getElementById('mirror-add-form'),
  mirrorAddResult: document.getElementById('mirror-add-result'),
  runStatusBtn: document.getElementById('run-status-check'),
  statusCheckResult: document.getElementById('status-check-result'),
};

export function renderStatusCheck(status) {
  sitesEls.statusCheckResult.replaceChildren();
  sitesEls.statusCheckResult.append(el('p', {}, status.problems === 0 ? t('noProblemsFound') : t('problemsFound', { n: status.problems })));
  if (status.versions.length) {
    const table = el('table', { class: 'swing-table' });
    const headers = [t('tableAccount'), t('tableSite'), t('tablePath'), 'CID', t('tableSize'), t('tableCreated'), t('tableHealth')];
    table.append(el('thead', {}, el('tr', {}, headers.map((h) => el('th', {}, h)))));
    const tbody = el('tbody');
    for (const v of status.versions) {
      const tr = el('tr', { 'data-health': v.health });
      tr.append(el('td', {}, v.npub ? shortenMiddle(v.npub, 10, 4) : '–'));
      tr.append(el('td', {}, v.d || '–'));
      tr.append(el('td', { class: 'swing-mono' }, v.path || '–'));
      tr.append(el('td', { class: 'swing-mono' }, v.cid ? shortenMiddle(v.cid, 8, 6) : '–'));
      tr.append(el('td', {}, formatBytes(v.size)));
      tr.append(el('td', {}, formatTime(v.created_at)));
      tr.append(
        el('td', {}, [
          el('span', { class: 'swing-badge', 'data-health': v.health }, v.health),
          v.detail ? el('span', { class: 'swing-hint' }, ` ${stripControlChars(v.detail)}`) : null,
        ]),
      );
      tbody.append(tr);
    }
    table.append(tbody);
    sitesEls.statusCheckResult.append(table);
  }
  if (status.sites.length) {
    sitesEls.statusCheckResult.append(el('h3', {}, t('actualSize')));
    sitesEls.statusCheckResult.append(el('p', { class: 'swing-hint' }, t('actualSizeHint')));
    const table = el('table', { class: 'swing-table' });
    const headers = [t('tableAccount'), t('tableSite'), t('tableSize')];
    table.append(el('thead', {}, el('tr', {}, headers.map((h) => el('th', {}, h)))));
    const tbody = el('tbody');
    for (const site of status.sites) {
      const tr = el('tr');
      tr.append(el('td', {}, shortenMiddle(site.npub, 10, 4)));
      tr.append(el('td', {}, site.d));
      tr.append(el('td', {}, site.actual == null ? '–' : formatBytes(site.actual)));
      tbody.append(tr);
    }
    table.append(tbody);
    sitesEls.statusCheckResult.append(table);
    sitesEls.statusCheckResult.append(
      el('p', {}, t('actualSizeTotal', { size: status.actual_bytes == null ? '–' : formatBytes(status.actual_bytes) })),
    );
  }
  if (status.garbage.length) {
    sitesEls.statusCheckResult.append(el('h3', {}, t('notInState')));
    const list = el('ul', { class: 'swing-plain-list' });
    for (const g of status.garbage) list.append(el('li', {}, `${g.path}${g.list_failed ? t('listFailedSuffix') : ''}`));
    sitesEls.statusCheckResult.append(list);
  }
}

const sitesLoadGuard = createLoadGuard();
const statusCheckGuard = createLoadGuard();

function buildAccountElement(acct, opts) {
  const filterVal = opts.filterVal;
  const storedOnly = opts.storedOnly;
  const filtering = Boolean(filterVal) || storedOnly;
  const sites = acct.sites.map((s) => normalizeSite(s, opts.siteDefaults)).filter((s) => matchesFilter(s, acct, filterVal, storedOnly));
  if (filtering && sites.length === 0) return null;

  const head = el('div', { class: 'swing-account-head' }, [
    el('span', { class: 'swing-account-key' }, shortenMiddle(acct.npub, 14, 6)),
    copyButton(acct.npub, t('copyNpub')),
  ]);
  if (opts.removable) {
    head.append(buildRemoveControl(acct, (result, err) => {
      if (err) renderOpError(opts.resultEl, err);
      else {
        renderMirrorOpResult(opts.resultEl, result, 'remove');
        opts.onChanged();
      }
    }));
  }

  const wrap = el('div', { class: 'swing-account' }, head);

  if (sites.length === 0) {
    wrap.append(el('p', { class: 'swing-account-empty' }, filtering ? t('noSitesMatchFilter') : t('noSitesPublishedYet')));
  } else {
    const set = el('div', { class: 'swing-site-set' });
    if (getStyle('sites') === 'table') {
      set.append(buildSiteTable(sites));
    } else {
      for (const s of sites) set.append(buildSiteEntry(s));
    }
    wrap.append(set);
  }
  return wrap;
}

export const SitesView = {
  init() {
    wireStyleSwitch('sites', () => this.render());
    sitesEls.filterText.addEventListener('input', () => this.render());
    sitesEls.filterStored.checked = storage.get('swing:sites:stored-only', '0') === '1';
    sitesEls.filterStored.addEventListener('change', () => {
      storage.set('swing:sites:stored-only', sitesEls.filterStored.checked ? '1' : '0');
      this.render();
    });
    wireSortSwitch('.swing-sort-switch', 'swing:sites:sort', 'updated', () => this.render());
    const mirrorAddBtn = sitesEls.mirrorAddForm.querySelector('button[type="submit"]');
    const mirrorKeysInput = sitesEls.mirrorAddForm.elements.keys;
    function refreshAddState() {
      mirrorAddBtn.disabled = !mirrorKeysInput.value.trim();
    }
    mirrorKeysInput.addEventListener('input', refreshAddState);
    refreshAddState();
    sitesEls.mirrorAddForm.addEventListener('submit', async (ev) => {
      ev.preventDefault();
      const input = mirrorKeysInput;
      const keys = input.value.split(/[\s,]+/).map((s) => s.trim()).filter(Boolean);
      if (keys.length === 0) {
        input.focus();
        renderOpError(sitesEls.mirrorAddResult, { message: t('mirrorKeysRequired') });
        return;
      }
      if (keys.length > MAX_MIRROR_KEYS) {
        renderOpError(sitesEls.mirrorAddResult, { message: t('tooManyKeys', { n: keys.length, max: MAX_MIRROR_KEYS }) });
        return;
      }
      setBusy(mirrorAddBtn, true);
      try {
        const result = await apiFetch('/api/mirror/add', { method: 'POST', body: JSON.stringify({ keys }) });
        renderMirrorOpResult(sitesEls.mirrorAddResult, result, 'add');
        input.value = '';
        refreshAddState();
        cache.mirror = null;
        await this.load(true);
      } catch (err) {
        renderOpError(sitesEls.mirrorAddResult, err);
      } finally {
        setBusy(mirrorAddBtn, false);
        refreshAddState();
      }
    });
    sitesEls.runStatusBtn.addEventListener('click', async () => {
      const gen = statusCheckGuard.start();
      const firstRun = sitesEls.statusCheckResult.childElementCount === 0;
      setBusy(sitesEls.runStatusBtn, true);
      if (firstRun) {
        sitesEls.statusCheckResult.replaceChildren(el('p', { class: 'swing-status', 'data-kind': 'loading' }, t('checkingStorage')));
      }
      try {
        const data = await apiFetch('/api/status');
        if (!statusCheckGuard.isCurrent(gen)) return;
        cache.status = data;
        renderStatusCheck(cache.status);
      } catch (err) {
        if (!statusCheckGuard.isCurrent(gen)) return;
        sitesEls.statusCheckResult.replaceChildren(el('p', { class: 'swing-status', 'data-kind': 'error' }, describeError(err)));
      } finally {
        if (statusCheckGuard.isCurrent(gen)) setBusy(sitesEls.runStatusBtn, false);
      }
    });
  },
  onShow() {
    if (cache.sites) this.render();
    else this.load();
  },
  async load(force, reloadBtn) {
    if (!force && cache.sites) return this.render();
    const gen = sitesLoadGuard.start();
    const firstLoad = !cache.sites;
    if (reloadBtn) setBusy(reloadBtn, true);
    if (firstLoad) setStatus(sitesEls.status, 'loading', t('loadingSites'));
    let data;
    try {
      data = await apiFetch('/api/sites');
    } catch (err) {
      if (!sitesLoadGuard.isCurrent(gen)) return;
      cache.sites = null;
      setStatus(sitesEls.status, 'error', describeError(err));
      return;
    } finally {
      if (reloadBtn) setBusy(reloadBtn, false);
    }
    if (!sitesLoadGuard.isCurrent(gen)) return;
    cache.sites = data;
    cache.mirror = null;
    this.render();
  },
  render() {
    const data = cache.sites;
    if (!data) return;
    document.body.dataset.style = getStyle('sites');

    if (data.replicas_error) {
      setStatus(sitesEls.status, 'error', t('replicasUnavailable', { reason: data.replicas_error }));
    } else if (data.follow_set && data.follow_set.note) {
      setStatus(sitesEls.status, 'empty', data.follow_set.note);
    } else if (!data.follow_set || !data.follow_set.found) {
      setStatus(sitesEls.status, 'empty', t('noMirrorFollowSet'));
    } else {
      clearStatus(sitesEls.status);
    }

    const filterVal = sitesEls.filterText.value.trim().toLowerCase();
    const storedOnly = sitesEls.filterStored.checked;
    const sortMode = storage.get('swing:sites:sort', 'updated');

    sitesEls.content.replaceChildren();
    const accountEls = sortAccounts(data.accounts, sortMode)
      .map((a) => buildAccountElement(a, {
        removable: true,
        filterVal,
        storedOnly,
        resultEl: sitesEls.mirrorAddResult,
        onChanged: () => this.load(true),
      }))
      .filter(Boolean);
    if (accountEls.length === 0) {
      sitesEls.content.append(
        el('p', { class: 'swing-status', 'data-kind': 'empty' }, data.accounts.length === 0 ? t('notFollowingAnyone') : t('noSitesMatchFilter')),
      );
    } else {
      for (const e of accountEls) sitesEls.content.append(e);
    }

    const unfollowed = data.unfollowed;
    if (unfollowed && unfollowed.accounts.length > 0) {
      sitesEls.unfollowedSection.hidden = false;
      sitesEls.unfollowedNote.textContent = unfollowed.remove_on_unfollow ? t('unfollowedRemoveNote') : t('unfollowedKeepNote');
      sitesEls.unfollowedContent.replaceChildren();
      for (const a of sortAccounts(unfollowed.accounts, sortMode)) {
        const elm = buildAccountElement(a, { removable: false, filterVal, storedOnly, siteDefaults: { stored: true } });
        if (elm) sitesEls.unfollowedContent.append(elm);
      }
    } else {
      sitesEls.unfollowedSection.hidden = true;
    }
  },
};
