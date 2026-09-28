import { storage } from './storage.js';
import { t } from './i18n.js';
import {
  cache,
  el,
  apiFetch,
  setStatus,
  clearStatus,
  describeError,
  shortenMiddle,
  sanitizeDisplayText,
  stripUnsafeUnicode,
  clamp,
  setBusy,
  getStyle,
  wireStyleSwitch,
  createLoadGuard,
} from './util.js';
import { copyButton, buildRemoveControl } from './ui.js';
import { createWebringGraph } from './graph.js';
import { SHOW_SELF_IN_WEBRING } from './notify-settings.js';

const webringEls = {
  status: document.getElementById('webring-status'),
  content: document.getElementById('webring-content'),
  detail: document.getElementById('webring-detail'),
  form: document.getElementById('webring-query-form'),
  layout: document.getElementById('webring-layout'),
};

function setDetailVisible(visible) {
  webringEls.detail.hidden = !visible;
  webringEls.layout.dataset.detail = String(visible);
}

function saveWebringQuery(rootVal, depthVal) {
  storage.set('swing:webring:query', JSON.stringify({ root: rootVal, depth: depthVal }));
}

function loadSavedWebringQuery() {
  const raw = storage.get('swing:webring:query', null);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object') return null;
    const root = typeof parsed.root === 'string' ? parsed.root : '';
    const depth = clamp(parseInt(parsed.depth, 10) || 0, 0, 4);
    return { root, depth };
  } catch {
    return null;
  }
}

function setWebringQuery(rootVal, depthVal) {
  currentWebringQuery = { roots: rootVal ? rootVal.split(/[\s,]+/).filter(Boolean) : [], depth: depthVal };
  saveWebringQuery(rootVal, depthVal);
}

let currentWebringQuery = { roots: [], depth: 2 };
let graphInstance = null;
let selectedNodePubkey = null;
const webringLoadGuard = createLoadGuard();
const selectNodeGuard = createLoadGuard();

function webringQueryKey(q) {
  const params = new URLSearchParams();
  for (const r of q.roots) params.append('root', r);
  params.set('depth', String(q.depth));
  return params.toString();
}

function labelOf(map, pk) {
  const n = map.get(pk);
  return n ? sanitizeDisplayText(n.label) : shortenMiddle(pk, 8, 4);
}

function tierTag(tier) {
  if (tier === 'author') return t('tagAuthor');
  if (tier === 'chosen') return t('tagChosen');
  return t('tagUnverified');
}

function graphLabels() {
  return {
    root: t('legendRoot'),
    mutual: t('legendMutual'),
    oneway: t('legendOneway'),
    noFollowSet: t('legendNoFollowSet'),
    fit: t('graphFit'),
    ariaLabel: t('graphAriaLabel'),
    empty: t('graphEmpty'),
  };
}

function renderGraphStyle(data) {
  graphInstance = createWebringGraph(webringEls.content, {
    nodes: data.nodes,
    edges: data.edges,
    selectedPubkey: selectedNodePubkey,
    onSelect: (node) => selectNode(node.pubkey),
    labels: graphLabels(),
  });
}

function renderListStyle(data) {
  const wrap = el('div', { class: 'swing-webring-list' });
  const nodeByKey = new Map(data.nodes.map((n) => [n.pubkey, n]));

  const accounts = el('div', { class: 'swing-webring-group' }, el('h3', {}, t('accountsHeading', { n: data.nodes.length })));
  const sorted = [...data.nodes].sort((a, b) => a.depth - b.depth || a.label.localeCompare(b.label));
  for (const n of sorted) {
    accounts.append(
      el('div', { class: 'swing-webring-account-row' }, [
        el('button', { type: 'button', dir: 'auto', onclick: () => selectNode(n.pubkey) }, sanitizeDisplayText(n.label)),
        el('span', { class: 'swing-hint' }, ` ${t('depthPrefix', { n: n.depth })}${n.root ? ` ${t('tagRoot')}` : ''}${!n.has_follow_set ? ` ${t('tagNoFollowSet')}` : ''}`),
      ]),
    );
  }
  wrap.append(accounts);

  const mutual = data.edges.filter((e) => e.mutual);
  const oneway = data.edges.filter((e) => !e.mutual);

  const mutualGroup = el('div', { class: 'swing-webring-group' }, el('h3', {}, t('mutualHeading', { n: mutual.length })));
  if (mutual.length === 0) mutualGroup.append(el('p', { class: 'swing-hint' }, t('none')));
  for (const e of mutual) mutualGroup.append(el('div', { class: 'swing-webring-account-row', dir: 'auto' }, `${labelOf(nodeByKey, e.from)} ↔ ${labelOf(nodeByKey, e.to)}`));
  wrap.append(mutualGroup);

  const onewayGroup = el('div', { class: 'swing-webring-group' }, el('h3', {}, t('onewayHeading', { n: oneway.length })));
  if (oneway.length === 0) onewayGroup.append(el('p', { class: 'swing-hint' }, t('none')));
  for (const e of oneway) onewayGroup.append(el('div', { class: 'swing-webring-account-row', dir: 'auto' }, `${labelOf(nodeByKey, e.from)} → ${labelOf(nodeByKey, e.to)}`));
  wrap.append(onewayGroup);

  const referencing = data.referencing || { accounts: [], more: 0 };
  const referencingGroup = el('div', { class: 'swing-webring-group' }, el('h3', {}, t('referencingHeading', { n: referencing.accounts.length })));
  if (referencing.accounts.length === 0) referencingGroup.append(el('p', { class: 'swing-hint' }, t('none')));
  for (const acct of referencing.accounts) referencingGroup.append(el('div', { class: 'swing-webring-account-row' }, acct.npub));
  if (referencing.more > 0) referencingGroup.append(el('p', { class: 'swing-hint' }, t('referencingMoreHint', { more: referencing.more })));
  wrap.append(referencingGroup);

  webringEls.content.append(wrap);
}

function renderAsciiStyle(data) {
  webringEls.content.append(el('pre', { class: 'swing-pre' }, stripUnsafeUnicode(data.text || '')));
}

function buildSourceBlock(title, text) {
  const block = el('div', { class: 'swing-source-block' });
  block.append(
    el('div', { class: 'swing-source-block-head' }, [
      el('h3', {}, title),
      copyButton(text || ''),
    ]),
  );
  block.append(el('pre', { class: 'swing-pre' }, text || ''));
  return block;
}

function renderSourceStyle(data) {
  webringEls.content.append(buildSourceBlock(t('graphvizTitle'), stripUnsafeUnicode(data.dot)));
  webringEls.content.append(buildSourceBlock(t('mermaidTitle'), stripUnsafeUnicode(data.mermaid)));
}

async function getMirrorMemberSet() {
  if (cache.sites) return new Set(cache.sites.accounts.map((a) => a.pubkey));
  if (cache.mirror) return new Set(cache.mirror.members.map((m) => m.pubkey));
  const data = await apiFetch('/api/mirror');
  cache.mirror = data;
  return new Set(data.members.map((m) => m.pubkey));
}

async function selectNode(pubkey) {
  selectedNodePubkey = pubkey;
  if (graphInstance) graphInstance.setSelected(pubkey);
  const gen = selectNodeGuard.start();
  setDetailVisible(true);
  webringEls.detail.replaceChildren(el('p', { class: 'swing-status', 'data-kind': 'loading' }, t('loadingReplicas')));
  const data = cache.webringByQuery.get(webringQueryKey(currentWebringQuery));
  const node = data ? data.nodes.find((n) => n.pubkey === pubkey) : null;
  try {
    const cachedReplicas = cache.replicasByKey.get(pubkey);
    const [replicas, memberSet] = await Promise.all([
      cachedReplicas || apiFetch(`/api/replicas?key=${encodeURIComponent(pubkey)}`),
      getMirrorMemberSet(),
    ]);
    if (!cachedReplicas) cache.replicasByKey.set(pubkey, replicas);
    if (!selectNodeGuard.isCurrent(gen)) return;
    renderNodeDetail(node, pubkey, replicas, memberSet);
  } catch (err) {
    if (!selectNodeGuard.isCurrent(gen)) return;
    webringEls.detail.replaceChildren(el('p', { class: 'swing-status', 'data-kind': 'error' }, describeError(err)));
  }
}

function renderNodeDetail(node, pubkey, replicasResp, memberSet) {
  webringEls.detail.replaceChildren();
  const isMirrored = memberSet.has(pubkey);
  webringEls.detail.append(
    el('div', { class: 'swing-heading-row' }, [
      el('h2', { dir: 'auto' }, node ? sanitizeDisplayText(node.label) : t('tableAccount')),
      isMirrored ? el('span', { class: 'swing-badge', 'data-mirrored': 'true' }, t('mirroredBadge')) : null,
    ]),
  );
  const dl = el('dl');
  const npubDd = node
    ? el('span', { class: 'swing-copyable' }, [
        document.createTextNode(node.short_npub || node.npub),
        copyButton(node.npub, t('copyNpub')),
      ])
    : shortenMiddle(pubkey, 10, 6);
  dl.append(el('dt', {}, 'npub'), el('dd', {}, npubDd));
  if (node) {
    const names = node.names.map((n) => sanitizeDisplayText(n)).join(', ');
    dl.append(el('dt', {}, t('detailNames')), el('dd', { dir: 'auto' }, node.names.length ? names : '–'));
    dl.append(el('dt', {}, t('detailDepth')), el('dd', {}, String(node.depth)));
  }
  webringEls.detail.append(dl);

  const author = replicasResp.authors.find((a) => a.pubkey === pubkey);
  if (!author || author.sites.length === 0) {
    webringEls.detail.append(el('p', { class: 'swing-hint' }, t('noPublishedSitesForNode')));
  } else {
    for (const site of author.sites) {
      const block = el('div', { class: 'swing-site' });
      const countText = site.unverified > 0
        ? t('replicaCountWithUnverified', { replicas: site.replicas, unverified: site.unverified })
        : t('replicaCountBadge', { replicas: site.replicas });
      block.append(
        el('div', { class: 'swing-site-row' }, [
          el('span', { class: 'swing-site-name', dir: 'auto' }, sanitizeDisplayText(site.d)),
          el('span', { class: 'swing-badge' }, countText),
        ]),
      );
      const reporters = el('ul', { class: 'swing-plain-list' });
      for (const r of site.reporters) {
        reporters.append(
          el('li', {}, `${r.npub} ${r.latest ? t('tagLatest') : t('tagOlderVersion')}${tierTag(r.tier)}`),
        );
      }
      block.append(reporters);
      if (site.dropped > 0) {
        block.append(el('p', { class: 'swing-hint' }, t('reportsMoreHint', { dropped: site.dropped })));
      }
      webringEls.detail.append(block);
    }
  }

  const actions = el('div', { class: 'swing-node-detail-actions' }, [
    el('button', { type: 'button', class: 'swing-btn', onclick: (ev) => useAsRoot(pubkey, ev.currentTarget) }, t('useAsRoot')),
  ]);
  const isSelf = cache.overview && cache.overview.pubkey === pubkey;
  if (!isSelf) {
    if (isMirrored) {
      actions.append(buildDetailRemoveControl(pubkey));
    } else {
      actions.append(el('button', { type: 'button', class: 'swing-btn swing-btn-accent', onclick: (ev) => addToMirrorFromDetail(pubkey, ev.currentTarget) }, t('addToMirror')));
    }
  }
  webringEls.detail.append(actions);
}

function useAsRoot(pubkey, btn) {
  webringEls.form.elements.root.value = pubkey;
  setWebringQuery(pubkey, currentWebringQuery.depth);
  WebringView.load(true, btn);
}

function buildDetailRemoveControl(pubkey) {
  return buildRemoveControl({ pubkey }, async (result, err) => {
    if (err) {
      webringEls.detail.append(el('p', { class: 'swing-status', 'data-kind': 'error' }, describeError(err)));
      return;
    }
    cache.sites = null;
    cache.mirror = null;
    await selectNode(pubkey);
  }, { size: 'normal', inline: true });
}

async function addToMirrorFromDetail(pubkey, btn) {
  setBusy(btn, true);
  try {
    await apiFetch('/api/mirror/add', { method: 'POST', body: JSON.stringify({ keys: [pubkey] }) });
    cache.sites = null;
    cache.mirror = null;
    setBusy(btn, false);
    await selectNode(pubkey);
  } catch (err) {
    setBusy(btn, false);
    webringEls.detail.append(el('p', { class: 'swing-status', 'data-kind': 'error' }, describeError(err)));
  }
}

let showSelfPending = false;

async function showSelf() {
  const self = cache.overview && cache.overview.pubkey;
  if (!self) return;
  const hasSelf = () => {
    const data = cache.webringByQuery.get(webringQueryKey(currentWebringQuery));
    return !!data && data.nodes.some((n) => n.pubkey === self);
  };
  await WebringView.load();
  if (!hasSelf()) {
    webringEls.form.elements.root.value = '';
    setWebringQuery('', currentWebringQuery.depth);
    await WebringView.load(true);
  }
  if (hasSelf()) selectNode(self);
}

export const WebringView = {
  init() {
    document.addEventListener(SHOW_SELF_IN_WEBRING, () => {
      if (document.body.dataset.view === 'webring') {
        showSelf();
        return;
      }
      showSelfPending = true;
      location.hash = '#/webring';
    });
    const saved = loadSavedWebringQuery();
    if (saved) {
      webringEls.form.elements.root.value = saved.root;
      webringEls.form.elements.depth.value = String(saved.depth);
      currentWebringQuery = { roots: saved.root ? saved.root.split(/[\s,]+/).filter(Boolean) : [], depth: saved.depth };
    }
    wireStyleSwitch('webring', () => this.render());
    webringEls.form.addEventListener('submit', (ev) => {
      ev.preventDefault();
      const rootVal = webringEls.form.elements.root.value.trim();
      const depthVal = clamp(parseInt(webringEls.form.elements.depth.value, 10) || 0, 0, 4);
      webringEls.form.elements.depth.value = String(depthVal);
      setWebringQuery(rootVal, depthVal);
      this.load(true, webringEls.form.querySelector('button[type="submit"]'));
    });
  },
  onShow() {
    if (showSelfPending) {
      showSelfPending = false;
      showSelf();
      return;
    }
    if (cache.webringByQuery.has(webringQueryKey(currentWebringQuery))) this.render();
    else this.load();
  },
  async load(force, updateBtn) {
    const key = webringQueryKey(currentWebringQuery);
    if (!force && cache.webringByQuery.has(key)) return this.render();
    const gen = webringLoadGuard.start();
    const hasContent = webringEls.content.childElementCount > 0;
    if (updateBtn) setBusy(updateBtn, true);
    if (hasContent) {
      webringEls.content.setAttribute('aria-busy', 'true');
    } else {
      setStatus(webringEls.status, 'loading', t('loadingWebring'));
    }
    let data;
    try {
      data = await apiFetch(`/api/webring${key ? `?${key}` : ''}`);
    } catch (err) {
      if (!webringLoadGuard.isCurrent(gen)) return;
      cache.webringByQuery.delete(key);
      setStatus(webringEls.status, 'error', describeError(err));
      webringEls.content.replaceChildren();
      webringEls.content.removeAttribute('aria-busy');
      return;
    } finally {
      if (updateBtn) setBusy(updateBtn, false);
    }
    if (!webringLoadGuard.isCurrent(gen)) return;
    webringEls.content.removeAttribute('aria-busy');
    cache.webringByQuery.set(key, data);
    clearStatus(webringEls.status);
    this.render();
  },
  render() {
    document.body.dataset.style = getStyle('webring');
    const data = cache.webringByQuery.get(webringQueryKey(currentWebringQuery));
    if (graphInstance) {
      graphInstance.destroy();
      graphInstance = null;
    }
    webringEls.content.replaceChildren();
    setDetailVisible(false);
    if (!data) return;

    if (data.nodes.length === 0) {
      webringEls.content.append(el('p', { class: 'swing-status', 'data-kind': 'empty' }, t('noAccountsForRootDepth')));
      return;
    }

    const style = getStyle('webring');
    if (style === 'graph') renderGraphStyle(data);
    else if (style === 'list') renderListStyle(data);
    else if (style === 'ascii') renderAsciiStyle(data);
    else renderSourceStyle(data);

    if (data.beyond > 0) {
      webringEls.content.append(el('p', { class: 'swing-hint' }, t('beyondHint', { beyond: data.beyond, depth: data.depth })));
    }
    if (data.over_budget > 0) {
      webringEls.content.append(el('p', { class: 'swing-hint' }, t('overBudgetHint', { overBudget: data.over_budget })));
    }
  },
};

export function renderWebringIfLoaded() {
  if (cache.webringByQuery.has(webringQueryKey(currentWebringQuery))) WebringView.render();
}
