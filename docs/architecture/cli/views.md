# 表示と Follow Set の操作（`src/mirror/`, `src/replicas.rs`, `src/health.rs`, `src/stats.rs`, `src/webring/`）

[`../cli.md`](../cli.md) の子ページ。`mirror list` / `add` / `remove`・`sites`・`replicas`・`status`・`stats`・`webring`。relay・Kubo へ直接つなぐか API を経由するかは [`../cli.md#共通`](../cli.md#共通)。

## mirror list / add / remove

自分の Follow Set を操作する。

- `add` / `remove` が叩くのは `POST /api/mirror/add` / `/api/mirror/remove`（body は `{"keys": [...]}`）。
- `add` / `remove` は `p` 以外のタグの値と `content`（NIP-51 の暗号化 private 部分）を保持して再署名する。タグは `d` → その他 → `p` の順に並べ直す。
- Follow Set が無い状態の `add` は `["title", "SWING mirror list"]` 付きで新規作成する。Follow Set が無い状態の `remove` は `(no follow set found); no changes` とだけ表示して終わる。
- 追加済みの `add`、未登録の `remove` は no-op と報告し、変更が無ければ publish しない。
- 再署名する Follow Set の `created_at` は現在時刻と「元の Follow Set の `created_at` + 1」の大きい方（`mirror::set::next_created_at`）。
- `add` は結果の `p` タグ数が `MAX_FOLLOW_SET_ENTRIES` を超えるならエラーで終了し、publish しない（エラーの内容は [`../dashboard/http-api/nostr.md`](../dashboard/http-api/nostr.md#post-apimirroradd-post-apimirrorremove)）。
- `list` は npub と hex を併記する。`title` タグは `sites` の `title:` 行と同じ無害化（下記）をして `Title:` に表示する。
- relay の Follow Set と `state.json` の `follow_set` を比べて新しい方を使う（検証条件は [agent の Follow Set の選び方](../agent.md#follow-set-の選び方) と同じ）。保存済みの方を使ったときは `(relays returned an older follow set; ...)` か `(follow set not found on relays; ...)` を表示する。state.json は読むだけ。`sites` も同じ。
- どの relay も答えなかった（[`nostr/fetch.md`](../nostr/fetch.md#1-回の-reqrelayclientfetch)）ときは「見つからない」とは扱わず、`list` も `add` / `remove` もエラーで終了する（`add` / `remove` は API の 502 の `error` を表示する）。

## sites

- Follow Set の対象者ごとに、サイトごとの最新のサイトイベントを 1 行（`d`、`cid`、`url`、`size`、`created_at`、NIP-05 検証結果、`replicas`、`[stored]` / `[not stored]`）表示する。検証結果と保存状況は `state.json` から読む。
  - `size` 列: 保存済みの版の実測値（`/api/sites` の `stored_size` と同じ値。[`dashboard/http-api/status.md#get-apisites`](../dashboard/http-api/status.md#get-apisites)）があれば数値で、無ければイベントの自己申告の `size` タグを括弧書き（例 `(12345)`）で、どちらも無ければ `-` を出す。
  - `replicas` 列: [replicas](#replicas) と同じ集計の最新版のレプリカ数を `3` または `3 (+12 unverified)`（未検証の報告者がいるとき）の形で出す（[「レプリカ報告の信頼度」](../nostr.md#レプリカ報告の信頼度replicastier)）。信頼度の判定には Follow Set の対象全員分をまとめて 1 回だけ取得する。レプリカ報告の取得に失敗したら `(fetching replica reports failed: ...)` を表示して `-` にする。
- `title` タグが有効なら次の行に `    title: `、`content` が空でなければ続けて `    message: ` を出す。どちらも制御文字を空白に置き換え、見えない書式文字（[`nostr::is_unsafe_char`](../nostr.md#検証)）を取り除き、200 文字を超える分を `…` に置き換えてから前後の空白を削る（`format::sanitize_display_text`）。
- 続けて、state に版があるのに Follow Set にいない pubkey を `Unfollowed but still stored` 見出しの下に `[unfollowed]` 付きで、サイトごとに state の最新版を 1 行（`url` は `-`）表示する。Follow Set が見つからなくても表示する。見出しには `remove_on_unfollow` と Follow Set が見つかったかどうかに応じて、次の poll で消えるか・残すか・Follow Set が見つかるまで消さないかを添える（いつ消えるかは [agent の unfollow](../agent.md#unfollow)）。
- 表示件数は [取得と表示の上限](../nostr/fetch.md#定数)（`MAX_FOLLOW_SET_ENTRIES`・`MAX_SITES_PER_AUTHOR_LISTED`）で打ち切る。

## replicas

state は読まない。

- `<key>` を作者として扱う。省略時は自分の pubkey。
- 作者ごとに、サイトごとの最新のサイトイベントについて `d`、`cid`、`replicas=<数> (reports=<有効な報告の数>)` を表示し（`<数>` は `sites` の `replicas` 列と同じ形）、続けて報告者ごとに npub と `[latest]` / `[older version]` を 1 行ずつ表示する。
- 報告者ごとに信頼度の tier を `[author]` / `[chosen]` / `[unverified]` で添える。tier の定義・数える報告の条件・並びは [「レプリカ報告の信頼度」](../nostr.md#レプリカ報告の信頼度replicastier)。
- サイトごとの報告は `MAX_REPORTS_PER_SITE`（[取得と表示の上限](../nostr/fetch.md#定数)）で切り詰める。`reports=` は切り詰め後の件数で、切り詰めがあれば報告者一覧の後に `… and N more report(s) not shown` を出す。
- サイトイベント・報告・Follow Set のどれかの取得に失敗したらエラーで終了する。

## status

`GET /api/status` を叩き（[共通](../cli.md#共通)）、API 側が `state.json` と Kubo だけを見て組み立てた結果を印字する。agent が未準備なら失敗する（[`dashboard/http-api.md#共通`](../dashboard/http-api.md#共通)）。

- `state.json` の版ごとに、版のパス・`cid`・`size`（state に記録された版ごとのサイズ）と判定を 1 行表示する。判定は [起動時の突き合わせ](../agent.md#起動時の突き合わせ) と同じ検査で、`[ok]` / `[missing]`（パスが無い）/ `[cid mismatch]` / `[incomplete]`（Kubo がブロックを手元に見つけられないと答えた）/ `[check failed]`（`files/stat` か `dag/stat` がそれ以外の理由で失敗。版は壊れたものとして扱わない）のいずれか。`ok` 以外は理由を添える。state のキーが `<pubkey hex>:<d>` として読めない版は検査せず、キーと `cid` に `[invalid site key]` を付けて出す（API の `health` は `invalid_key`）。
- 続けて `Actual size` 見出しの下に、サイトごとの実容量（そのサイトの全版をまとめた `dag/stat` の `TotalSize`。版どうしで共有しているブロックは 1 回だけ数える）と合計を表示する。測れなかったサイトは `unknown` にし、合計も `unknown` にする。
- 続けて `Not in state` 見出しの下に、[sweep](../agent.md#sweep) が消す MFS のパスを表示する。ディレクトリごと消えるものはそのディレクトリだけを出す。一覧に失敗したディレクトリは `[list failed]: <理由>` 付きで出す。
- `ok` 以外の版と `Not in state` の項目が 1 つでもあれば、件数を表示して 0 以外で終了する。
- agent の実行中は、保存途中の版（MFS に置いた後、state を保存する前）が `Not in state` に出ることがある。
- 実行時間は保存量に比例する（[起動時の突き合わせ](../agent.md#起動時の突き合わせ)）。ダッシュボードのリクエストタイムアウト（[`dashboard.md#タイムアウトsrcdashboardmodrs`](../dashboard.md#タイムアウトsrcdashboardmodrs)）と `ApiClient` 側のタイムアウト（125 秒）の範囲で待つ。

## stats

```
swing stats [--last <duration>, 既定 1h] [--json] [--config <path>]
```

`swing up` が測って持っているリソース使用量（[`stats.md`](../stats.md)）を `GET /api/stats?since=<今 − last>` で取り（[共通](../cli.md#共通)）、表にする。`--last` は `30m`・`6h`・`1d` のような長さ（単位なしは秒）。`swing up` が持つ期間は [`stats.md#測り方`](../stats.md#測り方)。セットアップモードでも使える。

- 1 行目にサンプル数・期間・間隔・最新のサンプルが何秒前か。サンプルが無ければ `No samples in the last <期間>; swing up takes one every <測る間隔>.` だけを出す（間隔は [`stats.md#測り方`](../stats.md#測り方)）。
- 表は `swing CPU`・`swing memory`・`Kubo CPU`・`Kubo memory`・`IPFS in`・`IPFS out` の 6 行で、列は `now`（最新のサンプルの値）・`avg`・`max`（期間内の値のあるサンプルでの平均と最大）。値が無ければ `-`。バイト数は 1024 基数で小数 1 桁に丸める。
- 表の下に、最新のサンプルに通信量があれば Kubo 起動からの累計（`IPFS total since Kubo started: in …, out …`）、`kubo_managed` が `false` なら Kubo が SWING の管理外で CPU・メモリを取得できない旨を出す。
- `--json` を付けると API の応答（[`dashboard/http-api/status.md#get-apistats`](../dashboard/http-api/status.md#get-apistats)）をそのまま整形して出す。

## webring

state は読まない。

- `<key>` を起点にする。複数指定でき、すべて深さ 0 の起点になる。省略時は自分の pubkey。`--depth` の既定は 2、`--format` の既定は `text`。
- たどり方（`webring::crawl`）: 起点を深さ 0 とし、深さ `d` のアカウントについて Follow Set を取得する。`d < depth` なら、その `p` のアカウントを、まだ見ていなければ深さ `d + 1` にする。深さ `depth` のアカウントも Follow Set は取得するが、先へは広げない。`#p`（起点を名指ししているだけの相手。`referencing`）は深さ 0（起点）についてだけ 1 回取得し、クロールを広げるのには使わない。
- グラフ（`webring::build_graph`）: 取得した Follow Set の `p` のうち、見つけたアカウント（`#p` で見つかっただけの相手を除く）を指すものを辺（A → B は A の Follow Set に B がいる）にする。自分自身への辺は捨てる。辺を向きを無視してたどり、起点につながらないアカウントは除く。
- 残ったアカウントのサイトイベントを取得し、サイトごとの最新版の `d` をアカウントの名前にする。
- `text`: 見出しに件数、`Accounts` にアカウントごとの名前（`d` を `, ` でつないだもの。無ければ縮めた npub。同じ名前が複数あれば縮めた npub を添える）・npub・深さ・`[root]` / `[no follow set]`、`Mutual` に双方向の組、`One-way` に片方向の辺を出す。並びは（深さ、名前）の順。
  - 続けて `Referencing the root (unverified)` に、`#p` で見つかったアカウントを npub で先頭 `MAX_REFERENCING_LISTED`（[`../nostr/fetch.md#定数`](../nostr/fetch.md#定数)）件まで、超えた分は `… and N more` として出す（1 件も無ければ `(none)`）。
  - 表示したアカウントの Follow Set に載っているのに crawl に加えなかったアカウントがあれば、その数（`beyond`）を `(accounts beyond depth N, not shown: N)` として出す。
- `dot`: Graphviz の `digraph`。ノード ID は hex、ラベルは名前と縮めた npub。起点は `penwidth=2`、双方向の組は `dir=both` の 1 本にする。`referencing` は含めない。
- `mermaid`: `graph LR`。ラベルは名前と縮めた npub。起点は `root` クラス、双方向の組は `<-->` にする。`referencing` は含めない。
- Follow Set・サイトイベントのどれかの取得に失敗したらエラーで終了する。
- たどるアカウントの総数は `MAX_CRAWL_NODES`（[取得と表示の上限](../nostr/fetch.md#定数)）を超えない。超えて見つかったアカウントは crawl に加えず件数（`over_budget`）だけ数え、`text` の末尾に `(crawl stopped at the N-account budget; not reached: N)` として出す。`beyond` は深さの上限の外にいるものとこの上限で弾いたものの両方を数えるので、`over_budget` と重なることがある。アカウントの名前に使う `d` も 1 アカウントあたり先頭 `MAX_SITES_PER_AUTHOR_LISTED` 件までに切り詰める。
