import { t, currentLang } from './i18n.js';
import { cache, el, apiFetch, setStatus, clearStatus, describeError, setBusy, setFormDisabled, createLoadGuard, pollUntil } from './util.js';
import { loadOverview } from './publish.js';
import { createPairing } from './pairing.js';

const setupEls = {
  status: document.getElementById('setup-status'),
  intro: document.getElementById('setup-intro'),
  form: document.getElementById('setup-form'),
  keyField: document.getElementById('setup-key-field'),
  keyChoiceHint: document.getElementById('setup-key-choice-hint'),
  signerField: document.getElementById('setup-signer-field'),
  formStatus: document.getElementById('setup-form-status'),
  result: document.getElementById('setup-result'),
};

const KEY_CHOICE_HINTS = { generate: 'setupKeyChoiceGenerate', existing: 'setupKeyChoiceExisting', signer: 'setupKeyChoiceSigner' };

const CONFIG_FIELDS = [
  { name: 'relays', path: 'nostr.relays', lockHintId: 'setup-relays-lock-hint', descHintId: 'setup-relays-desc-hint' },
  {
    name: 'maxTotalStorage',
    path: 'policy.max_total_storage',
    lockHintId: 'setup-max-total-storage-lock-hint',
    descHintId: 'setup-max-total-storage-desc-hint',
  },
  {
    name: 'maxPerSite',
    path: 'policy.max_per_site',
    lockHintId: 'setup-max-per-site-lock-hint',
    descHintId: 'setup-max-per-site-desc-hint',
  },
  {
    name: 'maxPerAccount',
    path: 'policy.max_per_account',
    lockHintId: 'setup-max-per-account-lock-hint',
    descHintId: 'setup-max-per-account-desc-hint',
  },
];

let lastConfig = null;
let lastResult = null;
let submitting = false;
let polling = false;
let pairing = null;
const lockedFields = new Set();

function renderIntro() {
  if (!lastConfig) return;
  setupEls.intro.textContent = t('setupIntro', { path: lastConfig.config_path || '' });
}

function configItem(config, path) {
  for (const section of config.sections) {
    for (const item of section.items) {
      if (`${section.name}.${item.key}` === path) return item;
    }
  }
  return null;
}

function formatFieldValue(raw) {
  if (Array.isArray(raw)) return raw.join('\n');
  return raw != null ? String(raw) : '';
}

function renderFieldDescriptions() {
  if (!lastConfig) return;
  const lang = currentLang();
  for (const field of CONFIG_FIELDS) {
    const descEl = document.getElementById(field.descHintId);
    if (!descEl) continue;
    const item = configItem(lastConfig, field.path);
    const description = item && item.description ? item.description[lang] || item.description.en : '';
    descEl.textContent = description || '';
  }
}

function prefillForm(config) {
  const form = setupEls.form;
  lockedFields.clear();
  for (const field of CONFIG_FIELDS) {
    const item = configItem(config, field.path);
    const locked = !!item && item.editable === false;
    const input = form.elements[field.name];
    input.value = item ? formatFieldValue(item.raw) : '';
    input.disabled = locked;
    const hintEl = document.getElementById(field.lockHintId);
    if (hintEl) {
      hintEl.hidden = !locked;
      hintEl.textContent = locked ? t('configLockedByEnv') : '';
    }
    if (locked) lockedFields.add(field.name);
  }
  renderFieldDescriptions();
}

function reapplyFieldLocks() {
  const form = setupEls.form;
  for (const field of CONFIG_FIELDS) {
    if (lockedFields.has(field.name)) form.elements[field.name].disabled = true;
  }
}

function updateKeyFieldVisibility() {
  const choice = setupEls.form.elements.keyChoice.value;
  setupEls.keyField.hidden = choice !== 'existing';
  setupEls.signerField.hidden = choice !== 'signer';
  setupEls.keyChoiceHint.textContent = t(KEY_CHOICE_HINTS[choice]);
}

function collectRelays() {
  return String(setupEls.form.elements.relays.value || '')
    .split('\n')
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

function renderSuccess(result) {
  lastResult = result;
  setupEls.form.hidden = true;
  setupEls.result.hidden = false;
  setupEls.result.replaceChildren(
    el('p', {}, t('setupSuccessIntro')),
    el('p', {}, [el('strong', {}, 'npub'), document.createTextNode(': '), el('code', {}, result.npub)]),
    el('p', { class: 'swing-hint' }, t(result.remoteSigner ? 'setupSignerStoredHint' : 'setupSecretStoredHint')),
  );
}

async function pollUntilReady() {
  if (polling) return;
  polling = true;
  try {
    const ready = await pollUntil(async () => {
      const overview = await apiFetch('/api/overview');
      cache.overview = overview;
      return !overview.setup;
    });
    if (ready) {
      clearStatus(setupEls.formStatus);
      location.hash = '#/settings';
    } else {
      setStatus(setupEls.formStatus, 'error', t('setupTimedOut'));
    }
  } finally {
    polling = false;
  }
}

async function handleSubmit(ev) {
  ev.preventDefault();
  if (submitting) return;
  const form = setupEls.form;
  const keyChoice = form.elements.keyChoice.value;
  const secretKey = String(form.elements.secretKey.value || '').trim();
  if (keyChoice === 'existing' && !secretKey) {
    setStatus(setupEls.formStatus, 'error', t('setupKeyRequired'));
    form.elements.secretKey.focus();
    return;
  }
  if (keyChoice === 'signer' && !pairing.ready()) {
    setStatus(setupEls.formStatus, 'error', t('setupSignerNotReady'));
    document.getElementById('setup-signer-start').focus();
    return;
  }
  const items = {};
  if (!lockedFields.has('relays')) items['nostr.relays'] = collectRelays();
  if (!lockedFields.has('maxTotalStorage')) items['policy.max_total_storage'] = String(form.elements.maxTotalStorage.value || '').trim();
  if (!lockedFields.has('maxPerSite')) items['policy.max_per_site'] = String(form.elements.maxPerSite.value || '').trim();
  if (!lockedFields.has('maxPerAccount')) items['policy.max_per_account'] = String(form.elements.maxPerAccount.value || '').trim();
  const body = {
    secret_key: keyChoice === 'existing' ? secretKey : null,
    remote_signer: keyChoice === 'signer',
    items,
  };
  const submitBtn = form.querySelector('button[type="submit"]');
  submitting = true;
  setFormDisabled(form, true);
  setBusy(submitBtn, true);
  setStatus(setupEls.formStatus, 'loading', t('setupSubmitting'));
  try {
    const result = await apiFetch('/api/setup', { method: 'POST', body: JSON.stringify(body) });
    form.elements.secretKey.value = '';
    pairing.stop();
    renderSuccess({ ...result, remoteSigner: body.remote_signer });
    setStatus(setupEls.formStatus, 'loading', t('setupRestarting'));
    pollUntilReady();
  } catch (err) {
    setStatus(setupEls.formStatus, 'error', describeError(err));
    setFormDisabled(form, false);
    reapplyFieldLocks();
    setBusy(submitBtn, false);
    submitting = false;
  }
}

const setupLoadGuard = createLoadGuard();

export const SetupView = {
  init() {
    setupEls.form.querySelectorAll('input[name="keyChoice"]').forEach((radio) => {
      radio.addEventListener('change', updateKeyFieldVisibility);
    });
    updateKeyFieldVisibility();
    pairing = createPairing({
      relay: document.getElementById('setup-signer-relay'),
      start: document.getElementById('setup-signer-start'),
      status: document.getElementById('setup-signer-status'),
      qr: document.getElementById('setup-signer-qr'),
      qrImg: document.getElementById('setup-signer-qr-img'),
      uri: document.getElementById('setup-signer-uri'),
    });
    setupEls.form.addEventListener('submit', handleSubmit);
  },
  onShow() {
    this.load();
  },
  async load() {
    const gen = setupLoadGuard.start();
    setupEls.form.hidden = true;
    setupEls.result.hidden = true;
    setStatus(setupEls.status, 'loading', t('loadingGeneric'));
    try {
      const overview = await loadOverview();
      if (!setupLoadGuard.isCurrent(gen)) return;
      if (!overview.setup) {
        location.hash = '#/sites';
        return;
      }
      const config = await apiFetch('/api/config');
      if (!setupLoadGuard.isCurrent(gen)) return;
      cache.config = config;
      lastConfig = config;
      prefillForm(config);
      renderIntro();
      clearStatus(setupEls.status);
      setupEls.form.hidden = false;
    } catch (err) {
      if (!setupLoadGuard.isCurrent(gen)) return;
      setStatus(setupEls.status, 'error', describeError(err));
    }
  },
  render() {
    renderIntro();
    renderFieldDescriptions();
    updateKeyFieldVisibility();
    if (pairing) pairing.render();
    if (lastResult) renderSuccess(lastResult);
  },
};
