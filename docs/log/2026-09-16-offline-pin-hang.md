# 2026-09-16 offline pin が止まる問題の修正

## 問題

Kubo 0.43.1 で、`pin/add?offline=true` は、ルートのブロックがローカルに無いときは即エラーになるが、ルートがあって子ブロックが欠けているときはエラーにならず止まり続けた（60 秒待っても返らず、`progress=true` で見ると同じ位置で止まったまま）。agent は `dag/export` の後、state のロックを持ったまま `pin/add` をしていたので、export が途中で切れて子ブロックが欠けると、`SWING_PIN_TIMEOUT`（既定 15 分）の間、他のサイトの確定がすべて止まる。

## 決めたこと

| 決定 | 理由 |
|---|---|
| `pin/add` の前に `dag/stat?offline=true` を行う | `dag/stat` は欠けたブロックがあると即エラーになる（同じ条件で確認した） |
| 実サイズでの最終判定も `pin/add` の前に行う | pin する前に reject できるので、reject 時の解放が要らなくなる |

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（ユニットテスト 144 件）。欠けた DAG や実サイズで reject した版に対して `pin_add_local` が呼ばれないことを確認するテストに置き換えた。
- ローカルの Kubo 0.43.1（`IPFS_PROFILE=test`）で統合テスト 6 件。子ブロックを `block/rm` した DAG で `dag_size_local` が 5 秒以内にエラーになるテストを追加した。

## ついでに分かったこと

Kubo 0.43.1 の `pin/add?progress=true` の進捗には `Bytes`（取得済みバイト数）も含まれていた。取得方法に `dag/export` を選んだとき（`2026-09-16-dos-hardening.md`）は、進捗がブロック数しか返さないと判断していた。今の方法で問題は無いので変えていない。
