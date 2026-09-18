import { createWebringGraph } from './graph.js';

const DEFAULT_STYLES = { sites: 'list', webring: 'graph' };
const MAX_MIRROR_KEYS = 100;

const MESSAGES = {
  en: {
    unknownError: 'Unknown error.',
    unreachable: 'Could not reach the agent.',
    copy: 'Copy',
    reload: 'Reload',
    openSite: 'Open site',
    openGateway: 'Open via gateway',

    navSites: 'Sites',
    navPublish: 'Publish',
    navSettings: 'Settings',
    navFooterMirror: 'mirror set: {name}',
    navFooterVersion: 'v{version}',

    sitesHeading: 'Sites',
    addToMirrorHeading: 'Add to mirror',
    addToMirrorPlaceholder: 'npub1…, hex, or nprofile1… (space or comma separated)',
    add: 'Add',
    styleList: 'List',
    styleCards: 'Cards',
    styleTable: 'Table',
    filterPlaceholder: 'Filter by name, domain, cid…',
    storedOnly: 'Stored only',
    sortUpdated: 'Updated',
    sortName: 'Name',
    sortPubkey: 'Pubkey',
    sortGroupLabel: 'Sort order',
    unfollowedHeading: 'Unfollowed but still stored',
    unfollowedRemoveNote: 'These will be removed the next time the mirror list is refreshed.',
    unfollowedKeepNote: 'remove_on_unfollow is disabled, so these are kept until removed manually.',
    storageCheckHeading: 'Storage check',
    storageCheckHint: "Compares every stored version against Kubo's MFS. Not run automatically — it can take a while.",
    runStorageCheck: 'Run storage check',
    checkingStorage: 'Checking storage… this can take a while.',
    loadingSites: 'Loading sites…',
    noMirrorFollowSet: 'No mirror follow set found yet.',
    replicasUnavailable: 'Replica counts unavailable: {reason}',
    notFollowingAnyone: 'Not following anyone yet.',
    noSitesMatchFilter: 'No sites match the filter.',
    noSitesPublishedYet: 'No sites published yet.',
    stored: 'stored',
    notStored: 'not stored',
    nip05Badge: 'nip05: {status}',
    replicasBadge: 'replicas: {n}',
    copyNpub: 'Copy npub',
    copied: 'Copied',
    copyFailed: 'Copy failed',
    mirrorKeysRequired: 'Enter an npub, hex, or nprofile.',
    removeFromMirror: 'Remove from mirror',
    removeFromMirrorConfirm: 'Remove from mirror?',
    confirm: 'Confirm',
    cancel: 'Cancel',
    removing: 'Removing…',
    adding: 'Adding…',
    mirrorAdded: 'Added {n}: {list}',
    mirrorRemoved: 'Removed {n}: {list}',
    mirrorAddNoChange: 'No change (already followed).',
    mirrorRemoveNoChange: 'No change (not followed).',
    mirrorUnchanged: 'Unchanged: {list}',
    tooManyKeys: 'Too many keys: {n} (max {max}).',
    tableSite: 'Site',
    tableStored: 'Stored',
    tableReplicas: 'Replicas',
    tableSize: 'Size',
    tableUpdated: 'Updated',
    tableLinks: 'Links',
    tableAccount: 'Account',
    tablePath: 'Path',
    tableCreated: 'Created',
    tableHealth: 'Health',
    noProblemsFound: 'No problems found.',
    problemsFound: '{n} problem(s) found.',
    notInState: 'Not in state',
    listFailedSuffix: ' [list failed]',

    rootLabel: 'Root',
    rootPlaceholder: 'self',
    depthLabel: 'Depth',
    update: 'Update',
    styleGraph: 'Graph',
    styleAscii: 'ASCII',
    styleSource: 'Source',
    loadingWebring: 'Loading webring…',
    accountsHeading: 'Accounts ({n})',
    mutualHeading: 'Mutual ({n})',
    onewayHeading: 'One-way ({n})',
    none: 'None.',
    depthPrefix: 'depth={n}',
    tagRoot: '[root]',
    tagNoFollowSet: '[no follow set]',
    graphvizTitle: 'Graphviz (dot)',
    mermaidTitle: 'Mermaid',
    loadingReplicas: 'Loading replicas…',
    detailNames: 'names',
    detailDepth: 'depth',
    noPublishedSitesForNode: 'No published sites.',
    replicasReportsBadge: 'replicas: {replicas} (reports: {reports})',
    tagLatest: '[latest]',
    tagOlderVersion: '[older version]',
    tagAuthor: ' [author]',
    tagNotFollowing: ' [not following]',
    useAsRoot: 'Use as root',
    addToMirror: 'Add to mirror',
    mirroredBadge: 'Mirrored',
    noAccountsForRootDepth: 'No accounts found for this root and depth.',
    beyondHint: '{beyond} more account(s) beyond depth {depth} not shown.',
    legendRoot: 'root',
    legendMutual: 'mutual',
    legendOneway: 'one-way (from → to)',
    legendNoFollowSet: 'no follow set',
    graphFit: 'Fit to view',
    graphAriaLabel: 'Webring graph',
    graphEmpty: 'No nodes to show for this root and depth.',

    publishHeading: 'Publish',
    yourIdentityHeading: 'Your identity',
    mirrorSetLabel: 'Mirror set',
    relaysLabel: 'Relays',
    mySitesHeading: 'My sites',
    loadingYourSites: 'Loading your sites…',
    publishFormHeading: 'Publish a site',
    folderLabel: 'Folder',
    siteLabel: 'Site (d tag)',
    urlLabel: 'URL (optional)',
    messageLabel: 'Message (optional)',
    nip05Default: 'Use configured default',
    publishBtn: 'Publish',
    use: 'Use these settings',
    loadingGeneric: 'Loading…',
    uploadInfo: '{n} file(s), {size} total.',
    uploadExceeds: ' Exceeds the upload limit ({max}).',
    uploadLimitError: 'Upload exceeds the limit ({max}).',
    nip05CheckFailed: 'NIP-05 check failed: {detail}',
    publishBusy: 'Another publish is already running on this agent. Try again shortly.',
    publishing: 'Publishing… this can take a while.',
    uploading: 'Uploading… this can take a while.',
    processingOnAgent: 'Processing on the agent… this can take a while.',
    chooseFolderToUpload: 'Choose a folder to upload.',
    resultSite: 'Site',
    resultUrl: 'URL',
    resultNip05: 'NIP-05',
    resultCid: 'CID',
    resultSize: 'Size',
    resultCreated: 'Created',
    resultMfsPath: 'MFS path',
    resultFiles: 'Files',
    resultPruned: 'Pruned',
    resultPruneError: 'Prune error',

    settingsHeading: 'Settings',
    settingsReadonlyHint: "Configuration is read-only here. Change swing.toml or the environment on the agent's host, then restart the agent.",
    loadingConfig: 'Loading configuration…',
    configFile: 'Config file: {path}',
    configEnvOnly: 'Running from environment variables only (no config file).',
    tableKey: 'Key',
    tableValue: 'Value',
    tableEnv: 'Env',
    displayHeading: 'Display',
    themeLabel: 'Theme',
    themeAuto: 'Match system',
    themeLight: 'Light',
    themeDark: 'Dark',
    langLabel: 'Language',
    langAuto: 'Auto',
    langEn: 'English',
    langJa: '日本語',
    customCssLabel: 'Custom CSS',
    apply: 'Apply',
    reset: 'Reset',
  },
  ja: {
    unknownError: '不明なエラー。',
    unreachable: 'エージェントに接続できない。',
    copy: 'コピー',
    reload: '再読み込み',
    openSite: 'サイトを開く',
    openGateway: 'ゲートウェイで開く',

    navSites: 'サイト',
    navPublish: '公開',
    navSettings: '設定',
    navFooterMirror: 'ミラーセット: {name}',
    navFooterVersion: 'v{version}',

    sitesHeading: 'サイト',
    addToMirrorHeading: 'ミラーに追加',
    addToMirrorPlaceholder: 'npub1…、hex、またはnprofile1…（スペースかカンマ区切り）',
    add: '追加',
    styleList: 'リスト',
    styleCards: 'カード',
    styleTable: '表',
    filterPlaceholder: '名前・ドメイン・cidで絞り込み…',
    storedOnly: '保存済みのみ',
    sortUpdated: '更新順',
    sortName: '名前順',
    sortPubkey: 'pubkey順',
    sortGroupLabel: '並び順',
    unfollowedHeading: 'フォロー解除済みだが保存中',
    unfollowedRemoveNote: '次にミラーリストを更新するときに削除される。',
    unfollowedKeepNote: 'remove_on_unfollowが無効なので、手動で削除するまで残る。',
    storageCheckHeading: 'ストレージチェック',
    storageCheckHint: 'すべての保存済みバージョンをKuboのMFSと照合する。自動実行はしない（時間がかかることがある）。',
    runStorageCheck: 'ストレージチェックを実行',
    checkingStorage: 'ストレージを確認中…時間がかかることがある。',
    loadingSites: 'サイトを読み込み中…',
    noMirrorFollowSet: 'ミラーのフォローセットがまだ見つからない。',
    replicasUnavailable: 'レプリカ数を取得できない: {reason}',
    notFollowingAnyone: 'まだ誰もフォローしていない。',
    noSitesMatchFilter: '絞り込みに一致するサイトがない。',
    noSitesPublishedYet: 'まだ公開したサイトがない。',
    stored: '保存済み',
    notStored: '未保存',
    nip05Badge: 'nip05: {status}',
    replicasBadge: 'レプリカ: {n}',
    copyNpub: 'npubをコピー',
    copied: 'コピー済み',
    copyFailed: 'コピー失敗',
    mirrorKeysRequired: 'npub・hex・nprofileのいずれかを入力する。',
    removeFromMirror: 'ミラーから削除',
    removeFromMirrorConfirm: 'ミラーから削除？',
    confirm: '確定',
    cancel: 'キャンセル',
    removing: '削除中…',
    adding: '追加中…',
    mirrorAdded: '{n}件追加した: {list}',
    mirrorRemoved: '{n}件削除した: {list}',
    mirrorAddNoChange: '変更なし（すでにフォロー済み）。',
    mirrorRemoveNoChange: '変更なし（フォローしていない）。',
    mirrorUnchanged: '変更なし: {list}',
    tooManyKeys: 'キーが多すぎる: {n}件（上限{max}件）。',
    tableSite: 'サイト',
    tableStored: '保存',
    tableReplicas: 'レプリカ',
    tableSize: 'サイズ',
    tableUpdated: '更新',
    tableLinks: 'リンク',
    tableAccount: 'アカウント',
    tablePath: 'パス',
    tableCreated: '作成',
    tableHealth: '状態',
    noProblemsFound: '問題は見つからなかった。',
    problemsFound: '{n}件の問題が見つかった。',
    notInState: 'stateに存在しない',
    listFailedSuffix: ' [一覧取得失敗]',

    rootLabel: 'ルート',
    rootPlaceholder: '自分',
    depthLabel: '深さ',
    update: '更新',
    styleGraph: 'グラフ',
    styleAscii: 'ASCII',
    styleSource: 'ソース',
    loadingWebring: 'Webringを読み込み中…',
    accountsHeading: 'アカウント（{n}）',
    mutualHeading: '相互（{n}）',
    onewayHeading: '一方向（{n}）',
    none: 'なし。',
    depthPrefix: '深さ={n}',
    tagRoot: '[ルート]',
    tagNoFollowSet: '[フォローセットなし]',
    graphvizTitle: 'Graphviz (dot)',
    mermaidTitle: 'Mermaid',
    loadingReplicas: 'レプリカを読み込み中…',
    detailNames: '名前',
    detailDepth: '深さ',
    noPublishedSitesForNode: '公開したサイトがない。',
    replicasReportsBadge: 'レプリカ: {replicas}（報告: {reports}）',
    tagLatest: '[最新]',
    tagOlderVersion: '[旧バージョン]',
    tagAuthor: ' [作者]',
    tagNotFollowing: ' [未フォロー]',
    useAsRoot: 'ルートにする',
    addToMirror: 'ミラーに追加',
    mirroredBadge: 'ミラー済み',
    noAccountsForRootDepth: 'このルートと深さではアカウントが見つからない。',
    beyondHint: 'さらに{beyond}件のアカウントが深さ{depth}の先にある（非表示）。',
    legendRoot: 'ルート',
    legendMutual: '相互',
    legendOneway: '一方向（元→先）',
    legendNoFollowSet: 'フォローセットなし',
    graphFit: '全体表示',
    graphAriaLabel: 'Webringグラフ',
    graphEmpty: 'このルートと深さで表示するノードがない。',

    publishHeading: '公開',
    yourIdentityHeading: '自分の情報',
    mirrorSetLabel: 'ミラーセット',
    relaysLabel: 'リレー',
    mySitesHeading: '自分のサイト',
    loadingYourSites: '自分のサイトを読み込み中…',
    publishFormHeading: 'サイトを公開',
    folderLabel: 'フォルダ',
    siteLabel: 'サイト（dタグ）',
    urlLabel: 'URL（任意）',
    messageLabel: 'メッセージ（任意）',
    nip05Default: '設定済みの既定値を使う',
    publishBtn: '公開',
    use: 'このサイト設定を使う',
    loadingGeneric: '読み込み中…',
    uploadInfo: '{n}件のファイル、合計{size}。',
    uploadExceeds: ' アップロード上限（{max}）を超えている。',
    uploadLimitError: 'アップロードが上限（{max}）を超えている。',
    nip05CheckFailed: 'NIP-05チェックに失敗: {detail}',
    publishBusy: '別の公開処理が実行中。しばらくしてから試す。',
    publishing: '公開中…時間がかかることがある。',
    uploading: 'アップロード中…時間がかかることがある。',
    processingOnAgent: 'エージェント側で処理中…時間がかかることがある。',
    chooseFolderToUpload: 'アップロードするフォルダを選ぶ。',
    resultSite: 'サイト',
    resultUrl: 'URL',
    resultNip05: 'NIP-05',
    resultCid: 'CID',
    resultSize: 'サイズ',
    resultCreated: '作成日時',
    resultMfsPath: 'MFSパス',
    resultFiles: 'ファイル数',
    resultPruned: '削除済み',
    resultPruneError: '削除エラー',

    settingsHeading: '設定',
    settingsReadonlyHint: '設定はここでは読み取り専用。swing.tomlかエージェントホストの環境変数を変更してエージェントを再起動する。',
    loadingConfig: '設定を読み込み中…',
    configFile: '設定ファイル: {path}',
    configEnvOnly: '環境変数のみで動作中（設定ファイルなし）。',
    tableKey: 'キー',
    tableValue: '値',
    tableEnv: '環境変数',
    displayHeading: '表示',
    themeLabel: 'テーマ',
    themeAuto: 'システムに合わせる',
    themeLight: 'ライト',
    themeDark: 'ダーク',
    langLabel: '言語',
    langAuto: '自動',
    langEn: 'English',
    langJa: '日本語',
    customCssLabel: 'カスタムCSS',
    apply: '適用',
    reset: 'リセット',
  },
};

const cache = {
  overview: null,
  sites: null,
  status: null,
  mirror: null,
  webringByQuery: new Map(),
  replicasByKey: new Map(),
  config: null,
  publishSites: null,
};

const storage = {
  get(key, fallback) {
    try {
      const v = localStorage.getItem(key);
      return v == null ? fallback : v;
    } catch {
      return fallback;
    }
  },
  set(key, value) {
    try {
      localStorage.setItem(key, value);
    } catch {}
  },
  remove(key) {
    try {
      localStorage.removeItem(key);
    } catch {}
  },
};

function currentLang() {
  const pref = storage.get('swing:lang', 'auto');
  if (pref === 'en' || pref === 'ja') return pref;
  return (navigator.language || '').toLowerCase().startsWith('ja') ? 'ja' : 'en';
}

function t(key, vars) {
  const lang = currentLang();
  const dict = MESSAGES[lang] || MESSAGES.en;
  let str = Object.prototype.hasOwnProperty.call(dict, key) ? dict[key] : MESSAGES.en[key];
  if (str == null) return key;
  if (vars) {
    for (const [k, v] of Object.entries(vars)) str = str.replace(new RegExp(`\\{${k}\\}`, 'g'), v);
  }
  return str;
}

function el(tag, attrs, children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') node.className = v;
    else if (k.startsWith('on') && typeof v === 'function') node.addEventListener(k.slice(2), v);
    else if (v === true) node.setAttribute(k, '');
    else node.setAttribute(k, v);
  }
  for (const c of [].concat(children == null ? [] : children)) {
    if (c == null) continue;
    node.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return node;
}

function clamp(v, lo, hi) {
  return Math.min(hi, Math.max(lo, v));
}

function setStatus(container, kind, message) {
  container.dataset.kind = kind;
  container.textContent = message || '';
}

function clearStatus(container) {
  container.removeAttribute('data-kind');
  container.textContent = '';
}

function describeError(err) {
  if (!err) return t('unknownError');
  if (err.status === 0) return err.message || t('unreachable');
  if (err.status) return `${err.message} (HTTP ${err.status})`;
  return err.message || String(err);
}

function formatBytes(n) {
  if (n == null) return '–';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${i === 0 ? v : v.toFixed(1)} ${units[i]}`;
}

function formatTime(sec) {
  if (sec == null) return '–';
  const lang = currentLang();
  try {
    return new Intl.DateTimeFormat(lang === 'ja' ? 'ja-JP' : 'en-US', {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(sec * 1000));
  } catch {
    return new Date(sec * 1000).toLocaleString();
  }
}

function stripControlChars(str) {
  return String(str).replace(/[\x00-\x1f\x7f]+/g, ' ').trim();
}

function sanitizeMessage(str, max) {
  if (!str) return null;
  const limit = max || 200;
  const cleaned = stripControlChars(str);
  if (!cleaned) return null;
  return cleaned.length > limit ? `${cleaned.slice(0, limit)}…` : cleaned;
}

function shortenMiddle(str, head, tail) {
  const h = head || 10;
  const t = tail || 6;
  if (!str || str.length <= h + t + 1) return str || '';
  return `${str.slice(0, h)}…${str.slice(-t)}`;
}

function maybeLink(url, text) {
  if (typeof url === 'string' && /^https?:\/\//i.test(url)) {
    return el('a', { href: url, target: '_blank', rel: 'noopener noreferrer' }, text || url);
  }
  return el('span', {}, text || url || '');
}

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text || '');
  } catch {}
}

async function copyWithFeedback(button, text) {
  ensureBusyStructure(button);
  const label = button.querySelector('.swing-btn-label');
  const original = label.textContent;
  clearTimeout(button._swingCopyTimer);
  let ok = true;
  try {
    await navigator.clipboard.writeText(text || '');
  } catch {
    ok = false;
  }
  label.textContent = ok ? t('copied') : t('copyFailed');
  button.dataset.copied = String(ok);
  button.dataset.copyFailed = String(!ok);
  button._swingCopyTimer = setTimeout(() => {
    label.textContent = original;
    delete button.dataset.copied;
    delete button.dataset.copyFailed;
  }, 1500);
}

function ensureBusyStructure(button) {
  if (button.querySelector('.swing-btn-label')) return;
  const label = el('span', { class: 'swing-btn-label' });
  while (button.firstChild) label.append(button.firstChild);
  const spinner = el('span', { class: 'swing-btn-spinner', 'aria-hidden': 'true' });
  button.append(label, spinner);
}

function setBusy(button, busy) {
  if (!button) return;
  ensureBusyStructure(button);
  button.disabled = busy;
  if (busy) button.setAttribute('aria-busy', 'true');
  else button.removeAttribute('aria-busy');
  button.classList.toggle('is-busy', busy);
}

async function apiFetch(path, opts) {
  const options = opts || {};
  const method = (options.method || 'GET').toUpperCase();
  const headers = Object.assign({}, options.headers);
  if (method !== 'GET' && method !== 'HEAD') {
    headers['X-Swing-Dashboard'] = '1';
    headers['Content-Type'] = 'application/json';
  }
  let res;
  try {
    res = await fetch(path, Object.assign({}, options, { headers }));
  } catch {
    const err = new Error(t('unreachable'));
    err.status = 0;
    throw err;
  }
  const text = await res.text();
  let body = null;
  if (text) {
    try {
      body = JSON.parse(text);
    } catch {
      body = null;
    }
  }
  if (!res.ok) {
    const message = body && typeof body.error === 'string' ? body.error : `HTTP ${res.status}`;
    const err = new Error(message);
    err.status = res.status;
    err.body = body;
    throw err;
  }
  return body;
}

function getStyle(view) {
  if (!(view in DEFAULT_STYLES)) return null;
  return storage.get(`swing:style:${view}`, DEFAULT_STYLES[view]);
}

function setStyle(view, value) {
  storage.set(`swing:style:${view}`, value);
}

function wireStyleSwitch(view, onChange) {
  const group = document.querySelector(`.swing-style-switch[data-target="${view}"]`);
  const current = getStyle(view);
  const buttons = group.querySelectorAll('button');
  for (const btn of buttons) {
    btn.setAttribute('aria-pressed', String(btn.dataset.styleValue === current));
    btn.addEventListener('click', () => {
      setStyle(view, btn.dataset.styleValue);
      for (const b of buttons) b.setAttribute('aria-pressed', String(b === btn));
      if (document.body.dataset.view === view) document.body.dataset.style = btn.dataset.styleValue;
      onChange(btn.dataset.styleValue);
    });
  }
}

function wireSortSwitch(selector, storageKey, defaultValue, onChange) {
  const group = document.querySelector(selector);
  const current = storage.get(storageKey, defaultValue);
  const buttons = group.querySelectorAll('button');
  for (const btn of buttons) {
    btn.setAttribute('aria-pressed', String(btn.dataset.sortValue === current));
    btn.addEventListener('click', () => {
      storage.set(storageKey, btn.dataset.sortValue);
      for (const b of buttons) b.setAttribute('aria-pressed', String(b === btn));
      onChange(btn.dataset.sortValue);
    });
  }
}

function setFormDisabled(form, disabled) {
  for (const field of form.elements) field.disabled = disabled;
}

const SITE_FIELD_DEFAULTS = { url: null, message: null, nip05: null, replicas: null, stored: null, gateway_url: null };

function normalizeSite(site, contextDefaults) {
  return Object.assign({}, SITE_FIELD_DEFAULTS, contextDefaults || {}, site);
}

function matchesFilter(site, acct, filterVal, storedOnly) {
  if (storedOnly && !site.stored) return false;
  if (!filterVal) return true;
  const hay = [site.d, site.message, site.cid, site.url, acct.npub, acct.pubkey]
    .filter(Boolean)
    .join(' ')
    .toLowerCase();
  return hay.includes(filterVal);
}

function sortAccounts(accounts, mode) {
  if (mode === 'pubkey') return accounts;
  const bySite = mode === 'name'
    ? (a, b) => a.d.localeCompare(b.d)
    : (a, b) => (b.created_at || 0) - (a.created_at || 0);
  const withSites = [];
  const withoutSites = [];
  for (const acct of accounts) {
    if (acct.sites.length === 0) withoutSites.push(acct);
    else withSites.push(Object.assign({}, acct, { sites: [...acct.sites].sort(bySite) }));
  }
  withSites.sort((a, b) => bySite(a.sites[0], b.sites[0]));
  return [...withSites, ...withoutSites];
}

function buildSiteEntry(site) {
  const wrap = el('div', { class: 'swing-site', 'data-stored': String(!!site.stored) });
  const row = el('div', { class: 'swing-site-row' }, [
    el('span', { class: 'swing-site-name' }, site.d),
    el('span', { class: 'swing-badge', 'data-stored': String(!!site.stored) }, site.stored ? t('stored') : t('notStored')),
    site.nip05 ? el('span', { class: 'swing-badge', 'data-nip05': site.nip05 }, t('nip05Badge', { status: site.nip05 })) : null,
    el('span', { class: 'swing-badge' }, t('replicasBadge', { n: site.replicas == null ? '–' : site.replicas })),
  ]);
  wrap.append(row);

  const meta = el('div', { class: 'swing-site-meta' }, [
    el('div', { class: 'swing-site-meta-cid' }, [
      el('span', { class: 'swing-copyable' }, [
        document.createTextNode(`cid: ${shortenMiddle(site.cid, 10, 6)}`),
        el('button', { type: 'button', class: 'swing-btn swing-copy-btn', onclick: (ev) => copyWithFeedback(ev.currentTarget, site.cid) }, t('copy')),
      ]),
    ]),
    el('div', { class: 'swing-site-meta-info' }, `${formatBytes(site.size)} · ${formatTime(site.created_at)}`),
  ]);
  wrap.append(meta);

  const links = el('div', { class: 'swing-site-links' }, [
    site.url ? maybeLink(site.url, t('openSite')) : null,
    site.gateway_url ? maybeLink(site.gateway_url, t('openGateway')) : null,
  ]);
  if (links.childNodes.length) wrap.append(links);

  const msg = sanitizeMessage(site.message);
  if (msg) wrap.append(el('p', { class: 'swing-site-message' }, `“${msg}”`));

  return wrap;
}

function buildSiteTable(sites) {
  const table = el('table', { class: 'swing-table' });
  const headers = [t('tableSite'), t('tableStored'), 'NIP-05', t('tableReplicas'), 'CID', t('tableSize'), t('tableUpdated'), t('tableLinks')];
  table.append(el('thead', {}, el('tr', {}, headers.map((h) => el('th', {}, h)))));
  const tbody = el('tbody');
  for (const site of sites) {
    const tr = el('tr', { 'data-stored': String(!!site.stored) });
    tr.append(el('td', {}, site.d));
    tr.append(el('td', {}, el('span', { class: 'swing-badge', 'data-stored': String(!!site.stored) }, site.stored ? t('stored') : t('notStored'))));
    tr.append(el('td', {}, site.nip05 ? el('span', { class: 'swing-badge', 'data-nip05': site.nip05 }, site.nip05) : '–'));
    tr.append(el('td', {}, site.replicas == null ? '–' : String(site.replicas)));
    tr.append(
      el('td', { class: 'swing-mono' }, [
        document.createTextNode(`${shortenMiddle(site.cid, 8, 6)} `),
        el('button', { type: 'button', class: 'swing-btn swing-copy-btn', onclick: (ev) => copyWithFeedback(ev.currentTarget, site.cid) }, t('copy')),
      ]),
    );
    tr.append(el('td', {}, formatBytes(site.size)));
    tr.append(el('td', {}, formatTime(site.created_at)));
    const linksTd = el('td', {});
    if (site.url) linksTd.append(maybeLink(site.url, t('openSite')));
    if (site.url && site.gateway_url) linksTd.append(document.createTextNode(' '));
    if (site.gateway_url) linksTd.append(maybeLink(site.gateway_url, t('openGateway')));
    if (!site.url && !site.gateway_url) linksTd.append(document.createTextNode('–'));
    tr.append(linksTd);
    tbody.append(tr);
  }
  table.append(tbody);
  return table;
}

function renderRelayResults(container, relays) {
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

function renderMirrorOpResult(container, result, kind) {
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

function renderOpError(container, err) {
  container.hidden = false;
  container.replaceChildren(el('p', { class: 'swing-status', 'data-kind': 'error' }, describeError(err)));
}

function buildRemoveControl(acct, onDone, opts) {
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

function buildAccountElement(acct, opts) {
  const filterVal = opts.filterVal;
  const storedOnly = opts.storedOnly;
  const filtering = Boolean(filterVal) || storedOnly;
  const sites = acct.sites.map((s) => normalizeSite(s, opts.siteDefaults)).filter((s) => matchesFilter(s, acct, filterVal, storedOnly));
  if (filtering && sites.length === 0) return null;

  const head = el('div', { class: 'swing-account-head' }, [
    el('span', { class: 'swing-account-key' }, shortenMiddle(acct.npub, 14, 6)),
    el('button', { type: 'button', class: 'swing-btn swing-copy-btn', 'aria-label': t('copyNpub'), onclick: (ev) => copyWithFeedback(ev.currentTarget, acct.npub) }, t('copy')),
  ]);
  if (opts.removable) {
    head.append(buildRemoveControl(acct, (result, err) => {
      if (err) renderOpError(opts.resultEl, err);
      else {
        renderMirrorOpResult(opts.resultEl, result, 'remove');
        opts.onChanged();
      }
    }));
  }

  const wrap = el('div', { class: 'swing-account' }, head);

  if (sites.length === 0) {
    wrap.append(el('p', { class: 'swing-account-empty' }, filtering ? t('noSitesMatchFilter') : t('noSitesPublishedYet')));
  } else {
    const set = el('div', { class: 'swing-site-set' });
    if (getStyle('sites') === 'table') {
      set.append(buildSiteTable(sites));
    } else {
      for (const s of sites) set.append(buildSiteEntry(s));
    }
    wrap.append(set);
  }
  return wrap;
}

const sitesEls = {
  status: document.getElementById('sites-status'),
  content: document.getElementById('sites-content'),
  unfollowedSection: document.getElementById('sites-unfollowed'),
  unfollowedNote: document.getElementById('unfollowed-note'),
  unfollowedContent: document.getElementById('unfollowed-content'),
  filterText: document.getElementById('sites-filter-text'),
  filterStored: document.getElementById('sites-filter-stored'),
  mirrorAddForm: document.getElementById('mirror-add-form'),
  mirrorAddResult: document.getElementById('mirror-add-result'),
  runStatusBtn: document.getElementById('run-status-check'),
  statusCheckResult: document.getElementById('status-check-result'),
};

function renderStatusCheck(status) {
  sitesEls.statusCheckResult.replaceChildren();
  sitesEls.statusCheckResult.append(el('p', {}, status.problems === 0 ? t('noProblemsFound') : t('problemsFound', { n: status.problems })));
  if (status.versions.length) {
    const table = el('table', { class: 'swing-table' });
    const headers = [t('tableAccount'), t('tableSite'), t('tablePath'), 'CID', t('tableSize'), t('tableCreated'), t('tableHealth')];
    table.append(el('thead', {}, el('tr', {}, headers.map((h) => el('th', {}, h)))));
    const tbody = el('tbody');
    for (const v of status.versions) {
      const tr = el('tr', { 'data-health': v.health });
      tr.append(el('td', {}, v.npub ? shortenMiddle(v.npub, 10, 4) : '–'));
      tr.append(el('td', {}, v.d || '–'));
      tr.append(el('td', { class: 'swing-mono' }, v.path || '–'));
      tr.append(el('td', { class: 'swing-mono' }, v.cid ? shortenMiddle(v.cid, 8, 6) : '–'));
      tr.append(el('td', {}, formatBytes(v.size)));
      tr.append(el('td', {}, formatTime(v.created_at)));
      tr.append(
        el('td', {}, [
          el('span', { class: 'swing-badge', 'data-health': v.health }, v.health),
          v.detail ? el('span', { class: 'swing-hint' }, ` ${stripControlChars(v.detail)}`) : null,
        ]),
      );
      tbody.append(tr);
    }
    table.append(tbody);
    sitesEls.statusCheckResult.append(table);
  }
  if (status.garbage.length) {
    sitesEls.statusCheckResult.append(el('h3', {}, t('notInState')));
    const list = el('ul', { class: 'swing-plain-list' });
    for (const g of status.garbage) list.append(el('li', {}, `${g.path}${g.list_failed ? t('listFailedSuffix') : ''}`));
    sitesEls.statusCheckResult.append(list);
  }
}

let sitesLoadGen = 0;
let statusCheckGen = 0;

const SitesView = {
  init() {
    wireStyleSwitch('sites', () => this.render());
    sitesEls.filterText.addEventListener('input', () => this.render());
    sitesEls.filterStored.checked = storage.get('swing:sites:stored-only', '0') === '1';
    sitesEls.filterStored.addEventListener('change', () => {
      storage.set('swing:sites:stored-only', sitesEls.filterStored.checked ? '1' : '0');
      this.render();
    });
    wireSortSwitch('.swing-sort-switch', 'swing:sites:sort', 'updated', () => this.render());
    const mirrorAddBtn = sitesEls.mirrorAddForm.querySelector('button[type="submit"]');
    const mirrorKeysInput = sitesEls.mirrorAddForm.elements.keys;
    function refreshAddState() {
      mirrorAddBtn.disabled = !mirrorKeysInput.value.trim();
    }
    mirrorKeysInput.addEventListener('input', refreshAddState);
    refreshAddState();
    sitesEls.mirrorAddForm.addEventListener('submit', async (ev) => {
      ev.preventDefault();
      const input = mirrorKeysInput;
      const keys = input.value.split(/[\s,]+/).map((s) => s.trim()).filter(Boolean);
      if (keys.length === 0) {
        input.focus();
        renderOpError(sitesEls.mirrorAddResult, { message: t('mirrorKeysRequired') });
        return;
      }
      if (keys.length > MAX_MIRROR_KEYS) {
        renderOpError(sitesEls.mirrorAddResult, { message: t('tooManyKeys', { n: keys.length, max: MAX_MIRROR_KEYS }) });
        return;
      }
      setBusy(mirrorAddBtn, true);
      try {
        const result = await apiFetch('/api/mirror/add', { method: 'POST', body: JSON.stringify({ keys }) });
        renderMirrorOpResult(sitesEls.mirrorAddResult, result, 'add');
        input.value = '';
        refreshAddState();
        cache.mirror = null;
        await this.load(true);
      } catch (err) {
        renderOpError(sitesEls.mirrorAddResult, err);
      } finally {
        setBusy(mirrorAddBtn, false);
        refreshAddState();
      }
    });
    sitesEls.runStatusBtn.addEventListener('click', async () => {
      const gen = ++statusCheckGen;
      const firstRun = sitesEls.statusCheckResult.childElementCount === 0;
      setBusy(sitesEls.runStatusBtn, true);
      if (firstRun) {
        sitesEls.statusCheckResult.replaceChildren(el('p', { class: 'swing-status', 'data-kind': 'loading' }, t('checkingStorage')));
      }
      try {
        const data = await apiFetch('/api/status');
        if (gen !== statusCheckGen) return;
        cache.status = data;
        renderStatusCheck(cache.status);
      } catch (err) {
        if (gen !== statusCheckGen) return;
        sitesEls.statusCheckResult.replaceChildren(el('p', { class: 'swing-status', 'data-kind': 'error' }, describeError(err)));
      } finally {
        if (gen === statusCheckGen) setBusy(sitesEls.runStatusBtn, false);
      }
    });
  },
  onShow() {
    if (cache.sites) this.render();
    else this.load();
  },
  async load(force, reloadBtn) {
    if (!force && cache.sites) return this.render();
    const gen = ++sitesLoadGen;
    const firstLoad = !cache.sites;
    if (reloadBtn) setBusy(reloadBtn, true);
    if (firstLoad) setStatus(sitesEls.status, 'loading', t('loadingSites'));
    let data;
    try {
      data = await apiFetch('/api/sites');
    } catch (err) {
      if (gen !== sitesLoadGen) return;
      cache.sites = null;
      setStatus(sitesEls.status, 'error', describeError(err));
      return;
    } finally {
      if (reloadBtn) setBusy(reloadBtn, false);
    }
    if (gen !== sitesLoadGen) return;
    cache.sites = data;
    cache.mirror = null;
    this.render();
  },
  render() {
    const data = cache.sites;
    if (!data) return;
    document.body.dataset.style = getStyle('sites');

    if (data.replicas_error) {
      setStatus(sitesEls.status, 'error', t('replicasUnavailable', { reason: data.replicas_error }));
    } else if (data.follow_set && data.follow_set.note) {
      setStatus(sitesEls.status, 'empty', data.follow_set.note);
    } else if (!data.follow_set || !data.follow_set.found) {
      setStatus(sitesEls.status, 'empty', t('noMirrorFollowSet'));
    } else {
      clearStatus(sitesEls.status);
    }

    const filterVal = sitesEls.filterText.value.trim().toLowerCase();
    const storedOnly = sitesEls.filterStored.checked;
    const sortMode = storage.get('swing:sites:sort', 'updated');

    sitesEls.content.replaceChildren();
    const accountEls = sortAccounts(data.accounts, sortMode)
      .map((a) => buildAccountElement(a, {
        removable: true,
        filterVal,
        storedOnly,
        resultEl: sitesEls.mirrorAddResult,
        onChanged: () => this.load(true),
      }))
      .filter(Boolean);
    if (accountEls.length === 0) {
      sitesEls.content.append(
        el('p', { class: 'swing-status', 'data-kind': 'empty' }, data.accounts.length === 0 ? t('notFollowingAnyone') : t('noSitesMatchFilter')),
      );
    } else {
      for (const e of accountEls) sitesEls.content.append(e);
    }

    const unfollowed = data.unfollowed;
    if (unfollowed && unfollowed.accounts.length > 0) {
      sitesEls.unfollowedSection.hidden = false;
      sitesEls.unfollowedNote.textContent = unfollowed.remove_on_unfollow ? t('unfollowedRemoveNote') : t('unfollowedKeepNote');
      sitesEls.unfollowedContent.replaceChildren();
      for (const a of sortAccounts(unfollowed.accounts, sortMode)) {
        const elm = buildAccountElement(a, { removable: false, filterVal, storedOnly, siteDefaults: { stored: true } });
        if (elm) sitesEls.unfollowedContent.append(elm);
      }
    } else {
      sitesEls.unfollowedSection.hidden = true;
    }
  },
};

const webringEls = {
  status: document.getElementById('webring-status'),
  content: document.getElementById('webring-content'),
  detail: document.getElementById('webring-detail'),
  form: document.getElementById('webring-query-form'),
  layout: document.getElementById('webring-layout'),
};

function setDetailVisible(visible) {
  webringEls.detail.hidden = !visible;
  webringEls.layout.dataset.detail = String(visible);
}

function saveWebringQuery(rootVal, depthVal) {
  storage.set('swing:webring:query', JSON.stringify({ root: rootVal, depth: depthVal }));
}

function loadSavedWebringQuery() {
  const raw = storage.get('swing:webring:query', null);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object') return null;
    const root = typeof parsed.root === 'string' ? parsed.root : '';
    const depth = clamp(parseInt(parsed.depth, 10) || 0, 0, 4);
    return { root, depth };
  } catch {
    return null;
  }
}

function setWebringQuery(rootVal, depthVal) {
  currentWebringQuery = { roots: rootVal ? rootVal.split(/[\s,]+/).filter(Boolean) : [], depth: depthVal };
  saveWebringQuery(rootVal, depthVal);
}

let currentWebringQuery = { roots: [], depth: 2 };
let graphInstance = null;
let selectedNodePubkey = null;
let webringLoadGen = 0;
let selectNodeGen = 0;

function webringQueryKey(q) {
  const params = new URLSearchParams();
  for (const r of q.roots) params.append('root', r);
  params.set('depth', String(q.depth));
  return params.toString();
}

function labelOf(map, pk) {
  const n = map.get(pk);
  return n ? n.label : shortenMiddle(pk, 8, 4);
}

function graphLabels() {
  return {
    root: t('legendRoot'),
    mutual: t('legendMutual'),
    oneway: t('legendOneway'),
    noFollowSet: t('legendNoFollowSet'),
    fit: t('graphFit'),
    ariaLabel: t('graphAriaLabel'),
    empty: t('graphEmpty'),
  };
}

function renderGraphStyle(data) {
  graphInstance = createWebringGraph(webringEls.content, {
    nodes: data.nodes,
    edges: data.edges,
    selectedPubkey: selectedNodePubkey,
    onSelect: (node) => selectNode(node.pubkey),
    labels: graphLabels(),
  });
}

function renderListStyle(data) {
  const wrap = el('div', { class: 'swing-webring-list' });
  const nodeByKey = new Map(data.nodes.map((n) => [n.pubkey, n]));

  const accounts = el('div', { class: 'swing-webring-group' }, el('h3', {}, t('accountsHeading', { n: data.nodes.length })));
  const sorted = [...data.nodes].sort((a, b) => a.depth - b.depth || a.label.localeCompare(b.label));
  for (const n of sorted) {
    accounts.append(
      el('div', { class: 'swing-webring-account-row' }, [
        el('button', { type: 'button', onclick: () => selectNode(n.pubkey) }, n.label),
        el('span', { class: 'swing-hint' }, ` ${t('depthPrefix', { n: n.depth })}${n.root ? ` ${t('tagRoot')}` : ''}${!n.has_follow_set ? ` ${t('tagNoFollowSet')}` : ''}`),
      ]),
    );
  }
  wrap.append(accounts);

  const mutual = data.edges.filter((e) => e.mutual);
  const oneway = data.edges.filter((e) => !e.mutual);

  const mutualGroup = el('div', { class: 'swing-webring-group' }, el('h3', {}, t('mutualHeading', { n: mutual.length })));
  if (mutual.length === 0) mutualGroup.append(el('p', { class: 'swing-hint' }, t('none')));
  for (const e of mutual) mutualGroup.append(el('div', { class: 'swing-webring-account-row' }, `${labelOf(nodeByKey, e.from)} ↔ ${labelOf(nodeByKey, e.to)}`));
  wrap.append(mutualGroup);

  const onewayGroup = el('div', { class: 'swing-webring-group' }, el('h3', {}, t('onewayHeading', { n: oneway.length })));
  if (oneway.length === 0) onewayGroup.append(el('p', { class: 'swing-hint' }, t('none')));
  for (const e of oneway) onewayGroup.append(el('div', { class: 'swing-webring-account-row' }, `${labelOf(nodeByKey, e.from)} → ${labelOf(nodeByKey, e.to)}`));
  wrap.append(onewayGroup);

  webringEls.content.append(wrap);
}

function renderAsciiStyle(data) {
  webringEls.content.append(el('pre', { class: 'swing-pre' }, data.text || ''));
}

function buildSourceBlock(title, text) {
  const block = el('div', { class: 'swing-source-block' });
  block.append(
    el('div', { class: 'swing-source-block-head' }, [
      el('h3', {}, title),
      el('button', { type: 'button', class: 'swing-btn swing-copy-btn', onclick: (ev) => copyWithFeedback(ev.currentTarget, text || '') }, t('copy')),
    ]),
  );
  block.append(el('pre', { class: 'swing-pre' }, text || ''));
  return block;
}

function renderSourceStyle(data) {
  webringEls.content.append(buildSourceBlock(t('graphvizTitle'), data.dot));
  webringEls.content.append(buildSourceBlock(t('mermaidTitle'), data.mermaid));
}

async function getMirrorMemberSet() {
  if (cache.sites) return new Set(cache.sites.accounts.map((a) => a.pubkey));
  if (cache.mirror) return new Set(cache.mirror.members.map((m) => m.pubkey));
  const data = await apiFetch('/api/mirror');
  cache.mirror = data;
  return new Set(data.members.map((m) => m.pubkey));
}

async function selectNode(pubkey) {
  selectedNodePubkey = pubkey;
  if (graphInstance) graphInstance.setSelected(pubkey);
  const gen = ++selectNodeGen;
  setDetailVisible(true);
  webringEls.detail.replaceChildren(el('p', { class: 'swing-status', 'data-kind': 'loading' }, t('loadingReplicas')));
  const data = cache.webringByQuery.get(webringQueryKey(currentWebringQuery));
  const node = data ? data.nodes.find((n) => n.pubkey === pubkey) : null;
  try {
    const cachedReplicas = cache.replicasByKey.get(pubkey);
    const [replicas, memberSet] = await Promise.all([
      cachedReplicas || apiFetch(`/api/replicas?key=${encodeURIComponent(pubkey)}`),
      getMirrorMemberSet(),
    ]);
    if (!cachedReplicas) cache.replicasByKey.set(pubkey, replicas);
    if (gen !== selectNodeGen) return;
    renderNodeDetail(node, pubkey, replicas, memberSet);
  } catch (err) {
    if (gen !== selectNodeGen) return;
    webringEls.detail.replaceChildren(el('p', { class: 'swing-status', 'data-kind': 'error' }, describeError(err)));
  }
}

function renderNodeDetail(node, pubkey, replicasResp, memberSet) {
  webringEls.detail.replaceChildren();
  const isMirrored = memberSet.has(pubkey);
  webringEls.detail.append(
    el('div', { class: 'swing-heading-row' }, [
      el('h2', {}, node ? node.label : t('tableAccount')),
      isMirrored ? el('span', { class: 'swing-badge', 'data-mirrored': 'true' }, t('mirroredBadge')) : null,
    ]),
  );
  const dl = el('dl');
  const npubDd = node
    ? el('span', { class: 'swing-copyable' }, [
        document.createTextNode(node.short_npub || node.npub),
        el('button', { type: 'button', class: 'swing-btn swing-copy-btn', 'aria-label': t('copyNpub'), onclick: (ev) => copyWithFeedback(ev.currentTarget, node.npub) }, t('copy')),
      ])
    : shortenMiddle(pubkey, 10, 6);
  dl.append(el('dt', {}, 'npub'), el('dd', {}, npubDd));
  if (node) {
    dl.append(el('dt', {}, t('detailNames')), el('dd', {}, node.names.length ? node.names.join(', ') : '–'));
    dl.append(el('dt', {}, t('detailDepth')), el('dd', {}, String(node.depth)));
  }
  webringEls.detail.append(dl);

  const author = replicasResp.authors.find((a) => a.pubkey === pubkey);
  if (!author || author.sites.length === 0) {
    webringEls.detail.append(el('p', { class: 'swing-hint' }, t('noPublishedSitesForNode')));
  } else {
    for (const site of author.sites) {
      const block = el('div', { class: 'swing-site' });
      block.append(
        el('div', { class: 'swing-site-row' }, [
          el('span', { class: 'swing-site-name' }, site.d),
          el('span', { class: 'swing-badge' }, t('replicasReportsBadge', { replicas: site.replicas, reports: site.reports })),
        ]),
      );
      const reporters = el('ul', { class: 'swing-plain-list' });
      for (const r of site.reporters) {
        reporters.append(
          el('li', {}, `${r.npub} ${r.latest ? t('tagLatest') : t('tagOlderVersion')}${r.is_author ? t('tagAuthor') : ''}${!r.following ? t('tagNotFollowing') : ''}`),
        );
      }
      block.append(reporters);
      webringEls.detail.append(block);
    }
  }

  const actions = el('div', { class: 'swing-node-detail-actions' }, [
    el('button', { type: 'button', class: 'swing-btn', onclick: (ev) => useAsRoot(pubkey, ev.currentTarget) }, t('useAsRoot')),
  ]);
  const isSelf = cache.overview && cache.overview.pubkey === pubkey;
  if (!isSelf) {
    if (isMirrored) {
      actions.append(buildDetailRemoveControl(pubkey));
    } else {
      actions.append(el('button', { type: 'button', class: 'swing-btn swing-btn-accent', onclick: (ev) => addToMirrorFromDetail(pubkey, ev.currentTarget) }, t('addToMirror')));
    }
  }
  webringEls.detail.append(actions);
}

function useAsRoot(pubkey, btn) {
  webringEls.form.elements.root.value = pubkey;
  setWebringQuery(pubkey, currentWebringQuery.depth);
  WebringView.load(true, btn);
}

function buildDetailRemoveControl(pubkey) {
  return buildRemoveControl({ pubkey }, async (result, err) => {
    if (err) {
      webringEls.detail.append(el('p', { class: 'swing-status', 'data-kind': 'error' }, describeError(err)));
      return;
    }
    cache.sites = null;
    cache.mirror = null;
    await selectNode(pubkey);
  }, { size: 'normal', inline: true });
}

async function addToMirrorFromDetail(pubkey, btn) {
  setBusy(btn, true);
  try {
    await apiFetch('/api/mirror/add', { method: 'POST', body: JSON.stringify({ keys: [pubkey] }) });
    cache.sites = null;
    cache.mirror = null;
    setBusy(btn, false);
    await selectNode(pubkey);
  } catch (err) {
    setBusy(btn, false);
    webringEls.detail.append(el('p', { class: 'swing-status', 'data-kind': 'error' }, describeError(err)));
  }
}

const WebringView = {
  init() {
    const saved = loadSavedWebringQuery();
    if (saved) {
      webringEls.form.elements.root.value = saved.root;
      webringEls.form.elements.depth.value = String(saved.depth);
      currentWebringQuery = { roots: saved.root ? saved.root.split(/[\s,]+/).filter(Boolean) : [], depth: saved.depth };
    }
    wireStyleSwitch('webring', () => this.render());
    webringEls.form.addEventListener('submit', (ev) => {
      ev.preventDefault();
      const rootVal = webringEls.form.elements.root.value.trim();
      const depthVal = clamp(parseInt(webringEls.form.elements.depth.value, 10) || 0, 0, 4);
      webringEls.form.elements.depth.value = String(depthVal);
      setWebringQuery(rootVal, depthVal);
      this.load(true, webringEls.form.querySelector('button[type="submit"]'));
    });
  },
  onShow() {
    if (cache.webringByQuery.has(webringQueryKey(currentWebringQuery))) this.render();
    else this.load();
  },
  async load(force, updateBtn) {
    const key = webringQueryKey(currentWebringQuery);
    if (!force && cache.webringByQuery.has(key)) return this.render();
    const gen = ++webringLoadGen;
    const hasContent = webringEls.content.childElementCount > 0;
    if (updateBtn) setBusy(updateBtn, true);
    if (hasContent) {
      webringEls.content.setAttribute('aria-busy', 'true');
    } else {
      setStatus(webringEls.status, 'loading', t('loadingWebring'));
    }
    let data;
    try {
      data = await apiFetch(`/api/webring${key ? `?${key}` : ''}`);
    } catch (err) {
      if (gen !== webringLoadGen) return;
      cache.webringByQuery.delete(key);
      setStatus(webringEls.status, 'error', describeError(err));
      webringEls.content.replaceChildren();
      webringEls.content.removeAttribute('aria-busy');
      return;
    } finally {
      if (updateBtn) setBusy(updateBtn, false);
    }
    if (gen !== webringLoadGen) return;
    webringEls.content.removeAttribute('aria-busy');
    cache.webringByQuery.set(key, data);
    clearStatus(webringEls.status);
    this.render();
  },
  render() {
    document.body.dataset.style = getStyle('webring');
    const data = cache.webringByQuery.get(webringQueryKey(currentWebringQuery));
    if (graphInstance) {
      graphInstance.destroy();
      graphInstance = null;
    }
    webringEls.content.replaceChildren();
    setDetailVisible(false);
    if (!data) return;

    if (data.nodes.length === 0) {
      webringEls.content.append(el('p', { class: 'swing-status', 'data-kind': 'empty' }, t('noAccountsForRootDepth')));
      return;
    }

    const style = getStyle('webring');
    if (style === 'graph') renderGraphStyle(data);
    else if (style === 'list') renderListStyle(data);
    else if (style === 'ascii') renderAsciiStyle(data);
    else renderSourceStyle(data);

    if (data.beyond > 0) {
      webringEls.content.append(el('p', { class: 'swing-hint' }, t('beyondHint', { beyond: data.beyond, depth: data.depth })));
    }
  },
};

const publishEls = {
  status: document.getElementById('publish-status'),
  result: document.getElementById('publish-result'),
  form: document.getElementById('publish-form'),
  npub: document.getElementById('pub-npub'),
  hex: document.getElementById('pub-hex'),
  mirrorSet: document.getElementById('pub-mirror-set'),
  relays: document.getElementById('pub-relays'),
  mySitesStatus: document.getElementById('my-sites-status'),
  mySitesContent: document.getElementById('my-sites-content'),
  uploadInput: document.getElementById('publish-upload-input'),
  uploadInfo: document.getElementById('publish-upload-info'),
  progress: document.getElementById('publish-progress'),
};

let publishing = false;
let publishLoadGen = 0;
let mySitesLoadGen = 0;

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

function updateUploadInfo() {
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
  wrap.append(el('div', { class: 'swing-site-row' }, el('span', { class: 'swing-site-name' }, site.d)));
  wrap.append(el('div', { class: 'swing-site-meta' }, `${formatBytes(site.size)} · ${formatTime(site.created_at)}`));
  const links = el('div', { class: 'swing-site-links' }, [
    site.url ? maybeLink(site.url, t('openSite')) : null,
    site.gateway_url ? maybeLink(site.gateway_url, t('openGateway')) : null,
  ]);
  if (links.childNodes.length) wrap.append(links);
  const msg = sanitizeMessage(site.message);
  if (msg) wrap.append(el('p', { class: 'swing-site-message' }, `“${msg}”`));
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
  refreshSubmitState();
}

function renderMySites() {
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
  const gen = ++mySitesLoadGen;
  const firstLoad = !cache.publishSites;
  if (reloadBtn) setBusy(reloadBtn, true);
  if (firstLoad) setStatus(publishEls.mySitesStatus, 'loading', t('loadingYourSites'));
  let data;
  try {
    data = await apiFetch('/api/publish/sites');
  } catch (err) {
    if (gen !== mySitesLoadGen) return;
    cache.publishSites = null;
    setStatus(publishEls.mySitesStatus, 'error', describeError(err));
    publishEls.mySitesContent.replaceChildren();
    return;
  } finally {
    if (reloadBtn) setBusy(reloadBtn, false);
  }
  if (gen !== mySitesLoadGen) return;
  cache.publishSites = data;
  clearStatus(publishEls.mySitesStatus);
  renderMySites();
}

async function loadOverview(force) {
  if (cache.overview && !force) return cache.overview;
  cache.overview = await apiFetch('/api/overview');
  return cache.overview;
}

function updateNavFooter(overview) {
  document.getElementById('nav-mirror-set').textContent = t('navFooterMirror', { name: overview.mirror_set });
  document.getElementById('nav-version').textContent = t('navFooterVersion', { version: overview.version });
  document.getElementById('page-footer').textContent = `${t('navFooterMirror', { name: overview.mirror_set })} · ${t('navFooterVersion', { version: overview.version })}`;
}

function renderIdentity(overview) {
  publishEls.npub.textContent = overview.npub;
  publishEls.hex.textContent = overview.pubkey;
  publishEls.mirrorSet.textContent = overview.mirror_set;
  publishEls.relays.replaceChildren();
  for (const r of overview.relays) publishEls.relays.append(el('li', {}, r));
}

function renderPublishResult(result, errBody) {
  publishEls.result.hidden = false;
  publishEls.result.replaceChildren();
  if (result) {
    const dl = el('dl', { class: 'swing-result-grid' });
    const addRow = (k, v) => dl.append(el('dt', {}, k), el('dd', {}, v));
    addRow(t('resultSite'), result.site);
    if (result.url) addRow(t('resultUrl'), result.url);
    addRow(t('resultNip05'), `${result.nip05.status}${result.nip05.detail ? ` — ${result.nip05.detail}` : ''}`);
    addRow(t('resultCid'), result.cid);
    addRow(t('resultSize'), formatBytes(result.size));
    addRow(t('resultCreated'), formatTime(result.created_at));
    addRow(t('resultMfsPath'), result.mfs_path);
    if (result.files != null) addRow(t('resultFiles'), String(result.files));
    if (result.pruned && result.pruned.length) addRow(t('resultPruned'), result.pruned.join(', '));
    if (result.prune_error) addRow(t('resultPruneError'), result.prune_error);
    publishEls.result.append(dl);
    if (result.gateway_url) publishEls.result.append(el('p', {}, maybeLink(result.gateway_url, t('openGateway'))));
    renderRelayResults(publishEls.result, result.relays);
  } else if (errBody && errBody.nip05) {
    const dl = el('dl', { class: 'swing-result-grid' });
    dl.append(el('dt', {}, t('resultNip05')), el('dd', {}, `${errBody.nip05.status}${errBody.nip05.detail ? ` — ${errBody.nip05.detail}` : ''}`));
    publishEls.result.append(dl);
  }
}

function handlePublishHttpError(status, body) {
  const errLike = { status, body, message: body && typeof body.error === 'string' ? body.error : `HTTP ${status}` };
  if (status === 413) {
    const maxUpload = cache.overview ? cache.overview.max_upload : null;
    setStatus(publishEls.status, 'error', t('uploadLimitError', { max: maxUpload != null ? formatBytes(maxUpload) : errLike.message }));
  } else if (status === 422 && body && body.nip05) {
    setStatus(publishEls.status, 'error', t('nip05CheckFailed', { detail: describeError(errLike) }));
    renderPublishResult(null, body);
  } else if (status === 409) {
    setStatus(publishEls.status, 'error', t('publishBusy'));
  } else {
    setStatus(publishEls.status, 'error', describeError(errLike));
  }
}

function buildUploadFormData({ site, url, message, nip05, files }) {
  const fd = new FormData();
  fd.append('site', site);
  if (url) fd.append('url', url);
  if (message) fd.append('message', message);
  if (nip05) fd.append('nip05', nip05);
  for (const file of files) {
    fd.append('file', file, computeRelativePath(file));
  }
  return fd;
}

function submitUpload({ site, url, message, nip05, files }) {
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
      setStatus(publishEls.status, 'loading', t('processingOnAgent'));
    });
    xhr.addEventListener('error', () => {
      showProgress('error', 100);
      setStatus(publishEls.status, 'error', t('unreachable'));
      finishUpload();
      resolve();
    });
    xhr.addEventListener('load', () => {
      let body = null;
      if (xhr.responseText) {
        try {
          body = JSON.parse(xhr.responseText);
        } catch {
          body = null;
        }
      }
      if (xhr.status >= 200 && xhr.status < 300) {
        showProgress('done', 100);
        clearStatus(publishEls.status);
        renderPublishResult(body, null);
        saveLastPublish({ site, url, message, nip05 });
        publishEls.uploadInput.value = '';
        updateUploadInfo();
      } else {
        showProgress('error', 100);
        handlePublishHttpError(xhr.status, body);
      }
      finishUpload();
      resolve();
    });
    xhr.send(buildUploadFormData({ site, url, message, nip05, files }));
  });

  function finishUpload() {
    publishing = false;
    setFormDisabled(publishEls.form, false);
    setBusy(submitBtn, false);
    refreshSubmitState();
  }
}

const PublishView = {
  init() {
    publishEls.uploadInput.addEventListener('change', () => updateUploadInfo());
    publishEls.form.elements.site.addEventListener('input', () => refreshSubmitState());
    document.querySelector('[data-action="reload-my-sites"]').addEventListener('click', (ev) => loadMySites(true, ev.currentTarget));

    const last = readLastPublish();
    if (last) {
      const form = publishEls.form;
      if (last.site) form.elements.site.value = last.site;
      if (last.url) form.elements.url.value = last.url;
      if (last.message) form.elements.message.value = last.message;
      if (last.nip05) form.elements.nip05.value = last.nip05;
    }
    refreshSubmitState();

    publishEls.form.addEventListener('submit', (ev) => {
      ev.preventDefault();
      if (publishing) return;
      hideProgress();
      const fd = new FormData(publishEls.form);
      const site = String(fd.get('site') || '').trim();
      const url = String(fd.get('url') || '').trim();
      const message = String(fd.get('message') || '').trim();
      const nip05 = String(fd.get('nip05') || '');

      const files = Array.from(publishEls.uploadInput.files || []);
      if (files.length === 0) {
        setStatus(publishEls.status, 'error', t('chooseFolderToUpload'));
        return;
      }
      submitUpload({ site, url, message, nip05, files });
    });
  },
  onShow() {
    this.load();
    loadMySites();
  },
  async load(force) {
    const gen = ++publishLoadGen;
    if (!cache.overview) setStatus(publishEls.status, 'loading', t('loadingGeneric'));
    try {
      const overview = await loadOverview(force);
      if (gen !== publishLoadGen) return;
      renderIdentity(overview);
      clearStatus(publishEls.status);
      updateNavFooter(overview);
      updateUploadInfo();
    } catch (err) {
      if (gen !== publishLoadGen) return;
      setStatus(publishEls.status, 'error', describeError(err));
    }
  },
};

const settingsEls = {
  status: document.getElementById('settings-config-status'),
  content: document.getElementById('settings-config-content'),
  themeSelect: document.getElementById('theme-select'),
  langSelect: document.getElementById('lang-select'),
  userCssInput: document.getElementById('user-css-input'),
  userCssApply: document.getElementById('user-css-apply'),
  userCssReset: document.getElementById('user-css-reset'),
};

function formatRawConfigValue(v) {
  if (Array.isArray(v)) return v.length ? v.join(', ') : '–';
  if (v == null) return '–';
  return String(v);
}

function buildConfigValueCell(item) {
  if (item.display != null) {
    return el('span', {}, [
      document.createTextNode(`${item.display} `),
      el('span', { class: 'swing-hint swing-mono' }, formatRawConfigValue(item.value)),
    ]);
  }
  return document.createTextNode(formatRawConfigValue(item.value));
}

function renderConfig(config) {
  settingsEls.content.replaceChildren();
  settingsEls.content.append(
    el('p', { class: 'swing-hint' }, config.config_path ? t('configFile', { path: config.config_path }) : t('configEnvOnly')),
  );
  for (const section of config.sections) {
    const sec = el('div', { class: 'swing-config-section' }, el('h3', {}, section.name));
    const table = el('table', { class: 'swing-table' });
    const headers = [t('tableKey'), t('tableValue'), t('tableEnv')];
    table.append(el('thead', {}, el('tr', {}, headers.map((h) => el('th', {}, h)))));
    const tbody = el('tbody');
    for (const item of section.items) {
      const tr = el('tr');
      tr.append(el('td', { class: 'swing-mono' }, item.key || '–'));
      tr.append(el('td', {}, buildConfigValueCell(item)));
      tr.append(el('td', { class: 'swing-mono' }, item.env || '–'));
      tbody.append(tr);
    }
    table.append(tbody);
    sec.append(table);
    settingsEls.content.append(sec);
  }
}

function applyTheme(theme) {
  const root = document.documentElement;
  if (theme === 'light' || theme === 'dark') root.setAttribute('data-theme', theme);
  else root.removeAttribute('data-theme');
}

let settingsLoadGen = 0;

const SettingsView = {
  init() {
    const savedTheme = storage.get('swing:theme', 'auto');
    applyTheme(savedTheme);
    settingsEls.themeSelect.value = savedTheme;
    settingsEls.themeSelect.addEventListener('change', () => {
      storage.set('swing:theme', settingsEls.themeSelect.value);
      applyTheme(settingsEls.themeSelect.value);
    });

    settingsEls.langSelect.value = storage.get('swing:lang', 'auto');
    settingsEls.langSelect.addEventListener('change', () => {
      storage.set('swing:lang', settingsEls.langSelect.value);
      applyLanguage();
    });

    const userCssEl = document.getElementById('user-css');
    const savedCss = storage.get('swing:user-css', '');
    userCssEl.textContent = savedCss;
    settingsEls.userCssInput.value = savedCss;
    settingsEls.userCssApply.addEventListener('click', () => {
      storage.set('swing:user-css', settingsEls.userCssInput.value);
      userCssEl.textContent = settingsEls.userCssInput.value;
    });
    settingsEls.userCssReset.addEventListener('click', () => {
      settingsEls.userCssInput.value = '';
      storage.remove('swing:user-css');
      userCssEl.textContent = '';
    });
  },
  onShow() {
    if (cache.config) renderConfig(cache.config);
    else this.load();
  },
  async load(force) {
    const gen = ++settingsLoadGen;
    setStatus(settingsEls.status, 'loading', t('loadingConfig'));
    try {
      const data = force || !cache.config ? await apiFetch('/api/config') : cache.config;
      if (gen !== settingsLoadGen) return;
      cache.config = data;
      clearStatus(settingsEls.status);
      renderConfig(cache.config);
    } catch (err) {
      if (gen !== settingsLoadGen) return;
      setStatus(settingsEls.status, 'error', describeError(err));
    }
  },
};

const VIEWS = { sites: SitesView, webring: WebringView, publish: PublishView, settings: SettingsView };

function currentRoute() {
  const hash = location.hash.replace(/^#\/?/, '');
  return Object.prototype.hasOwnProperty.call(VIEWS, hash) ? hash : 'sites';
}

function showRoute() {
  const name = currentRoute();
  document.body.dataset.view = name;
  for (const key of Object.keys(VIEWS)) {
    document.getElementById(`view-${key}`).hidden = key !== name;
  }
  const style = getStyle(name);
  if (style) document.body.dataset.style = style;
  else delete document.body.dataset.style;
  VIEWS[name].onShow();
}

function wireReloadButtons() {
  document.querySelector('[data-action="reload-sites"]').addEventListener('click', (ev) => SitesView.load(true, ev.currentTarget));
}

function wireCopyButtons() {
  document.querySelectorAll('[data-copy-target]').forEach((btn) => {
    btn.addEventListener('click', (ev) => copyWithFeedback(ev.currentTarget, document.getElementById(btn.dataset.copyTarget).textContent));
  });
}

function applyStaticI18n() {
  document.documentElement.lang = currentLang() === 'ja' ? 'ja' : 'en';
  document.querySelectorAll('[data-i18n]').forEach((elm) => {
    elm.textContent = t(elm.dataset.i18n);
  });
  document.querySelectorAll('[data-i18n-placeholder]').forEach((elm) => {
    elm.placeholder = t(elm.dataset.i18nPlaceholder);
  });
  document.querySelectorAll('[data-i18n-aria-label]').forEach((elm) => {
    elm.setAttribute('aria-label', t(elm.dataset.i18nAriaLabel));
  });
  document.querySelectorAll('[data-i18n-title]').forEach((elm) => {
    elm.setAttribute('title', t(elm.dataset.i18nTitle));
  });
}

function applyLanguage() {
  applyStaticI18n();
  if (cache.overview) {
    renderIdentity(cache.overview);
    updateNavFooter(cache.overview);
  }
  updateUploadInfo();
  if (cache.sites) SitesView.render();
  if (cache.webringByQuery.has(webringQueryKey(currentWebringQuery))) WebringView.render();
  if (cache.publishSites) renderMySites();
  if (cache.config) renderConfig(cache.config);
}

function init() {
  applyStaticI18n();
  SitesView.init();
  WebringView.init();
  PublishView.init();
  SettingsView.init();
  wireReloadButtons();
  wireCopyButtons();
  window.addEventListener('hashchange', showRoute);
  showRoute();
  loadOverview()
    .then(updateNavFooter)
    .catch(() => {});
}

init();
