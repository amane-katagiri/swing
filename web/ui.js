import { el, apiFetch, setBusy, copyWithFeedback, describeError, maybeLink, sanitizeMessage } from './util.js';
import { t } from './i18n.js';

export function copyButton(text, ariaLabel) {
  const attrs = {
    type: 'button',
    class: 'swing-btn swing-copy-btn',
    onclick: (ev) => copyWithFeedback(ev.currentTarget, text),
  };
  if (ariaLabel) attrs['aria-label'] = ariaLabel;
  return el('button', attrs, t('copy'));
}

export function storedBadge(site) {
  return el('span', { class: 'swing-badge', 'data-stored': String(!!site.stored) }, site.stored ? t('stored') : t('notStored'));
}

export function buildSiteNameRow(site) {
  const title = sanitizeMessage(site.title);
  return el('div', { class: 'swing-site-row' }, [
    el('span', { class: 'swing-site-name' }, site.d),
    title ? el('span', { class: 'swing-hint' }, title) : null,
  ]);
}

export function appendLinksAndMessage(wrap, site) {
  const links = el('div', { class: 'swing-site-links' }, [
    site.url ? maybeLink(site.url, t('openSite')) : null,
    site.gateway_url ? maybeLink(site.gateway_url, t('openGateway')) : null,
  ]);
  if (links.childNodes.length) wrap.append(links);

  const msg = sanitizeMessage(site.message);
  if (msg) wrap.append(el('p', { class: 'swing-site-message' }, `“${msg}”`));
}

export function renderRelayResults(container, relays) {
  const wrap = el('div', { class: 'swing-relay-results' });
  for (const r of relays) {
    wrap.append(
      el('div', { class: 'swing-relay-result', 'data-ok': String(r.ok) }, [
        el('span', { class: 'swing-relay-mark' }, r.ok ? '✓' : '✗'),
        el('span', {}, r.relay),
        r.error ? el('span', { class: 'swing-hint' }, r.error) : null,
      ]),
    );
  }
  container.append(wrap);
}

export function renderMirrorOpResult(container, result, kind) {
  container.hidden = false;
  container.replaceChildren();
  if (result.changed.length === 0) {
    container.append(el('p', {}, kind === 'add' ? t('mirrorAddNoChange') : t('mirrorRemoveNoChange')));
  } else {
    const list = result.changed.map((a) => a.npub).join(', ');
    container.append(el('p', {}, t(kind === 'add' ? 'mirrorAdded' : 'mirrorRemoved', { n: result.changed.length, list })));
  }
  if (result.unchanged.length > 0) {
    container.append(el('p', { class: 'swing-hint' }, t('mirrorUnchanged', { list: result.unchanged.map((a) => a.npub).join(', ') })));
  }
  if (result.published) renderRelayResults(container, result.relays);
}

export function renderOpError(container, err) {
  container.hidden = false;
  container.replaceChildren(el('p', { class: 'swing-status', 'data-kind': 'error' }, describeError(err)));
}

export function buildRemoveControl(acct, onDone, opts) {
  const small = !opts || opts.size !== 'normal';
  const btnClass = small ? 'swing-btn swing-btn-small' : 'swing-btn';
  const containerClass = opts && opts.inline ? 'swing-inline-actions' : 'swing-account-actions';
  const container = el('span', { class: containerClass });
  function draw(confirming) {
    container.replaceChildren();
    if (!confirming) {
      container.append(el('button', { type: 'button', class: btnClass, onclick: () => draw(true) }, t('removeFromMirror')));
    } else {
      const confirmBtn = el('button', { type: 'button', class: `${btnClass} swing-btn-danger`, onclick: () => doRemove(confirmBtn, cancelBtn) }, t('confirm'));
      const cancelBtn = el('button', { type: 'button', class: btnClass, onclick: () => draw(false) }, t('cancel'));
      container.append(el('span', { class: 'swing-hint' }, t('removeFromMirrorConfirm')), confirmBtn, cancelBtn);
    }
  }
  async function doRemove(confirmBtn, cancelBtn) {
    setBusy(confirmBtn, true);
    cancelBtn.disabled = true;
    try {
      const result = await apiFetch('/api/mirror/remove', { method: 'POST', body: JSON.stringify({ keys: [acct.pubkey] }) });
      onDone(result);
      draw(false);
    } catch (err) {
      onDone(null, err);
      draw(false);
    }
  }
  draw(false);
  return container;
}
