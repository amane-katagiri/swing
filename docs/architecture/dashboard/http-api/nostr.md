# Follow Set・webring・レプリカ（`src/dashboard/api.rs`, `src/dashboard/dto.rs`）

[`../http-api.md`](../http-api.md) の子ページ。共通の形式・エラー・準備状態・件数の上限と同時実行数は親ページを参照。

## GET /api/mirror

```json
{ "title": "SWING mirror list", "note": null, "members": [ { "pubkey": "…", "npub": "…" } ] }
```

`mirror::collect_mirror_list` の結果（[`../../mirror.md#使う-follow-set`](../../mirror.md#使う-follow-set)）。Follow Set が無ければ `title: null`、`members: []`。`note` は保存済みの版を使ったときの注記から前後の括弧を外した文字列。無ければ `null`。

## POST /api/mirror/add, POST /api/mirror/remove

リクエスト: `{ "keys": ["npub1…", "hex…", "nprofile1…"] }`。空、件数の上限（[`../http-api.md#件数と負荷`](../http-api.md#件数と負荷)）超え、パース不能のいずれかで 400。

```json
{ "changed": [ { "pubkey": "…", "npub": "…" } ], "unchanged": [ { "pubkey": "…", "npub": "…" } ], "published": true, "relays": [ { "relay": "wss://…", "ok": true, "error": null } ], "members": [ { "pubkey": "…", "npub": "…" } ], "note": null, "follow_set_found": true }
```

Follow Set の書き換え（`mirror::apply_add` / `apply_remove`）は [`../../mirror.md`](../../mirror.md#follow-set-の書き換えmirrorapply_add--apply_remove)。

応答:

- `changed`: 実際に追加・削除したもの。`unchanged`: 既に追加済み／もともと未登録で no-op だったもの。
- 変更が無ければ `published: false`、`relays: []`。`published: true` なのにどの relay にも受理されなければ 502。
- 成功（1 relay 以上が accept）したら agent をすぐ poll させる（[`../../dashboard.md#概要`](../../dashboard.md#概要)）。
- 現在の Follow Set の取得から発行までを、`mirror/add`・`mirror/remove` どうしで 1 件ずつ順に行う（`AppState.locks.mirror_writes()`。ロックは `dashboard::locks::Locks` の中にあり、このメソッドからしか取れない）。
- `note`: 保存済みの版を使ったときの注記（[`../../mirror.md#使う-follow-set`](../../mirror.md#使う-follow-set)）。括弧付きの文字列そのまま、無ければ `null`。
- `follow_set_found`: 操作前に Follow Set が見つかっていたか。
- `add` が Follow Set の上限（`MAX_FOLLOW_SET_ENTRIES`）を超える（[`../../mirror.md`](../../mirror.md#follow-set-の書き換えmirrorapply_add--apply_remove)）ときは publish せず 409（`{"error": "would grow the follow set to <N> entries, over the 500-entry limit; remove some first"}`）。それ以外の失敗は 502。
- relay から Follow Set を取れなかったとき（答えた relay が 1 つも無い）も 502 で、「Follow Set が無い」とはみなさず publish もしない。

## GET /api/webring?root=\<key\>&depth=\<N\>

`root` は繰り返し指定可、省略時は自分の pubkey 1 つ、件数の上限を超えると 400。`depth` は省略時 2、4 超か非数値で 400。`webring::collect` の結果で、たどり方・グラフ・名前は [`../../webring.md`](../../webring.md)。

```json
{ "depth": 2,
  "nodes": [ { "pubkey": "…", "npub": "…", "short_npub": "npub1abc…uvwxyz", "names": ["example.com"], "label": "example.com", "depth": 0, "root": true, "has_follow_set": true } ],
  "edges": [ { "from": "<hex>", "to": "<hex>", "mutual": true } ], "beyond": 0, "over_budget": 0,
  "referencing": { "accounts": [ { "pubkey": "…", "npub": "…" } ], "more": 0 },
  "text": "…swing webring と同じ text 出力…", "dot": "…同じ dot 出力…", "mermaid": "…同じ mermaid 出力…" }
```

- `root: true` は `depth == 0` のノード。ノードの並びは（深さ、ラベル）順。`names` は[ノードの名前](../../webring.md#ノードの名前)、`label` は `swing webring` の `text` と同じ表示名。
- 双方向の組は `mutual: true` の辺 1 本、片方向は `mutual: false` の辺。
- `beyond`・`over_budget`・`referencing` の意味は [`../../webring.md`](../../webring.md)。`referencing.more` は `referencing_dropped`。`referencing` は `nodes`・`edges`・`dot`・`mermaid` には含まれない。
- `has_follow_set: false` は Follow Set を取得できなかったノード。`text`・`dot`・`mermaid` は `swing webring` の各 `--format` の出力（[`../../cli/views.md#webring`](../../cli/views.md#webring)）と同じ。

## GET /api/replicas?key=\<key\>

`key` は繰り返し指定可、省略時は自分 1 つ、件数の上限を超えると 400。集計は [`replicas::collect`](../../nostr.md#一覧の集計replicascollect)。

```json
{ "authors": [ { "pubkey": "…", "npub": "…", "sites": [
  { "d": "example.com", "cid": "bafy…", "replicas": 2, "unverified": 1, "reports": 3, "dropped": 0, "reporters": [ { "pubkey": "…", "npub": "…", "latest": true, "tier": "chosen" } ] }
] } ] }
```

- `tier`: `"author"` / `"chosen"` / `"other"` のいずれか（定義は [`../../nostr.md#レプリカ報告の信頼度replicastier`](../../nostr.md#レプリカ報告の信頼度replicastier)）。`"other"` が CLI の `[unverified]` に相当する。
- `replicas` は最新版を持つ報告者のうち tier が `author`・`chosen` の数、`unverified` は tier が `other` の数。
- `reports`（＝ `reporters.length`）は切り詰めた後の件数、`dropped` は切り詰めで落ちた件数。`reporters` の並びは [`replicas_of`](../../nostr.md#レプリカ報告の信頼度replicastier) の順。
