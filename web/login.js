import { t } from './i18n.js';
import { apiFetch, setStatus, clearStatus, describeError, setBusy, setFormDisabled } from './util.js';

const loginEls = {
  form: document.getElementById('login-form'),
  status: document.getElementById('login-status'),
};

let submitting = false;
let statusKey = null;
let linkedCode = false;

function showStatus(kind, key) {
  statusKey = key;
  setStatus(loginEls.status, kind, t(key));
}

async function handleSubmit(ev) {
  ev.preventDefault();
  if (submitting) return;
  linkedCode = false;
  const form = loginEls.form;
  const code = String(form.elements.code.value || '').trim();
  if (!code) {
    form.elements.code.focus();
    return;
  }
  const submitBtn = form.querySelector('button[type="submit"]');
  submitting = true;
  setFormDisabled(form, true);
  setBusy(submitBtn, true);
  showStatus('loading', 'loginSubmitting');
  try {
    await apiFetch('/api/login', { method: 'POST', body: JSON.stringify({ code }) });
    location.replace('/');
  } catch (err) {
    if (err.status === 401) {
      showStatus('error', 'loginInvalid');
    } else {
      statusKey = null;
      setStatus(loginEls.status, 'error', describeError(err));
    }
    setFormDisabled(form, false);
    setBusy(submitBtn, false);
    submitting = false;
  }
}

export const LoginView = {
  init() {
    loginEls.form.addEventListener('submit', handleSubmit);
    loginEls.form.elements.code.addEventListener('input', () => {
      linkedCode = false;
    });
    const linked = location.hash.match(/^#\/?login\/code\/([0-9a-fA-F]+)$/);
    if (linked) {
      history.replaceState(null, '', '#/login');
      loginEls.form.elements.code.value = linked[1];
      linkedCode = true;
    }
  },
  onShow() {
    if (location.hash.replace(/^#\/?/, '') === 'login/invalid') {
      showStatus('error', 'loginInvalid');
    } else if (linkedCode) {
      showStatus('ok', 'loginLinkReady');
    } else if (!submitting) {
      statusKey = null;
      clearStatus(loginEls.status);
    }
    if (linkedCode) {
      loginEls.form.querySelector('button[type="submit"]').focus();
    } else {
      loginEls.form.elements.code.focus();
    }
  },
  render() {
    if (statusKey) setStatus(loginEls.status, loginEls.status.dataset.kind, t(statusKey));
  },
};
