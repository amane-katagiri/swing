# ダッシュボード API の 500 で内部エラーの詳細を返さない

## 決めたこと

500（`ApiError::Internal`）の本文から、パスや OS のエラーメッセージを除く。詳細は `ApiError::into_response` で `error!` を使ってログに出し、クライアントには `internal error; see the swing log for details` だけを返す。

- 呼び出し元は `api::internal(context, e)` で `ApiError::Internal` を作る。以前は `internal` の中で `error!` を出していたが、ログは `into_response` の 1 か所に寄せた（アップロード・トークンの作り直し・QR の生成は `error!` を出していなかった）。
- 502（relay・Kubo・署名アプリの失敗）と 400 の本文は今までどおり詳細を返す。直し方をダッシュボードで伝える必要があり、中身は相手先の応答か入力の検証結果で、手元のパスは入らないため。

認証済みのクライアントしか見られない情報だが、絶対パス（ユーザー名を含む）や OS のエラーメッセージはリバースプロキシのログやブラウザの拡張などへ漏れやすい。ダッシュボードの利用者はサーバの持ち主なので、詳細はログで見てもらう。

## 変更

- `src/dashboard/api.rs`: `into_response` で詳細をログに出して汎用メッセージを返す。`internal` を `pub(super)` にして、`upload.rs`・`session.rs` と QR の生成もこれを使う。
- 設定ファイルが書けないときのテストは、以前は本文に `swing.toml` が入ることを確かめていた。入らないことと、ログへの案内が入ることを確かめる形に変えた。

## 検証

- `internal_errors_do_not_reach_the_client` を追加。
- `cargo test --workspace`・`cargo clippy --workspace --all-targets -- -D warnings` が通る。
