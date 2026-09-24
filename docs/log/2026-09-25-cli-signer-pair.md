# 2026-09-25 CLI から署名アプリとペアリングする（`swing signer pair`）

## 作ったもの

- `swing signer pair [--config] [--relay <URL>]...` を足した（`src/pair.rs`）。ターミナルに QR コードと `nostrconnect://` のリンクを出し、署名アプリの接続と確認の署名を待って `<state_dir>/remote-signer.json` に保存する。これまで NIP-46 のペアリングはダッシュボード（セットアップ画面とつなぎ直し）からしかできなかった。
- ダッシュボードと共通にするため、`signer.rs` に `parse_pairing_relays`（`dashboard/api.rs` の `parse_signer_relays` を移した）・`PairingRequest::for_config`・`qr_text`（Unicode のブロック文字の QR）を足した。
- 署名アプリ役のテスト用部品（`TestSigner`・`serve_test_signer`）を `signer::tests` から `src/test_support.rs` に移し、`pair::tests` と共有した。

## 決めたこと

| 決定 | 理由 |
|---|---|
| ダッシュボード API を通さず、CLI のプロセス内でペアリングする | `swing up` が動いていなくてもペアリングできるようにするため。書くのは `remote-signer.json` だけで、ダッシュボードの `pairing` 状態とは関係しない |
| 秘密鍵が設定されていれば QR を出す前に断る | 鍵と `remote-signer.json` の両方があると `Signer::load` がエラーになる。秘密鍵から署名アプリへの切り替えは todo の別項目（ダッシュボードでもまだできない）で、今回は扱わない |
| `remote-signer.json` があれば、同じアカウントのつなぎ直しだけを受け付ける | ダッシュボードの `POST /api/signer/reconnect` と同じ決まり。アカウントを変えると、Follow Set・公開したサイト・レプリカ報告が前のアカウントに残るため |
| 別アカウントは、接続した時点（確認の署名をリクエストする前）で断る | 署名アプリに承認を求めてから断ると、ユーザーに無駄な承認をさせることになる。公開鍵は `Checking` の時点で分かる |
| 保存後に `swing up` を自動で再起動しない。再起動するよう案内だけ出す | 再起動は不要と判断した（ユーザーの判断で入れない） |
| relay の既定は `wss://relay.primal.net` | ダッシュボードのペアリング画面の既定値に合わせた |
| QR は明るいモジュールをブロック文字で描く | ターミナルは暗い背景が多い。明るい背景では白黒が逆になるので、読めないときは一緒に出すリンクを貼ってもらう |

## 検証したこと

- `cargo fmt`・`cargo clippy --all-targets -- -D warnings`・`cargo test --workspace` が通った。
- 新しいテスト: `pair::tests`（新規保存・秘密鍵があれば断る・別アカウントを断ってファイルを変えない）、`signer::tests::qr_text_is_a_block_of_equal_width_lines`、`signer::tests::pairing_relays_are_trimmed_and_validated`。
- デモ環境（mirror を外に出す `SWING_DEMO_NIP05=1` のモード）で、`wss://relay.primal.net` を使って次を確かめた。
  - 秘密鍵を設定した mirror で実行すると、秘密鍵のエラーで止まる。
  - `SWING_NOSTR_SECRET_KEY=` と使い捨ての `SWING_STATE_DIR` で実行し、nostr-connect の `NostrConnectRemoteSigner` で作った使い捨ての署名アプリ役から、新規・同じアカウントのつなぎ直し・別アカウントの 3 通りを試した。別アカウントのときは、署名アプリ役に `get_public_key` しか届かない（確認の署名をリクエストしない）。
  - 実機の署名アプリで QR を読み取り、ペアリングできた。
- 署名アプリ役を URI が出た直後に起動すると、ペアリングが最後まで進まないことがあった。kind 24133 は relay に保存されない（ephemeral）ので、SWING が relay を購読し始める前に届いた `connect` の応答は失われる。ダッシュボードも同じ `Pairing::start` を使っていて同じ条件にある。人が QR を読み取るには数秒かかるので、実際には起きないと判断して直していない（既存のテストも同じ理由で 500ms 待ってから署名アプリ役を動かす）。
