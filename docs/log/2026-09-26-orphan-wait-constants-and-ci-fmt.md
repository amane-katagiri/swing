# 孤児 Kubo 回収の待ち時間の定数化と CI の fmt 対象

[停止の時間予算](2026-09-25-shutdown-timeout-budget.md)の回で `Daemon::stop` の待ち時間は `SHUTDOWN_RPC_TIMEOUT`・`SIGTERM_GRACE` に名前を付けたが、同じ `kubo.rs` の孤児回収（`recover_orphan`）は数値を直書きしたままだった。あわせて、CI の `cargo fmt --check` がルートパッケージしか見ていないことに気づいたので直した。

## 孤児回収の待ち時間

値は変えず、名前だけを付けた。

| 定数 | 値 | 使う場所 |
|---|---|---|
| `ORPHAN_SHUTDOWN_RPC_TIMEOUT` | 3 秒 | `attempt_graceful_shutdown` の `/api/v0/shutdown` のリクエストタイムアウト |
| `ORPHAN_SHUTDOWN_GRACE` | 30 秒 | API が応答した後の終了待ち |
| `ORPHAN_SIGTERM_GRACE`（unix のみ） | 30 秒 | SIGTERM 後の終了待ち |
| `ORPHAN_KILL_WAIT` | 10 秒 | SIGKILL（unix）・`taskkill /T /F`（Windows）後の終了待ち |

- `Daemon::stop` の定数とは共有しなかった。RPC タイムアウトは 5 秒と 3 秒、SIGTERM 後の待ちは 10 秒と 30 秒で値が違い、前者はシグナル時の時間予算に収める必要がある一方、孤児回収はトークンの cancel で途中終了できるので予算の制約を受けない。意味が違うものを 1 つの名前にまとめると、片方の都合で変えたときにもう片方も動いてしまう。命名は `Daemon::stop` 側（`SHUTDOWN_RPC_TIMEOUT`・`SIGTERM_GRACE`）に `ORPHAN_` を付けた形に揃えた。
- SIGKILL 後と `taskkill /F` 後の待ちは「強制終了した後にプロセスが消えるのを待つ」という同じ意味なので、1 つの定数にした。
- 回収の最悪時間を `const fn`（`daemon_stop_budget` と同様）で表すことは見送った。`daemon_stop_budget` は `up.rs` の停止予算の計算に使われているが、孤児回収の最悪時間を使う側が無く、テストのためだけの関数になるため。

## CI の fmt

`.github/workflows/release.yml` の `cargo fmt --check` を `cargo fmt --all --check` にした。`cargo fmt` は `--all` が無いとルートパッケージ（`swing`）だけを整形・検査し、ワークスペースの `swing-tray`（`tray/`）を見ない。clippy / test は既に `--workspace` で回していたので、これで 3 つとも対象が揃った。AGENTS.md のコーディング規則も `cargo fmt --all` / `--workspace` 付きの clippy / test に直した。

`cargo fmt --all --check` を実行した時点で `tray/` に差分は無かった。

## 検証

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo xwin clippy --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings`（`#[cfg(windows)]` の `terminate_process` に触れたため）
- `cargo test --workspace`
