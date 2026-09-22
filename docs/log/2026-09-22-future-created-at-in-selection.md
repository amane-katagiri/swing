# 選択における未来の created_at

## きっかけ

`created_at` はイベントの署名者が自己申告する値で、`policy::decide`（コミット 90eb335）は保存するかどうかの判定でこれを拒否し（`MAX_FUTURE_SKEW`、900 秒）、`ReplicaReport::counts_at`（コミット 3c0711f）はレプリカ報告を数えるかどうかの判定で同じことをした。だが `docs/todo.md` に残っていた監査項目の通り、「どれを現在の版として扱うか」「どの Follow Set を今の対象リストとして使うか」を決める側はこの制限を持たず、`created_at` が最大のものを無条件に選んでいた。

具体的には 2 種類の穴があった。

1. **表示・集計の毒害。** `nostr::select_latest`（サイトイベントの現行版選び）は `created_at` の最大値を素朴に取る。攻撃者が遠未来の `created_at` を付けたサイトイベントを 1 件 relay に流すと、`policy::decide` はそれを正しく拒否して保存しないが、`swing sites` / `replicas` / `webring` とダッシュボードの `/api/publish/sites` はそのイベントの CID を「現在の版」として表示し、レプリカ数もそれに対して数える。relay にまだ残っていれば正当な旧版がある場合でも、`select_latest` の時点で捨てられるので `Agent::submit` にすら渡らない。`Agent::submit` 自身にも同じ穴があり、同じサイトを処理中のときにキューへ積む「次のイベント」を `created_at` の大小だけで決めていたので、遠未来のイベントが実行待ちの正当なイベントを追い出せた。
2. **Follow Set の凍結。** `nostr::choose_follow_set`（agent の自分の Follow Set 選び）と `nostr::is_newer_replaceable` を使うすべての箇所（`RelayClient::fetch_follow_set` / `fetch_follow_sets`、`nostr::newest_by_address`、`mirror.rs` の `newest_follow_set`）は NIP-01 の置き換え規則（`created_at` が大きい方）だけで新しさを比べていた。遠未来の `created_at` を持つ kind 30000 が一度 `state.follow_set` に保存されると、以後どんなにまともな時刻で再署名しても「新しい」と判定されず、ミラー対象リストがその版のまま凍結する。自分の Follow Set はこの構造上、自分の秘密鍵が要る（自己申告を疑うという意味では作者が自分自身であっても同じ理由が当てはまる：署名鍵の取り違えやクライアントの不具合で一度でも先の時刻を書いてしまえば、正しい時刻の更新が二度と勝てなくなる）。一方、webring や replicas が取得する他アカウントの Follow Set（`fetch_follow_sets`・`newest_by_address`）は他人の署名で成立するので、その相手が自分の Follow Set を遠未来の `created_at` で 1 回発行するだけで、以後 webring 上のその人の対象リストが凍結して見える。

## やったこと

### `MAX_FUTURE_SKEW` と `plausible_at` を `nostr.rs` に集約する

`policy.rs` にあった `pub const MAX_FUTURE_SKEW: u64 = 900;`（コメントごと）を `src/nostr.rs` に移し、判定を関数に切り出した。

```rust
// `created_at` is self-declared by the author, so a small tolerance is all that
// separates honest clock skew from a timestamp forged to defeat the rate limit.
pub const MAX_FUTURE_SKEW: u64 = 900;

pub fn plausible_at(created_at: u64, now: u64) -> bool {
    created_at <= now.saturating_add(MAX_FUTURE_SKEW)
}
```

`policy::decide` と `ReplicaReport::counts_at` はここから読むだけにした（`policy.rs` が `nostr` に依存する向きで、逆方向の依存は無い）。判定順序や `policy::decide` の他の分岐は変えていない。

### 現行版・Follow Set の選択すべてに適用する

- `nostr::select_latest(events, now)`: 候補ごとに `plausible_at` で先にふるい、通ったものだけで最大の `created_at` を選ぶ。呼び出し元（`agent/follow.rs` の過去分取り込み、`mirror.rs` の `sites`、`replicas.rs` の `collect`、`webring.rs` の `collect`、`dashboard/api.rs` の `publish_sites`）はそれぞれの `now`（agent は `now_secs()`、CLI・ダッシュボードは `Timestamp::now().as_secs()`）を渡すよう変更した。
- `nostr::newest_by_address(events, now)`: 住所（pubkey + kind + d タグ）ごとに最新を選ぶ内部の汎用関数。同じくふるい落としを先に行う。`replicas::collect_reports`（レプリカ報告の重複排除）と `agent/replicas.rs` の `load_sent_reports`（自分が送った報告の重複排除）の両方で使っているので、レプリカ報告側の「未来の報告に本来の報告が上書きされて `counts_at` に到達すらしない」問題もついでに塞がれた。
- `RelayClient::fetch_follow_set` / `fetch_follow_sets`: relay から取得した直後、`is_follow_set_of` の検証と並べて `plausible_at` でふるう。`Timestamp::now()` を関数内で読むだけで、呼び出し元のシグネチャは変えずに済んだ。
- `nostr::choose_follow_set(fetched, fetch_succeeded, stored, now)`: 引数に `now` を追加し、`fetched` と `stored` の両方を `plausible_at` でフィルタしてから、既存の 6 分岐の match に渡す。`stored` も同じ基準でふるうのが要点で、これをやらないと前述の凍結が直らない（`fetched` だけ弾いても、次の poll でまた「保存済みの方が新しい」と判定されて凍結したまま）。`fetch_follow_set` が既に未来のイベントを弾いているので `fetched` 側のフィルタは二重になるが、無害なので残した。
- `mirror.rs` の `newest_follow_set(fetched, saved, now)`: CLI 表示用の同種の関数にも同じ引数と同じフィルタを足した。
- `Agent::submit`: 対象判定より前に `nostr::plausible_at(ev.created_at, now_secs())` を確かめ、false なら `warn!`（`site`・`pubkey`・`created_at`・`reason = "future_created_at"`）を出してキューに触れずに終わる。実行中・待機中のイベントを置き換えることはない。

### 変更しなかった箇所

`RelayClient::fetch_follow_set_authors_referencing`（`#p` で Follow Set の作者を見つけるための webring の補助取得）は「新しさ」を比較せず、条件に合うイベントの作者をそのまま集合に入れるだけなので、ここでの意味の毒害や凍結は起きない。手を入れなかった。

## 検証

- `src/nostr.rs`: `select_latest` が遠未来のイベントを無視して plausible な最新を残すこと、`choose_follow_set` が遠未来の `fetched` を無視して `stored` を残すこと（`republish: true`）、遠未来の `stored` を無視して plausible な `fetched` を採用・保存すること、遠未来の `stored` しか無ければ `None`（保存も再送もしない）を返すこと、`newest_by_address` が遠未来のイベントを無視すること、900 秒ちょうどはどこでも許容されることをテストで確認した。
- `src/agent/store.rs`: `submit` が遠未来のイベントをキューに触れずに捨て、既にキューにある正当なイベントを置き換えないことを、既存の `submit_coalesces_events_for_a_busy_site_to_the_newest` と同じ形のフィクスチャで確認した。
- `src/mirror.rs`: CLI 側の `newest_follow_set` が毒された `saved` を無視すること、`fetched` があれば復旧に使えることを確認した。
- `src/policy.rs` の既存テストは `nostr::MAX_FUTURE_SKEW` からの参照に変えただけで、判定順序のテストはすべて通ったままであることを確認した。
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` を通した（268 passed, 12 ignored）。

## 見送ったこと

- `MAX_FUTURE_SKEW` の設定化。これまでの 2 件の修正と同じ理由で、判定基準を増やさないために定数のままにした。
- relay が住所（pubkey + kind + d）ごとに最新の 1 件しか保持しない、という NIP-01 の性質そのものは変えられない。ある relay が一度でも遠未来のイベントを受理すると、その relay 上ではそれ以前の正当な版は失われる。今回の修正はクライアント側で遠未来のイベントを「無いもの」として扱うので、
  - 他の relay がまだ正当な版を持っていればそれを拾える（`fetch_events` は設定した relay 群すべての結果を束ねて返すので、1 つの relay が毒されていても他が生きていれば `select_latest` の入力プールに正当な版が残る）、
  - 作者が改めてまともな時刻で発行し直せば、それを受理した relay からは次の取得でそれが最新として選ばれる（`select_latest` は毒された版を比較対象から除外する）。ただし遠未来のイベントを既に持っている relay は、NIP-01 の置き換え規則で再発行を「古い」として拒否するので、この経路が効くのは毒されていない relay か、毒されたイベントを消した relay に限られる、
  という 2 つの経路でしか復旧しない。設定した relay がすべて毒されている間は「現在の版が無い」ままになる。これは relay の仕様上の制約であり、この修正の範囲では解消できない。
