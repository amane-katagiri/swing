# 他人が影響できる文字列を端末にそのまま出さない

[追加修正の見直し](2026-10-07-followup-review.md) で、`install.sh` が他人の付けたディレクトリ名をそのまま root の端末に出すことを LOW として残した。同じ種類のものを洗い出して直した。

## 決めたこと

- 他人が影響できる文字列（relay や署名アプリの応答、Kubo や他のサーバーの応答、公開するディレクトリのファイル名、他人が名前を付けられるディレクトリのパスなど）は、端末に出す前に制御文字と不可視の書式文字を取り除く。改行やエスケープシーケンスで、出力を偽装したり端末を操作したりできないようにする。
- `swing up` などのログは、tracing-subscriber が本文の一部の制御文字しかエスケープせず、`%` で渡したフィールドは何もしない。呼び出しごとに直すのではなく、フィールドの書式（`logging::SanitizedFields`）でまとめて取り除く。
- CLI のエラーは、anyhow に任せず `format::error_report` で出す。原因の連鎖を 1 つずつ取り除いてから出すため。改行を含めていた 2 つのエラー（`--system` の検査の結果、外部コマンドの失敗）は 1 行にした。
- 取り除かないもの: 検証済みの値（`d`・`title`・`cid`）、運用者自身の入力（`--message` など。`message` はわざと複数行にできる）、自分の設定から来るパスや relay の URL、ローカルのツールの出力、JSON の API の応答（Web UI が表示の前に取り除く）。

## 変えたもの

- `install.sh` は、`info`・`warn`・`die` のメッセージ全体の制御文字と不可視の書式文字を `?` にしてから出す。
- `format::Sanitized` を足し、`sanitize_display_text` もこれを使うようにした。publish・`status`・`mirror sites`・`signer pair`・`service` の一覧で、外から来る値をこれで包んで出す。

## 検証したこと

- `cargo fmt`・`cargo clippy -D warnings`・`cargo test --workspace`・Windows 向けの `cargo xwin clippy` が通る。
- `packaging/linux/test-install.sh` が、一般ユーザーと root（Debian のコンテナ）で通る。名前に改行・ESC・U+202E を含むディレクトリの下での拒否が 1 行で出て、ESC が出力に残らないことも確かめる。
- CLI のエラーが `Error: ...` の形で出て、終了コードが 1 のままであること。
