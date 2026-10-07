# ポリシー判定（`policy.rs`）

[`../agent.md`](../agent.md) の子ページ。判定と取得の上限を使う箇所は [`../agent.md#保存の順序`](../agent.md#保存の順序) と [`../agent.md#sweep`](../agent.md#sweep)。

## 判定の順（`policy::decide`）

`policy::decide` は純粋関数。入力は同サイトの既存版、使用量（他サイトの合計容量、同じ pubkey の他サイトの合計容量とサイト数）、候補（cid, size, created_at）、ポリシー設定、現在時刻。出力は `Decision { store: Option<String>, evict: Vec<String>, reason: String }`。

1. `nostr::plausible_at` が false（`created_at` が未来ずれの許容を超えて先。[`nostr.md`](../nostr.md#未来ずれの許容nostrmax_future_skew)）なら skip（`future_created_at`）。
2. 同じ CID が同サイトに記録済みなら skip（`duplicate_cid`）。
3. 新しいサイトで、同じ pubkey の記録済みのサイトが `max_sites_per_account` 個以上あれば skip（`max_sites_per_account`）。
4. `created_at` が同サイトの最新版以下なら skip（`stale`）。`created_at` が等しいときは `id` によらず先に保存した版を保つ（[`../protocol.md`](../../protocol.md) 第 4 節）。
5. 現在時刻が同サイトの最大の `stored_at` から `min_update_interval` 未満なら skip（`min_update_interval`）。
6. `size` が `max_update_size` を超えるなら skip（`max_update_size`）。
7. 同サイト合計が `max_per_site` を超えるなら古い版（`created_at` の小さい版）から evict する。新版単体で超えるなら skip（`max_per_site_exceeded_alone`）。
8. `keep_versions`（最低 1 に丸める）を超える古い版を evict する。
9. `created_at` が現在から `keep_days` 日より前の版を evict する。最新版は残す。`keep_days = 0` なら何もしない。
10. evict 後、同じ pubkey の全サイト合計が `max_per_account` を超えるなら skip（`max_per_account`）。他のサイトは削らない。
11. evict 後の全サイト合計が `max_total_storage` を超えるなら skip（`max_total_storage`）。他のサイトは削らない。

`size` 不明の事前判定では 6 を飛ばし、新版を 0 バイトとして 7〜11 を評価する。

間隔（5）は `stored_at` で測る。見送った版は次の poll で再評価される。

## 取得の上限

`policy::fetch_budget` は「保存の順序」の 4・5 で使う取得の上限を、次の最小値として返す。新しい版はどれを evict しても残るので、これを超える内容は取得後の `policy::decide` で必ず skip になる。

- `policy::fetch_limit`（`max_update_size`・`max_per_site`・`max_per_account` の最小値）
- `max_per_account` から、そのアカウントのほかのサイトの合計を引いた残り
- `max_total_storage` から、ほかのサイトの合計を引いた残り
