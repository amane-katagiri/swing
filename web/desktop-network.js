import { t, setUiLabels } from './i18n.js';
import { apiFetch, createLoadGuard } from './util.js';

const STATES = {
  public: { icon: 'icon-desk-tray-network', label: 'networkPublic' },
  private: { icon: 'icon-desk-tray-network-private', label: 'networkPrivate' },
  unknown: { icon: 'icon-desk-tray-network-unknown', label: 'networkUnknown' },
  offline: { icon: 'icon-desk-tray-network-offline', label: 'networkOffline' },
};

const trayEl = document.getElementById('desk-tray-network');
const useEl = trayEl && trayEl.querySelector('use');
const loadGuard = createLoadGuard();
let current = 'unknown';

function show(reachability) {
  if (!trayEl || !useEl) return;
  current = Object.hasOwn(STATES, reachability) ? reachability : 'unknown';
  const state = STATES[current];
  const label = t(state.label);
  useEl.setAttribute('href', `/desktop-icons.svg#${state.icon}`);
  setUiLabels(trayEl, { title: label, label });
}

function desktopShowing() {
  return document.body.dataset.view === 'desktop' && !document.hidden;
}

async function fetchReachability() {
  try {
    const data = await apiFetch('/api/network');
    return data ? data.reachability : 'unknown';
  } catch {
    return 'unknown';
  }
}

async function refresh() {
  if (!desktopShowing()) return;
  const gen = loadGuard.start();
  const reachability = await fetchReachability();
  if (loadGuard.isCurrent(gen)) show(reachability);
}

let watcher = null;

export const DesktopNetwork = {
  init({ updates }) {
    watcher = updates;
    show('unknown');
    document.addEventListener('swing:langchange', () => show(current));
    updates.onChecked((ok) => {
      if (ok) {
        refresh();
        return;
      }
      loadGuard.start();
      show('unknown');
    });
  },
  /* The watcher skips every check while the interval is 「確認しない」, so showing the screen is the only chance to fetch then. */
  onShow() {
    if (watcher && !watcher.enabled()) refresh();
  },
};
