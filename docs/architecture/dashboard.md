# ダッシュボード（src/dashboard/, web/）

[`architecture.md`](../architecture.md) の一部。agent 全体のループは [`agent.md`](agent.md)、Docker での公開は [`docker.md`](docker.md)。

## 概要

`swing agent` のプロセス内で HTTP サーバー（axum 0.8）を立てる、ブラウザ向けの管理画面。専用のサブコマンドは無く、`[dashboard].listen` が `off` でなければ agent 起動時に自動で立ち上がる。

- relay 接続は agent が使っているものと同じ `Arc<RelayClient>` を共有する（リクエストのたびに接続し直さない）。
- 保存状態は agent の `Agent` 構造体が持つ `Mutex<State>` には触れず、CLI の各サブコマンド（`sites`・`status` など）と同じく `state.json` をディスクから読む（`mirror::collect_sites`・`health::collect_status` などが内部で `State::load` する）。`Agent` は private のままで、agent のロックと競合しない。
- `mirror add` / `mirror remove` が relay に受理されると、`tokio::sync::Notify` で agent の待ち受けループに知らせる。agent 側は poll tick と同じ `poll_once`（sweep → Follow Set の再取得 → レプリカ報告の同期）をその場で実行するので、次の poll を待たずに反映される（`poll_timer.tick()` と `notify.notified()` のどちらの分岐も同じ `poll_once` を呼ぶ）。
- 自分の公開鍵（`own_pubkey`）は `AppState::new` が起動時に 1 回だけ秘密鍵から求めて保持する（`/api/overview`・`/api/webring`・`/api/replicas`・`/api/publish/upload` はリクエストのたびに鍵をパースし直さない）。この時点で秘密鍵のパースに失敗すると `AppState::new` がエラーを返すが、`agent::run` はこれより前に `RelayClient::connect` で同じ鍵のパースにすでに成功しているため、実運用でここが失敗することはない。

### 起動

`agent::run` の中で次の順に行う。詳しい流れ（シグナル絡み）は [`agent.md`](agent.md#シグナルと終了) を参照。

1. relay に接続し、state を読み、`[dashboard].listen` が `Off` でなければ `TcpListener::bind` する。**bind に失敗すると agent の起動自体がエラーで終了する**（黙って続行しない）。`Off` なら何もしない。bind したアドレスがループバック（`127.0.0.1`/`::1`）以外、または `allowed_hosts` が空でなければ、ダッシュボードに認証が無いことを `tracing::warn` で警告する。
2. `Agent` を組み立て、SIGINT・SIGTERM の永続リスナーを作ってから `reconcile`（起動時の突き合わせ）を行う。この突き合わせはシグナルと競争させており、途中でシグナルが来たら打ち切って shutdown に進む（この時点ではまだダッシュボードを起動していないので、relay を切断するだけで終わる）。
3. bind できていれば、まず `<state_dir>/upload/` を掃除（`dashboard::cleanup_upload_dir`。前回の異常終了で残った展開先ディレクトリを消す）してから `dashboard::AppState`（relay・config・IpfsClient・Notify・起動時刻・publish 用の Mutex・`own_pubkey`）を作り、`dashboard::serve` を別タスクとして `tokio::spawn` する。`AppState::new` 自体の失敗（秘密鍵パース）も agent の起動失敗として伝播する。
4. 本体のループ（relay 通知・poll tick・Notify・タスク完了・ダッシュボードタスクの終了・SIGINT・SIGTERM）に入る。

### 終了

SIGINT・SIGTERM のどちらでも同じように終了する（`docker stop` の SIGTERM でも graceful shutdown が効く）。

- poll tick・`Notify` の処理（`poll_once`）中にシグナルが来た場合も、その I/O を打ち切って即座に shutdown へ進む（`agent::race_with_shutdown`。詳しくは [`agent.md`](agent.md#シグナルと終了)）。
- shutdown 処理（`agent::shutdown_dashboard`）: ダッシュボードの `oneshot::Sender` に送った後、そのサーバタスクの `JoinHandle` を最大 5 秒待つ（`axum::serve(...).with_graceful_shutdown(...)` が実行中のリクエストを finish させるのを待つ）。5 秒以内に終わらなければ warn ログを出して待つのをやめ、そのまま relay の `client.shutdown()` を呼んでループを抜ける。
- それとは独立な watchdog（別タスク、別の SIGINT/SIGTERM リスナー）がシグナル受信から 10 秒数えており、それまでに上の shutdown が終わっていなければ `std::process::exit(1)` で強制終了する。
- ダッシュボードのサーバタスクの `JoinHandle` は本体のループの `select!` でも監視している。シャットダウンの合図を送る前にこのタスクが（panic などで）終了した場合は error ログを出すだけで、**agent 本体は止めない**（以降そのプロセスではダッシュボードだけが無くなり、relay 通知・poll・mirror 保存は続く）。
- `[dashboard].listen = off` の場合、bind もタスク起動もせず、agent の動作は今までと変わらない。

## 設定（`[dashboard]`）

`swing.example.toml` のパターンに合わせ、環境変数が TOML を上書きする。

| TOML | 環境変数 | 既定 | 意味 |
|---|---|---|---|
| `listen` | `SWING_DASHBOARD_LISTEN` | `127.0.0.1:8082` | 待ち受けアドレス。`off`（大文字小文字を区別しない）で無効。それ以外は `SocketAddr` としてパースでき無ければ設定エラーで起動しない |
| `allowed_hosts` | `SWING_DASHBOARD_ALLOWED_HOSTS`（カンマ区切り、前後の空白を trim） | `[]` | Host ヘッダで追加で許可するホスト名（ポート抜き、大文字小文字を区別しない） |
| `gateway` | `SWING_DASHBOARD_GATEWAY` | `http://127.0.0.1:8080` | 保存済みサイトを開くリンクの IPFS Gateway のベース URL。空文字なら `gateway_url` を出さない |
| `custom_css` | `SWING_DASHBOARD_CUSTOM_CSS` | なし | `/custom.css` として配信する CSS ファイルのパス |
| `max_upload` | `SWING_DASHBOARD_MAX_UPLOAD` | `2GB` | `POST /api/publish/upload` のリクエストボディ上限（容量の書式は他の容量設定と同じ）。`0` は設定エラー |

- `SWING_DASHBOARD_GATEWAY` に空文字を設定しても既定値に戻ってしまい、gateway リンクを無効にできない（環境変数はすべて空文字を「未設定」として扱う共通処理を通るため）。TOML の `gateway = ""` でなら無効にできる。[`docs/todo.md`](../todo.md) に追記済み。
- `custom_css` が読めない（未設定・存在しない・権限が無いなど）場合、`/custom.css` は空の 200 `text/css` を返す。エラーにはしない。

## ガード（`src/dashboard/guard.rs`）

全リクエストに axum middleware（`security_middleware`）がかかる。

1. **Host 検証**（DNS rebinding 対策）: `Host` ヘッダが無ければ 403。あれば `extract_host`（IPv6 の `[...]` を考慮してポートを外し小文字化）した値が `localhost` / `127.0.0.1` / `::1` か `allowed_hosts` のいずれかでなければ 403。
2. **書き込み系（GET/HEAD 以外）はさらに**:
   - `X-Swing-Dashboard: 1` ヘッダが無ければ 403。CORS ヘッダを一切返さないので、他オリジンの `fetch` は素の CORS では通らない。
   - `Origin` ヘッダがあれば、そこから取り出した authority（スキームを外し末尾の `/` を削っただけ。ポートを含む）が `Host` ヘッダの値と大文字小文字を無視して一致しなければ 403。
3. **レスポンスヘッダ**（成功・失敗どちらにも付く。すべてのルートに一律で付く分はこの middleware で、`/custom.css` の `Cache-Control` だけはハンドラ自身が付ける）:
   - `X-Content-Type-Options: nosniff`
   - `Content-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'none'`（`frame-ancestors 'none'` で iframe 埋め込みを禁止）
   - `Referrer-Policy: no-referrer`
   - `X-Frame-Options: DENY`（CSP の `frame-ancestors` に対応しない古いブラウザ向けの保険）
   - `Cache-Control: no-store`（`/api/` 配下は middleware が付ける。`/custom.css` は `assets::custom_css` 自身が付けており、静的ファイル配信の内容変更後にブラウザキャッシュへ古い CSS が残らないようにしている。`/`・`/style.css`・`web/` 配下の各 ES module（`/app.js` `/graph.js` `/storage.js` `/i18n.js` `/util.js` `/ui.js` `/sites.js` `/webring.js` `/publish.js` `/settings.js`）には付かない）
4. **秘密鍵**: `Config` に `Serialize` を実装しない（型として JSON に出せない）ことで、`/api/config` を含めどの DTO にも秘密鍵の値が現れない。

## タイムアウト（`src/dashboard/mod.rs`）

`tower_http::timeout::TimeoutLayer` を `router()` に掛けている。タイムアウトすると空ボディの `408 Request Timeout` を返す（`guard::security_middleware` がヘッダーだけ付け足す）。

- `POST /api/publish/upload` 以外の全ルート: 120 秒。relay 通信や重い DAG 走査（`/api/status`）を含めても十分な余裕を持たせつつ、slowloris 的にリクエストを長時間占有する接続を打ち切る。
- `POST /api/publish/upload`: 30 分。フォルダアップロードは `[dashboard].max_upload`（既定 2GB）まで許容するため、遅い回線での転送を打ち切らないよう別枠で長く取っている。
- ヘッダー読み取り自体のタイムアウトは、`axum::serve`（configuration を持たない単純なラッパー）を使っている都合上、現状は設定していない（`hyper_util` を直接使う construction に切り替えれば可能）。

## 静的ファイルの配信（`src/dashboard/assets.rs`）

`web/` 配下の全ファイル（`index.html`・`style.css`・ES module 一式）をビルド時に `include_str!` でバイナリに埋め込む（実行時にファイルを探しに行かない。単一バイナリ配布と Docker の両方で同じ動きになる）。Dockerfile のビルドステージは `COPY web ./web` してから `cargo build --release` する。ファイルを 1 つ追加するときは `assets.rs` に定数+ハンドラを、`mod.rs` の router にルートを 1 対 1 で足す（ビルド工程が無いぶん、この対応関係が単純さの拠り所になっている）。

| ルート | 内容 |
|---|---|
| `GET /` | `index.html`（`text/html; charset=utf-8`） |
| `GET /style.css` | `style.css`（`text/css; charset=utf-8`） |
| `GET /app.js` | `app.js`（`text/javascript; charset=utf-8`。ルーター兼エントリポイント） |
| `GET /graph.js` | `graph.js`（`text/javascript; charset=utf-8`。webring の force-directed layout） |
| `GET /storage.js` | `storage.js`（`text/javascript; charset=utf-8`。localStorage の薄いラッパー） |
| `GET /i18n.js` | `i18n.js`（`text/javascript; charset=utf-8`。`MESSAGES`・`t`・`currentLang`・`applyStaticI18n`） |
| `GET /util.js` | `util.js`（`text/javascript; charset=utf-8`。`cache`・DOM/fetch共通ユーティリティ） |
| `GET /ui.js` | `ui.js`（`text/javascript; charset=utf-8`。複数画面で共有する UI 部品） |
| `GET /sites.js` | `sites.js`（`text/javascript; charset=utf-8`。Sites 画面） |
| `GET /webring.js` | `webring.js`（`text/javascript; charset=utf-8`。Webring 画面） |
| `GET /publish.js` | `publish.js`（`text/javascript; charset=utf-8`。Publish 画面） |
| `GET /settings.js` | `settings.js`（`text/javascript; charset=utf-8`。Settings 画面） |
| `GET /custom.css` | `[dashboard].custom_css` の中身をリクエストのたびにディスクから読んで返す（`text/css; charset=utf-8`、`Cache-Control: no-store`）。未設定・読み込み失敗なら空文字 |

## HTTP API（`src/dashboard/api.rs`, `src/dashboard/dto.rs`）

共通:

- すべて JSON。公開鍵は `pubkey`（小文字 hex）と `npub` を併記する。時刻は epoch 秒の整数。無い値は `null`。
- エラーは `{ "error": "<メッセージ>" }` とステータスコード（`api::ApiError`）。入力不正は 400、`state.relay`（テスト以外では常に `Some`）が無ければ 500、relay や Kubo・Nostr 発行の失敗は 502（`Upstream`）、publish の多重実行は 409。
- POST の JSON ボディは専用の `AppJson` エクストラクタ（axum の `Json` をラップ）で受ける。構文エラー・必須フィールド欠落・`Content-Type` が `application/json` でない、のいずれも一律 400 + `{"error":…}` になる（axum 既定の `JsonRejection` は構文エラーなどを 422 にするが、このダッシュボードでは **422 は publish の NIP-05 `require` 失敗専用**にするため、`AppJson` が axum の判定を 400 に読み替えている）。ボディが大きすぎる場合だけ例外的に 413 になる。`POST /api/publish/upload` だけは JSON ではなく `multipart/form-data` を受けるので `AppJson` を使わない（ガード自体は他の書き込み系と同じ。詳細は当該節）。
- `POST /api/mirror/add`・`/remove` の `keys`、`GET /api/webring` の `root`、`GET /api/replicas` の `key` は、どれも **1 リクエストあたり最大 100 件**（`MAX_KEYS`）。超えると 400。
- relay を引く API（sites・mirror・webring・replicas）はサーバ側でキャッシュしない。フロントがビュー切り替え中だけメモリに保持する。**サーバ側の同時実行数の制限やレート制限も無い**ので、認証の無いブラウザから何度も叩けば、そのたびに relay への `fetch_events` が積み上がる。

### 既知の性質

- relay を引く GET はキャッシュ・同時実行制限・レート制限のいずれも無い（上記）。件数上限（`MAX_KEYS`、webring の depth 上限）はあるが、リクエスト頻度そのものは制限していない。
- `POST /api/publish/upload` の 409 は、**ダッシュボード内で同時に来た publish リクエストどうし**しか排他しない。ダッシュボードで publish している間に、同じホスト上で CLI の `swing publish` を別途実行した場合は排他されない（`publish_lock` は `AppState` 内だけのロック）。
- ダッシュボードには認証が無く、ガードは Host 検証・Origin 検証・書き込み系ヘッダの 3 点だけ。`allowed_hosts` を広げて `localhost`/`127.0.0.1` 以外のホスト名からアクセスできるようにする構成は、認証が無いまま到達範囲を広げることになるため想定していない。
- `run_publish`（NIP-05 検証）は `nip05::HttpNip05Verifier::public_only()` を使い、プライベート/ループバック/リンクローカルなどに解決されるホストへの接続を拒否する（SSRF 対策。`src/agent/` が他人の `d` を検証する場合と同じ verifier）。それでも接続エラーの詳細（TCP 接続拒否・TLS 失敗・タイムアウトなど）はレスポンスの `nip05.detail` にそのまま出さず、`unreachable`/`timeout`/`invalid_response` の粗い分類にしてから返す。生のエラー文字列は `tracing::warn` にだけ出す（内部ネットワークに対するポートスキャン用オラクルにしない）。

### GET /api/overview

```json
{
  "version": "0.1.0",
  "pubkey": "ab12…", "npub": "npub1…",
  "relays": ["wss://relay.damus.io"],
  "mirror_set": "swing",
  "gateway": "http://127.0.0.1:8080",
  "started_at": 1790000000,
  "max_upload": 2147483648
}
```

`gateway` は `[dashboard].gateway` が空なら `null`。`started_at` は `AppState` を作った時刻（＝ダッシュボードが有効になった起動時刻）。`max_upload` は `[dashboard].max_upload` のバイト数で、フロントが `POST /api/publish/upload` の送信前チェックに使う。

### GET /api/sites

`mirror::collect_sites` をそのまま JSON にしたもの。

```json
{
  "follow_set": { "found": true, "note": null },
  "accounts": [
    {
      "pubkey": "…", "npub": "…",
      "sites": [
        {
          "d": "example.com", "cid": "bafy…", "url": "https://example.com/",
          "size": 12345, "created_at": 1790000000, "message": "update note",
          "nip05": "verified", "replicas": 3, "stored": true,
          "gateway_url": "http://127.0.0.1:8080/ipfs/bafy…/"
        }
      ]
    }
  ],
  "replicas_error": null,
  "unfollowed": {
    "remove_on_unfollow": true,
    "accounts": [ { "pubkey": "…", "npub": "…", "sites": [ { "...": "同じ形。ただし url・message・replicas は常に null、stored は常に true" } ] } ]
  }
}
```

- `follow_set.note`: CLI が `(relays returned an older follow set; ...)` のように括弧付きで出す注記から、括弧を外した文字列。無ければ `null`。
- `nip05`・`message`・`size` は state / イベントに値が無ければ `null`。`message` は生の `content`（サニタイズや切り詰めはしない。フロントの責務）。
- `replicas`: レプリカ報告の取得に失敗すると全サイトで `null` になり、`replicas_error` に理由が入る。
- `gateway_url`: `stored` が true かつ gateway 設定がある版だけに付く。`unfollowed` 側は state にある版＝常に保存済みなので、gateway 設定さえあれば付く。

### GET /api/status

`health::collect_status` の結果。**relay には接続しない**（`state.json` と Kubo だけを見る、CLI の `swing status` と同じ）。全版の DAG をたどるので重い。フロントも自動では呼ばない。

```json
{
  "versions": [
    { "pubkey": "…", "npub": "…", "d": "example.com", "path": "/swing/…", "cid": "bafy…",
      "size": 123, "created_at": 1, "health": "ok", "detail": null },
    { "pubkey": null, "npub": null, "d": null, "path": null, "cid": "bafy…",
      "size": null, "created_at": null, "health": "invalid_key", "detail": "<state.json の生のキー>" }
  ],
  "garbage": [ { "path": "/swing/…", "list_failed": false } ],
  "problems": 0
}
```

`health` は `ok` / `missing` / `cid_mismatch` / `incomplete` / `check_failed` / `invalid_key` のいずれか。`ok` 以外は `detail` に理由の文字列が入る。`invalid_key` は `state.json` の `sites` のキーが `<pubkey hex>:<d>` の形式として不正だった場合（`health::StatusLine::InvalidKey`）で、`pubkey`・`npub`・`d`・`path`・`size`・`created_at` はすべて `null` になり、`cid` だけ分かれば入り、`detail` に元のキー文字列が入る。問題があっても HTTP は常に 200（`problems` の件数で分かる）。

### GET /api/mirror

```json
{ "title": "SWING mirror list", "note": null, "members": [ { "pubkey": "…", "npub": "…" } ] }
```

Follow Set が無ければ `title: null`、`members: []`。

### POST /api/mirror/add, POST /api/mirror/remove

リクエスト: `{ "keys": ["npub1…", "hex…", "nprofile1…"] }`。`keys` が空、100 件を超える、またはどれか 1 つでもパースできなければ 400 で何もしない。

```json
{
  "changed": [ { "pubkey": "…", "npub": "…" } ],
  "unchanged": [ { "pubkey": "…", "npub": "…" } ],
  "published": true,
  "relays": [ { "relay": "wss://…", "ok": true, "error": null } ],
  "members": [ { "pubkey": "…", "npub": "…" } ]
}
```

- `changed`: 実際に追加・削除したもの。`unchanged`: 既に追加済み／もともと未登録で no-op だったもの。
- 変更が無ければ `published: false`、`relays: []`（relay へは送らない）。
- `published: true` なのにどの relay にも受理されなければ 502。
- 成功（`published: true` かつ 1 relay 以上が accept）したら `notify.notify_one()` を呼び、agent に即時 refresh を促す。

### GET /api/webring?root=\<key\>&depth=\<N\>

`root` は 0〜100 回（省略時は自分の pubkey 1 つ、101 回以上は 400）、繰り返し指定できる。`depth` は省略時 2、4 を超えるかパースできなければ 400。

```json
{
  "depth": 2,
  "nodes": [
    { "pubkey": "…", "npub": "…", "short_npub": "npub1abcdefg…uvwxyz",
      "names": ["example.com"], "label": "example.com",
      "depth": 0, "root": true, "has_follow_set": true }
  ],
  "edges": [ { "from": "<hex>", "to": "<hex>", "mutual": true } ],
  "beyond": 0,
  "text": "…swing webring と同じ text 出力…",
  "dot": "…同じ dot 出力…",
  "mermaid": "…同じ mermaid 出力…"
}
```

`root: true` は `depth == 0` のノード。双方向の組は `mutual: true` の辺 1 本に、片方向は `mutual: false` の辺にする（`webring::split_links` を流用）。`label` は CLI の webring と同じ表示名。ノードの並びは（深さ、ラベル）順。

### GET /api/replicas?key=\<key\>

`key` は 0〜100 回（省略時は自分 1 つ、101 回以上は 400）。

```json
{
  "authors": [
    { "pubkey": "…", "npub": "…",
      "sites": [
        { "d": "example.com", "cid": "bafy…", "replicas": 2, "reports": 3,
          "reporters": [ { "pubkey": "…", "npub": "…", "latest": true, "is_author": false, "following": true } ] }
      ] }
  ]
}
```

`following: false` が CLI の `[not following]` に相当する。

### POST /api/publish/upload

`multipart/form-data`。ガードは他の書き込み系と同じ（`X-Swing-Dashboard: 1` ヘッダと Origin 検証。`Content-Type` は multipart なので `AppJson` の JSON 判定は関係しない）。

パート（テキストパートはファイルより先に送る想定）:

- `site`（必須）・`url`・`message`・`nip05`（`url`/`message`/`nip05` は省略可、`nip05` 省略時は `[publish].nip05`）。`site` は `d` タグの制約、`url` を指定するなら http/https URL であることを CLI と同じ規則で検証し、違反は 400。
- `file`（1 個以上）: 各パートの `filename` がサイトルートからの相対パス（`/` 区切り。ブラウザは `webkitRelativePath` の先頭フォルダ名を取り除いて送る）

サーバの検証（`upload::validate_relative_path`、違反はすべて 400 で何も書かない）:

- パスは非空、`/` で始まらない、`\` を含まない、制御文字を含まない
- パスの長さは `MAX_PATH_LEN`（4096 バイト）以下、セグメント数（`/` の個数 + 1）は `MAX_PATH_SEGMENTS`（32）以下
- 各セグメントが非空・`.`・`..` のいずれでもない（`a//b`、`a/./b`、`a/../b`、絶対パス、相対パスの親ディレクトリ参照はすべて拒否）
- 同じパスが 2 回来たら 400、`file` パートが 0 個なら 400、`site` が無ければ 400
- `file` パートの総数は `MAX_UPLOAD_FILES`（10,000）まで。超えた時点で以降のパートを読まずに 400 を返す

いずれの上限も固定の定数（`src/dashboard/upload.rs`）で、設定項目にはしていない。`[dashboard].max_upload`（ボディサイズ）と組み合わせても、1 パートあたりのバイト数を小さくして大量のファイル・深いディレクトリを作らせる DoS（ディスク/inode 枯渇）を防ぐのが目的。超過時はどの検証もアップロード先の展開ディレクトリを丸ごと削除してから 400 を返す（成功・失敗どちらでも同じ後始末経路を通る、下記「処理」参照）。

同時に実行できる publish は 1 本だけ（`AppState.publish_lock` を `try_lock`）。実行中にもう 1 本来たら 409。

処理:

1. `<state_dir>/upload/<ランダム名>/`（現在時刻のナノ秒・プロセス ID・カウンタから作る名前）を作り、各 `file` パートをストリーミングで書き込む（メモリに全体を載せない）。
2. `api::run_publish`（NIP-05 検証 → Kubo に add して MFS に置く → サイトイベントを署名して送信 → 古い版を `[publish].keep_versions` 個まで残して削除）を、展開先ディレクトリをサイトのディレクトリとして呼ぶ。処理順は CLI の `swing publish` と同じ。削除に失敗した版は `prune_error` に理由文字列が入るだけで、レスポンス全体は成功扱い。
3. **成功でも失敗でも**展開先ディレクトリを削除する（`tokio::fs::remove_dir_all`。削除に失敗したら warn ログを出すだけで、エラーはレスポンスに影響しない）。`<state_dir>/upload/` 自体は agent 起動時に丸ごと掃除される（前回の異常終了で残った分の後始末）。
4. ボディが `[dashboard].max_upload` を超えたら 413（`axum::extract::DefaultBodyLimit` をこのルートだけに `layer` している。ストリーミング中に超えた場合も打ち切って 413 にする。413 かどうかは multipart のエラーチェーンに `"length limit"` という文字列が含まれるかで判定している）。

```json
{
  "site": "example.com", "url": "https://example.com/", "message": "note",
  "nip05": { "status": "verified", "detail": null },
  "cid": "bafy…", "size": 12345, "created_at": 1790000000,
  "mfs_path": "/swing/publish/<hex>/example.com/1790000000",
  "relays": [ { "relay": "wss://…", "ok": true, "error": null } ],
  "pruned": ["1780000000"],
  "prune_error": null,
  "gateway_url": "http://127.0.0.1:8080/ipfs/bafy…/",
  "files": 3
}
```

- `nip05.status` は `off` / `verified` / `mismatch` / `not_applicable` / `error`。
- `require` で検証が通らなければ、add する前に 422 を返す: `{ "error": "...", "nip05": { "status": "...", "detail": "..." } }`。
- どの relay にも受理されなければ 502（Kubo に add した内容と古い版はそのまま残す）。
- `files` は受け取ったファイル数（`PublishUploadResultDto`、`#[serde(flatten)]` で他のフィールドは publish の結果そのもの）。

### GET /api/publish/sites

自分（agent の鍵）が過去に公開したサイトの一覧。relay から自分の pubkey のサイトイベントを取得し、`d` ごとの最新版を `d` の順に返す。

```json
{ "sites": [ { "d": "example.com", "url": "https://example.com/", "cid": "bafy…", "size": 123, "created_at": 1790000000, "message": null, "gateway_url": "http://127.0.0.1:8080/ipfs/bafy…/" } ] }
```

`gateway_url` は gateway 設定があれば付ける（自分が publish したサイトは自分の Kubo にあるはずなので、`sites`/`replicas` のような `stored` 判定はしない）。relay の取得に失敗したら 502。state.json は見ない（`swing sites` の「自分の行」とは取得経路が異なるが、同じ relay データを見るので内容は一致する）。

### GET /api/config

```json
{
  "config_path": "/path/to/swing.toml",
  "sections": [
    { "name": "nostr", "items": [
      { "key": "secret_key", "env": "SWING_NOSTR_SECRET_KEY", "value": "(set, hidden)" },
      { "key": "relays", "env": "SWING_NOSTR_RELAYS", "value": ["wss://…"] }
    ] },
    { "name": "policy", "items": [
      { "key": "max_total_storage", "env": "SWING_MAX_TOTAL_STORAGE", "value": 107374182400, "display": "100 GB" },
      { "key": "min_update_interval", "env": "SWING_MIN_UPDATE_INTERVAL", "value": 300, "display": "5m" }
    ] }
  ]
}
```

- `sections` は `nostr` / `ipfs` / `policy` / `agent` / `publish` / `dashboard` の順で、[`architecture.md`](../architecture.md#設定と環境変数) の設定表と同じキーを同じ順で列挙する。TOML キーが無い `SWING_FETCH_TIMEOUT` / `SWING_FETCH_IDLE_TIMEOUT` は `agent` セクションに `key: null` で入る（`env` だけがある）。
- `secret_key` は常に文字列 `"(set, hidden)"` で、実際の値は型としても存在しない（下の「秘密鍵を出さない仕組み」を参照）。
- `value` は文字列・真偽・数値・文字列配列（`ConfigValue` の `#[serde(untagged)]`）で、常に生の値（バイト数・秒数など）。
- 容量・時間の項目には、人が読める文字列を追加の任意フィールド `display` に添える（`dto::format_bytes` / `dto::format_duration_secs`）。無い項目は `display` フィールドごと出ない（`#[serde(skip_serializing_if = "Option::is_none")]`）。
  - 容量（`max_total_storage`・`max_per_site`・`max_per_account`・`max_update_size`・`dashboard.max_upload`）: 1024 基数で最大の単位に割り切れれば `"100 GB"`、割り切れなければ小数第 1 位まで（`"1.5 KB"`）、KB 未満は `"<n> B"`。
  - 時間（`min_update_interval`・`nip05_cache_ttl`・`poll_interval`・`report_ttl`・`SWING_FETCH_TIMEOUT`・`SWING_FETCH_IDLE_TIMEOUT`）: 日/時/分のどれかで割り切れれば `"5m"` のように大きい単位優先、割り切れなければ秒（`"90s"`）。
  - `max_sites_per_account`・`keep_versions`・`keep_days`・`concurrency` は素の個数・日数なので `display` は付かない。
- `config_path` は実際に読んだ設定ファイルのパス。環境変数だけで動いているなら `null`。

## 画面（`web/index.html`, `web/*.js`）

ビルド工程なし・外部依存なしの素の HTML + CSS + ES modules。フロントは役割ごとに次のファイルへ分かれている（依存は下から上への一方向で、循環 import は無い）。

| ファイル | 役割 |
|---|---|
| `storage.js` | `localStorage` の薄いラッパー（`get`/`set`/`remove`、例外を握りつぶす）。他のどのモジュールにも依存しない |
| `i18n.js` | `MESSAGES`（en/ja 辞書）・`t()`・`currentLang()`・`applyStaticI18n()`（`data-i18n*` 属性への流し込み）。`storage.js` にだけ依存する |
| `util.js` | `cache`（全画面で共有する取得結果のキャッシュ）、`el()`・`clamp()` などの DOM/汎用ユーティリティ、`apiFetch`・`copyWithFeedback`・`setBusy` などの共通処理、`getStyle`/`setStyle`/`wireStyleSwitch`/`wireSortSwitch`（表示スタイル・並び順切替の配線）、`createLoadGuard()`（世代カウンタ付き非同期ロードのガード）。`storage.js`・`i18n.js` に依存する |
| `ui.js` | 複数画面で共有する UI 部品（`copyButton`・`storedBadge`・`appendLinksAndMessage`・`renderRelayResults`・`renderMirrorOpResult`・`renderOpError`・`buildRemoveControl`）。`util.js`・`i18n.js` に依存する |
| `graph.js` | webring 用の自前 force-directed layout（`createWebringGraph`）。`util.js` の `clamp` だけに依存する |
| `sites.js` | Sites 画面（`SitesView`、mirror 追加・削除、Storage check） |
| `webring.js` | Webring 画面（`WebringView`、ノード詳細、`graph.js` を利用） |
| `publish.js` | Publish 画面（`PublishView`、My sites、フォルダアップロード） |
| `settings.js` | Settings 画面（`SettingsView`、テーマ・言語・カスタム CSS） |
| `app.js` | ルーター兼エントリポイント（`VIEWS`・`showRoute`・`applyLanguage`・`init`）。`<script type="module" src="/app.js">` から読み込まれ、他の画面モジュールを import する起点 |

`#/sites` `#/webring` `#/publish` `#/settings` の 4 画面をハッシュルーティングで切り替える（`location.hash` → `document.body.dataset.view`。既定は `sites`）。書き込みリクエストには `X-Swing-Dashboard: 1` と `Content-Type: application/json` を付ける（`apiFetch`）。relay 由来の文字列は DOM API（`textContent` / `el()` ヘルパ）だけで挿入し、`innerHTML` は使わない。`url` は `^https?://` にマッチするときだけ `<a>`（`rel="noopener noreferrer" target="_blank"`）にする。

言語切り替え（Settings 画面のセレクタ）は `settings.js` が `swing:langchange` という `CustomEvent` を `document` に投げ、`app.js` がそれを購読して各画面の再描画（`applyLanguage`）をまとめて行う。これは `settings.js` → `app.js` → `settings.js` の循環 import を避けるための構成で、`applyLanguage` 自体は `sites.js`/`webring.js`/`publish.js`/`settings.js` の描画関数をすべて呼べる `app.js` 側に置いている。

各画面の取得は世代カウンタ付きの非同期ロードでガードしている（`util.js` の `createLoadGuard()` が返す `{ start(), isCurrent(gen) }` を各画面のロード関数が使う）。呼び出しのたびに `start()` でカウンタをインクリメントし、`fetch` が返ってきた時点で `isCurrent(gen)` が false なら描画せずに捨てる。画面を素早く切り替えたり、webring の root/depth を続けて変えたり、グラフのノードを連続でクリックしたりしても、古いレスポンスが新しい画面の上に描画されない。

再読み込みの UI は画面ごとに異なる。UI 調整で「画面の下に大きな Reload ボタン」という統一パターンをやめ、各画面に合う形にした:

- Sites: 見出し `<h1>` の横にアイコンボタン（`.swing-icon-btn` + `<svg><use href="#icon-reload"></use></svg>`、`index.html` の `<symbol id="icon-reload">`）。
- Webring: 専用の Reload は無く、クエリフォームの「Update」（送信で現在の root/depth のまま再取得）が兼ねる。
- Publish: 画面自体に Reload は無い。`My sites` サブセクションだけ、その見出し `<h2>` の横に Sites と同じアイコンボタンを持つ。
- Settings: Reload は無い（画面を開いたときに読み込むだけ）。

| 画面 | 内容 | 表示スタイル（`data-style`、localStorage キー `swing:style:<view>`） |
|---|---|---|
| Sites | `/api/sites` の一覧、アカウントごとの「mirror から外す」、上部の「mirror に追加」フォーム（送信前にブラウザ側でも 100 件（`MAX_MIRROR_KEYS`）を超えていないか確認し、超えていればサーバに送らずエラー表示する）、`Unfollowed but still stored` セクション、ボタンを押したときだけ走る Storage check（`/api/status`、`data-health="invalid_key"` の行は danger 配色） | `list`（既定）/ `cards` は実装上は list と同じ要素を使う簡易版 / `table` |
| Webring | `/api/webring` を root・depth 指定で取得。ノードを選ぶと `/api/replicas?key=` を引いて右パネルに `short_npub`（クリックで完全な npub をコピー）とサイトごとのレプリカ、ミラー操作ボタンを表示する | `graph`（既定、SVG の自前 force layout）/ `list`（Accounts・Mutual・One-way）/ `ascii`（`text` を `<pre>`）/ `source`（`dot` と `mermaid` を `<pre>` + コピー） |
| Publish | 自分の npub/hex/relays/mirror_set（`/api/overview`）、`My sites`（`/api/publish/sites`）、publish フォーム（常にフォルダアップロード）、結果表示 | スタイル切替なし |
| Settings | `/api/config` をセクションごとの表で表示（読み取り専用）。`display` がある項目はそれを主表示にし、生の値（バイト数・秒数など）を小さく併記する。ブラウザ側のテーマ・言語・カスタム CSS の設定 | スタイル切替なし |

### Sites 画面の詳細

- ツールバーは 2 段（どちらも `.swing-toolbar.swing-toolbar-row`）。1 段目: 表示スタイル切替（`.swing-style-switch`）、並び順切替（`.swing-sort-switch`）、右端寄せ（`.swing-toolbar-end`）の「Stored only」チェックボックス（`localStorage["swing:sites:stored-only"]`、`"1"`/`"0"`）。2 段目: 絞り込みテキスト入力。
- 並び順（`.swing-sort-switch`、`localStorage["swing:sites:sort"]`、既定 `updated`）:
  - `updated`: アカウントの最初のサイトを `created_at` 降順で比べて並べる（各アカウント内のサイトも同じ基準で降順に並べ替える）。サイトを 1 つも持たないアカウントは最後。
  - `name`: 最初のサイトの `d` のロケール順（`localeCompare`）。アカウント内のサイトも同じ基準で並べ替える。サイトを持たないアカウントは最後。
  - `pubkey`: 並べ替えをしない（`/api/sites` が返す順、`pubkey` の hex 順）。
  - `Unfollowed but still stored` セクションも同じ並び順ロジックを共有する。
- サイトのメタ情報は 2 行に分かれている: `.swing-site-meta-cid`（cid とコピーボタン）と `.swing-site-meta-info`（サイズ・更新時刻）。
- `cards` は `grid-template-columns: repeat(auto-fill, minmax(320px, 1fr))`。`table` はテーブル自体に `min-width: 640px` を敷き、狭い画面では横スクロールになる（`overflow-x: auto`）。

### Publish 画面の詳細

- **My sites**（`.swing-my-sites`）: `GET /api/publish/sites` の一覧を `d`・url・更新時刻・開くリンク・「Use」ボタンで表示する。「Use」（i18n: 英語 "Use these settings" / 日本語「このサイト設定を使う」）は `site` と `url`（`message` は含めない）を下のフォームに入れるだけ。空なら「No sites published yet.」。
- **publish フォームは常にフォルダアップロード**（「Path on agent host」に相当する UI は撤去済み）。`<input type="file" webkitdirectory multiple>` でフォルダを選ぶと、ファイル数と合計サイズを表示し、`/api/overview` の `max_upload` を超えていれば送信ボタンを無効化して理由を表示する。送信は `POST /api/publish/upload` へ `XMLHttpRequest` で行い、`upload.onprogress` を `.swing-progress`/`.swing-progress-bar`（`data-state="uploading|processing|done|error"`）に反映する。各ファイルの送信名は `webkitRelativePath` から先頭セグメント（選んだフォルダ名）を除いたもの。同じフォルダを選び直せるよう、送信後に `value = ''` でリセットする。
- 送信中はフォーム全体を無効化する。413（`max_upload` 超過）は「上限を超えた」という文言に言い換えて表示する。
- 最後に使った `site`・`url`・`message`・`nip05` を publish 成功時に `localStorage["swing:publish:last"]` に JSON（`{site, url, message, nip05}`）で保存し、次回開いたときにプリフィルする。`dir` やソースの種類は保存しない（常にアップロードのため不要）。

### Webring 画面の詳細

- root と depth は送信のたびに `localStorage["swing:webring:query"]` に `{root, depth}`（`root` はフォームの生の文字列）で保存し、次にこの画面を開いたときにフォームへ復元してその条件で取得する。
- 再取得中、画面に何かすでに表示されていれば、それを消さずに `aria-busy="true"` を付けて薄く表示する（`.swing-webring-content[aria-busy="true"] { opacity: 0.6; }`）。初回（まだ何も表示していない）ときだけ「Loading webring…」のテキスト表示になる。
- ノード詳細パネルのミラー操作は、選んだノードが自分自身かどうかで変える。判定は `cache.sites`（`/api/sites` の結果、無ければ `cache.mirror`、それも無ければ `GET /api/mirror` を取得）の pubkey 集合。
  - 自分自身: ミラー操作のボタンは出さない。
  - ミラー済み: `.swing-badge[data-mirrored="true"]` バッジと「ミラーから削除」ボタン（確認ステップ付き、Sites 画面の「mirror から外す」と同じ `buildRemoveControl`）。
  - 未ミラー: 「ミラーに追加」ボタン。
- 「Fit」ボタンの表示文字列は `createWebringGraph()` に渡す `labels`（`webring.js` の `t()` で作る）経由で i18n されており、英語は "Fit to view"、日本語は「全体表示」。

### Webring のグラフ（`web/graph.js`）

外部ライブラリを使わない自前の force-directed layout。

- ノードは反発力（`REPULSION` 2600）、辺はばね（目標距離 `LINK_DISTANCE` 130px）、中心に弱い引力。`alpha` を毎フレーム減衰させ、動きが小さくなったら `requestAnimationFrame` を止める。`prefers-reduced-motion: reduce` のときはアニメーションせず、`requestAnimationFrame` を使わずに収束するまで同期的にステップを回してから 1 回だけ描画する（`runSync`）。
- ラベルは 16 文字（`LABEL_MAX`）で省略し、全文は `<title>`（ホバー時のツールチップ）で見せる。文字サイズはズーム倍率で逆補正しており、`--swing-graph-label-size`（既定 11px）から計算した基準サイズを毎フレーム `zoom` で割ることで、ズームしても画面上はほぼ同じ大きさに見えるようにしている。凡例（下記）のラベルも同じ変数を使うので、サイズを変えれば両方に反映される。
- ノード（`<g class="swing-node">`）をポインタでドラッグして固定でき（`fx`/`fy`）、クリック（動かさずに離した場合）で選択して右パネルを開く。ラベルの文字もノードの `<g>` の子要素なので、文字の上でクリックしても同じく選択できる。
- キーボード操作: 各ノードに `tabindex="0"` `role="button"` `aria-label="<ラベル>"` を付け、Tab で移動して Enter または Space で選択できる。フォーカス時は `.swing-node:focus-visible circle` を `--swing-focus` で縁取りする。
- `svg.swing-graph` 全体をポインタドラッグでパン、ホイールでズーム（0.15〜4 倍、カーソル位置基準）。「Fit」ボタン（表示文字列は i18n、上記）で全ノードが収まる位置に戻す。`ResizeObserver` でグラフ枠のサイズ変化を監視し、パン/ズームを手動で操作していない間はリサイズのたびに自動で再 fit する。
- 辺（`<line class="swing-edge">`）は `pointer-events: none` で、辺の上にポインタを置いてもノードやラベルのクリック・ドラッグを妨げない。片方向の辺は終点だけに矢印（`marker-end`）、双方向の辺は両端に矢印（`marker-start` と `marker-end`）を付け、矢印の塗りはそれぞれ `.swing-arrow-oneway-fill` / `.swing-arrow-mutual-fill`。
- 矢印マーカーは `markerUnits="userSpaceOnUse"` で辺の太さやズームに関わらず固定サイズ（`ARROW_LEN` 8px 四方、`refX="0"` で線の終点に矢印の根元を合わせる）。線自体はノードの円周（半径 `NODE_RADIUS` 10px）に加えて矢印の長さ分（片方向は着地側だけ、双方向は両側）短くしてから引くので、矢印の下に線がめり込まず、矢印の底辺できっちり止まる。
- グラフの下に凡例（`.swing-legend`、横並びの `.swing-legend-item` とその中の `.swing-legend-swatch`）を出し、root・mutual・one-way・no-follow-set の見分け方を示す。凡例のラベル文字列も `createWebringGraph()` の `labels` 引数（i18n 済み）から取る。凡例内の `.swing-node` スウォッチには `pointer-events: none` を付けて、クリック可能に見えないようにしている。
- 色は付けず、`class="swing-node"` / `class="swing-edge"` と `data-root` / `data-has-follow-set` / `data-depth` / `data-selected` / `data-mutual` だけを付ける。色は CSS 側の役目（下記）。

## 共通の UI 部品

- **busy 表示**: 時間のかかる操作を起こすボタンは `setBusy(button, true/false)` で `disabled` 属性・`aria-busy="true"`・`.is-busy` class を切り替える。`ensureBusyStructure()` がボタンの中身を初回だけ `<span class="swing-btn-label">`（元のテキスト/アイコン）と `<span class="swing-btn-spinner" aria-hidden="true">`（絶対配置の回転スピナー）に組み替えるので、busy 中もボタン幅が変わらない。スピナーは `prefers-reduced-motion: reduce` では回転を止め、代わりに一部だけ塗り残す静止表示にする。
- **コピー**: `copyWithFeedback(button, text)` が `navigator.clipboard.writeText` を呼び、成功なら 1.5 秒だけボタンのラベルを「コピー済み」相当の文字列に変え `data-copied="true"`、失敗なら `data-copy-failed="true"` を付ける（どちらも 1.5 秒後に外れて元の表示に戻る）。コピー系ボタンの表示文字列は用途によらずすべて `t('copy')`（英語 "Copy" / 日本語「コピー」）で統一し、コピー対象の違いは `aria-label`（例: npub のコピーボタンなら `copyNpub`）で表す。
- **ボタンのサイズ体系**（CSS コメントにも明記）: `.swing-btn`（フォーム送信・主要な操作）と、それより小さい `.swing-btn-small`（見出し右・行単位の操作）・`.swing-copy-btn`（インラインコピー、`-small` とは独立）はテキストボタン、`.swing-icon-btn` はアイコンのみの正方形。4 つとも `font-family: var(--swing-font-ui)` を自分で指定しており、置かれた場所の等幅/serif フォントを継承しない。
- **フォーム部品**: `color-scheme` は `<html>` は `light dark`、`:root[data-theme="light"]`/`[data-theme="dark"]` でそれぞれ `light`/`dark` に固定する（テーマ切り替えでチェックボックスやスクロールバーなどネイティブ部品の配色も追従させ、ライトテーマなのにネイティブ部品だけ黒くなるような食い違いを防ぐ）。`accent-color: var(--swing-accent)`（チェックボックス・ラジオ・`<progress>`）。`input[type="file"]::file-selector-button` はダッシュボードの他のボタンと揃えたスタイル。
- **モバイル幅のフッタ**: 画面幅 760px 以下では `.swing-nav-footer`（サイドナビ内のミラーセット名・バージョン表示）を隠し、代わりにページ最下部の `<footer id="page-footer" class="swing-page-footer">` に同じ内容を 1 行で表示する。

## localStorage キー一覧

ダッシュボードが使う `localStorage` のキーはこれで全部（他にサーバに送るものは無い）。

| キー | 値の形 | 意味 |
|---|---|---|
| `swing:style:<view>`（`view` は `sites`/`webring`） | 文字列（スタイル名） | 画面ごとの表示スタイル |
| `swing:sites:sort` | `updated` / `name` / `pubkey` | Sites の並び順（既定 `updated`） |
| `swing:sites:stored-only` | `"1"` / `"0"` | Sites の「Stored only」チェックボックスの状態 |
| `swing:webring:query` | JSON `{root, depth}` | Webring の最後のクエリ（起動時に復元） |
| `swing:publish:last` | JSON `{site, url, message, nip05}` | Publish フォームの最後の入力（起動時にプリフィル） |
| `swing:theme` | `auto` / `light` / `dark` | 表示テーマ |
| `swing:lang` | `auto` / `en` / `ja` | 表示言語 |
| `swing:user-css` | 文字列（CSS） | Settings のカスタム CSS 欄の内容 |

## 表示言語（i18n）

`web/i18n.js` 内に `MESSAGES = { en: {...}, ja: {...} }` を持ち、`t(key, vars)`（`{vars}` プレースホルダを置換）で参照する。静的な HTML の文字列は `data-i18n`（テキスト）／`data-i18n-placeholder`（`placeholder` 属性）を付けた要素に対して起動時とビュー描画時に流し込む。

- 言語の決定: `localStorage["swing:lang"]`（`auto`/`en`/`ja`、Settings 画面のセレクタで変更）。`auto` のときは `navigator.language` が `ja` で始まるかどうかで判定する。切り替えは再読み込み不要。
- 訳が無いキーは英語にフォールバックする。
- **訳さないもの**: ナビゲーションの「Webring」（見出しも含め `data-i18n` を付けていない）、webring の ASCII/DOT/Mermaid のテキスト出力そのもの、API が返すエラー文字列、npub・hex・CID・パスのような値、SWING のワードマーク、環境変数名・設定キー名、`nip05`/`health` のステータス値（`verified`/`mismatch`/...、`ok`/`missing`/...）、NIP-05 モードの `off`/`warn`/`require`。
- 日本語訳は「mirror」を指す語をすべて「ミラー」に統一している（「ミラーに追加」「ミラーから削除」「ミラーリスト」「ミラーセット: swing」など。UI 調整ラウンドで訳語のばらつきを揃えた）。
- コピー系ボタンの表示文字列は対象によらず共通の `copy`（"Copy"/「コピー」）キー 1 つで、対象の違いは `aria-label` 側の個別キーで表す（上記「共通の UI 部品」参照）。
- 日時表示（`formatTime`）は `Intl.DateTimeFormat`（`ja-JP` / `en-US`）を使う。

## CSS カスタマイズのインターフェース

読み込み順は `style.css` → `/custom.css`（サーバ設定） → `<style id="user-css">`（ブラウザの localStorage、後勝ち）。

既定のデザインは白黒中立基調＋不透明度で段階を付けたグレー（`--swing-surface-alt`・`--swing-border` などは黒/白に対する alpha）に、アクセント 1 色（インクブルー、`--swing-accent: #1f5aa8`）だけを差す配色にしている。フォントは serif の見出し用フォントをやめて `system-ui` 系のみに統一した。`--swing-root`（webring のグラフの root ノードの色）は独立した色ではなく `var(--swing-accent)` を参照しているので、アクセント 1 色を変えるだけで揃って変わる。ロゴの回転アニメーションは廃止し、テーブルは 1 行おきに `--swing-surface-alt` で塗る（`.swing-table tbody tr:nth-child(even)`）。CSS 変数の名前・class 名・data 属性はこの見た目の変更でも変わっていない。

- **CSS 変数**（`web/style.css` の `:root`）: `--swing-bg` `--swing-surface` `--swing-surface-alt` `--swing-surface-raised` `--swing-border` `--swing-border-strong` `--swing-text` `--swing-text-muted` `--swing-text-faint` `--swing-accent` `--swing-accent-strong` `--swing-accent-soft` `--swing-warn` `--swing-warn-soft` `--swing-danger` `--swing-danger-soft` `--swing-ok` `--swing-ok-soft` `--swing-root`（`--swing-accent` を参照） `--swing-focus` `--swing-font-display` `--swing-font-ui` `--swing-font-mono` `--swing-space-1`〜`--swing-space-6` `--swing-radius` `--swing-radius-lg` `--swing-nav-width` `--swing-graph-label-size`（既定 11px。webring のグラフのラベルと凡例の文字サイズ）。
- **テーマ**: 既定は `@media (prefers-color-scheme: dark)` に連動。`<html data-theme="light">` / `data-theme="dark"` で上書きできる（Settings 画面のテーマ選択が `localStorage["swing:theme"]` に保存し、この属性を付け替える）。
- **状態フック**: `<body data-view="sites|webring|publish|settings" data-style="<現在の表示スタイル>">`。
- **安定 class**（抜粋。命名は `swing-` 接頭辞で統一）: レイアウト系 `swing-shell` `swing-nav` `swing-main` `swing-view` `swing-view-head` `swing-heading-row` `swing-panel` `swing-toolbar` `swing-toolbar-row` `swing-toolbar-end` `swing-inline-actions` `swing-style-switch` `swing-sort-switch`。Sites 系 `swing-site` `swing-site-row` `swing-site-name` `swing-site-meta` `swing-site-meta-cid` `swing-site-meta-info` `swing-site-message` `swing-site-links` `swing-account` `swing-account-head` `swing-account-actions` `swing-site-set`。共通部品 `swing-badge` `swing-btn`（`swing-btn-accent` / `swing-btn-danger` / `swing-btn-small`）`swing-copy-btn` `swing-icon-btn` `swing-icon` `swing-btn-label` `swing-btn-spinner` `swing-status` `swing-hint` `swing-table` `swing-mono` `swing-pre` `swing-plain-list` `swing-copyable` `swing-relay-results` `swing-relay-result` `swing-relay-mark` `swing-page-footer`。Webring 系 `swing-graph` `swing-graph-wrap` `swing-graph-controls` `swing-node` `swing-edge` `swing-arrow-oneway-fill` `swing-arrow-mutual-fill` `swing-legend` `swing-legend-item` `swing-legend-swatch` `swing-webring-list` `swing-webring-group` `swing-webring-account-row` `swing-webring-layout` `swing-node-detail` `swing-node-detail-actions` `swing-source-block`（`source` 表示スタイルの dot/mermaid ブロック。webring 専用で、下記の Publish の「Source」入力トグルとは無関係）`swing-source-block-head`。Publish 系 `swing-my-sites` `swing-progress` `swing-progress-bar` `swing-identity`。UI 調整でパス指定モードの切り替え UI（`swing-source-switch` / `swing-source-panel`）は削除した。
- **状態は data 属性**: `data-stored="true|false"`（サイト行・バッジ）、`data-nip05="verified|mismatch|not_applicable|error"`（バッジ）、`data-health="ok|missing|cid_mismatch|incomplete|check_failed|invalid_key"`（status check の行・バッジ。`invalid_key` は他の異常系と同じ danger 配色）、`data-ok="true|false"`（relay 結果）、`data-kind="loading|error|empty"`（`swing-status`）、`data-root="true|false"` / `data-has-follow-set="true|false"` / `data-depth="<N>"` / `data-selected="true|false"`（グラフのノード）、`data-mutual="true|false"`（グラフの辺）、`data-style-value`（スタイル切替ボタン自身の値）、`data-sort-value`（並び順切替ボタン自身の値）、`data-mirrored="true"`（webring 詳細パネルのミラー済みバッジ）、`data-detail="true|false"`（`.swing-webring-layout`、詳細パネルを表示中か）、`data-copied="true|false"` / `data-copy-failed="true|false"`（コピーボタン、1.5 秒だけ）、`data-state="uploading|processing|done|error"`（`swing-progress`）、`aria-busy="true"`（busy 中のボタンと、再取得中の webring 表示領域）。
- SVG グラフは class と `data-*` だけを付け、色は一切 JS に書かない（すべて `style.css` 側で `.swing-node[data-root="true"] circle { ... }` のように当てる）。キーボードフォーカスの色も `--swing-focus` という専用の CSS 変数を介しているので、アクセントと別の色にしたい場合はそこだけ変えられる。

## 秘密鍵を出さない仕組み

- `config::Config`（および `NostrConfig`）に `Serialize` を実装していない。DTO は手書きの構造体（`dto::ConfigDto` など）で、`secret_key` の実値を持つフィールドが型として存在しない。`/api/config` は常に固定文字列 `"(set, hidden)"` を返す。
- `Config::Debug` も `NostrSecretKey` の `Debug` 実装で `<redacted>` になる（ログにも出ない）。
- 表示するのは `npub` / hex 公開鍵のみ。nsec は API のどのエンドポイントにも登場しない。
