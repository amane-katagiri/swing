# 取得と表示の上限（`nostr::budget`, `nostr/client.rs`, `nostr/client/fetch.rs`, `nostr/client/resend.rs`）

[`../nostr.md`](../nostr.md) の子ページ。サイトイベント・レプリカ報告・Follow Set・サイトイベントの削除要求を relay から読む経路は、`nostr::budget` の定数で件数を打ち切る。表示用の経路（`swing sites` / `replicas` / `webring`、ダッシュボードの `/api/sites` / `/api/replicas` / `/api/webring`）だけでなく、agent の取り込みと `/api/publish/sites` にも一部が効く。

## 1 回の REQ（`RelayClient::fetch`）

- 読み取りの relay ごとに nostr-sdk の `Relay::stream_events` で同じ REQ を出して 1 本のストリームに合わせ、30 秒（`FETCH_TIMEOUT`）で打ち切る。
- relay ごとの結果:
  - 答えた: ストリームが EOSE（または接頭辞の無い CLOSED）で終わり、そのとき relay がまだ接続中。
  - 答えていない: 購読できない、接頭辞付きの CLOSED、認証失敗、途中で接続が切れた、30 秒以内に終わらない。
- 答えた relay が 1 つも無ければ取得全体を `no relay answered` のエラーにする（relay が 1 つも無い場合も同じ）。1 つでも答えていれば、他の relay の失敗は debug ログだけにして結果を返す。
- 受け取りながら重複を除き、`created_at` が未来ずれの許容（[`../nostr.md`](../nostr.md#未来ずれの許容nostrmax_future_skew)）を超えるものは捨てる。全 relay の合計が `MAX_RELAY_FETCH_LIMIT` 件か、イベントの JSON の合計が `MAX_FETCH_TOTAL_BYTES / FETCH_CONCURRENCY`（16 MiB）を超えたら、`created_at` の古いものから捨てて新しい方だけを返す。
- イベントの JSON の大きさ（`event_bytes`）は、初めて受け取ったときに 1 回だけ測ってイベントと一緒に持ち（`Measured`）、捨てるときと複数の REQ の合計（下記）にもその値を使う。同じ `id` の 2 通目以降は測らずに捨てる。

## 複数の REQ に分ける取得（`RelayClient::fetch_all`）

- REQ を `FETCH_CONCURRENCY`（4）本ずつ並行に出し、結果を合わせる。
- 合わせた件数が `MAX_FETCH_TOTAL_EVENTS`（50,000）件、JSON の合計が `MAX_FETCH_TOTAL_BYTES`（64 MiB）に達したら、そこで打ち切って warn を出し、それまでの分を返す。
- 全体の期限は 120 秒（`FETCH_DEADLINE`）。過ぎたら warn を出してそれまでに届いた分を返す。
- どの relay も答えなかった REQ（`no relay answered`）があっても、他の REQ の結果は捨てずに返し、失敗した本数を warn に出す。すべての REQ が失敗したときだけ全体をエラーにする（REQ が 1 本も無ければ空の結果）。

## ページに分ける取得（`RelayClient::fetch_pages`）

自分のレプリカ報告（`fetch_own_reports`）は、1 回の `limit` に収めずにページに分けて全部読む。

- 読み取りの relay ごとに別々にたどる（`walk_pages`）。各 relay に `limit` 500（`OWN_REPORTS_PAGE`）の REQ を出し、返ったうちの最も古い `created_at` を次の REQ の `until` にする。`until` は境界の秒を含むので、新しい `id` が 1 件も増えなかったページは次のように扱う。
  - そのページが満杯でなければ、その relay は読み終えたものとする。満杯とは、件数が `limit` とその relay がそれまでに返した最も多い件数の小さい方以上であること（relay が自分の上限で `limit` より少なく返すことがあるため）。
  - 満杯なら、同じ秒に 1 ページより多くのイベントがあるので、`until` をその秒の 1 秒前にして続ける（debug を出す）。その秒のページに入らなかった分は取れない。
- 1 ページは「1 回の REQ」と同じく 30 秒で打ち切り、答えなかったらその relay はそこまでの分で止める。全体の期限は 120 秒（`FETCH_DEADLINE`）。
- relay 1 台あたりの件数と JSON の合計は、`MAX_FETCH_TOTAL_EVENTS`・`MAX_FETCH_TOTAL_BYTES` を relay の数で割った値まで。超えたら warn を出してそこまでの分を返す。
- 最初のページにどの relay も答えなければ `no relay answered` のエラー。各 relay の結果は `id` で重複を除いて合わせる。
- 結果（`Paged`）の `complete` は、最後まで（終わりのページか件数・大きさの上限まで）たどれた relay が 1 つでもあれば true。期限切れか途中のページに答えなかった relay だけなら false。

## relay ごとの取得

agent の保存した版のサイトイベントの再送（[`../agent/resend.md`](../agent/resend.md)）が使う `fetch_sites_per_relay`・`fetch_site_deletions` は、relay の答えを合わせずに、relay ごと・REQ ごとに「答えたか」を持って返す。共通の部分は `fold_per_relay`。

- 読み取りの relay ごとに、REQ を `FETCH_CONCURRENCY`（4）本ずつ並行に出す。relay どうしは並行に進めるので、同時に開いている REQ は最大で relay の数 × 4 本。1 本は「1 回の REQ」と同じく 30 秒（`FETCH_TIMEOUT`）で打ち切る。全体の期限は全 relay で共通の 120 秒（`FETCH_DEADLINE`）で、過ぎてから始まる REQ は出さない。
- 受け取ったイベントはその場で畳み込み、生のイベントの列は持たず、REQ ごとに畳み込んだ結果だけを持つ。例外として `fetch_site_deletions` は、同じ削除要求を二度数えないように REQ ごとに受け取った削除要求の `id` の集合を持つ（件数は下の relay 1 台あたりの上限までで頭打ち）。
- 件数と JSON の大きさは、1 本あたり `MAX_RELAY_FETCH_LIMIT` 件・16 MiB（「1 回の REQ」と同じ）、relay 1 台あたりの合計が `MAX_FETCH_TOTAL_EVENTS`・`MAX_FETCH_TOTAL_BYTES` を relay の数で割った値（「ページに分ける取得」と同じ）まで。超えたら warn を出し、その REQ は「答えていない」とする。
- REQ が「答えた」のは、EOSE（または接頭辞の無い CLOSED）まで受け取り、そのとき relay がまだ接続中だったときだけ（「1 回の REQ」と同じ）。期限切れ・上限超え・途中で切れた REQ は「答えていない」。
- エラーにはならない。どの relay も答えなければ、すべての対象がすべての relay で「答えていない」になる。

`fetch_sites_per_relay` は、対象のサイトを `AUTHORS_PER_SPLIT_REQ`（10）件ずつ 1 つの REQ にまとめ、その中でサイトごとに別のフィルタ（kind・作者・`d`、`limit` 2）を置く。受け取ったイベントは、[受信後の条件](../nostr.md#検証)を満たすものだけを NIP-01 の置き換え規則でサイトごとに最新の 1 件へ畳み込む（`nostr::keep_newest_site`）。結果（`RelayAnswers`）は relay ごと・サイトごとに `SiteAnswer` の次のどれか。

- `Unanswered`: 答えていない。
- `Unfinished`: 答えていない（REQ が期限切れ・上限超え・途中で切れたなど）が、それまでに畳み込んだ 1 件がある。条件は `Found` と同じく確かめてあるので置き換えの根拠には使えるが、その relay が保存している版を持つかどうかは分からない。
- `Missing`: 答えたが無い。
- `Found`: 答えた最新の 1 件。

`fetch_site_deletions` は、送り直す予定のサイトの作者ごとに（作者のサイトが `COORDINATES_PER_FILTER`（250）件を超えたら 250 件ずつに分けて）2 つのフィルタを作る。kind 5・その作者 1 人で、`#a` にその作者のサイトの座標を並べたものと、`#e` にその作者の保存している版の `id` を並べたもの（1 つのフィルタに入れると両方を満たすものしか返らない）。`limit` はどちらもその作者のサイトの数 × 2 と `MAX_DELETIONS_PER_FILTER`（100）の小さい方。この 2 つのフィルタの組（作者 1 人の 250 件までのサイト分。サイトの多い作者は複数の組になる）を `UNITS_PER_REQ`（`AUTHORS_PER_SPLIT_REQ` / 2 = 5）組ずつ、つまりフィルタ `AUTHORS_PER_SPLIT_REQ` 個ずつ 1 つの REQ にまとめる。フィルタと `limit` を作者ごとに分けておかないと、ある作者がほかの作者のサイトを指す削除要求を大量に出して、その作者の削除要求を押し出せる。

- 受け取った kind 5 は、`id` で重複を除き、作者がそのフィルタの作者のものだけを、要求した `e`（保存している版の `id`）と `a`（そのサイトの座標）のタグだけ索引に入れる（`nostr::Withdrawals`。`e` は作者と `id` の組の集合、`a` は作者と座標の組ごとに最大の `created_at`）。要求したタグだけを見るので、タグの多い削除要求でも手間は要求したサイトの数で決まる。`e` は版の `id` そのものを指すので、`created_at` によらず索引に入れる。`a` の `created_at` は、今の時刻 + 未来ずれの許容（`MAX_FUTURE_SKEW`）で頭打ちにして索引に入れる（先の時刻のままだと、まだ出ていない版まで取り下げに含まれる）。
- 作者ごとに、`#a` と `#e` のそれぞれに当たった削除要求（索引に入れたかによらず、要求したタグを持つもの）を数える。件数が `limit` に達したフィルタは、当たったもののうち `created_at` が新しい方から `limit` 番目の値をそのフィルタの境（cutoff）とし、debug を出す。境は答えた REQ（EOSE まで受け取ったもの）のものだけを使う。relay は `limit` のある REQ の古い方を切るので、境より新しい削除要求は見落としていないとみなす。
- サイトが relay で「答えた」のは、その作者を含む REQ に答え、`#e` のフィルタが `limit` に達しておらず、`#a` のフィルタが `limit` に達していないか保存している版の `created_at` がその境より大きいとき（`answered_past`）。`#e` が `limit` に達したら、その作者（組）のサイトはどれも「答えていない」とする（`e` の削除要求は `created_at` によらず版を取り下げるので、境からはどの版を指していたか分からない）。`#a` の境と同じ秒の版も「答えていない」とする（debug）。
- 切られたことが分かるのは、relay が返した件数がこちらの `limit` に達したときだけ。relay の `limit` の上限が `MAX_DELETIONS_PER_FILTER` より小さいと、黙って切られた古い削除要求を見落として「答えた」とすることがある（relay の上限は `MAX_DELETIONS_PER_FILTER` 以上であることを前提にしている）。
- 結果（`DeletionAnswers`）は、全 relay の全 REQ（答えていない REQ・境で切られた答えの分も含む）の索引を合わせたものと、relay ごと・サイトごとの「答えたか」。

## 定数

| 定数 | 値 | 適用箇所 |
|---|---|---|
| `MAX_FOLLOW_SET_ENTRIES` | 500 | `nostr::follow_set_pubkeys_capped`。tag 順で最初の 500 件の重複しない `p` を残す。自分の Follow Set も対象で、`agent::follow::resubscribe_and_backfill` と `mirror::collect_sites` は切り詰めたら warn を 1 回出す。`swing mirror add` は上限を超える追加をエラーにし、切り詰めない（[`../mirror.md`](../mirror.md#follow-set-の書き換えmirrorapply_add--apply_remove)） |
| `MAX_SITES_PER_AUTHOR_LISTED` | 50 | `nostr::cap_sites_per_author`（`select_latest` の直後に呼ぶ）。作者ごとに `d` の昇順で先頭 50 件だけを残す。`mirror::collect_sites`・`replicas::collect`・`webring::collect`・`/api/publish/sites` で使う。agent の取り込みの件数制限（`max_sites_per_account`。[`../agent.md`](../agent.md#全体の流れ)）とは別に効く |
| `MAX_REPORTS_PER_SITE` | 200 | `replicas::collect_reports`。数える報告を [「レプリカ報告の信頼度」](../nostr.md#レプリカ報告の信頼度replicastier) の順に並べ替えて、サイトごとに先頭 200 件を残す。`SiteReplicas.reports` は残した件数、`SiteReplicas.dropped` は切り捨てた件数（`swing replicas`・`/api/replicas`・ダッシュボードはここから「…and N more」を出す） |
| `MAX_CRAWL_NODES` | 1000 | `webring::crawl`。`Crawl.depths` がこれを超えないように新規ノードの追加を止め、弾いた件数を `Crawl.over_budget`（テキスト出力・`/api/webring` の `over_budget`）に積む。追加しなかったノードは次のレベルの取得にも現れない |
| `MAX_REFERENCING_LISTED` | 50 | `webring::crawl`。`#p` で見つかる「起点を名指ししているだけの相手」（`Crawl.referencing`）の一覧を先頭 50 件までに切り詰める。超えた件数は `Crawl.referencing_dropped` に積む。たどり方は [`../webring.md`](../webring.md#たどり方webringcrawl) |
| `MAX_RELAY_FETCH_LIMIT` | 10,000 | relay 1 台への 1 回の REQ に付ける `limit` の上限（下記）。1 回の REQ で全 relay から受け取って残す件数の上限も同じ値 |
| `MAX_DELETIONS_PER_FILTER` | 100 | `fetch_site_deletions` の 1 つのフィルタの `limit` の上限（[上記](#relay-ごとの取得)） |
| `COORDINATES_PER_FILTER` | 250 | `fetch_replica_reports`・`fetch_replica_reports_by` が 1 つのフィルタの `#a` に入れる座標の数。超える分は組に分けて別の REQ にする。`fetch_site_deletions` が 1 人の作者のフィルタに入れるサイトの数にも使う |
| `AUTHORS_PER_FILTER` / `AUTHORS_PER_SPLIT_REQ` | 50 / 10 | 作者を並べる取得の組の大きさ（下記）。`AUTHORS_PER_SPLIT_REQ` は `fetch_sites_per_relay` が 1 つの REQ にまとめるサイトの数と、`fetch_site_deletions` が 1 つの REQ に置くフィルタの数にも使う |
| `MAX_FETCH_TOTAL_EVENTS` / `MAX_FETCH_TOTAL_BYTES` / `FETCH_CONCURRENCY` | 50,000 / 64 MiB / 4 | 複数の REQ に分ける取得の合計の上限と並行数（上記） |

`nostr::budget` の残りの定数（`MAX_CONTENT_BYTES`・`MAX_TRUSTED_REPORTERS`・イベントとメッセージの大きさ）は [`../nostr.md`](../nostr.md#検証)。

## `limit` と組の大きさ

- relay への `Filter::limit` は `capped_limit(count, per)`（`nostr/client/fetch.rs`。`min(count * per, MAX_RELAY_FETCH_LIMIT)`）で、取得先の件数（作者数・サイト数など）に経路ごとの倍率を掛けて決める。`limit` は relay ごとに付くので、合計の取得件数は relay 数倍になり得る。
- 作者（または `#p` の相手）を並べる取得は、組に分けて別々の REQ にする。
  - `fetch_follow_sets`・`fetch_follow_set_authors_referencing`: `AUTHORS_PER_FILTER`（50）人ずつ 1 つのフィルタにまとめ、`limit` もその組の人数から決める（`RelayClient::fetch_by_authors`）。
  - `fetch_replica_reports_by`: 報告者 `AUTHORS_PER_FILTER`（50）人 × 座標 `COORDINATES_PER_FILTER`（250）件の組ごとに REQ を 1 つ作り、`limit` は組の人数 × 座標数 × 2。
  - `fetch_site_events`・`fetch_reports_about`: `AUTHORS_PER_SPLIT_REQ`（10）人ずつ 1 つの REQ にまとめ、その中で作者ごとに別のフィルタを置き、`limit` も作者ごとに付ける（`RelayClient::fetch_per_author`。サイトイベントは `MAX_SITES_PER_AUTHOR_LISTED`、報告はその 2 倍）。
- `fetch_replica_reports` は複数サイトの座標（`COORDINATES_PER_FILTER` 件まで）を 1 つのフィルタにまとめるので、1 サイトの報告が多いと同じ組の他のサイトの報告が押し出されることがある（ダッシュボードは `key`/`root` を 1 リクエストあたり 100 件までに絞る）。
