# swing service（service/）

[`../architecture.md`](../architecture.md) の一部。`swing up` そのものは [`up.md`](up.md)。サブコマンドの一覧は [`../architecture.md#cli`](../architecture.md#cli) を参照。

## 共通

- 登録する `swing` のコマンドは `<exe> up --config <config>`（`<exe>` は `current_exe()` の絶対パス。Windows ではこれを `conhost.exe` で包み、引数を足す。[下記](#windowsタスクスケジューラ)）。
- 設定ファイルは `config::resolve_config_path` で決め、そのパスにファイルが無ければ「service install needs a config file (swing.toml): pass --config or set SWING_CONFIG」でエラー（環境変数だけで動かす構成は非対応）。パスは `canonicalize` して絶対パスにする。
- 作業ディレクトリは設定ファイルの親ディレクトリ。設定ファイルに書いた相対パスと既定値は作業ディレクトリに関係なく設定ファイルのディレクトリから解決される（[`../architecture.md#設定と環境変数`](../architecture.md#設定と環境変数)）。
- `--system` は Linux でのみ有効で、他 OS で指定すると「--system is only supported on Linux」でエラー。`--no-start` は登録だけ行い起動しない（`install` のみ）。`--run-as <user>` は `install --system` でだけ使える（clap の `requires`。`service::install` も `--system` なしなら「--run-as is only valid with --system」でエラー）。launchd・タスクスケジューラへの登録はどれもログインユーザーのもので、システム全体への登録は無い。
- 生成する unit / plist / タスク XML / トレイの登録内容の文字列は `service/templates.rs` の純粋関数（`systemd_unit`・`launchd_plist`・`launchd_tray_plist`・`schtasks_xml`・`tray_run_command`）で作り、ユニットテストで検証している。以下の表は動作に効く値だけを挙げ、全文はこれらの関数が正本。埋め込むパス（実行ファイル・設定ファイル・作業ディレクトリ・ログ・トレイ）とユーザー名に制御文字（改行・タブ・NUL など `char::is_control`）が入っていれば、「<何> contains a control character and cannot be written into a service definition: ...」でエラーにし、何も書き出さない（改行で unit の行や XML の外へ抜けられないように）。
- OS 依存の実行部分（ファイル書き込み・`systemctl`/`launchctl`/`schtasks` の呼び出し）は `service/linux.rs`・`macos.rs`・`windows.rs` に置き、`mod.rs` は `cfg` でその 1 つを `platform` として選んで呼ぶ（どれも `install`・`uninstall`・`start`・`stop`・`is_installed`・`status` を同じ形で持つ）。対象 3 OS 以外では `unsupported.rs` が選ばれ、`install`/`uninstall`/`start`/`status`/`stop` すべて「service management is not supported on this OS」でエラーになる。外部コマンドの実行（`run_command`・`output_with_timeout`・出力の文字コード変換）は `service/process.rs`。
- `service::is_installed(system)` は `swing` 本体が登録済みかどうかを `Option<bool>` で返す（`Some(true)` 登録済み、`Some(false)` 未登録、`None` 分からない）。Linux は unit ファイル、macOS は plist の有無で決まり、`None` は返さない。Windows は下記「Windows」の `schtasks` による判定。CLI からは使わず、`swing-tray` が使う（[`tray.md`](tray.md)）。

## タスクトレイの自動起動（Windows と macOS）

`install` は、`swing` 実行ファイルと同じディレクトリにトレイ（Windows は `swing-tray.exe`、macOS は `SWING.app/Contents/MacOS/swing-tray`。`tray_exe_path`）があれば、ログイン時に `swing-tray --config <config>` を起動するよう登録する（[`tray.md`](tray.md)）。macOS では `SWING.app` の外にある素の `swing-tray` は見ない。見つからなければ「<そのパス> was not found next to ...」と出して、トレイの登録だけ飛ばす。`--no-start` でなければ、登録した直後にトレイも起動する。`--no-tray` を付けると登録せず、既に登録があれば消す（付けずに `install` し直した後で外すときのため）。`uninstall` は、先にトレイの登録を消してから（無ければ何もしない）本体を停止・削除する。Linux ではトレイを扱わず、`--no-tray` は何もしない。

| OS | 登録先 | 直後の起動 | `uninstall` |
|---|---|---|---|
| Windows | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` の値 `swing-tray`（`RegSetKeyValueW`）。中身は `tray_run_command` が作る `"<swing-tray.exe>" --config "<config>"`（`canonicalize` が付ける `\\?\` は外し、`\\?\UNC\` は `\\` に戻す） | `swing-tray.exe` を `CreateProcessW` でハンドルを継承させずに（`bInheritHandles = FALSE`）起動し、待たない。`std::process::Command` はハンドルを継承させるので、呼び出し元のパイプをトレイが開いたままにし、`swing service install` の出力をパイプで受けた側が終わらなくなる | 値を消す（`RegDeleteKeyValueW`。無ければ何もしない）。動いているトレイには触らない。トレイはタスク `swing` の削除を見つけて閉じる（[`tray.md`](tray.md#サービスの登録が消えたら終了する)）ので、`install --no-tray` で値だけを消したときは動き続ける |
| macOS | `~/Library/LaunchAgents/jp.ne.ama.swing-tray.plist`（`launchd_tray_plist`）。`ProgramArguments` は `SWING.app` の中の `swing-tray` を直接指す。`RunAtLoad = true`・`LimitLoadToSessionType = Aqua`・`ProcessType = Interactive`・`AssociatedBundleIdentifiers = [jp.ne.ama.swing]`、`KeepAlive` は無し | 先に `launchctl bootout gui/<uid>/jp.ne.ama.swing-tray`（失敗は無視）してから `launchctl bootstrap gui/<uid> <plist>` | plist があれば `bootout`（失敗は無視）して plist を消す。動いているトレイも止まる |

## Linux（systemd）

| 項目 | 値 |
|---|---|
| unit パス（user） | `$XDG_CONFIG_HOME/systemd/user/swing.service`（既定 `~/.config/systemd/user/swing.service`） |
| unit パス（`--system`） | `/etc/systemd/system/swing.service` |
| 実行ユーザー（`--system`） | `User=<name>`（`Group=` は付けず、そのユーザーの主グループになる）。下記「system unit の実行ユーザー」 |
| 制限（`--system`） | `NoNewPrivileges=yes`・`PrivateTmp=yes`・`ProtectSystem=full`（`/usr`・`/boot`・`/efi`・`/etc` を読み取り専用にする）・`ReadWritePaths="<workdir>"`（ダッシュボードの設定画面が設定ファイルを書き換えるので、設定ファイルのディレクトリは `/etc` の下でも書ける）。インストール時に設定を読み（`Config::load`。そのときの環境変数も効く）、`[agent].state_dir` と `[kubo].repo` が絶対パスで `<workdir>` の外にあれば、それぞれ `"-<path>"` として同じ行に足す（`-` は存在しなくても unit を失敗させない指定）。設定を読めなければ警告を出して `<workdir>` だけにする。`ProtectHome` は付けない（設定と `state_dir` をホームに置く構成が普通のため）。user unit には何も付けない |
| `ExecStart` | `<exe> up --config <config>` |
| `WorkingDirectory` | 設定ファイルの親ディレクトリ |
| 起動の順序と有効化 | `After=`・`Wants=network-online.target`、`WantedBy=default.target`（`--system` なら `multi-user.target`） |
| 再起動 | `Restart=on-failure`、`RestartSec=5` |
| 停止 | `KillSignal=SIGTERM`、`TimeoutStopSec=90`（`service::STOP_TIMEOUT`。`swing up` の強制終了までの猶予 70 秒より長い。[`up.md#停止の時間予算`](up.md#停止の時間予算)） |
| ログ | journal（`journalctl [--user] -u swing -f`） |

`ExecStart` の各パスは systemd の指定子（`%`）と環境変数の展開（`$`）が効かないよう引用・エスケープする（`quote_systemd_arg`）。`ReadWritePaths` は引用して `%`・`\`・`"` をエスケープし（`quote_systemd_path`）、`WorkingDirectory` と `User` は `%` だけをエスケープする。

- `install`: `--system` なら実行ユーザーを決め（下記）、unit を書き出し → `systemctl [--user] daemon-reload` → `systemctl [--user] enable [--now] swing`（`--no-start` なら `--now` を付けない）。`--system` でなければ続けて UID を明示して `loginctl enable-linger <uid>` を試み、失敗したら `` Warning: could not run `loginctl enable-linger`. ... `` を標準出力に出す（インストール自体は失敗にしない）。
- `start`: `systemctl [--user] start swing`。
- `uninstall`: `systemctl [--user] disable --now swing`（失敗は「未登録だったかもしれない」旨の注記のみ）→ unit ファイル削除 → `systemctl [--user] daemon-reload`。
- `stop`: `systemctl [--user] stop swing`。SIGTERM で停止シーケンス（[`up.md#shutdownshutdownrs`](up.md#shutdownshutdownrs)）に入り、登録は残る（次のログイン/`systemctl start swing` で再び動く）。停止シーケンスは最悪でも 70 秒の watchdog までに終わるので `TimeoutStopSec` の SIGKILL には届かない。`systemctl stop` による終了なので、watchdog や 2 回目のシグナルで終了コードが 1 になっても systemd は再起動しない。
- `status`: unit ファイルが無ければ `not installed` と出して終わる。あれば `systemctl [--user] status swing --no-pager` をそのまま実行し、標準入出力をそのまま引き継ぐ（終了コードは呼び出し元に伝播しない）。

### system unit の実行ユーザー

`--system` の unit は root では動かさない。`install --system` は次の順に実行ユーザーを決め、passwd（`getpwnam`・`getpwuid`）で引いた名前を `User=` に書く。引けなければ「no such user: <name>」「no user with uid <uid>」でエラー。

1. `--run-as <user>`（名前か数字の uid）。明示すれば `root` も指定できる。
2. 環境変数 `SUDO_UID`（`sudo swing service install --system` を実行したユーザー）。`0` のときは使わない。数字でなければエラー。
3. 実行している uid（root でなければ）。
4. どれでもなければ（root で直接実行し `--run-as` も無い）「refusing to register a system service that runs swing as root: ...」でエラーにし、unit を書かない。

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
| 停止の上限 | `ExitTimeOut = 90`（`service::STOP_TIMEOUT`。launchd が自分で止めるとき（`bootout` など）に SIGTERM から SIGKILL までを待つ秒数。既定の 20 秒では `swing up` の停止シーケンスが終わらないため） |
| ログ | `StandardOutPath`・`StandardErrorPath` とも `~/Library/Logs/swing.log` |
| `EnvironmentVariables.PATH` | `/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin`（Homebrew の bin を含める） |
| `AssociatedBundleIdentifiers` | `[jp.ne.ama.swing]`（`SWING.app` の `CFBundleIdentifier`）。ad-hoc 署名では効かず、「ログイン項目と機能拡張」では本体は `swing` のまま出る（トレイは実行ファイルが `SWING.app` の中にあるので `SWING` で出る）。バンドルとエージェントを同じ Team ID で署名したときに、本体も `SWING` に紐づけるために付けている |

パス・値は XML エスケープする。

- `install`: 既に `launchctl print gui/<uid>/jp.ne.ama.swing` が成功する（＝ロード済み）なら先に `bootout` してから、plist を書き出す。`--no-start` でなければ `launchctl bootstrap gui/<uid> <plist>` でロードする。
- `start`: ロード済み（`launchctl print` が成功する）なら `launchctl kickstart gui/<uid>/jp.ne.ama.swing`。ロードされていなければ `launchctl bootstrap gui/<uid> <plist>`（plist が無ければ「swing is not registered as a service」でエラー）。
- `uninstall`: `launchctl bootout gui/<uid>/jp.ne.ama.swing`（失敗は無視）→ plist ファイル削除。
- `stop`: `launchctl kill SIGTERM gui/<uid>/jp.ne.ama.swing`（bootout ではなくプロセスに直接 SIGTERM を送るだけ。停止シーケンスは [`up.md#shutdownshutdownrs`](up.md#shutdownshutdownrs)）。停止シーケンスは強制終了までの猶予（70 秒、[`up.md#停止の時間予算`](up.md#停止の時間予算)）の内に終わるように組んであり、終われば exit 0 なので、`KeepAlive` により止まったままになる（次のログインで `RunAtLoad` により再び起動する）。停止シーケンスが猶予を超えて watchdog に打ち切られたときと、停止中にもう一度 SIGINT/SIGTERM を送って即時終了させたときだけ exit 1 になり、launchd が再起動しうる。
- `status`: plist が無ければ `not installed`。あれば `launchctl print gui/<uid>/jp.ne.ama.swing` をそのまま実行。

## Windows（タスクスケジューラ）

| 項目 | 値 |
|---|---|
| タスク名 | `swing` |
| トリガー | 現在ユーザー（`USERDOMAIN\USERNAME`、ドメインが空ならユーザー名のみ）のログオン時（`LogonTrigger`） |
| 実行するセッション | `InteractiveToken`・`LeastPrivilege`。ログオン中のユーザーのセッションで動き、ログオフすると止まる |
| コマンド | `%SystemRoot%\System32\conhost.exe --headless "<exe>" up --config "<config>" --log-file "<log>" --exit-with-parent`。`conhost.exe --headless` でコンソールウィンドウを出さない。`--exit-with-parent` は親（conhost）の終了で Ctrl+C と同じグレースフルシャットダウンに入る（[`up.md`](up.md#shutdownshutdownrs)） |
| 作業ディレクトリ（`<workdir>`） | 設定ファイルの親ディレクトリ |
| 多重起動 | `MultipleInstancesPolicy = IgnoreNew` |
| 再起動 | 無い。swing のプロセスが終わったら次のログオンまで起動しない（Kubo・agent は `swing up` の中で起動し直す） |
| 実行時間・電源 | `ExecutionTimeLimit = PT0S`（無制限）。バッテリー駆動でも起動し、止めない |
| ログ | `<workdir>/swing.log`（`up --log-file <path>` で渡す。Linux・macOS は OS 側のログリダイレクトを使うので `service::install` はこのオプションを付けない） |

- `install`: XML を一時ファイルに書き、`schtasks /Create /TN swing /XML <tmpfile> /F` で登録してから一時ファイルを削除する。`schtasks` の出力は OEM コードページ（日本語環境では CP932）なので、失敗時の標準エラーと `status` の標準出力は UTF-8 として読めなければ OEM コードページとして変換して表示する。`--no-start` でなければ `schtasks /Run /TN swing` で即時起動する。
- `start`: `schtasks /Run /TN swing`。
- `uninstall`: トレイの登録を消した後、`stop`（下記）と同じグレースフルな停止を試みる。設定ファイルが見つからない・読めない・`stop::run` が失敗したときは `stop` と同じ `Warning: …` を出して続ける。続けて `schtasks /End /TN swing`（失敗は無視、既にグレースフルに止まっていれば no-op）→ `schtasks /Delete /TN swing /F`。
- `stop`: 設定ファイルを `resolve_config_path(None)`（`--config` は取らない。`SWING_CONFIG`、無ければカレントディレクトリの `swing.toml`）で探して `Config::load` し、`stop::run`（[`cli.md#stop`](cli.md#stop)）を 60 秒（`service::GRACEFUL_STOP_TIMEOUT`。`swing stop --timeout` の既定値と同じ定数）のタイムアウトで呼ぶ。`install` と違って実行ファイルのパスは解決しない。次のときは `Warning: …` を標準出力に出して `schtasks /End /TN swing` にフォールバックする（`/End` が終わらせるのは `conhost.exe` で、`swing` は `--exit-with-parent` で親の終了を検知してグレースフルに止まる）。
  - 設定ファイルが無い: `` Warning: could not find the config file (swing.toml) at <path> to stop swing through its dashboard; set SWING_CONFIG or run this from the directory containing swing.toml. Falling back to `schtasks /End`. ``
  - 設定ファイルを読めない（`Config::load` の失敗）: `` Warning: could not read the config file <path> to stop swing through its dashboard (<error>). Falling back to `schtasks /End`. ``
  - `stop::run` が失敗した: `` Warning: graceful stop failed (...); falling back to `schtasks /End`. ``

  タスクの登録自体は残る。素の `stop` も `/End` によるフォールバックも、次のログオン時トリガーまで再起動しない。
- 登録の判定（`is_installed` と `status`）: `schtasks` はどのエラーでも終了コード 1 を返し、メッセージはロケールで変わるので、終了コードやメッセージからは「タスクが無い」を見分けられない。そこで `schtasks /Query /TN swing` が成功すれば登録済み、失敗したら `schtasks /Query /FO CSV /NH` で全タスクを列挙し、その一覧取得が成功して先頭列に `"\swing"`（大文字小文字は区別しない。サブフォルダのタスクは含めない）が無いときだけ未登録とする。起動の失敗、一覧取得の失敗、10 秒のタイムアウト（超えたら `schtasks` を kill する）はすべて「分からない」（`is_installed` は `None`）。
- `status`: 上の判定で未登録なら `not installed`、分からなければエラー終了する。登録済みなら `schtasks /Query /TN swing /FO LIST /V`（10 秒のタイムアウト付き）の標準出力をそのまま表示し、失敗したらエラー終了する。

`schtasks` はすべて `CREATE_NO_WINDOW` を付けて起動する（`swing-tray`（[`tray.md`](tray.md)）から呼んでもコンソールウィンドウは開かない）。出力はパイプで受け取るので、CLI から呼んだときの表示は変わらない。
