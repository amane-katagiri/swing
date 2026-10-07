# Follow Set・webring・レプリカ（`src/dashboard/api.rs`, `src/dashboard/dto.rs`）

[`../http-api.md`](../http-api.md) の子ページ。共通の形式・エラー・準備状態・件数の上限と同時実行数は親ページを参照。

## GET /api/mirror

```json
{ "title": "SWING mirror list", "note": null, "members": [ { "pubkey": "…", "npub": "…" } ] }
```

Follow Set が無ければ `title: null`、`members: []`。`note` は relay から取れた Follow Set より `state.json` に保存済みの版を使った場合の注記で、`add`・`remove` の `note`（下記）から前後の括弧を外した文字列。無ければ `null`。

## POST /api/mirror/add, POST /api/mirror/remove

リクエスト: `{ "keys": ["npub1…", "hex…", "nprofile1…"] }`。空、件数の上限（[`../http-api.md#件数と負荷`](../http-api.md#件数と負荷)）超え、パース不能のいずれかで 400。

```json
{ "changed": [ { "pubkey": "…", "npub": "…" } ], "unchanged": [ { "pubkey": "…", "npub": "…" } ], "published": true, "relays": [ { "relay": "wss://…", "ok": true, "error": null } ], "members": [ { "pubkey": "…", "npub": "…" } ], "note": null, "follow_set_found": true }
```

- `changed`: 実際に追加・削除したもの。`unchanged`: 既に追加済み／もともと未登録で no-op だったもの。
- 変更が無ければ `published: false`、`relays: []`。`published: true` なのにどの relay にも受理されなければ 502。
- 成功（1 relay 以上が accept）したら agent に即時 refresh を促す。
- 現在の Follow Set の取得から発行までを、`mirror/add`・`mirror/remove` どうしで 1 件ずつ順に行う（`AppState::lock_mirror_writes`。ロックは `dashboard::locks::Locks` の中にあり、このメソッドからしか取れない）。
- `note`: relay から取れた Follow Set より `state.json` に保存済みの版を使った場合の注記（`(relays returned an older follow set; ...)` / `(follow set not found on relays; ...)`）。括弧付きの文字列そのまま、無ければ `null`。
- `follow_set_found`: 操作前に Follow Set が見つかっていたか。
- `add` の結果の `p` タグのうち公開鍵としてパースできたものの数が `MAX_FOLLOW_SET_ENTRIES`（500。[取得と表示の上限](../../nostr/fetch.md)）を超えるときは publish せず 409（`{"error": "would grow the follow set to <N> entries, over the 500-entry limit; remove some first"}`）。それ以外の失敗は 502。
- relay から Follow Set を取れなかったとき（答えた relay が 1 つも無い）も 502 で、「Follow Set が無い」とはみなさず publish もしない。

## GET /api/webring?root=\<key\>&depth=\<N\>

`root` は繰り返し指定可、省略時は自分の pubkey 1 つ、件数の上限を超えると 400。`depth` は省略時 2、4 超か非数値で 400。たどり方・グラフの組み立ては CLI の `swing webring` と同じ（[`../../cli/views.md#webring`](../../cli/views.md#webring)）。

```json
{ "depth": 2,
  "nodes": [ { "pubkey": "…", "npub": "…", "short_npub": "npub1abc…uvwxyz", "names": ["example.com"], "label": "example.com", "depth": 0, "root": true, "has_follow_set": true } ],
  "edges": [ { "from": "<hex>", "to": "<hex>", "mutual": true } ], "beyond": 0, "over_budget": 0,
  "referencing": { "accounts": [ { "pubkey": "…", "npub": "…" } ], "more": 0 },
  "text": "…swing webring と同じ text 出力…", "dot": "…同じ dot 出力…", "mermaid": "…同じ mermaid 出力…" }
```

- `root: true` は `depth == 0` のノード。ノードの並びは（深さ、ラベル）順。`names` は 1 アカウントあたり `MAX_SITES_PER_AUTHOR_LISTED` 件まで。
- 双方向の組は `mutual: true` の辺 1 本、片方向は `mutual: false` の辺。
- `beyond` と `over_budget` は `swing webring` の同名のカウンタ（[`../../cli/views.md#webring`](../../cli/views.md#webring)、[取得と表示の上限](../../nostr/fetch.md)）。
- `referencing`: `#p` で見つかった、起点を名指ししているだけでクロールには加えていないアカウント（[`../../nostr.md#レプリカ報告の信頼度replicastier`](../../nostr.md#レプリカ報告の信頼度replicastier)）。`accounts` は先頭 `MAX_REFERENCING_LISTED` 件まで、`more` は切り詰めで落ちた件数。`nodes`・`edges`・`dot`・`mermaid` には含まれない。

## GET /api/replicas?key=\<key\>

`key` は繰り返し指定可、省略時は自分 1 つ、件数の上限を超えると 400。集計は CLI の `swing replicas` と同じ（[`../../cli/views.md#replicas`](../../cli/views.md#replicas)）。

```json
{ "authors": [ { "pubkey": "…", "npub": "…", "sites": [
  { "d": "example.com", "cid": "bafy…", "replicas": 2, "unverified": 1, "reports": 3, "dropped": 0, "reporters": [ { "pubkey": "…", "npub": "…", "latest": true, "tier": "chosen" } ] }
] } ] }
```

- `tier`: `"author"` / `"chosen"` / `"other"` のいずれか（定義は [`../../nostr.md#レプリカ報告の信頼度replicastier`](../../nostr.md#レプリカ報告の信頼度replicastier)）。`"other"` が CLI の `[unverified]` に相当する。
- `replicas` は最新版を持つ報告者のうち tier が `author`・`chosen` の数、`unverified` は tier が `other` の数。
- `reports`（＝ `reporters.length`）は `MAX_REPORTS_PER_SITE` で切り詰めた後の件数、`dropped` は切り詰めで落ちた件数。`reporters` の並びは tier（`author`→`chosen`→`other`）、同じ tier では `latest: true` が先、最後に hex の順（[取得と表示の上限](../../nostr/fetch.md)）。
- `sites` は 1 作者あたり `MAX_SITES_PER_AUTHOR_LISTED` 件まで。
