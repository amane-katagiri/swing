# 版の検査と MFS の突き合わせ（`src/health.rs`）

`state.json` に記録した版と、MFS 上の agent の領域（`<mfs_root>/agent/...`。[`mfs.md#mfs-の使い方`](mfs.md#mfs-の使い方)）を突き合わせる処理。agent の[起動時の突き合わせ](agent.md#起動時の突き合わせ)と [sweep](agent.md#sweep)、CLI の `swing status`（[`cli/views.md#status`](cli/views.md#status)）とダッシュボードの `GET /api/status`（[`dashboard/http-api/status.md`](dashboard/http-api/status.md#get-apistatus)）が共有する。ここには判定だけを書き、検査の結果で版を消すかどうかは agent 側、出力の書式と JSON はそれぞれのページに置く。

## 版の検査（`health::check_site`）

サイトごとに、そのサイトの全版をまとめて確かめる。

1. 版ごとに、版のパスの CID（`files/stat`）が記録と一致するかを見る。パスが無ければ `Missing`、違う CID なら `Mismatch`、`files/stat` が失敗したら `CheckFailed`。
2. 一致した版の CID をまとめて 1 回の `dag/stat`（`offline=true`）に渡し、成功すればその版はすべて完全（`Ok`）とする。
3. 失敗したときだけ版ごとに `dag/stat` をやり直して、どの版が欠けているかを決める。Kubo がブロックを手元に見つけられないと答えた（エラーに `ipld: could not find` を含む）ときだけ `Incomplete` とし、タイムアウトなどそれ以外の失敗は `CheckFailed` とする。

`Missing`・`Mismatch`・`Incomplete` は壊れた版（`VersionHealth::is_broken`）、`CheckFailed` は確認できなかった版で、壊れたものとしては扱わない。

同じ呼び出しで、`Ok` の版をまとめた `dag/stat` の `TotalSize` をサイトの実容量として返す（版どうしで共有しているブロックは 1 回だけ数える）。測れなかったら無し。DAG をたどるので、時間は保存量に比例する。

## state に無いパス（`health::find_garbage`）

`<mfs_root>/agent` を `<pubkey hex>/<site>/<created_at>` の 3 階層たどり、state の版のパスに当たらない項目を集める。この下は agent だけが使うので、state が参照しないものは残り物とみなす。

- 版が 1 つも残らない `<site>` は `<site>` のディレクトリごと、さらに `<pubkey hex>` の下に残る版が 1 つも無ければ `<pubkey hex>` のディレクトリごと 1 項目にする。想定外の階層にあるファイルも項目にする。
- 一覧に失敗したディレクトリは、理由と一緒に別に返し、その下は項目にしない。`<site>` の一覧に失敗したときは、その `<pubkey hex>` をディレクトリごとの項目にしない。

## status の集計（`health::collect_status`）

`state.json` を読み（無ければ空の state）、キー（`<pubkey hex>:<d>`）ごとに次を組み立てる。relay には接続しない。

- 版ごとの行: 版のパス・`cid`・state に記録した `size`・`created_at` と、[版の検査](#版の検査healthcheck_site)の結果。
- キーが `<pubkey hex>:<d>` として読めない版は検査せず、キーと `cid` だけを持つ「不正なキー」の行にする。
- サイトごとの実容量（[版の検査](#版の検査healthcheck_site)の `TotalSize`）と、その合計。測れなかったサイトが 1 つでもあれば合計も無し。
- [state に無いパス](#state-に無いパスhealthfind_garbage)と、一覧に失敗したディレクトリ。agent の実行中は、保存途中の版（MFS に置いた後、state を保存する前。[保存の順序](agent.md#保存の順序)）がここに出ることがある。
- 問題の件数: `Ok` 以外の版（`CheckFailed` を含む）と不正なキーの版の数、state に無いパスと一覧に失敗したディレクトリの数の合計。
