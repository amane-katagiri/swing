# 表示と Follow Set の操作（`src/mirror/print.rs`, `src/replicas.rs`, `src/health.rs`, `src/stats.rs`, `src/webring/render.rs`）

[`../cli.md`](../cli.md) の子ページ。`mirror list` / `add` / `remove`・`sites`・`replicas`・`status`・`stats`・`webring` の出力の形式。ダッシュボード API と共有する集計は [`../mirror.md`](../mirror.md)（Follow Set と sites）・[`../health.md`](../health.md)（status）・[`../webring.md`](../webring.md)（webring）にある。relay・Kubo へ直接つなぐか API を経由するかは [`../cli.md#共通`](../cli.md#共通)。

## mirror list / add / remove

自分の Follow Set を操作する。`list` は relay に直接つなぎ、`add` / `remove` は `POST /api/mirror/add` / `/api/mirror/remove`（body は `{"keys": [...]}`、応答とエラーは [`../dashboard/http-api/nostr.md`](../dashboard/http-api/nostr.md#post-apimirroradd-post-apimirrorremove)）を叩いて結果を印字する。使う Follow Set の選び方と書き換え方は [`../mirror.md`](../mirror.md)。

- 保存済みの Follow Set を使ったときは、その注記（[`../mirror.md#使う-follow-set`](../mirror.md#使う-follow-set)）を先頭に出す。`sites` も同じ。
- `list` は `Mirror set: <mirror_set> (kind 30000)`、`title` タグがあれば `Title:`（`sites` の `title:` 行と同じく無害化する）、続けて pubkey ごとに npub と hex を併記する。Follow Set が無ければ `(no follow set found)`。
- `add` / `remove` は no-op の鍵を `already in mirror set` / `not in mirror set`、変更した鍵を `added` / `removed` として出し、続けて relay ごとの送信結果と変更後の pubkey の一覧を出す。変更が無ければ `no changes; not publishing` で終わる。
- Follow Set が無い状態の `remove` は `(no follow set found); no changes` とだけ表示して終わる。
- どの relay も答えなかったときは、`list` も `add` / `remove` もエラーで終了する。`add` / `remove` は API がエラーを返したら（Follow Set の上限を超える `add` の 409 を含む）その `error` を表示して終了する。

## sites

集計は [`../mirror.md`](../mirror.md#sites-の集計mirrorcollect_sites)。Follow Set が無ければ `(no follow set found)`、対象が空なら `(follow set is empty)` を出す。

- アカウントごとに npub と hex の見出しを出し、サイトごとに 1 行（`d`、`cid`、`url`、`size`、`created_at`、NIP-05 検証結果、`replicas`、`[stored]` / `[not stored]`）表示する。サイトイベントが無いアカウントは `(no site events)`。
  - `size` 列: `stored_size` があれば数値で、無ければイベントの自己申告の `size` を括弧書き（例 `(12345)`）で、どちらも無ければ `-` を出す。
  - `replicas` 列: `3` または `3 (+12 unverified)`（未検証の報告者がいるとき）の形（`replicas::format_replica_counts`）。レプリカ報告の取得に失敗したら先頭に `(fetching replica reports failed: ...)` を表示して `-` にする。
- `title` があれば次の行に `    title: `、`message` があれば続けて `    message: ` を出す。どちらも `format::sanitize_display_text` で無害化し 200 文字までにする（超えた分は `…`）。`url` 列も `format::Sanitized` を通す。
- 続けて、unfollow の一覧（[`../mirror.md#unfollow-の一覧`](../mirror.md#unfollow-の一覧)）を `Unfollowed but still stored` 見出しの下に、アカウントの見出しに `[unfollowed]` を付け、サイトごとに `url` を `-`、状態を `[unfollowed]` にした行で出す。見出しには `remove_on_unfollow` と Follow Set が見つかったかどうかに応じて、次の poll で消えるか・残すか・Follow Set が見つかるまで消さないかを添える。

## replicas

state は読まない。

- `<key>` を作者として扱う。省略時は自分の pubkey。
- 作者ごとに、サイトごとの最新のサイトイベントについて `d`、`cid`、`replicas=<数> (reports=<有効な報告の数>)` を表示し（`<数>` は `sites` の `replicas` 列と同じ形）、続けて報告者ごとに npub と `[latest]` / `[older version]` を 1 行ずつ表示する。
- 報告者ごとに信頼度の tier を `[author]` / `[chosen]` / `[unverified]` で添える。tier の定義・数える報告の条件・並びは [「レプリカ報告の信頼度」](../nostr.md#レプリカ報告の信頼度replicastier)。
- 集計は [`replicas::collect`](../nostr.md#一覧の集計replicascollect)。`reports=` は切り詰め後の件数で、切り詰めがあれば報告者一覧の後に `… and N more report(s) not shown` を出す。
- 集計がエラーになったらエラーで終了する。

## status

`GET /api/status` を叩き（[共通](../cli.md#共通)）、API 側が `state.json` と Kubo だけを見て組み立てた結果（[`../health.md`](../health.md#status-の集計healthcollect_status)）を印字する。agent が未準備なら失敗する（[`dashboard/http-api.md#共通`](../dashboard/http-api.md#共通)）。

- `Stored versions (<state.json のパス>):` の下に、版ごとにパス・`cid`・`size` と判定を 1 行表示する。判定は `[ok]` / `[missing]` / `[cid mismatch]` / `[incomplete]` / `[check failed]` で、`ok` 以外は `: <理由>` を添える。キーが読めない版はキーと `cid` に `[invalid site key]` を付けて出す。
- `Actual size` 見出しの下に、サイトのパスごとの実容量と `total` を表示する。測れなかったものは `unknown`。
- `Not in state` 見出しの下に、state に無いパスを表示する。一覧に失敗したディレクトリは `[list failed]: <理由>` 付きで出す。
- 問題が 1 つでもあれば `<件数> problem(s) found` で 0 以外で終了する。
- 実行時間は保存量に比例する。ダッシュボードのリクエストタイムアウト（[`dashboard.md#タイムアウトsrcdashboardmodrs`](../dashboard.md#タイムアウトsrcdashboardmodrs)）と `ApiClient` 側のタイムアウト（125 秒）の範囲で待つ。

## stats

`swing up` が測って持っているリソース使用量（[`stats.md`](../stats.md)）を `GET /api/stats?since=<今 − last>` で取り（[共通](../cli.md#共通)）、表にする。`--last`（既定 `1h`）は `30m`・`6h`・`1d` のような長さ（単位なしは秒）。`swing up` が持つ期間は [`stats.md#測り方`](../stats.md#測り方)。セットアップモードでも使える。

- 1 行目にサンプル数・期間・間隔・最新のサンプルが何秒前か。サンプルが無ければ `No samples in the last <期間>; swing up takes one every <測る間隔>.` だけを出す（間隔は [`stats.md#測り方`](../stats.md#測り方)）。
- 表は `swing CPU`・`swing memory`・`Kubo CPU`・`Kubo memory`・`IPFS in`・`IPFS out` の 6 行で、列は `now`（最新のサンプルの値）・`avg`・`max`（期間内の値のあるサンプルでの平均と最大）。値が無ければ `-`。バイト数は 1024 基数で小数 1 桁に丸める。
- 表の下に、最新のサンプルに通信量があれば Kubo 起動からの累計（`IPFS total since Kubo started: in …, out …`）、`kubo_managed` が `false` なら Kubo が SWING の管理外で CPU・メモリを取得できない旨を出す。
- `--json` を付けると API の応答（[`dashboard/http-api/status.md#get-apistats`](../dashboard/http-api/status.md#get-apistats)）をそのまま整形して出す。

## webring

- `<key>` を起点にする。複数指定でき、すべて深さ 0 の起点になる。省略時は自分の pubkey。`--depth` の既定は 2（上限なし）、`--format` の既定は `text`。たどり方・グラフ・名前・`beyond`・`over_budget`・`referencing` は [`../webring.md`](../webring.md)。
- `text`: 見出しに件数、`Accounts` にアカウントごとの名前（無ければ縮めた npub。同じ名前が複数あれば縮めた npub を添える）・npub・深さ・`[root]` / `[no follow set]`、`Mutual` に双方向の組、`One-way` に片方向の辺を出す。並びは（深さ、名前）の順。
  - 続けて `Referencing the root (unverified)` に `referencing` を npub で出し、落とした分は `… and N more`（1 件も無ければ `(none)`）。
  - `beyond` が 0 より大きければ `(accounts beyond depth N, not shown: N)`、`over_budget` が 0 より大きければ `(crawl stopped at the N-account budget; not reached: N)` を末尾に出す。
- `dot`: Graphviz の `digraph`。ノード ID は hex、ラベルは名前と縮めた npub。起点は `penwidth=2`、双方向の組は `dir=both` の 1 本にする。`referencing` は含めない。
- `mermaid`: `graph LR`。ラベルは名前と縮めた npub。起点は `root` クラス、双方向の組は `<-->` にする。`referencing` は含めない。
- 取得に失敗したらエラーで終了する。
