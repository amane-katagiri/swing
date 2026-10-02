# swing service（service/）

[`../architecture.md`](../architecture.md) の一部。`swing up` そのものは [`up.md`](up.md)。サブコマンドとオプションは [`cli.md#service-install--uninstall--start--stop--status`](cli.md#service-install--uninstall--start--stop--status)。登録の持ち主の判定（`uninstall --only-from`・`status --points-into`）は子ページの [`service/ownership.md`](service/ownership.md)。

## 共通

- 登録する `swing` のコマンドは `<exe> up --config <config>`（`<exe>` は `current_exe()` を `std::path::absolute` で絶対パスにしたもの。シンボリックリンクは解決しない。Windows ではこれを `conhost.exe` で包み、引数を足す。[下記](#windowsタスクスケジューラ)）。
- 設定ファイルは `config::locate_config` で決める（[`config.md`](config.md#設定ファイルの場所)）。そのパスにファイルが無いときは決まり方で分かれる。
  - `--config`／`SWING_CONFIG` で指した: 「config file not found: <path>」でエラー。
  - ユーザーごとの既定の場所（`--system` でないとき）: ディレクトリ（Unix では `0700`）と空の設定ファイル（Unix では `0600`）を作り、`Created an empty config file at <path>` と表示して登録する。サービスはセットアップモード（[`up.md`](up.md#セットアップモード鍵未設定)）で起動する。
  - それ以外（`--system` での既定の場所、既定の場所を決められずカレントディレクトリになった）: 「service install needs a config file (swing.toml): pass --config or set SWING_CONFIG」でエラー。環境変数だけで動かす構成は非対応。
- 設定ファイルのパスは `canonicalize` する。実行ファイルのパスは `canonicalize` しないので、macOS で Homebrew の `opt` のようなシンボリックリンク経由で起動すると、そのパスが登録される（[`homebrew.md`](homebrew.md#パスと-brew-upgrade)）。Linux の `current_exe()` は解決済みのパスを返す。
- 作業ディレクトリは設定ファイルの親ディレクトリ。設定ファイルに書いた相対パスと既定値は作業ディレクトリに関係なく設定ファイルのディレクトリから解決される（[`config.md`](config.md)）。
- `--system` は Linux でのみ有効で、他 OS で指定すると「--system is only supported on Linux」でエラー。`--run-as <user>` は `install --system` でだけ使える。launchd・タスクスケジューラへの登録はどれもログインユーザーのもので、システム全体への登録は無い。
- 生成する unit / plist / タスク XML / トレイの登録内容は `service/templates.rs` の関数（`systemd_unit`・`launchd_plist`・`launchd_tray_plist`・`schtasks_xml`・`tray_run_command`）が正本で、以下の表は動作に効く値だけを挙げる。埋め込むパス（実行ファイル・設定ファイル・作業ディレクトリ・ログ・トレイ）とユーザー名に制御文字（`char::is_control`）があるか、パスが UTF-8 でなければエラーにし、何も書き出さない。
- OS ごとの実行部分は `service/linux.rs`・`macos.rs`・`windows.rs`、外部コマンドの実行は `service/process.rs`。対象 3 OS 以外では `unsupported.rs` が選ばれ、`is_installed` 以外の操作は「service management is not supported on this OS」でエラーになる（`is_installed` は未登録を返す）。
- `service::is_installed(system)` は本体が登録済みかを `Option<bool>`（`None` は分からない）で返す。Linux は unit ファイル、macOS は plist の有無。Windows は[下記](#windowsタスクスケジューラ)。`swing-tray` が使う（[`tray.md`](tray.md)）。
- 停止にかけるサービスマネージャの上限（`service::STOP_TIMEOUT`、90 秒）と `swing up` の停止の時間予算の関係は [`up.md#停止の時間予算`](up.md#停止の時間予算)。
- `swing stop --restart` やダッシュボードの再起動は、プロセスを終了させずに同じプロセス内で `swing up` をやり直す（[`up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）ので、サービスマネージャの再起動ポリシーは関わらない。

## タスクトレイの自動起動（Windows と macOS）

`install` は、`swing` 実行ファイルと同じディレクトリにトレイ（Windows は `swing-tray.exe`、macOS は `SWING.app/Contents/MacOS/swing-tray`。`tray_exe_path`）があれば、ログイン時に `swing-tray --config <config>` を起動するよう登録する（[`tray.md`](tray.md)）。

- 探す順は、`<exe>` の隣、`<exe>` を `canonicalize` した先の隣（`swing` だけをシンボリックリンクで PATH に置いた構成）。見つけたパスをそのまま登録する。
- macOS では `SWING.app` の外にある素の `swing-tray` は見ない。
- どちらの OS でも、見つからなければ「<そのパス> was not found next to ...」と出して、トレイの登録だけ飛ばす。
- `--no-start` でなければ、登録した直後にトレイも起動する。
- `--no-tray` を付けると登録せず、既に登録があれば消す。
- `uninstall` は、先にトレイの登録を消してから（無ければ何もしない）本体を停止・削除する。`--only-from` を付けたときは、トレイと本体を別々に判定して `<dir>` の下のものだけを消す（[`service/ownership.md`](service/ownership.md)）。
- Linux ではトレイを扱わず、`--no-tray` は何もしない。

| OS | 登録先 | 直後の起動 | 登録を消すとき |
|---|---|---|---|
| Windows | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` の値 `swing-tray`。中身は `"<swing-tray.exe>" --config "<config>"`（`canonicalize` が付ける `\\?\` は外し、`\\?\UNC\` は `\\` に戻す） | `CreateProcessW` でハンドルを継承させずに起動し、待たない | 値を消す（無ければ何もしない）。動いているトレイには触らない（トレイが閉じる条件は [`tray.md`](tray.md#サービスの登録が消えたら終了する)） |
| macOS | `~/Library/LaunchAgents/jp.ne.ama.swing-tray.plist`。`ProgramArguments` は `SWING.app` の中の `swing-tray` を直接指す。`RunAtLoad = true`・`LimitLoadToSessionType = Aqua`・`ProcessType = Interactive`・`AssociatedBundleIdentifiers = [jp.ne.ama.swing]`、`KeepAlive` は無し | 先に `launchctl bootout gui/<uid>/jp.ne.ama.swing-tray`（失敗は無視）してから `launchctl bootstrap gui/<uid> <plist>` | plist があれば `bootout`（失敗は無視）して plist を消す。動いているトレイも止まる |

## Linux（systemd）

| 項目 | 値 |
|---|---|
| unit パス（user） | `$XDG_CONFIG_HOME/systemd/user/swing.service`（既定 `~/.config/systemd/user/swing.service`） |
| unit パス（`--system`） | `/etc/systemd/system/swing.service` |
| 実行ユーザー（`--system`） | `User=<name>`（`Group=` は付けず、そのユーザーの主グループになる）。下記「system unit の実行ユーザー」 |
| 制限（`--system`） | `NoNewPrivileges=yes`・`PrivateTmp=yes`・`ProtectSystem=full`・`ReadWritePaths="<workdir>"`。インストール時に設定を読み（`Config::load`。そのときの環境変数も効く）、`[agent].state_dir` と `[kubo].repo` が絶対パスで `<workdir>` の外にあれば、それぞれ `"-<path>"` として同じ行に足す。設定を読めなければ警告を出して `<workdir>` だけにする。`ProtectHome` は付けない。user unit には何も付けない |
| `ExecStart` | `<exe> up --config <config>` |
| `WorkingDirectory` | 設定ファイルの親ディレクトリ |
| 起動の順序と有効化 | `After=`・`Wants=network-online.target`、`WantedBy=default.target`（`--system` なら `multi-user.target`） |
| 再起動 | `Restart=on-failure`、`RestartSec=5` |
| 停止 | `KillSignal=SIGTERM`、`TimeoutStopSec=90`（`service::STOP_TIMEOUT`） |
| ログ | journal（`journalctl [--user] -u swing -f`） |

unit に埋め込むパスとユーザー名は、systemd の指定子（`%`）と、`ExecStart` では環境変数の展開（`$`）も効かないようにエスケープする。

- `install`: `--system` なら実行ユーザーを決め（下記）、unit を書き出し → `systemctl [--user] daemon-reload` → `systemctl [--user] enable [--now] swing`（`--no-start` なら `--now` を付けない）。`--system` でなければ続けて `loginctl enable-linger <uid>` を試み、失敗したら `` Warning: could not run `loginctl enable-linger`. ... `` を標準出力に出す（インストール自体は失敗にしない）。
- `start`: `systemctl [--user] start swing`。
- `uninstall`: `systemctl [--user] disable --now swing`（失敗は「未登録だったかもしれない」旨の注記のみ）→ unit ファイル削除 → `systemctl [--user] daemon-reload`。
- `stop`: `systemctl [--user] stop swing`。SIGTERM で停止シーケンス（[`up.md#shutdownshutdownrs`](up.md#shutdownshutdownrs)）に入り、登録は残る。watchdog や 2 回目のシグナルで終了コードが 1 になっても systemd は再起動しない。
- `status`: unit ファイルが無ければ `not installed` と出して終わる。あれば `systemctl [--user] status swing --no-pager` をそのまま実行する（終了コードは呼び出し元に伝播しない）。

### system unit の実行ユーザー

`--system` の unit は root では動かさない。`install --system` は次の順に実行ユーザーを決め、passwd（`getpwnam`・`getpwuid`）で引いた名前を `User=` に書く。引けなければ「no such user: <name>」「no user with uid <uid>」でエラー。

1. `--run-as <user>`（名前か数字の uid）。明示すれば `root` も指定できる。
2. 環境変数 `SUDO_UID`。`0` のときは使わない。数字でなければエラー。
3. 実行している uid（root でなければ）。
4. どれでもなければ「refusing to register a system service that runs swing as root: ...」でエラーにし、unit を書かない。

決めたユーザーは `The service runs as user <name>.` と表示する。`swing up` はそのユーザー（`HOME` もそのユーザーのもの）で設定ファイルと `state_dir` を読み書きし、Kubo のバイナリを実行するので、設定ファイルと `state_dir` はそのユーザーが書ける場所に置く。

## macOS（launchd）

| 項目 | 値 |
|---|---|
| plist パス | `~/Library/LaunchAgents/jp.ne.ama.swing.plist` |
| `Label` | `jp.ne.ama.swing` |
| `ProgramArguments` | `[<exe>, "up", "--config", <config>]` |
| `WorkingDirectory` | 設定ファイルの親ディレクトリ |
| 起動 | `RunAtLoad = true`（ログイン時に起動） |
| 再起動 | `KeepAlive = { SuccessfulExit = false }`（0 以外で終わったときだけ再起動） |
| 停止の上限 | `ExitTimeOut = 90`（`service::STOP_TIMEOUT`。launchd が自分で止めるとき（`bootout` など）に SIGTERM から SIGKILL までを待つ秒数） |
| ログ | `StandardOutPath`・`StandardErrorPath` とも `~/Library/Logs/swing.log` |
| `EnvironmentVariables.PATH` | `/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin` |
| `AssociatedBundleIdentifiers` | `[jp.ne.ama.swing]`（`SWING.app` の `CFBundleIdentifier`）。ad-hoc 署名では効かず、「ログイン項目と機能拡張」では本体は `swing` のまま出る（トレイは `SWING` で出る） |

パス・値は XML エスケープする。

- `install`: 既に `launchctl print gui/<uid>/jp.ne.ama.swing` が成功する（ロード済み）なら先に `bootout` してから、plist を書き出す。`--no-start` でなければ `launchctl bootstrap gui/<uid> <plist>` でロードする。
- `start`: ロード済みなら `launchctl kickstart gui/<uid>/jp.ne.ama.swing`。ロードされていなければ `launchctl bootstrap gui/<uid> <plist>`（plist が無ければ「swing is not registered as a service」でエラー）。
- `uninstall`: `launchctl bootout gui/<uid>/jp.ne.ama.swing`（失敗は無視）→ plist ファイル削除。
- `stop`: `launchctl kill SIGTERM gui/<uid>/jp.ne.ama.swing`（プロセスに SIGTERM を送るだけ。停止シーケンスは [`up.md#shutdownshutdownrs`](up.md#shutdownshutdownrs)）。停止シーケンスが終われば exit 0 なので、`KeepAlive` により止まったままになる（次のログインで `RunAtLoad` により再び起動する）。watchdog に打ち切られたときと、停止中にもう一度 SIGINT/SIGTERM を送ったときだけ exit 1 になり、launchd が再起動しうる。
- `status`: plist が無ければ `not installed`。あれば `launchctl print gui/<uid>/jp.ne.ama.swing` をそのまま実行。

## Windows（タスクスケジューラ）

| 項目 | 値 |
|---|---|
| タスク名 | `swing` |
| トリガー | 現在ユーザー（`USERDOMAIN\USERNAME`、ドメインが空ならユーザー名のみ）のログオン時（`LogonTrigger`） |
| 実行するセッション | `InteractiveToken`・`LeastPrivilege`。ログオン中のユーザーのセッションで動き、ログオフすると止まる |
| コマンド | `%SystemRoot%\System32\conhost.exe --headless "<exe>" up --config "<config>" --log-file "<log>" --exit-with-parent`。コンソールウィンドウを出さない。`--exit-with-parent` は親（conhost）の終了で Ctrl+C と同じグレースフルシャットダウンに入る（[`up.md`](up.md#shutdownshutdownrs)） |
| 作業ディレクトリ（`<workdir>`） | 設定ファイルの親ディレクトリ |
| 多重起動 | `MultipleInstancesPolicy = IgnoreNew` |
| 起動し損ねたとき | `StartWhenAvailable = true`（予定の時刻に起動できなかったら、できるようになったときに起動する） |
| 表示 | `Hidden = true`（タスクスケジューラの一覧で隠れたタスクになる） |
| 再起動 | 無い。swing のプロセスが終わったら次のログオンまで起動しない（Kubo・agent は `swing up` の中で起動し直す） |
| 実行時間・電源 | `ExecutionTimeLimit = PT0S`（無制限）。バッテリー駆動でも起動し、止めない |
| ログ | `<workdir>/swing.log`（`up --log-file <path>` で渡す。Linux・macOS ではこのオプションを付けない） |
| OS のシャットダウン・ログオフ | タスクのプロセスに通知されず、`swing up` はグレースフルな停止を経ずに（Job Object に割り当てた Kubo ごと。[`kubo.md`](kubo.md#デーモンの起動kubodaemonspawn)）kill される |

- `install`: XML を一時ファイルに書き、`schtasks /Create /TN swing /XML <tmpfile> /F` で登録してから一時ファイルを削除する。`--no-start` でなければ `schtasks /Run /TN swing` で即時起動する。
- `start`: `schtasks /Run /TN swing`。
- `uninstall`: トレイの登録を消した後、`stop`（下記）と同じグレースフルな停止を試みる。失敗しても `stop` と同じ `Warning: …` を出して続ける。続けて `schtasks /End /TN swing`（失敗は無視）→ `schtasks /Delete /TN swing /F`。
- `stop`: 設定ファイルを `resolve_config_path(None)`（`--config` は取らない。`SWING_CONFIG`、カレントディレクトリの `swing.toml`、ユーザーごとの既定の場所の順。[`config.md`](config.md#設定ファイルの場所)）で探して `Config::load` し、`stop::run`（[`cli.md#stop`](cli.md#stop)）を 60 秒（`service::GRACEFUL_STOP_TIMEOUT`）のタイムアウトで呼ぶ。設定ファイルが無い・読めない・`stop::run` が失敗したときは、理由を `Warning: …` として標準出力に出し、`schtasks /End /TN swing` にフォールバックする（`conhost.exe` が終わり、`swing` は `--exit-with-parent` でグレースフルに止まる）。タスクの登録は残り、次のログオンまで起動しない。
- 登録の判定（`is_installed` と `status`）: `schtasks /Query /TN swing` が成功すれば登録済み。失敗したら `schtasks /Query /FO CSV /NH` で全タスクを列挙し、その一覧取得が成功して先頭列に `"\swing"`（大文字小文字は区別しない。サブフォルダのタスクは含めない）が無いときだけ未登録とする。起動の失敗、一覧取得の失敗、10 秒のタイムアウト（超えたら `schtasks` を kill する）はすべて「分からない」（`None`）。
- `status`: 上の判定で未登録なら `not installed`、分からなければエラー終了する。登録済みなら `schtasks /Query /TN swing /FO LIST /V`（10 秒のタイムアウト付き）の標準出力をそのまま表示し、失敗したらエラー終了する。
- `schtasks` はすべて `CREATE_NO_WINDOW` を付けて起動する。出力（失敗時の標準エラーと `status` の標準出力）は UTF-8 として読めなければ OEM コードページとして変換して表示する。
