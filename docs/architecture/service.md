# swing service（service/）

[`../architecture.md`](../architecture.md) の一部。`swing up` そのものは [`up.md`](up.md)。サブコマンドとオプションは [`cli.md#service-install--uninstall--start--stop--status`](cli.md#service-install--uninstall--start--stop--status)。登録の持ち主の判定（`uninstall --only-from`・`status --points-into`）は子ページの [`service/ownership.md`](service/ownership.md)。

## 共通

- 登録する `swing` のコマンドは `<exe> up --config <config>`。`<exe>` は `current_exe()` を `std::path::absolute` で絶対パスにしたもので、シンボリックリンクは解決しない（macOS で Homebrew の `opt` 経由で起動すると、そのパスが登録される。[`homebrew.md`](homebrew.md#パスと-brew-upgrade)）。Windows ではこれを `conhost.exe` で包む（[下記](#windowsタスクスケジューラ)）。
- 設定ファイルは `config::locate_config` で決め（[`config.md`](config.md#設定ファイルの場所)）、`canonicalize` して登録する。そのパスにファイルが無いときは決まり方で分かれる。
  - `--config`／`SWING_CONFIG` で指した: `config file not found: <path>` でエラー。
  - ユーザーごとの既定の場所（`--system` でないとき）: ディレクトリ（unix は 0700）と空の設定ファイル（unix は 0600）を作り、`Created an empty config file at <path>` と表示して登録する。サービスはセットアップモード（[`up.md`](up.md#セットアップモード鍵未設定)）で起動する。
  - それ以外（`--system` での既定の場所、既定の場所を決められないときのカレントディレクトリ）: `service install needs a config file (swing.toml): pass --config or set SWING_CONFIG` でエラー。環境変数だけで動かす構成は登録できない。
- 作業ディレクトリは設定ファイルの親ディレクトリ。
- `--system` は Linux でのみ有効で、他 OS では `--system is only supported on Linux` でエラー。launchd・タスクスケジューラへの登録はログインユーザーのもので、システム全体への登録は無い。
- unit／plist／タスク XML／トレイの登録内容は `service/templates.rs` の関数（`systemd_unit`・`launchd_plist`・`launchd_tray_plist`・`schtasks_xml`・`tray_run_command`）が正本で、以下の表は動作に効く値だけを挙げる。埋め込むパスとユーザー名に制御文字があるか、パスが UTF-8 でなければエラーにし、何も書き出さない。
- Linux・macOS・Windows 以外では、`is_installed` 以外の操作は `service management is not supported on this OS` でエラーになる（`is_installed` は未登録を返す）。
- `service::is_installed(system)` は本体が登録済みかを `Option<bool>`（`None` は分からない）で返す。Linux は unit ファイル、macOS は plist の有無、Windows は[下記](#windowsタスクスケジューラ)。`swing-tray` が使う（[`tray.md`](tray.md)）。
- サービスマネージャにかける停止の上限は `service::STOP_TIMEOUT`（90 秒）。`swing up` の停止の時間予算との関係は [`up.md#停止の時間予算`](up.md#停止の時間予算)。
- `swing stop --restart` やダッシュボードの再起動は同じプロセス内で `swing up` をやり直す（[`up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）ので、サービスマネージャの再起動ポリシーは関わらない。

## タスクトレイの自動起動（Windows と macOS）

`install` は、トレイ（Windows は `swing-tray.exe`、macOS は `SWING.app/Contents/MacOS/swing-tray`。`tray_exe_path`）が `<exe>` の隣か、`<exe>` を `canonicalize` した先の隣（`swing` だけをシンボリックリンクで PATH に置いた構成）にあれば、見つけたパスをログイン時に `swing-tray --config <config>` で起動するよう登録する（[`tray.md`](tray.md)）。

- 見つからなければ `<そのパス> was not found next to ...` と出して、トレイの登録だけ飛ばす。macOS では `SWING.app` の外にある素の `swing-tray` は見ない。
- `--no-start` でなければ、登録した直後にトレイも起動する。
- `--no-tray` を付けると登録せず、既に登録があれば消す。
- `uninstall` は、先にトレイの登録を消してから（無ければ何もしない）本体を停止・削除する。
- Linux ではトレイを扱わず、`--no-tray` は何もしない。

| OS | 登録先 | 直後の起動 | 登録を消すとき |
|---|---|---|---|
| Windows | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` の値 `swing-tray`。中身は `"<swing-tray.exe>" --config "<config>"`（`\\?\` は外し、`\\?\UNC\` は `\\` に戻す） | `CreateProcessW` でハンドルを継承させずに起動し、待たない | 値を消す。動いているトレイには触らない（トレイが閉じる条件は [`tray.md`](tray.md#サービスの登録が消えたら終了する)） |
| macOS | `~/Library/LaunchAgents/jp.ne.ama.swing-tray.plist`。`ProgramArguments` は `SWING.app` の中の `swing-tray` を直接指す。`RunAtLoad = true`・`LimitLoadToSessionType = Aqua`・`ProcessType = Interactive`・`AssociatedBundleIdentifiers = [jp.ne.ama.swing]`、`KeepAlive` は無し | plist を書いた後に `launchctl bootout gui/<uid>/jp.ne.ama.swing-tray`（失敗は無視）し、続けて `launchctl bootstrap gui/<uid> <plist>` | `bootout`（失敗は無視）して plist を消す。動いているトレイも止まる |

## Linux（systemd）

| 項目 | 値 |
|---|---|
| unit パス（user） | `$XDG_CONFIG_HOME/systemd/user/swing.service`（既定 `~/.config/systemd/user/swing.service`） |
| unit パス（`--system`） | `/etc/systemd/system/swing.service` |
| 実行ユーザー（`--system`） | `User=<name>`（`Group=` は付けない）。下記「[system unit の実行ユーザー](#system-unit-の実行ユーザー)」 |
| 制限（`--system`） | `NoNewPrivileges=yes`・`PrivateTmp=yes`・`ProtectSystem=full`・`ReadWritePaths="<workdir>"`。インストール時に設定を読み（`Config::load`。そのときの環境変数も効く）、`[agent].state_dir` と `[kubo].repo` が絶対パスで `<workdir>` の外にあれば `"-<path>"` として同じ行に足す。設定を読めなければ警告を出して `<workdir>` だけにする。`ProtectHome` は付けない。user unit には何も付けない |
| `ExecStart` | `<exe> up --config <config>` |
| `WorkingDirectory` | 設定ファイルの親ディレクトリ |
| 起動の順序と有効化 | `After=`・`Wants=network-online.target`、`WantedBy=default.target`（`--system` なら `multi-user.target`） |
| 再起動 | `Restart=on-failure`、`RestartSec=5` |
| 停止 | `KillSignal=SIGTERM`、`TimeoutStopSec=90`（`service::STOP_TIMEOUT`） |
| ログ | journal（`journalctl [--user] -u swing -f`） |

unit に埋め込むパスとユーザー名は、systemd の指定子（`%`）と、`ExecStart` では環境変数の展開（`$`）も効かないようにエスケープする。

- `install`: `--system` なら実行ユーザーを決めて検査し（下記）、unit を書き出し（同じディレクトリの一時ファイルに書いて flush してから rename する。plist も同じ）→ `systemctl [--user] daemon-reload` → `systemctl [--user] enable [--now] swing`（`--no-start` なら `--now` なし）。`--system` でなければ続けて `loginctl enable-linger <uid>` を試み、失敗したら `` Warning: could not run `loginctl enable-linger`. ... `` を出す（インストールは失敗にしない）。
- `start`: `systemctl [--user] start swing`。
- `stop`: `systemctl [--user] stop swing`。SIGTERM で停止シーケンス（[`up.md#shutdownshutdownrs`](up.md#shutdownshutdownrs)）に入り、登録は残る。watchdog や 2 回目のシグナルで終了コードが 1 になっても、systemd の停止操作なので再起動しない。
- `uninstall`: `systemctl [--user] disable --now swing`（失敗は注記を出すだけ）→ unit ファイル削除 → `systemctl [--user] daemon-reload`。
- `status`: unit ファイルが無ければ `not installed`。あれば `systemctl [--user] status swing --no-pager` をそのまま実行する（終了コードは伝えない）。

### system unit の実行ユーザー

`--system` の unit は root では動かさない。`install --system` は次の順に実行ユーザーを決め、passwd（`getpwnam`・`getpwuid`）で引いた名前を `User=` に書く。引けなければ `no such user: <name>`・`no user with uid <uid>` でエラー。

1. `--run-as <user>`（名前か数字の uid）。引いた uid が 0 なら、`--allow-root` も付けないとエラー。
2. 環境変数 `SUDO_UID`。`0` のときは使わない。数字でなければエラー。
3. 実行している uid（root でなければ）。
4. どれでもなければ `refusing to register a system service that runs swing as root: ...` でエラーにし、unit を書かない。

続けて、実行ユーザー以外が unit の動かすものを差し替えられないことを確かめる。対象は実行ファイル（登録するパスと、シンボリックリンクを解決したパス）・設定ファイル・作業ディレクトリと、設定を読めて `[kubo].managed` が true なら Kubo のバイナリ（`[kubo].binary`、無ければ `kubo::locate_binary` が見つけたもの。見つからなければ確かめない）。それぞれ自身と祖先のディレクトリすべてが次をみたさなければ、問題のあるパスを並べた `refusing to register a system service that runs as <name>: ...` でエラーにし、unit を書かない。

- 所有者が root か実行ユーザー（シンボリックリンクそのものは所有者だけを見る）。
- その他のユーザーが書き込めない。グループの書き込みは、そのグループが所有者の個人グループ（名前が所有者のユーザー名と同じで、ほかのメンバーがいない）のときだけ許す。持ち主が root でも許さないので、Debian の `root:staff 2775` の `/usr/local` も断る。
- root が持つ sticky ビット付きの祖先のディレクトリ（`/tmp` など）は上の 2 つを問わない。ただし確かめる対象そのものと、実行ファイル・Kubo のバイナリを直接収めるディレクトリ（書かれたパスと解決したパスのそれぞれの親）にはこの例外を当てはめない。
- 調べられない（存在しないなど）ものもエラーに含める。

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

- `install`: plist を書き出してから、ロード済み（`launchctl print gui/<uid>/jp.ne.ama.swing` が成功する）なら `bootout` し、`--no-start` でなければ `launchctl bootstrap gui/<uid> <plist>` でロードする。
- `start`: ロード済みなら `launchctl kickstart gui/<uid>/jp.ne.ama.swing`、ロードされていなければ `launchctl bootstrap gui/<uid> <plist>`（plist が無ければ `swing is not registered as a service` でエラー）。
- `stop`: `launchctl kill SIGTERM gui/<uid>/jp.ne.ama.swing`（停止シーケンスは [`up.md#shutdownshutdownrs`](up.md#shutdownshutdownrs)）。停止シーケンスが終われば exit 0 なので `KeepAlive` により止まったままになり、次のログインで再び起動する。watchdog に打ち切られたときと、停止中にもう一度 SIGINT/SIGTERM を受けたときだけ exit 1 になり、launchd が再起動しうる。
- `uninstall`: `launchctl bootout gui/<uid>/jp.ne.ama.swing`（失敗は無視）→ plist ファイル削除。
- `status`: plist が無ければ `not installed`。あれば `launchctl print gui/<uid>/jp.ne.ama.swing` をそのまま実行する。

## Windows（タスクスケジューラ）

| 項目 | 値 |
|---|---|
| タスク名 | `swing` |
| トリガー | 現在ユーザー（`USERDOMAIN\USERNAME`、ドメインが空ならユーザー名のみ）のログオン時（`LogonTrigger`） |
| 実行するセッション | `InteractiveToken`・`LeastPrivilege`。ログオン中のユーザーのセッションで動き、ログオフすると止まる |
| コマンド | `%SystemRoot%\System32\conhost.exe --headless "<exe>" up --config "<config>" --log-file "<log>" --exit-with-parent`。コンソールウィンドウを出さない。`--exit-with-parent` は親（conhost）の終了で Ctrl+C と同じグレースフルな停止に入る（[`up.md`](up.md#shutdownshutdownrs)） |
| 作業ディレクトリ（`<workdir>`） | 設定ファイルの親ディレクトリ |
| 多重起動 | `MultipleInstancesPolicy = IgnoreNew` |
| 起動し損ねたとき | `StartWhenAvailable = true` |
| 表示 | `Hidden = true` |
| 再起動 | 無い。swing のプロセスが終わったら次のログオンまで起動しない（Kubo・agent は `swing up` の中で起動し直す） |
| 実行時間・電源 | `ExecutionTimeLimit = PT0S`（無制限）。バッテリー駆動でも起動し、止めない |
| ログ | `<workdir>/swing.log`（`up --log-file <path>` で渡す） |
| OS のシャットダウン・ログオフ | タスクのプロセスに通知されず、`swing up` はグレースフルな停止を経ずに（Job Object に割り当てた Kubo ごと。[`kubo/daemon.md`](kubo/daemon.md#デーモンの起動kubodaemonspawn)）kill される |

- `install`: XML を UTF-16LE（BOM 付き。XML 宣言の `encoding="UTF-16"` に合わせる）で一時ディレクトリの `swing-task-<乱数>.xml` に本人だけが読める形で書き、`schtasks /Create /TN swing /XML <tmpfile> /F` で登録してから一時ファイルを消す。`--no-start` でなければ `schtasks /Run /TN swing` で起動する。
- `start`: `schtasks /Run /TN swing`。
- `stop`: 設定ファイルを `resolve_config_path(None)`（`--config` は取らない。`SWING_CONFIG`、ユーザーごとの既定の場所の順）で探して読み、`stop::run`（[`cli.md#stop`](cli.md#stop)）を 60 秒（`service::GRACEFUL_STOP_TIMEOUT`）のタイムアウトで呼ぶ。設定ファイルが無い・読めない・`stop::run` が失敗したときは、理由を `Warning: …` として出し、`schtasks /End /TN swing` にフォールバックする（`conhost.exe` が終わり、`swing` は `--exit-with-parent` でグレースフルに止まる）。タスクの登録は残る。
- `uninstall`: トレイの登録を消した後、`stop` と同じグレースフルな停止を試み（失敗しても `Warning: …` を出して続ける）、`schtasks /End /TN swing`（失敗は無視）→ `schtasks /Delete /TN swing /F`。
- 登録の判定（`is_installed` と `status`）: `schtasks /Query /TN swing` が成功すれば登録済み。失敗したら `schtasks /Query /FO CSV /NH` で全タスクを列挙し、それが成功して先頭列に `"\swing"`（大文字小文字は区別しない。サブフォルダのタスクは含めない）が無いときだけ未登録とする。どちらかの起動の失敗やタイムアウトは「分からない」（`None`）。
- `status`: 未登録なら `not installed`、分からなければエラー終了する。登録済みなら `schtasks /Query /TN swing /FO LIST /V` の標準出力をそのまま表示する。
- `schtasks` は `%SystemRoot%\System32\schtasks.exe` を絶対パスで（`SystemRoot` が無ければ `C:\Windows`）、`CREATE_NO_WINDOW` を付けて起動する。タイムアウトは登録の判定・`status`・`uninstall` の `/End` が 10 秒、`/Create`・`/Run`・`stop` の `/End`・`/Delete` が 60 秒で、超えたら kill してエラーにする。出力は UTF-8 として読めなければ OEM コードページとして変換する。
