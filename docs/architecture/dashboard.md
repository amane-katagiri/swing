# ダッシュボード（src/dashboard/）

[`../architecture.md`](../architecture.md) の一部。agent 全体のループは [`agent.md`](agent.md)、シグナル・終了処理と起動順は [`up.md`](up.md)、Docker での公開は [`docker.md`](docker.md)。詳細は役割ごとに分けている:

- [`dashboard/security.md`](dashboard/security.md) — ガード・認証・リバースプロキシ経由での公開・既知の弱点・秘密鍵を出さない仕組み
- [`dashboard/http-api.md`](dashboard/http-api.md) — HTTP API の共通規則とエンドポイント一覧（各エンドポイントは `dashboard/http-api/` の子ページ）
- [`dashboard/web.md`](dashboard/web.md) — フロントエンド（`web/*.js`）の構成と共通部品
  - [`dashboard/views.md`](dashboard/views.md) — Desktop 以外の各画面
    - [`dashboard/views/publish.md`](dashboard/views/publish.md) — Publish 画面
  - [`dashboard/css.md`](dashboard/css.md) — CSS カスタマイズのインターフェース
- [`dashboard/desktop.md`](dashboard/desktop.md) — Desktop 画面（`web/desktop*`）
  - [`dashboard/desktop/control-panel.md`](dashboard/desktop/control-panel.md) — 「コントロール パネル」ダイアログ
  - [`dashboard/mascot.md`](dashboard/mascot.md) — Desktop 画面のマスコット（`web/desktop-mascot*`・`web/mascots/`）
- [`dashboard/notices.md`](dashboard/notices.md) — どの画面でも動く更新の確認・おしらせの出し分け・ブラウザの通知

## 概要

HTTP サーバー（axum 0.8）で、ブラウザ向けの管理画面（`[dashboard].ui = true` のとき）と、CLI の一部（一覧は [`cli.md#共通`](cli.md#共通)）と `swing-tray` が呼ぶ制御 API（`/api/*`、常に有効）を兼ねる。認証の要るパスは [`dashboard/security.md#ガード`](dashboard/security.md#ガード)。専用のサブコマンドは無く、`swing up` が起動する（[`up.md`](up.md)）。

- relay 接続（`Arc<RelayClient>`）と Kubo クライアント（`IpfsClient`）は agent から `AppState::set_ready` で受け取り、`tokio::sync::RwLock<Option<...>>` に持つ。`set_ready` の前と `set_not_ready` の後は `None`（呼ぶ時機は [`agent.md#全体の流れ`](agent.md#全体の流れ) と [`agent.md#シグナルと終了`](agent.md#シグナルと終了)）。その間に `503` を返す API は [`dashboard/http-api.md#共通`](dashboard/http-api.md#共通)。
- 保存状態は agent の `Mutex<State>` に触れず、CLI と同じく `state.json` をディスクから読み直す（`mirror::collect_sites`・`health::collect_status` など）。
- `mirror add` / `mirror remove` が relay に受理されると、`tokio::sync::Notify` で agent に知らせ、poll tick と同じ `poll_once`（[agent.md#全体の流れ](agent.md#全体の流れ) の 4）をその場で走らせる。
- 署名の方法（`AppState.signer: Option<signer::Signer>`。[`signer.md`](signer.md)）は `up::run` が決めて渡す。自分の公開鍵（`AppState.own_pubkey`）はそこから求めて持つ（署名アプリにリクエストは送らない）。

### セットアップモードと `AppState::setup_mode`

セットアップモード（条件と起動の流れは [`up.md#セットアップモード鍵未設定`](up.md#セットアップモード鍵未設定)）では `AppState::new` に `signer: None` が渡り、`own_pubkey` も `None` になる。`AppState::setup_mode()` は `own_pubkey` が `None` かどうかで判定する。使える API と `503`／`409` の条件は [`dashboard/http-api.md#共通`](dashboard/http-api.md#共通)。署名アプリとのペアリングは `AppState.pairing` に 1 つだけ持つ（[`signer.md#ペアリングpairing`](signer.md#ペアリングpairing)）。

### UI と API の分離（`[dashboard].ui`）

`dashboard::router` は静的ファイルのルート（下記「[静的ファイルの配信](#静的ファイルの配信srcdashboardassetsrs)」）とログインリンクの受け口 `/login` を `ui_router()` にまとめ、`ui = true` のときだけ `/api/*` のルータにマージする。`ui = false` なら `/` などは `404` で、`/api/*` は通常どおり応答する。

### 起動と終了

`up::run` の中で行う（全体の起動順とサーバタスクを止める時機は [`up.md`](up.md)）。

- `[dashboard].listen` に bind する。失敗すると `swing up` の起動がエラーで終わる（セットアップモードではポートをずらす。[`up.md#セットアップモードでのポートの調整`](up.md#セットアップモードでのポートの調整)）。ループバック以外で待ち受けたときの warn は [`dashboard/security.md#リバースプロキシ経由での公開`](dashboard/security.md#リバースプロキシ経由での公開)。
- `dashboard::serve` を別タスクとして `tokio::spawn` し、``dashboard listening; run `swing dashboard open` to log in`` を info で出す。

## 設定（`[dashboard]`）

キー・環境変数・既定値は [`../../swing.example.toml`](../../swing.example.toml)、設定全体の規則は [`config.md`](config.md)。

- `listen`: 待ち受けアドレス（`SocketAddr`。`config::parse_dashboard_listen`）。
- `ui`: `true`（既定）なら Web UI を配信する（上記「UI と API の分離」）。
- `allowed_hosts`: Host ヘッダで追加で許可するホスト名（ポート抜き、大文字小文字を区別しない）。ループバックの名前と `listen` の IP は入れなくても通る（[`dashboard/security.md#ガード`](dashboard/security.md#ガード)）。環境変数はカンマ区切りで、前後の空白を取り除く。`[gateway].hosts` と同じ名前は入れられない（[`gateway.md#設定gateway`](gateway.md#設定gateway)）。
- `public_url`: ブラウザからダッシュボードを開くときのベース URL（`http(s)://host[:port]`。[`config.md#検証`](config.md#検証)。`config::parse_public_url`）。`swing dashboard open` と `swing-tray` が作るログインリンクの頭（`login::request_link`。未設定なら `http://<listen>` で、`listen` が `0.0.0.0`/`::` ならループバックに直す）と、`https://` で始まるときのセッション cookie の `Secure` に使う。待ち受けや Host の検証には関わらない。ダッシュボードからは編集できない。
- `gateway`: 保存済みサイトを開くリンクの IPFS Gateway のベース URL。既定は `[kubo].managed = true` なら `http://localhost:<[kubo].gateway_listen のポート>`、そうでなければ `http://localhost:8080`。規則は `public_url` と同じ（`config::parse_dashboard_gateway`）。空文字なら `gateway_url` を出さない（環境変数の空文字は未設定扱いなので、無効にするには TOML で `gateway = ""`）。SWING が設定する Kubo は `127.0.0.1`・`::1` でパス形式の URL に 404 を返す（[`kubo.md#パス形式を返さないホスト`](kubo.md#パス形式を返さないホスト)）ので、そのホストを指すとリンクが開けない。
- `custom_css`: `/custom.css` として配信する CSS ファイルのパス。
- `desktop_page` / `desktop_page_css` / `desktop_banner`: Desktop 画面のリンク集ページ（`/desktop-page.html`）・その CSS（`/desktop-page.css`）・88×31 バナー（`/desktop-banner`）を差し替えるファイルのパス。未設定なら同梱のものを使う。読めないパスや、`desktop_banner` の拡張子が `.png` `.gif` `.jpg` `.jpeg` `.webp` `.svg` 以外なら、同梱版に戻さず `swing up` の起動をエラーで止める。
- `mascots_dir`: ユーザー定義の Desktop マスコットパックを置くディレクトリ（1 サブディレクトリ = 1 パック）。未設定なら同梱の 3 パック（`yureko`・`mochi`・`neko`）だけを配信する。読み込みの規則は [`dashboard/mascot/pack.md#配信`](dashboard/mascot/pack.md#配信srcdashboardmascots)。
- `max_upload`: `POST /api/publish/upload` のリクエストボディの上限。

`desktop_page`・`desktop_page_css`・`desktop_banner`・`mascots_dir` は `ui = true` のときだけ `AppState::new` で 1 回読むので、ファイルの差し替え・追加の反映には `swing up` の再起動が要る。

## タイムアウト（`src/dashboard/mod.rs`）

`tower_http::timeout::TimeoutLayer` を `router()` に掛けている。タイムアウトすると空ボディの `408 Request Timeout` を返す。

- `POST /api/publish/upload`: 30 分（後片付けは [`dashboard/http-api/publish.md#post-apipublishupload`](dashboard/http-api/publish.md#post-apipublishupload)）。
- ほかの全ルート: 120 秒。
- ヘッダ読み取りのタイムアウトは無い（[`dashboard/security.md#既知の弱点`](dashboard/security.md#既知の弱点)）。

## 静的ファイルの配信（`src/dashboard/assets.rs`）

`[dashboard].ui = true` のときだけ配信する（`ui_router()`）。差し替えないファイルは `assets.rs::STATIC_ASSETS`（パス・Content-Type・本体の配列）にまとめてあり、`assets::register()` がそのままルートに登録する。`/desktop-page.html`・`/desktop-page.css`・`/desktop-banner` は `DesktopAssets::load` で読んだものを `AppState.desktop` に、`/mascots/*` は `mascots::MascotRegistry` に持つ（読む時機は上記「[設定](#設定dashboard)」）。これらは認証が要る（[`dashboard/security.md#ガード`](dashboard/security.md#ガード)）。リクエストのたびにディスクを読むのは `/custom.css` だけ。

| ルート | Content-Type |
|---|---|
| `GET /` | `text/html; charset=utf-8`（`index.html`） |
| `GET /favicon.svg` `/desktop-icons.svg` | `image/svg+xml` |
| `GET /favicon-32.png` `/apple-touch-icon.png` | `image/png` |
| `GET /*.css` | `text/css; charset=utf-8`。一覧は `STATIC_ASSETS` が正本 |
| `GET /*.js` | `text/javascript; charset=utf-8`。一覧は `STATIC_ASSETS` が正本（役割は [`dashboard/web.md#構成`](dashboard/web.md#構成)） |
| `GET /desktop-page.html` `/desktop-page.css` | `text/html; charset=utf-8` / `text/css; charset=utf-8`。リンク集ページとその CSS（ページの契約は [`dashboard/desktop.md#リンク集ページiframe`](dashboard/desktop.md#リンク集ページiframe)） |
| `GET /desktop-banner` | 同梱版は `image/gif`。差し替えると拡張子から決める |
| `GET /mascots/index.json` `/mascots/<id>/manifest.json` | `application/json` |
| `GET /mascots/<id>/<sprite>` | `image/png`・`image/gif`・`image/webp`（拡張子から決める） |
| `GET /fonts/pixelmplus12-regular.woff2` `/fonts/pixelmplus12-bold.woff2` | `font/woff2` |
| `GET /login?code=<code>` | ファイルではなくログインリンクの受け口（`session::login_page`）。`303` で Web UI に送る（[`dashboard/security.md#ブラウザ`](dashboard/security.md#ブラウザ)） |
| `GET /custom.css` | `[dashboard].custom_css` をリクエストのたびに読んで返す（`text/css; charset=utf-8`、`Cache-Control: no-store`）。未設定・読み込み失敗なら空 |
