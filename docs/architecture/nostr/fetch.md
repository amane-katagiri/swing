# 取得と表示の上限（`nostr::budget`, `nostr/client.rs`, `nostr/client/fetch.rs`）

[`../nostr.md`](../nostr.md) の子ページ。サイトイベント・レプリカ報告・Follow Set を relay から読む経路は、`nostr::budget` の定数で件数を打ち切る。表示用の経路（`swing sites` / `replicas` / `webring`、ダッシュボードの `/api/sites` / `/api/replicas` / `/api/webring`）だけでなく、agent の取り込みと `/api/publish/sites` にも一部が効く。

## 1 回の REQ（`RelayClient::fetch`）

- 読み取りの relay ごとに nostr-sdk の `Relay::stream_events` で同じ REQ を出して 1 本のストリームに合わせ、30 秒（`FETCH_TIMEOUT`）で打ち切る。1 つの relay が遅いと、その取得は 30 秒まで待つ。
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

自分のレプリカ報告（`fetch_own_reports`）は、持っているサイトの数だけあり得るので、1 回の `limit` に収めずにページに分けて全部読む。

- 読み取りの relay ごとに別々にたどる（`walk_pages`）。各 relay に `limit` 500（`OWN_REPORTS_PAGE`）の REQ を出し、返ったうちの最も古い `created_at` を次の REQ の `until` にする。relay が自分の上限で `limit` より少なく返すことがあるので、新しい `id` が 1 件も増えなかったら、その relay は読み終えたものとする。
- 1 ページは「1 回の REQ」と同じく 30 秒で打ち切り、答えなかったらその relay はそこまでの分で止める。全体の期限は 120 秒（`FETCH_DEADLINE`）。
- relay 1 台あたりの件数と JSON の合計は、`MAX_FETCH_TOTAL_EVENTS`・`MAX_FETCH_TOTAL_BYTES` を relay の数で割った値まで。超えたら warn を出してそこまでの分を返す。
- 最初のページにどの relay も答えなければ `no relay answered` のエラー。各 relay の結果は `id` で重複を除いて合わせる。

## 定数

| 定数 | 値 | 適用箇所 |
|---|---|---|
| `MAX_FOLLOW_SET_ENTRIES` | 500 | `nostr::follow_set_pubkeys_capped`。tag 順で最初の 500 件の重複しない `p` を残す。自分の Follow Set も対象で、`agent::follow::resubscribe_and_backfill` と `mirror::collect_sites` は切り詰めたら warn を 1 回出す。`swing mirror add` は上限を超える追加をエラーにし、切り詰めない（[`../dashboard/http-api/nostr.md`](../dashboard/http-api/nostr.md#post-apimirroradd-post-apimirrorremove)） |
| `MAX_SITES_PER_AUTHOR_LISTED` | 50 | `nostr::cap_sites_per_author`（`select_latest` の直後に呼ぶ）。作者ごとに `d` の昇順で先頭 50 件だけを残す。`mirror::collect_sites`・`replicas::collect`・`webring::collect`・`/api/publish/sites` で使う。agent の取り込みの件数制限（`max_sites_per_account`。[`../agent.md`](../agent.md#全体の流れ)）とは別に効く |
| `MAX_REPORTS_PER_SITE` | 200 | `replicas::collect_reports`。数える報告を [「レプリカ報告の信頼度」](../nostr.md#レプリカ報告の信頼度replicastier) の順に並べ替えて、サイトごとに先頭 200 件を残す。`SiteReplicas.reports` は残した件数、`SiteReplicas.dropped` は切り捨てた件数（`swing replicas`・`/api/replicas`・ダッシュボードはここから「…and N more」を出す） |
| `MAX_CRAWL_NODES` | 1000 | `webring::crawl`。`Crawl.depths` がこれを超えないように新規ノードの追加を止め、弾いた件数を `Crawl.over_budget`（テキスト出力・`/api/webring` の `over_budget`）に積む。追加しなかったノードは次のレベルの取得にも現れない |
| `MAX_REFERENCING_LISTED` | 50 | `webring::crawl`。`#p` で見つかる「起点を名指ししているだけの相手」（`Crawl.referencing`）の一覧を先頭 50 件までに切り詰める。超えた件数は `Crawl.referencing_dropped` に積む。たどり方は [`../cli/views.md#webring`](../cli/views.md#webring) |
| `MAX_RELAY_FETCH_LIMIT` | 10,000 | relay 1 台への 1 回の REQ に付ける `limit` の上限（下記）。1 回の REQ で全 relay から受け取って残す件数の上限も同じ値 |
| `COORDINATES_PER_FILTER` | 250 | `fetch_replica_reports`・`fetch_replica_reports_by` が 1 つのフィルタの `#a` に入れる座標の数。超える分は組に分けて別の REQ にする |
| `AUTHORS_PER_FILTER` / `AUTHORS_PER_SPLIT_REQ` | 50 / 10 | 作者を並べる取得の組の大きさ（下記） |
| `MAX_FETCH_TOTAL_EVENTS` / `MAX_FETCH_TOTAL_BYTES` / `FETCH_CONCURRENCY` | 50,000 / 64 MiB / 4 | 複数の REQ に分ける取得の合計の上限と並行数（上記） |

`nostr::budget` の残りの定数（`MAX_CONTENT_BYTES`・`MAX_TRUSTED_REPORTERS`・イベントとメッセージの大きさ）は [`../nostr.md`](../nostr.md#検証)。

## `limit` と組の大きさ

- relay への `Filter::limit` は `capped_limit(count, per)`（`nostr/client/fetch.rs`。`min(count * per, MAX_RELAY_FETCH_LIMIT)`）で、取得先の件数（作者数・サイト数など）に経路ごとの倍率を掛けて決める。`limit` は relay ごとに付くので、合計の取得件数は relay 数倍になり得る。
- 作者（または `#p` の相手）を並べる取得は、組に分けて別々の REQ にする。
  - `fetch_follow_sets`・`fetch_follow_set_authors_referencing`: `AUTHORS_PER_FILTER`（50）人ずつ 1 つのフィルタにまとめ、`limit` もその組の人数から決める（`RelayClient::fetch_by_authors`）。
  - `fetch_replica_reports_by`: 報告者 `AUTHORS_PER_FILTER`（50）人 × 座標 `COORDINATES_PER_FILTER`（250）件の組ごとに REQ を 1 つ作り、`limit` は組の人数 × 座標数 × 2。
  - `fetch_site_events`・`fetch_reports_about`: `AUTHORS_PER_SPLIT_REQ`（10）人ずつ 1 つの REQ にまとめ、その中で作者ごとに別のフィルタを置き、`limit` も作者ごとに付ける（`RelayClient::fetch_per_author`。サイトイベントは `MAX_SITES_PER_AUTHOR_LISTED`、報告はその 2 倍）。
- `fetch_replica_reports` は複数サイトの座標（`COORDINATES_PER_FILTER` 件まで）を 1 つのフィルタにまとめるので、1 サイトの報告が多いと同じ組の他のサイトの報告が押し出されることがある（ダッシュボードは `key`/`root` を 1 リクエストあたり 100 件までに絞る）。
