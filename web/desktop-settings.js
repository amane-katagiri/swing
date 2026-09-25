import { createDialog } from './desktop-dialog.js';
import { WallpaperPage } from './desktop-wallpaper.js';

const PAGES = [WallpaperPage];

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

function activateTab(id) {
  for (const btn of els.tabButtons) {
    const active = btn.dataset.tab === id;
    btn.classList.toggle('is-active', active);
    btn.setAttribute('aria-selected', String(active));
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
  init() {
    for (const page of PAGES) page.init({ changed: updateApplyUI, dialog: els.dialog });
    activateTab(PAGES[0].id);
    for (const btn of els.tabButtons) btn.addEventListener('click', () => activateTab(btn.dataset.tab));
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
