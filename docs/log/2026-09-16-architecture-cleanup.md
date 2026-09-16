# 2026-09-16 architecture.md の整理と分割

## 問題

`docs/architecture.md` が 475 行に膨らみ、理由の説明（「〜のため」）、README や `.env.example` と重複する内容、他の節と重複する説明が混ざっていた。

## 決めたこと

| 決定 | 理由 |
|---|---|
| `architecture.md` を構成・CLI・設定・イベントの検証・テストと索引に絞り、agent・NIP-05・Kubo/MFS・Docker を `docs/architecture/` の 4 ファイルに分ける | agent の動作と Kubo の詳細が大半を占め、CLI や設定を探しにくかった。NIP-05 は agent と publish の両方から参照するので独立させた |
| 理由の説明を削る（add 中の GC を `dag/stat` で検出する理由、配置を確認より先に行う理由、Follow Set を state に保存する理由、`Provide.Strategy` で確認した値など） | AGENTS.md の規則どおり、理由は log（`mfs-storage`、`follow-set-rollback`、`pin-kubo-version` など）に既にある |
| publish の出力例を削り、README に任せる | README と同じ内容だった |
| `docs/examples/publish.sh` の節を削る | スクリプト自体を読めば足り、サポート対象でもない |
| リポジトリ構成から docs 配下の説明を削る | AGENTS.md と重複していた |
| `kind`・`d` の既定値の再掲、`reqwest::Client` の持ち方、`key.rs` の関数分割の説明を削る | 設定の節やコードと重複していた |
| agent の流れをコードの順序（接続 → 突き合わせ → tick ごとに sweep・Follow Set・unfollow・再取得）に合わせて書き直し、unfollow を独立した節にする | 旧版は突き合わせが最後の項目にあり、実際の順序と読み違えやすかった |
| relay からの取得が 30 秒でタイムアウトすることを追記する | コードにあるのに記述が無かった |

AGENTS.md の文書の役割表と、README のリンク（ポリシー判定の参照先、ドキュメント一覧）を合わせて更新した。

## 検証

- 分割後の記述を `src/agent.rs`（`run`、`refresh_follow_set`）、`src/nostr.rs`、`src/nip05.rs`、`compose.yaml`、`Dockerfile` と照らし合わせた。
