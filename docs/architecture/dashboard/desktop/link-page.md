# リンク集ページ（`web/desktop-page.html`, `web/desktop-page.css`, `web/desktop-frame.css`, `web/desktop.js`）

[`../desktop.md`](../desktop.md) の子ページ。SWING Explorer ウィンドウの中身（`#desk-page-frame`）は、同一オリジンの iframe で開く `/desktop-page.html` の別ドキュメント。ルートと差し替え設定は [`../../dashboard.md#静的ファイルの配信srcdashboardassets`](../../dashboard.md#静的ファイルの配信srcdashboardassets)。

- CSS: `index.html` のダッシュボードの CSS は当たらず、ページ側の CSS も外に漏れない。窓側の `desktop-frame.css` だけは `desktop.js` が iframe の `<head>` の先頭に `<link>` で差し込む。ページ専用 CSS は `--desk-*`・`@font-face`・リセットまで自己完結する。
- 読み込みの時機: このページとバナー・マスコットはログインが要る（[`security.md#ガード`](../security.md#ガード)）。`index.html` の iframe には `src` を書かず、`app.js` が `/api/overview` の読み込みを終えた後（失敗しても 401 でなければ）に呼ぶ `DesktopView.start()` が `src` を入れ、`DesktopMascots.start()` でパックを読み始める。ログイン画面を出すときは呼ばない。
- `src` の無い間の `about:blank` の `load` は無視する。

iframe には `sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"` を付ける。差し替えたページの `<script>`・イベントハンドラ属性・フォーム送信・親ウィンドウの遷移は動かず、`target="_blank"` のリンクは制限の無い新しいタブで開く。親の `desktop.js` は `contentDocument` を直接読み書きする。

`desktop.js` はページ側の JS を前提にせず、決まった `id` を見つけたときだけ書き込む（無い `id` は黙って飛ばす）。差し替えるページはこの `id` と、`desktop.js` が差し込む要素のクラス・属性を契約として使える:

| `id` | 書き込む内容 |
|---|---|
| `desk-link-list` | リンク行（下表）を `replaceChildren` で流し込む |
| `desk-page-status` | 読み込み中・エラー・空の文面。`data-kind` に `loading` / `error` / `empty` を付ける（リンクを出せたときは文面を消して属性ごと外す） |
| `desk-marquee-text` | マーキーの文面 |
| `desk-counter` | 来訪者カウンタ |

| 差し込む要素 | 内容 |
|---|---|
| `li.desk-link-row` | 1 サイトの行 |
| `.desk-status-icon` | ステータスアイコン。`data-status` は `ng` / `err` / `new` / `up` / `default`（下記） |
| `.desk-link-date` | 日付（`YYYY.MM.DD`） |
| `.desk-link-title` | タイトル（リンクか `span`） |
| `.desk-link-d` | `title` を出すときに併記する `(d)` |
| `.desk-link-honke` | `[本家]` の第二リンク |
| `.desk-link-cache` | 更新待ちのサイトの `[キャッシュ]`（保存済みの前の版）のリンク |
| `.desk-link-message` | 更新メモ |

データは `cache.sites`（Sites 画面と共有）の `accounts` から保存済み（`stored` が true）と更新待ち（`stored` が false で `previous` がある）のサイトを、`unfollowed.accounts` から保存済みのサイトだけを集め、`created_at` 降順で並べる。各行のステータスアイコン（`.desk-status-icon`、`data-status`）は次の優先順で 1 個だけ選ぶ: (1) `nip05 === "mismatch"` → `ng`。(2) `nip05 === "error"`・未知の文字列 → `err`。(3) それ以外は更新の新しさで `new`（7 日以内）/`up`（30 日以内）/`default`（`not_applicable` は NIP-05 の問題扱いにせず新しさ判定に乗る）。

タイトルは自己申告の `title`（サニタイズ済み、最大 120 文字）があればそれを、無ければ `d` を表示し、`title` を出すときは必ず `d` も括弧付きで併記する。リンク先は `gateway_url` を一次リンク（無ければ `url`）にし、両方あれば `url` を `[本家]` の第二リンクにする。更新待ちのサイトは、日付・アイコン・タイトル・更新メモを新しい版（イベント）のもので出し、まだ取り込んでいない新しい版にはリンクしないのでタイトルをリンクにせず、`title` 属性で最新版がまだ保存されていないことを示す。`url` があれば `[本家]` を、`previous.gateway_url` があればそれを `[キャッシュ]` のリンクにして並べる。npub・cid・size・replicas は表示しない。来訪者カウンタは、ページ読み込みのたびに 1 増やす `localStorage["swing:desktop:visits"]` に、リンク集に出すサイトの数から求めた基準値を足して表示する。

## ツールチップ

各行の `title` と、それと対になる `aria-label` は UI 表示言語で出し、その要素の `lang` も UI 表示言語にする（[`../desktop.md#desktop-画面の表示言語`](../desktop.md#desktop-画面の表示言語)）。言語を切り替えたら `cache.sites` から行を作り直す（API は呼ばない）。

| 要素 | `title` | `aria-label` | キー |
|---|---|---|---|
| ステータスアイコン | アイコンの意味 | `title` と同じ | `deskIconMismatch`（`ng`）・`deskIconError`（`err`）・`deskIconNew`・`deskIconUp`・`deskIconDefault` |
| タイトル（リンク） | 開き方 | なし（リンクの文字が名前） | `openGateway`（`gateway_url` があるとき）・`openSite` |
| タイトル（更新待ちの `span`） | 最新版がまだ保存されていないこと | なし | `deskPending` |
| `[本家]` | なし | `<サイトのタイトル> — <開き方>` | `openSite` |
| `[キャッシュ]` | 前の版をゲートウェイで開くこと | `<サイトのタイトル> — <title と同じ文言>` | `deskOpenCache` |

リンクにしたタイトル・更新待ちのタイトル・`[本家]`・`[キャッシュ]` の見える文字は `<span lang="ja">` で包み、ページ（`lang="ja"`）の中のほかの文字と同じ扱いのままにする。更新待ちでもリンクでもないタイトルはツールチップが無いので包まない。
