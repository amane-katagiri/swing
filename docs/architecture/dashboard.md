# ダッシュボード（src/dashboard/, web/）

[`architecture.md`](../architecture.md) の一部。agent 全体のループとシグナル・終了処理は [`agent.md`](agent.md)、Docker での公開は [`docker.md`](docker.md)。詳細は役割ごとに分けている:

- [`dashboard/http-api.md`](dashboard/http-api.md) — HTTP API の入出力
- [`dashboard/web.md`](dashboard/web.md) — 画面・フロントエンド（`web/*.js`）とCSSカスタマイズ

## 概要

HTTP サーバー（axum 0.8）で、ダッシュボードのブラウザ向け管理画面（`[dashboard].ui = true` のとき）と、CLI の一部（`status`・`mirror add`・`mirror remove`・`stop`。[`../architecture/cli.md`](cli.md)）が叩く制御 API（`/api/*`、常に有効）の両方を兼ねる。専用のサブコマンドは無く、`swing up`（`src/up.rs::run`）が起動する。API の寿命は `swing up` プロセスそのものと同じで、Kubo や mirror-agent が落ちて再起動している間も `/api/overview`・`/api/config`・`/api/shutdown`・`/api/restart` は動き続ける。

- relay 接続（`Arc<RelayClient>`）と Kubo クライアント（`IpfsClient`、`Clone`）は agent が接続・確定させたものを `AppState::set_ready` で受け取り、`tokio::sync::RwLock<Option<...>>` に保持する。agent がまだ relay に接続していない、または Kubo の URL を確定していない間（起動直後、または agent が落ちて再起動待ちの間）は `None` で、agent が `run_until` を抜けるときに `set_not_ready` で `None` に戻す。
  - relay・ipfs を使うエンドポイント（`/api/sites`・`/api/status`・`/api/mirror`・`/api/mirror/add`・`/api/mirror/remove`・`/api/webring`・`/api/replicas`・`/api/publish/sites`・`/api/publish/upload`）は `None` の間 `503 Service Unavailable`、body `{"error": "agent is not ready"}` を返す（`ApiError::NotReady`）。
  - `/api/overview`・`/api/config`・`/api/shutdown`・`/api/restart` は agent の準備状態に関わらず常に応答する。
- 保存状態は agent の `Mutex<State>` には触れず、CLI の各サブコマンドと同じく `state.json` をディスクから読み直す（`mirror::collect_sites`・`health::collect_status` など）。
- `mirror add` / `mirror remove` が relay に受理されると `tokio::sync::Notify` で agent の待ち受けループに知らせ、poll tick と同じ `poll_once`（sweep → Follow Set の再取得 → レプリカ報告の同期）をその場で実行させる。
- 自分の公開鍵（`own_pubkey`）は起動時に 1 回だけ秘密鍵から求めて保持する（リクエストのたびにパースし直さない）。

### UI と API の分離（`[dashboard].ui`）

`AppState.desktop: Option<DesktopAssets>` は `[dashboard].ui = true` のときだけ `DesktopAssets::load` で読み込む。`ui = false` なら `None` のままで、Desktop 画面用のハンドラ（`/desktop-page.html` など）は呼ばれない構成（ルート自体を登録しない）なので `expect` で守っている（`assets::desktop`）。

`dashboard::router` は `[dashboard].ui` に応じてルーティングを分ける: 静的ファイルのルート（`/`・`/style.css`・`/*.js`・`/desktop-*`・`/fonts/*`・`/custom.css`）を `ui_router()` にまとめ、`ui = true` のときだけ `/api/*` のルータにマージする。`/api/*` は `ui` の値に関わらず常に登録する。`ui = false` のとき `/` などは `404`、`/api/*` は通常どおり応答する。

### 起動

`up::run`（`src/up.rs`）の中で行う。全体の起動順は [`up.md`](up.md) を参照。

1. `swing.lock` の取得・シグナルハンドラの設定の後、`<state_dir>/upload/` を掃除（`dashboard::cleanup_upload_dir`）する。
2. `dashboard::AppState::new` を作る（relay・ipfs はまだ `None`）。`AppState::new` 自体の失敗（秘密鍵パース、`ui = true` のときの `DesktopAssets::load` 失敗）は `swing up` の起動失敗として伝播する。
3. `[dashboard].listen` に `TcpListener::bind` する。bind に失敗すると `swing up` の起動自体がエラーで終了する。bind したアドレスがループバック（`127.0.0.1`/`::1`）以外、または `allowed_hosts` が空でなければ、認証が無いことを `tracing::warn` で警告する。
4. `dashboard::serve` を別タスクとして `tokio::spawn` する。この時点でダッシュボードは応答するが、relay・ipfs を使うエンドポイントは agent が起動して `set_ready` を呼ぶまで `503` を返す。
5. この後 Kubo（`managed` なら）と mirror-agent（`agent::run_until`）の起動ループに入る。`agent::run_until` は relay 接続と Kubo の URL 確定が終わった時点で `dashboard.set_ready(relay, ipfs)` を呼び、`run_until` を抜けるとき（エラーでも正常終了でも）`dashboard.set_not_ready()` を呼ぶ。

### 終了

`up::run` がシグナル（SIGINT/SIGTERM）または `/api/shutdown` `/api/restart`（`ExitRequest`）でシャットダウンを開始すると、Kubo・agent 側の終了処理（[`agent.md`](agent.md#シグナルと終了)、[`up.md`](up.md)）と並行して、`up::run` がダッシュボードの `oneshot::Sender` に送り、サーバタスクの `JoinHandle` を最大 5 秒（`DASHBOARD_SHUTDOWN_TIMEOUT`）待つ。超えたら warn ログを出して待つのをやめ、残す。agent 側の終了自体はダッシュボードを止めない（agent は `set_not_ready` を呼ぶだけで、API サーバ自体は `swing up` プロセスが終わるまで動き続ける）。

## 設定（`[dashboard]`）

キー・環境変数・既定値は [`architecture.md`](../architecture.md#設定と環境変数) の設定サンプルを参照。

- `listen`: 待ち受けアドレス。`SocketAddr` としてパースする（`config::parse_dashboard_listen`）。Web UI だけを止めたい場合は `ui = false` を使う。
- `ui`: `true`（既定）なら静的な Web UI（`/`・`/style.css`・`/*.js`・`/desktop-*`・`/fonts/*`・`/custom.css`）を配信する。`false` なら配信せず（ルート自体を登録しない）、`/api/*` の制御 API だけを残す。
- `allowed_hosts`: Host ヘッダで追加で許可するホスト名（ポート抜き、大文字小文字を区別しない）。環境変数はカンマ区切りで、前後の空白を取り除く。
- `gateway`: 保存済みサイトを開くリンクの IPFS Gateway のベース URL。空文字なら `gateway_url` を出さない。環境変数の空文字は未設定として扱うので、無効にするには TOML の `gateway = ""` を使う。
- `custom_css`: `/custom.css` として配信する CSS ファイルのパス。未設定か読めなければ `/custom.css` は空の 200 を返す。
- `desktop_page` / `desktop_page_css` / `desktop_banner`: Desktop 画面のリンク集ページ（`/desktop-page.html`）・その CSS（`/desktop-page.css`）・88×31 バナー（`/desktop-banner`）を差し替えるファイルのパス。未設定なら同梱のものを使う。`custom_css` と違い**起動時に 1 回だけ読んでメモリに載せる**ので、差し替えの反映には agent の再起動が要る。読めないパスを指定した場合は同梱版へのフォールバックはせず、agent の起動をエラーで止める。`desktop_banner` の Content-Type は拡張子から決める（`.png` `.gif` `.jpg` `.jpeg` `.webp` `.svg` のみ。それ以外は起動時エラー）。
- `max_upload`: `POST /api/publish/upload` のリクエストボディ上限。`0` は設定エラー。

## ガード（`src/dashboard/guard.rs`）

全リクエストに axum middleware（`security_middleware`）がかかる。認証は無く、以下の 3 点だけがガード:

1. Host 検証: `Host` ヘッダが無ければ 403。あれば `extract_host`（IPv6 の `[...]` を考慮してポートを外し小文字化）した値が `localhost` / `127.0.0.1` / `::1` か `allowed_hosts` のいずれかでなければ 403。`allowed_hosts` を広げてループバック以外からアクセスできるようにする構成はサポート対象外。`[dashboard].listen` を未指定アドレス（`0.0.0.0` / `::`）で bind している構成では、CLI（`src/api_client.rs::ApiClient`）は接続先アドレスと `Host` ヘッダの両方をループバックの同じポートへ正規化してから送る（`0.0.0.0:8082` をそのまま `Host` に送るとこの検証に落ちるため）。
2. 書き込み系（GET/HEAD 以外）はさらに: `X-Swing-Dashboard: 1` ヘッダが無ければ 403（CORS ヘッダは一切返さない）。`Origin` ヘッダがあれば、その authority（スキームを外し末尾の `/` を削っただけ）が `Host` ヘッダと大文字小文字を無視して一致しなければ 403。
3. レスポンスヘッダ（成功・失敗どちらにも付く）: `X-Content-Type-Options: nosniff`、`Content-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'self'`、`Referrer-Policy: no-referrer`、`X-Frame-Options: SAMEORIGIN`。Desktop 画面のリンク集ページを同一オリジンの iframe で入れ子にするため、自分自身からの埋め込みだけを許している（他オリジンからの埋め込みは従来どおり不可）。`Cache-Control: no-store` は `/api/` 配下だけ middleware が付ける（`/custom.css` はハンドラ自身が付け、`/`・`/style.css`・ES module には付かない）。
4. 秘密鍵: `Config` に `Serialize` を実装しないことで、`/api/config` を含めどの DTO にも秘密鍵の値が現れない（詳しくは下記「秘密鍵を出さない仕組み」）。

## タイムアウト（`src/dashboard/mod.rs`）

`tower_http::timeout::TimeoutLayer` を `router()` に掛けている。タイムアウトすると空ボディの `408 Request Timeout` を返す。

- `POST /api/publish/upload` 以外の全ルート: 120 秒。
- `POST /api/publish/upload`: 30 分。
- ヘッダー読み取り自体のタイムアウトは設定していない（`axum::serve` を使っている都合）。

## 静的ファイルの配信（`src/dashboard/assets.rs`）

`[dashboard].ui = true` のときだけ配信する（`ui_router()`。上の「UI と API の分離」を参照）。`web/` 配下の全ファイルをビルド時に `include_str!` でバイナリに埋め込む（実行時にファイルを探しに行かない）。ファイルを 1 つ追加するときは `assets.rs` に定数+ハンドラを、`mod.rs` の `ui_router()` にルートを 1 対 1 で足す。

例外は `/desktop-page.html`・`/desktop-page.css`・`/desktop-banner` の 3 つで、`[dashboard]` にパスが設定されていればそのファイルを起動時（`AppState::new` → `DesktopAssets::load`）に読んで `AppState.desktop` に持ち、以降はそこから配信する（設定が無ければ同梱のものを `Bytes::from_static` で持つ）。リクエストのたびにディスクを見るのは `/custom.css` だけ。

| ルート | Content-Type |
|---|---|
| `GET /` | `text/html; charset=utf-8`（`index.html`） |
| `GET /style.css` `/desktop.css` | `text/css; charset=utf-8` |
| `GET /boot.js` `/app.js` `/graph.js` `/storage.js` `/i18n.js` `/util.js` `/ui.js` `/sites.js` `/webring.js` `/publish.js` `/settings.js` `/desktop.js` | `text/javascript; charset=utf-8` |
| `GET /desktop-page.html` `/desktop-page.css` | `text/html; charset=utf-8` / `text/css; charset=utf-8`。Desktop 画面の iframe に入るリンク集ページとその CSS |
| `GET /desktop-frame.css` | `text/css; charset=utf-8`。同じ iframe に `desktop.js` が差し込む窓側の CSS（スクロールバー）。差し替え対象ではない |
| `GET /desktop-banner` | 既定は `image/gif`。リンク集ページの 88×31 バナー画像。差し替えられるので拡張子はパスに持たせない |
| `GET /fonts/pixelmplus12-regular.woff2` `/fonts/pixelmplus12-bold.woff2` | `font/woff2`。Desktop 画面の同梱フォント PixelMplus12（400/700、`include_bytes!`）。ライセンスは `web/fonts/LICENSE-PixelMplus.txt` |
| `GET /custom.css` | `[dashboard].custom_css` の中身をリクエストのたびにディスクから読んで返す（`text/css; charset=utf-8`、`Cache-Control: no-store`）。未設定・読み込み失敗なら空文字 |

各 JS ファイルの役割・依存関係は [`dashboard/web.md#構成`](dashboard/web.md#構成) を参照。

## 秘密鍵を出さない仕組み

- `config::Config`（および `NostrConfig`）に `Serialize` を実装していない。DTO は手書きの構造体で、`secret_key` の実値を持つフィールドが型として存在しない。`/api/config` は常に固定文字列 `"(set, hidden)"` を返す。
- `Config` の `Debug` 実装も秘密鍵の値を `<redacted>` にする（ログにも出ない）。
- 表示するのは `npub` / hex 公開鍵のみ。nsec は API のどのエンドポイントにも登場しない。
