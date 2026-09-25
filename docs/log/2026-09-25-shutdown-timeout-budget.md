# 2026-09-25 シグナルによる停止の時間予算をそろえる

シグナル（SIGINT/SIGTERM、Windows は Ctrl+C と `--exit-with-parent` の親の終了）で `swing up` を止めるときの時間の上限がそろっていなかったので直した回。architecture 側の結果は [`up.md`](../architecture/up.md#停止の時間予算)・[`kubo.md`](../architecture/kubo.md#停止daemonstopgrace)・[`service.md`](../architecture/service.md)・[`../architecture.md`](../architecture.md) に反映済み。

## きっかけ

`shutdown.rs` の `on_signal` はシグナルを受けてトークンを cancel したあと、固定の 10 秒（`FORCE_EXIT_GRACE_PERIOD`）で `std::process::exit(1)` していた。一方 managed の停止手順は agent 待ち 15 秒 → `Daemon::stop(30s)`（RPC 5 秒 → 30 秒 → unix は SIGTERM → 10 秒 → kill）→ ダッシュボード 5 秒で、最悪 65 秒かかる。そのため:

- Kubo の RPC による丁寧な停止が、待ち切る前に打ち切られることがあった。
- macOS の plist は `KeepAlive = { SuccessfulExit = false }` なので、`swing service stop`（`launchctl kill SIGTERM`）で止めたつもりが exit 1 で終わり、launchd に再起動されることがあった。
- systemd の `TimeoutStopSec=60` も、内側の最悪 65 秒より短かった。

## 決めたこと

- 「内側の停止手順の最悪合計 < 強制終了までの猶予 < サービスマネージャの上限」をそろえる。値はコードの定数から導き、関係をユニットテストで固定する（`up::tests::stop_budget_fits_within_force_exit_and_service_manager_limits`）。
- 内側の最悪は実際の経路を数えた。managed でシグナルが来た経路（`token.cancelled()` の腕、および agent が cancel を見て `Ok` で返る腕）は agent 待ち 15 秒 + `Daemon::stop` + ダッシュボード 5 秒。起動中（`wait_healthy` 中・バックオフ中）の cancel は `Daemon::stop` だけなのでこれより短い。Kubo が落ちた直後の agent 待ち中の cancel も 15 秒 + ダッシュボードで短い。unmanaged は agent + ダッシュボード、セットアップモードはダッシュボードだけ。
- 数えている途中で、予算からはみ出す経路が 2 つ見つかったので塞いだ。
  - unmanaged は `agent::run_until` を上限なしで await していた（agent の起動中の `RelayClient::connect` などは cancel を見ない）。managed と同じ `AGENT_STOP_TIMEOUT`（15 秒）で打ち切るようにした。spawn せずに future を pin して `select!` し、上限を超えたら future を捨てる。
  - managed の起動時の `recover_orphan` は cancel を見ず、最悪 3 + 30 + 30 + 10 = 73 秒かかる。cancel と競争させ、cancel が先なら回収をやめて終わる。`kubo.pid` は残るので次の起動で回収し直せ、状態は起動前と変わらない。
- `DAEMON_STOP_GRACE` は 30 秒 → 20 秒にした。Kubo は RPC shutdown の後ふつう数秒で終わり、20 秒を超えても unix では SIGTERM 後にさらに 10 秒待つ（SIGTERM も Kubo にとってはグレースフルな停止）ので、Kubo が自分で終われる時間は RPC を含め最大 35 秒ある。これで managed の内側の最悪は 15 + (5 + 20 + 10) + 5 = 55 秒（Windows は SIGTERM 段が無いので 45 秒）。
- runtime の `shutdown_timeout`（10 秒）は、以前は watchdog の外にあった（watchdog は tokio のタスクで、runtime を畳むときに捨てられる）。watchdog を専用の OS スレッドにして、シグナルからプロセス終了までの全体を 1 つの猶予で抑えるようにした。猶予は `STOP_BUDGET`（55 秒）+ `RUNTIME_SHUTDOWN_TIMEOUT`（10 秒）+ 余裕 5 秒 = 70 秒（`up::FORCE_EXIT_GRACE`）。`cancel_on_signal(grace)` は猶予を引数で受け取り、`main.rs` がそれを渡す。`cancel_on_signal` を使うのは `swing up` だけなので、他のコマンド向けの既定は作っていない。
- サービスマネージャの上限は 90 秒（`service::STOP_TIMEOUT`）。systemd の `TimeoutStopSec` を 60 → 90 にし、launchd の plist にも `ExitTimeOut = 90` を足した。launchd は自分で止めるとき（`bootout` など）既定で 20 秒後に SIGKILL を送るので、同じ不整合が launchd 側にもあったため。90 秒は systemd の既定の `DefaultTimeoutStopSec` と同じで、猶予 70 秒に 20 秒の余裕がある。
- 2 回目のシグナルで即終了する。1 回目でグレースフルシャットダウンに入り、runtime が動いている間にもう一度 SIGINT/SIGTERM（Windows は Ctrl+C）を受けたら、`error!` を出して `exit(1)` する。Ctrl+C を 2 回押せばすぐ止まる、という CLI でよくある挙動に合わせた。Windows の親プロセスの終了は 1 回目としては扱うが、2 回目としては扱わない（ユーザーが急かしているわけではなく、Task Scheduler の `/End` が conhost を殺しただけなので）。1 回目かどうかの判定は `SignalWatch` が持つ `AtomicBool` で、シグナル監視と親の監視で共有する。
- `Signer::shutdown` と、`ensure_repo`・`apply_config` などが起動する短命の `ipfs` コマンドは上限を持たないまま予算に入れていない。前者は relay との接続を閉じるだけ、後者はローカルのリポジトリを触るだけで、詰まったときは watchdog が止める。

## やったこと

- `src/shutdown.rs`: 固定の `FORCE_EXIT_GRACE_PERIOD` を削除。`cancel_on_signal(grace) -> Result<SignalWatch>` にし、`SignalWatch`（`token()`、Windows のみ `cancel_when_parent_exits()`）を追加。watchdog を OS スレッドにし、シグナル監視をループにして 2 回目で即 `exit(1)`。unix の SIGINT は `ctrl_c()` ではなく `signal(SignalKind::interrupt())` で受け、Windows は `tokio::signal::windows::ctrl_c()` を使い続けて受ける。`RUNTIME_SHUTDOWN_TIMEOUT` を `main.rs` からここへ移した。
- `src/kubo.rs`: `Daemon::stop` の 5 秒・10 秒を `SHUTDOWN_RPC_TIMEOUT`・`SIGTERM_GRACE` にし、最悪時間を返す `daemon_stop_budget(grace)` を追加。
- `src/up.rs`: `DAEMON_STOP_GRACE` を 20 秒に。`STOP_BUDGET` と `FORCE_EXIT_GRACE` を定数から導出。unmanaged の agent 待ちに `AGENT_STOP_TIMEOUT`、`recover_orphan` に cancel との競争を追加。予算の関係を固定するテストを追加。
- `src/service.rs`: `STOP_TIMEOUT`（90 秒）を追加し、systemd unit の `TimeoutStopSec` と launchd plist の `ExitTimeOut` に使う。既存テストを更新。
- `src/main.rs`: `cancel_on_signal(up::FORCE_EXIT_GRACE)` を呼び、`--exit-with-parent` では `SignalWatch::cancel_when_parent_exits()` を呼ぶ。
- docs: `up.md`（shutdown 節、停止の時間予算の表、unmanaged・managed の流れ）、`kubo.md`（停止の定数と予算、孤児回収の cancel）、`service.md`（`TimeoutStopSec=90`、`ExitTimeOut`、systemd・launchd の stop の説明）、`architecture.md`（shutdown.rs の索引）。

## 検証

- `cargo fmt`、`cargo clippy -j 2 --workspace --all-targets -- -D warnings`、`cargo xwin clippy -j 2 --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings` を通した。
- `cargo test -j 2 --workspace --no-fail-fast`: 追加したテストを含めて通った。並行作業中の別変更のテストと、負荷時に 5 秒のタイムアウトに当たる `signer::tests::answers_from_a_signer_whose_clock_runs_behind_are_received` が 1 回ずつ落ちたが、どちらもこの変更とは関係せず、後者は単独で繰り返し実行して通ることを確かめた。
- セットアップモード（鍵なし、ダッシュボードのみ）で手で確かめた（実 Kubo は使っていない）:
  - SIGINT 1 回: `shutdown requested reason="SIGINT" grace_period=70s` を出して exit 0。
  - ダッシュボードにヘッダの途中で止まった HTTP 接続を張ったまま SIGTERM 1 回: ダッシュボードの停止待ち 5 秒で warn を出し、約 5 秒で exit 0（watchdog は発火しない）。
  - 同じ状態で SIGTERM の 1 秒後に SIGINT: `received another signal during graceful shutdown; exiting immediately signal="SIGINT"` を出して、1 回目から約 1 秒で exit 1。
