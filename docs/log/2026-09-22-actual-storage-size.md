# 2026-09-22 サイトごとの実容量を測って表示する

## 問題

- state の `sites[].size` は版ごとに `dag/stat` の `TotalSize` を測った値で、集計（`state::site_bytes` / `account_bytes` / `total_bytes`、`policy::total_size`）はその単純な和だった。
- 1 つの DAG の中では同じブロックを 1 回しか数えないが、版と版の間では数えない。差分更新のサイトでは、ほとんど同じ内容の版が `keep_versions` 個ぶん丸ごと計上される。
- そのため帳簿の値は常に Kubo の実使用量以上になるが、どれだけ離れているかを見る手段が無かった。

## 決めたこと

| 決定 | 理由 |
|---|---|
| ポリシー（`max_per_site` / `max_per_account` / `max_total_storage` の判定と evict）は版ごとの合計のままにする | 上限を実容量で測ると、evict の途中経過ごとに `dag/stat` をやり直すことになり、純粋関数の `policy::decide` が IO に依存する。今の数え方は常に保守側（実使用量以上）に外れるので、上限を破ることはない |
| 実容量は表示だけに使い、state には記録しない | 読むのは `swing status` とダッシュボードの Storage check だけで、どちらも元から全版の DAG をたどる。state に持たせると `state.json` の形が変わり、古い state を読むための処置も要る |
| `dag_size_local` を CID の配列を取る形にし、`dag/stat` に `arg` を複数並べる | Kubo は複数の CID を渡すと `TotalSize` を重複排除後の合計で返す。実測で確かめた（下記） |
| サイトの検査を `health::check_site` にまとめ、まず全版を 1 回の `dag/stat` に渡す。成功したらその版はすべて完全とし、失敗したときだけ版ごとにやり直す | 1 つでもブロックが欠けていれば呼び出し全体が失敗するので、成功は全版の完全性を意味する。共有ブロックを 1 回しかたどらないぶん、起動時の突き合わせと `swing status` はむしろ速くなる。実容量も同じ呼び出しで得られる |
| 起動時の突き合わせ（`agent::reconcile`）も `check_site` に寄せる | 突き合わせと `swing status` が同じ基準で判定する形（[status コマンドの回](2026-09-17-status-command.md)）を崩さない |
| アカウント・全体の実容量はサイトごとの実容量の和にする | サイトをまたいだ共有まで数えるには全サイトの全版を一度に `dag/stat` に渡すことになり、リポジトリ全体をたどる。サイトをまたいで同じブロックを持つのは稀なので、ここは保守側のずれを許す |
| 測れなかったサイトは `unknown`（API は `null`）にし、合計も `unknown` にする | 一部だけ足した合計を実容量として出すと、少なく見える |
| `SharedSize` や `Ratio` は出さない | 判断に使うのは実容量そのもの。共有量は帳簿の値との差で分かる |

## 作ったもの

- `ipfs.rs`: `dag_size_local(&[&str])`。CID が 0 個なら呼ばずに 0 を返す。
- `health.rs`: `check_site` と `SiteHealth`（版ごとの判定 + 実容量）、`StatusReport.sites` と `actual_bytes()`。`swing status` は `Actual size` 見出しでサイトごとの実容量と合計を出す。
- `dashboard/dto.rs`: `/api/status` に `sites[]`（`actual`）と `actual_bytes` を追加。
- `web/sites.js`: Storage check の結果にサイトごとの実容量の表と合計を出す（`actualSize` / `actualSizeHint` / `actualSizeTotal`）。
- `web/style.css`: `#status-check-result` の中の見出しに上の余白、表に下の余白を足す。`.swing-table` は余白を持たないので、表のすぐ下に見出しや合計行が貼り付いていた（`Not in state` の見出しにも同じ余白が付く）。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（248 passed、12 ignored）。`check_site` が全版を 1 回の `dag/stat` で測ること、欠けた版があるときだけ版ごとにやり直して残りの実容量を出すことのテストを追加した。
- Kubo v0.43.1（test プロファイル）で統合テスト 8 件。追加したのは、同じ大きいファイルを共有する 2 版の `dag/stat` が「1 版より大きく、2 版の和より小さい」こと（`dag_size_local_counts_blocks_shared_by_versions_once`）と、完全な CID と欠けた CID を一緒に渡すと失敗すること。
- 手で確かめた `dag/stat` の挙動（Kubo v0.43.1）: 300KB のファイルを共有する 2 版で、単独が 300228 と 300236、両方を渡すと `TotalSize` が 300356（`UniqueBlocks` 7、単独の和 600464）。片方の葉ブロックを `block/rm` すると、両方を渡した呼び出しは `block was not found locally (offline)` で失敗する。
- デモ環境（`docker/demo/demo.sh up`）で、同じ 300KB の画像を持つ版を alice が 2 回 publish した状態を作り、`swing status` とダッシュボードの Storage check の両方で、帳簿 586.6 KB に対して実容量 294.3 KB と出ることを確認した。
