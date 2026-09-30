# 設定とセットアップ（`src/dashboard/api.rs`, `src/dashboard/setup.rs`, `src/dashboard/config_dto.rs`）

[`../http-api.md`](../http-api.md) の子ページ。共通の形式・エラー・準備状態は親ページを、設定のカタログ・書き込み範囲・書き込み手順は [`../../config.md`](../../config.md) を参照。

## GET /api/config

```json
{ "config_path": "/path/to/swing.toml", "config_exists": true, "writable": true, "restart_required": false, "sections": [
  { "name": "nostr", "items": [
    { "key": "secret_key", "env": "SWING_NOSTR_SECRET_KEY", "value": "(set, hidden)", "source": "file", "editable": false, "kind": "secret", "description": { "en": "Signing secret key (nsec or hex)...", "ja": "署名用の秘密鍵（nsec または hex）..." } },
    { "key": "relays", "env": "SWING_NOSTR_RELAYS", "value": ["wss://relay.damus.io", ...], "source": "default", "editable": true, "kind": "list", "raw": ["wss://relay.damus.io", ...], "description": { "en": "Nostr relays to connect to...", "ja": "接続する Nostr relay（カンマ区切り）" } } ] },
  { "name": "policy", "items": [
    { "key": "max_total_storage", "env": "SWING_MAX_TOTAL_STORAGE", "value": 107374182400, "display": "100 GiB", "source": "env", "editable": false, "kind": "size", "raw": "100 GiB", "description": { "en": "Total storage cap...", "ja": "保存する全サイト合計の容量上限" } } ] } ] }
```

- `config_path`: 設定ファイルのパス（ファイルが無くても入る。決め方は [`../../config.md`](../../config.md)）。`config_exists` はそこに実際にファイルがあるかどうか。
- `writable`: ファイルがあればそれを追記で開けるか、無ければ親ディレクトリが読み取り専用でないか。
- `restart_required`: 設定を書き換えてから、まだ再起動していないか（立つ条件は [`../../config.md#ダッシュボードでの直列化と反映`](../../config.md#ダッシュボードでの直列化と反映)）。
- `sections`: `nostr`/`ipfs`/`policy`/`agent`/`publish`/`dashboard`/`kubo`/`gateway` の順で、設定のカタログ（`settings::SETTINGS`）の全項目を宣言順に並べる。
- 各 `items[]`:
  - `value`: 生の値（文字列・真偽・数値・文字列配列）。パスは解決後の値（規則は [`../../config.md`](../../config.md)）。容量・時間の項目には読みやすい文字列の `display`（`"100 GiB"`・`"5m"` など）が付く。
  - `source`: `"env"` / `"file"` / `"default"`。
  - `editable`: 書き込み範囲に入っていて、かつ `source` が `"env"` でないときだけ `true`。
  - `kind`: `"size"` / `"duration"` / `"bool"` / `"integer"` / `"string"` / `"list"` / `"mode"` / `"path"` / `"socket_addr"` / `"port"` / `"url"` / `"secret"` / `"listen"` のいずれか。書き込み範囲に入るのは前の 7 種だけ。
  - `raw`: 書き込み範囲の項目だけに付く、`PUT /api/config`／`POST /api/setup` の `items` にそのまま送り返せる形の値（`size`/`duration` は単位付きの文字列、`list` は文字列配列、それ以外は文字列）。
  - `options`: `kind: "mode"` のときだけ付く `["off", "warn", "require"]`。
  - `description`: `{ "en": ..., "ja": ... }` の 1 文の説明。
- 特別な値: `nostr.secret_key` の値は常に `"(set, hidden)"` か `"(not set)"` で `editable` は常に `false`（[`../security.md`](../security.md)）。`ipfs.api` は `[kubo].managed = true` なら `"managed"`、`false` なら実際の URL。`dashboard.listen` は常に `SocketAddr` の文字列（`dashboard.ui` も項目に入る）。`kubo.binary` は未設定なら空文字、`kubo.swarm_port` は未設定なら `"-"`、`gateway.listen` は無効なら `"off"`。

## PUT /api/config

設定ファイルの値を書き換える。

```json
{ "items": { "policy.max_total_storage": "20GiB", "nostr.relays": ["wss://relay.damus.io", "wss://nos.lol"] } }
```

- キーは `"<section>.<フィールド名>"`（`GET /api/config` の `raw` が付くキーと同じ）、値の形式は `raw` と同じ（`nostr.relays` は空配列だと 400）。受け付けるキーの範囲と、範囲外のキーが混ざったときの 400 は [`../../config.md#編集できるキー`](../../config.md#編集できるキー)。
- 成功したら `restart_required: true` の `GET /api/config` と同じ形を返す。動いている relay・Kubo・agent への反映は次の再起動から。
- 書き込み先のファイルは要求のたびに読み直す。書き込みの手順と、ほかの書き込み API との直列化は [`../../config.md#設定の書き換えsrcsettingseditrs`](../../config.md#設定の書き換えsrcsettingseditrs)・[`../../config.md#ダッシュボードでの直列化と反映`](../../config.md#ダッシュボードでの直列化と反映)。
- 入力・検証の失敗は 400（ファイルに触れない）、設定ファイルの読み書きの失敗は 500。どちらもプロセスの状態は変わらない。

## POST /api/setup

セットアップモード（[`../../up.md#セットアップモード鍵未設定`](../../up.md#セットアップモード鍵未設定)）の間だけ使える。それ以外は 409 `{"error": "swing is already configured; setup is no longer available"}`。

```json
{ "secret_key": null, "remote_signer": false, "items": { "nostr.relays": ["wss://relay.damus.io"], "policy.max_total_storage": "100GiB", "policy.max_per_site": "10GiB", "policy.max_per_account": "20GiB" } }
```

- `remote_signer`（省略時 `false`）: `true` なら秘密鍵を書かず、[`POST /api/setup/signer`](#post-apisetupsigner) で済ませたペアリングの結果を `<state_dir>/remote-signer.json` に保存する（[`../../signer.md#remote-signerjson`](../../signer.md#remote-signerjson)）。ペアリングが `ready` でなければ 409 `{"error": "no signer app is connected yet; scan the QR code first"}`。`true` のとき `secret_key` は見ない。
- `secret_key`: `null`（または省略・空文字）なら新しい鍵を作る。nsec か hex の文字列ならその鍵を使う。
- `items` は `PUT /api/config` と同じ規則で受け付ける。
- 成功したら `[nostr].secret_key` に鍵の hex を書き、`items` と合わせて 1 回の書き込みで保存する。`remote_signer: true` のときは `items` だけを書いてから `remote-signer.json` を書き、ペアリングの状態を捨てる。
- 失敗のステータスは `PUT /api/config` と同じ（`secret_key` が鍵として読めないときも 400、`remote-signer.json` の書き込みの失敗も 500）。
- 成功してから再起動するまでの 2 回目は、設定ファイルに触れず 409 `{"error": "setup is already done; swing is restarting"}`。

```json
{ "ok": true, "npub": "npub1…", "restart": true }
```

- `npub` は、秘密鍵ならその鍵の、署名アプリならペアリングで受け取ったユーザーの公開鍵。秘密鍵の値そのものは応答に含めない。
- 成功したら約 300ms 後に `POST /api/restart` と同じプロセス内再起動をする（`setup::schedule_restart`。[`../../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](../../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）。

## POST /api/setup/signer

セットアップモードの間と、署名アプリを使っている間（つなぎ直し）だけ使える（秘密鍵で動いているときは 409）。NIP-46 の署名アプリとのペアリングを始める（[`../../signer.md#ペアリングpairing`](../../signer.md#ペアリングpairing)）。前のペアリングがあれば捨てる。

```json
{ "relays": ["wss://relay.primal.net"] }
```

- `relays`: 署名アプリとのやりとりに使う relay。空白だけの要素は無視し、1〜5 個。ws / wss の URL でなければ 400。
- URI の `perms` は常に `get_public_key` と、レプリカ報告・サイトイベント・Follow Set の `sign_event:<kind>`。

```json
{ "uri": "nostrconnect://<アプリの公開鍵>?relay=…&secret=…&perms=…&name=SWING&metadata=…", "qr_svg": "<svg …>" }
```

`qr_svg` は `uri` を QR コードにした SVG（黒と白、余白付き、256px 以上）。

## GET /api/setup/signer

`POST /api/setup/signer` と同じ条件で使える（それ以外は 409）。今のペアリングの状態を返す。

```json
{ "state": "ready", "npub": "npub1…", "probe_signed": false, "error": "the signer app refused the request: Rejected" }
```

| `state` | 意味 | 値の入るフィールド（ほかは `null`） |
|---|---|---|
| `idle` | ペアリングを始めていない | なし |
| `waiting` | 署名アプリの接続を待っている（最大 10 分。15 秒たっても relay につながらなければ `failed`） | なし |
| `checking` | 接続できた。確認のためレプリカ報告の kind の署名をリクエストしている（最大 60 秒） | `npub` |
| `ready` | ペアリング完了。`POST /api/setup` の `remote_signer: true` か `POST /api/signer/reconnect` で保存できる | `npub`、`probe_signed`（確認の署名が通ったか）、通らなかったときの `error` |
| `failed` | 接続できなかった | `error` |

## POST /api/signer/reconnect

署名アプリを使っている間だけ使える（秘密鍵で動いている・セットアップモードのときは 409 `{"error": "swing does not use a signer app"}`）。ボディは無し。`POST /api/setup/signer` で済ませたペアリングの結果で `<state_dir>/remote-signer.json` を書き換え、[`POST /api/setup`](#post-apisetup) と同じくプロセス内再起動をする。

- ペアリングが `ready` でなければ 409 `{"error": "no signer app is connected yet; scan the QR code first"}`。
- 署名アプリがいまの公開鍵と別のアカウントで署名するなら 409（`... connect the same Nostr account`）。ファイルは書き換えない。
- `remote-signer.json` は設定ファイルを書く他の API と順に書き、失敗は 500。
- 成功したら `restart_required` を立て、ペアリングの状態を捨てて `{ "ok": true, "npub": "npub1…", "restart": true }` を返す。
