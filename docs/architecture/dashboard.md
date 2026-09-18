# ダッシュボード（src/dashboard/, web/）

[`architecture.md`](../architecture.md) の一部。agent 全体のループとシグナル・終了処理は [`agent.md`](agent.md)、Docker での公開は [`docker.md`](docker.md)。詳細は役割ごとに分けている:

- [`dashboard/http-api.md`](dashboard/http-api.md) — HTTP API の入出力
- [`dashboard/web.md`](dashboard/web.md) — 画面・フロントエンド（`web/*.js`）とCSSカスタマイズ

## 概要

`swing agent` のプロセス内で HTTP サーバー（axum 0.8）を立てる、ブラウザ向けの管理画面。専用のサブコマンドは無く、`[dashboard].listen` が `off` でなければ agent 起動時に自動で立ち上がる。

- relay 接続は agent が使っているものと同じ `Arc<RelayClient>` を共有する。
- 保存状態は agent の `Mutex<State>` には触れず、CLI の各サブコマンドと同じく `state.json` をディスクから読み直す（`mirror::collect_sites`・`health::collect_status` など）。
- `mirror add` / `mirror remove` が relay に受理されると `tokio::sync::Notify` で agent の待ち受けループに知らせ、poll tick と同じ `poll_once`（sweep → Follow Set の再取得 → レプリカ報告の同期）をその場で実行させる。
- 自分の公開鍵（`own_pubkey`）は起動時に 1 回だけ秘密鍵から求めて保持する（リクエストのたびにパースし直さない）。

### 起動

`agent::run` の中で行う。全体のライフサイクル（reconcile・SIGINT/SIGTERM・watchdog）は [`agent.md`](agent.md#全体の流れ) を参照。

1. relay 接続・state 読み込みの後、`[dashboard].listen` が `Off` でなければ `TcpListener::bind` する。bind に失敗すると agent の起動自体がエラーで終了する。bind したアドレスがループバック（`127.0.0.1`/`::1`）以外、または `allowed_hosts` が空でなければ、認証が無いことを `tracing::warn` で警告する。
2. 起動時の突き合わせ（`reconcile`）の後、bind できていれば `<state_dir>/upload/` を掃除（`dashboard::cleanup_upload_dir`）してから `dashboard::AppState` を作り、`dashboard::serve` を別タスクとして `tokio::spawn` する。`AppState::new` 自体の失敗（秘密鍵パース）も agent の起動失敗として伝播する。

### 終了

SIGINT・SIGTERM のどちらでも同じように終了する。シグナルの受け方・`race_with_shutdown`・watchdog の詳細は [`agent.md`](agent.md#シグナルと終了) を参照。ダッシュボード固有の部分:

- `agent::shutdown_dashboard` はダッシュボードの `oneshot::Sender` に送った後、サーバタスクの `JoinHandle` を最大 5 秒待つ。超えたら warn ログを出して待つのをやめ、relay を切断してループを抜ける。
- ダッシュボードのサーバタスクが（panic などで）シャットダウン開始前に終了した場合は error ログを出すだけで、agent 本体は止めない（relay 通知・poll・mirror 保存は続く）。
- `[dashboard].listen = off` の場合、bind もタスク起動もせず、agent の動作は変わらない。

## 設定（`[dashboard]`）

キー・環境変数・既定値は [`architecture.md`](../architecture.md#設定と環境変数) の設定サンプルを参照。

- `listen`: 待ち受けアドレス。`off`（大文字小文字を区別しない）で無効。それ以外は `SocketAddr` としてパースできなければ設定エラー。
- `allowed_hosts`: Host ヘッダで追加で許可するホスト名（ポート抜き、大文字小文字を区別しない）。環境変数はカンマ区切りで、前後の空白を取り除く。
- `gateway`: 保存済みサイトを開くリンクの IPFS Gateway のベース URL。空文字なら `gateway_url` を出さない。環境変数の空文字は未設定として扱うので、無効にするには TOML の `gateway = ""` を使う。
- `custom_css`: `/custom.css` として配信する CSS ファイルのパス。未設定か読めなければ `/custom.css` は空の 200 を返す。
- `max_upload`: `POST /api/publish/upload` のリクエストボディ上限。`0` は設定エラー。

## ガード（`src/dashboard/guard.rs`）

全リクエストに axum middleware（`security_middleware`）がかかる。認証は無く、以下の 3 点だけがガード:

1. Host 検証: `Host` ヘッダが無ければ 403。あれば `extract_host`（IPv6 の `[...]` を考慮してポートを外し小文字化）した値が `localhost` / `127.0.0.1` / `::1` か `allowed_hosts` のいずれかでなければ 403。`allowed_hosts` を広げてループバック以外からアクセスできるようにする構成はサポート対象外。
2. 書き込み系（GET/HEAD 以外）はさらに: `X-Swing-Dashboard: 1` ヘッダが無ければ 403（CORS ヘッダは一切返さない）。`Origin` ヘッダがあれば、その authority（スキームを外し末尾の `/` を削っただけ）が `Host` ヘッダと大文字小文字を無視して一致しなければ 403。
3. レスポンスヘッダ（成功・失敗どちらにも付く）: `X-Content-Type-Options: nosniff`、`Content-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'none'`、`Referrer-Policy: no-referrer`、`X-Frame-Options: DENY`。`Cache-Control: no-store` は `/api/` 配下だけ middleware が付ける（`/custom.css` はハンドラ自身が付け、`/`・`/style.css`・ES module には付かない）。
4. 秘密鍵: `Config` に `Serialize` を実装しないことで、`/api/config` を含めどの DTO にも秘密鍵の値が現れない（詳しくは下記「秘密鍵を出さない仕組み」）。

## タイムアウト（`src/dashboard/mod.rs`）

`tower_http::timeout::TimeoutLayer` を `router()` に掛けている。タイムアウトすると空ボディの `408 Request Timeout` を返す。

- `POST /api/publish/upload` 以外の全ルート: 120 秒。
- `POST /api/publish/upload`: 30 分。
- ヘッダー読み取り自体のタイムアウトは設定していない（`axum::serve` を使っている都合）。

## 静的ファイルの配信（`src/dashboard/assets.rs`）

`web/` 配下の全ファイルをビルド時に `include_str!` でバイナリに埋め込む（実行時にファイルを探しに行かない）。ファイルを 1 つ追加するときは `assets.rs` に定数+ハンドラを、`mod.rs` の router にルートを 1 対 1 で足す。

| ルート | Content-Type |
|---|---|
| `GET /` | `text/html; charset=utf-8`（`index.html`） |
| `GET /style.css` | `text/css; charset=utf-8` |
| `GET /boot.js` `/app.js` `/graph.js` `/storage.js` `/i18n.js` `/util.js` `/ui.js` `/sites.js` `/webring.js` `/publish.js` `/settings.js` | `text/javascript; charset=utf-8` |
| `GET /custom.css` | `[dashboard].custom_css` の中身をリクエストのたびにディスクから読んで返す（`text/css; charset=utf-8`、`Cache-Control: no-store`）。未設定・読み込み失敗なら空文字 |

各 JS ファイルの役割・依存関係は [`dashboard/web.md#構成`](dashboard/web.md#構成) を参照。

## 秘密鍵を出さない仕組み

- `config::Config`（および `NostrConfig`）に `Serialize` を実装していない。DTO は手書きの構造体で、`secret_key` の実値を持つフィールドが型として存在しない。`/api/config` は常に固定文字列 `"(set, hidden)"` を返す。
- `Config` の `Debug` 実装も秘密鍵の値を `<redacted>` にする（ログにも出ない）。
- 表示するのは `npub` / hex 公開鍵のみ。nsec は API のどのエンドポイントにも登場しない。
