import { applyStaticI18n, t } from './i18n.js';
import { storage } from './storage.js';
import { cache, copyWithFeedback, getStyle } from './util.js';
import { DesktopView } from './desktop.js';
import { SitesView, renderStatusCheck } from './sites.js';
import { WebringView, renderWebringIfLoaded } from './webring.js';
import { PublishView, renderIdentity, updateNavFooter, updateUploadInfo, renderMySites, loadOverview } from './publish.js';
import { SettingsView, renderConfig } from './settings.js';

const VIEWS = { desktop: DesktopView, sites: SitesView, webring: WebringView, publish: PublishView, settings: SettingsView };

function currentRoute() {
  const hash = location.hash.replace(/^#\/?/, '');
  return Object.prototype.hasOwnProperty.call(VIEWS, hash) ? hash : 'sites';
}

function showRoute() {
  const name = currentRoute();
  document.body.dataset.view = name;
  document.querySelectorAll('.swing-nav-list a[data-route]').forEach((a) => {
    if (a.dataset.route === name) a.setAttribute('aria-current', 'page');
    else a.removeAttribute('aria-current');
  });
  for (const key of Object.keys(VIEWS)) {
    document.getElementById(`view-${key}`).hidden = key !== name;
  }
  const style = getStyle(name);
  if (style) document.body.dataset.style = style;
  else delete document.body.dataset.style;
  VIEWS[name].onShow();
}

function navCollapsed() {
  return storage.get('swing:nav:collapsed', '0') === '1';
}

function applyNavState() {
  const collapsed = navCollapsed();
  if (collapsed) document.body.dataset.nav = 'collapsed';
  else delete document.body.dataset.nav;
  const toggle = document.getElementById('nav-toggle');
  const label = t(collapsed ? 'navExpand' : 'navCollapse');
  toggle.setAttribute('aria-expanded', String(!collapsed));
  toggle.setAttribute('aria-label', label);
  toggle.title = label;
  document.querySelectorAll('.swing-nav-list a[data-route]').forEach((a) => {
    if (collapsed) a.title = a.textContent.trim();
    else a.removeAttribute('title');
  });
}

function wireNavToggle() {
  document.getElementById('nav-toggle').addEventListener('click', () => {
    storage.set('swing:nav:collapsed', navCollapsed() ? '0' : '1');
    applyNavState();
  });
}

function wireReloadButtons() {
  document.querySelector('[data-action="reload-sites"]').addEventListener('click', (ev) => SitesView.load(true, ev.currentTarget));
}

function wireCopyButtons() {
  document.querySelectorAll('[data-copy-target]').forEach((btn) => {
    btn.addEventListener('click', (ev) => copyWithFeedback(ev.currentTarget, document.getElementById(btn.dataset.copyTarget).textContent));
  });
}

function applyLanguage() {
  applyStaticI18n();
  applyNavState();
  if (cache.overview) {
    renderIdentity(cache.overview);
    updateNavFooter(cache.overview);
  }
  updateUploadInfo();
  if (cache.sites) SitesView.render();
  renderWebringIfLoaded();
  if (cache.publishSites) renderMySites();
  if (cache.config) renderConfig(cache.config);
  if (cache.status) renderStatusCheck(cache.status);
}

function init() {
  applyStaticI18n();
  delete document.documentElement.dataset.i18nPending;
  applyNavState();
  wireNavToggle();
  DesktopView.init();
  SitesView.init();
  WebringView.init();
  PublishView.init();
  SettingsView.init();
  wireReloadButtons();
  wireCopyButtons();
  document.addEventListener('swing:langchange', applyLanguage);
  window.addEventListener('hashchange', showRoute);
  showRoute();
  loadOverview()
    .then(updateNavFooter)
    .catch(() => {});
}

init();
