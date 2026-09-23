# 2026-09-23 Windows 実機での初回起動確認

WSL から `cargo xwin build --release --target x86_64-pc-windows-msvc` で作った `swing.exe` を、Windows の実機で初めて動かした回。見つかった問題もこの回で直した。

## 確かめたこと

- `swing up` を端末から直接起動した（タスクスケジューラ経由ではない）。
- Kubo は `swing.exe` と同じフォルダに置いた `ipfs.exe` が選ばれて起動した（[`up.md`](../architecture/up.md) の探索順の 2 番目）。
- 起動時に Windows のファイアウォールのダイアログが出た。対話セッションで直接起動した場合は、ユーザーが許可するかどうか選べる。
- ダッシュボードをブラウザで開けた。
- 5 つの relay すべてに接続できた。ただし `sites=0` で Follow Set もまだ無く（`no follow set found yet`）、ミラー対象が無いので pin までは動いていない。
- Ctrl+C で止めた。swing は `shutdown requested signal="ctrl-c"` を出して agent を止め、relay を閉じた。Kubo も同じコンソールの Ctrl+C を直接受け取って自分で終了し始めた（`Received interrupt signal, shutting down...`）。強制終了（`graceful shutdown did not finish within the grace period`）のログは出ていない。
- `shutdown requested` が 2 回出た。`main.rs` の `swing up` ループが restart のたびに `up::run` → `shutdown::cancel_on_signal()` を呼び、前の回の Ctrl+C 監視タスクが残ったままになるため（ダッシュボードの初回設定などで 1 回 restart していればこの数になる）。

- `swing service install` が `schtasks /Create` の段階で失敗した。標準エラーは CP932 を UTF-8 として読んで文字化けしていたが、中身は「エラー: アクセスが拒否されました。」。

## 直したこと

- `shutdown requested` の重複: Ctrl+C の監視（`shutdown::cancel_on_signal`）を `main.rs` の `swing up` ループの外で 1 回だけ張り、`up::run` は引数でその子トークンを受け取るようにした。restart は子トークンだけを cancel するので、監視タスクは増えない。
- タスク登録の「アクセス拒否」: 原因はタスクの `LogonType` を `S4U` にしていたこと。S4U のタスクは登録に管理者への昇格が要り、「UAC を出さない」という[配布方式の設計](2026-09-21-distribution-design.md)の前提と両立していなかった。S4U を選んでいたのは `InteractiveToken` だとコンソールウィンドウが出るからだったので、次の 3 案から選んだ。
  - `InteractiveToken` + `conhost.exe --headless swing.exe up ...`（採用）: 昇格不要でウィンドウも出ない。ユーザーのセッションで動くので、S4U で心配していた「ファイアウォールのダイアログが出ず 4001 が黙って遮られる」問題もなくなる。`--headless` は conhost の非公開オプション（Windows 10 1903 以降）なのが弱み。
  - `InteractiveToken` で起動し、swing が自分で `FreeConsole` する: ログオン時に一瞬ウィンドウが出るので見送った。
  - S4U のまま、`service install` だけ管理者のターミナルで実行してもらう: UAC が出るうえ、非対話セッションのファイアウォール問題が残るので見送った。
  - ログオフすると止まるようになったが、トリガーがもともとログオン時なので実質の変化は小さい。
- `swing up --log-file` で起動が失敗したときに、ログファイルが空のまま終わっていた。`main` が返す `Err` は anyhow が標準エラーに出すだけで、タスク経由（`conhost --headless`）では標準エラーがどこにも残らない。`--log-file` 指定時は `main` の最後で `tracing::error!` にも流すようにした。最初の登録ではタスクの「前回の結果」が `0`・`swing.log` が空で、原因はポートの衝突だった（swing が `Err` で終わっても `0` と出ていたのは、次の項目のとおり conhost が子の exit code を返さないため）。
- タスクの `RestartOnFailure` を外した。実機で 2 つ確かめたところ、(1) `conhost.exe --headless cmd.exe /c "exit 3"` の exit code は 0（直接の `cmd.exe /c "exit 3"` は 3）で conhost は子の exit code を返さない、(2) conhost を挟まず `cmd.exe /c exit 1` を `RestartCount 3`・`RestartInterval 1 分` のタスクで動かしても、2 分半のあいだ再実行されなかった（`LastTaskResult` は 1）。`RestartOnFailure` はタスクの起動失敗にしか効かず、S4U で swing を直接起動していた頃から、落ちた swing は再起動されていなかったことになる。対策として、タスクから見張り役を起動して `swing up` を子として動かし非 0 なら起動し直す案と、数分ごとの時間トリガーを足す案（`service stop` で止めても起動し直してしまう）も考えたが、Kubo と agent は `swing up` の中で起動し直しているので、swing のプロセス自体が落ちたときは次のログオンまで待つことにした。
- `schtasks` の出力の文字化け: `service.rs` の `decode_output` で、UTF-8 として読めなければ OEM コードページ（`GetOEMCP` + `MultiByteToWideChar`）で変換するようにした。失敗時の標準エラーと `service status` の出力に使う。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`、`cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings`、`cargo xwin build --release --target x86_64-pc-windows-msvc` が通った。
- 変更後の `swing service install` を実機で実行し、管理者権限なしで登録・起動できた。タスクの状態は「実行中」（前回の結果 `267009` = `0x41301` `SCHED_S_TASK_RUNNING`）、`swing.log` に Kubo の起動・relay への接続まで出た。
- `$env:SWING_*` を PowerShell で設定してから `service install` しても、タスクで起動した swing には引き継がれない。ゲートウェイのポートなどは `swing.toml` に書く必要がある。
- タスク経由の起動では、ログオン時も含めてウィンドウは出なかった。ファイアウォールのダイアログも出なかったが、1 回目の直接起動で `ipfs.exe` を許可済みだった（ダイアログで作られるルールはプログラム単位）ためと考えられ、未許可の環境で出るかは確かめていない。

## まだ確かめていないこと

- Follow Set があるときにミラーの pin が最後まで終わること。

## 追記

- タスク経由で起動した swing を `swing service stop` で止め、swing と Kubo の両方が終了することを実機で確認した。
- `Stop-Process -Name swing -Force` で swing を強制終了すると、Kubo（`ipfs`）も Job Object の `KILL_ON_JOB_CLOSE` で道連れに終了することを実機で確認した。
- `schtasks /End /TN swing` は「正しく中断されました」と出るが、swing と Kubo は残った。`/End` が終わらせるのはタスクのプロセス（`conhost.exe --headless`）だけで、子の swing には何も届かない。S4U で swing を直接起動していた頃には無かった問題で、`service stop`・`uninstall` の予備（グレースフルな停止に失敗したときの `/End`）とタスクスケジューラの画面の「終了」が効かなくなっていた。
  - 対策に、`swing up` に Windows 専用の隠しオプション `--exit-with-parent` を足してタスクの引数に付けた。swing が親（conhost）を `PROCESS_SYNCHRONIZE` で開いて OS スレッドで終了を待ち、終わったら Ctrl+C と同じ経路でグレースフルに止まる。`/End` で Kubo も RPC シャットダウンを経て止まるようになる。
  - 見送った案: 予備の停止を `/End` ではなく `swing.lock` の PID への `taskkill /PID <pid> /T /F` に変える案。swing 自身のコマンドは直るが、画面の「終了」や手で打った `/End` では止まらないままになる。
  - 常に（端末から起動したときも）親を見張る案は採らなかった。端末から起動した swing はコンソールを閉じれば止まり、起動元が先に終わるランチャーなどから起動した場合に巻き込まれて止まるのを避けるため、タスク経由のときだけ付ける。
  - `--exit-with-parent` を付けたタスクを登録し直して `schtasks /End /TN swing` すると、swing と Kubo の両方が終了することを実機で確認した（`swing.log` の `signal="parent exited"` は見ていない）。
