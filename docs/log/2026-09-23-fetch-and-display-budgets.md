# 2026-09-23 relay から取得するイベントと表示件数の上限

## きっかけ

`docs/todo.md` にあった監査項目（レビュー・DoS）。kind 35980（サイトイベント）・35981（レプリカ報告）・30000（Follow Set）はどれも誰でも捨て鍵で大量に発行できるが、読み取り専用の経路（`swing sites` / `replicas` / `webring`、ダッシュボードの `/api/sites` / `/api/replicas` / `/api/webring`）は relay からの取得件数にも、集計後の表示件数にも上限が無かった。`fetch_events` は 30 秒のタイムアウトだけで止めており、`extract_follow_set_pubkeys` は Follow Set の `p` タグを無制限に読み、`replicas::collect` / `webring::collect` は 1 作者の `d` をすべて引き、`webring::crawl` はノード数を無制限に広げ、`replicas::collect_reports` は 1 サイトの報告を無制限に数えていた。ダッシュボードの GET はこれらと同じ `collect_*` 関数を呼ぶので、認証の無いブラウザからも同じ負荷をかけられる。

## 決めたこと

| 決定 | 理由 |
|---|---|
| 上限は設定項目にせず `nostr::budget` 内の定数にする（`MAX_FOLLOW_SET_ENTRIES=500`・`MAX_SITES_PER_AUTHOR_LISTED=50`・`MAX_REPORTS_PER_SITE=200`・`MAX_CRAWL_NODES=1000`・`MAX_RELAY_FETCH_LIMIT=20,000`） | 資源保護のための下限的な安全弁であり、運用者が緩めたいと思うような値ではない。設定項目にすると「大きくすれば直る」という誤った期待を招く |
| Follow Set の `p` タグの上限は自分の Follow Set にも例外なく適用し、超えたら `warn!` で知らせる。`swing mirror add` は上限を超えて追加しようとするとエラーで終了する | 上限を「他人の Follow Set だけの防御」にすると、運用者自身が誤って 500 件を超える Follow Set を作ったときに一部の対象が静かに無視され、気づかないまま「一部しかミラーされていない」状態になる。`mirror add` は黙って切り詰めるより先に断る方が安全 |
| 1 作者あたりのサイト表示件数は `d` の昇順で先頭 50 件。agent の取り込み側にある `limit_sites_per_account`（ポリシー値 `max_sites_per_account`、保存済みを優先する別ロジック）はそのまま残す | 表示側の上限と保存側の上限は目的が違う（表示側は relay 応答の解析・レンダリング量の抑制、保存側はディスクの割り当て）。1 つにまとめると片方の要件を歪める |
| レプリカ報告は `created_at` が新しい順に先頭 200 件を残し、`reports`（残した数）とは別に `dropped`（切り捨てた数）を持たせる | CLI・ダッシュボードのどちらでも「何件残したか」と「何件落としたか」を区別して出せるようにする。古い報告を優先して落とすのは、`expiration` が近い＝出し直す機会が既にあった報告者から順に切るのが妥当なため |
| `webring::crawl` はノード総数が 1000 に達したら以降の新規ノードを弾き、弾いた数を `over_budget` として別カウンタに積む（深さの上限外で表示しない `beyond` とは別物） | frontier に入らないノードは次のレベルの `follow_sets`/`referencing` 呼び出しにも現れないので、上限を frontier より前段（ノード admission）に置くだけで 1 レベルあたりの件数も自動的に抑えられる |
| relay への `Filter::limit` は `count * per` を `MAX_RELAY_FETCH_LIMIT`（20,000）で頭打ちにする（`fetch_site_events`: authors×50、`fetch_replica_reports`: sites×200、`fetch_follow_sets`: authors×2、`fetch_follow_set_authors_referencing`: targets×100） | relay 側の応答件数をそもそも絞ることで、集計側の上限（表示件数の上限）と二重に効かせる。`fetch_follow_sets` の 2 倍は「1 作者につき有効な版は 1 つ」という前提を、relay が古い版を返す場合に備えて緩めた値 |

## やったこと

- `src/nostr.rs`: `pub mod budget` に上記の定数をまとめた。`RelayClient` の 4 つの取得関数（`fetch_site_events`・`fetch_replica_reports`・`fetch_follow_sets`・`fetch_follow_set_authors_referencing`）に `capped_limit(count, per)` で計算した `.limit()` を付けた。`extract_follow_set_pubkeys` は `follow_set_pubkeys_capped`（重複しない `p` を tag 順で先頭 500 件まで、超えたかどうかも返す）の薄いラッパにした。`select_latest` の隣に `cap_sites_per_author`（作者ごとに `d` でソートして先頭 N 件を残す）を追加した。
- `src/replicas.rs`: `collect_reports` の戻り値を `HashMap<SiteAddress, Vec<ReplicaReport>>` から `HashMap<SiteAddress, SiteReportSet>`（`reports` と `dropped`）に変え、`created_at` 降順（同着は reporter の hex 順）でソートしてから 200 件に切り詰める。`collect` は `cap_sites_per_author` を `select_latest` の直後に呼び、`SiteReplicas` に `dropped` を追加した。CLI の `print_replicas` は `dropped > 0` のとき `… and N more report(s) not shown` を出す。
- `src/mirror.rs`: `collect_sites` の Follow Set 読み取りを `follow_set_pubkeys_capped` に変え、切り詰められたら `warn!` を出す。サイト一覧に `cap_sites_per_author` を適用する。`apply_change`（`MirrorOp::Add` のときだけ）に `ensure_within_follow_set_cap` を追加し、追加後の `p` タグ数が上限を超えるならエラーで終了して publish しない。`replica_count` の型を `replicas::SiteReportSet` に合わせた。
- `src/webring.rs`: `Crawl`/`Graph` に `over_budget` を追加。`crawl` はノード admission の直前で `MAX_CRAWL_NODES` を確認し、超える分は弾いて `over_budget` に積む（root も対象）。`render_text` は `over_budget > 0` のとき末尾に行を追加する。`collect` は `fetch_site_events` に渡すアカウント数がそもそも `graph.nodes`（≤1000）に限られる上、`cap_sites_per_author` で作者ごとの `d` も切り詰める。
- `src/agent/follow.rs`: `refresh_follow_set` が `follow_set_pubkeys_capped` を使い、切り詰められたら `warn!` を出す（agent の対象選びには影響しない範囲での通知）。
- `src/dashboard/api.rs`・`src/dashboard/dto.rs`: `/api/publish/sites` に `cap_sites_per_author` を適用。`ReplicasDto`/`SiteReplicasDto` に `dropped`、`WebringDto`/`Graph` に `over_budget` を追加して JSON に出るようにした。
- `web/webring.js`・`web/i18n.js`: ノード詳細パネルの報告者一覧に `dropped > 0` のときの注記（`reportsMoreHint`）、webring 全体表示に `over_budget > 0` のときの注記（`overBudgetHint`）を英日で追加した。
- `docs/architecture.md`（新設の「取得と表示の上限」節）・`docs/architecture/cli.md`（sites/replicas/webring/mirror add）・`docs/architecture/dashboard/http-api.md`（`/api/sites`・`/api/webring`・`/api/replicas`・`/api/publish/sites`）・`docs/architecture/dashboard/web.md`・`docs/architecture/agent.md`・`docs/protocol.md`（Follow Set の `p` 数と、サイトごとの報告数を受信側が絞ってよい旨の MAY を実装非依存の言葉で追記）・`docs/todo.md`（該当行を、今回の変更で残った部分だけに絞って書き直した）を更新した。

## 検証

- 追加した主なユニットテスト: `nostr::tests::follow_set_pubkeys_capped_dedups_repeated_p_tags` / `follow_set_pubkeys_capped_stops_at_the_budget`、`nostr::tests::cap_sites_per_author_keeps_the_first_n_by_d`、`replicas::tests::collect_reports_keeps_the_newest_and_flags_truncation_past_the_budget`、`webring::tests::crawl_stops_at_the_node_budget_and_counts_the_excess`、`webring::tests::text_reports_accounts_dropped_by_the_crawl_budget`、`mirror::tests::ensure_within_follow_set_cap_allows_exactly_the_budget` / `_rejects_growing_past_the_budget`。
- 既存テスト（`extracts_follow_set_pubkeys`・`collect_reports_keeps_the_newest_live_report_per_reporter` など）は新しい戻り値の形（`SiteReportSet.reports`）に合わせて更新した。
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` をすべて通した（ユニットテスト 286 件、無視 13 件）。relay も Kubo も使わないので統合テストへの影響は無い。

## 見送ったこと

- 上限を設定可能にすること。運用者が緩めても資源保護という目的は変わらないため、固定値にした。
- サイトごとに relay への問い合わせを分けて、1 サイトへのレプリカ報告の集中が他サイトの取得を押し出す（crowding）問題を避けること。`fetch_replica_reports` は複数サイトの座標を 1 つのフィルタにまとめたままで、`docs/architecture.md` にその制約を明記するに留めた。呼び出し元の座標数（≒ 作者数 × `MAX_SITES_PER_AUTHOR_LISTED`）を絞ることでしか被害の範囲は抑えられない。
- `/api/sites`・`/api/replicas`・`/api/webring` のサーバ側キャッシュ・同時実行数の制限。`docs/todo.md` に残した。
- レプリカ報告 1 件が持てる `cid` タグ数の上限。今回の対象は「報告の件数」であって「1 件の中身」ではないため見送り、`docs/todo.md` に残した。
- `webring::crawl` の 1000 件の内訳（`beyond` と `over_budget` のどちらでどれだけ弾かれたか）を account 単位で区別して見せること。今は総数だけを出す。
