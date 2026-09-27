# ダッシュボード HTTP API（`src/dashboard/api.rs`, `src/dashboard/dto.rs`, `src/dashboard/config_dto.rs`）

[`../dashboard.md`](../dashboard.md) の一部。ガード・タイムアウトは [`../dashboard.md`](../dashboard.md)、画面側からの使い方は [`web.md`](web.md) を参照。

## 共通

### 形式とエラー

- すべて JSON。公開鍵は `pubkey`（小文字 hex）と `npub` を併記する。時刻は epoch 秒の整数。無い値は `null`。`POST /api/publish/upload` だけ `multipart/form-data` を受ける。
- ハンドラが返すエラーは `{ "error": "<メッセージ>" }` とステータスコード。

| ステータス | 条件 |
|---|---|
| 400 | 入力不正。JSON の構文エラー・必須フィールド欠落・`Content-Type` 不一致も 400（axum の既定の 422 にしない）。`POST /api/publish/upload` に multipart でない `Content-Type` を送ったときは axum の素の 400（本文は JSON ではない） |
| 401 | 認証が通らない（下記） |
| 403 | ガードの Host・`X-Swing-Dashboard`・Origin の検証に通らない（[`../dashboard.md#ガードsrcdashboardguardrs`](../dashboard.md#ガードsrcdashboardguardrs)） |
| 404 | 存在しないルート（空ボディ。JSON ではない） |
| 408 | リクエストタイムアウト（空ボディ。[`../dashboard.md#タイムアウトsrcdashboardmodrs`](../dashboard.md#タイムアウトsrcdashboardmodrs)） |
| 409 | publish の多重実行、セットアップ・ペアリング・つなぎ直しを使えない状態（セットアップが済んで再起動を待っている間の 2 回目の `POST /api/setup` を含む）、`mirror/add` で Follow Set が上限を超える（各エンドポイント） |
| 413 | ボディが大きすぎる |
| 422 | publish の NIP-05 `require` 失敗だけ |
| 500 | ファイルの読み書きなど内部の失敗 |
| 502 | relay・Kubo・Nostr 発行・署名アプリの失敗 |
| 503 | agent の未準備・セットアップモード（下記） |

### 認証とガード

- `POST /api/login` 以外の `/api/*` は認証が要る。`Authorization: Bearer <token>` かセッション cookie が無い・合わなければ 401 `{"error": "missing or invalid dashboard token or session"}`（仕組みは [`../dashboard.md#認証srcauthrs-srcdashboardsessionrs`](../dashboard.md#認証srcauthrs-srcdashboardsessionrs)）。
- GET 以外のエンドポイント（`POST /api/login` と `POST /api/publish/upload` を含む）は、`X-Swing-Dashboard: 1` ヘッダと Origin の検証を通す（[`../dashboard.md#ガードsrcdashboardguardrs`](../dashboard.md#ガードsrcdashboardguardrs)）。

### agent の準備状態とセットアップモード

API は `swing up` の寿命で動き続ける（[`../up.md`](../up.md)）。

| エンドポイント | 使えないとき |
|---|---|
| relay・Kubo を使うもの（`/api/sites`・`/api/status`・`/api/mirror`・`/api/mirror/add`・`/api/mirror/remove`・`/api/webring`・`/api/replicas`・`/api/publish/sites`・`/api/publish/upload`） | agent が起動時の突き合わせ（保存量に比例して時間がかかる）を終えて `AppState::set_ready` を呼ぶまでと、agent が落ちて `set_not_ready` を呼んでから次に `set_ready` するまで（[`../agent.md#全体の流れ`](../agent.md#全体の流れ)）は 503 `{"error": "agent is not ready"}`。セットアップモード（[`../up.md#セットアップモード鍵未設定`](../up.md#セットアップモード鍵未設定)）の間は常に 503 `{"error": "agent is not configured"}` |
| `/api/overview`・`/api/activity`・`/api/stats`・`/api/config`・`/api/shutdown`・`/api/restart`・`/api/login`・`/api/login-code`・`/api/token/rotate` | 無い（常に応答する） |
| `/api/setup` | セットアップモードでなければ 409。セットアップが一度成功してから再起動するまでも 409 |
| `/api/setup/signer` | セットアップモードでも署名アプリを使っている間でもなければ 409 |
| `/api/signer/reconnect` | 署名アプリを使っていなければ 409 |

`/api/publish/upload` の判定の順は [下記](#post-apipublishupload)。署名を伴う API（`/api/mirror/add`・`/api/mirror/remove`・`/api/publish/upload`）は、NIP-46 の署名アプリを使っていると署名アプリの返事を待ち、署名できなければ 502 になる（待ち時間と署名アプリがオフラインのときの扱いは [`../signer.md`](../signer.md)）。

### 件数と負荷

- `keys`（mirror add/remove）・`root`（webring）・`key`（replicas）は 1 リクエストあたり最大 100 件、超えると 400。
- relay を引く API（sites・mirror・webring・replicas）はサーバ側でキャッシュせず、同時実行数の制限やレート制限も無い。
- API を叩く CLI サブコマンドの一覧は [`../cli.md#共通`](../cli.md#共通)（クライアント実装は `src/api_client.rs::ApiClient`）。

## 既知の性質

- `POST /api/publish/upload` の 409 はダッシュボード内で同時に来た publish リクエストどうしだけを排他する。同じホスト上の CLI `swing publish` とは排他されない。
- `run_publish` の NIP-05 検証の SSRF 対策と、`nip05.detail` がエラー時に粗い分類（`unreachable`/`timeout`/`invalid_response`）だけになる規則は [`../nip05.md`](../nip05.md) を参照。

## GET /api/overview

```json
{ "version": "0.1.0", "setup": false, "pubkey": "ab12…", "npub": "npub1…", "relays": ["wss://relay.damus.io"], "mirror_set": "swing", "gateway": "http://localhost:8080", "started_at": 1790000000, "instance": "cdeee5bc85519f44", "max_upload": 2147483648, "signer": { "remote": true, "relays": ["wss://relay.primal.net"], "last_failure": { "at": 1790000100, "message": "the signer app did not answer in time; check that it is running and approve the request" } } }
```

- `gateway`: `[dashboard].gateway` が空なら `null`。
- `started_at`: ダッシュボードが有効になった起動時刻。
- `instance`: `up::run` の回ごと（`AppState` を作るたび）に変わるランダムな 16 桁の 16 進文字列。同じプロセスの中での再起動（`POST /api/restart`）も見分けられる（`started_at` は秒単位なので 1 秒以内の再起動では変わらない）。`swing stop --restart` が再起動の完了を待つのに使う（[`../cli.md#stop`](../cli.md#stop)）。
- `max_upload`: `[dashboard].max_upload` のバイト数。
- `signer`: 署名の方法。`remote` は NIP-46 の署名アプリを使っているかどうか、`relays` は署名アプリとのやりとりに使っている relay（秘密鍵なら空）、`last_failure` は署名アプリへの最後のリクエストが失敗したときの時刻とメッセージ（成功していれば・秘密鍵なら `null`。[`../signer.md`](../signer.md)）。
- `setup`: 鍵も署名アプリも未設定（セットアップモード）かどうか。そのときは `pubkey`／`npub`／`signer` も `null` になる。詳細は [`../up.md#セットアップモード鍵未設定`](../up.md#セットアップモード鍵未設定)。画面側の扱いは [`web.md`](web.md)。

## GET /api/activity

```json
{ "latest_stored_at": 1790000000 }
```

- `latest_stored_at`: `state.json` に記録された全版の `stored_at` の最大値。版が 1 つも無い（`state.json` が無い場合を含む）と `null`。
- `state.json` を読むだけで relay にも Kubo にも接続しないので、定期的に呼んでも軽い。Desktop 画面の更新確認はこれを見て、値が進んだときだけ `/api/sites` を取り直す（[`desktop.md`](desktop.md)）。
- agent の準備状態に関わらず応答する。`state.json` の読み込み・解釈に失敗したら 500（`error!` でログに出す）。

## GET /api/stats

```json
{ "interval": 60, "kubo_managed": true, "samples": [ { "at": 1790000000, "swing": { "cpu_percent": 0.4, "rss_bytes": 17432576 }, "kubo": { "cpu_percent": 2.1, "rss_bytes": 251658240 }, "traffic": { "in_per_sec": 5120, "out_per_sec": 2048, "total_in": 734003200, "total_out": 104857600 } } ] }
```

- `swing up` が測って持っているリソース使用量（[`../stats.md`](../stats.md)）を古い順に返す。メモリ上の配列を写すだけで、呼んでも測り直さない。
- `since`（epoch 秒、省略時 0）より後の `at` のサンプルだけを返す。前回の最後の `at` を渡せば新しい分だけ取れる。数値でなければ 400。
- `interval`: 測る間隔の秒数。
- `kubo_managed`: `[kubo].managed`。`false`（外部の Kubo）なら `kubo` は常に `null`。
- 各サンプルの `swing`・`kubo`・`traffic` と、その中の `cpu_percent`・`in_per_sec`・`out_per_sec` は、取れなかったときや前回との差が出せないときに `null`（条件は [`../stats.md#測り方`](../stats.md#測り方)）。`cpu_percent` は 1 コアを 100% とする小数、ほかはバイト数（`*_per_sec` は毎秒）の整数。
- agent の準備状態に関わらず応答する（セットアップモードでも）。

## GET /api/sites

`mirror::collect_sites` をそのまま JSON にしたもの（CLI の `swing sites` と同じ集計。[`../cli.md#sites`](../cli.md#sites)）。

```json
{ "follow_set": { "found": true, "note": null },
  "accounts": [ { "pubkey": "…", "npub": "…", "sites": [ { "d": "example.com", "cid": "bafy…", "url": "…", "size": 12345, "stored_size": 12300, "stored_at": 1790000100, "created_at": 1790000000, "title": "…", "message": "…", "nip05": "verified", "replicas": 3, "unverified_replicas": 0, "stored": true, "gateway_url": "…" } ] } ],
  "replicas_error": null,
  "unfollowed": { "remove_on_unfollow": true, "accounts": [ { "...": "同じ形。ただし url・title・message・replicas・unverified_replicas は常に null、stored は常に true、stored_size は size と同じ値" } ] } }
```

- `follow_set.note`: CLI が括弧付きで出す注記から括弧を外した文字列。無ければ `null`。
- `nip05`・`title`・`message`・`size` は値が無ければ `null`。`title` は作者の自己申告で受信側は信頼しない（[`protocol.md` 第 4 節](../../protocol.md#4-サイトイベント)）。`message` は生の `content`（サニタイズ・切り詰めはフロントの責務）。`title` も同様にサニタイズはフロントの責務。
- `size` はイベントの自己申告の `size` タグ。`stored_size` は `cid` と一致する `state.json` の `VersionRecord.size`（保存時に `dag/stat` で測った値。この呼び出しのために改めて Kubo は呼ばない）で、一致する版が無ければ `null`。画面での出し方は [`web.md`](web.md#sites-画面)。版ごとの重複排除込みの実測合計は `/api/status` の `sites[].actual` にしかない。
- `stored_at`: `stored_size` と同じ版の `VersionRecord.stored_at`（その版を保存した時刻）。一致する版が無ければ `null`。
- `replicas`・`unverified_replicas`: レプリカ報告の取得に失敗すると全サイトで両方 `null` になり、`replicas_error` に理由が入る。`replicas` は報告者が作者自身か、作者かこちらの Follow Set に入っている報告者（信頼できる tier）の数、`unverified_replicas` はそれ以外（自称にすぎない tier）の数（[「レプリカ報告の信頼度」](../nostr.md#レプリカ報告の信頼度replicastier)）。
- `gateway_url`: `stored` が true かつ gateway 設定がある版だけに付く。
- `accounts[].sites` は 1 アカウントあたり `d` の昇順で先頭 `MAX_SITES_PER_AUTHOR_LISTED` 件まで、Follow Set の対象は先頭 `MAX_FOLLOW_SET_ENTRIES` 件まで（[取得と表示の上限](../nostr.md#取得と表示の上限nostrbudget)）。

## GET /api/status

`health::collect_status` の結果（CLI の `swing status` と同じ集計。[`../cli.md#status`](../cli.md#status)）。relay には接続しない。[起動時の突き合わせ](../agent.md#起動時の突き合わせ)と同じ検査をするので重い。

```json
{ "versions": [
    { "pubkey": "…", "npub": "…", "d": "example.com", "path": "/swing/…", "cid": "bafy…", "size": 123, "created_at": 1, "health": "ok", "detail": null },
    { "pubkey": null, "npub": null, "d": null, "path": null, "cid": "bafy…", "size": null, "created_at": null, "health": "invalid_key", "detail": "<state.json の生のキー>" } ],
  "sites": [ { "pubkey": "…", "npub": "…", "d": "example.com", "path": "/swing/…", "actual": 123 } ],
  "actual_bytes": 123,
  "garbage": [ { "path": "/swing/…", "list_failed": false, "list_failed_reason": null } ], "problems": 0 }
```

`sites` はサイトごとの実容量。`actual` はそのサイトの全版をまとめた `dag/stat` の `TotalSize` で、版どうしで共有しているブロックは 1 回だけ数える。測れなかったサイトは `null`。`actual_bytes` は `actual` の合計で、`null` のサイトが 1 つでもあれば `null`。

`health` は `ok`/`missing`/`cid_mismatch`/`incomplete`/`check_failed`/`invalid_key`（CLI の判定を snake_case で返す）。`ok` 以外は `detail` に理由が入る。`invalid_key` は `state.json` のキーが `<pubkey hex>:<d>` の形式として不正だった場合で、`pubkey`・`npub`・`d`・`path`・`size`・`created_at` は `null`、`cid` にはその版の CID が入り、`detail` に元のキー文字列が入る。問題があっても HTTP は常に 200（`problems` の件数で分かる）。

`garbage[].list_failed_reason`: 一覧に失敗したディレクトリ（`list_failed: true`）だけ理由の文字列が入る。`list_failed: false` なら常に `null`。CLI（`swing status`）は `[list failed]: <理由>` として表示する。

## GET /api/mirror

```json
{ "title": "SWING mirror list", "note": null, "members": [ { "pubkey": "…", "npub": "…" } ] }
```

Follow Set が無ければ `title: null`、`members: []`。

## POST /api/mirror/add, POST /api/mirror/remove

リクエスト: `{ "keys": ["npub1…", "hex…", "nprofile1…"] }`。空、100 件超、パース不能のいずれかで 400。`add` の結果の `p` タグ数が `MAX_FOLLOW_SET_ENTRIES`（[取得と表示の上限](../nostr.md#取得と表示の上限nostrbudget)）を超えるときは publish せず 409（`{"error": "would grow the follow set to <N> entries, over the 500-entry limit; remove some first"}`）。`mirror::apply_add` はこのとき `mirror::FollowSetCapExceeded` を返し、`api::mirror_add_error` がそれを downcast して 409 に、それ以外のエラーは `api::upstream` で 502 にする。

```json
{ "changed": [ { "pubkey": "…", "npub": "…" } ], "unchanged": [ { "pubkey": "…", "npub": "…" } ], "published": true, "relays": [ { "relay": "wss://…", "ok": true, "error": null } ], "members": [ { "pubkey": "…", "npub": "…" } ], "note": null, "follow_set_found": true }
```

- `changed`: 実際に追加・削除したもの。`unchanged`: 既に追加済み／もともと未登録で no-op だったもの。
- 変更が無ければ `published: false`、`relays: []`。`published: true` なのにどの relay にも受理されなければ 502。
- 成功（1 relay 以上が accept）したら agent に即時 refresh を促す。
- `note`: relay から取れた Follow Set より `state.json` に保存済みの版を使った場合の注記（`(relays returned an older follow set; ...)` / `(follow set not found on relays; ...)`）。括弧付きの文字列そのまま、無ければ `null`。`follow_set_found`: 操作前に Follow Set が見つかっていたか。CLI の `swing mirror remove` は Follow Set が無ければ（`follow_set_found: false`）`(no follow set found); no changes` とだけ表示して他のフィールドを見ない（[`../cli.md#mirror-list--add--remove`](../cli.md#mirror-list--add--remove)）。`add` はこの分岐を使わない。

## GET /api/webring?root=\<key\>&depth=\<N\>

`root` は繰り返し指定可、省略時は自分の pubkey 1 つ、100 件超で 400。`depth` は省略時 2、4 超か非数値で 400。たどり方・グラフの組み立ては CLI の `swing webring` と同じ（[`../cli.md#webring`](../cli.md#webring)）。

```json
{ "depth": 2,
  "nodes": [ { "pubkey": "…", "npub": "…", "short_npub": "npub1abc…uvwxyz", "names": ["example.com"], "label": "example.com", "depth": 0, "root": true, "has_follow_set": true } ],
  "edges": [ { "from": "<hex>", "to": "<hex>", "mutual": true } ], "beyond": 0, "over_budget": 0,
  "referencing": { "accounts": [ { "pubkey": "…", "npub": "…" } ], "more": 0 },
  "text": "…swing webring と同じ text 出力…", "dot": "…同じ dot 出力…", "mermaid": "…同じ mermaid 出力…" }
```

- `root: true` は `depth == 0` のノード。ノードの並びは（深さ、ラベル）順。`names` は 1 アカウントあたり `MAX_SITES_PER_AUTHOR_LISTED` 件まで。
- 双方向の組は `mutual: true` の辺 1 本、片方向は `mutual: false` の辺（`webring::split_links` で分ける）。
- `beyond` と `over_budget` は `swing webring` の同名のカウンタ（[`../cli.md#webring`](../cli.md#webring)、[取得と表示の上限](../nostr.md#取得と表示の上限nostrbudget)）。
- `referencing`: `#p` で見つかった、起点を名指ししているだけでクロールには加えていないアカウント（[「レプリカ報告の信頼度」](../nostr.md#レプリカ報告の信頼度replicastier)）。`accounts` は先頭 `MAX_REFERENCING_LISTED` 件まで、`more` は切り詰めで落ちた件数。`nodes`・`edges`・`dot`・`mermaid` には含まれない（グラフはフォロー先の辺だけで描く）。

## GET /api/replicas?key=\<key\>

`key` は繰り返し指定可、省略時は自分 1 つ、100 件超で 400。集計は CLI の `swing replicas` と同じ（[`../cli.md#replicas`](../cli.md#replicas)）。

```json
{ "authors": [ { "pubkey": "…", "npub": "…", "sites": [
  { "d": "example.com", "cid": "bafy…", "replicas": 2, "unverified": 1, "reports": 3, "dropped": 0, "reporters": [ { "pubkey": "…", "npub": "…", "latest": true, "tier": "chosen" } ] }
] } ] }
```

`tier` は `"author"` / `"chosen"` / `"other"` のいずれか（`"other"` が CLI の `[unverified]` に相当する）。`replicas` は最新版を持つ報告者のうち tier が `author`・`chosen` の数、`unverified` は tier が `other` の数（[「レプリカ報告の信頼度」](../nostr.md#レプリカ報告の信頼度replicastier)）。`reports`（＝ `reporters.length`）は `MAX_REPORTS_PER_SITE` で切り詰めた後の件数、`dropped` は切り詰めで落ちた件数。`reporters` の並びは tier（`author`→`chosen`→`other`）、同じ tier では `latest: true` が先、最後に hex の順。切り詰めと並びの規則は [取得と表示の上限](../nostr.md#取得と表示の上限nostrbudget) と [「レプリカ報告の信頼度」](../nostr.md#レプリカ報告の信頼度replicastier)。`sites` も 1 作者あたり `MAX_SITES_PER_AUTHOR_LISTED` 件まで。

## POST /api/publish/upload

`multipart/form-data`。

パート: `site`（必須）・`url`・`title`・`message`・`nip05`（省略可、`nip05` 省略時は `[publish].nip05`）。`site`/`url`/`title` は CLI と同じ規則で検証し違反は 400。`title` が空白のみなら未指定として扱う。`file`（1 個以上）: 各パートの `filename` がサイトルートからの相対パス（`/` 区切り。ブラウザは `webkitRelativePath` の先頭フォルダ名を取り除いて送る）。

サーバの検証（`upload::validate_relative_path` など。パートを受け取りながら順に検証し、違反は 400）:

- パスは非空、`/` で始まらない、`\` や制御文字を含まない
- 長さ `MAX_PATH_LEN`（4096 バイト）以下、セグメント数 `MAX_PATH_SEGMENTS`（32）以下、各セグメントは非空かつ `.`/`..` でない
- 各セグメントは `.` や半角スペースで終わらない、Windows の予約デバイス名（`CON`・`PRN`・`AUX`・`NUL`・`COM1`〜`9`・`LPT1`〜`9`、大小文字無視、拡張子付き `nul.txt` も含む）でない（プラットフォームを問わず拒否。他 OS での展開時の破損防止）
- 同じパスの重複、`file` 0 個、`site` 無し、はいずれも 400
- `file` パートの総数は `MAX_UPLOAD_FILES`（10,000）まで

上限はすべて固定の定数（`src/dashboard/upload.rs`）。違反や上限超過が見つかるまでに受け取ったファイルは展開先に書かれるが、400 を返す前に展開先ディレクトリを丸ごと削除する（下記 (3)）。

判定の順は、パートの受信とパスの検証（400・413）→ `site`/`url`/`title`/`nip05` の検証（400）→ 多重実行（409）→ セットアップモード（503 `agent is not configured`）→ NIP-05（422）→ agent の準備（503 `agent is not ready`）。同時に実行できる publish は 1 本だけ（`AppState.publish_lock`）で、本体を最後まで受け取ってから判定するので、実行中にもう 1 本来ても 409 はアップロードの後になる。

処理: (1) `<state_dir>/upload/` 配下に一時ディレクトリを作り、各 `file` パートをストリーミングで書き込む。展開先ディレクトリとその中の各ディレクトリは unix では `0o700`、書き込むファイルは `0o600` で作成する（umask 任せにしない。Windows では no-op）。(2) `api::run_publish`（NIP-05 検証 → Kubo に add して MFS に置く → サイトイベントを署名して送信 → 古い版を `[publish].keep_versions` 個まで残して削除、処理順は CLI の `swing publish`（[`../cli.md#publish`](../cli.md#publish)）と同じ）を、展開先ディレクトリをサイトのディレクトリとして呼ぶ。削除に失敗した版は `prune_error` に理由が入るだけでレスポンス全体は成功扱い。(3) 成功でも失敗でも、ハンドラの途中でのリクエスト打ち切り（[`../dashboard.md` のタイムアウト](../dashboard.md#タイムアウトsrcdashboardmodrs)の 30 分超過、またはクライアントの切断）を含めて、展開先ディレクトリは `upload::UploadDirGuard` の `Drop` により必ず削除される。取りこぼした分は `<state_dir>/upload/` ごと `up::run` の起動時（プロセス内再起動を含む。[`../up.md`](../up.md)）に掃除される。(4) ボディが `[dashboard].max_upload` を超えたら 413（ストリーミング中に超えた場合も打ち切る）。multipart の受信エラーは `upload::multipart_error_to_api` が axum の `MultipartError::status()` で振り分け、`413 Payload Too Large` なら 413、それ以外は 400 にする。`status()` が 500 を返すもの（ボディの読み取り自体の失敗。クライアントの切断など）も 400 にする。

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

`gateway_url` は gateway 設定があれば付ける（`stored` 判定はしない）。relay の取得に失敗したら 502。state.json は見ないので、`/api/sites` の `stored_size` に相当するフィールドは無く、`size` は常に自己申告の値。`sites` も `d` の昇順で先頭 `MAX_SITES_PER_AUTHOR_LISTED` 件まで（[取得と表示の上限](../nostr.md#取得と表示の上限nostrbudget)）。

## GET /api/config

```json
{ "config_path": "/path/to/swing.toml", "config_exists": true, "writable": true, "restart_required": false, "sections": [
  { "name": "nostr", "items": [
    { "key": "secret_key", "env": "SWING_NOSTR_SECRET_KEY", "value": "(set, hidden)", "source": "file", "editable": false, "kind": "secret", "description": { "en": "Signing secret key (nsec or hex)...", "ja": "署名用の秘密鍵（nsec または hex）..." } },
    { "key": "relays", "env": "SWING_NOSTR_RELAYS", "value": ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://yabu.me", "wss://relay-jp.nostr.wirednet.jp"], "source": "default", "editable": true, "kind": "list", "raw": ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://yabu.me", "wss://relay-jp.nostr.wirednet.jp"], "description": { "en": "Nostr relays to connect to...", "ja": "接続する Nostr relay（カンマ区切り）" } },
    { "key": "max_total_storage", "env": "SWING_MAX_TOTAL_STORAGE", "value": 107374182400, "display": "100 GiB", "source": "env", "editable": false, "kind": "size", "raw": "100 GiB", "description": { "en": "Total storage cap...", "ja": "保存する全サイト合計の容量上限" } } ] } ] }
```

- `sections` は `nostr`/`ipfs`/`policy`/`agent`/`publish`/`dashboard`/`kubo`/`gateway` の順で、`settings::SETTINGS`（[`../../architecture.md`](../../architecture.md#設定と環境変数)）の宣言順そのままを列挙する（そちらが正本）。カタログの設定はすべて TOML フィールドを持つので、`key` が無い項目は無い。
- `secret_key` の値は常に `"(set, hidden)"` か `"(not set)"`（[`../dashboard.md`](../dashboard.md#秘密鍵を出さない仕組み) を参照）。`editable` は常に `false`（書けるのは `POST /api/setup` だけ）。
- `ipfs.api` は `[kubo].managed = true` のとき固定文字列 `"managed"` になる（動的なポートを含む実際の URL ではなく、`swing up` が `<repo>/api` から解決した値であることを示す。[`../kubo.md`](../kubo.md#動的な-api-ポートと-repoapi)）。`managed = false` なら実際の URL（`[ipfs].api` の値）。
- `kubo.binary`/`kubo.repo` はパスを文字列で返す（`binary` が未設定なら空文字）。`kubo.swarm_port` は常に文字列で、未設定なら `"-"`。`gateway.listen` は無効なら `"off"`。
- `value` は文字列・真偽・数値・文字列配列のいずれか（常に生の値）。容量・時間の項目は読みやすい文字列を `display` に添える（`crate::format::format_bytes`・`format_duration_secs`。値を正確に（小数は 1 桁まで）表せるいちばん大きい単位で、容量は 1024 基数の `"100 GiB"`・`"1.5 KiB"`、時間は `"5m"` など）。個数系（`keep_versions` など）には `display` が付かず、無い項目はフィールドごと出ない。パスの項目（`kind: "path"`）の `value` は解決後のパスで、設定ファイルがあれば、設定ファイルに書いた相対パスと既定値は設定ファイルのディレクトリを起点にした絶対パスになる（[`../../architecture.md#設定と環境変数`](../../architecture.md#設定と環境変数)）。
- `config_path` は常に何か文字列が入る（環境変数だけで動いている、かつ設定ファイルが無くても `null` にはならない。パスの決め方は [`../../architecture.md#設定と環境変数`](../../architecture.md#設定と環境変数) が正本）。`config_exists` はそのパスに実際にファイルがあるかどうか。
- `writable`: `config_exists` なら（`std::fs::OpenOptions::append(true)` で）そのファイルを開けるかどうか、`config_exists` が `false` なら親ディレクトリの `Permissions::readonly()` を見て判定する（`src/dashboard/config_dto.rs::is_config_writable`。副作用は無い）。画面での扱いは [`web.md#設定編集`](web.md#設定編集)。
- `restart_required`: `AppState.restart_required` の値（立つ条件は [`../dashboard.md#設定の読み込みと編集srcsettings`](../dashboard.md#設定の読み込みと編集srcsettings)）。
- `dashboard` セクションに `ui`（真偽値、`SWING_DASHBOARD_UI`）が入る。`listen` は常に `SocketAddr` の文字列。
- 各 `items[]` は追加で次のフィールドを持つ:
  - `source`: `"env"` / `"file"` / `"default"`（`config::Config::sources`、キーは `"<section>.<フィールド名>"`。`ConfigDto` はこれをそのまま `Source::Env`→`"env"` のように文字列化する）。
  - `editable`: カタログ上そのキーが `editable: true` で、かつ `source` が `"env"` ではないときだけ `true`。
  - `kind`: カタログの全キーに付く（`source` や `editable` に関わらず）。`"size"` / `"duration"` / `"bool"` / `"integer"` / `"string"` / `"list"` / `"nip05"` / `"path"` / `"socket_addr"` / `"port"` / `"url"` / `"secret"` / `"listen"` のいずれか。編集可能なキー（[`../dashboard.md#設定の読み込みと編集srcsettings`](../dashboard.md#設定の読み込みと編集srcsettings) の表）の種類は前者 7 種だけ。画面の入力欄の出し分けは [`web.md#設定編集`](web.md#設定編集)。
  - `raw`: 編集可能なキー（一覧は [`../dashboard.md#設定の読み込みと編集srcsettings`](../dashboard.md#設定の読み込みと編集srcsettings)）だけに付く、現在の値を `PUT /api/config`／`POST /api/setup` の `items` にそのまま送り返せる形にしたもの（`size`/`duration` は `parse_size`/`parse_duration_secs` が受け付ける文字列、`list` は文字列配列、それ以外は文字列）。
  - `options`: `kind: "nip05"` のときだけ付く。取りうる値の一覧 `["off", "warn", "require"]`（`config::NIP05_MODE_NAMES`）。
  - `description`: `{ "en": ..., "ja": ... }`。カタログの `Setting.description`（`settings::SETTINGS`）をそのまま返す、1 文の英語・日本語の説明。環境変数名は含まない（`env` フィールドと別出し）。

## PUT /api/config

設定ファイルの値を書き換える。

```json
{ "items": { "policy.max_total_storage": "20GiB", "nostr.relays": ["wss://relay.damus.io", "wss://nos.lol"] } }
```

- キーは `"<section>.<フィールド名>"`（`GET /api/config` の `raw` が付くキーと同じ）。値の形式は `raw` と同じ（`nostr.relays` は空配列だと 400）。受け付けないキーの規則と書き込み手順は [`../dashboard.md#設定の読み込みと編集srcsettings`](../dashboard.md#設定の読み込みと編集srcsettings)。
- 成功したら `restart_required: true` の `ConfigDto`（`GET /api/config` と同じ形）を返す。実際に動いている relay・Kubo・agent への反映は次の再起動から（[`../dashboard.md#設定の読み込みと編集srcsettings`](../dashboard.md#設定の読み込みと編集srcsettings)）。
- 書き込み先のファイルは要求のたびに読み直す（起動時に無くても、前の `PUT`/`POST /api/setup` が作ったファイルに重ねて書く）。設定ファイルを書く API（`PUT /api/config`・`POST /api/setup`・`POST /api/signer/reconnect`）は同じ Mutex で 1 つずつ順に処理する（[`../dashboard.md#設定の読み込みと編集srcsettings`](../dashboard.md#設定の読み込みと編集srcsettings)）。
- 入力・検証の失敗は 400、設定ファイルの読み込み・ディレクトリ作成・書き込み・`rename`・書き込み後の読み直しの失敗は 500（`error!` でログに出す）。どちらもプロセスの状態は変わらない。400 のときはファイルにも触れない。

## POST /api/setup

セットアップモード（[`../up.md#セットアップモード鍵未設定`](../up.md#セットアップモード鍵未設定)）の間だけ使える。それ以外のとき（`AppState::setup_mode() == false`）は 409 `{"error": "swing is already configured; setup is no longer available"}`。

```json
{ "secret_key": null, "remote_signer": false, "items": { "nostr.relays": ["wss://relay.damus.io"], "policy.max_total_storage": "100GiB", "policy.max_per_site": "10GiB", "policy.max_per_account": "20GiB" } }
```

- `remote_signer`（省略時 `false`）: `true` なら秘密鍵を書かず、`POST /api/setup/signer`（下記）で済ませたペアリングの結果を `<state_dir>/remote-signer.json` に保存する（[`../signer.md#remote-signerjson`](../signer.md#remote-signerjson)）。ペアリングが `ready` になっていなければ 409 `{"error": "no signer app is connected yet; scan the QR code first"}`。`true` のとき `secret_key` は見ない。
- `secret_key`: `null`（または省略・空文字）なら `nostr_sdk::Keys::generate()` で新しい鍵を作る。nsec か hex の文字列を渡せば `Keys::parse` でその鍵を使う。
- `items` は `PUT /api/config` と同じ規則で受け付ける。
- 成功したら `[nostr].secret_key` に鍵の hex を書き、`items` と合わせて 1 回の書き込みで保存する（`settings::setup`。手順は [`../dashboard.md#設定の読み込みと編集srcsettings`](../dashboard.md#設定の読み込みと編集srcsettings)）。`remote_signer: true` のときは `items` だけを書いてから `remote-signer.json` を書き、ペアリングの状態を捨てる。
- `npub` は、秘密鍵ならその鍵の、署名アプリならペアリングで受け取ったユーザーの公開鍵。
- 失敗のステータスは `PUT /api/config` と同じ（入力・検証の失敗と `secret_key` が鍵として読めないときは 400、設定ファイルと `remote-signer.json` の読み書きの失敗は 500）。
- 成功してから再起動で `AppState` が作り直されるまでの 2 回目は、設定ファイルに触れず 409 `{"error": "setup is already done; swing is restarting"}`（`ConfigWrites.setup_done`。1 回目に返した `npub` の鍵がそのまま残る）。

```json
{ "ok": true, "npub": "npub1…", "restart": true }
```

- 秘密鍵の値そのものは応答に含めない（`npub` だけ）。
- 成功したら `tokio::spawn` した別タスクで約 300ms 待ってから `shutdown::ExitRequest::restart()` を呼ぶ（`api::schedule_restart`。レスポンスの送出をブロックしない）。これは `POST /api/restart`（下記）と同じ経路で、プロセスを終了させずに `swing up` をプロセス内で再起動する（[`../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）。

## POST /api/setup/signer

セットアップモードの間と、署名アプリを使っている間（つなぎ直し）だけ使える（秘密鍵で動いているときは 409）。NIP-46 の署名アプリとのペアリングを始める（[`../signer.md#ペアリングpairing`](../signer.md#ペアリングpairing)）。前のペアリングがあれば捨てる。

```json
{ "relays": ["wss://relay.primal.net"] }
```

- `relays`: 署名アプリとのやりとりに使う relay。空白だけの要素は無視し、1〜5 個。ws / wss の URL でなければ 400。
- URI の `perms` は常に `get_public_key` と、レプリカ報告・サイトイベント・Follow Set の `sign_event:<kind>`（[`../signer.md#ペアリングpairing`](../signer.md#ペアリングpairing)）。

```json
{ "uri": "nostrconnect://<アプリの公開鍵>?relay=…&secret=…&perms=…&name=SWING&metadata=…", "qr_svg": "<svg …>" }
```

`qr_svg` は `uri` を QR コードにした SVG（黒と白、余白付き、256px 以上）。

## GET /api/setup/signer

`POST /api/setup/signer` と同じ条件で使える（それ以外は 409）。今のペアリングの状態を返す。

```json
{ "state": "ready", "npub": "npub1…", "probe_signed": false, "error": "the signer app refused the request: Rejected" }
```

| `state` | 意味 | 付く値 |
|---|---|---|
| `idle` | ペアリングを始めていない | なし |
| `waiting` | 署名アプリの接続を待っている（最大 10 分。15 秒たっても relay につながらなければ `failed`） | なし |
| `checking` | 接続できた。確認のためレプリカ報告の kind の署名をリクエストしている（最大 60 秒） | `npub` |
| `ready` | ペアリング完了。`POST /api/setup` の `remote_signer: true` で保存できる | `npub`、`probe_signed`（確認の署名が通ったか。自動で許可されたのかその場で承認されたのかは区別しない）、通らなかったときの `error` |
| `failed` | 接続できなかった | `error` |

## POST /api/signer/reconnect

署名アプリを使っている間だけ使える（秘密鍵で動いている・セットアップモードのときは 409 `{"error": "swing does not use a signer app"}`）。ボディは無し。`POST /api/setup/signer` で済ませたペアリングの結果で `<state_dir>/remote-signer.json` を書き換え、プロセス内再起動をスケジュールする（[`POST /api/setup`](#post-apisetup) と同じ）。

- ペアリングが `ready` でなければ 409 `{"error": "no signer app is connected yet; scan the QR code first"}`。
- 署名アプリがいまの公開鍵と別のアカウントで署名するなら 409（`... connect the same Nostr account`）。ファイルは書き換えない。
- `remote-signer.json` の書き込みは設定ファイルを書く他の API と同じ Mutex の中で行い、失敗は 500（`error!` でログに出す）。
- 成功したら `AppState.restart_required` を `true` にし、ペアリングの状態を捨てて `{ "ok": true, "npub": "npub1…", "restart": true }` を返す。

## POST /api/shutdown, POST /api/restart

エージェントプロセス（`swing up`）を止める／再起動する。ボディは不要（送っても無視する）。

`shutdown::ExitRequest`（[`../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）の `stop()`／`restart()` を呼んでから `202 Accepted` を返す。どちらもトークンを cancel するだけでブロックしないので、応答はすぐ返る。実際の終了は `up::run` の中でトークンの cancel が伝わったとき（セットアップモードなら `token.cancelled()` を待っているだけなので即座）。

```json
{ "ok": true, "action": "stop" }
```
```json
{ "ok": true, "action": "restart" }
```

`restart` はプロセスを終了させない。プロセス内再起動の流れ（同じプロセス・同じ PID のまま `up::run` を呼び直す）は [`../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit) が正本。

## POST /api/login-code

使い捨てのログインコードを発行する（`swing dashboard open` と `swing-tray` が使う）。ボディは不要。`expires_in` は有効期限の秒数（コードの性質は [`../dashboard.md#認証srcauthrs-srcdashboardsessionrs`](../dashboard.md#認証srcauthrs-srcdashboardsessionrs)）。

```json
{ "code": "cc2455ac565b74586b0628e1d7bda4c3", "expires_in": 300 }
```

## POST /api/login

認証なしで受け付ける唯一の API。ボディは `{"code": "<ログインコード>"}`（前後の空白は無視、大文字小文字は区別しない）。コードが有効なら消費して `200 {"ok": true}` とセッション cookie（`Set-Cookie`。属性は [`../dashboard.md#認証srcauthrs-srcdashboardsessionrs`](../dashboard.md#認証srcauthrs-srcdashboardsessionrs)）を返す。無効・期限切れ・使用済みなら 401 `{"error": "invalid or expired login code"}`。

## POST /api/token/rotate

トークンを作り直す（効果は [`../dashboard.md#認証srcauthrs-srcdashboardsessionrs`](../dashboard.md#認証srcauthrs-srcdashboardsessionrs)）。成功で `200 {"ok": true}`、ファイルが書けなければ 500。
