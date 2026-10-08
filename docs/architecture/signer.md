# 署名（signer/）

[`../architecture.md`](../architecture.md) の一部。

SWING が出すイベント（サイトイベント・Follow Set・レプリカ報告）の署名は、すべて `signer::Signer` を通す。秘密鍵を設定に置く方法と、NIP-46 の署名アプリ（remote signer）に署名をリクエストする方法の 2 通りがある。イベントの形式はどちらでも同じで、[`../protocol.md`](../protocol.md) は署名の方法に依存しない。

## `Signer`

| 型 | 中身 | 署名 |
|---|---|---|
| `Signer::Local(Keys)` | `[nostr].secret_key`（`SWING_NOSTR_SECRET_KEY`）をパースした鍵 | `EventBuilder::finalize` でその場で署名する |
| `Signer::Remote(Arc<RemoteSigner>)` | `<state_dir>/remote-signer.json` から組み立てた、署名アプリ用の relay 接続（`nostr_sdk::Client`）とアプリ鍵・署名アプリの公開鍵・ユーザーの公開鍵 | NIP-46 の `sign_event` を署名アプリに送り、返事を待つ |

- `Signer::load(config)` が読み込み規則を持つ。秘密鍵だけあれば `Local`、`remote-signer.json` だけあれば `Remote`、どちらも無ければ `None`（`swing up` はセットアップモード。[`up.md#セットアップモード鍵未設定`](up.md#セットアップモード鍵未設定)）、両方あればエラー（どちらかを消すよう促す）。`Signer::require` は `None` をエラーにする。
- `public_key()` は署名せずに自分の公開鍵を返す。`Remote` はペアリング時に署名アプリから受け取った公開鍵をファイルに持っていて、それを使う（`get_public_key` を送らない）。
- `RelayClient`（`nostr/client.rs`）は `Signer` を持ち、`RelayClient::sign(builder)` で署名する。サイトイベント（`publish::sign_site_event`。CLI とダッシュボードの公開）・Follow Set の更新（`mirror::publish_if_changed`）・レプリカ報告（`ReportRelay::send_report`）の 3 か所がこれを呼ぶ。ペアリングの probe（下記）だけは `RemoteSigner` を直接使う。
- `up::run` は呼ばれるたびに（プロセス内再起動を含む）`Signer::load` を 1 回呼び、`dashboard::AppState.signer` に置く。agent（`agent::lifecycle::run_until`）は再起動のたびにそれを使い回し、`RelayClient::connect` に渡す。agent の終了時は relay の `Client` だけを閉じ、署名アプリとの接続は `up::run` の最後に `Signer::shutdown` で閉じる。
- relay に直接つなぐ CLI（[`cli.md#共通`](cli.md#共通)）はコマンドごとに `Signer::require` し、終わるときに `RelayClient::shutdown` で relay と署名アプリの両方の接続を閉じる（`sites`・`mirror list` は取得がエラーなら閉じずに終わる）。`swing up` と同じアプリ鍵を使うので、署名アプリ側の許可はそのまま効く。

## 署名アプリへのリクエスト（`RemoteSigner`）

- `RemoteSigner` は最初のリクエストのときに `remote-signer.json` の `relays` にだけつなぎ（`[nostr].relays` とは別の接続）、アプリ鍵宛ての kind 24133 を `since` 無し・`limit(0)`（保存済みのイベントは受け取らない）で購読する。
- リクエストは `NostrConnectRequest::SignEvent` を `NostrConnectMessage` にして NIP-44 で暗号化し（`NostrConnectEventBuilder`）、署名アプリの公開鍵宛てに送る。返事は、署名アプリの公開鍵から来た kind 24133 のうち、復号できて、送ったリクエストのどれかと同じ `id` の応答だけを待つ。
- 副作用のないリクエスト（`get_public_key` と `ping`。今送るのはペアリング中の `get_public_key` だけ）は、返事が来るまで `RESEND_INTERVAL`（5 秒）ごとに送り直す。送り直すたびに新しい `id` の新しいイベントにし、どの `id` への返事も受け付ける。送り直しに失敗しても待ち続け、待ち時間は送り直しで延びない。`sign_event` は送り直さない。
- 返事の判定は次の順。`result` が `auth_url` ならエラー（`error` の URL をメッセージに入れる）。`error` が空でなければ `the signer app refused the request: <error>`。`result` が無いか空なら `the signer app answered without a result`、`result` が `error` なら `the signer app refused the request`。残りは `result` をメソッドに応じて読み、読めなければ `reading the signer app's answer`。`error` と URL は `format::sanitize_display_text` を通して `MAX_SIGNER_ERROR_CHARS`（500 文字）で切る。
- `connect` は送らない。nostr-connect の `NostrConnect` は使わず、ペアリングも同じ購読とリクエストの仕組み（`signer::Channel`）で進める。
- 1 回のリクエストの待ち時間は `SIGN_TIMEOUT`（90 秒）。署名アプリでユーザーが承認するまでの時間を含む。
- 返ってきたイベントは `check_signed` で確かめる。`pubkey` が自分の公開鍵であること、`id` がリクエストした未署名イベントから計算した `id` と一致すること（内容を書き換えていないこと）、署名が正しいこと。どれかが違えばエラー。
- エラーはタイムアウト・拒否・relay の切断・送信や暗号化や応答の読み取りの失敗を区別し、段階を示す説明を付けて返す。ペアリング中の `get_public_key` も同じ説明になり、ペアリングの `Failed` の理由（`GET /api/setup/signer` の `error`、`swing signer pair` のエラー）として署名アプリの拒否理由がそのまま出る。
- 最後のリクエストの結果を `last_failure: Option<SignFailure>`（時刻とメッセージ）に残す。成功すれば `None` に戻す。`GET /api/overview` の `signer.last_failure` がこれを返す（[`dashboard/http-api/status.md#get-apioverview`](dashboard/http-api/status.md#get-apioverview)）。

## `remote-signer.json`

`<state_dir>/remote-signer.json`（`signer::REMOTE_SIGNER_FILE`）。`auth::write_private_file` で書く（`dashboard.token` と同じ手順。[`dashboard/security.md`](dashboard/security.md#トークン)）。

```json
{
  "app_secret_key": "<アプリ鍵の秘密鍵 hex>",
  "signer_pubkey": "<署名アプリの公開鍵 hex>",
  "relays": ["wss://relay.primal.net"],
  "user_pubkey": "<ユーザーの公開鍵 hex>"
}
```

- `app_secret_key` は SWING が作った使い捨ての鍵で、署名アプリとの暗号化にだけ使う。ユーザーとして署名する力は無く、署名アプリが許可した範囲のリクエストしか通らない。`RemoteSignerFile` の `Debug` はこの値を出さない。メモリ上では `zeroize::Zeroizing` に入れ、読み書きに使う JSON の文字列とともに解放時に消す。署名アプリ用の relay 接続が受け取る大きさの上限は [`nostr.md#検証`](nostr.md#検証)。ペアリングの secret は保存しない。
- ファイルを書くのはセットアップ（`POST /api/setup`）と、署名アプリとのつなぎ直し（`POST /api/signer/reconnect`）と、`swing signer pair`（[`cli.md#signer-pair`](cli.md#signer-pair)）だけ。つなぎ直しは、今と同じ Nostr アカウント（公開鍵）で署名する署名アプリでなければ受け付けない。秘密鍵に戻す手順は [`guide/security.md`](../guide/security.md#署名アプリnip-46で署名する)。

## ペアリング（`Pairing`）

QR コードを使う `nostrconnect://`（クライアント起点）の接続だけを実装している（`signer/pair.rs`。型と関数は `signer::` から再公開している）。

1. `Pairing::start(PairingRequest)` がアプリ鍵と 16 バイトの secret を作り、`nostrconnect_uri` で URI を組み立てて、ペアリングのタスクを spawn する。URI のクエリは `relay`（複数可）・`secret`・`perms`・`name=SWING`・`metadata={"name":"SWING"}`。
2. `PairingRequest::for_config` が `[nostr]` の kind から perms・probe の kind と、`PAIRING_TIMEOUT`・`RELAY_CONNECT_TIMEOUT`・`PROBE_TIMEOUT` を組み立てる（ダッシュボードと `swing signer pair` で共通）。relay は `parse_pairing_relays` で確かめる（空白を除いて 1〜`MAX_PAIRING_RELAYS`（5）個、`ws`/`wss` の URL）。`perms` は `requested_perms(kinds)` が作り、`get_public_key` と、SWING が署名する 4 種類（`[nostr].replica_event_kind`・`[nostr].site_event_kind`・`30000`・`1`）の `sign_event:<kind>` を並べる。perms はリクエストでしかなく、自動で許可するかどうかは署名アプリが決める。
3. タスクは `bounded_client`（上限は署名アプリ用の接続と同じ）で URI の relay につなぎ、アプリ鍵宛ての kind 24133 を `limit(0)` で購読して、`connect` 応答を `PAIRING_TIMEOUT`（10 分）まで待つ。受け付けるのは、復号できて、`result` が URI の secret と一致する応答だけ（`is_connect_with_secret`）で、その送り主を署名アプリの公開鍵とする。`result` が `ack` の応答は warn を出して無視するので、secret を返さず `ack` だけを返す署名アプリとはペアリングできない。続けて、同じ接続でその署名アプリにだけ `get_public_key` を送り（返事は署名アプリの公開鍵から来たものだけを見る。`PUBLIC_KEY_TIMEOUT`（60 秒）まで、5 秒ごとに送り直しながら待つ）、ユーザーの公開鍵が分かったら状態を `Checking` にする。待っている間、始めてから `RELAY_CONNECT_TIMEOUT`（15 秒）たった時点で relay に 1 つもつながっていなければ、`could not connect to the relay (<relay>)` で失敗にする。
4. 保存するのと同じ内容から `RemoteSigner` を作り直し（`PROBE_TIMEOUT`、60 秒。ユーザーが署名アプリで承認する時間を含む）、kind `[nostr].replica_event_kind` の空のイベント（`alt` は `SWING signer check`）の署名を、再起動後と同じ手順（`connect` を送らずにリクエストだけを送る）でリクエストする（probe）。probe の署名は relay に送らない。
5. probe が成功すれば `Ready { probe_signed: true }`、失敗しても `Ready { probe_signed: false, probe_error }` にする。3 までに失敗したら `Failed(メッセージ)`。probe が通っても、署名アプリが自動で許可したのか、ユーザーがその場で承認したのかは区別できない。

QR コードはダッシュボード用に `qr_svg`（SVG）、`swing signer pair` 用に `qr_text`（Unicode のブロック文字）で描く。

ダッシュボードでは、状態を `AppState.pairing: Mutex<Option<Pairing>>` に 1 つだけ持つ。新しいペアリングを始めると古いものは捨て、`Drop` でタスクを止める。セットアップかつなぎ直しが成功したら `None` に戻す。ペアリングはセットアップモードの間のほか、署名アプリで動いている間も始められる（署名アプリ側で接続が切れたときのつなぎ直し）。API は [`dashboard/http-api/config.md#post-apisetupsigner`](dashboard/http-api/config.md#post-apisetupsigner)。

## 署名アプリがオフラインのとき

- レプリカ報告の送信（`agent::replicas::sync_reports`）は、1 件の送信がエラー（署名の失敗・タイムアウトを含む）になった時点でその回を打ち切り、残りは次の同期で送り直す（[`agent/replicas.md`](agent/replicas.md)）。
- publish・ミラー対象の変更は、そのリクエストがエラーで返る（ダッシュボードは 502、CLI は非ゼロ終了）。
- サイトの取得・保存・検証は署名を使わないので止まらない。`public_key()` が署名アプリに問い合わせないので、`swing up` の起動・relay の購読・一覧系のコマンドも動く。

## テスト

外部の relay や Docker は使わず、nostr-sdk の `LocalRelay`（プロセス内の relay）と署名アプリ役（`src/test_support.rs` の `serve_test_signer`、個々の応答を組み立てる `signer::tests::answer_as`）を使う（`#[ignore]` にしていない）。テストは `signer::tests`・`signer::pair::tests`・`dashboard::setup::tests`・`pair::tests`（`swing signer pair`）にある。
