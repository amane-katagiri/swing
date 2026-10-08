# ダッシュボードのガードと認証（`src/dashboard/guard.rs`, `src/auth.rs`, `src/dashboard/session.rs`, `src/api_client.rs`）

[`../dashboard.md`](../dashboard.md) の子ページ。認証まわりの API の入出力は [`http-api.md`](http-api.md) から辿る子ページ、`[dashboard]` の各キーは [`../dashboard.md#設定dashboard`](../dashboard.md#設定dashboard) を参照。

## ガード

全リクエストに axum middleware（`guard::security_middleware`）がかかり、次の順に判定する。エラーは `{"error": ...}` の JSON。

1. **Host**: `Host` ヘッダが無ければ 403 `missing Host header`。`host::extract_host`（IPv6 の `[...]` を考慮してポートを外し小文字にする。`]` の後ろが `:<数字>` 以外ならヘッダ全体をホスト名として扱う。内蔵 gateway と共有）した値が `localhost`・`127.0.0.1`・`::1`、`[dashboard].listen` の IP（`0.0.0.0`・`::` を除く。IP として解釈して比べるので IPv6 の表記ゆれは同じとみなす）、`[dashboard].allowed_hosts`（大文字小文字を区別しない）のどれでもなければ 403 `host not allowed`（DNS rebinding を防ぐ。rebinding で届く Host は攻撃者のホスト名なので、待ち受け IP そのものの Host は許してよい）。`listen` が LAN の IP のときに `swing dashboard open`・`swing-tray` が送る Host とログインリンクの頭（`http://<listen>`）はこの IP になる。ループバック以外の名前は、下記「[リバースプロキシ経由での公開](#リバースプロキシ経由での公開)」の構成でだけサポートする。
2. **`X-Swing-Dashboard` と Origin**: GET/HEAD 以外と、`Authorization: Bearer` の無い `/api/*` の GET/HEAD が対象。`X-Swing-Dashboard: 1` が無ければ 403 `missing X-Swing-Dashboard header`。`Origin` があれば、その authority（スキームを外し末尾の `/` を削ったもの）が `Host` ヘッダと大文字小文字を無視して一致しなければ 403 `origin does not match host`。CORS ヘッダは返さない。`Sec-Fetch-Site` は見ない（Web UI の `/api/*` の読み方は [`web.md#api-の呼び方`](web.md#api-の呼び方)）。
3. **認証**: `/api/login`・`/api/identity` 以外の `/api/*` と、Desktop 画面だけで使う差し替え可能なファイル（`/desktop-page.html`・`/desktop-page.css`・`/desktop-banner` と `/mascots/` で始まるパス。`guard::needs_auth`）は、`guard::authenticate` を通らなければ 401 `missing or invalid dashboard token or session`。`Authorization: Bearer <token>` があればそれだけで判定し（`auth::token_matches`）、無ければセッション cookie を見る（下記「[認証](#認証)」）。`/api/` の外のこれらのファイルは 2 の対象外で、Bearer かセッション cookie のどちらかがあれば通る。どちらで通ったか（`guard::AuthMethod`）はリクエストの extension に入れる。
4. **Bearer 限定**: `POST /api/login-code` と `POST /api/token/rotate` は Bearer で通ったときだけ受け付け（`guard::require_bearer`）、セッション cookie なら 403 `this endpoint needs the dashboard token, not a browser session`。セッション cookie からログインコードを作れると、漏れた cookie を期限ごとに作り直して使い続けられるため。Web UI はどちらも呼ばない。

ログイン前の画面が読むもの（`/`・`*.js`・`/desktop-page.css` を除く `*.css`・`/fonts/*`・favicon・`/apple-touch-icon.png`・`/desktop-icons.svg`・`/custom.css`）と `/login` は認証なしで返す。ダッシュボードの Host に届く人には誰にでも見えるので、`/custom.css` には秘密を置かない。

どのレスポンスにも次のヘッダを付ける。

| ヘッダ | 値・対象 |
|---|---|
| `X-Content-Type-Options` | `nosniff` |
| `Content-Security-Policy` | `default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'self'; base-uri 'none'; form-action 'self'; object-src 'none'` |
| `Referrer-Policy` | `no-referrer` |
| `X-Frame-Options` | `SAMEORIGIN` |
| `Cache-Control` | `no-store`。middleware が付けるのは `/api/` 配下だけ（`/custom.css` はハンドラが付ける） |
| `Cross-Origin-Resource-Policy` | `same-origin`。認証の要る `/api/` 外のファイル（上の 3）の応答だけ |

## 認証

### トークン

- `<state_dir>/dashboard.token` に 32 バイトの乱数の hex（64 文字）を 1 行置く。`swing up` が起動時に `auth::load_or_create_token` で読み、無ければ作る。再起動しても同じトークンを使う。メモリ上の値は `AppState` の `std::sync::RwLock<String>`（`AppState::token` / `set_token`）。
- 書き込み（`auth::write_new_token`）は `auth::write_private_file` で行う。同じディレクトリの一時ファイルに `create_new` で書き、`sync_all` してから rename する。失敗したら一時ファイルを消す。Unix では `0600` で作る。Windows は `state_dir` の ACL を継承する。
- `state_dir` を swing が新しく作るとき（`auth::create_private_dir_all`）は Unix なら `0700` で作る。すでにあるディレクトリのパーミッションは変えず、確かめもしない。
- Unix では `auth::read_token` が読むたびに、ファイルのパーミッションが `0600` より広ければ `warn` を出す（変えはしない）。
- `POST /api/token/rotate` はファイルを書き換えてメモリ上の値も差し替え、未使用のログインコードを捨てる。HMAC の鍵が変わるので既存のセッション cookie はすべて無効になる。

### CLI と `swing-tray`（`ApiClient`）

- `api_client::ApiClient::for_config` が作るたびにトークンファイルを読み、すべてのリクエストに `Authorization: Bearer <token>`（ファイルが無ければ付けない）と `X-Swing-Dashboard: 1` を付ける。プロキシの環境変数は使わない（`no_proxy`）。
- **サーバの本人確認**: トークン付きのリクエストのたびに、その前に 32 バイトの乱数の hex の nonce（`auth::new_identity_nonce`）を `POST /api/identity` に送り、返ってきた `proof` が期待する値（[`http-api/session.md#post-apiidentity`](http-api/session.md#post-apiidentity)）と一致するかを確かめる。一致しなければトークンを送らず `ApiClientError::NotSwing` で止める（相手がエンドポイントを持たなければ `Http` エラー）。結果は覚えない。トークンが無いときは確かめない。
- 確かめる前の相手からの本文（`POST /api/identity` の応答と、2xx 以外の応答のエラー文言）は 64 KiB（`MAX_UNVERIFIED_BODY_BYTES`）までしか読まない。超えたら `identity` は `NotSwing`、エラー文言は HTTP のステータス文字列にする。
- `ApiClient::identity` は本人確認だけを行って応答の `instance` を返す（トークンは送らない）。`swing stop` と `swing-tray` の「終了（はい）」は停止・再起動を待つポーリングにこれを使う（[`../cli.md#stop`](../cli.md#stop)）。

### ブラウザ

- 永続トークンはブラウザに渡さない。`swing dashboard open`（と `swing-tray`）が Bearer で `POST /api/login-code` を呼んで使い捨てのログインコードをもらい、`<public_url>/login?code=<code>` を開く。コードは 16 バイトの乱数の hex（32 文字）で、有効 5 分・1 回限り（`auth::LoginCodes`。プロセスのメモリにだけ持ち、再起動で消える）。
- `login::request_link` は受け取ったコードが小文字の hex 32 文字（`auth::is_login_code`）でなければ、表示もブラウザで開くこともせずエラーにする。
- `GET /login` はコードを消費しない。`code` が 1 文字以上の hex なら `303 /#/login/code/<code>`、hex でなければ `303 /#/login/invalid`、`code` が無ければ `303 /#/login` を返す。Web UI がコードを `POST /api/login` に送ってセッション cookie と交換する（ログイン画面に貼ったコードも同じ）。

### セッション cookie

- 名前は `swing_session_<port>`（`Host` ヘッダのポート。無ければ `swing_session`）。
- 値は `<発行時刻（epoch 秒）>.<HMAC-SHA256 の hex>`。鍵はトークン、メッセージは用途ラベル `dashboard-session`・`\0`・発行時刻の 10 進表記（`auth::sign_session`）。
- 属性は `HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000`。`X-Forwarded-Proto` の先頭の値が `https`（大文字小文字は無視）か、`[dashboard].public_url` が `https://` で始まれば `Secure` も付ける（`session::served_over_https`）。
- 検証（`auth::verify_session`）は、発行から 30 日（`auth::SESSION_TTL`）以上たったものと、発行時刻が 5 分より先のものを拒む。期限はサーバ側で判定する。

## リバースプロキシ経由での公開

ダッシュボードは平文の HTTP しか話さない。ループバックの外から使うのは、TLS を終端する HTTP のリバースプロキシの裏に置く構成に限ってサポートする（プロキシ側の設定は [`guide/security.md`](../../guide/security.md#ダッシュボードを外の端末から使う)）。

- `[dashboard].listen` の IP がループバック（`127.0.0.0/8` と `::1`）でなければ、起動時に平文で流れることを `warn` で出す（判定は待ち受けアドレスだけ）。
- この構成で効くのは、Host の検証（公開ホスト名を `allowed_hosts` に入れる）、Origin の検証（プロキシが `Host` を書き換えると書き込み系がすべて 403 になる）、cookie の `Secure`、ログインリンクの頭になる `public_url`。
- プロキシは `Host` をそのまま転送し（nginx なら `proxy_set_header Host $host;`）、公開ホスト名を `allowed_hosts` に入れる。`Host` を上流の待ち受け IP に書き換えるプロキシ（nginx の `proxy_pass` の既定）でも、待ち受け IP は常に許すので Host の検証は通り、ブラウザがどの名前で来たかをダッシュボードは確かめられない。それでも cookie はブラウザが使った名前のホストにだけ結び付くので別の名前には送られず、書き込み系は `Origin` と書き換え後の `Host` が合わずに 403 になる。

## 既知の弱点

- **未認証の相手からの DoS**: ヘッダ読み取りのタイムアウトが無く（Slowloris）、レート制限も同時接続数の制限も無い。HTTP のリバースプロキシの裏ならプロキシ側のタイムアウトで止まるが、平文のまま LAN に直接出す構成や TCP をそのまま流す前段では止まらない。ボディを少しずつ送る接続はハンドラの中で読むので `TimeoutLayer`（[`../dashboard.md#タイムアウトsrcdashboardmodrs`](../dashboard.md#タイムアウトsrcdashboardmodrs)）で切れる。止まるのはダッシュボードだけで、agent と Kubo は動き続ける。
- **LAN の名前や外部の Kubo では、peer のサイトがダッシュボードと同じホストで開かれうる**: SWING が設定する Kubo の gateway（managed と compose の `ipfs`）は `127.0.0.1`・`::1`・`*.localhost` でパス形式の URL に 404 を返し、`localhost` ではサブドメイン形式に移る（[`../kubo.md#パス形式を返さないホスト`](../kubo.md#パス形式を返さないホスト)）ので、既定の構成（ダッシュボードをループバックで開く）では peer の HTML がダッシュボードと同じホストで返ることはない。返りうるのは、ダッシュボードを LAN の IP や `allowed_hosts` の名前で開き、Kubo の gateway にも同じ名前で届くときと、SWING が設定しない外部の Kubo（`[ipfs].api`）を使うときだけ（設定の検証（[`../gateway.md#設定gateway`](../gateway.md#設定gateway)）が防ぐのは内蔵 gateway の `[gateway].hosts` との重なりだけ）。cookie はポートを区別しないので、そのときはパス形式で開いた peer のページとダッシュボードが同じ cookie を見る。
  - それでもできないこと: セッションの cookie を読むことも、上書きすることも、`path=/api` の cookie で覆い隠すこともできない（`HttpOnly`）。`/api/` を呼ぶこともできない（[ガード](#ガード)の 2。ヘッダ付きのクロスオリジン要求は CORS のプリフライトで止まり、`mode: 'no-cors'` ではヘッダを付けられない）。認証の要る `/api/` 外のファイルも `Cross-Origin-Resource-Policy` で読めない。
  - できること: 同じホストに cookie を数百個置いて、ブラウザの 1 ドメインあたりの上限からセッションの cookie を追い出し、ログアウトさせること（HMAC があるので偽造はできない）。
- **peer のサイトは `localhost` 系の扱いを受ける**: サブドメイン形式の `<cid>.ipfs.localhost` はブラウザにとってループバックなので、平文の HTTP でも secure context になり（`https://` のサイトと同じ API が使え、使うときの許可の確認も同じ）、Safe Browsing の警告も当てにできない。`ipfs.localhost` は Public Suffix List に無いので、すべての CID が 1 つのサイト `ipfs.localhost` になり、あるサイトが `Domain=ipfs.localhost` の cookie を置くとほかの peer のサイトに届く。ダッシュボード（`localhost`・`127.0.0.1`）とは別のサイトなので、ダッシュボードの cookie には届かない。ほかのローカルのポートや LAN へのリクエストには Local Network Access の確認が出ないので、gateway の CSP で JavaScript とフォームから `http:`・`ws:` の他の origin へ送らせない（[`../kubo.md#応答に付けるヘッダー`](../kubo.md#応答に付けるヘッダー)）。`<img>`・`<iframe>`・ページの移動による GET は届く。
- **止まっている間にポートを取った相手に cookie が送られうる**: cookie はポートでなくホストに結び付き、ブラウザには CLI の本人確認に当たるものが無い。Web UI は露出を減らすだけで、止めてはいない（[`web.md#止まっている間の呼び出し`](web.md#止まっている間の呼び出し)）。`/api/identity` をまねて `instance` を返す相手には cookie が渡る。ほかのユーザーがログインできるマシンでは、`swing up` が止まっている間はダッシュボードのタブを閉じるか、疑わしければ `swing dashboard rotate-token` で全セッションを無効にする。
- **セッションを 1 つだけ取り消せない**: サーバはセッションの一覧を持たない。端末をなくしたら `swing dashboard rotate-token` で全セッションを無効にするしかなく、しなければその cookie は発行から 30 日間有効のまま。

## 秘密鍵を出さない仕組み

- `config::Config`（と `NostrConfig`）は `Serialize` を実装していない。DTO は手書きの構造体で、`secret_key` の実値を持つフィールドが型として無い。`/api/config` は固定文字列を返す（[`http-api/config.md#get-apiconfig`](http-api/config.md#get-apiconfig)）。
- 表示するのは `npub` と hex の公開鍵だけ。nsec・秘密鍵の hex・署名アプリとの接続に使うアプリ鍵（`remote-signer.json` の `app_secret_key`）はどの API の応答にも出ない（`POST /api/setup` も `npub` だけを返す）。`POST /api/setup/signer` が返す `nostrconnect://` URI には、署名アプリに渡すアプリの公開鍵とペアリングの secret が入る。
