# 2026-09-23 `swing agent` サブコマンドを削除

## きっかけ

`swing up`（[配布方式の実装](2026-09-23-distribution-implementation.md)）が `[kubo].managed = false` のときも外部 Kubo のヘルスを待ってから `agent::run_until` を動かすようになり、失敗時の再起動ループも備えたことで、`swing agent`（`run_until` を単発で呼ぶだけの CLI エントリ、再起動ループなし）は `up` unmanaged の下位互換でしかなくなっていた。README も導入手順として `swing up` しか案内しておらず、`swing agent` を使う理由が無くなっていた。

## 決めたこと

- `swing agent` サブコマンドを削除する。`up --once`（起動して supervisor を経由せず 1 回だけ動かす）のような代替も作らない。デバッグや手動実行は `swing up`（`[kubo].managed = false` にすれば外部 Kubo に対してそのまま動く）で足りると判断した。
- `agent::run`（`lock::acquire` + `shutdown::cancel_on_signal` + `run_until` を呼ぶだけの CLI ラッパ）も一緒に削除する。呼び出し元は `main.rs` の `Command::Agent` だけだった。`run_until` とロック・シグナル周りの関数（`lock::acquire`・`shutdown::cancel_on_signal`）はそのまま残し、`up::run` が引き続き使う。
- ログメッセージの文言・モジュール名（`swing::agent::lifecycle` など）・「mirror-agent」という概念自体は変えない。無くすのは CLI のサブコマンドだけ。

## やったこと

- `src/main.rs`: `Command::Agent` と関連する分岐を削除。`Command::Stop` の `about` 文言から `swing agent` を除いた。
- `src/agent/lifecycle.rs`: `pub async fn run(config)` を削除し、`lock`・`shutdown` の未使用 import を整理（`ExitRequest`/`Exit` の import は `run_until` が使うので残した）。
- `src/agent/mod.rs`: `pub use lifecycle::{run, run_until}` から `run` を外した。
- ドキュメントを一通り修正: `docs/architecture/cli.md`（`agent` の節を削除、共通の注記・`up`/`stop` の説明を更新）、`docs/architecture.md`（CLI 一覧・`swing agent`/`swing up` の説明文・dashboard.md の索引の説明文）、`docs/architecture/agent.md`（公開関数の説明、`agent/lifecycle.rs` の役割、「シグナルと終了」の主語を `swing up` に変更）、`docs/architecture/dashboard.md`・`docs/architecture/dashboard/http-api.md`（`swing agent` 単体プロセスへの言及を `swing up` に統一）、`docs/architecture/up.md`（`cancel_on_signal`/`lock::acquire`/`ipfs_api_url`/exit code の説明から `agent::run`/`swing agent` を除去）、`docs/architecture/service.md`（`swing stop` の対象を `swing up` のみに）。`docs/plan.md`（歴史的資料）と過去の `docs/log/*` は変更していない。

## 検証

- `cargo fmt`
- `cargo clippy -j 3 --all-targets -- -D warnings`
- `cargo test -j 3`
- `cargo xwin clippy -j 3 --target x86_64-pc-windows-msvc --all-targets -- -D warnings`
- `cargo run -q -- --help` の出力に `agent` が無いことを確認。
- リポジトリ全体を `swing agent` / `Command::Agent` / `agent::run(` / `swing::agent::run` で再検索し、`docs/log/*`（過去ログ）と `docs/plan.md` 以外に残っていないことを確認。
