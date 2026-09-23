# ダッシュボード HTTP API（`src/dashboard/api.rs`, `src/dashboard/dto.rs`）

[`dashboard.md`](../dashboard.md) の一部。ガード・タイムアウトは [`dashboard.md`](../dashboard.md)、画面側からの使い方は [`web.md`](web.md) を参照。

## 共通

- すべて JSON。公開鍵は `pubkey`（小文字 hex）と `npub` を併記する。時刻は epoch 秒の整数。無い値は `null`。
- エラーは `{ "error": "<メッセージ>" }` とステータスコード。入力不正は 400、relay 未接続は 500、relay や Kubo・Nostr 発行の失敗は 502、publish の多重実行は 409。JSON の構文エラー・必須フィールド欠落・`Content-Type` 不一致はすべて 400（422 は publish の NIP-05 `require` 失敗専用。ボディが大きすぎる場合だけ 413）。`POST /api/publish/upload` だけ `multipart/form-data` を受ける。
- `keys`（mirror add/remove）・`root`（webring）・`key`（replicas）は 1 リクエストあたり最大 100 件、超えると 400。
- relay を引く API（sites・mirror・webring・replicas）はサーバ側でキャッシュせず、同時実行数の制限やレート制限も無い。

## 既知の性質

- `POST /api/publish/upload` の 409 はダッシュボード内で同時に来た publish リクエストどうしだけを排他する。同じホスト上の CLI `swing publish` とは排他されない。
- `run_publish` の NIP-05 検証はプライベート/ループバック/リンクローカル等に解決されるホストへの接続を拒否する。接続エラーの詳細は `nip05.detail` に出さず、`unreachable`/`timeout`/`invalid_response` の粗い分類だけを返す（生のメッセージは `tracing::warn` にのみ出す）。

## GET /api/overview

```json
{ "version": "0.1.0", "pubkey": "ab12…", "npub": "npub1…", "relays": ["wss://relay.damus.io"], "mirror_set": "swing", "gateway": "http://localhost:8080", "started_at": 1790000000, "max_upload": 2147483648 }
```

`gateway` は `[dashboard].gateway` が空なら `null`。`started_at` はダッシュボードが有効になった起動時刻。`max_upload` は `[dashboard].max_upload` のバイト数。

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
  "garbage": [ { "path": "/swing/…", "list_failed": false } ], "problems": 0 }
```

`sites` はサイトごとの実容量。`actual` はそのサイトの全版をまとめた `dag/stat` の `TotalSize` で、版どうしで共有しているブロックは 1 回だけ数える。測れなかったサイトは `null`。`actual_bytes` は `actual` の合計で、`null` のサイトが 1 つでもあれば `null`。

`health` は `ok`/`missing`/`cid_mismatch`/`incomplete`/`check_failed`/`invalid_key`（CLI の判定を snake_case で返す）。`ok` 以外は `detail` に理由が入る。`invalid_key` は `state.json` のキーが `<pubkey hex>:<d>` の形式として不正だった場合で、`pubkey`・`npub`・`d`・`path`・`size`・`created_at` は `null`、`cid` だけ分かれば入り、`detail` に元のキー文字列が入る。問題があっても HTTP は常に 200（`problems` の件数で分かる）。

## GET /api/mirror

```json
{ "title": "SWING mirror list", "note": null, "members": [ { "pubkey": "…", "npub": "…" } ] }
```

Follow Set が無ければ `title: null`、`members: []`。

## POST /api/mirror/add, POST /api/mirror/remove

リクエスト: `{ "keys": ["npub1…", "hex…", "nprofile1…"] }`。空、100 件超、パース不能のいずれかで 400。

```json
{ "changed": [ { "pubkey": "…", "npub": "…" } ], "unchanged": [ { "pubkey": "…", "npub": "…" } ], "published": true, "relays": [ { "relay": "wss://…", "ok": true, "error": null } ], "members": [ { "pubkey": "…", "npub": "…" } ] }
```

- `changed`: 実際に追加・削除したもの。`unchanged`: 既に追加済み／もともと未登録で no-op だったもの。
- 変更が無ければ `published: false`、`relays: []`。`published: true` なのにどの relay にも受理されなければ 502。
- 成功（1 relay 以上が accept）したら agent に即時 refresh を促す。

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
{ "config_path": "/path/to/swing.toml", "sections": [
  { "name": "nostr", "items": [
    { "key": "secret_key", "env": "SWING_NOSTR_SECRET_KEY", "value": "(set, hidden)" },
    { "key": "max_total_storage", "env": "SWING_MAX_TOTAL_STORAGE", "value": 107374182400, "display": "100 GB" } ] } ] }
```

- `sections` は `nostr`/`ipfs`/`policy`/`agent`/`publish`/`dashboard`/`kubo`/`gateway` の順で、[`architecture.md`](../../architecture.md#設定と環境変数) の設定表と同じキーを同じ順で列挙する（そちらが正本）。TOML キーの無い `SWING_FETCH_TIMEOUT`/`SWING_FETCH_IDLE_TIMEOUT` は `agent` セクションに `key: null` で入る。
- `secret_key` は常に `"(set, hidden)"`（[`dashboard.md`](../dashboard.md#秘密鍵を出さない仕組み) を参照）。
- `ipfs.api` は `[kubo].managed = true` のとき固定文字列 `"managed"` になる（動的なポートを含む実際の URL ではなく、`swing up` が `<repo>/api` から解決した値であることを示す。[`up.md`](../up.md#動的な-api-ポートとrepoapi)）。`managed = false` なら実際の URL（`[ipfs].api` の値）。
- `kubo.binary`/`kubo.repo` はパスを文字列で返す（`binary` が未設定なら空文字）。`kubo.swarm_port` は未設定なら文字列 `"-"`（他のセクションと違い、数値でなく文字列で返る）。
- `value` は文字列・真偽・数値・文字列配列のいずれか（常に生の値）。容量・時間の項目は読みやすい文字列を `display` に添える: 容量は 1024 基数の最大単位に割り切れれば整数（`"100 GB"`）、割り切れなければ小数第 1 位まで、KB 未満はバイト表記。時間は日/時/分のどれかで割り切れれば大きい単位優先（`"5m"`）、割り切れなければ秒。個数系（`keep_versions` など）には `display` が付かず、無い項目はフィールドごと出ない。
- `config_path` は実際に読んだ設定ファイルのパス。環境変数だけで動いているなら `null`。

## POST /api/shutdown, POST /api/restart

エージェントプロセス（`swing up`）を止める／再起動する。ガードは他の書き込み系と同じ（`X-Swing-Dashboard: 1` ヘッダと Origin 検証）。ボディは不要（送っても無視する）。

`202 Accepted` を即座に返してから（レスポンスの送出をブロックせずに）`shutdown::ExitRequest`（[`../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）の `stop()`／`restart()` を呼ぶ。実際の終了は agent のループが次に cancel を検知したタイミング（通常は即座）。

```json
{ "ok": true, "action": "stop" }
```
```json
{ "ok": true, "action": "restart" }
```

`swing up` に対して `restart` を要求しても、プロセス自身は exit code 3 で終了するだけで、それを見て再起動するかどうかはサービスマネージャ側の設定次第（[`up.md`](../up.md#各サービスマネージャの反応)）。ダッシュボードが無効（`AppState.exit` が `None`。通常は起きない。ダッシュボードが動いていればこのルートに来る時点で必ず `Some`）ならエラー扱い（500）。
