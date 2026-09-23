# 2026-09-23 多重起動防止と孤児 Kubo 回収

[配布方式の実装](2026-09-23-distribution-implementation.md)の todo に残っていた 2 件（二重起動検出、孤児 Kubo の回収）を実装した回。architecture 側の結果は [`up.md`](../architecture/up.md)・[`cli.md`](../architecture/cli.md)・[`../architecture.md`](../architecture.md) に反映済み（このログには経緯だけを書く）。

## きっかけ

配布方式の実装ログの「未確認」節に残っていた 2 点:

- `swing up` の二重起動検出は Kubo の repo.lock と dashboard の bind 失敗に任せていて、明示的な検出が無い。
- Windows/macOS で swing が SIGKILL 相当で死んだときの子 Kubo の孤児化。孤児が repo.lock を持ったままだと次の `swing up` が起動ループしうる。

この 2 つはどちらも「同じ `state_dir`/`repo` に対して複数のプロセスが競合する」という同じ種類の問題なので、まとめて 1 回で片付けた。

## 決めたこと

- 多重起動の防止は advisory file lock（`std::fs::File::try_lock`、Rust 1.89 で安定化した std API）を `<state_dir>/swing.lock` に対して取る方式にした。OS が管理するロックなので、プロセスが `kill -9` で死んでもカーネルが自動的に外す。ロック取得に失敗したら、ファイルに書いてある相手の PID を読んでエラーメッセージに含める（読めなくても致命的にはしない）。ロックファイル自体は drop 時に消さない。消すと「A が解放 → B が消す前に C が同じパスを開いて取る → B が（C が取った後の）ファイルを消す」という取り合いのレースがあり得るため、ファイルは残したまま中身の上書きだけで次回の取得を回す。
- ロックは `up::run` と `agent::run`（CLI エントリの方。`run_until` ではなく）の冒頭で取る。`swing up` が `swing agent` の中身を内部で呼ぶときは `run_until` を直接呼ぶので二重にロックを取ろうとしない。
- 孤児 Kubo の回収は「乗っ取って使う」のではなく「殺して自分で新しく起動し直す」方式にした。Kubo の状態（動いているポート、内部状態）を外から正確に把握する手段が無く、`swing up` 側は常に自分が起動した Kubo の設定・ポートを前提にしているため、乗っ取るより殺して作り直す方が単純で確実。
- 孤児かどうかの判定は PID ファイル（`<state_dir>/kubo.pid`）+ プロセス名の確認にした。PID は再利用されるので、ファイルにある PID が「生きていて、かつ `ipfs` という名前を含むプロセスである」ことまで確認してから手を出す（`ps -p <pid> -o comm=` / windows は `tasklist`）。`recover_orphan` は `swing up` が[多重起動防止のロック](../architecture/up.md#多重起動の防止lockrs)を取った直後、Kubo を起動するより前に 1 回だけ呼ぶ（ロックが取れている時点で、この repo に対する Kubo がもしいればそれは自分より前の swing の孤児か無関係の別プロセスのどちらかでしかない、という前提に立てるため）。
- 終了させる手順は unix が SIGTERM → 30 秒待つ → SIGKILL → 10 秒待つ（既存の `Daemon::stop` の `DAEMON_STOP_GRACE = 30s` と揃えた）。Windows はグレースフルに終了させる標準的な方法が無い（自分が起動した子でなくコンソールも無いプロセスへ `CTRL_BREAK_EVENT` を送るのは別プロセスグループだと機能しない）ため `taskkill /PID <pid> /T /F` の強制終了 1 段階だけにした。
- Windows の子プロセス回収は Job Object にした。`CreateJobObjectW` + `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` + `AssignProcessToJobObject` で、swing プロセスが `TerminateProcess` 相当で消えたときに OS がハンドルを閉じ、道連れで Kubo も落ちるようにする。これが Linux の `PR_SET_PDEATHSIG` に一番近い Windows の機構。macOS にはこれに相当する仕組みが無く、次回起動時の `recover_orphan` に任せる（今回追加した孤児回収がそのまま macOS のセーフティネットにもなる）。
- repo lock の当て込みは、`recover_orphan` では拾えない「swing の管理下に無い Kubo（手動起動、別 swing 管理下など）」向けの補助として、Kubo の標準エラーに `"lock"` を含む行が出たかどうかを `Daemon` に覚えさせ、起動失敗時のログにヒントとして出すだけに留めた（自動では何もしない。相手が何のプロセスか分からない以上、自動で kill するのは孤児回収より一段危険なため）。

## やったこと

- `src/lock.rs`（新規）: `InstanceLock` と `acquire(state_dir)`。
- `src/up.rs`: `run` の冒頭でロックを取得。`run_managed` の起動時に `kubo::recover_orphan` を 1 回呼ぶ。`Daemon::spawn` 成功直後に `kubo.pid` を書き、`Daemon::stop` を呼ぶ全経路と Kubo が自壊した経路の両方で消す（`stop_daemon` ヘルパーに集約）。`wait_healthy` 失敗時、daemon が既に exit していて `saw_repo_lock_error()` が立っていれば warn を追加。
- `src/agent/lifecycle.rs`: `run`（CLI エントリ）の冒頭でロックを取得。
- `src/kubo.rs`: `write_pid_file`/`read_pid_file`/`remove_pid_file`、`recover_orphan`、プロセスの生死・`ipfs` 判定（`process_is_ipfs`）、終了処理（`terminate_process`）を追加。`Daemon` に `saw_repo_lock_error: Arc<AtomicBool>` を追加し、`forward_lines` が標準エラーの行を見て立てる。`cfg(windows)` で Job Object のラッパーモジュール（`windows_job`）を追加し、`Daemon::spawn`/`Drop` に組み込んだ。
- `Cargo.toml`: `[target.'cfg(windows)'.dependencies]` に `windows-sys = "0.61.2"`（features: `Win32_Foundation`, `Win32_Security`, `Win32_System_JobObjects`, `Win32_System_Threading`）を追加。`Win32_Security` は `CreateJobObjectW` の `SECURITY_ATTRIBUTES` 引数がこの feature 越しにしか出てこないため必要だった。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（ユニット、346 passed, 15 ignored）を通した。
- `kubo::tests` の `#[ignore]` 2 本を実 Kubo 0.43.1（`SWING_TEST_KUBO_BIN`）で通した:
  - `full_lifecycle_against_real_kubo`（既存）。
  - `recover_orphan_kills_a_leftover_daemon`（新規）: repo 初期化 → `Daemon::spawn` → `kubo.pid` を書く → `wait_healthy` → `std::mem::forget(daemon)` で孤児を模す → `recover_orphan` → API が応答しなくなり `kubo.pid` が消えていることを確認。
    - 実装中に見つけたバグ: unix の `kill(pid, 0)`/`kill(pid, SIGKILL)` が返す `ESRCH`（存在しないプロセス）を `io::Error::kind() == ErrorKind::NotFound` で判定していたが、実際には `ESRCH` は `ErrorKind::Uncategorized` にマップされ `NotFound` にならない（`std::io::Error::last_os_error().raw_os_error()` で確かめた）。この判定ミスのせいで「プロセスは既に死んでいるのに `process_alive` が true を返し続ける」→ `wait_for_exit` が常にタイムアウトする、という不具合があり、テストで実際に踏んで見つけた。`raw_os_error() == Some(libc::ESRCH)` の比較に直した。
    - `std::mem::forget` は tokio の `Child` の `Drop`（孤児キューへの登録）もスキップするため、殺した後にテストプロセス自身の下でゾンビのまま残ってしまう。本物の孤児回収では相手（元の swing）が既にいないので init/subreaper が自動的に reap するが、テストではそれが起きないので、`libc::waitpid` を別スレッドで回してテスト側が reap 役を肩代わりしている。
- スモークテスト（実 Kubo 0.43.1 + ローカル nostr-rs-relay、公開ネットワーク不使用。ポート 19080-19083、`docker/demo` の環境とは別ポートで衝突を避けた）:
  - `swing up` を起動 → `swing.lock` に自分の PID、`kubo.pid` に Kubo の PID（swing の子であることを `ps -o ppid` で確認）が書かれることを確認。
  - 同じ設定で 2 個目の `swing up` を起動 → `Error: another swing instance is already running on ./data (pid <1個目の PID>)` で即座（exit code 1）に終了することを確認。
  - 1 個目を `kill -9` → Linux の `PR_SET_PDEATHSIG` により Kubo も一緒に落ちることを確認（孤児回収を試すには別の方法が要る、という設計ログどおりの結果）。
  - 孤児回収の確認: `IPFS_PATH=<repo> ipfs daemon &` で手動 Kubo を起動し、その PID を `kubo.pid` に書いてから `swing up` を起動 → ログに `terminating orphaned Kubo left by a previous swing`（pid 付き）が出て、手動 Kubo が消え、新しい Kubo が別ポートで起動し、agent・gateway・dashboard も正常に立ち上がることを確認。
  - 最後に SIGTERM で `swing up` を止め、`kubo.pid` が消え `data/` 以下に他の取りこぼしが無いことを確認。relay コンテナを削除し、リポジトリ直下には何も残していない。
- Windows: `windows-sys` のソース（`~/.cargo/registry/src/*/windows-sys-0.61.2/src/Windows/Win32/System/JobObjects/mod.rs` ほか）で `CreateJobObjectW`/`SetInformationJobObject`/`AssignProcessToJobObject`/`CloseHandle` のシグネチャと `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` のフィールド名を直接確認した上で実装した。さらに、`windows-sys` + `tokio`（`process` feature）だけを使う最小クレートを作り、`cargo check --target x86_64-pc-windows-msvc`（このマシンには `x86_64-pc-windows-msvc` の rustup target が入っている）で型として通ることは確認した。ただし `swing` 本体は `ring` のクロスビルドが Windows SDK を要求して通らないため、実際にリンク・実行しての確認はできていない。`tokio::process::Child::raw_handle()`（`cfg(windows)` のみ公開される API、tokio 1.53.1 で確認）を使って Job Object に割り当てている。

### Windows 向けクロスビルド（追記）

`cargo-xwin`（Windows SDK と CRT を自動取得し `clang-cl` + `lld-link` で組む）を WSL に入れたところ、`ring` を含む `swing` 本体の Windows 向けビルドが通った。

```bash
cargo install cargo-xwin
brew install lld   # lld-link。clang-cl / llvm-lib / llvm-rc は Homebrew の llvm に含まれる
cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings
cargo xwin build --release --target x86_64-pc-windows-msvc
```

- 成果物は `<target-dir>/x86_64-pc-windows-msvc/release/swing.exe`（PE32+ console、約 16 MB）。
- Windows ターゲットでだけ出た clippy の指摘を 4 件直した: `shutdown.rs` の `Context` が unix 以外で未使用、`Daemon::stop` の `grace` が unix 以外で未使用、`Daemon.job` が読まれない（Drop のためだけに持つ）、`service.rs` の Windows 用 `current_user` の入れ子 `if`。
- `lock::tests::second_acquire_fails_with_first_pid_then_succeeds_after_drop` が他のテストと並走すると稀に落ちた。`flock` は fork した子が exec するまで共有されるため、別テストの子プロセス起動と重なると drop 直後の再取得が `WouldBlock` になる。テスト側で短い再試行を入れた（本番では 1 個目が生きている限り「already running」で正しいので実害は無い）。
- 実行（`swing up` の Windows での動作、Job Object、タスクスケジューラ）は引き続き未確認。macOS は SDK をライセンス上自動取得できないので同じ方法は使えない。

## 見送ったこと

- 孤児 Kubo を kill せず「乗っ取って再利用する」案は見送った。動いているポートは分かっても、その Kubo が今どんな `ipfs config` で動いているか保証できず、`apply_config` を上から当てても再起動なしには反映されない設定がある。殺して作り直す方が状態のズレを避けられる。
- repo lock のヒント（`saw_repo_lock_error`）を見て自動で何か（kill するなど）する案は見送った。`recover_orphan` は自分が `kubo.pid` に書いた PID にしか手を出さない。ヒント経由で見つかる「lock を握っている何か」の正体は分からない（手動起動、別ユーザーの swing、docker 等）ため、ログで知らせるだけに留めた。
- `swing.lock` の中身を PID 以外の情報（ホスト名、起動時刻等）まで持たせる案は見送った。今のところ「他のプロセスが取っている」ことと PID だけで足りる。
- macOS 向けの `launchd` の `KeepAlive`/独自の PDEATHSIG 相当の実装は見送った（設計ログの結論どおり）。macOS は次回起動時の `recover_orphan` に任せる。
