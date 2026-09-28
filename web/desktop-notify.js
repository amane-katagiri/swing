import { t, currentLang } from './i18n.js';
import { storage } from './storage.js';
import { isHttpUrl, sanitizeDisplayText, sanitizeMessage } from './util.js';
import { isNoticeEvent } from './desktop-updates.js';
import { SHOW_SELF_IN_WEBRING, browserNotifyReady, readNotifySettings } from './notify-settings.js';
import { startupView } from './desktop-system-settings.js';

export const NOTIFIED_KEY = 'swing:desktop:notified';
const TITLE_MAX = 60;
const TEXT_KEYS = {
  stored: ['notifySiteStored', 'notifySitesStored'],
  published: ['notifySitePublished', 'notifySitesPublished'],
  replica: ['notifyReplicaAdded', 'notifyReplicasAdded'],
};

function siteTitle(site) {
  return sanitizeMessage(site.title, TITLE_MAX) || sanitizeDisplayText(site.d, TITLE_MAX);
}

function readNotified() {
  try {
    const parsed = JSON.parse(storage.get(NOTIFIED_KEY, 'null'));
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

/* Unacknowledged notices are announced again after a reload so the mascot can still bring them up; this keeps the browser from showing them twice. */
function takeUnnotified(notices) {
  const notified = readNotified();
  const fresh = notices.filter((n) => !Number.isFinite(notified[n.kind]) || n.at > notified[n.kind]);
  if (fresh.length === 0) return fresh;
  for (const n of fresh) {
    if (!Number.isFinite(notified[n.kind]) || n.at > notified[n.kind]) notified[n.kind] = n.at;
  }
  storage.set(NOTIFIED_KEY, JSON.stringify(notified));
  return fresh;
}

function followClick(notices, byMascot) {
  if (byMascot) {
    location.hash = '#/desktop';
    return;
  }
  const newest = notices[notices.length - 1];
  if (newest.kind === 'published') {
    location.hash = '#/publish';
  } else if (newest.kind === 'replica') {
    document.dispatchEvent(new CustomEvent(SHOW_SELF_IN_WEBRING));
  } else if (notices.length === 1 && isHttpUrl(newest.href)) {
    window.open(newest.href, '_blank', 'noopener,noreferrer');
  } else {
    location.hash = `#/${startupView()}`;
  }
}

export function createBrowserNotifier({ updates, mascotsShowing }) {
  function open(notices, byMascot) {
    const newest = notices[notices.length - 1];
    const [one, many] = TEXT_KEYS[newest.kind];
    const body = notices.length === 1 ? t(one, { title: siteTitle(newest.site) }) : t(many, { count: notices.length });
    let n;
    try {
      n = new Notification(t('notifyTitle'), { body, tag: `swing:${newest.kind}:${newest.at}`, icon: '/apple-touch-icon.png', lang: currentLang() });
    } catch {
      return;
    }
    n.addEventListener('click', () => {
      n.close();
      window.focus();
      followClick(notices, byMascot);
    });
  }

  updates.subscribe((event) => {
    if (!isNoticeEvent(event) || mascotsShowing()) return;
    const settings = readNotifySettings();
    if (!browserNotifyReady(settings)) return;
    const notices = takeUnnotified(event.notices.filter((n) => settings.browser[n.kind] && !updates.isAcknowledged(n)));
    if (notices.length > 0) open(notices, settings.mascot[notices[0].kind]);
  });
}
