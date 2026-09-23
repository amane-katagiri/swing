# ダッシュボードの認証

todo の「ダッシュボードの認証トークン」（優先度 高）を片付けた。`PUT /api/config`／`POST /api/setup` で設定の書き換えや鍵の生成までできるようになったのに、閲覧も書き込みも認証なしだったため。

## 決めたこと

- Jupyter と同じ方式にする。`<state_dir>/dashboard.token` に置いたトークンを CLI が Bearer で送り、ブラウザには HMAC で署名した cookie を渡す。
- ループバックからのアクセスも認証の対象にする。CLI はファイルを読むだけで手間が増えないし、除外すると同じマシンの別ユーザーから丸見えになる。
- 認証を切る設定（`[dashboard] auth = false` のようなもの）は作らない。
- トークンは永続化して、起動のたびには作り直さない。作り直すと `swing stop --restart` や設定変更に伴う再起動のたびにブラウザがログアウトされる。作り直したいときは `swing dashboard rotate-token` を使う。
- 永続トークンはブラウザの URL に載せない。Jupyter のように `?token=` を開くと、ブラウザの履歴やログに残るため。CLI が Bearer で使い捨てのログインコード（5 分・1 回限り）を発行し、ブラウザにはそのコードだけを渡す。コンテナの中などブラウザを開けない場合に備えて、ログイン画面にもコードを貼れるようにした。
- cookie には永続トークンではなく `<発行時刻>.<HMAC(トークン, 用途ラベル・発行時刻)>` を入れる。期限はサーバー側で判定する（`Max-Age` はブラウザへのお願いでしかなく、コピーされた cookie には効かない）。永続トークンがブラウザに入らないので、cookie が漏れても被害は期限までで止まる。鍵がトークンなので、作り直せば全セッションがまとめて無効になる。サーバー側には何も保存しないので、再起動してもログインは続く。
- セッションの期限は 30 日の定数にする（設定項目にはしない）。ときどき開くだけの個人用ダッシュボードなので、短くするとログインし直す手間のほうが目立つ。ループバックで盗まれる経路は同じマシンのブラウザのプロファイルくらいで、そこまで入られていれば `dashboard.token` も読まれる。使っている間に期限を延ばし続ける方式（スライディング）は採らない。セッション cookie（`Max-Age` なし）は、ブラウザが前回のタブを復元すると結局残るので、期間を短くする効果が薄い。
- cookie 名にポートを入れる（`swing_session_<port>`）。cookie はポートを区別しないので、同じ `127.0.0.1` で動く 2 つのインスタンス（例: デモ環境の 18082 とローカルの 8082）が互いの cookie を上書きしてしまうため。ポートは `Host` ヘッダから取り、ブラウザから見えるポート（compose のポート転送後の値）で分かれるようにした。
- 静的ファイル（`/`・JS・CSS）は認証なしで返す。秘密を含まず、未ログインのブラウザにもログイン画面を出す必要があるため。`/` を認証必須にしてリダイレクトする案も考えたが、CLI から開いたブラウザでの最初の遷移を cross-site と見るブラウザでは `SameSite=Strict` の cookie が `/` へのリダイレクトに付かない可能性があり、`/` を公開にしておけばその問題が起きない（同一オリジンの `fetch` には必ず付く）。
- 用途ラベル（`dashboard-session`）を HMAC に混ぜて `src/auth.rs` に切り出した。後で自分専用のゲートウェイに同じ仕組みを使うとき、ゲートウェイの cookie でダッシュボードに入れないようにするため。ゲートウェイ側は設計の判断が残っているので todo に回した。
- 書き込み系の CSRF 対策（`X-Swing-Dashboard` ヘッダと Origin 検証）は残す。cookie で認証するようになったので、むしろ必要になった。
- `swing dashboard open` が出す URL の頭は設定 `[dashboard].public_url`（`SWING_DASHBOARD_PUBLIC_URL`）で変えられるようにした。コマンドはコンテナの中で動くと自分の待ち受け（`127.0.0.1:8082`）しか知らず、デモ環境（socat で `18082 → mirror:8082`）や `SWING_DASHBOARD_BIND` でポートを変えた compose ではブラウザの URL とずれるため。
  - `--base-url` のようなフラグにする案もあった。設定項目が増えないのが利点だが、ずれる人は毎回打つことになる。`.env` に 1 回書けば済むほうを採った。
  - compose で `SWING_DASHBOARD_BIND` から自動で作ることはしない。`0.0.0.0:8082` のような bind 用のアドレスはブラウザから開く URL にならないし、Host 検証（`localhost`／`127.0.0.1`／`::1` と `allowed_hosts`）でも弾かれる。LAN の別の端末から開く人の実際のアドレスは compose からは分からない。
  - ダッシュボードからは編集できないようにした（`editable: false`）。ログインした相手に書き換えられると、ログインコードを任意のホストへ送る CLI になってしまうため。
  - `scheme://host[:port]` だけを受け付ける。パス付きは拒否する（ダッシュボードはサブパスでの配信に対応していないため）。
  - デモ環境は `docker/demo/compose.yaml` で `SWING_DASHBOARD_PUBLIC_URL=http://127.0.0.1:18082` を渡す。`demo.env` ではなく compose 側に置いたのは、既にある `demo.env` を作り直さずに効くようにするため。
- ループバックの外から使う構成として、TLS を終端する HTTP のリバースプロキシ（nginx・Caddy・Cloudflare Tunnel）の裏に置く形を正式にサポートすることにした。それまでは「`allowed_hosts` を広げる構成はサポート対象外」としていた。認証が無かったからで、`allowed_hosts` 自体は DNS rebinding 対策の Host 検証の逃げ道として最初から置いてあった。
  - cookie の `Secure` は、`X-Forwarded-Proto` の先頭が `https` か、`public_url` が `https://` のときに付ける。`public_url` だけで判定すると、`allowed_hosts` だけ設定したプロキシ構成で付かない。`X-Forwarded-Proto` は偽装できるが、cookie はリクエストした本人にしか返らないので、信じても他人に影響しない。
  - 「ループバック以外の Host なら必ず `Secure`」という案も考えた。ブラウザは `http://` のページからは `Secure` の cookie をセットさせないので、LAN で平文のまま使う人がログインできなくなる。これは採らなかった。
  - プロキシは `Host` を書き換えない前提にした。書き換えると Origin 検証と合わず書き込み系が 403 になるので、ドキュメントに書いた。
  - 起動時の警告は「ループバック以外で待ち受けているとき」だけにした。`allowed_hosts` だけの構成は、同じホストのプロキシから受ける正しい使い方なので警告しない。
  - 未認証の相手からの DoS（ヘッダ読み取りのタイムアウトが無い）と、セッションを 1 つだけ取り消せないことは、今回は直さずに architecture の「既知の弱点」に書いた。DoS は HTTP のリバースプロキシの裏なら前段で止まる。

## 作ったもの

- `src/auth.rs`: トークンファイルの読み書き（`0600`、tmp に書いて rename）、`sign_session`／`verify_session`、Bearer トークンの比較（`token_matches`。両辺の MAC を比べて比較時間が一致の長さに依存しないようにした）、使い捨てコードの `LoginCodes`。
- `src/dashboard/guard.rs`: `/api/login` 以外の `/api/*` に認証をかけた（401）。
- `src/dashboard/session.rs`: `POST /api/login`、`POST /api/login-code`、`POST /api/token/rotate`、`GET /login?code=`。
- `src/api_client.rs`: `ApiClient::for_config` がトークンファイルを読み、すべてのリクエストに Bearer を付ける。
- `src/login.rs`・`main.rs`: `swing dashboard open [--no-browser]` と `swing dashboard rotate-token`。rotate-token は `swing up` が動いていなければファイルを直接書き換える。
- `web/login.js`・`index.html`・`i18n.js`: Login 画面。`apiFetch` が 401 を受けると `swing:unauthorized` を投げ、`app.js` が Login 画面に切り替えてサイドナビを隠す。
- `swing up` の警告文を「認証が無い」から「平文 HTTP でコードと cookie が流れる」に変えた。
- 依存に `hmac`・`sha2`・`getrandom` を足した。`getrandom` は既に間接依存としてロックファイルに入っていた。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`。Windows は `cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings`（Linux の clippy では出なかった `manual_is_multiple_of` をここで拾って直した）。
- 追加したテスト: セッションの期限・未来の発行時刻・改ざん・トークンと用途への束縛、トークンファイルの作成・再利用・作り直し・`0600`、コードの 1 回限り、cookie 名のポート、Bearer と cookie の判定、ルータ経由での 401・コードから cookie への交換・リンクからのログイン・作り直し後の旧トークンと旧 cookie の拒否、`ApiClient` が GET・POST の両方に Bearer を付けること。
- デモ環境（`docker/demo/demo.sh up --seed`）で次を確かめた。
  - 未ログインで `/` を開くと Login 画面になり、サイドナビが空になる。
  - `/login?code=bogus` だと「無効か期限切れ」を表示する。
  - コンテナ内の `swing dashboard open --no-browser` で出たコードを貼るとログインできて Sites 画面になる。
  - ログインリンクを開くと Sites 画面になる。
  - `swing dashboard rotate-token` の後に開き直すと Login 画面に戻る。
  - `swing status`（Bearer）はそのまま動く。
  - seed の使い捨て `swing up` 相手の `mirror add`／`stop` も、同じ `state_dir` のトークンで通る。
- `public_url` を渡したデモ環境で、コンテナ内の `swing dashboard open --no-browser` が `http://127.0.0.1:18082/login?code=...` を出し、ホストのブラウザでそのまま開いてログインできた。Settings 画面の dashboard 表に `public_url` が読み取り専用の行として出る。
- `X-Forwarded-Proto: https`（大文字、カンマ区切りの先頭を含む）と `public_url = https://...` のそれぞれで `Secure` が付き、`http` や無指定では付かないことをテストした。
- 入力欄のすぐ下に注記を置いたら余白が無かったので、既存の `swing-field` で包んで `--swing-space-1` の間隔にした。
