import { t } from './i18n.js';
import { apiFetch, el, formatBytes, formatTime, setStatus, clearStatus, describeError, createLoadGuard, dashboardAnswers, backoffDelay } from './util.js';

const HOUR = 3600;
const DAY = 86400;

const statsEls = {
  view: document.getElementById('view-settings'),
  status: document.getElementById('stats-status'),
  content: document.getElementById('stats-content'),
};

const statsLoadGuard = createLoadGuard();
let samples = [];
let intervalSecs = 60;
let kuboManaged = true;
let loaded = false;
let timer = null;
let failures = 0;
let unreachable = false;

const percent = (v) => `${v.toFixed(1)}%`;
const bytes = (v) => formatBytes(Math.round(v));
const rate = (v) => `${formatBytes(Math.round(v))}/s`;

const ROWS = [
  { key: 'statsSwingCpu', pick: (s) => s.swing?.cpu_percent, format: percent },
  { key: 'statsSwingMemory', pick: (s) => s.swing?.rss_bytes, format: bytes },
  { key: 'statsKuboCpu', pick: (s) => s.kubo?.cpu_percent, format: percent },
  { key: 'statsKuboMemory', pick: (s) => s.kubo?.rss_bytes, format: bytes },
  { key: 'statsIpfsIn', pick: (s) => s.traffic?.in_per_sec, format: rate },
  { key: 'statsIpfsOut', pick: (s) => s.traffic?.out_per_sec, format: rate },
];

function valuesSince(pick, since) {
  return samples.filter((s) => s.at > since).map(pick).filter((v) => v != null);
}

function cell(v, format) {
  return el('td', { class: 'swing-nowrap' }, v == null ? '–' : format(v));
}

export function renderStats() {
  if (!loaded) return;
  const latest = samples[samples.length - 1];
  clearStatus(statsEls.status);
  if (!latest) {
    statsEls.content.replaceChildren(el('p', { class: 'swing-hint' }, t('statsNoSamples')));
    return;
  }
  const hourAgo = latest.at - HOUR;
  const dayAgo = latest.at - DAY;
  const table = el('table', { class: 'swing-table' });
  const headers = ['', t('statsNow'), t('statsAvg1h'), t('statsMax1h'), t('statsMax24h')];
  table.append(el('thead', {}, el('tr', {}, headers.map((h) => el('th', {}, h)))));
  const tbody = el('tbody');
  for (const row of ROWS) {
    const hour = valuesSince(row.pick, hourAgo);
    const day = valuesSince(row.pick, dayAgo);
    const avg = hour.length ? hour.reduce((a, b) => a + b, 0) / hour.length : null;
    const max = (vs) => (vs.length ? Math.max(...vs) : null);
    tbody.append(
      el('tr', {}, [
        el('th', { scope: 'row' }, t(row.key)),
        cell(row.pick(latest), row.format),
        cell(avg, row.format),
        cell(max(hour), row.format),
        cell(max(day), row.format),
      ]),
    );
  }
  table.append(tbody);
  const notes = [el('p', { class: 'swing-hint' }, t('statsSampledAt', { time: formatTime(latest.at) }))];
  if (latest.traffic) {
    notes.push(el('p', {}, t('statsIpfsTotal', { in: formatBytes(latest.traffic.total_in), out: formatBytes(latest.traffic.total_out) })));
  }
  if (!kuboManaged) notes.push(el('p', { class: 'swing-hint' }, t('statsKuboUnmanaged')));
  statsEls.content.replaceChildren(el('div', { class: 'swing-table-scroll' }, table), ...notes);
}

function scheduleNext() {
  clearTimeout(timer);
  timer = setTimeout(() => {
    timer = null;
    if (!statsEls.view.hidden) loadStats();
  }, backoffDelay(intervalSecs * 1000, failures));
}

export async function loadStats() {
  const gen = statsLoadGuard.start();
  if (!loaded) setStatus(statsEls.status, 'loading', t('statsLoading'));
  const last = samples[samples.length - 1];
  if (unreachable && !(await dashboardAnswers())) {
    if (!statsLoadGuard.isCurrent(gen)) return;
    failures += 1;
    setStatus(statsEls.status, 'error', t('unreachable'));
    scheduleNext();
    return;
  }
  try {
    const data = await apiFetch(`/api/stats?since=${last ? last.at : 0}`);
    if (!statsLoadGuard.isCurrent(gen)) return;
    failures = 0;
    unreachable = false;
    intervalSecs = data.interval || intervalSecs;
    kuboManaged = data.kubo_managed;
    const newest = data.samples[data.samples.length - 1];
    samples = samples.concat(data.samples);
    if (newest) samples = samples.filter((s) => s.at > newest.at - DAY);
    loaded = true;
    renderStats();
  } catch (err) {
    if (!statsLoadGuard.isCurrent(gen)) return;
    failures += 1;
    unreachable = err.status === 0;
    setStatus(statsEls.status, 'error', describeError(err));
  }
  scheduleNext();
}
