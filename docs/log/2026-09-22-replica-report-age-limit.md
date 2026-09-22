# レプリカ報告の年齢制限

## きっかけ

kind 35981 のレプリカ報告は `replicas::collect_reports`（`src/replicas.rs`）が `ReplicaReport::is_expired_at`（旧 `src/nostr.rs`）で数えていたが、2 つの穴があった。

1. **`expiration` が壊れていても「無期限」として数えていた。** `parse_replica_report` は `expiration` を nostr-sdk の `Tags::expiration()` から取っていたが、この関数はタグが無いときと、タグはあるが値がパースできないとき（`nostr-0.45.5` の `src/event/tag/list.rs:152-158`、`Nip40Tag::try_from` が失敗すると素通しで `None` を返す）の両方で `None` を返す。`None` は「無期限」を意味するので、壊れた `expiration` を持つ報告は無期限の報告と区別できなかった。
2. **`expiration` に上限が無く、`created_at` にも下限が無かった。** サイトイベントの `created_at` には `MAX_FUTURE_SKEW`（900 秒、コミット 90eb335）があるが、レプリカ報告にはこれが無く、`expiration` にも上限が無かった。1 回だけ署名すれば `expiration` を西暦 5138 年にでも、無しにでもできる報告者が、二度と報告を出し直さずに永久に数えられ続けられた。プロトコル文書（`docs/protocol.md` 第 8 節）は「止まった報告者の報告は期限切れで数えられなくなる」としているが、これが成立していなかった。

## やったこと

### `expiration` タグを自前でパースする

`parse_replica_report`（`src/nostr.rs`）で `Tags::expiration()` の使用をやめ、`tag_value(event, "expiration")` でタグの生の値を取り、あれば `u64` としてパースする。パースに失敗したら報告全体を `Err` で拒否する。既存の `cid` タグの検証と同じ扱いにした。タグが無ければ `None`（無期限）のまま。

### `is_expired_at` を `counts_at` に置き換える

`ReplicaReport::is_expired_at(now)` を `counts_at(now)` に変えた。「期限切れかどうか」ではなく「今数えるかどうか」を返す関数にし、3 条件の AND にした。

```rust
pub fn counts_at(&self, now: u64) -> bool {
    self.created_at <= now.saturating_add(crate::policy::MAX_FUTURE_SKEW)
        && now.saturating_sub(self.created_at) <= MAX_REPORT_AGE
        && self.expiration.is_none_or(|exp| exp > now)
}
```

`MAX_FUTURE_SKEW` は `policy.rs` の定数を `pub` にして再利用した（サイトイベントと同じ「未来の `created_at` は詐称の手段」という理由が、報告の `created_at` にもそのまま当てはまる）。

新しく `nostr::MAX_REPORT_AGE`（7 日）を足した。`expiration` の値そのものに上限を設ける案も検討したが、プロトコル上 `expiration` 無し（無期限）は正当な値であり、それを塞げない上限には意味が無い。代わりに `created_at` からの経過時間という、自己申告に頼らない基準にした。

7 日にしたのは、agent 自身が送る報告の `report_ttl` の既定が `3d`（`swing.example.toml:30`、`src/agent/replicas.rs:167`）であるため。これより長く取ることで、`report_ttl` を長めに設定した他クライアントの報告が、期限切れより先に「数えなくなる」ことがないようにした。設定値にはせず定数にした（`MAX_FUTURE_SKEW` と同じ理由で、判定基準を増やさないため）。

`src/replicas.rs` の `collect_reports` は `report.is_expired_at(now)` を `!report.counts_at(now)` に変えるだけで済んだ。

### 自分が送る報告への影響を確認した

`src/agent/replicas.rs` の `load_sent_reports` / `sync_reports` は `parse_replica_report` の結果から `cids` と `created_at` しか読んでおらず、`expiration` や `counts_at` は見ていない。自分が送る報告は `build_replica_report_builder` で必ず `expiration = created_at + report_ttl`（有限値）を付けるので、`expiration` のパースに失敗することもない。影響は無い。

`src/agent/test_support.rs` の `take_reports` は送信直後のイベントを `Tags::expiration()` で直接検査しており、`parse_replica_report` を経由しないテストヘルパーなのでそのままにした。

## 検証

- `src/nostr.rs`: `expiration` タグが数値以外・空文字のとき `parse_replica_report` が `Err` になること、タグが無いとき `None` になることをテストで確認。`counts_at` について、無期限の新しい報告が数えられること、`MAX_REPORT_AGE` を超えた報告は `expiration` が遠い未来でも無しでも数えられないこと、`created_at` が `MAX_FUTURE_SKEW` を超えて先だと数えられないこと、許容ずれ以内は数えられること、`expiration` がちょうど `now` のとき数えられないことを確認。
- `src/replicas.rs`: `expiration` が壊れた報告が `collect_reports` の結果から落ちること、`MAX_REPORT_AGE` を超えて古い報告が（`expiration` が遠い未来でも）落ちることを確認。
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` を通した（261 passed, 12 ignored）。

## 見送ったこと

- `expiration` の値そのものへの上限。無期限（`expiration` 無し）が正当な値である以上、値の上限だけでは同じ抜け道が残る。
- `MAX_REPORT_AGE` の設定化。`MAX_FUTURE_SKEW` と同じ理由で定数のままにした。
