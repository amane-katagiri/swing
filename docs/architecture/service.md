# swing service（service.rs）

[`../architecture.md`](../architecture.md) の一部。`swing up` そのものは [`up.md`](up.md)。

```
swing service install   [--config <path>] [--system] [--no-start] [--no-tray]
swing service uninstall [--system]
swing service start     [--system]
swing service stop      [--system]
swing service status    [--system]
```

登録するコマンドは常に `<exe> up --config <config>`（`<exe>` は `current_exe()` の絶対パス）。設定ファイルは `config::resolve_config_path`（`pub`）で決め、見つからなければ「service install needs a config file (swing.toml): pass --config or set SWING_CONFIG」でエラー（環境変数だけで動かす構成は非対応）。パスは `canonicalize` して絶対パスにする。作業ディレクトリは設定ファイルの親ディレクトリ（相対な `state_dir = "./data"` がそのまま使えるように、登録するユニット/plist/タスクの working directory をそこに合わせる）。

`--system` は Linux でのみ有効。他 OS で指定するとエラー（「--system is only supported on Linux」）。`--no-start` は登録だけ行い起動しない（`install` のみ）。

## タスクトレイの自動起動（Windows と macOS）

`install` は、`swing` 実行ファイルと同じディレクトリに `swing-tray`（Windows は `swing-tray.exe`。`tray_exe_path`）があれば、ログイン時に `swing-tray --config <config>` を起動するよう登録する（[`tray.md`](tray.md)）。見つからなければ「swing-tray was not found next to ...」と出して、トレイの登録だけ飛ばす。`--no-start` でなければ、登録した直後にトレイも起動する。`--no-tray` を付けると登録せず、既に登録があれば消す（付けずに `install` し直した後で外すときのため）。`uninstall` は、トレイの登録もあれば消す。Linux ではトレイを扱わず、`--no-tray` は何もしない。

| OS | 登録先 | 直後の起動 | `uninstall` |
|---|---|---|---|
| Windows | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` の値 `swing-tray`（`RegSetKeyValueW`）。中身は `tray_run_command` が作る `"<swing-tray.exe>" --config "<config>"` | `swing-tray.exe` を子プロセスとして起動し、待たない | 値を消す（`RegDeleteKeyValueW`。無ければ何もしない）。動いているトレイは、登録が消えたのを自分で見つけて終わる（[`tray.md`](tray.md#サービスの登録が消えたら終了する)） |
| macOS | `~/Library/LaunchAgents/jp.ne.ama.swing-tray.plist`（`launchd_tray_plist`）。`RunAtLoad = true`・`LimitLoadToSessionType = Aqua`・`ProcessType = Interactive`、`KeepAlive` は無し | 先に `launchctl bootout gui/<uid>/jp.ne.ama.swing-tray`（失敗は無視）してから `launchctl bootstrap gui/<uid> <plist>` | `bootout`（失敗は無視）して plist を消す。動いているトレイも止まる |

- Windows でタスクスケジューラではなく Run キーにしているのは、タスクマネージャーの「スタートアップ アプリ」に出て、ユーザーが画面から無効にできるため。`swing-tray.exe` は GUI サブシステムなので、`swing up` のように `conhost --headless` を挟む必要も無い。
- Run キーに書くパスからは `\\?\` を外す（`canonicalize` が付ける。`\\?\UNC\` は `\\` に戻す）。Explorer は Run キーの値を `CreateProcess` で起動するが、`CreateProcess` がこの形式のパスを受け付けるとは書かれていないため。
- macOS で `KeepAlive` を付けないのは、トレイのメニューの「終了」で閉じたなら、次のログインまで出さないため。

生成する unit / plist / XML の文字列はそれぞれ純粋関数（`systemd_unit`・`launchd_plist`・`schtasks_xml`）で作り、ユニットテストで検証している。OS 依存の実行部分（ファイル書き込み・`systemctl`/`launchctl`/`schtasks` の呼び出し）だけ `cfg(target_os = ...)` で分岐し、対象 3 OS 以外では `install`/`uninstall`/`start`/`status`/`stop` すべて「service management is not supported on this OS」でエラーになる（コンパイル自体は全 OS で通る）。`service::uninstall`/`service::stop` は（Windows がグレースフルな停止で非同期処理を要するため）`async fn`（他は同期のまま）。

`service::is_installed(system)` は登録済みかどうかを返す（Linux は unit ファイル、macOS は plist の有無、Windows は `schtasks /Query /TN swing` が成功するか）。CLI からは使わず、`swing-tray` が「起動」を出すかどうかの判定に使う（[`tray.md`](tray.md)）。

## `swing stop`（`stop.rs`）

```
swing stop [--config <path>] [--restart] [--timeout <secs>, 既定 60]
```

`swing service stop` とは別に、`swing up` を OS のサービス登録に関わらず直接止められる CLI（[`up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）。動いている `swing up` のダッシュボード API を使う（手順は `stop::run`）:

1. `--restart` のときだけ、先に `GET /api/overview` で今の `instance` を読んでおく（接続できなければ `not running` で成功終了）。
2. `POST http://<[dashboard].listen>/api/shutdown`（`--restart` なら `/api/restart`）を `ApiClient`（`src/api_client.rs`）経由で叩く。API に接続できなければ（＝ `swing up` が動いていない）`not running` と出してすぐ成功終了する。2xx 以外のレスポンスはそのままエラーにする。
3. 呼び出しが通ったら `GET /api/overview` を 500ms 間隔でポーリングする。`--restart` なしなら、接続できなくなった時点（プロセスが終了した時点）で `stopped` と出して成功終了する。`--restart` なら、応答の `instance` が 1. で読んだ値と変わった時点（同じプロセスの中で `up::run` がやり直された時点）で `restarted` と出して成功終了する（再起動中にダッシュボードが閉じている時間はポーリング間隔より短いことが多いので、接続できなくなることは待たない）。`--timeout` 秒（既定 60）を超えたらエラー（「swing did not stop within N s」／「swing did not come back within N s」）。

`swing service stop`（Windows のみ）はこの `stop::run` をそのまま使う（上記「Windows（タスクスケジューラ）」参照）。

## Linux（systemd）

| 項目 | 値 |
|---|---|
| unit パス（user） | `$XDG_CONFIG_HOME/systemd/user/swing.service`（既定 `~/.config/systemd/user/swing.service`） |
| unit パス（`--system`） | `/etc/systemd/system/swing.service` |

unit の内容（`systemd_unit`）:

```ini
[Unit]
Description=SWING mirror agent
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=<exe> up --config <config>
WorkingDirectory=<workdir>
Restart=on-failure
RestartSec=5
KillSignal=SIGTERM
TimeoutStopSec=60

[Install]
WantedBy=default.target        # --system なら multi-user.target
```

`ExecStart` の各パスは二重引用符で囲み、`%` を `%%`（systemd の指定子）、`$` を `$$`（環境変数の展開）にし、`\`・`"` をエスケープする（`quote_systemd_arg`）。`WorkingDirectory` は引用符を受け付けない（「path is not absolute」で unit が読み込めなくなる）ので、囲まずに `%` だけ `%%` にする（空白はそのままでよい）。

- `install`: unit を書き出し → `systemctl [--user] daemon-reload` → `systemctl [--user] enable [--now] swing`（`--no-start` なら `--now` を付けない）。`--system` でなければ続けて `loginctl enable-linger <uid>` を試み（引数なしだと呼び出し元の logind セッションが対象になり、WSL のシェルなどセッションが無い環境では「No such device or address」で失敗するため UID を渡す）、失敗したら「ログアウト中も動かし続けるには自分で実行して」という warn を出す（インストール自体は失敗にしない）。
- `start`: `systemctl [--user] start swing`。
- `uninstall`: `systemctl [--user] disable --now swing`（失敗は「未登録だったかもしれない」旨の注記のみ）→ unit ファイル削除 → `systemctl [--user] daemon-reload`。
- `stop`: `systemctl [--user] stop swing`。unit の `ExecStart` はプロセスに SIGTERM を送る（`KillSignal=SIGTERM`。[`up.md`](up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit) の停止シーケンスに入る）のを `systemd` が待つだけで、登録は残る（`enable` はそのまま。次のログイン/`systemctl start swing`で再び動く）。`Restart=on-failure` なので、正常終了（exit code 0）扱いの `stop`（`systemctl stop` は SIGTERM 送出後 `TimeoutStopSec=60` まで待ってから `exit 0` で終わったとみなす）では自動再起動しない。
- `status`: unit ファイルが無ければ `not installed` と出して終わる。あれば `systemctl [--user] status swing --no-pager` をそのまま実行し、標準入出力をそのまま引き継ぐ（終了コードは呼び出し元に伝播しない）。
- ログは journal（`journalctl [--user] -u swing -f`）。

## macOS（launchd）

| 項目 | 値 |
|---|---|
| plist パス | `~/Library/LaunchAgents/jp.ne.ama.swing.plist` |
| ログ | `~/Library/Logs/swing.log`（stdout/stderr 共通） |
| `Label` | `jp.ne.ama.swing` |

`--system` は非対応（macOS には渡せない。`require_system_supported` が Linux 以外での `--system` を拒否する）。

plist の主なキー（`launchd_plist`）: `ProgramArguments` = `[<exe>, "up", "--config", <config>]`、`WorkingDirectory`、`RunAtLoad = true`、`KeepAlive = { SuccessfulExit = false }`（非 0 で終わったときだけ再起動する。systemd の `Restart=on-failure` と同じ意味）、`StandardOutPath`/`StandardErrorPath` = ログパス、`EnvironmentVariables.PATH = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"`（Homebrew の bin を含める）。パス・値は XML エスケープする。

- `install`: 既に `launchctl print gui/<uid>/jp.ne.ama.swing` が成功する（＝ロード済み）なら先に `bootout` してから、plist を書き出す。`--no-start` でなければ `launchctl bootstrap gui/<uid> <plist>` でロードする。
- `start`: ロード済み（`launchctl print` が成功する）なら `launchctl kickstart gui/<uid>/jp.ne.ama.swing`。ロードされていなければ `launchctl bootstrap gui/<uid> <plist>`（plist が無ければ「swing is not registered as a service」でエラー）。
- `uninstall`: `launchctl bootout gui/<uid>/jp.ne.ama.swing`（失敗は無視）→ plist ファイル削除。
- `stop`: `launchctl kill SIGTERM gui/<uid>/jp.ne.ama.swing`（bootout ではなくプロセスに直接 SIGTERM を送るだけ。[`up.md`](up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit) の停止シーケンスに入る）。`swing up` は exit 0 で終わるので `KeepAlive = { SuccessfulExit = false }` により止まったままになる（次のログインで `RunAtLoad` により再び起動する）。手で再開するなら `launchctl kickstart -k gui/<uid>/jp.ne.ama.swing`。ログインをまたいで止め続けるなら `launchctl disable gui/<uid>/jp.ne.ama.swing` してから `bootout`（戻すときは `enable` → `service install`）。
- `status`: plist が無ければ `not installed`。あれば `launchctl print gui/<uid>/jp.ne.ama.swing` をそのまま実行。
- `uid` は `libc::getuid()`。

## Windows（タスクスケジューラ）

| 項目 | 値 |
|---|---|
| タスク名 | `swing` |
| ログ | `<workdir>/swing.log`（`up --log-file <path>` で渡す。下記） |

`--system` は非対応。`sc.exe` のサービス登録は管理者権限と UAC が要るため使わず、タスクスケジューラの「ログオン時トリガー」で代替する（[配布方式の設計](../log/2026-09-21-distribution-design.md)）。systemd の `Restart=on-failure` や launchd の `KeepAlive` に当たる、落ちたプロセスの自動再起動は無い。タスクスケジューラの `RestartOnFailure` はタスクの起動に失敗したときだけ働き、起動後にプロセスが非 0 で終わっても再起動しないうえ、`conhost.exe --headless` は子の exit code を返さず常に 0 で終わるため（どちらも実機で確認。[Windows 実機での初回起動確認](../log/2026-09-23-windows-first-run.md)）。Kubo や agent が落ちた場合は `swing up` の中で起動し直す（[`up.md`](up.md)）ので、再起動されないのは swing のプロセス自体が終わったときだけで、その場合は次のログオンで起動する。

タスク XML（`schtasks_xml`。UTF-16 宣言だが本文は ASCII 範囲で問題ない）の主な設定:

- `<Triggers><LogonTrigger>`: 現在ユーザー（`USERDOMAIN\USERNAME`、ドメインが空ならユーザー名のみ）でログオン時に起動。
- `<Principal><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel>`: ログオン中のユーザーのセッションで動かす。`S4U` は登録に管理者への昇格が要る（非昇格の `schtasks /Create` が「アクセスが拒否されました」で失敗する）ため使わない。ユーザーのセッションで動くので、Kubo の 4001 inbound に対する Windows ファイアウォールのダイアログもユーザーに出る。ログオフすると止まる。
- `<Settings>`: `MultipleInstancesPolicy = IgnoreNew`、`StartWhenAvailable = true`、`ExecutionTimeLimit = PT0S`（無制限）、`DisallowStartIfOnBatteries = false`、`StopIfGoingOnBatteries = false`、`Hidden = true`。
- `<Actions><Exec>`: `Command` = `%SystemRoot%\System32\conhost.exe`、`Arguments` = `--headless "<exe>" up --config "<config>" --log-file "<log>" --exit-with-parent`、`WorkingDirectory` = `<workdir>`。`swing.exe` はコンソールサブシステムなので、`InteractiveToken` でそのまま起動するとコンソールウィンドウが開く。`conhost.exe --headless` 経由にしてウィンドウを出さない（`--headless` は Windows 10 1903 以降の conhost の非公開オプション）。タスクスケジューラの `schtasks /End`（画面の「終了」も同じ）が終わらせるのはタスクのプロセスである conhost だけで、子の swing は残る。そのため `--exit-with-parent` を付け、swing が親（conhost）の終了を待って、Ctrl+C を受けたときと同じグレースフルシャットダウンに入るようにしている（[`up.md`](up.md) の `shutdown.rs`）。

タスクの XML には環境変数を書けないため、ログ出力先は `up` のコマンドライン引数 `--log-file` で渡す（下記）。

- `install`: XML を一時ファイルに書き、`schtasks /Create /TN swing /XML <tmpfile> /F` で登録してから一時ファイルを削除する。`schtasks` の出力は OEM コードページ（日本語環境では CP932）なので、失敗時の標準エラーと `status` の標準出力は UTF-8 として読めなければ OEM コードページとして変換して表示する。`--no-start` でなければ `schtasks /Run /TN swing` で即時起動する。
- `start`: `schtasks /Run /TN swing`。
- `uninstall`: まず `stop`（下記）と同じグレースフルな停止を試みる（失敗しても無視して続ける）。続けて `schtasks /End /TN swing`（失敗は無視、既にグレースフルに止まっていれば no-op）→ `schtasks /Delete /TN swing /F`。
- `stop`: `swing stop`（[`up.md`](up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）と同じロジック（`stop::run`、上記「`swing stop`」）を、設定ファイルを `service.rs` の既存のパス解決（`resolve_service_paths`。`--config` は取らず、`install` と同じ規則で探す）で見つけて 60 秒のタイムアウトで呼ぶ（ダッシュボード API 経由）。失敗したら warn を出して `schtasks /End /TN swing`（強制終了）にフォールバックする。タスクの登録自体は残る。`swing stop --restart` はプロセスを終了させずに同じ PID のまま再起動する（[`up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）。素の `stop` も `/End` によるフォールバックも、次のログオン時トリガーまで再起動しない。
- `status`: `schtasks /Query /TN swing /FO LIST /V` を実行し、標準出力をそのまま表示する。失敗（未登録など）なら `not installed` と出す。

`schtasks` はすべて `CREATE_NO_WINDOW` を付けて起動する。GUI サブシステムの `swing-tray`（[`tray.md`](tray.md)）から呼ぶと、付けない場合は呼ぶたびにコンソールウィンドウが一瞬開くため。出力はパイプで受け取るので、CLI から呼んだときの表示は変わらない。

## `swing up --log-file <path>`

`Command::Up` の任意オプション。指定すると `main.rs` の `init_tracing` が `tracing-subscriber` の出力先をそのファイル（追記オープン、ANSI 無効）に切り替える。標準エラーには出なくなる。

- Linux（systemd）・macOS（launchd の `StandardOutPath`/`StandardErrorPath`）は OS 側がログをファイルにリダイレクトする仕組みを持つため、`service::install` はこのオプションを付けない。
- Windows だけ、`service::install` が生成するタスクの `Arguments` に `--log-file <workdir>/swing.log` を含める（タスクスケジューラにはログリダイレクトの仕組みが無いため）。
