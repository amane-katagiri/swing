# webring のたどり方とグラフ（`src/webring/mod.rs`）

CLI の `swing webring`（[`cli/views.md#webring`](cli/views.md#webring)）とダッシュボードの `GET /api/webring`（[`dashboard/http-api/nostr.md`](dashboard/http-api/nostr.md#get-apiwebringrootkeydepthn)）が共有する集計（`webring::collect`）。出力の書式（`text`・`dot`・`mermaid` は `webring/render.rs`）と JSON はそれぞれのページに置く。state は読まない。

起点と深さの上限は呼び出し元が決める（既定は自分の pubkey と深さ 2。API だけ深さ 4 までに制限する）。

## たどり方（`webring::crawl`）

- 起点を深さ 0 とし、深さ `d` のアカウントの Follow Set（kind 30000、`d = [nostr].mirror_set`）を 1 段ずつまとめて取得する。各 Follow Set の `p` は重複を除いてタグ順に `MAX_FOLLOW_SET_ENTRIES` 件まで使う。
- `d` が上限より小さければ、その `p` のうちまだ見ていないアカウントを深さ `d + 1` にする。深さが上限のアカウントも Follow Set は取得するが、先へは広げない。
- たどるアカウントの総数は起点を含めて `MAX_CRAWL_NODES`（[取得と表示の上限](nostr/fetch.md#定数)）まで。超えて見つかったアカウントは加えず、件数を `over_budget` に数える。
- 起点を名指ししているだけの相手（`referencing`）: 深さ 0 のときだけ、起点を `#p` に入れて Follow Set の作者を 1 回取得する。名指しは起点が相手を選んだ証拠にならないので、クロールを広げるのには使わない。クロールに加えたアカウントを除き、hex の昇順で先頭 `MAX_REFERENCING_LISTED` 件までにし、落とした件数を `referencing_dropped` に数える。
- Follow Set・サイトイベント・`referencing` のどれかの取得に失敗したら全体がエラーになる。

## グラフ（`webring::build_graph`）

- 取得した Follow Set の `p` のうち、クロールに加えたアカウントを指すものを辺（A → B は A の Follow Set に B がいる）にする。自分自身への辺は捨てる。
- 辺を向きを無視してたどり、起点につながらないアカウントは除く。残ったアカウントがノード、その間の辺がグラフの辺になる。
- Follow Set を取得できなかったノードは「Follow Set 無し」として区別する。
- `beyond`: ノードの Follow Set に載っているのにクロールに加えなかったアカウントの数（重複は 1 回）。深さの上限の外にいるものと `MAX_CRAWL_NODES` で弾いたものの両方を数えるので、`over_budget` と重なることがある。
- 双方向の組（A → B と B → A の両方がある）と片方向の辺は `webring::split_links` で分ける。

## ノードの名前

ノードのアカウントのサイトイベントを取得し、サイトごとの最新（`nostr::select_latest`）の `d` を、1 アカウントあたり `d` の昇順で先頭 `MAX_SITES_PER_AUTHOR_LISTED` 件まで名前にする（`webring::site_name_lists`）。表示名はそれを `, ` でつないだもの。サイトイベントの無いアカウントは名前を持たない（表示側で縮めた npub（`webring::short_npub`。先頭 12 文字と末尾 6 文字）を使う）。
