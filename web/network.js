import { t } from './i18n.js';
import { apiFetch, el, setStatus, clearStatus, describeError, createLoadGuard, setBusy, maybeLink, shortenMiddle } from './util.js';
import { fingerprintBadge } from './ui.js';

const networkEls = {
  status: document.getElementById('network-status'),
  content: document.getElementById('network-content'),
  reload: document.getElementById('network-reload'),
};

const STATES = {
  public: { kind: 'ok', label: 'networkPublic', text: 'networkPublicText' },
  private: { kind: 'warn', label: 'networkPrivate', text: 'networkPrivateText' },
  unknown: { kind: 'info', label: 'networkUnknown', text: 'networkUnknownText' },
  offline: { kind: 'error', label: 'networkOffline', text: 'networkOfflineText', bare: true },
};

const ADDRESS_GROUPS = [
  { field: 'reachable', label: 'networkAddrReachable', reach: 'reachable' },
  { field: 'unreachable', label: 'networkAddrUnreachable', reach: 'unreachable' },
  { field: 'unknown', label: 'networkAddrUnknown', reach: 'unknown' },
];

const networkLoadGuard = createLoadGuard();
let network = null;

function addressTable(data) {
  const rows = ADDRESS_GROUPS.flatMap((g) =>
    (data[g.field] || []).map((addr) =>
      el('tr', {}, [el('td', { class: 'swing-mono' }, addr), el('td', { class: 'swing-nowrap' }, el('span', { class: 'swing-badge', 'data-reach': g.reach }, t(g.label)))]),
    ),
  );
  if (!rows.length) return el('p', { class: 'swing-hint' }, t('networkNoAddresses'));
  const table = el('table', { class: 'swing-table' });
  table.append(el('thead', {}, el('tr', {}, [el('th', {}, t('networkAddress')), el('th', {}, t('networkAddrState'))])));
  table.append(el('tbody', {}, rows));
  return el('div', { class: 'swing-table-scroll' }, table);
}

function anchorReason(a) {
  if (a.reason === 'mutual') return t('networkAnchorMutual');
  return t(a.sites === 1 ? 'networkAnchorSite' : 'networkAnchorSites', { n: a.sites });
}

function anchorConnected(a) {
  if (a.connected == null) return '–';
  return a.connected
    ? el('span', { class: 'swing-badge', 'data-reach': 'reachable' }, t('networkAnchorConnected'))
    : el('span', { class: 'swing-badge', 'data-reach': 'unknown' }, t('networkAnchorNotConnected'));
}

function advertiseLine(anchor) {
  if (anchor.advertising) return 'networkAnchorAdvertising';
  if (!anchor.advertise) return 'networkAnchorAdvertiseOff';
  return 'networkAnchorIdle';
}

function anchorSection(anchor) {
  const parts = [el('h3', {}, t('networkAnchorHeading'))];
  const anchors = anchor.anchors || [];
  if (!anchors.length) {
    parts.push(el('p', { class: 'swing-hint' }, t(anchor.max_peers === 0 ? 'networkAnchorsOff' : 'networkNoAnchors')));
    return parts;
  }
  const rows = anchors.map((a) => el('tr', {}, [
    el('td', { class: 'swing-nowrap', title: a.npub }, [fingerprintBadge(a.fingerprint, 1), ' ', shortenMiddle(a.npub, 10, 4)]),
    el('td', { class: 'swing-mono swing-nowrap', title: a.peer_id }, shortenMiddle(a.peer_id, 8, 6)),
    el('td', { class: 'swing-nowrap' }, anchorReason(a)),
    el('td', { class: 'swing-nowrap' }, anchorConnected(a)),
  ]));
  const table = el('table', { class: 'swing-table' });
  table.append(el('thead', {}, el('tr', {}, ['networkAnchorAccount', 'networkAnchorPeer', 'networkAnchorReason', 'networkAnchorState'].map((k) => el('th', {}, t(k))))));
  table.append(el('tbody', {}, rows));
  parts.push(el('p', { class: 'swing-hint' }, t('networkAnchorsHint')), el('div', { class: 'swing-table-scroll' }, table));
  return parts;
}

export function renderNetwork() {
  if (!network) return;
  const state = STATES[network.reachability] || STATES.unknown;
  const stateLine = el('div', { class: 'swing-status', 'data-kind': state.kind }, t(state.label));
  const parts = [stateLine, el('p', {}, t(state.text))];
  if (network.reachability === 'public' && network.anchor) parts.push(el('p', {}, t(advertiseLine(network.anchor))));
  if (!state.bare) {
    if (network.reachability !== 'public') {
      if (network.nat) parts.push(el('p', {}, t('networkBehindNat')));
      parts.push(el('p', {}, maybeLink(network.guide, t('networkGuide'))));
    }
    parts.push(addressTable(network));
  }
  if (!state.bare && network.anchor) parts.push(...anchorSection(network.anchor));
  networkEls.content.replaceChildren(...parts);
}

export async function loadNetwork() {
  const gen = networkLoadGuard.start();
  setBusy(networkEls.reload, true);
  if (!network) setStatus(networkEls.status, 'loading', t('networkLoading'));
  try {
    const data = await apiFetch('/api/network');
    if (!networkLoadGuard.isCurrent(gen)) return;
    network = data;
    clearStatus(networkEls.status);
    renderNetwork();
  } catch (err) {
    if (!networkLoadGuard.isCurrent(gen)) return;
    network = null;
    networkEls.content.replaceChildren();
    setStatus(networkEls.status, 'error', describeError(err));
  } finally {
    if (networkLoadGuard.isCurrent(gen)) setBusy(networkEls.reload, false);
  }
}

export function initNetwork() {
  networkEls.reload.addEventListener('click', loadNetwork);
}
