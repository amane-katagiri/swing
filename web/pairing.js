import { t } from './i18n.js';
import { apiFetch, setStatus, clearStatus, describeError, setBusy, createLoadGuard, ensureBusyStructure, sleep, sanitizeDisplayText } from './util.js';

const PAIRING_POLL_MS = 1500;

export function createPairing(els, onChange) {
  let pairing = null;
  const guard = createLoadGuard();

  function ready() {
    return !!(pairing && pairing.status && pairing.status.state === 'ready');
  }

  function render() {
    ensureBusyStructure(els.start);
    els.start.querySelector('.swing-btn-label').textContent = t(pairing ? 'setupSignerRestart' : 'setupSignerStart');
    els.qrImg.alt = t('setupSignerQrAlt');
    if (!pairing) return;
    const status = pairing.status;
    els.qr.hidden = !status || status.state === 'failed' || status.state === 'ready';
    if (!status) {
      setStatus(els.status, 'loading', t('setupSignerStarting'));
    } else if (status.state === 'waiting' || status.state === 'idle') {
      setStatus(els.status, 'loading', t('setupSignerWaiting'));
    } else if (status.state === 'checking') {
      setStatus(els.status, 'loading', t('setupSignerChecking', { npub: status.npub }));
    } else if (status.state === 'ready' && status.probe_signed) {
      setStatus(els.status, 'ok', t(els.readyKey || 'setupSignerReady', { npub: status.npub }));
    } else if (status.state === 'ready') {
      setStatus(els.status, 'warn', t('setupSignerProbeFailed', { npub: status.npub, detail: sanitizeDisplayText(status.error) }));
    } else {
      setStatus(els.status, 'error', t('setupSignerFailed', { detail: sanitizeDisplayText(status.error) }));
    }
  }

  function update(status) {
    pairing.status = status;
    render();
    if (onChange) onChange();
  }

  async function poll(gen) {
    while (guard.isCurrent(gen)) {
      await sleep(PAIRING_POLL_MS);
      if (!guard.isCurrent(gen)) return;
      try {
        const status = await apiFetch('/api/setup/signer');
        if (!guard.isCurrent(gen)) return;
        update(status);
        if (status.state === 'ready' || status.state === 'failed') return;
      } catch (err) {
        if (!guard.isCurrent(gen)) return;
        update({ state: 'failed', error: describeError(err) });
        return;
      }
    }
  }

  async function start() {
    const relay = String(els.relay.value || '').trim();
    if (!relay) {
      setStatus(els.status, 'error', t('setupSignerRelayRequired'));
      els.relay.focus();
      return;
    }
    const gen = guard.start();
    pairing = { status: null };
    els.qr.hidden = true;
    render();
    if (onChange) onChange();
    setBusy(els.start, true);
    try {
      const started = await apiFetch('/api/setup/signer', {
        method: 'POST',
        body: JSON.stringify({ relays: [relay] }),
      });
      if (!guard.isCurrent(gen)) return;
      els.qrImg.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(started.qr_svg)}`;
      els.uri.textContent = started.uri;
      update({ state: 'waiting' });
      poll(gen);
    } catch (err) {
      if (!guard.isCurrent(gen)) return;
      update({ state: 'failed', error: describeError(err) });
    } finally {
      setBusy(els.start, false);
    }
  }

  function reset() {
    guard.start();
    pairing = null;
    els.qr.hidden = true;
    clearStatus(els.status);
    render();
    if (onChange) onChange();
  }

  els.start.addEventListener('click', start);
  render();
  return { render, ready, reset, stop: () => guard.start() };
}
