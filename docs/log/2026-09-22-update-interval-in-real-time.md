# 取り込み間隔の基準を実時刻にする

## きっかけ

`SWING_MIN_UPDATE_INTERVAL` を長くしたときの挙動を確かめていて、2 つの問題が見つかった。

1. **見送った更新を取り直せない。** 判定が `候補.created_at - 保存版.created_at < min_update_interval` だったため、同じイベントを何度評価しても結論が変わらない。サイトイベントは addressable event で relay に最新 1 件しか残らないので、窓に入らなかった最新版は「作者が次に更新するまで据え置き」になっていた。poll ごとに最新版を投入し直す仕組み（[agent.md](../architecture/agent.md) 全体の流れ 4.4）はあるのに、再試行が意味を持っていなかった。
2. **`created_at` の詐称で間隔制限を回避できる。** `created_at` は作者の自己申告で、署名が縛るのは「その pubkey がその値で署名した」ことだけ。`parse_site_event` は値をそのまま取り、購読・取得のフィルタにも `until` は無く、`policy::decide` は `now` を受け取りながら時刻の妥当性を見ていなかった。`created_at` を `now, now+interval, now+2*interval, ...` と刻んで秒単位で連投すれば間引きは効かない。

2 は署名が要るので他人になりすませず、`submit` が Follow Set 外の pubkey を捨てるため、できるのは自分がミラー対象に入れた相手だけ。容量系の上限（`max_update_size`・`max_per_site`・`max_per_account`・`max_total_storage`・`keep_versions`）と `duplicate_cid` は `created_at` と無関係に効くのでディスクは食い潰されないが、取得と IO の churn は起こせる。

さらに未来の `created_at` を保存してしまうと、`keep_days` の判定（`created_at < now - keep_days*86400`）に永久に掛からない版ができ、`stale` 判定のせいでその後のまともな時刻の更新がすべて弾かれてサイトが凍結する。

## やったこと

### 未来の `created_at` を拒否する

`policy::decide` の最初に、`created_at` が `now + MAX_FUTURE_SKEW`（900 秒）を超えるなら `future_created_at` で skip する判定を足した。許容ずれを設定値にはせず定数にしている（設定面を増やさないため）。保存しないので、作者はまともな時刻で出し直せば復帰でき、凍結もしない。

`stale` と `keep_days` は `created_at` 基準のままにした。この 2 つは「イベントの新旧」「版の古さ」を見るものなので基準としては正しく、詐称への備えは入口で拒否する側に寄せた。

### 間隔の基準を `stored_at` にする

`policy::VersionInfo` に `stored_at` を足し、間隔の判定を `now - 同サイトの最大の stored_at < min_update_interval` に変えた。`stored_at` は `state.json` の版にすでに入っていて（`src/state.rs`、書き込みは `src/agent/store.rs` の `now_secs()`）、`version_infos` で詰め替えるだけで済んだ。state の形式は変わらないので移行の処置は要らない。

これで判定が `now` に依存するようになり、既存の poll ごとの再投入がそのまま回収機構になる。見送った版も `min_update_interval` 経過後の poll で受理される。取り込みの遅れは最大で `min_update_interval + poll_interval`。relay に中間の版は残らないので途中の版は取れないままだが、最新の内容には必ず追いつく。

事前判定（保存の順序 1）で skip するのは fetch より前なので、見送っている間に `dag/export` は走らない。取得コストは増えない。

### 既定値を `10m` から `1h` に

これまで短かったのは、`created_at` 基準では長くするほど「最新版が窓に入らず永久に据え置き」のリスクが増えたから。回収できるようになった今は、長くして失うのは鮮度だけになった。

- 静止期間のあとの 1 発目は `now - stored_at` が十分開いているので今までどおり即取り込む。間隔が効くのは連投を畳むときだけで、そのとき拾うのは途中の版ではなく最終形になった。
- `10m` は 1 サイトあたり最大 144 回/日の取得を許す。個人の静的サイトの更新頻度に対して過剰だった。`1h` なら 24 回/日。
- 代償は版履歴の粒度が粗くなること（`keep_versions = 5` で保持する 5 版の間隔が開く）。個人サイトのミラーでは許容と判断した。

## 検証

- `policy.rs` のユニットテストを更新・追加。`stored_at` 基準であること、同じイベントが待ち時間の経過後に受理されること、`created_at` を先に振っても待ちが縮まらないこと、許容ずれ以内は受理すること。
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` を通した（253 passed）。

## 見送ったこと

- `MAX_FUTURE_SKEW` の設定化。
- relay 側の `created_at` 上限（NIP-11 の `created_at_upper_limit`）への依存。relay 任せにはせず受信側で判定する。
