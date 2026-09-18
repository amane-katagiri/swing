# ダッシュボードの画面（`web/index.html`, `web/*.js`）

[`dashboard.md`](../dashboard.md) の一部。サーバ側の起動・ガード・タイムアウト・静的ファイル配信は [`dashboard.md`](../dashboard.md)、HTTP API の入出力は [`http-api.md`](http-api.md) を参照。

## 構成

ビルド工程なし・外部依存なしの素の HTML + CSS + ES modules。依存は下から上への一方向で循環 import は無い。

| ファイル | 役割 |
|---|---|
| `storage.js` | `localStorage` の薄いラッパー。他のどのモジュールにも依存しない |
| `i18n.js` | 多言語辞書と `t()`。`storage.js` にだけ依存する |
| `util.js` | 画面間で共有するキャッシュ・DOM/fetch ユーティリティ・表示スタイル切替・非同期ロードのガード（`createLoadGuard`）。`storage.js`・`i18n.js` に依存する |
| `ui.js` | 複数画面で共有する UI 部品（コピーボタン、バッジ、relay 結果表示など）。`util.js`・`i18n.js` に依存する |
| `graph.js` | webring 用の自前 force-directed layout。`util.js` の `clamp` だけに依存する |
| `sites.js` / `webring.js` / `publish.js` / `settings.js` | 各画面（それぞれ Sites・Webring・Publish・Settings） |
| `app.js` | ルーター兼エントリポイント。`<script type="module" src="/app.js">` から読み込まれ、各画面モジュールを import する |

`#/sites` `#/webring` `#/publish` `#/settings` の 4 画面をハッシュルーティングで切り替える（既定は `sites`）。書き込みリクエストには `X-Swing-Dashboard: 1` と `Content-Type: application/json` を付ける。relay 由来の文字列は DOM API だけで挿入し、`innerHTML` は使わない。`url` は `^https?://` にマッチするときだけリンクにする。

言語切り替え（Settings 画面）は `settings.js` が `swing:langchange` という `CustomEvent` を `document` に投げ、`app.js` がそれを購読して各画面を再描画する。各画面のロードは世代カウンタ（`createLoadGuard`）でガードし、切り替えが速くても古いレスポンスで上書きしない。

| 画面 | 内容 | 表示スタイル（`data-style`、localStorage キー `swing:style:<view>`） |
|---|---|---|
| Sites | [`/api/sites`](http-api.md#get-apisites) の一覧、mirror への追加・削除、`Unfollowed but still stored`、[`/api/status`](http-api.md#get-apistatus) を呼ぶ Storage check | `list`（既定）/ `cards` / `table` |
| Webring | [`/api/webring`](http-api.md#get-apiwebringrootkeydepthn) を root・depth 指定で取得。ノード選択で [`/api/replicas?key=`](http-api.md#get-apireplicaskeykey) を引き、詳細パネルからミラー操作もできる | `graph`（既定）/ `list` / `ascii` / `source`（dot・mermaid） |
| Publish | [`/api/overview`](http-api.md#get-apioverview)・[`/api/publish/sites`](http-api.md#get-apipublishsites)（My sites）、publish フォーム（常にフォルダアップロード） | スタイル切替なし |
| Settings | [`/api/config`](http-api.md#get-apiconfig) を読み取り専用表示。テーマ・言語・カスタム CSS の設定 | スタイル切替なし |

## Sites 画面

並び順（`.swing-sort-switch`、`localStorage["swing:sites:sort"]`、既定 `updated`）:

- `updated`: 各アカウントの最初のサイトを `created_at` 降順で比較（アカウント内のサイトも同基準）。サイト無しは最後。
- `name`: 最初のサイトの `d` のロケール順（`localeCompare`）。サイト無しは最後。
- `pubkey`: 並べ替えなし（`/api/sites` が返す順）。

`Unfollowed but still stored` セクションも同じ並び順ロジックを共有する。「Stored only」チェックボックスの状態は `localStorage["swing:sites:stored-only"]`。

## Publish 画面

- My sites: 一覧の「Use」ボタンは `site`・`url`（`message` を除く）をフォームに入れるだけ。
- publish フォームは常にフォルダアップロード（`<input type="file" webkitdirectory multiple>`）。ファイル数・合計サイズを表示し、`max_upload` を超えれば送信ボタンを無効化する。送信は `XMLHttpRequest` で、進捗を `.swing-progress`/`.swing-progress-bar`（`data-state`）に反映する。413 は「上限を超えた」という文言に言い換える。
- 各ファイルの送信名は `webkitRelativePath` から選んだフォルダ名を除いたもの。最後に使ったフォーム内容は `swing:publish:last` に保存する（下記の localStorage 一覧を参照）。

## Webring 画面

- root/depth のクエリは `swing:webring:query` に保存し、次に開いたときに復元する。
- 再取得中、既存の表示は消さず `aria-busy="true"` で薄く表示する。初回だけ「Loading webring…」になる。
- ノード詳細パネルのミラー操作は選んだノードが自分自身かどうかで変える（自分自身: ボタン無し／ミラー済み: 削除ボタン／未ミラー: 追加ボタン）。判定はキャッシュ済みの `/api/sites` か `/api/mirror` の pubkey 集合。

### グラフ（`web/graph.js`）

外部ライブラリを使わない自前の force-directed layout。ドラッグでノードを固定でき、クリックで選択して詳細パネルを開く。キーボード操作（Tab で移動、Enter/Space で選択）に対応する。パン・ホイールズーム（0.15〜4 倍）と全体表示（Fit）ができ、`prefers-reduced-motion: reduce` ではアニメーションせず同期的に 1 回だけ描画する。ノード・辺は class と `data-*` だけを持ち、色は付けない（配色は CSS 側、下記参照）。

## 共通の UI 部品

- busy 表示: `setBusy(button, bool)` で `disabled`・`aria-busy`・`.is-busy` を切り替える。ボタン幅は変わらない。`prefers-reduced-motion: reduce` ではスピナーを止め静止表示にする。
- コピー: 成功で 1.5 秒だけ `data-copied="true"`、失敗で `data-copy-failed="true"`。表示文字列はすべて共通の `copy` キーで、対象の違いは `aria-label` 側で表す。
- ボタン: `.swing-btn`（主要操作）、`.swing-btn-small`（行単位）、`.swing-copy-btn`（インラインコピー）、`.swing-icon-btn`（アイコンのみ）。
- モバイル幅: 760px 以下でサイドナビのフッタを隠し、ページ最下部の `<footer id="page-footer">` に同じ内容を表示する。

## localStorage キー一覧

ダッシュボードが使う `localStorage` のキーはこれで全部（他にサーバに送るものは無い）。

| キー | 値の形 | 意味 |
|---|---|---|
| `swing:style:<view>`（`view` は `sites`/`webring`） | 文字列（スタイル名） | 画面ごとの表示スタイル |
| `swing:sites:sort` | `updated` / `name` / `pubkey` | Sites の並び順（既定 `updated`） |
| `swing:sites:stored-only` | `"1"` / `"0"` | Sites の「Stored only」チェックボックスの状態 |
| `swing:webring:query` | JSON `{root, depth}` | Webring の最後のクエリ（起動時に復元） |
| `swing:publish:last` | JSON `{site, url, message, nip05}` | Publish フォームの最後の入力（起動時にプリフィル） |
| `swing:theme` | `auto` / `light` / `dark` | 表示テーマ |
| `swing:lang` | `auto` / `en` / `ja` | 表示言語 |
| `swing:user-css` | 文字列（CSS） | Settings のカスタム CSS 欄の内容 |

## 表示言語（i18n）

`web/i18n.js` の `MESSAGES = { en: {...}, ja: {...} }` を `t(key, vars)` で参照する。`localStorage["swing:lang"]`（`auto`/`en`/`ja`。`auto` は `navigator.language` が `ja` で始まるかで判定）で切り替え、再読み込みは不要。訳が無いキーは英語にフォールバックする。

訳さないもの: ナビゲーションの「Webring」、webring の ASCII/DOT/Mermaid 出力、API のエラー文字列、npub・hex・CID・パス、環境変数名・設定キー名、`nip05`/`health` のステータス値、NIP-05 モードの `off`/`warn`/`require`。日本語訳は「mirror」を指す語をすべて「ミラー」に統一している。日時表示は `Intl.DateTimeFormat`（`ja-JP`/`en-US`）を使う。

## CSS カスタマイズのインターフェース

読み込み順は `style.css` → `/custom.css`（サーバ設定、[`dashboard.md`](../dashboard.md#静的ファイルの配信srcdashboardassetsrs)） → `<style id="user-css">`（ブラウザの localStorage、後勝ち）。

既定は白黒中立基調＋アクセント 1 色（`--swing-accent: #1f5aa8`）の配色。`--swing-root`（webring の root ノードの色）は `var(--swing-accent)` を参照するので、アクセントを変えるだけで揃って変わる。

- CSS 変数（`web/style.css` の `:root`）: `--swing-bg` `--swing-surface` `--swing-surface-alt` `--swing-surface-raised` `--swing-border` `--swing-border-strong` `--swing-text` `--swing-text-muted` `--swing-text-faint` `--swing-accent` `--swing-accent-strong` `--swing-accent-soft` `--swing-warn` `--swing-warn-soft` `--swing-danger` `--swing-danger-soft` `--swing-ok` `--swing-ok-soft` `--swing-root` `--swing-focus` `--swing-font-display` `--swing-font-ui` `--swing-font-mono` `--swing-space-1`〜`--swing-space-6` `--swing-radius` `--swing-radius-lg` `--swing-nav-width` `--swing-graph-label-size`（既定 11px）。
- テーマ: 既定は `@media (prefers-color-scheme: dark)` に連動。`<html data-theme="light"|"dark">` で上書き（Settings 画面が `localStorage["swing:theme"]` に保存してこの属性を付け替える）。
- 状態フック: `<body data-view="sites|webring|publish|settings" data-style="<現在の表示スタイル>">`。
- 安定 class（抜粋。`swing-` 接頭辞で統一）: レイアウト系 `swing-shell` `swing-nav` `swing-main` `swing-view` `swing-panel` `swing-toolbar` `swing-style-switch` `swing-sort-switch`。Sites 系 `swing-site` `swing-site-row` `swing-site-meta-cid` `swing-site-meta-info` `swing-account`。共通部品 `swing-badge` `swing-btn`（`swing-btn-accent`/`swing-btn-danger`/`swing-btn-small`）`swing-copy-btn` `swing-icon-btn` `swing-status` `swing-hint` `swing-table` `swing-mono` `swing-pre` `swing-relay-results` `swing-page-footer`。Webring 系 `swing-graph` `swing-node` `swing-edge` `swing-arrow-oneway-fill` `swing-arrow-mutual-fill` `swing-legend` `swing-webring-layout` `swing-node-detail` `swing-source-block`。Publish 系 `swing-my-sites` `swing-progress` `swing-progress-bar` `swing-identity`。
- 状態は data 属性: `data-stored="true|false"`、`data-nip05="verified|mismatch|not_applicable|error"`、`data-health="ok|missing|cid_mismatch|incomplete|check_failed|invalid_key"`、`data-ok="true|false"`（relay 結果）、`data-kind="loading|error|empty"`（`swing-status`）、`data-root`/`data-has-follow-set`/`data-depth`/`data-selected`（グラフのノード）、`data-mutual`（グラフの辺）、`data-style-value`/`data-sort-value`（切替ボタン自身の値）、`data-mirrored="true"`、`data-detail="true|false"`（詳細パネル表示中か）、`data-copied`/`data-copy-failed`、`data-state="uploading|processing|done|error"`（`swing-progress`）、`aria-busy="true"`（busy 中のボタン、再取得中の webring 表示領域）。
- SVG グラフは class と `data-*` だけを付け、色は JS に書かない（`style.css` 側で `.swing-node[data-root="true"] circle { ... }` のように当てる）。
