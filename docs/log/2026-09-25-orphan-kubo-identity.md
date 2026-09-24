# 2026-09-25 孤児 Kubo 回収の本人確認強化

[多重起動防止と孤児 Kubo 回収](2026-09-23-instance-lock-and-orphan-kubo.md)で入れた `recover_orphan` の見直し。architecture 側の結果は [`up.md`](../architecture/up.md) に反映済み（このログには経緯だけを書く）。

## きっかけ

これまでの `recover_orphan` は `kubo.pid` の PID が「生きていて、`ps`/`tasklist` の出力に `ipfs` を含む」ことだけを確認して SIGTERM/SIGKILL を送っていた。PID は OS が使い回すので、元の Kubo が死んだ後に別の（たまたま名前に `ipfs` を含む）同一ユーザーのプロセスが同じ PID を拾うと、swing が無関係のプロセスに kill 相当のシグナルを送ってしまう。

## 決めたこと（A+B）

- A: `kubo.pid` に PID と一緒に、そのとき選んだ Kubo の API ポートを記録する。回収時はまずこのポートに `POST /api/v0/shutdown` を短いタイムアウトで送ってみる。本物の Kubo でなければまず応答しないので、応答があってプロセスが実際に終了すれば、それだけで本人確認を兼ねたグレースフルな回収が完了する。
- B: API 経由で終わらなかったとき（応答なし、または応答はあったのに終了しなかった）だけ、強制終了の前に「記録した PID のプロセス開始時刻」と「今その PID を持っているプロセスの開始時刻」を突き合わせる。一致しなければ別プロセスだと判断し、**何も kill せず** `kubo.pid` を消すだけにする。一致して初めて SIGTERM/SIGKILL（unix）・`taskkill /T /F`（windows）に進む。
  - Linux: `/proc/<pid>/stat` の `starttime`（22 番目のフィールド）。`comm` にスペースや `)` が入り得るので、文字列全体の最後の `)` より後ろだけをフィールド分割してパースする。
  - macOS: `ps -o lstart= -p <pid>` の出力をそのまま識別子として使う（日時としてはパースしない。文字列が一致するかどうかだけを見る）。実機での動作確認はできていない（後述）。
  - Windows: `GetProcessTimes` の作成時刻（`FILETIME`）。`windows-sys` は既存の依存で、`OpenProcess`/`CloseHandle` は `shutdown.rs` の親プロセス監視で使っているパターンをそのまま踏襲した。新しい依存は増えていない。
- 既存のプロセス名チェック（`ps -o comm=`/`tasklist` に `ipfs` が含まれるか）は削除した。PID + 開始時刻の一致という、はるかに強い識別子に置き換わったので、名前の文字列一致を別立てで残す意味がない。
- `kubo.pid` のフォーマットは JSON（`{"pid", "api_port", "started_at"}`、`started_at` は上記の OS ごとの不透明な文字列）にした。以前の「PID の 10 進数だけ」という形式からの読み替え・互換処理は入れていない。新しい形式として JSON でパースできないファイルは、内容に関わらず「検証不能」として扱う（`warn!` を出してファイルを消すだけで、絶対に kill しない）。以前の形式で書かれたファイルもこの一般的な「パース失敗」経路を通るだけで、特別扱いはしていない。

## やったこと

- `src/kubo.rs`:
  - `PidRecord { pid, api_port, started_at }`（非公開）を追加。`write_pid_file(state_dir, pid, api_port)` は書き込み時に `process_start_marker(pid)` を呼んで `started_at` を埋める（取れなければ `Err`）。`read_pid_file` は非公開にし、JSON パース失敗をそのまま `Err` として返す。
  - `process_start_marker(pid) -> Option<String>` を OS ごとに実装（Linux は `/proc/<pid>/stat` + `parse_proc_stat_starttime`、macOS は `ps -o lstart=`、Windows は `GetProcessTimes`）。
  - `process_alive(pid) -> bool` を Windows にも追加（`OpenProcess` + `GetExitCodeProcess` で `STILL_ACTIVE` を見る）。これで `wait_for_exit` を OS 分岐なしの共通実装 1 本にまとめられた（以前は unix 版だけで、windows の `terminate_process` は `process_is_ipfs` で終了待ちしていた）。
  - `attempt_graceful_shutdown(api_port, pid)`: `POST /api/v0/shutdown`（タイムアウト 3 秒）→ 応答があれば `wait_for_exit(pid, 30s)`。
  - `recover_orphan`: `kubo.pid` が読めない→警告して削除して終了、読めれば API 経由のグレースフル停止を試し、だめなら開始時刻の一致を確認してから初めて `terminate_process` を呼ぶよう書き換えた。開始時刻が取れない（プロセスがもう無い）場合と、取れたが不一致の場合とでログの調子を分けた（前者はただの日常的な「もう死んでいた」、後者は「PID が再利用された」という異常系）。
  - `process_is_ipfs` は削除。
- `src/up.rs`: `write_pid_file` の呼び出しに、そのとき選んだ `api_port` を渡すよう変更しただけ。
- テスト（`src/kubo.rs` 内）:
  - `pid_file_round_trips`: 自分自身の PID で書いて読み直し、`started_at` が非空であることまで確認。
  - `read_pid_file_rejects_unparseable_content`、`recover_orphan_removes_unparseable_pid_file_without_killing_anything`: JSON として壊れたファイルはエラー扱い・kill せず削除。
  - `parses_starttime_from_proc_stat_with_simple_comm` / `_with_spaces_and_parens_in_comm` / `_is_none_without_a_closing_paren`: `/proc/pid/stat` のパースを純関数として直接テスト。
  - `recover_orphan_does_not_kill_on_start_time_mismatch`（unix のみ）: `sleep 5` を実際に spawn し、その PID に対してわざと不一致な `started_at` を書いた `kubo.pid` を用意して `recover_orphan` を呼び、プロセスが生き残ること（`process_alive`）を確認。テスト自身のプロセスではなく使い捨ての子プロセスを対象にしているので、ロジックにバグがあってもテストランナー自体を巻き込まない。
  - `recover_orphan_removes_stale_pid_file`: 存在しない PID を書いた場合の従来どおりの「ただの stale」経路。
  - `#[ignore]` の実 Kubo テストを 2 本に分割: `recover_orphan_shuts_down_a_leftover_daemon_via_its_api`（正しい `api_port` を記録 → API 経由のグレースフル停止で終わることを確認）、`recover_orphan_falls_back_to_signals_when_api_is_unreachable`(記録した `api_port` をわざと使われていない別のポートに書き換え → API 経由が失敗 → 開始時刻は一致するので SIGTERM/SIGKILL で終わることを確認)。共通のセットアップは `spawn_orphan` ヘルパーに切り出した。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test --workspace`（470 passed, 16 ignored）。
- `cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` を通した。実装中に `windows-sys` の `STILL_ACTIVE` が `Win32::System::Threading` ではなく `Win32::Foundation` にあり、型も `NTSTATUS`(`i32`) であることに気づいて import と cast を直した（ソースは `~/.cargo/registry/src/*/windows-sys-0.61.2/src/Windows/Win32/Foundation/mod.rs` で確認）。Windows 実機・実バイナリでの動作確認はできていない(このマシンでは実行できないため)。
- 過去のセッションで別作業用に取得済みだった実 Kubo 0.43.1 の Linux バイナリが手元に残っていたので、`SWING_TEST_KUBO_BIN` でこの回のために新規追加・更新した `#[ignore]` テスト 3 本（`full_lifecycle_against_real_kubo`・上記 2 本）をすべて通した。
- macOS 向けのコード（`ps -o lstart=` 呼び出し）はこのマシンでコンパイル・実行のどちらもできない。実装は他の macOS 分岐（`Daemon` の項目)と同程度に注意して書いたが、未検証であることを明記しておく。
