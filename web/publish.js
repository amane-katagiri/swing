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
  formatTime,
  clamp,
  maybeLink,
  setBusy,
  setFormDisabled,
  createLoadGuard,
  waitForNewInstance,
  sanitizeDisplayText,
  parseApiBody,
  apiResponseError,
} from './util.js';
import { appendLinksAndMessage, renderRelayResults, buildSiteNameRow } from './ui.js';
import { createPairing } from './pairing.js';

const publishEls = {
  status: document.getElementById('publish-status'),
  result: document.getElementById('publish-result'),
  form: document.getElementById('publish-form'),
  npub: document.getElementById('pub-npub'),
  hex: document.getElementById('pub-hex'),
  mirrorSet: document.getElementById('pub-mirror-set'),
  relays: document.getElementById('pub-relays'),
  signer: document.getElementById('pub-signer'),
  signerStatus: document.getElementById('pub-signer-status'),
  reconnect: document.getElementById('pub-signer-reconnect'),
  reconnectRelay: document.getElementById('pub-signer-relay'),
  reconnectSave: document.getElementById('pub-signer-save'),
  reconnectCancel: document.getElementById('pub-signer-cancel'),
  reconnectStatus: document.getElementById('pub-signer-save-status'),
  mySitesStatus: document.getElementById('my-sites-status'),
  mySitesContent: document.getElementById('my-sites-content'),
  uploadInput: document.getElementById('publish-upload-input'),
  uploadInfo: document.getElementById('publish-upload-info'),
  progress: document.getElementById('publish-progress'),
};

let publishing = false;
let reconnectPairing = null;
let reconnectSaving = false;
const publishLoadGuard = createLoadGuard();
const mySitesLoadGuard = createLoadGuard();

function readLastPublish() {
  const raw = storage.get('swing:publish:last', null);
  if (!raw) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

function saveLastPublish(data) {
  storage.set('swing:publish:last', JSON.stringify(data));
}

function computeRelativePath(file) {
  const rel = file.webkitRelativePath;
  if (!rel) return file.name;
  const idx = rel.indexOf('/');
  return idx === -1 ? rel : rel.slice(idx + 1);
}

function refreshSubmitState() {
  const submitBtn = publishEls.form.querySelector('button[type="submit"]');
  if (!submitBtn) return;
  if (publishing) {
    submitBtn.disabled = true;
    return;
  }
  const site = String(publishEls.form.elements.site.value || '').trim();
  if (!site) {
    submitBtn.disabled = true;
    return;
  }
  const files = Array.from(publishEls.uploadInput.files || []);
  if (files.length === 0) {
    submitBtn.disabled = true;
    return;
  }
  const total = files.reduce((sum, f) => sum + f.size, 0);
  const maxUpload = cache.overview ? cache.overview.max_upload : null;
  submitBtn.disabled = maxUpload != null && total > maxUpload;
}

export function updateUploadInfo() {
  const files = Array.from(publishEls.uploadInput.files || []);
  if (files.length === 0) {
    publishEls.uploadInfo.textContent = '';
    refreshSubmitState();
    return;
  }
  const total = files.reduce((sum, f) => sum + f.size, 0);
  const maxUpload = cache.overview ? cache.overview.max_upload : null;
  let text = t('uploadInfo', { n: files.length, size: formatBytes(total) });
  if (maxUpload != null && total > maxUpload) {
    text += t('uploadExceeds', { max: formatBytes(maxUpload) });
  }
  publishEls.uploadInfo.textContent = text;
  refreshSubmitState();
}

function showProgress(state, pct) {
  publishEls.progress.hidden = false;
  publishEls.progress.dataset.state = state;
  const bar = publishEls.progress.querySelector('.swing-progress-bar');
  if (state === 'uploading') bar.style.width = `${clamp(pct, 0, 100)}%`;
  else bar.style.width = '';
}

function hideProgress() {
  publishEls.progress.hidden = true;
  publishEls.progress.removeAttribute('data-state');
  publishEls.progress.querySelector('.swing-progress-bar').style.width = '';
}

function buildMySiteEntry(site) {
  const wrap = el('div', { class: 'swing-site' });
  wrap.append(buildSiteNameRow(site));
  wrap.append(el('div', { class: 'swing-site-meta' }, `${formatBytes(site.size)} · ${formatTime(site.created_at)}`));
  appendLinksAndMessage(wrap, site);
  wrap.append(
    el('div', { class: 'swing-account-actions' }, [
      el('button', { type: 'button', class: 'swing-btn swing-btn-small', onclick: () => useMySite(site) }, t('use')),
    ]),
  );
  return wrap;
}

function useMySite(site) {
  publishEls.form.elements.site.value = site.d;
  publishEls.form.elements.url.value = site.url || '';
  publishEls.form.elements.title.value = site.title || '';
  refreshSubmitState();
}

export function renderMySites() {
  const data = cache.publishSites;
  publishEls.mySitesContent.replaceChildren();
  if (!data) return;
  if (data.sites.length === 0) {
    publishEls.mySitesContent.append(el('p', { class: 'swing-status', 'data-kind': 'empty' }, t('noSitesPublishedYet')));
    return;
  }
  for (const site of data.sites) publishEls.mySitesContent.append(buildMySiteEntry(site));
}

async function loadMySites(force, reloadBtn) {
  if (!force && cache.publishSites) return renderMySites();
  const gen = mySitesLoadGuard.start();
  const firstLoad = !cache.publishSites;
  if (reloadBtn) setBusy(reloadBtn, true);
  if (firstLoad) setStatus(publishEls.mySitesStatus, 'loading', t('loadingYourSites'));
  let data;
  try {
    data = await apiFetch('/api/publish/sites');
  } catch (err) {
    if (!mySitesLoadGuard.isCurrent(gen)) return;
    cache.publishSites = null;
    setStatus(publishEls.mySitesStatus, 'error', describeError(err));
    publishEls.mySitesContent.replaceChildren();
    return;
  } finally {
    if (reloadBtn) setBusy(reloadBtn, false);
  }
  if (!mySitesLoadGuard.isCurrent(gen)) return;
  cache.publishSites = data;
  clearStatus(publishEls.mySitesStatus);
  renderMySites();
}

export async function loadOverview(force) {
  if (cache.overview && !force) return cache.overview;
  cache.overview = await apiFetch('/api/overview');
  return cache.overview;
}

export function updateNavFooter(overview) {
  const mirrorSet = document.getElementById('nav-mirror-set');
  mirrorSet.textContent = t('navFooterMirror', { name: overview.mirror_set });
  mirrorSet.title = mirrorSet.textContent;
  document.getElementById('nav-version').textContent = t('navFooterVersion', { version: overview.version });
  document.getElementById('page-footer').textContent = `${t('navFooterMirror', { name: overview.mirror_set })} · ${t('navFooterVersion', { version: overview.version })}`;
}

export function renderIdentity(overview) {
  publishEls.npub.textContent = overview.npub;
  publishEls.hex.textContent = overview.pubkey;
  publishEls.mirrorSet.textContent = overview.mirror_set;
  publishEls.relays.replaceChildren();
  for (const r of overview.relays) publishEls.relays.append(el('li', {}, r));
  renderSigner(overview.signer);
}

function renderSigner(signer) {
  const remote = !!(signer && signer.remote);
  const children = [el('span', {}, t(remote ? 'signerRemote' : 'signerLocal'))];
  if (remote) {
    children.push(
      el('button', { type: 'button', class: 'swing-btn swing-btn-small', onclick: openReconnect }, t('signerReconnectBtn')),
      el('span', { class: 'swing-hint' }, t('signerRemoteHint')),
    );
  }
  publishEls.signer.replaceChildren(...children);
  const failure = signer && signer.last_failure;
  if (failure) {
    setStatus(
      publishEls.signerStatus,
      'warn',
      `${t('signerLastFailure', { time: formatTime(failure.at), message: sanitizeDisplayText(failure.message) })} ${t('signerLastFailureHint')}`,
    );
  } else {
    clearStatus(publishEls.signerStatus);
  }
  if (!remote) publishEls.reconnect.hidden = true;
  if (reconnectPairing) reconnectPairing.render();
}

function refreshReconnectSave() {
  publishEls.reconnectSave.disabled = reconnectSaving || !reconnectPairing || !reconnectPairing.ready();
}

function openReconnect() {
  const signer = cache.overview && cache.overview.signer;
  if (!reconnectPairing) {
    reconnectPairing = createPairing(
      {
        relay: publishEls.reconnectRelay,
        start: document.getElementById('pub-signer-start'),
        status: document.getElementById('pub-signer-pairing-status'),
        qr: document.getElementById('pub-signer-qr'),
        qrImg: document.getElementById('pub-signer-qr-img'),
        uri: document.getElementById('pub-signer-uri'),
        readyKey: 'signerReconnectReady',
      },
      refreshReconnectSave,
    );
  }
  if (publishEls.reconnect.hidden && signer && signer.relays && signer.relays.length) {
    publishEls.reconnectRelay.value = signer.relays[0];
  }
  clearStatus(publishEls.reconnectStatus);
  publishEls.reconnect.hidden = false;
  refreshReconnectSave();
}

function closeReconnect() {
  if (reconnectPairing) reconnectPairing.reset();
  clearStatus(publishEls.reconnectStatus);
  publishEls.reconnect.hidden = true;
}

async function waitForRestart(previousInstance) {
  const restarted = await waitForNewInstance(previousInstance);
  if (restarted) location.reload();
  else setStatus(publishEls.reconnectStatus, 'error', t('setupTimedOut'));
}

async function saveReconnect() {
  if (reconnectSaving) return;
  reconnectSaving = true;
  refreshReconnectSave();
  setBusy(publishEls.reconnectSave, true);
  setStatus(publishEls.reconnectStatus, 'loading', t('signerReconnectSaving'));
  try {
    const previous = cache.overview ? cache.overview.instance : null;
    await apiFetch('/api/signer/reconnect', { method: 'POST' });
    reconnectPairing.stop();
    setStatus(publishEls.reconnectStatus, 'loading', t('setupRestarting'));
    await waitForRestart(previous);
  } catch (err) {
    setStatus(publishEls.reconnectStatus, 'error', describeError(err));
    reconnectSaving = false;
    setBusy(publishEls.reconnectSave, false);
    refreshReconnectSave();
  }
}

async function refreshSigner() {
  if (!usesSignerApp()) return;
  try {
    renderIdentity(await loadOverview(true));
  } catch {}
}

const NIP05_ERROR_KEYS = { unreachable: 'nip05ErrorUnreachable', timeout: 'nip05ErrorTimeout', invalid_response: 'nip05ErrorInvalidResponse' };

function describeNip05(nip05) {
  switch (nip05.status) {
    case 'verified':
      return t('nip05StatusVerified');
    case 'mismatch':
      return t('nip05StatusMismatch');
    case 'not_applicable':
      return t('nip05StatusNotApplicable');
    case 'off':
      return t('nip05StatusOff');
    case 'error':
      return t('nip05StatusError', { reason: NIP05_ERROR_KEYS[nip05.detail] ? t(NIP05_ERROR_KEYS[nip05.detail]) : nip05.detail || '' });
    default:
      return `${nip05.status}${nip05.detail ? ` — ${nip05.detail}` : ''}`;
  }
}

function describeDotfiles(dotfiles, published) {
  if (dotfiles.status === 'off') return t('checkOff');
  if (dotfiles.status === 'ok') return t('dotfilesNone');
  const more = dotfiles.count > dotfiles.paths.length ? t('dotfilesMore', { n: dotfiles.count - dotfiles.paths.length }) : '';
  const text = t('dotfilesFound', { count: dotfiles.count, paths: dotfiles.paths.join(', ') }) + more;
  return published ? text + t('dotfilesPublishedAnyway') : text;
}

function describeSizeCheck(size) {
  if (size.status === 'off') return t('checkOff');
  const params = { size: formatBytes(size.bytes), threshold: formatBytes(size.threshold) };
  return t(size.status === 'over' ? 'sizeOver' : 'sizeOk', params);
}

function describeUnchanged(unchanged, published) {
  switch (unchanged.status) {
    case 'off':
      return t('checkOff');
    case 'changed':
      return t('unchangedChanged');
    case 'unchanged':
      return t(published ? 'unchangedSamePublished' : 'unchangedSame');
    case 'no_previous':
      return t('unchangedNoPrevious');
    case 'unknown':
      return t('unchangedUnknown', { detail: unchanged.detail || '' });
    default:
      return unchanged.status;
  }
}

function describeCheckBlock(checks) {
  const reasons = [];
  if (checks.dotfiles && checks.dotfiles.mode === 'require' && checks.dotfiles.status === 'found') reasons.push(t('siteCheckDotfilesBlocked'));
  if (checks.size && checks.size.mode === 'require' && checks.size.status === 'over') reasons.push(t('siteCheckSizeBlocked'));
  return reasons.join(' ');
}

function addCheckRows(addRow, checks, published) {
  if (!checks) return;
  addRow(t('resultDotfiles'), describeDotfiles(checks.dotfiles, published));
  addRow(t('resultSizeCheck'), describeSizeCheck(checks.size));
  if (checks.unchanged) addRow(t('resultUnchanged'), describeUnchanged(checks.unchanged, published));
}

function usesSignerApp() {
  return !!(cache.overview && cache.overview.signer && cache.overview.signer.remote);
}

function renderPublishResult(result, errBody) {
  publishEls.result.hidden = false;
  publishEls.result.replaceChildren();
  const dl = el('dl', { class: 'swing-result-grid' });
  const addRow = (k, v) => dl.append(el('dt', {}, k), el('dd', {}, v));
  if (result) {
    const published = result.published !== false;
    if (!published) {
      setStatus(publishEls.status, 'ok', t('publishUnchanged', { site: result.site }));
    } else {
      const accepted = result.relays.filter((r) => r.ok).length;
      const counts = { site: result.site, ok: accepted, total: result.relays.length };
      if (accepted === result.relays.length) setStatus(publishEls.status, 'ok', t('publishDone', counts));
      else setStatus(publishEls.status, 'warn', t('publishDonePartial', counts));
    }
    publishEls.result.append(el('h2', {}, t('publishResultHeading')));
    addRow(t('resultSite'), result.site);
    if (result.url) addRow(t('resultUrl'), result.url);
    if (result.title) addRow(t('resultTitle'), result.title);
    addRow(t('resultNip05'), describeNip05(result.nip05));
    addCheckRows(addRow, result.checks, published);
    if (published && usesSignerApp()) addRow(t('signerLabel'), t('signerRemote'));
    addRow(t('resultCid'), result.cid);
    addRow(t('resultSize'), formatBytes(result.size));
    if (result.created_at != null) addRow(t('resultCreated'), formatTime(result.created_at));
    if (result.mfs_path) addRow(t('resultMfsPath'), result.mfs_path);
    if (result.files != null) addRow(t('resultFiles'), String(result.files));
    if (result.pruned && result.pruned.length) addRow(t('resultPruned'), result.pruned.join(', '));
    if (result.prune_error) addRow(t('resultPruneError'), result.prune_error);
    if (published) {
      const relayCell = el('dd', {});
      renderRelayResults(relayCell, result.relays);
      dl.append(el('dt', {}, t('resultRelays')), relayCell);
    }
    publishEls.result.append(dl);
    if (result.gateway_url) publishEls.result.append(el('p', {}, maybeLink(result.gateway_url, t('openGateway'))));
  } else if (errBody && errBody.nip05) {
    addRow(t('resultNip05'), describeNip05(errBody.nip05));
    addCheckRows(addRow, errBody.checks, false);
    publishEls.result.append(dl);
  }
}

function handlePublishHttpError(err) {
  const { status, body } = err;
  if (status === 413) {
    const maxUpload = cache.overview ? cache.overview.max_upload : null;
    setStatus(publishEls.status, 'error', t('uploadLimitError', { max: maxUpload != null ? formatBytes(maxUpload) : err.message }));
  } else if (status === 422 && body && body.checks) {
    setStatus(publishEls.status, 'error', t('siteCheckFailed', { detail: describeCheckBlock(body.checks) || describeError(err) }));
    renderPublishResult(null, body);
  } else if (status === 422 && body && body.nip05) {
    setStatus(publishEls.status, 'error', t('nip05CheckFailed', { detail: describeError(err) }));
    renderPublishResult(null, body);
  } else if (status === 409) {
    setStatus(publishEls.status, 'error', t('publishBusy'));
  } else {
    setStatus(publishEls.status, 'error', describeError(err));
  }
}

const MODE_FIELDS = ['nip05', 'check_dotfiles', 'check_size', 'check_unchanged'];

function buildUploadFormData({ site, url, title, message, modes, files }) {
  const fd = new FormData();
  fd.append('site', site);
  if (url) fd.append('url', url);
  if (title) fd.append('title', title);
  if (message) fd.append('message', message);
  for (const name of MODE_FIELDS) {
    if (modes[name]) fd.append(name, modes[name]);
  }
  for (const file of files) {
    fd.append('file', file, computeRelativePath(file));
  }
  return fd;
}

function submitUpload({ site, url, title, message, modes, files }) {
  const submitBtn = publishEls.form.querySelector('button[type="submit"]');
  return new Promise((resolve) => {
    publishing = true;
    setFormDisabled(publishEls.form, true);
    setBusy(submitBtn, true);
    publishEls.result.hidden = true;
    setStatus(publishEls.status, 'loading', t('uploading'));
    showProgress('uploading', 0);

    const xhr = new XMLHttpRequest();
    xhr.open('POST', '/api/publish/upload');
    xhr.setRequestHeader('X-Swing-Dashboard', '1');
    xhr.upload.addEventListener('progress', (ev) => {
      if (ev.lengthComputable) showProgress('uploading', (ev.loaded / ev.total) * 100);
    });
    xhr.upload.addEventListener('load', () => {
      showProgress('processing', 100);
      setStatus(publishEls.status, 'loading', t(usesSignerApp() ? 'processingOnAgentSigner' : 'processingOnAgent'));
    });
    xhr.addEventListener('error', () => {
      showProgress('error', 100);
      setStatus(publishEls.status, 'error', t('unreachable'));
      finishUpload();
      resolve();
    });
    xhr.addEventListener('load', () => {
      const body = parseApiBody(xhr.responseText);
      const err = apiResponseError('/api/publish/upload', xhr.status, body);
      if (!err) {
        hideProgress();
        renderPublishResult(body, null);
        if (body && body.published !== false) document.dispatchEvent(new CustomEvent('swing:published', { detail: body }));
        saveLastPublish({ site, url, title, message, ...modes });
        publishEls.uploadInput.value = '';
        updateUploadInfo();
        loadMySites(true);
      } else {
        showProgress('error', 100);
        handlePublishHttpError(err);
      }
      finishUpload();
      refreshSigner();
      resolve();
    });
    xhr.send(buildUploadFormData({ site, url, title, message, modes, files }));
  });

  function finishUpload() {
    publishing = false;
    setFormDisabled(publishEls.form, false);
    setBusy(submitBtn, false);
    refreshSubmitState();
  }
}

export const PublishView = {
  init() {
    publishEls.uploadInput.addEventListener('change', () => updateUploadInfo());
    publishEls.form.elements.site.addEventListener('input', () => refreshSubmitState());
    document.querySelector('[data-action="reload-my-sites"]').addEventListener('click', (ev) => loadMySites(true, ev.currentTarget));
    publishEls.reconnectSave.addEventListener('click', saveReconnect);
    publishEls.reconnectCancel.addEventListener('click', closeReconnect);

    const last = readLastPublish();
    if (last) {
      const form = publishEls.form;
      if (last.site) form.elements.site.value = last.site;
      if (last.url) form.elements.url.value = last.url;
      if (last.title) form.elements.title.value = last.title;
      if (last.message) form.elements.message.value = last.message;
      for (const name of MODE_FIELDS) {
        if (last[name]) form.elements[name].value = last[name];
      }
    }
    refreshSubmitState();

    publishEls.form.addEventListener('submit', (ev) => {
      ev.preventDefault();
      if (publishing) return;
      hideProgress();
      const fd = new FormData(publishEls.form);
      const site = String(fd.get('site') || '').trim();
      const url = String(fd.get('url') || '').trim();
      const title = String(fd.get('title') || '').trim();
      const message = String(fd.get('message') || '').trim();
      const modes = Object.fromEntries(MODE_FIELDS.map((name) => [name, String(fd.get(name) || '')]));

      const files = Array.from(publishEls.uploadInput.files || []);
      if (files.length === 0) {
        setStatus(publishEls.status, 'error', t('chooseFolderToUpload'));
        return;
      }
      submitUpload({ site, url, title, message, modes, files });
    });
  },
  onShow() {
    this.load(usesSignerApp());
    loadMySites();
  },
  async load(force) {
    const gen = publishLoadGuard.start();
    if (!cache.overview) setStatus(publishEls.status, 'loading', t('loadingGeneric'));
    try {
      const overview = await loadOverview(force);
      if (!publishLoadGuard.isCurrent(gen)) return;
      renderIdentity(overview);
      clearStatus(publishEls.status);
      updateNavFooter(overview);
      updateUploadInfo();
    } catch (err) {
      if (!publishLoadGuard.isCurrent(gen)) return;
      setStatus(publishEls.status, 'error', describeError(err));
    }
  },
};
