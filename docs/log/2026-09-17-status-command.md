# 2026-09-17 `swing status` を追加し、突き合わせと sweep の検査を共通化する

## 問題

- state.json に記録した版が Kubo の MFS に本当に揃っているかは、agent の起動時の突き合わせでしか確かめられず、結果も warn ログにしか出なかった。
- MFS に残った余分なパスも sweep が黙って消すだけで、外から確認する手段が無かった。

## 決めたこと

| 決定 | 理由 |
|---|---|
| 版の検査（`files/stat` の CID と `dag/stat`）と、sweep が消すパスの算出を `health.rs` に切り出し、agent と `swing status` で共有する | 同じ基準で判定しないと、status が「ok」と言った版を agent が取り直す、といったずれが起きる |
| sweep は `find_garbage` が返したパスを消すだけにする。版が 1 つも残らない `<site>` / `<pubkey hex>` はディレクトリごと 1 回で消す | status で「sweep が消すもの」を最上位のパスだけで表示でき、agent 側の結果も以前と同じ |
| `status` は読み取り専用で relay に接続しない | state.json と Kubo だけで判定でき、Follow Set の状態は `swing sites` の担当 |
| 問題（`ok` 以外の版、余分なパス、一覧の失敗）が 1 つでもあれば 0 以外で終了する | cron や監視から使えるようにする |
| `files/stat` 自体の失敗（`check failed`）も問題として数える | agent は残すが、Kubo に届いていない状態は監視側で気づきたい |
| エラーは `{:#}` で原因の連鎖まで出す | 最上位の文脈（`POST /api/v0/files/stat`）だけでは接続失敗かどうか分からなかった |
| Kubo のエラー本文は前後の空白を除いて埋め込む | 本文が改行で終わるので、表示に空行が混ざっていた |
| 全版の DAG をたどる遅さは許容する | 起動時の突き合わせと同じ検査で、手動・定期実行の用途なら問題にならない |

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（ユニットテスト 156 件、ほかに `#[ignore]` 1 件）。`check_version` の各判定、`find_garbage` が最上位の余分なパスを返すこと、一覧に失敗したディレクトリを消さないことのテストを追加した。既存の sweep・突き合わせのテストはそのまま通る。
- `agent_stores_and_removes_through_real_kubo` をローカルの Kubo（v0.43.1、test プロファイル）で通した。
- 同じ Kubo に対して、正常・パス欠落・CID 不一致の版と、state に無い版・pubkey ディレクトリを用意して `swing status` を実行し、各判定と余分なパスの表示、終了コード 1 を確認した。余分なものを消すと終了コード 0、到達できない API を指定すると `check failed` と `list failed` が原因付きで出ることも確認した。
- MFS に置いたディレクトリの子ブロックを `block/rm` で消し、`files/stat` は一致するが `dag/stat`（offline）が失敗する版を作って、`incomplete` と終了コード 1 を確認した。
