# state.json を持ち主だけが読めるように書く

## 決めたこと

`state.json` の書き込みを、ダッシュボードのトークンや `remote-signer.json` と同じ `auth::write_private_file` にする。unix では `0o600` の一時ファイルに書いて `fsync` し、rename する。`state_dir` が無ければ `auth::create_private_dir_all`（新しく作るディレクトリだけ `0o700`）で作る。

todo では「`state.json` とアップロードの一時ディレクトリ」を挙げていたが、アップロードの一時ディレクトリとファイルは `upload::create_private_dir_all`・`create_private_file` で `0o700`・`0o600` になっていたので、今回は `state.json` だけを直した。

`state.json` に秘密鍵は入らないが、ミラーしているアカウントとサイトの一覧が入るので、同じマシンの他のユーザーからは読めないようにした。

既存の環境への影響:

- 既にある `state.json` は、次に保存したときに rename で置き換わって `0o600` になる。移行のためのコードは足していない。
- 既にある `state_dir` のモードは変えない（`create_private_dir_all` は既存のディレクトリに触れない）。

## 検証

- `state::tests::saved_state_and_new_state_dir_are_private` を追加（unix だけ）。
- `cargo test --workspace`・`cargo clippy --workspace --all-targets -- -D warnings` が通る。
