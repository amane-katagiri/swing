# Windows でファイルとディレクトリの衝突が 500 になっていた

## 何が起きたか

v0.1.0 のリリースのワークフローで、`x86_64-pc-windows-msvc` のテスト `upload_rejects_a_file_and_a_directory_at_the_same_path` が落ちた。`a/b` の後に `a` を受け取ると、400 ではなく 500 を返していた。

## 原因

衝突の判定を `create_new` や `create_dir_all` の失敗の `ErrorKind`（`AlreadyExists`・`NotADirectory`・`IsADirectory`）に頼っていた。Windows では既存のディレクトリと同じ名前のファイルを `create_new` で開くと「アクセス拒否」になり、`PermissionDenied` として返るので、内部エラー扱いになっていた。

## 決めたこと

- 受け取ったパスを大文字小文字を区別せずに覚えておき、ファイルのパスとその親ディレクトリのパスの集合どうしで、書き込む前に衝突を判定する。OS ごとのエラーの違いに左右されない。
- `ErrorKind` による判定は、そのまま残す。

## 検証

- Linux で `cargo test -p swing --lib dashboard::upload`、`cargo clippy --workspace --all-targets -- -D warnings` が通ることを確かめた。
- Windows ではリリースのワークフローで確かめる。
