# ダッシュボード HTTP API（`src/dashboard/api.rs`, `src/dashboard/setup.rs`, `src/dashboard/session.rs`, `src/dashboard/upload.rs`, `src/dashboard/dto.rs`, `src/dashboard/config_dto.rs`）

[`../dashboard.md`](../dashboard.md) の子ページ。ガードと認証は [`security.md`](security.md)、画面側からの使い方は [`views.md`](views.md)（Publish は [`views/publish.md`](views/publish.md)）を参照。各エンドポイントの入出力は [一覧](#エンドポイント一覧)から辿る子ページにある。

## 共通

### 形式とエラー

- すべて JSON。公開鍵は `pubkey`（小文字 hex）と `npub` を併記する。時刻は epoch 秒の整数。無い値は `null`（例外は `GET /api/config` の `display`・`raw`・`options` で、無いときはフィールドごと出ない）。`POST /api/publish/upload` だけ `multipart/form-data` を受ける。
- ハンドラが返すエラーは `{ "error": "<メッセージ>" }` とステータスコード。

| ステータス | 条件 |
|---|---|
| 400 | 入力不正。JSON の構文エラー・必須フィールド欠落・`Content-Type` 不一致も 400（422 にはしない）。`POST /api/publish/upload` に multipart でない `Content-Type` を送ったときは axum の素の 400（本文は JSON ではない） |
| 401 | 認証が通らない（[`security.md#ガード`](security.md#ガード)）。認証の要る `/api/*` は、存在しないパスでも 401 |
| 403 | ガードの Host・`X-Swing-Dashboard`・Origin の検証に通らない |
| 404 | 存在しないルート（空ボディ）。`/api/*` では認証を通った後だけ |
| 405 | ルートはあるがメソッドが違う（空ボディ） |
| 408 | リクエストタイムアウト（空ボディ。[`../dashboard.md#タイムアウトsrcdashboardmodrs`](../dashboard.md#タイムアウトsrcdashboardmodrs)） |
| 409 | publish の多重実行、セットアップ・ペアリング・つなぎ直しを使えない状態、`mirror/add` で Follow Set が上限を超える（各エンドポイント） |
| 413 | ボディが大きすぎる |
| 422 | publish の NIP-05・ドットファイル・サイズの確認の `require` 失敗だけ |
| 500 | ファイルの読み書きなど内部の失敗。詳細はログにだけ出し、本文は常に `{"error": "internal error; see the swing log for details"}` |
| 502 | relay・Kubo・Nostr 発行・署名アプリの失敗 |
| 503 | agent の未準備・セットアップモード・relay を引く API の同時実行数の上限（下記） |

### agent の準備状態とセットアップモード

API は `swing up` の寿命で動き続ける（[`../up.md`](../up.md)）。

| エンドポイント | 使えないとき |
|---|---|
| relay か Kubo を使うもの（`/api/sites`・`/api/status`・`/api/mirror`・`/api/mirror/add`・`/api/mirror/remove`・`/api/webring`・`/api/replicas`・`/api/publish/sites`・`/api/publish/previous-files`・`/api/publish/upload`） | agent が relay と Kubo を渡すまで（起動時の突き合わせの後。[`../agent.md#全体の流れ`](../agent.md#全体の流れ)）と、agent が落ちてから次に渡すまで（[`../dashboard.md`](../dashboard.md)）は 503 `{"error": "agent is not ready"}`。セットアップモード（[`../up.md#セットアップモード鍵未設定`](../up.md#セットアップモード鍵未設定)）の間は常に 503 `{"error": "agent is not configured"}` |
| `/api/overview`・`/api/activity`・`/api/stats`・`/api/config`・`/api/shutdown`・`/api/restart`・`/api/login`・`/api/identity`・`/api/login-code`・`/api/token/rotate` | 無い（常に応答する） |
| `/api/setup` | セットアップモードでなければ 409（ほかの 409 は [`http-api/config.md`](http-api/config.md#post-apisetup)） |
| `/api/setup/signer` | セットアップモードでも署名アプリを使っている間でもなければ 409 |
| `/api/signer/reconnect` | 署名アプリを使っていなければ 409 |

`/api/publish/upload` の判定の順は [`http-api/publish.md`](http-api/publish.md#post-apipublishupload)。署名を伴う API（`/api/mirror/add`・`/api/mirror/remove`・`/api/publish/upload`）は、NIP-46 の署名アプリを使っていると署名アプリの返事を待ち、署名できなければ 502 になる（待ち時間とオフラインのときの扱いは [`../signer.md`](../signer.md)）。

### 件数と負荷

- `keys`（mirror add/remove）・`root`（webring）・`key`（replicas）は 1 リクエストあたり最大 100 件、超えると 400。
- relay を引く GET の API（`/api/sites`・`/api/mirror`・`/api/webring`・`/api/replicas`・`/api/publish/sites`・`/api/publish/previous-files`）はサーバ側でキャッシュしない。同時に relay を引けるのはこれらを合わせて 4 本までで、空きを最大 20 秒待っても取れなければ 503 `{"error": "too many relay queries are running; try again later"}`。`/api/publish/previous-files` は relay から取る間だけ枠を使い、その後の Kubo での一覧では使わない。
- `webring`・`replicas` の判定の順は、件数の上限（400）→ 空きを取る（503）→ agent の準備（503）→ `root`・`key` の解析（400）。`mirror/add`・`mirror/remove` は件数（空も含む）と解析（400）→ agent の準備（503）の順で、空きは取らない。レート制限は無い。
- API を叩く CLI サブコマンドの一覧は [`../cli.md#共通`](../cli.md#共通)。

## 既知の性質

- publish の NIP-05 検証の SSRF 対策と、`nip05.detail` がエラー時に粗い分類（`unreachable`/`timeout`/`invalid_response`）だけになる規則は [`../nip05.md`](../nip05.md) を参照。

## エンドポイント一覧

| メソッド | パス | 内容 | 詳細 |
|---|---|---|---|
| GET | `/api/overview` | バージョン・鍵・relay・署名の方法などの概要 | [status.md](http-api/status.md#get-apioverview) |
| GET | `/api/activity` | 保存・publish・レプリカ報告の最新時刻（更新の確認用） | [status.md](http-api/status.md#get-apiactivity) |
| GET | `/api/stats` | リソース使用量のサンプル | [status.md](http-api/status.md#get-apistats) |
| GET | `/api/sites` | ミラー対象のサイト一覧（`swing sites` と同じ集計） | [status.md](http-api/status.md#get-apisites) |
| GET | `/api/status` | 保存済みの版の健全性と実容量（`swing status` と同じ集計） | [status.md](http-api/status.md#get-apistatus) |
| GET | `/api/mirror` | Follow Set の一覧 | [nostr.md](http-api/nostr.md#get-apimirror) |
| POST | `/api/mirror/add`・`/api/mirror/remove` | Follow Set への追加・削除 | [nostr.md](http-api/nostr.md#post-apimirroradd-post-apimirrorremove) |
| GET | `/api/webring` | フォローのグラフ（`swing webring` と同じ） | [nostr.md](http-api/nostr.md#get-apiwebringrootkeydepthn) |
| GET | `/api/replicas` | レプリカ報告の集計（`swing replicas` と同じ） | [nostr.md](http-api/nostr.md#get-apireplicaskeykey) |
| POST | `/api/publish/upload` | フォルダをアップロードして publish | [publish.md](http-api/publish.md#post-apipublishupload) |
| GET | `/api/publish/sites` | 自分が公開したサイトの一覧 | [publish.md](http-api/publish.md#get-apipublishsites) |
| GET | `/api/publish/previous-files` | 自分のサイトの最新版に入っているファイルの一覧（増えたファイルの確認用） | [publish.md](http-api/publish.md#get-apipublishprevious-filessited) |
| GET | `/api/config` | 設定の一覧 | [config.md](http-api/config.md#get-apiconfig) |
| PUT | `/api/config` | 設定ファイルの書き換え | [config.md](http-api/config.md#put-apiconfig) |
| POST | `/api/setup` | 初回セットアップ（鍵と初期設定の保存） | [config.md](http-api/config.md#post-apisetup) |
| POST | `/api/setup/signer` | 署名アプリとのペアリングを始める | [config.md](http-api/config.md#post-apisetupsigner) |
| GET | `/api/setup/signer` | ペアリングの状態 | [config.md](http-api/config.md#get-apisetupsigner) |
| POST | `/api/signer/reconnect` | 署名アプリのつなぎ直しを保存して再起動 | [config.md](http-api/config.md#post-apisignerreconnect) |
| POST | `/api/shutdown`・`/api/restart` | `swing up` の停止・プロセス内再起動 | [session.md](http-api/session.md#post-apishutdown-post-apirestart) |
| POST | `/api/login-code` | 使い捨てのログインコードの発行 | [session.md](http-api/session.md#post-apilogin-code) |
| POST | `/api/identity` | 相手が `swing up` であることの確認（認証なし） | [session.md](http-api/session.md#post-apiidentity) |
| POST | `/api/login` | ログインコードをセッション cookie に交換（認証なし） | [session.md](http-api/session.md#post-apilogin) |
| POST | `/api/token/rotate` | トークンの作り直し | [session.md](http-api/session.md#post-apitokenrotate) |
