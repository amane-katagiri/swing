# CLI（main.rs と各サブコマンド）

サブコマンドの一覧は [`../architecture.md#cli`](../architecture.md#cli)、設定ファイルの探し方は [`config.md`](config.md)、出力例は README を参照。子ページは [`cli/publish.md`](cli/publish.md)（`publish`）と [`cli/views.md`](cli/views.md)（`mirror list` / `add` / `remove`・`sites`・`replicas`・`status`・`stats`・`webring`）。

## 共通

- `<key>` は npub / hex / nprofile を受け付ける。
- 「Follow Set」は kind 30000、`d = mirror_set` の作者ごとに最新のもの。「サイトごとの最新のサイトイベント」は sites・replicas・webring で `nostr::select_latest` が選ぶもの。どちらも新しさの比べ方は [`nostr.md`](nostr.md#未来ずれの許容nostrmax_future_skew)。
- ファイルを書くのは次のコマンドだけで、ほかは読み取り専用。
  - `up`
  - `publish`: MFS
  - `service install`/`uninstall`: OS のサービス登録と、Windows・macOS では `swing-tray` の自動起動の登録（[`service.md`](service.md#タスクトレイの自動起動windows-と-macos)）
  - `signer pair`: `<state_dir>/remote-signer.json`
  - `dashboard rotate-token`: `swing up` が動いていなければ `<state_dir>/dashboard.token`
- `mirror add` / `remove` は Follow Set を relay に送る。受理された後の agent の動きは [`dashboard.md#概要`](dashboard.md#概要)。`stop` と `service start`/`stop` は動いているプロセスやサービスを起動・停止させるだけ。
- `status`・`stats`・`mirror add`・`mirror remove`・`stop`・`dashboard open`・`dashboard rotate-token`（と Windows の `service stop`）は relay/Kubo に直接つながず、動いている `swing up` のダッシュボード API（`[dashboard].listen`、既定 `127.0.0.1:8082`）を `src/api_client.rs::ApiClient` で叩く。
  - `<[agent].state_dir>/dashboard.token` を `Authorization: Bearer` で送る。相手がそのトークンを知っていると確かめられなければ送らずにエラー終了する（[`dashboard/security.md`](dashboard/security.md#cli-と-swing-trayapiclient)）。そのため `swing up` と同じ設定（同じ `state_dir`）を読めて、そのファイルを読めるユーザーで実行する。
  - `listen` が未指定アドレス（`0.0.0.0` / `::`）でも、接続先と `Host` ヘッダはループバックの同じポートにする。
  - API に接続できなければ、`status`・`stats`・`mirror add`・`mirror remove`・`dashboard open` は `swing up is not running (cannot connect to <addr>)` で非ゼロ終了し、`stop` は `not running` を出して終了コード 0 で終わる。
- エラーで終了するときは `Error: <メッセージ>` と、原因があれば `Caused by:` に続けて 1 つずつ（2 つ以上なら番号付きで）標準エラーに出し、終了コード 1 で終わる（`format::error_report`）。メッセージと原因はそれぞれ `format::Sanitized` を通すので、改行を含まず 1 行ずつになる。`RUST_BACKTRACE` などでバックトレースを取っていれば続けて出す。`swing up --log-file` のときは同じエラーをログにも出す。
- `sites`・`replicas`・`webring`・`mirror list`・`publish` は API を経由せず relay/Kubo に直接つなぐので、`swing up` が動いていなくても使える。これらは秘密鍵か署名アプリの接続情報が要り、どちらも無ければエラー終了する（[`signer.md`](signer.md#signer)）。
- API 経由のコマンドは鍵が無くても動く。ただし鍵の無い `swing up` はセットアップモード（[up](#up)）で、そこでは `status`・`mirror add`・`mirror remove` は 503 `agent is not configured` を返す。

## up

Kubo（`[kubo].managed` なら子プロセス）と mirror-agent を 1 プロセスの supervisor として動かす。起動の順・多重起動の防止・セットアップモード・停止は [`up.md`](up.md)。mirror-agent を単体で動かすサブコマンドは無い。

- `--log-file <path>`: 標準エラーの代わりにそのファイルへ追記でログを出す（新しく作るときは unix で 0600）。
- `--no-port-shift`（`SWING_NO_PORT_SHIFT`）: セットアップモードでポートをずらさない（[`up.md#セットアップモードでのポートの調整`](up.md#セットアップモードでのポートの調整)）。

## stop

動いている `swing up` に正常終了、または `--restart` でプロセス内再起動を要求する（[`up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）。`--timeout`（秒、既定 60。`service::GRACEFUL_STOP_TIMEOUT`）は identity・停止要求の POST・ポーリングを合わせた全体の上限（開始時に期限を決め、各リクエストもその期限で打ち切る。大きすぎて期限を作れない値は `--timeout is too large` のエラー）で、超えたら `swing did not stop within Ns`（`--restart` なら `did not come back`）でエラー終了する。最初の identity が期限までに返らなければ `swing did not answer within Ns`（Windows は拒否された接続を約 2 秒再試行してから失敗を返すので、それより短い `--timeout` では動いていなくてもこのエラーになる）。実装は `src/stop.rs` で、ダッシュボード API だけを使う。

1. トークンを送らない [`POST /api/identity`](dashboard/http-api/session.md#post-apiidentity)（`ApiClient::identity`）で今の `instance` を読む。接続できなければ `not running` で正常終了し、相手がトークンを知っていると確かめられなければエラー終了する。
2. `POST /api/shutdown`（`--restart` なら `/api/restart`）を叩く。接続できなければ `not running` を出して正常終了する。
3. `ApiClient::identity` を 500ms 間隔でポーリングする。`--restart` なしなら接続できなくなった時点で `stopped`、`--restart` なら確かめられた応答の `instance` が 1. の値から変わった時点で `restarted` を出して正常終了する。確かめられない応答はどちらでも待ち続ける。

Windows の `swing service stop` もこの `stop::run` を使う（失敗したときの扱いは [`service.md`](service.md#windowsタスクスケジューラ)）。

## dashboard open / rotate-token

実装は `src/login.rs`（`swing-tray` と共通。[`tray.md`](tray.md)）。認証の仕組みは [`dashboard/security.md`](dashboard/security.md#認証)。

- `dashboard open [--config] [--no-browser]`: `POST /api/login-code` で使い捨てのログインコードをもらい、`<[dashboard].public_url>/login?code=<code>`（`public_url` 未設定時の URL の決め方は [`dashboard.md#設定dashboard`](dashboard.md#設定dashboard)）とコード（`login code (single use, valid for 5 minutes): ...`）を標準出力に出す。
  - `--no-browser` が無ければ続けて OS の既定ブラウザで URL を開く（Linux は `xdg-open`、macOS は `open`、Windows は `%SystemRoot%\System32\rundll32.exe url.dll,FileProtocolHandler`）。開けなければ標準エラーに案内を出して正常終了する。
  - `[dashboard].ui = false` ならエラー終了する。
  - 返ってきたコードが小文字の hex 32 文字でなければ、URL もコードも出さず、ブラウザも開かずに `the dashboard at <addr> returned a malformed login code` でエラー終了する。
- `dashboard rotate-token [--config]`: `POST /api/token/rotate` でトークンを作り直す（ブラウザのセッションはすべて無効になる）。`swing up` が動いていなければ `<state_dir>/dashboard.token` を直接書き換える。

## service install / uninstall / start / stop / status

`swing up` を OS のサービス（systemd user unit・launchd LaunchAgent・Windows タスクスケジューラ）として登録・操作する。OS ごとの実体は [`service.md`](service.md)。

- `install` のオプション: `--config`（決め方は [`service.md#共通`](service.md#共通)）・`--system`（Linux のみ）・`--run-as <user>`・`--allow-root`（実行ユーザーが root でも登録する。どちらも `--system` と一緒にだけ使える。[`service.md#system-unit-の実行ユーザー`](service.md#system-unit-の実行ユーザー)）・`--no-start`（登録だけで起動しない）・`--no-tray`（Windows と macOS で `swing-tray` の自動起動を登録しない）。
- `start`/`stop`/`status`/`uninstall` は `--system` を取る。`stop` は登録を残してプロセスだけを止め、`uninstall` は止めてから登録を消す。
- `uninstall --only-from <dir>` と `status --points-into <dir>`（どちらも `--help` に出さない）は、インストーラー向けに、起動する実行ファイルが `<dir>` の下にある登録だけを消す・その有無を終了コードで答える。規則・出力・終了コードは [`service/ownership.md`](service/ownership.md)。
- どれもサービス機構を通す。Windows の `service stop` だけは [stop](#stop) を使う。

## publish

`swing publish` は [`cli/publish.md`](cli/publish.md) を参照。ダッシュボードと共有する処理は [`publish.md`](publish.md)。

## signer pair

NIP-46 の署名アプリと、ターミナルに出した QR コードでペアリングし、`<state_dir>/remote-signer.json` に保存する（実装は `src/pair.rs`。ペアリングの仕組み・タイムアウト・確認の署名は [`signer.md#ペアリングpairing`](signer.md#ペアリングpairing)）。`--config` と `--relay <URL>`（署名アプリとのやりとりに使う relay。繰り返し指定で最大 5 個、既定 `wss://relay.primal.net`）を取る。ダッシュボード API は使わず、動いている `swing up` があってもなくても同じように動く。

1. `[nostr].secret_key`（`SWING_NOSTR_SECRET_KEY`）が設定されていれば、QR を出す前にエラーで終了する。
2. QR コードを Unicode のブロック文字で標準出力に出し（描けなければ標準エラーに案内）、続けて `nostrconnect://` のリンクと、待つ時間の案内を出す。
3. 状態を 200ms 間隔で見る。署名アプリが接続したら（`Checking`）`connected as <npub>; asking the signer app to sign a check event...` を出す。失敗（`Failed`。タイムアウトを含む）なら `pairing failed: ...` でエラー終了する。
4. `remote-signer.json` が既にあるときはつなぎ直しとして扱い、そのファイルのユーザーと同じ公開鍵の署名アプリだけを受け付ける。違えば、状態が `Checking` か `Ready` になったのを見た時点で `the signer app signs as <npub>, not as this swing's <npub>; connect the same Nostr account` でエラー終了し、ファイルは変えない（確認の署名のリクエストより前に止まるとは限らない）。
5. `Ready` になったら `remote-signer.json` を書く。確認の署名が通れば `the check event was signed`、通らなければ標準エラーに警告を出す（保存はする）。最後に `paired: swing now signs as <npub> (saved to <path>)` と、動いている `swing up` には再起動するまで反映されないこと（`swing stop --restart` かサービスの再起動）を出す。`remote-signer.json` 以外の設定は書かない。

## key generate

新しい鍵ペアの nsec / npub / hex（秘密鍵・公開鍵）を標準出力に出す。設定を読まない。

## config example / config env-example

設定ファイルを読まない（`--config` を取らない）。`src/settings/catalog.rs::SETTINGS` の設定カタログから、リポジトリ直下の `swing.example.toml`／`.env.example` と同じ内容を標準出力に印字する（[`config.md`](config.md)）。両ファイルがこの出力と一致することを `cargo test` が確かめる。
