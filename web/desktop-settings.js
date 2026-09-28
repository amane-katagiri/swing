import { createDialog } from './desktop-dialog.js';
import { WallpaperPage } from './desktop-wallpaper.js';
import { MascotSettingsPage } from './desktop-mascot-settings.js';
import { SystemSettingsPage } from './desktop-system-settings.js';

const PAGES = [WallpaperPage, MascotSettingsPage, SystemSettingsPage];

const dialogRoot = document.getElementById('desk-dialog-control-panel');

const els = {
  icon: document.getElementById('desk-icon-control-panel'),
  dialog: dialogRoot,
  applyBtn: dialogRoot.querySelector('[data-dialog-action="apply"]'),
  tabButtons: Array.from(dialogRoot.querySelectorAll('.desk-dialog-tab')),
  tabPanels: Array.from(dialogRoot.querySelectorAll('.desk-tabpanel')),
};

function updateApplyUI() {
  els.applyBtn.disabled = !PAGES.some((page) => page.isDirty());
}

const TAB_KEYS = { ArrowLeft: -1, ArrowRight: 1 };

function onTabKey(ev) {
  const i = els.tabButtons.indexOf(ev.currentTarget);
  const n = els.tabButtons.length;
  let next;
  if (ev.key in TAB_KEYS) next = (i + TAB_KEYS[ev.key] + n) % n;
  else if (ev.key === 'Home') next = 0;
  else if (ev.key === 'End') next = n - 1;
  else return;
  ev.preventDefault();
  activateTab(els.tabButtons[next].dataset.tab);
  els.tabButtons[next].focus();
}

function activateTab(id) {
  for (const btn of els.tabButtons) {
    const active = btn.dataset.tab === id;
    btn.classList.toggle('is-active', active);
    btn.setAttribute('aria-selected', String(active));
    btn.tabIndex = active ? 0 : -1;
  }
  for (const panel of els.tabPanels) {
    panel.hidden = panel.id !== `desk-tabpanel-${id}`;
  }
}

const dialog = createDialog({
  root: els.dialog,
  returnFocus: els.icon,
  onOpen() {
    for (const page of PAGES) page.open();
  },
  onOk() {
    return PAGES.filter((page) => page.isDirty()).every((page) => page.save());
  },
  onCancel() {
    for (const page of PAGES) page.discard();
  },
  onApply() {
    for (const page of PAGES.filter((page) => page.isDirty())) page.save();
  },
  onKey(ev) {
    for (const page of PAGES) {
      if (page.onKey && page.onKey(ev)) return true;
    }
    return false;
  },
});

export const DesktopSettings = {
  init({ updates }) {
    for (const page of PAGES) page.init({ changed: updateApplyUI, dialog: els.dialog, updates });
    activateTab(PAGES[0].id);
    for (const btn of els.tabButtons) {
      btn.addEventListener('click', () => activateTab(btn.dataset.tab));
      btn.addEventListener('keydown', onTabKey);
    }
  },
  open() {
    dialog.open();
  },
  boot() {
    for (const page of PAGES) {
      if (page.boot) page.boot();
    }
  },
};
