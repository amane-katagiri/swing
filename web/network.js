import { t } from './i18n.js';
import { apiFetch, el, setStatus, clearStatus, describeError, createLoadGuard, setBusy, maybeLink } from './util.js';

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

export function renderNetwork() {
  if (!network) return;
  const state = STATES[network.reachability] || STATES.unknown;
  const stateLine = el('div', { class: 'swing-status', 'data-kind': state.kind }, t(state.label));
  const parts = [stateLine, el('p', {}, t(state.text))];
  if (!state.bare) {
    if (network.reachability !== 'public') {
      if (network.nat) parts.push(el('p', {}, t('networkBehindNat')));
      parts.push(el('p', {}, maybeLink(network.guide, t('networkGuide'))));
    }
    parts.push(addressTable(network));
  }
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
