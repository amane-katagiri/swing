# CSS カスタマイズのインターフェース（`web/style.css`）

[`web.md`](web.md) の子ページ。利用者が上書きしてよいダッシュボードの CSS 変数・class・data 属性をまとめる。

## 読み込み順

`style.css` → Desktop 系 6 ファイル（[`desktop.md#構成`](desktop.md#構成)） → `/custom.css`（サーバ設定。[`../dashboard.md#静的ファイルの配信srcdashboardassetsrs`](../dashboard.md#静的ファイルの配信srcdashboardassetsrs)） → `<style id="user-css">`（Settings 画面のカスタム CSS。`localStorage["swing:user-css"]`）。後に読むものが勝つ。Desktop 画面は `--swing-*` 変数を参照せず、リンク集ページ（iframe）にはどれも届かない（[`desktop.md#リンク集ページiframe`](desktop.md#リンク集ページiframe)）。

## CSS 変数（`web/style.css` の `:root`）

`--swing-bg` `--swing-surface` `--swing-surface-alt` `--swing-surface-raised` `--swing-border` `--swing-border-strong` `--swing-text` `--swing-text-muted` `--swing-text-faint` `--swing-accent` `--swing-accent-strong` `--swing-accent-soft` `--swing-warn` `--swing-warn-soft` `--swing-danger` `--swing-danger-soft` `--swing-ok` `--swing-ok-soft` `--swing-root` `--swing-focus` `--swing-font-display` `--swing-font-ui` `--swing-font-mono` `--swing-space-1`〜`--swing-space-6` `--swing-radius` `--swing-radius-lg` `--swing-nav-width` `--swing-nav-collapsed-width`（既定 64px） `--swing-graph-label-size`（既定 11px）。

- `--swing-root`（webring の root ノードの色）と `--swing-focus` は `var(--swing-accent)` を参照する。
- `--swing-dark-*`（`--swing-root`・`--swing-focus` 以外の色変数と同名の組）は、ダークテーマのときに色変数へ代入される元の値。ダークテーマの色だけを変えるならこちらを上書きする。

## テーマ

既定は `@media (prefers-color-scheme: dark)` に連動する。`<html data-theme="light"|"dark">` で上書きする（Settings 画面が保存した値で付け替える。キーは [`web.md#localstorage-キー一覧`](web.md#localstorage-キー一覧)）。

## 状態フック

`<body data-view="desktop|sites|webring|publish|settings|setup|login" data-style="<現在の表示スタイル>" data-nav="collapsed">`（`data-nav` は畳んでいるときだけ）。Desktop 画面でのレイアウトの違いは [`desktop.md#レイアウト`](desktop.md#レイアウト)。

## 安定 class

`swing-` 接頭辞で統一する（抜粋）。Desktop 画面専用の `desk-` 接頭辞のクラスの扱いは [`desktop.md#内部クラス非安定`](desktop.md#内部クラス非安定)。

| 分類 | class |
|---|---|
| レイアウト | `swing-shell` `swing-nav` `swing-nav-list` `swing-nav-icon` `swing-nav-label` `swing-nav-toggle` `swing-main` `swing-view` `swing-panel` `swing-toolbar` `swing-style-switch` `swing-sort-switch` |
| Sites | `swing-site` `swing-site-row` `swing-site-badges` `swing-site-meta-cid` `swing-site-meta-info` `swing-account` |
| 共通部品 | `swing-badge` `swing-btn`（`swing-btn-accent`/`swing-btn-danger`/`swing-btn-small`） `swing-copy-btn` `swing-icon-btn` `swing-status` `swing-hint` `swing-table`（セル用に `swing-nowrap` `swing-break-anywhere`） `swing-table-scroll`（表 1 つだけを包む横スクロール用ラッパー） `swing-mono` `swing-pre` `swing-relay-results` `swing-page-footer` |
| Webring | `swing-graph` `swing-node` `swing-edge` `swing-arrow-oneway-fill` `swing-arrow-mutual-fill` `swing-legend` `swing-webring-layout` `swing-node-detail` `swing-source-block` |
| Publish | `swing-my-sites` `swing-progress` `swing-progress-bar` `swing-identity` |
| Setup・Publish | `swing-signer-qr`（署名アプリ接続用の QR と注記・コピーボタンを縦に並べる `<figure>`。Setup 画面と Publish 画面のつなぎ直しで使う） |

## 状態を表す data 属性

| 属性 | 付く要素 |
|---|---|
| `data-stored="true\|false"` | サイトのカード・テーブルの行、保存状態のバッジ |
| `data-nip05="verified\|mismatch\|not_applicable\|error"` | NIP-05 のバッジ |
| `data-health="ok\|missing\|cid_mismatch\|incomplete\|check_failed\|invalid_key"` | Storage check の行とバッジ |
| `data-ok="true\|false"` | relay 結果の行（`swing-relay-result`） |
| `data-kind="loading\|error\|empty\|ok\|warn"` | `swing-status` |
| `data-root`・`data-has-follow-set`・`data-depth`・`data-selected` | グラフのノード |
| `data-mutual` | グラフの辺 |
| `data-style-value`・`data-sort-value` | 切替ボタン自身の値 |
| `data-mirrored="true"` | webring のノード詳細パネルのミラー済みバッジ |
| `data-detail="true\|false"` | `swing-webring-layout`（詳細パネルを表示中か） |
| `data-copied`・`data-copy-failed` | コピーボタン |
| `data-state="uploading\|processing\|error"` | `swing-progress`（完了時は属性ごと外して隠す） |
| `aria-busy="true"` | busy 中のボタン、再取得中の webring 表示領域 |

リンク集ページの `data-status`・`data-kind` は [`desktop.md#リンク集ページiframe`](desktop.md#リンク集ページiframe)。

SVG グラフは class と `data-*` だけを付け、色は JS に書かない（`style.css` 側で `.swing-node[data-root="true"] circle { ... }` のように当てる）。
