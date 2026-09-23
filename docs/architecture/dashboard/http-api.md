# ダッシュボード HTTP API（`src/dashboard/api.rs`, `src/dashboard/dto.rs`）

[`dashboard.md`](../dashboard.md) の一部。ガード・タイムアウトは [`dashboard.md`](../dashboard.md)、画面側からの使い方は [`web.md`](web.md) を参照。

## 共通

- すべて JSON。公開鍵は `pubkey`（小文字 hex）と `npub` を併記する。時刻は epoch 秒の整数。無い値は `null`。
- エラーは `{ "error": "<メッセージ>" }` とステータスコード。入力不正は 400、relay や Kubo が未準備（agent がまだ接続・確定していない）なら 503 `{"error": "agent is not ready"}`、鍵が未設定（セットアップモード。下記）なら同じ 503 で `{"error": "agent is not configured"}`、relay や Kubo・Nostr 発行の失敗は 502、publish の多重実行は 409。JSON の構文エラー・必須フィールド欠落・`Content-Type` 不一致はすべて 400（422 は publish の NIP-05 `require` 失敗専用。ボディが大きすぎる場合だけ 413）。`POST /api/publish/upload` だけ `multipart/form-data` を受ける。
- `keys`（mirror add/remove）・`root`（webring）・`key`（replicas）は 1 リクエストあたり最大 100 件、超えると 400。
- relay を引く API（sites・mirror・webring・replicas）はサーバ側でキャッシュせず、同時実行数の制限やレート制限も無い。
- API は `swing up` プロセスの寿命でずっと動く（[`../dashboard.md`](../dashboard.md#概要)）。relay・ipfs を使うエンドポイント（`/api/sites`・`/api/status`・`/api/mirror`・`/api/mirror/add`・`/api/mirror/remove`・`/api/webring`・`/api/replicas`・`/api/publish/sites`・`/api/publish/upload`）は agent が relay 接続と Kubo の URL 確定を終えるまで 503 を返す。鍵が未設定（セットアップモード。[`../up.md#セットアップモード鍵未設定`](../up.md#セットアップモード鍵未設定)）の間はこれらが常に 503 `agent is not configured` を返す（`dashboard::api::not_ready` が `AppState::setup_mode()` を見て `NotReady` と `NotConfigured` を切り替える）。`/api/overview`・`/api/config`・`/api/shutdown`・`/api/restart` は agent の準備状態に関わらず常に応答する。`/api/setup` はセットアップモードの間だけ `200`、それ以外は `409`（下記）。
- CLI の `swing status`・`swing mirror add`・`swing mirror remove`・`swing stop` はこの API のクライアント（`src/api_client.rs::ApiClient`）で、それぞれ `/api/status`・`/api/mirror/add`・`/api/mirror/remove`・`/api/shutdown`（`--restart` なら `/api/restart`）を叩く。`swing sites`・`replicas`・`webring`・`mirror list`・`publish` はこの API を経由せず relay/Kubo に直接つなぐ（[`../cli.md`](../cli.md)）。

## 既知の性質

- `POST /api/publish/upload` の 409 はダッシュボード内で同時に来た publish リクエストどうしだけを排他する。同じホスト上の CLI `swing publish` とは排他されない。
- `run_publish` の NIP-05 検証はプライベート/ループバック/リンクローカル等に解決されるホストへの接続を拒否する。接続エラーの詳細は `nip05.detail` に出さず、`unreachable`/`timeout`/`invalid_response` の粗い分類だけを返す（生のメッセージは `tracing::warn` にのみ出す）。

## GET /api/overview

```json
{ "version": "0.1.0", "setup": false, "pubkey": "ab12…", "npub": "npub1…", "relays": ["wss://relay.damus.io"], "mirror_set": "swing", "gateway": "http://localhost:8080", "started_at": 1790000000, "instance": "cdeee5bc85519f44", "max_upload": 2147483648 }
```

`gateway` は `[dashboard].gateway` が空なら `null`。`started_at` はダッシュボードが有効になった起動時刻。`instance` は `up::run` の回ごと（`AppState` を作るたび）に変わるランダムな 16 桁の 16 進文字列で、同じプロセスの中での再起動（`POST /api/restart`）も見分けられる（`started_at` は秒単位なので 1 秒以内の再起動では変わらない）。`swing stop --restart` が再起動の完了を待つのに使う（[`../service.md`](../service.md)）。`max_upload` は `[dashboard].max_upload` のバイト数。`setup` は鍵が未設定（セットアップモード）かどうかで、そのときは `pubkey`／`npub` も `null` になる（[`../up.md#セットアップモード鍵未設定`](../up.md#セットアップモード鍵未設定)）。フロント（`web/app.js`）はこれを見て、通常なら hash ルーティングするところをどのルートでも常に `#/setup` に固定する（[`web.md`](web.md)）。

## GET /api/sites

`mirror::collect_sites` をそのまま JSON にしたもの（CLI の `swing sites` と同じ集計。[`architecture/cli.md#sites`](../cli.md#sites)）。

```json
{ "follow_set": { "found": true, "note": null },
  "accounts": [ { "pubkey": "…", "npub": "…", "sites": [ { "d": "example.com", "cid": "bafy…", "url": "…", "size": 12345, "stored_size": 12300, "created_at": 1790000000, "title": "…", "message": "…", "nip05": "verified", "replicas": 3, "unverified_replicas": 0, "stored": true, "gateway_url": "…" } ] } ],
  "replicas_error": null,
  "unfollowed": { "remove_on_unfollow": true, "accounts": [ { "...": "同じ形。ただし url・title・message・replicas・unverified_replicas は常に null、stored は常に true、stored_size は size と同じ値" } ] } }
```

- `follow_set.note`: CLI が括弧付きで出す注記から括弧を外した文字列。無ければ `null`。
- `nip05`・`title`・`message`・`size` は値が無ければ `null`。`title` は作者の自己申告で受信側は信頼しない（`docs/protocol.md` 第 4 節）。`message` は生の `content`（サニタイズ・切り詰めはフロントの責務）。`title` も同様にサニタイズはフロントの責務。
- `size` はイベントの自己申告の `size` タグ。`stored_size` は `cid` と一致する `state.json` の `VersionRecord.size`（保存時に `dag/stat` で測った値。この呼び出しのために改めて Kubo は呼ばない）で、一致する版が無ければ `null`。フロントは `stored_size` があればそれを実測値として出し、無ければ `size` を未確認の申告として括弧書きで出す（[`web.md`](web.md#sites-画面)）。版ごとの重複排除込みの実測合計は `/api/status` の `sites[].actual` にしかない。
- `replicas`・`unverified_replicas`: レプリカ報告の取得に失敗すると全サイトで両方 `null` になり、`replicas_error` に理由が入る。`replicas` は報告者が作者自身か、作者かこちらの Follow Set に入っている報告者（信頼できる tier）の数、`unverified_replicas` はそれ以外（自称にすぎない tier）の数（[「レプリカ報告の信頼度」](../../architecture.md#レプリカ報告の信頼度replicastier)）。
- `gateway_url`: `stored` が true かつ gateway 設定がある版だけに付く。
- `accounts[].sites` は 1 アカウントあたり `d` の昇順で先頭 50 件（`nostr::budget::MAX_SITES_PER_AUTHOR_LISTED`）まで（[取得と表示の上限](../../architecture.md#取得と表示の上限nostrbudget)）。`follow_set` の `p` も先頭 500 件まで。

## GET /api/status

`health::collect_status` の結果（CLI の `swing status` と同じ集計。[`architecture/cli.md#status`](../cli.md#status)）。relay には接続しない。サイト単位で DAG をたどるので重く、フロントも自動では呼ばない。

```json
{ "versions": [
    { "pubkey": "…", "npub": "…", "d": "example.com", "path": "/swing/…", "cid": "bafy…", "size": 123, "created_at": 1, "health": "ok", "detail": null },
    { "pubkey": null, "npub": null, "d": null, "path": null, "cid": "bafy…", "size": null, "created_at": null, "health": "invalid_key", "detail": "<state.json の生のキー>" } ],
  "sites": [ { "pubkey": "…", "npub": "…", "d": "example.com", "path": "/swing/…", "actual": 123 } ],
  "actual_bytes": 123,
  "garbage": [ { "path": "/swing/…", "list_failed": false, "list_failed_reason": null } ], "problems": 0 }
```

`sites` はサイトごとの実容量。`actual` はそのサイトの全版をまとめた `dag/stat` の `TotalSize` で、版どうしで共有しているブロックは 1 回だけ数える。測れなかったサイトは `null`。`actual_bytes` は `actual` の合計で、`null` のサイトが 1 つでもあれば `null`。

`health` は `ok`/`missing`/`cid_mismatch`/`incomplete`/`check_failed`/`invalid_key`（CLI の判定を snake_case で返す）。`ok` 以外は `detail` に理由が入る。`invalid_key` は `state.json` のキーが `<pubkey hex>:<d>` の形式として不正だった場合で、`pubkey`・`npub`・`d`・`path`・`size`・`created_at` は `null`、`cid` だけ分かれば入り、`detail` に元のキー文字列が入る。問題があっても HTTP は常に 200（`problems` の件数で分かる）。

`garbage[].list_failed_reason`: 一覧に失敗したディレクトリ（`list_failed: true`）だけ理由の文字列が入る。`list_failed: false` なら常に `null`。CLI（`swing status`）は `[list failed]: <理由>` として表示する。

## GET /api/mirror

```json
{ "title": "SWING mirror list", "note": null, "members": [ { "pubkey": "…", "npub": "…" } ] }
```

Follow Set が無ければ `title: null`、`members: []`。

## POST /api/mirror/add, POST /api/mirror/remove

リクエスト: `{ "keys": ["npub1…", "hex…", "nprofile1…"] }`。空、100 件超、パース不能のいずれかで 400。

```json
{ "changed": [ { "pubkey": "…", "npub": "…" } ], "unchanged": [ { "pubkey": "…", "npub": "…" } ], "published": true, "relays": [ { "relay": "wss://…", "ok": true, "error": null } ], "members": [ { "pubkey": "…", "npub": "…" } ], "note": null, "follow_set_found": true }
```

- `changed`: 実際に追加・削除したもの。`unchanged`: 既に追加済み／もともと未登録で no-op だったもの。
- 変更が無ければ `published: false`、`relays: []`。`published: true` なのにどの relay にも受理されなければ 502。
- 成功（1 relay 以上が accept）したら agent に即時 refresh を促す。
- `note`: relay から取れた Follow Set より `state.json` に保存済みの版を使った場合の注記（`(relays returned an older follow set; ...)` / `(follow set not found on relays; ...)`）。括弧付きの文字列そのまま、無ければ `null`。`follow_set_found`: 操作前に Follow Set が見つかっていたか。CLI の `swing mirror remove` は Follow Set が無ければ（`follow_set_found: false`）`(no follow set found); no changes` とだけ表示して他のフィールドを見ない（[`../cli.md#mirror-list--add--remove`](../cli.md#mirror-list--add--remove)）。`add` はこの分岐を使わない（無い状態からの新規作成を許すため）。

## GET /api/webring?root=\<key\>&depth=\<N\>

`root` は繰り返し指定可、省略時は自分の pubkey 1 つ、100 件超で 400。`depth` は省略時 2、4 超か非数値で 400。たどり方・グラフの組み立ては CLI の `swing webring` と同じ（[`architecture/cli.md#webring`](../cli.md#webring)）。

```json
{ "depth": 2,
  "nodes": [ { "pubkey": "…", "npub": "…", "short_npub": "npub1abc…uvwxyz", "names": ["example.com"], "label": "example.com", "depth": 0, "root": true, "has_follow_set": true } ],
  "edges": [ { "from": "<hex>", "to": "<hex>", "mutual": true } ], "beyond": 0, "over_budget": 0,
  "referencing": { "accounts": [ { "pubkey": "…", "npub": "…" } ], "more": 0 },
  "text": "…swing webring と同じ text 出力…", "dot": "…同じ dot 出力…", "mermaid": "…同じ mermaid 出力…" }
```

`root: true` は `depth == 0` のノード。双方向の組は `mutual: true` の辺 1 本、片方向は `mutual: false` の辺（`webring::split_links` を流用）。ノードの並びは（深さ、ラベル）順。`beyond` は深さの上限の外にいて表示していないアカウント数。`over_budget` はクロールの上限（`nostr::budget::MAX_CRAWL_NODES`、1000）を超えたために crawl に加えなかったアカウント数（[取得と表示の上限](../../architecture.md#取得と表示の上限nostrbudget)）。`names` も 1 アカウントあたり先頭 50 件まで。
- `referencing`: `#p` で見つかった、起点を名指ししているだけでクロールには加えていないアカウント（[「レプリカ報告の信頼度」](../../architecture.md#レプリカ報告の信頼度replicastier)）。`accounts` は先頭 50 件（`nostr::budget::MAX_REFERENCING_LISTED`）まで、`more` は切り詰めで落ちた件数。`nodes`・`edges`・`dot`・`mermaid` には含まれない（グラフはフォロー先の辺だけで描く）。

## GET /api/replicas?key=\<key\>

`key` は繰り返し指定可、省略時は自分 1 つ、100 件超で 400。集計は CLI の `swing replicas` と同じ（[`architecture/cli.md#replicas`](../cli.md#replicas)）。

```json
{ "authors": [ { "pubkey": "…", "npub": "…", "sites": [
  { "d": "example.com", "cid": "bafy…", "replicas": 2, "unverified": 1, "reports": 3, "dropped": 0, "reporters": [ { "pubkey": "…", "npub": "…", "latest": true, "tier": "chosen" } ] }
] } ] }
```

`tier` は `"author"` / `"chosen"` / `"other"` のいずれか（`"other"` が CLI の `[unverified]` に相当する）。`replicas` は最新版を持つ報告者のうち tier が `author`・`chosen` の数、`unverified` は tier が `other` の数（[「レプリカ報告の信頼度」](../../architecture.md#レプリカ報告の信頼度replicastier)）。`reports`（＝ `reporters.length`）はサイトごとに（tier、`created_at` の新しい順）で先頭 200 件（`nostr::budget::MAX_REPORTS_PER_SITE`）までに切り詰めた後の件数、`dropped` は切り詰めで落ちた件数（[取得と表示の上限](../../architecture.md#取得と表示の上限nostrbudget)）。`reporters` の並びも（tier、`latest`、npub）順。`sites` も 1 作者あたり先頭 50 件まで。

## POST /api/publish/upload

`multipart/form-data`。ガードは他の書き込み系と同じ（`X-Swing-Dashboard: 1` ヘッダと Origin 検証、multipart なので `AppJson` は使わない）。

パート: `site`（必須）・`url`・`title`・`message`・`nip05`（省略可、`nip05` 省略時は `[publish].nip05`）。`site`/`url`/`title` は CLI と同じ規則で検証し違反は 400。`title` が空白のみなら未指定として扱う。`file`（1 個以上）: 各パートの `filename` がサイトルートからの相対パス（`/` 区切り。ブラウザは `webkitRelativePath` の先頭フォルダ名を取り除いて送る）。

サーバの検証（`upload::validate_relative_path`、違反はすべて 400 で何も書かない）:

- パスは非空、`/` で始まらない、`\` や制御文字を含まない
- 長さ `MAX_PATH_LEN`（4096 バイト）以下、セグメント数 `MAX_PATH_SEGMENTS`（32）以下、各セグメントは非空かつ `.`/`..` でない
- 同じパスの重複、`file` 0 個、`site` 無し、はいずれも 400
- `file` パートの総数は `MAX_UPLOAD_FILES`（10,000）まで

上限はすべて固定の定数（`src/dashboard/upload.rs`）で設定項目にはしていない。超過時はアップロード先の展開ディレクトリを丸ごと削除してから 400 を返す。

同時に実行できる publish は 1 本だけ（`AppState.publish_lock`）。実行中にもう 1 本来たら 409。

処理: (1) `<state_dir>/upload/` 配下に一時ディレクトリを作り、各 `file` パートをストリーミングで書き込む。(2) `api::run_publish`（NIP-05 検証 → Kubo に add して MFS に置く → サイトイベントを署名して送信 → 古い版を `[publish].keep_versions` 個まで残して削除、処理順は CLI の `swing publish`（[`architecture/cli.md#publish`](../cli.md#publish)）と同じ）を、展開先ディレクトリをサイトのディレクトリとして呼ぶ。削除に失敗した版は `prune_error` に理由が入るだけでレスポンス全体は成功扱い。(3) 成功でも失敗でも展開先ディレクトリを削除する（`<state_dir>/upload/` 自体は agent 起動時に丸ごと掃除される）。(4) ボディが `[dashboard].max_upload` を超えたら 413（ストリーミング中に超えた場合も打ち切る）。

```json
{ "site": "example.com", "url": "…", "title": "…", "message": "note", "nip05": { "status": "verified", "detail": null },
  "cid": "bafy…", "size": 12345, "created_at": 1790000000, "mfs_path": "/swing/publish/<hex>/example.com/1790000000",
  "relays": [ { "relay": "wss://…", "ok": true, "error": null } ], "pruned": ["1780000000"], "prune_error": null,
  "gateway_url": "…", "files": 3 }
```

- `nip05.status` は `off`/`verified`/`mismatch`/`not_applicable`/`error`。`require` で検証が通らなければ、add する前に 422 を返す: `{ "error": "...", "nip05": { "status": "...", "detail": "..." } }`。
- どの relay にも受理されなければ 502（Kubo に add した内容と古い版はそのまま残す）。
- `files` は受け取ったファイル数。他のフィールドは publish の結果そのもの。

## GET /api/publish/sites

自分（agent の鍵）が過去に公開したサイトの一覧。relay から自分の pubkey のサイトイベントを取得し、`d` ごとの最新版を `d` の順に返す。

```json
{ "sites": [ { "d": "example.com", "url": "https://example.com/", "cid": "bafy…", "size": 123, "created_at": 1790000000, "title": null, "message": null, "gateway_url": "http://localhost:8080/ipfs/bafy…/" } ] }
```

`gateway_url` は gateway 設定があれば付ける（`stored` 判定はしない）。relay の取得に失敗したら 502。state.json は見ないので、`/api/sites` の `stored_size` に相当するフィールドは無く、`size` は常に自己申告の値。`sites` も `d` の昇順で先頭 50 件（`nostr::budget::MAX_SITES_PER_AUTHOR_LISTED`）まで。

## GET /api/config

```json
{ "config_path": "/path/to/swing.toml", "config_exists": true, "writable": true, "restart_required": false, "sections": [
  { "name": "nostr", "items": [
    { "key": "secret_key", "env": "SWING_NOSTR_SECRET_KEY", "value": "(set, hidden)", "source": "file", "editable": false, "kind": "secret", "description": { "en": "Signing secret key (nsec or hex)...", "ja": "署名用の秘密鍵（nsec または hex）..." } },
    { "key": "relays", "env": "SWING_NOSTR_RELAYS", "value": ["wss://relay.damus.io"], "source": "default", "editable": true, "kind": "list", "raw": ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://yabu.me", "wss://relay-jp.nostr.wirednet.jp"], "description": { "en": "Nostr relays to connect to...", "ja": "接続する Nostr relay（カンマ区切り）" } },
    { "key": "max_total_storage", "env": "SWING_MAX_TOTAL_STORAGE", "value": 107374182400, "display": "100 GB", "source": "env", "editable": false, "kind": "size", "raw": "100 GB", "description": { "en": "Total storage cap...", "ja": "保存する全サイト合計の容量上限" } } ] } ] }
```

- `sections` は `nostr`/`ipfs`/`policy`/`agent`/`publish`/`dashboard`/`kubo`/`gateway` の順で、`settings::SETTINGS`（[`architecture.md`](../../architecture.md#設定と環境変数)）の宣言順そのままを列挙する（そちらが正本）。カタログの設定はすべて TOML フィールドを持つので、`key` が無い項目は無い（`SWING_FETCH_TIMEOUT`/`SWING_FETCH_IDLE_TIMEOUT` も `agent.fetch_timeout`/`agent.fetch_idle_timeout` という通常のキーとして入る）。
- `secret_key` の値は常に `"(set, hidden)"` か `"(not set)"`（[`dashboard.md`](../dashboard.md#秘密鍵を出さない仕組み) を参照）。`secret_key` はカタログ上 `editable: false` なので `editable` は常に `false`（値そのものはダッシュボードからは変更できず、セットアップ時にしか書けない。下記 `POST /api/setup`）。
- `ipfs.api` は `[kubo].managed = true` のとき固定文字列 `"managed"` になる（動的なポートを含む実際の URL ではなく、`swing up` が `<repo>/api` から解決した値であることを示す。[`up.md`](../up.md#動的な-api-ポートとrepoapi)）。`managed = false` なら実際の URL（`[ipfs].api` の値）。
- `kubo.binary`/`kubo.repo` はパスを文字列で返す（`binary` が未設定なら空文字）。`kubo.swarm_port` は未設定なら文字列 `"-"`（他のセクションと違い、数値でなく文字列で返る）。
- `value` は文字列・真偽・数値・文字列配列のいずれか（常に生の値）。容量・時間の項目は読みやすい文字列を `display` に添える: 容量は 1024 基数の最大単位に割り切れれば整数（`"100 GB"`）、割り切れなければ小数第 1 位まで、KB 未満はバイト表記。時間は日/時/分のどれかで割り切れれば大きい単位優先（`"5m"`）、割り切れなければ秒。個数系（`keep_versions` など）には `display` が付かず、無い項目はフィールドごと出ない。
- `config_path` は常に何か文字列が入る（環境変数だけで動いている、かつ設定ファイルが無くても `null` にはならない。下記の「設定ファイルのパス解決」）。`config_exists` はそのパスに実際にファイルがあるかどうか。
- `writable`: `config_exists` なら（`std::fs::OpenOptions::append(true)` で）そのファイルを開けるかどうか、`config_exists` が `false` なら親ディレクトリの `Permissions::readonly()` を見て判定する（`src/dashboard/dto.rs::is_config_writable`。副作用は無い）。`false` なら Settings／Setup 画面は編集フォームを出さず、読み取り専用表示にする（[`web.md`](web.md)）。
- `restart_required`: この `swing up` プロセスが起動してから一度でも `PUT /api/config` か `POST /api/setup` が成功していれば `true`（`AppState.restart_required`、`AtomicBool`。プロセスが実際に再起動する—`Exit::Restart` を経て `up::run` が呼び直される—までリセットされない）。
- `dashboard` セクションに `ui`（真偽値、`SWING_DASHBOARD_UI`）が入る。`listen` は常に `SocketAddr` の文字列。
- 各 `items[]` は追加で次のフィールドを持つ:
  - `source`: `"env"` / `"file"` / `"default"`（`config::Config::sources`、キーは `"<section>.<フィールド名>"`。`ConfigDto` はこれをそのまま `Source::Env`→`"env"` のように文字列化する）。
  - `editable`: カタログ上そのキーが `editable: true` で、かつ `source` が `"env"` ではないときだけ `true`。
  - `kind`: カタログの全キーに付く（`source` や `editable` に関わらず）。`"size"` / `"duration"` / `"bool"` / `"integer"` / `"string"` / `"list"` / `"nip05"` / `"path"` / `"socket_addr"` / `"port"` / `"url"` / `"secret"` / `"listen"` のいずれか。編集フォームが分岐するのは前者 7 種のみ（下記「設定の読み込みと編集」に同じ）。
  - `raw`: 編集可能な 20 キーだけに付く、現在の値を `PUT /api/config`／`POST /api/setup` の `items` にそのまま送り返せる形にしたもの（`size`/`duration` は `parse_size`/`parse_duration_secs` が受け付ける文字列、`list` は文字列配列、それ以外は文字列）。
  - `options`: `kind: "nip05"` のときだけ付く。取りうる値の一覧 `["off", "warn", "require"]`（`config::NIP05_MODE_NAMES`）。
  - `description`: `{ "en": ..., "ja": ... }`。カタログの `Setting.description`（`settings::SETTINGS`）をそのまま返す、1 文の英語・日本語の説明。環境変数名は含まない（`env` フィールドと別出し）。

## PUT /api/config

設定ファイルの値を書き換える。ガードは他の書き込み系と同じ（`X-Swing-Dashboard: 1` ヘッダと Origin 検証）。

```json
{ "items": { "policy.max_total_storage": "20GB", "nostr.relays": ["wss://relay.damus.io", "wss://nos.lol"] } }
```

- キーは `"<section>.<フィールド名>"`（`GET /api/config` の `raw` が付くキーと同じ）で、カタログ上 `editable: true` ではないキー、または現在 `source: "env"` のキーが 1 つでも含まれていれば、ファイルには一切触れずに 400 で拒否する（`settings::check_not_env_sourced`。部分適用はしない）。値の形式は `raw` と同じ（`nostr.relays` は空配列だと 400）。
- 適用順（`settings::update`）: 既存のファイルを `toml_edit::DocumentMut` として読む（無ければ空文書）→ 渡された `items` だけをその場で書き換える（`toml_edit` なのでコメントや他のキーはそのまま残る）→ `config::build_config_from_str` で妥当性を確認する（ここで失敗したらファイルには書かない。他の設定項目との整合や `parse_size`/`parse_duration_secs` などのバリデーションを全部通す）→ tmp ファイルに書いて `rename`（atomic）。ファイルが元から存在していればその権限を引き継ぎ、新規作成なら unix で `0600`。
- 成功したら `AppState.restart_required` を `true` にし、`AppState.display_config`（[`dashboard.md`](../dashboard.md#概要)）を書き換え後の設定に差し替えてから、`GET /api/config` と同じ形の `ConfigDto`（`restart_required: true`）を返す。実際に動いている relay・Kubo・agent はまだ古い設定のままで、値が反映されるのは次の再起動から（`display_config` はあくまで「再起動したらこうなる」を見せるための、表示専用のコピー）。
- 失敗（400）した場合はファイルもプロセスの状態も変わらない。

## POST /api/setup

セットアップモード（[`../up.md#セットアップモード鍵未設定`](../up.md#セットアップモード鍵未設定)）の間だけ使える。それ以外のとき（`AppState::setup_mode() == false`）は 409 `{"error": "swing is already configured; setup is no longer available"}`。

```json
{ "secret_key": null, "items": { "nostr.relays": ["wss://relay.damus.io"], "policy.max_total_storage": "100GB", "policy.max_per_site": "10GB", "policy.max_per_account": "20GB" } }
```

- `secret_key`: `null`（または省略・空文字）なら `nostr_sdk::Keys::generate()` で新しい鍵を作る。nsec か hex の文字列を渡せば `Keys::parse` でその鍵を使う。
- `items` は `PUT /api/config` と同じホワイトリスト・同じ env 由来チェックを通す（`settings::setup` も内部で `check_not_env_sourced` を呼ぶ）。セットアップ画面（`web/setup.js`）は relays と 3 つの保存上限だけをフォームに出す。
- 成功したら `[nostr].secret_key` に鍵の hex を書き、`items` と合わせて 1 回の書き込みで保存する（`settings::setup`。バリデーション・atomic write は `PUT /api/config` と同じ。ファイルが無ければ `config_path`—常に決まっている、下記—に新規作成する）。

```json
{ "ok": true, "npub": "npub1…", "restart": true }
```

- 秘密鍵の値そのものは応答に含めない（`npub` だけ）。
- レスポンスを返した後、約 300ms 待ってから `shutdown::ExitRequest::restart()` を呼ぶ（`tokio::spawn` した別タスクで。レスポンスの送出をブロックしない）。これは `POST /api/restart`（下記）と同じ経路で、プロセスを終了させずに `swing up` をプロセス内で再起動する（[`up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）。`web/setup.js` は成功表示を出した後、`GET /api/overview` を 1 秒間隔でポーリングして `setup: false` に変わるのを待つ。

### 設定ファイルのパス解決

`config::resolve_config_path`（`--config` → `SWING_CONFIG`（空文字は未設定扱い）→ `<カレントディレクトリ>/swing.toml`）は、ファイルが無くても常にパスを返す。そのため `swing.toml` が無い状態で `swing up` を起動しても `Config.config_path` は必ず何か具体的なパスを指し、セットアップ画面はそこに新規作成する（`config_exists: false` のときの Settings 画面は `configWillBeCreated` の注記を出す）。

## POST /api/shutdown, POST /api/restart

エージェントプロセス（`swing up`）を止める／再起動する。ガードは他の書き込み系と同じ（`X-Swing-Dashboard: 1` ヘッダと Origin 検証）。ボディは不要（送っても無視する）。

`202 Accepted` を即座に返してから（レスポンスの送出をブロックせずに）`shutdown::ExitRequest`（[`../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）の `stop()`／`restart()` を呼ぶ。実際の終了は agent のループが次に cancel を検知したタイミング（通常は即座）。

```json
{ "ok": true, "action": "stop" }
```
```json
{ "ok": true, "action": "restart" }
```

`restart` はプロセスを終了させない。`ExitRequest.restart()` → 最上位トークンの cancel → `run_managed`／`run_unmanaged`（と `agent::run_until`）がグレースフルに終わる → `up::run` が `Exit::Restart` を返す → `main.rs` のループが設定を読み直して同じプロセス・同じ PID のまま `up::run` を呼び直す。以前あった「exit code 3 で終了し、サービスマネージャの再起動ポリシー任せにする」経路は無くなった（[`up.md`](../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）。`AppState.exit` は常に存在する（`Option` ではない）。
