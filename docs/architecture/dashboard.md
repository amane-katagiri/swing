# ダッシュボード（src/dashboard/）

[`../architecture.md`](../architecture.md) の一部。agent 全体のループは [`agent.md`](agent.md)、シグナル・終了処理と起動順は [`up.md`](up.md)、Docker での公開は [`docker.md`](docker.md)。詳細は役割ごとに分けている:

- [`dashboard/http-api.md`](dashboard/http-api.md) — HTTP API の入出力
- [`dashboard/web.md`](dashboard/web.md) — 画面・フロントエンド（`web/*.js`）とCSSカスタマイズ
- [`dashboard/desktop.md`](dashboard/desktop.md) — Desktop 画面（`web/desktop*`。`web.md` と並ぶ子ページ）
  - [`dashboard/mascot.md`](dashboard/mascot.md) — Desktop 画面のマスコット（`web/desktop-mascot*`・`web/mascots/`。`desktop.md` の子ページ）

## 概要

HTTP サーバー（axum 0.8）で、ダッシュボードのブラウザ向け管理画面（`[dashboard].ui = true` のとき）と、CLI の一部（一覧は [`cli.md#共通`](cli.md#共通)）が叩く制御 API（`/api/*`、常に有効）の両方を兼ねる。`/api/*` は認証が要る（下記「認証」）。専用のサブコマンドは無く、`swing up`（`src/up.rs::run`）が起動する（寿命と起動・終了順は [`up.md`](up.md)）。

- relay 接続（`Arc<RelayClient>`）と Kubo クライアント（`IpfsClient`、`Clone`）は agent から `AppState::set_ready` で受け取り、`tokio::sync::RwLock<Option<...>>` に保持する。`set_ready` の前と `set_not_ready` の後は `None`（呼ぶ時機は [`agent.md#全体の流れ`](agent.md#全体の流れ) と [`agent.md#シグナルと終了`](agent.md#シグナルと終了)）。その間とセットアップモードの間に `503` を返す API は [`dashboard/http-api.md#共通`](dashboard/http-api.md#共通)。
- 保存状態は agent の `Mutex<State>` には触れず、CLI の各サブコマンドと同じく `state.json` をディスクから読み直す（`mirror::collect_sites`・`health::collect_status` など）。
- `mirror add` / `mirror remove` が relay に受理されると `tokio::sync::Notify` で agent の待ち受けループに知らせ、poll tick と同じ `poll_once`（sweep → Follow Set の再取得 → レプリカ報告の同期）をその場で実行させる。
- 署名の方法（`AppState.signer: Option<signer::Signer>`。秘密鍵か NIP-46 の署名アプリ。[`signer.md`](signer.md)）は `up::run` が決めて渡す（作り直す時機は [`signer.md#signer`](signer.md#signer)）。自分の公開鍵（`AppState.own_pubkey: Option<PublicKey>`）はそこから求めて保持する（署名アプリの場合もリクエストは送らない）。

### セットアップモードと `AppState::setup_mode`

セットアップモード（条件と起動の流れは [`up.md#セットアップモード鍵未設定`](up.md#セットアップモード鍵未設定)、`Signer::load` の規則は [`signer.md`](signer.md)）では `AppState::new` に `signer: None` が渡り、`own_pubkey` も `None` になる。`AppState::setup_mode()` は `own_pubkey` が `None` かどうかで判定する。このモードで使える API と `503`／`409` の条件は [`dashboard/http-api.md#共通`](dashboard/http-api.md#共通) を参照。ペアリング（署名アプリとの接続）は `AppState.pairing` に 1 つだけ持つ（[`signer.md#ペアリングpairing`](signer.md#ペアリングpairing)）。画面側の扱いは [`dashboard/web.md`](dashboard/web.md)。

### UI と API の分離（`[dashboard].ui`）

`dashboard::router` は `[dashboard].ui` に応じてルーティングを分ける: 静的ファイルのルート（`/`・`/favicon.svg`・`/favicon-32.png`・`/apple-touch-icon.png`・`/*.css`・`/*.js`・`/desktop-*`・`/mascots/*`・`/fonts/*`・`/custom.css`）とログインリンクの受け口 `/login` を `ui_router()` にまとめ、`ui = true` のときだけ `/api/*` のルータにマージする。`/api/*` は `ui` の値に関わらず常に登録する。`ui = false` のとき `/` などは `404`、`/api/*` は通常どおり応答する。

### 起動と終了

`up::run`（`src/up.rs`）の中で行う。全体の起動順・サーバタスクを止める時機は [`up.md`](up.md) を参照。ダッシュボード固有の点は次の 2 つ:

- `[dashboard].listen` に `TcpListener::bind` する。bind に失敗すると `swing up` の起動自体がエラーで終了する（セットアップモードではポートをずらして bind し、そのアドレスを Kubo の gateway のアドレスと一緒に設定ファイルへ書き込む。[`up.md#セットアップモードでのポートの調整`](up.md#セットアップモードでのポートの調整)）。`[dashboard].listen` の IP が `is_loopback()`（`127.0.0.0/8` と `::1`）でなければ、前段に TLS を終端する HTTP リバースプロキシが無いとログインコードとセッション cookie が平文で流れることを `tracing::warn` で警告する（判定は待ち受けアドレスだけで、`allowed_hosts` は見ない）。サーバが動き出すと `dashboard listening; run `swing dashboard open` to log in` を info で出す。
- `dashboard::serve` を別タスクとして `tokio::spawn` する。

## 設定（`[dashboard]`）

キー・環境変数・既定値は [`../architecture.md`](../architecture.md#設定と環境変数) の設定カタログ・[`../../swing.example.toml`](../../swing.example.toml) を参照。

- `listen`: 待ち受けアドレス。`SocketAddr` としてパースする（`config::parse_dashboard_listen`）。Web UI だけを止めたい場合は `ui = false` を使う。
- `ui`: `true`（既定）なら静的な Web UI を配信し、`false` なら配信しない。挙動の詳細は上記「UI と API の分離」を参照。
- `allowed_hosts`: Host ヘッダで追加で許可するホスト名（ポート抜き、大文字小文字を区別しない）。環境変数はカンマ区切りで、前後の空白を取り除く。`[gateway].hosts` と同じ名前は入れられない（設定の読み込みがエラーになる。[`gateway.md`](gateway.md#設定)）。
- `public_url`: ブラウザからダッシュボードを開くときのベース URL（`http(s)://host[:port]`。パス・クエリ・`@` は不可、末尾の `/` は取り除く。`config::parse_public_url`）。使い道は 2 つで、`swing dashboard open` と `swing-tray` が作るログインリンクの頭（`login::request_link`。未設定なら `http://<listen>` で、`listen` が未指定アドレス `0.0.0.0`/`::` のときだけループバックに直す）と、`https://` で始まるときのセッション cookie の `Secure`（下記「認証」）。サーバの待ち受けや Host 検証には関わらない。`editable: false`（任意ホストへのログインコード送信防止）。
- `gateway`: 保存済みサイトを開くリンクの IPFS Gateway のベース URL。既定は `[kubo].managed = true` なら `http://localhost:<[kubo].gateway_listen のポート>`、そうでなければ `http://localhost:8080`（ホストを `localhost` にするのは、Kubo がサブドメイン形式の `<cid>.ipfs.localhost` に移してサイトごとに別オリジンにするため）。`public_url` と同じ規則（`http(s)://host[:port]`、パス・クエリ・`@` は不可、末尾の `/` は取り除く。`config::parse_dashboard_gateway`）で検証し、違反はエラー。空文字なら `gateway_url` を出さない。環境変数の空文字は未設定として扱うので、無効にするには TOML の `gateway = ""` を使う。
- `custom_css`: `/custom.css` として配信する CSS ファイルのパス（下記「静的ファイルの配信」）。
- `desktop_page` / `desktop_page_css` / `desktop_banner`: Desktop 画面のリンク集ページ（`/desktop-page.html`）・その CSS（`/desktop-page.css`）・88×31 バナー（`/desktop-banner`）を差し替えるファイルのパス。未設定なら同梱のものを使う。読み込みの時機は下記「静的ファイルの配信」で、差し替えの反映には `swing up` の再起動が要る。読めないパスや、`desktop_banner` の拡張子が対象外（`.png` `.gif` `.jpg` `.jpeg` `.webp` `.svg` 以外）のときは同梱版へのフォールバックはせず、`swing up` の起動をエラーで止める。`desktop_banner` の Content-Type は拡張子から決める。
- `mascots_dir`: ユーザー定義の Desktop マスコットパックを置くディレクトリ（1 サブディレクトリ = 1 パック。パック形式は [`dashboard/mascot.md#パック形式-1`](dashboard/mascot.md#パック形式-1)）。`ui = true` のときだけ、`AppState::new`（`DesktopAssets::load` と並んで）で起動時に 1 回スキャンする。未設定なら同梱の 2 パック（`mochi`・`neko`）だけを配信する。差し替え・追加の反映には `swing up` の再起動が要る。読み込みの規則は下記「静的ファイルの配信」。
- `max_upload`: `POST /api/publish/upload` のリクエストボディ上限。

## ガード（`src/dashboard/guard.rs`）

全リクエストに axum middleware（`security_middleware`）がかかる。以下の順に判定する（判定で返すエラーは `{"error": ...}` の JSON）:

1. Host 検証: `Host` ヘッダが無ければ 403。あれば `host::extract_host`（IPv6 の `[...]` を考慮してポートを外し小文字化。`]` の後ろが `:<数字>` 以外なら、ヘッダ全体をホスト名として扱うのでどれにも一致しない。内蔵 gateway と共有）した値が `localhost` / `127.0.0.1` / `::1` か `allowed_hosts` のいずれかでなければ 403。ループバック以外の名前で開くのは、下記「リバースプロキシ経由での公開」の構成で公開ホスト名を `allowed_hosts` に入れる場合に限ってサポートする。CLI が未指定アドレス（`0.0.0.0` / `::`）の `listen` に接続するときの扱いは [`cli.md#共通`](cli.md#共通)。
2. 書き込み系（GET/HEAD 以外）はさらに: `X-Swing-Dashboard: 1` ヘッダが無ければ 403（CORS ヘッダは一切返さない）。`Origin` ヘッダがあれば、その authority（スキームを外し末尾の `/` を削っただけ）が `Host` ヘッダと大文字小文字を無視して一致しなければ 403。
3. 認証: パスが `/api/` で始まり、`/api/login`・`/api/identity` 以外なら `guard::authorized` を通らないと 401 `{"error": "missing or invalid dashboard token or session"}`。`Authorization: Bearer <token>` が付いていればそれだけで判定し（`auth::token_matches`）、無ければセッション cookie を見る（下記「認証」）。静的ファイル（`/`・`*.js`・`*.css` など）と `/login` は認証なしで返す。

どのレスポンス（成功・失敗とも）にも次のヘッダを付ける: `X-Content-Type-Options: nosniff`、`Content-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'self'; base-uri 'none'; form-action 'self'; object-src 'none'`、`Referrer-Policy: no-referrer`、`X-Frame-Options: SAMEORIGIN`。Desktop 画面のリンク集ページの同一オリジン iframe 埋め込みのため、自分自身からの埋め込みだけを許可し、他オリジンからの埋め込みは不可（クリックジャッキング耐性）。`Cache-Control: no-store` は `/api/` 配下だけ middleware が付ける（`/custom.css` はハンドラ自身が付け、`/`・`/style.css`・ES module には付かない）。

秘密鍵をレスポンスに出さない仕組みは、この middleware ではなく DTO の型自体で担保している（下記「秘密鍵を出さない仕組み」）。

## 認証（`src/auth.rs`, `src/dashboard/session.rs`）

- トークン: `<state_dir>/dashboard.token` に 32 バイトの乱数を hex（64 文字）で 1 行置く。`swing up` が起動時に `auth::load_or_create_token` で読み、無ければ作る（再起動しても同じトークンを使い続ける）。書き込み（`auth::write_new_token`）は `auth::write_private_file` で、同じディレクトリの `<名前>.<16 桁の乱数 hex>.tmp` を `create_new` で作って書き、`sync_all` してから rename する（同時に書く呼び出し同士が一時ファイルを取り合わない。失敗したら一時ファイルを消す）。Unix ではパーミッション `0600` で作る。Windows は `state_dir` の ACL を継承する。メモリ上の値は `AppState` の `std::sync::RwLock<String>` に持つ（`AppState::token` / `set_token`）。`state_dir` 自体を swing が新規作成するとき（`auth::create_private_dir_all`。`auth::write_new_token`・`lock::acquire`・`signer::RemoteSignerFile::save` が共有する）は Unix なら `0700` で作る。すでにあるディレクトリはユーザーが作ったものとみなし、パーミッションは変更しない。
- CLI: `api_client::ApiClient::for_config` がトークンファイルを読み、すべてのリクエストに `Authorization: Bearer <token>` を付ける（ファイルが無ければ付けない）。プロキシの環境変数（`HTTP_PROXY` など）は使わない（`no_proxy`）。トークンを送る前に、相手がそのトークンを知っていることを確かめる（下記「サーバの本人確認」）。
- サーバの本人確認: `swing up` が止まっている間は別のプログラムが `[dashboard].listen` のポートで待ち受けられるので、`ApiClient` はトークン付きの最初のリクエストの前に、32 バイトの乱数を hex にした nonce（64 文字、`auth::new_identity_nonce`）を [`POST /api/identity`](dashboard/http-api.md#post-apiidentity) に送り、返ってきた `proof` が HMAC-SHA256（鍵はトークン、メッセージは `swing-identity:` と小文字の nonce。`auth::identity_proof`）と一致するかを `auth::verify_identity_proof` で確かめる。一致しなければトークンを送らず `ApiClientError::NotSwing` で止める（相手がエンドポイントを持たないときは `Http` エラー）。確認できた結果は `ApiClient` のインスタンスごとに覚え、接続できなかった（`Unreachable`）ときは忘れて次のリクエストで確かめ直す。トークンが無いときは確かめない。
- ブラウザ: 永続トークンはブラウザに渡さない。`swing dashboard open`（と `swing-tray`）が Bearer で `POST /api/login-code` を叩いて使い捨てのログインコード（16 バイトの乱数を hex で 32 文字、有効 5 分、1 回限り、`auth::LoginCodes`。プロセスのメモリにだけ持ち、再起動で消える）をもらい、`<public_url>/login?code=<code>` を開く（上記「設定」の `public_url`）。`GET /login` はコードを消費しない（リンクのプレビューを取りに来たボットにコードを使われないため）。`code` が 1 文字以上の hex なら `303 /#/login/code/<code>` を返し、Web UI がそのコードを [`POST /api/login`](dashboard/http-api.md#post-apilogin) に送って交換する（[`dashboard/web.md`](dashboard/web.md)）。hex でなければ `303 /#/login/invalid`、`code` が無ければ `303 /#/login` を返す。ログイン画面に貼ったコードも同じ `POST /api/login` で交換する。
- セッション cookie: 名前は `swing_session_<port>`（`Host` ヘッダのポート。ポートが無ければ `swing_session`）。値は `<発行時刻（epoch 秒）>.<HMAC-SHA256 の hex>`。HMAC の鍵はトークン、メッセージは用途ラベル `dashboard-session`・`\0`・発行時刻の 10 進表記（`auth::sign_session`）。属性は `HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000`。リクエストの `X-Forwarded-Proto` の先頭の値が `https`（大文字小文字は無視）か、`[dashboard].public_url` が `https://` で始まるときは `Secure` も付ける（`session::served_over_https`）。
- セッションの検証（`auth::verify_session`）: 発行時刻から 30 日（`auth::SESSION_TTL`）以上経ったもの、5 分より先の発行時刻のものは拒否する。期限はブラウザの `Max-Age` ではなくサーバ側で判定する。
- トークンの作り直し: `POST /api/token/rotate` がファイルを書き換えてメモリ上の値も差し替え、未使用のログインコードを捨てる。HMAC の鍵が変わるので、既存のセッション cookie はすべてその場で無効になる。`ApiClient::for_config` は作るたびにファイルを読むので、CLI は次のコマンドから新しいトークンを使う。
- CSRF: 書き込み系は `X-Swing-Dashboard` ヘッダと `Origin` の検証（上記「ガード」）を通す。cookie は `SameSite=Strict`。DNS rebinding は Host 検証で止める。

## リバースプロキシ経由での公開

ダッシュボードは平文の HTTP しか話さない。ループバックの外から使うのは、TLS を終端する HTTP のリバースプロキシの裏に置く構成に限ってサポートする（プロキシ側の設定は README の「[ダッシュボードを外の端末から使う](../../README.md#ダッシュボードを外の端末から使う)」）。この構成で効くのは、上記「ガード」の Host 検証（`allowed_hosts`）と Origin 検証（プロキシが `Host` を書き換えると書き込み系がすべて 403 になる）、「認証」の cookie の `Secure`、「設定」の `public_url`。

### 既知の弱点

- 未認証の相手からの DoS に弱い。ヘッダ読み取りのタイムアウトが無く（`axum::serve` は hyper にタイマーを渡さないので、ヘッダを少しずつ送り続ける接続を切れない。いわゆる Slowloris）、レート制限も同時接続数の制限も無い。HTTP のリバースプロキシはヘッダを受け取りきってから転送するので、プロキシ側のタイムアウトで止まる。平文のまま LAN に直接出す構成や、TCP をそのまま流す前段では止まらない。ボディを少しずつ送る場合はハンドラの中で読むので、`TimeoutLayer` で切れる。止められるのはダッシュボードだけで、agent と Kubo は動き続ける。
- peer のサイトの HTML がダッシュボードと同じホストの別ポートで開かれうる。設定の検証（[`gateway.md`](gateway.md#設定gateway)）が防ぐのは内蔵 gateway の `[gateway].hosts` とダッシュボードのホスト名の重なりだけで、Kubo 自身の gateway（`[kubo].gateway_listen`、既定 `127.0.0.1:8080`）と `[dashboard].gateway` のリンク先は `localhost`・`127.0.0.1` 上で peer の HTML を返す。`localhost` ならサブドメイン形式（`<cid>.ipfs.localhost`）に移るが、`http://127.0.0.1:8080/ipfs/<cid>/` のようなパス形式ではループバックのダッシュボードと同じホストで、ポートだけが違う。サイトはポートを区別しないので、cookie の `SameSite=Strict` はこの経路を止めない。守りになっているのは、cookie が `HttpOnly` でスクリプトから読めないこと、書き込み系が `X-Swing-Dashboard` ヘッダと `Origin` の検証を要し（ポートが違えば別オリジン）、ヘッダ付きのクロスオリジン要求は CORS のプリフライトで止まること、API が CORS ヘッダを返さないので別オリジンから応答を読めないこと。残るのは、同じホストの cookie をそのページが上書き・削除できること（値は HMAC で守られるので偽造はできず、ログアウトさせられる程度）。
- セッションを 1 つだけ取り消す手段が無い。サーバ側にセッションの一覧を持たないので、端末をなくした場合は `swing dashboard rotate-token` で全セッションをまとめて無効にするしかない。それをしなければ、その端末の cookie は発行から 30 日間有効のまま。

## タイムアウト（`src/dashboard/mod.rs`）

`tower_http::timeout::TimeoutLayer` を `router()` に掛けている。タイムアウトすると空ボディの `408 Request Timeout` を返す。

- `POST /api/publish/upload` 以外の全ルート: 120 秒。
- `POST /api/publish/upload`: 30 分。タイムアウトやクライアント切断時の後片付け（展開先ディレクトリの削除）は [`dashboard/http-api.md#post-apipublishupload`](dashboard/http-api.md#post-apipublishupload) を参照。
- ヘッダー読み取り自体のタイムアウトは無い（上記「既知の弱点」）。

## 静的ファイルの配信（`src/dashboard/assets.rs`）

`[dashboard].ui = true` のときだけ配信する（`ui_router()`。上の「UI と API の分離」を参照）。差し替え不要なファイルは `assets.rs::STATIC_ASSETS`（パス・Content-Type・本体の配列）にまとめてあり、`assets::register()` がこれをそのままルートに登録する。

例外は `/desktop-page.html`・`/desktop-page.css`・`/desktop-banner` の 3 つで、`ui = true` のときだけ、`[dashboard]` にパスが設定されていればそのファイルを起動時（`AppState::new` → `DesktopAssets::load`）に 1 回読んで `AppState.desktop` に持ち、以降はそこから配信する（設定が無ければ同梱のものを持つ）。`/mascots/*`（下記）も同じく起動時に 1 回読む例外だが、`assets.rs` ではなく `mascots/` が持つ。リクエストのたびにディスクを見るのは `/custom.css` だけ。

### マスコットのパック配信（`src/dashboard/mascots/`）

`mascots/mod.rs` がパックの読み込み・配信、`mascots/nofollow.rs` がリンクを辿らずにディレクトリとファイルを開く処理（Unix と Windows の実装を `Dir` にまとめる）と上限付きの読み込み（`read_limited`）、`mascots/image.rs` がスプライトのヘッダの読み取りと大きさの検証（`check_sprite`）を持つ。

`/mascots/index.json` と `/mascots/{id}/{file}` は `assets::register()` の対象外で、`ui_router()` に直接ルートを持つ。`AppState::new` が `ui = true` のときだけ `mascots::MascotRegistry::load(config.dashboard.mascots_dir)` を呼び、結果を `AppState.mascots` に持つ（`DesktopAssets::load` と並ぶ起動時 1 回読み込み。設定の再反映には再起動が要る）。

- 同梱の `yureko`・`mochi`・`neko`（`web/mascots/<id>/`、`include_str!`/`include_bytes!` でバイナリに埋め込み）は常に読み込める。
- `mascots_dir` が設定されていれば、次の順に読む。読み飛ばすものは `warn!` でエントリ名（`Debug` 形式でエスケープ）と理由を出す（`swing up` は止めない。`mascots_dir` 自体が無い・読めない場合も同梱パックだけで続行する）。読み飛ばしの `warn!` は 1 回の読み込みで 10 件まで個別に出し、残りは件数だけを 1 行にまとめる。
  1. **候補を集める**: `mascots_dir` 直下のエントリを最大 1024 個まで見る（超えた分は 1 行の `warn!` を出して見ない）。この段階ではファイルを開かず、名前とエントリの種類（リンクを辿らない）だけで次を満たすものを候補にする: 名前（= パック id）が UTF-8 で `^[a-z0-9][a-z0-9-]{0,31}$` に一致し、同梱の id（`yureko`・`mochi`・`neko`）と重ならず、シンボリックリンクではないディレクトリである。
  2. **上限で切る**: 候補を id の昇順に並べ、先頭の 32 個だけを残す。超えた分は中身を読まずに読み飛ばす（候補に残ったパックが後の検証で落ちても、33 個目以降は繰り上がらない）。
  3. **各パックを読む**: 残った候補ごとに次をすべて満たさなければ読み飛ばす。
     - パックディレクトリ・`manifest.json`・スプライトファイルを、リンクを辿らずに開けて、開いたハンドルから取ったメタデータで判定する（パスで確かめてから別に開くことはしない）。Unix では `mascots_dir` を開いたディレクトリを基準に `openat(O_NOFOLLOW | O_NONBLOCK)` でパックディレクトリ（`O_DIRECTORY` 付き）を、さらにそれを基準にファイルを開くので、途中でどこかをシンボリックリンクに差し替えられても外を読まない。FIFO は開くときに待たず、通常ファイルでないとして落ちる。Windows では `FILE_FLAG_OPEN_REPARSE_POINT` で開いてリパースポイント（シンボリックリンク・ジャンクション）なら落とし、開いたハンドルの実パス（`GetFinalPathNameByHandleW`）の親が、`canonicalize` した `mascots_dir`（パックディレクトリなら）かパックディレクトリの実パス（ファイルなら）と一致することを確かめる。
     - `manifest.json` とスプライトが通常ファイルで、Unix ではハードリンクの数が 1（`nlink() > 1` なら読み飛ばす）。
     - `manifest.json` が 64 KiB 以下、JSON オブジェクトとしてパースでき、`format` が `1`、`sprite` がプレーンなファイル名（`/`・`\`・`..` を含まず `.` で始まらない、拡張子が大文字小文字を問わず `.png`/`.gif`/`.webp`）。マニフェストのそれ以上の検証（`animations.idle` の有無、値の範囲など）はフロントエンド（`desktop-mascot-pack.js::normalizeManifest`）に任せる。
     - スプライトが 1 MiB 以下。大きさはハンドルのメタデータで確かめたうえで、読むときも上限 + 1 バイトまでしか読まず、読んでいる間に上限を超えたら落とす。
     - スプライトの先頭のヘッダ（PNG の `IHDR`、GIF の論理画面記述子、WebP の `VP8 `・`VP8L`・`VP8X` チャンク）を読んで得た形式が拡張子と一致し、幅・高さがどちらも 1〜4096 px で、幅 × 高さが 4,194,304（2048 × 2048）ピクセル以下。
- 読み込んだ `manifest.json`・スプライトのバイト列はそのままメモリに持つ。`GET /mascots/index.json` の本文（`{"packs":[{"id","base"}]}`）も `load()` で 1 回だけ組み立てて持ち、リクエストごとにはそれを返す。並び順は同梱パック（`yureko`・`mochi`・`neko`）→ ユーザー定義パック（id 昇順）。`GET /mascots/{id}/manifest.json` と `GET /mascots/{id}/{sprite}`（`sprite` はそのパックのマニフェストが指すファイル名そのもの）だけを返し、それ以外の `{id}`・`{file}` の組み合わせは `404`。

| ルート | Content-Type |
|---|---|
| `GET /` | `text/html; charset=utf-8`（`index.html`） |
| `GET /favicon.svg` | `image/svg+xml` |
| `GET /favicon-32.png` `/apple-touch-icon.png` | `image/png`（`include_bytes!`） |
| `GET /style.css` `/desktop.css` `/desktop-dialog.css` `/desktop-wallpaper.css` `/desktop-mascot-settings.css` `/desktop-system-settings.css` | `text/css; charset=utf-8` |
| `GET /*.js` | `text/javascript; charset=utf-8`。ファイルの一覧は `assets.rs::STATIC_ASSETS`（`web/*.js`）が正本 |
| `GET /desktop-icons.svg` | `image/svg+xml` |
| `GET /desktop-page.html` `/desktop-page.css` | `text/html; charset=utf-8` / `text/css; charset=utf-8`。Desktop 画面の iframe に入るリンク集ページとその専用 CSS（既定は `web/` の同名ファイル。ページの契約は [`dashboard/desktop.md#リンク集ページiframe`](dashboard/desktop.md#リンク集ページiframe)） |
| `GET /desktop-frame.css` | `text/css; charset=utf-8`。差し替え対象ではない |
| `GET /desktop-banner` | 既定は `image/gif`（`web/desktop-banner.gif`）。差し替えると拡張子から決める。差し替えられるので拡張子はパスに持たせない |
| `GET /desktop-mascot.css` | `text/css; charset=utf-8` |
| `GET /mascots/index.json` `/mascots/<id>/manifest.json` | `application/json`。Desktop 画面のマスコットのパック一覧と各パックのマニフェスト（同梱パックと `mascots_dir` のユーザー定義パック。詳細は上記「[マスコットのパック配信](#マスコットのパック配信srcdashboardmascots)」、パック形式は [`dashboard/mascot.md#パック形式-1`](dashboard/mascot.md#パック形式-1)） |
| `GET /mascots/<id>/<sprite>` | `image/png`/`image/gif`/`image/webp`（拡張子から決める）。各パックのスプライトシート |
| `GET /fonts/pixelmplus12-regular.woff2` `/fonts/pixelmplus12-bold.woff2` | `font/woff2` |
| `GET /login?code=<code>` | ファイルではなくログインリンクの受け口（`session::login_page`）。コードは消費せず、`303` で Web UI にリダイレクトする（上記「認証」） |
| `GET /custom.css` | `[dashboard].custom_css` の中身をリクエストのたびにディスクから読んで返す（`text/css; charset=utf-8`、`Cache-Control: no-store`）。未設定・読み込み失敗なら空文字 |

各 JS ファイルの役割・依存関係は [`dashboard/web.md#構成`](dashboard/web.md#構成) を参照。

## 設定の読み込みと編集（`src/settings/`）

すべての設定キーは `src/settings/mod.rs::SETTINGS`（`Setting` の配列。キー・セクション・TOML フィールド・環境変数・種類・`swing.example.toml` 上の見え方・編集可否・英日の説明を持つ）に 1 箇所のカタログとしてまとまっている（[`../architecture.md#設定と環境変数`](../architecture.md#設定と環境変数)）。`GET /api/config` はこのカタログをそのまま列挙するので、載っている項目（パス・待ち受けアドレス・ポート、kind 番号なども含め）はすべて `kind`/`description` を持つ。

そのうち書き込める（`PUT /api/config`／`POST /api/setup` で受け付ける）キーはカタログの `editable: true` が付いているものだけに絞っている。`editable: false` のキー（パス・待ち受けアドレス・ポート、`kubo.binary`、`dashboard.ui`、`allowed_hosts`、`kubo.managed`、`ipfs.*`、kind 番号、`gateway.*` など）は対象外（ログインできる相手によるファイルパス・リスニングアドレスの差し替え防止）。編集可能なキー（`section.field`、種類）:

| キー | 種類 |
|---|---|
| `nostr.relays` | list |
| `nostr.mirror_set` | string |
| `policy.max_total_storage` / `max_per_site` / `max_per_account` / `max_update_size` | size |
| `policy.max_sites_per_account` / `keep_versions` / `keep_days` | integer |
| `policy.min_update_interval` / `nip05_cache_ttl` | duration |
| `policy.remove_on_unfollow` | bool |
| `policy.nip05` / `publish.nip05` / `publish.check_dotfiles` / `publish.check_size` / `publish.check_unchanged` | mode（`off`/`warn`/`require`） |
| `agent.poll_interval` / `report_ttl` | duration |
| `agent.concurrency` | integer |
| `publish.keep_versions` | integer |
| `publish.dotfiles_allow` | list |
| `kubo.storage_max` | size |
| `dashboard.gateway` | string |

この表はダッシュボードの書き込み範囲を決める安全境界で、実体は `settings::SETTINGS` の `editable` フィールドである。`settings::find`・`settings::is_editable` はこのカタログを引く。`settings::raw_value`（[`dashboard/http-api.md#get-apiconfig`](dashboard/http-api.md#get-apiconfig) の `raw`）は編集可能なキーを手で列挙した `match` で、カタログとの食い違いはテスト `raw_value_covers_every_editable_key` が検出する。カタログに載っていても `source: "env"`（環境変数由来）なら `editable: false` になる。`PUT`/`POST /api/setup` は、編集可能でないキーか env 由来のキーが 1 つでも混ざっていれば要求全体を 400 で拒否し、部分的な適用はしない（非公開の `settings::edit::check_not_env_sourced`）。`nostr.secret_key` はカタログ上 `editable: false` なので `PUT /api/config` からは絶対に書けず、`POST /api/setup` だけが書ける（下記）。

書き込みは `settings::update`（`PUT /api/config`）と `settings::setup`（`POST /api/setup`）と `settings::pin_addrs`（セットアップモードの `swing up` がダッシュボードの待ち受けアドレスを書く。[`up.md#セットアップモードでのポートの調整`](up.md#セットアップモードでのポートの調整)）の 3 つだけで、どれも同じ手順を踏む（`settings::pin_addrs` は `editable: false` のアドレスのキー（`Kind::SocketAddr`）だけを書く）: 書き込み先のファイルを毎回その場で `toml_edit::DocumentMut` として読む（起動時の `Config.config_exists` は見ず、読んだ時点で無ければ空文書。起動後に前の保存で作られたファイルもここで読み直すので、保存を重ねても前の項目は消えない）→ 渡された項目だけを書き換える（`toml_edit` なのでコメントや他のキーはそのまま残る）→ `config::build_config_from_str` で組み立て直して妥当性を確認する（失敗したらファイルには一切触れない）→ 親ディレクトリが無ければ `auth::create_private_dir_all` で作る → `auth::write_private_file` で書く（どちらも上記「認証」と同じ実装）。既存ファイルの権限は引き継がず、unix では既存・新規を問わず常に `0600` にする。書き込み先は設定ファイルの探索順（[`../architecture.md#設定と環境変数`](../architecture.md#設定と環境変数)）で決まるパスで、ファイルが無ければ新規作成になる。

失敗は `settings::EditError` の 2 種類に分かれ、HTTP ステータスが変わる:

- `Invalid` — 要求の中身の問題。編集できないキー・env 由来のキー・値の形式違い・組み立て直した設定の検証失敗・既存ファイルの TOML としての解析失敗・`setup` を鍵のある状態で呼んだこと。API は 400 を返す。
- `Io` — 設定ファイルの読み込み（`NotFound` 以外）・親ディレクトリの作成・一時ファイルの書き込み・`rename`、書き込み後の `Config::load` による読み直しの失敗。読み直しは直前に同じ文字列・同じ環境変数で検証済みなので、ここで失敗するのはファイルの読み込みかプロセス外からの差し替えしかなく、要求側の問題ではないためこちらに入れる。API は 500 を返し、`error!` でログに出す。

設定ファイルを書く API（`PUT /api/config`・`POST /api/setup`・`POST /api/signer/reconnect` の `remote-signer.json` 保存）は `AppState.config_writes: tokio::sync::Mutex<ConfigWrites>` を取ってから読み書きするので、同じプロセス内では 1 つずつ順に走る（並行した保存が互いの項目を消す後勝ちと、`auth::write_private_file` の固定名の一時ファイル `<path>.tmp` の消し合いを防ぐ。プロセス外からの同時書き込みは対象外）。`ConfigWrites.setup_done` は `POST /api/setup` が成功したときに立ち、再起動で `AppState` が作り直されるまで次の `POST /api/setup` を 409 にする。

書き込み成功後の状態は 2 つに分かれる:

- `AppState.restart_required: AtomicBool` — `PUT /api/config` か `POST /api/signer/reconnect` が一度でも成功すれば `true` になり、プロセス内再起動で `AppState` が作り直されるまで戻らない（`POST /api/setup` は立てない）。`GET`/`PUT /api/config` の `restart_required` はこれをそのまま返す。
- `AppState.display_config: RwLock<Arc<Config>>` — 起動時は `AppState.config`（実際に relay・Kubo・agent が使っている設定）のコピーだが、`PUT /api/config` が成功するたびに書き換え後の設定に差し替わる。`GET /api/config` は常に `display_config` を見るので、まだ再起動していなくても「再起動したらこうなる」設定を UI に見せられる。実際に動いている relay・Kubo・agent 側の設定（`AppState.config`）は再起動するまで変わらない。

`POST /api/setup` は上と同じ書き込みに鍵（または `remote-signer.json`）の保存とプロセス内再起動が加わる（[`dashboard/http-api.md#post-apisetup`](dashboard/http-api.md#post-apisetup)）。

## 秘密鍵を出さない仕組み

- `config::Config`（および `NostrConfig`）に `Serialize` を実装していない。DTO は手書きの構造体で、`secret_key` の実値を持つフィールドが型として存在しない。`/api/config` は常に固定文字列 `"(set, hidden)"`（未設定なら `"(not set)"`）を返す。
- 表示するのは `npub` / hex 公開鍵のみ。nsec・鍵の hex は API のどのレスポンスにも登場しない（`POST /api/setup` も `npub` だけを返す）。署名アプリとの接続に使うアプリ鍵（`remote-signer.json` の `app_secret_key`）もどの API にも出さない。`POST /api/setup/signer` が返す `nostrconnect://` URI にはアプリの公開鍵とペアリングの secret が入る（署名アプリに渡すためのもの）。
