# 2026-09-25 mirror add の Follow Set 上限超過を 409 にする

## 問題

`POST /api/mirror/add` で追加後の Follow Set が `MAX_FOLLOW_SET_ENTRIES`（500）を超えると、`mirror::ensure_within_follow_set_cap` の `anyhow` エラーが `api::upstream` を通って 502 Bad Gateway になっていた。relay の障害ではなく利用者の操作が原因なので、上流エラーのステータスは不適切。

## 決めたこと

| 決定 | 理由 |
|---|---|
| ステータスは 409 Conflict | ダッシュボード API の表では 400 は「入力不正」（リクエスト単体で判定できる誤り）、422 は publish の NIP-05 `require` 失敗専用、409 は「リクエスト自体は正しいが今の状態では受け付けられない」（publish の多重実行、セットアップ・ペアリングを使えない状態）に使っている。上限超過は `keys` 自体は正しく、今の Follow Set の件数との組み合わせで決まるので 409 に当たる。422 を広げると publish 画面が 422 を NIP-05 失敗として扱う前提が崩れる |
| エラーは専用の型 `mirror::FollowSetCapExceeded` にし、API 側で `anyhow::Error::downcast_ref` で見分ける | `apply_add` の戻り値は `anyhow::Result` のままにして CLI・他の呼び出し元を変えずに済む。文言は従来の `bail!` と同じ |
| 振り分けは `api::mirror_add_error` に置き、`mirror_remove` は `upstream` のまま | 上限の判定は `add` にしか無い |
| レスポンスボディは既存の `{ "error": "<メッセージ>" }`（`ApiError::Conflict`） | 他のエラーと同じ形にして、Web UI（`describeError`）と `ApiClient` がそのまま扱えるようにする |
| CLI（`swing mirror add`）は変更しない | `ApiClient` はステータスに関係なく `error` の文言をそのまま出すので、従来どおり `would grow the follow set to <N> entries, over the 500-entry limit; remove some first` と表示して非ゼロで終わる |

## 作ったもの

- `src/mirror.rs`: `FollowSetCapExceeded { total }`（`Display`・`std::error::Error`）。`ensure_within_follow_set_cap` がこれを返す。
- `src/dashboard/api.rs`: `mirror_add_error`。`FollowSetCapExceeded` なら `ApiError::Conflict`（409）、それ以外は `upstream`（502）。
- テスト（`src/dashboard/api.rs`、Router に oneshot）:
  - `mirror_add_past_the_follow_set_cap_is_conflict_not_bad_gateway`: プロセス内の `LocalRelay` に自分の鍵で 500 件の `p` を持つ Follow Set を置き、1 件追加すると 409 と上限の文言が返る。
  - `mirror_add_relay_failure_stays_bad_gateway`: relay を 1 つも持たない `RelayClient` で Follow Set の取得を失敗させると 502 のまま。

## 検証したこと

- `cargo fmt`・`cargo clippy -j 2 --workspace --all-targets -- -D warnings`・`cargo test -j 2 --workspace`（480 passed, 16 ignored）が成功。
- `mirror_add_error` を常に `upstream` にすると上限超過のテストが 502 で落ちることを確かめ、テストが振り分けを検出できることを確認した。
- ドキュメント: `docs/architecture/dashboard/http-api.md`（ステータス表の 409 と mirror add の節）、`docs/architecture/cli.md`（mirror 節）、`docs/architecture/nostr.md`（`MAX_FOLLOW_SET_ENTRIES` の行）を更新した。
