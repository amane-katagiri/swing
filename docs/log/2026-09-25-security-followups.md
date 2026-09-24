# 2026-09-25 セキュリティレビューの小さな追随対応

## 対応した項目

前日（[2026-09-24 セキュリティレビュー対応](2026-09-24-security-hardening.md)）の続きで見つかった小さな項目 3 件。

1. **todo に追記のみ**: `src/dashboard/upload.rs`・`src/dashboard/api.rs` の `ApiError::Internal(format!("{e:#}"))` が、認証済みクライアントに内部エラーの詳細（絶対パスや OS のエラーメッセージを含みうる）をそのまま返している（ペアリング・remote-signer の保存・QR 生成・アップロードの各エンドポイント）。今回は直さず `docs/todo.md` に「レビュー（セキュリティ）」出所・優先度 低で記録した。修正案は `tracing::error!` で詳細を残しつつ `Internal` はクライアントには汎用メッセージだけ返すこと。
2. `src/kubo.rs::locate_binary` が `[kubo].binary` 明示以外（実行ファイルの隣・`PATH`）で見つけたときに解決済みパスを `tracing::info!` するようにした。
3. `src/settings/edit.rs::write_atomic` の親ディレクトリ作成を `std::fs::create_dir_all` から `crate::auth::create_private_dir_all` に変え、新規作成分だけ unix で `0o700` にした（既存ディレクトリは触らない）。

## 決めたこと

| 決定 | 理由 |
|---|---|
| `locate_binary` のログは `[kubo].binary` を明示したケースには出さない | 明示した場合は設定ファイルに書いてある値をそのまま使うだけで、解決の余地（＝ログで見せる価値のある分岐）が無い |
| `locate_binary` は `swing up`（`src/up.rs`）からしか呼ばれておらず、他の非 daemon な CLI パスからの呼び出しは無い | grep で確認済み。「info だとノイズになる別経路」は存在しないため、呼び出し元ごとの出し分けは不要と判断した |
| `write_atomic` の親ディレクトリ作成は `auth::create_private_dir_all` に統一する | 2026-09-24 の対応で `state_dir` 系（`auth.rs`/`lock.rs`/`signer.rs`）はすでにこのヘルパーに揃えていた。`swing.toml` の親ディレクトリだけ素の `create_dir_all`（umask 任せ）が残っていたので同じ扱いに揃えた |

## 検証したこと

- `cargo fmt`・`cargo clippy -j 2 --workspace --all-targets -- -D warnings`・`cargo test -j 2 --workspace`（464 passed, 15 ignored）・`cargo xwin clippy -j 3 --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings` すべて成功。
- 新規テスト: `settings::edit::tests::missing_parent_directory_is_created_as_0700`（既存の `new_file_gets_owner_only_permissions` と同じ「未作成のネストしたディレクトリ」構成で、今度は親ディレクトリのパーミッションを検証）。
- `src/up.rs` で `main.rs::init_tracing` が `run(cli)` より前に必ず呼ばれることを確認し、`swing up` 実行時に `locate_binary` の info ログが実際に出る経路であることを確かめた。
- `docs/architecture/up.md`（Kubo バイナリ検出の節。ログと、複数ユーザーホストでは `[kubo].binary` を明示すべきという注意を追記）・`docs/architecture/dashboard.md`（設定ファイルの書き込み手順に親ディレクトリ作成の記述を追加）・`docs/todo.md` を更新した。
