# 2026-09-23 レプリカ報告者と webring の信頼度分け

## きっかけ

レプリカ報告（kind 35981）も `#p` で見つかる Follow Set も、誰でも捨て鍵で出せる。[前回](2026-09-23-fetch-and-display-budgets.md)は件数の上限だけを入れたが、件数を絞っても「捨て鍵を 200 個作って `replicas` を水増しする」「自分の Follow Set に他人を勝手に入れて `#p` 経由で webring に紛れ込む」といった、数を絞るだけでは防げない自己申告の悪用が残っていた。プロトコル上、対価なしに偽造できない手がかりが 2 つある。作者が自分でその報告者を Follow Set に入れているか、そしてオペレータ（今動いている relay 接続の持ち主）自身がその相手を Follow Set に入れているかである。どちらも「相手が名乗る」のではなく「こちらが選ぶ」行為なので、報告者や `#p` の自己申告より重い。

## 決めたこと

| 決定 | 理由 |
|---|---|
| ブロックはせず、`Tier`（`Author` / `Chosen` / `Other`）の 3 段階で表示・並び替え・件数の内訳だけを変える | 誰かを完全に無視すると、その人が実は正しい報告をしていた場合に検知できなくなる。信頼度を分けて見せることで、判断は利用者に委ねる |
| `Chosen` の基準は「報告者が作者の Follow Set に入っている」OR「報告者がオペレータ自身の Follow Set に入っている」。報告者自身の Follow Set にその作者が入っているか（＝「フォローされている」への自己申告）は見ない | 報告者自身の Follow Set は報告者が自分で書けるので、対価なしに偽造できる。作者の Follow Set とオペレータの Follow Set はどちらも「こちらが選んだ」もので、報告者には書き換えられない |
| 報告者ごとの Follow Set の個別取得（以前の `replicas::collect` にあった、報告者数ぶんの `fetch_follow_sets` 呼び出し）を廃止し、作者ごとの Follow Set をまとめて 1 回、オペレータ自身の Follow Set を 1 回取る `replicas::Chosen` に置き換えた | 報告者数は件数上限を入れても最大でサイト数 × 200 まで増え得り、そのたびに Follow Set を引くのは無制限に近いファンアウトだった。作者数・対象数は呼び出し元の入力（ダッシュボードなら 100 件、Follow Set なら 500 件）で最初から抑えられている |
| `mirror::collect_sites` は、既に読んでいる自分の Follow Set（`targets`）をそのままオペレータの Chosen 集合として使い、relay へは取りに行かない | 二重に同じ情報を取りに行く必要が無い |
| `MAX_REPORTS_PER_SITE` での切り詰めは、`created_at` の新しい順ではなく `(tier, created_at 降順)` の順に変えた | 単純な新しさ順だと、捨て鍵を大量に「今」出せば `Other` 側の報告で `Author`/`Chosen` の古い報告を押し出せてしまう。tier を先に見ることで、信頼できる報告者は切り詰めの影響を受けにくくする |
| レプリカ数は `replicas`（`Author`+`Chosen` の合計）と `unverified`（`Other`）に分けて返し、表示は `3` か `3 (+12 unverified)`（0 件なら括弧を出さない） | 「怪しい報告も含めた総数」と「信頼できる報告だけの数」を混ぜて 1 つの数字にすると、水増しに気づけない |
| webring の `#p` 検索（`referencing`）は深さ 0（起点）についてのみ 1 回取得し、クロールを広げるのにも Mutual/One-way の辺にも使わない。結果は `Crawl.referencing` として別枠にし、`MAX_REFERENCING_LISTED`（50 件）で切り詰めて残りの件数を持つ | `#p` は「相手が自分を名指ししている」だけで、こちらがフォローし返しているとは限らない自己申告。今までは `#p` で見つけたアカウントもクロールに加えて先へ広げていたので、誰かが大量の捨て鍵から自分を名指しするだけで他人の webring に登場したり、深さを稼いだりできた |
| `beyond`（深さの上限外で表示していないアカウント数）の意味は変えない | `beyond` は admitted ノードの outbound な Follow Set だけを見ており、今回の `referencing` の扱いの変更とは独立している。確認した上でそのままにした |

## やったこと

- `src/replicas.rs`: `Tier`（`Author`/`Chosen`/`Other`、`Ord` 導出で切り詰めの優先順を兼ねる）、`Chosen`（作者ごとの Follow Set とオペレータの Follow Set を持つ）、`fetch_chosen` / `fetch_chosen_with_own`、`tier_of`、`ReplicaCounts`（`trusted`/`unverified`）、`count_replicas`、`format_replica_counts` を追加。`Replica`・`Reporter` は `is_author`/`following` を `tier: Tier` に統合。`collect_reports` は `chosen` を受け取って切り詰め前の並べ替えに使う。`replicas_of` は作者と `chosen` を受け取って `tier` を埋める。`collect`（`swing replicas` / `/api/replicas`）から、報告者ごとの Follow Set 個別取得を削除し `fetch_chosen` に置き換えた。CLI 出力は `tier_mark`（`[author]`/`[chosen]`/`[unverified]`、常に表示。以前の `follow_mark` は `[not following]` の場合だけ表示し `following` な場合は何も出さなかった）に変えた。
- `src/mirror.rs`: `collect_sites` は `targets` をオペレータの Chosen 集合として使う `fetch_tiered_reports`（`replicas::fetch_chosen_with_own` → `replicas::fetch_for_sites`）を呼ぶ。`SiteRow.replicas` の型を `Option<usize>` から `Option<replicas::ReplicaCounts>` に変え、`format_site_line` は `replicas::format_replica_counts` で表示する。`swing mirror add` の上限判定（`ensure_within_follow_set_cap`）はそのまま。
- `src/webring.rs`: `Crawl` に `referencing: Vec<PublicKey>` と `referencing_dropped: usize` を追加。`crawl` は `depth == 0` のときだけ `source.referencing` を呼び、その結果を候補ノードの admission には使わない（outbound な `follow_sets` だけで `candidates` を作る）。ループを抜けた後、admitted ノードを除いた `referencing` を `MAX_REFERENCING_LISTED` 件に切り詰め、残りを `referencing_dropped` に積む。`render_text` は `Referencing the root (unverified)` の節を追加（`referencing`/`referencing_dropped` を引数に取る）。`WebringView`/`collect` はこれらを引き回す。
- `src/dashboard/dto.rs`: `ReporterDto` を `tier: String`（`"author"`/`"chosen"`/`"other"`）に、`SiteReplicasDto` に `unverified`、`SiteDto` に `unverified_replicas` を追加。`WebringDto` に `referencing: ReferencingDto { accounts, more }` を追加。
- `web/sites.js`: サイト一覧・テーブルのレプリカ数を `3 (+12)` 形式（`unverified_replicas` が 0 なら括弧なし）にする `replicaCountText` を追加。
- `web/webring.js`: ノード詳細パネルの報告者一覧を tier ベースのタグ（`tagAuthor`/`tagChosen`/`tagUnverified`）に、サイトの件数バッジを `replicaCountBadge`/`replicaCountWithUnverified` に変更。`list` 表示に `Referencing`（未検証）グループを追加。`beyond`/`over_budget` の下に置いていた `[not following]` 系の表示は撤去。
- `web/i18n.js`: 上記の新しいキー（`replicaCountBadge`・`replicaCountWithUnverified`・`tagChosen`・`tagUnverified`・`referencingHeading`・`referencingMoreHint`）を英日で追加し、使わなくなった `replicasReportsBadge`・`tagNotFollowing` を削除した。
- ドキュメント: `docs/architecture.md`（新設の「レプリカ報告の信頼度」節、上限表に `MAX_REFERENCING_LISTED`）、`docs/architecture/cli.md`（`replicas`・`sites`・`webring` の tier・件数表記・Referencing 節の説明）、`docs/architecture/dashboard/http-api.md`（`/api/sites`・`/api/replicas`・`/api/webring` のフィールド）、`docs/architecture/dashboard/web.md`、`docs/protocol.md`（第 7 節に Follow Set は自己申告で片方向だけでは関係を示さない旨の SHOULD、第 8 節に受信側が Chosen な報告者だけを数えてよい旨の MAY。どちらも数値は書かない）、`README.md`（`swing replicas`/`swing webring` の出力例と説明）、`docs/todo.md`（「レプリカ報告者の数に上限を付ける」の行を削除し、ブロックリストを新しい行として積んだ）を更新した。

## 検証

- 追加した主なユニットテスト: `replicas::tests::tier_of_classifies_author_chosen_and_other`、`tier_mark_labels_each_tier`、`format_replica_counts_omits_the_parenthesis_when_there_is_nothing_unverified`、`collect_reports_keeps_trusted_reporters_before_older_others_when_truncating`（tier を先に見る切り詰め順の検証）、`webring::tests::crawl_expands_only_along_outbound_edges_past_the_roots`（`#p` だけで見つかる相手はクロールに加わらず `referencing` に入ることの検証）、`crawl_at_depth_zero_still_queries_referencing_for_the_roots`、`crawl_caps_referencing_accounts_and_counts_the_rest`、`text_lists_referencing_accounts_and_the_remainder`。
- 既存テスト（`replicas_are_counted_by_the_latest_cid`・`collect_reports_keeps_the_newest_live_report_per_reporter`・`text_lists_accounts_and_links_by_depth` など）は新しい引数・戻り値の形に合わせて更新した。
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` をすべて通した（ユニットテスト 291 件、無視 13 件）。relay も Kubo も使わないので統合テストへの影響は無い。
- `web/i18n.js` は en/ja のキー集合が一致すること、`web/sites.js`・`web/webring.js` が参照する `t()` キーがすべて定義されていることを Node でスクリプト的に確認した。

## 見送ったこと

- 報告者の NIP-05 検証を tier の 1 つにすること。報告者ごとに HTTP リクエストが要り、件数上限を入れてもなお重い。
- 「作者から見た webring 上の到達可能性」をレプリカの tier に使うこと（例えば `Chosen` を「作者の Follow Set」だけでなく「作者から N ホップ以内」まで広げる）。問い合わせのたびにクロールが要り、コストと実装量に見合わない。
- 報告者や Follow Set 由来のアカウントを個別に締め出すブロックリスト。今回は信頼度の提示だけに留め、`docs/todo.md` に別機能として積んだ。
- レプリカ報告の実体確認（`routing/findprovs` で実際に提供しているか確かめる）。既存の `docs/todo.md` の項目のまま。
