# swing up（up.rs, ports.rs, shutdown.rs, lock.rs）

[`../architecture.md`](../architecture.md) の一部。設定キーは [`config.md`](config.md)、内蔵 gateway は [`gateway.md`](gateway.md)、Kubo プロセスの管理は [`kubo.md`](kubo.md)、OS への常駐登録は [`service.md`](service.md)、コンテナでの起動は [`docker.md`](docker.md)。

`swing up`（`up::run(config, token, port_shift)`）は次の順で起動する。`up::run` は呼ばれるたびにこの順で最初からやり直す。

1. `swing.lock` の取得（[多重起動の防止](#多重起動の防止lockrs)）
2. 終了要求（`ExitRequest`、[下記](#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）の作成
3. `<state_dir>/upload/` の掃除（`dashboard::cleanup_upload_dir`）
4. `signer::Signer::load` で署名の方法を決める（[`signer.md`](signer.md#signer)。無ければ下記「セットアップモード」）
5. ダッシュボードの `TcpListener::bind`（`up::bind_dashboard`。セットアップモードではポートをずらすことがある）
6. ダッシュボードのトークン（`<state_dir>/dashboard.token`）の読み込み・作成
7. ダッシュボードの `AppState` 作成・`dashboard::serve` の起動。bind した時点で応答を始める（agent の準備が整うまで 503 を返す API は [`dashboard/http-api.md#共通`](dashboard/http-api.md#共通)）
8. リソース使用量の測定（`stats::run`、[`stats.md`](stats.md)）の起動
9. 鍵の有無・`[kubo].managed` に応じた Kubo / agent の起動ループ（[下記](#swing-up-のループuprs)）
10. 終わったら測定とダッシュボードを止め、署名アプリとの接続を閉じる（`Signer::shutdown`）。ダッシュボードの API サーバは agent の終了では止まらず（[`agent.md#シグナルと終了`](agent.md#シグナルと終了)）、ここで止まるのを待つ（上限は [停止の時間予算](#停止の時間予算)）。超えたら warn を出して待つのをやめる。

## セットアップモード（鍵未設定）

秘密鍵（`[nostr].secret_key` / `SWING_NOSTR_SECRET_KEY`）も `<state_dir>/remote-signer.json` も無く、`signer::Signer::load` が `None` を返すと（読み込み規則は [`signer.md`](signer.md)）、`up::run` は Kubo も agent も起動せず、ダッシュボードとリソース使用量の測定だけを動かして `token.cancelled()` を待つ（[`dashboard.md`](dashboard.md#セットアップモードと-appstatesetup_mode)）。このモードでの API の応答は [`dashboard/http-api.md#共通`](dashboard/http-api.md#共通)。

セットアップモードを抜ける経路は 2 つある。

- `POST /api/setup`（[`dashboard/http-api/config.md#post-apisetup`](dashboard/http-api/config.md#post-apisetup)）: 鍵（または署名アプリの接続情報）と初期設定を書き込んだ後、プロセス内再起動（[下記](#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）をスケジュールする。
- `swing signer pair`（[`cli.md#signer-pair`](cli.md#signer-pair)）: `remote-signer.json` だけを書く。次に `up::run` が始まったとき（`swing stop --restart` などによる再起動）に `Signer::load` がそれを読んでセットアップモードを抜ける。

`swing up` は鍵が無くても、`swing.toml` が無くても起動できる（設定ファイルの探索順は [`config.md`](config.md#設定ファイルの場所)）。セットアップモードに入るときのログ（`running in setup mode`）に、書き込み先の設定ファイルのパスを `config` として出す。

### セットアップモードでのポートの調整

セットアップモードの間は、ダッシュボードと管理下の Kubo の gateway のポートを、空いているものにずらして設定ファイルに書き込む。

- ずらさない条件: `swing up --no-port-shift`（`SWING_NO_PORT_SHIFT`。`up::run` の `port_shift = false`。Docker イメージの既定は [`docker.md#dockerfile`](docker.md#dockerfile)）のときは両方ともずらさず、書き込みもしない。値が環境変数由来のキー（`ports::may_shift`）は、キーごとにずらさず書き込みもしない。設定ファイルに書いてある値はずらす対象に含める。
- ずらし方（`ports::bind_shifting`）: 設定のアドレスから始めて、同じ IP でポートを 1 ずつ上げながら最大 20 個（`ports::PROBE_COUNT`）先まで bind を試し、それでも駄目ならポート 0（OS が選ぶ空きポート）で bind する。次の候補に進むのは bind が `AddrInUse` か `PermissionDenied`（Windows の除外ポート範囲）で失敗したときだけで、それ以外の失敗はそのままエラーにする。
- ダッシュボード: この手順で bind し、ずらしたときは warn（`dashboard port is in use; listening on another port`）を出す。
- Kubo の gateway: `[kubo].managed = true` なら、`[kubo].gateway_listen` から同じ手順で空いているアドレスを探す（`ports::free_addr`。bind してすぐ閉じる）。ずらしたときは warn（`Kubo gateway port is in use; using another port`）を出す。探すこと自体に失敗したら warn を出して、このキーは書き込まない。
- 対象になったアドレスは、ずらしたかどうかに関わらず `settings::pin_addrs` で 1 回で書き込み、書き込んだ後の設定を読み直して `AppState` を作る。`[dashboard].gateway` が既定値のままなら、そのリンク先のポートもずらした Kubo の gateway のポートになる（[`dashboard.md#設定dashboard`](dashboard.md#設定dashboard)）。書き込みに失敗したら warn を出し、メモリ上の設定のダッシュボードのアドレスだけを bind したものに直して続ける。
- セットアップを終える経路（`POST /api/setup` と `swing signer pair`）は、どちらもこのとき書き込んだ値をそのまま引き継ぐ。セットアップモードで起動するたびにやり直すので、前回書き込んだポートが使われていても、セットアップを終えるまではまたずらせる。

セットアップモードでないときはずらさない。ダッシュボードの bind に失敗すると `swing up` の起動自体がエラーで終わり、Kubo の gateway のポートが使えなければ Kubo の起動失敗としてバックオフして再起動を繰り返す。

## `swing up` のループ（up.rs）

鍵が設定されていれば、`up::run` は `config.kubo.managed` で `run_unmanaged` / `run_managed` に分かれ、どちらも `up::run` が受け取ったトークン（[shutdown](#shutdownshutdownrs)）の下で動く。

- Kubo の起動・ヘルス待ちの失敗、Kubo の異常終了、agent の異常終了（managed は `Err` と panic、unmanaged は `Err` だけ）ではバックオフして再起動し、`up::run` は動き続ける。
- managed の `locate_binary`・`version`・`recover_orphan` の失敗と、unmanaged の `config.ipfs_client()` の失敗は、`up::run` ごとエラーで終わる。
- Kubo の停止（`stop_daemon`）は、どの経路でも `Daemon::stop` が失敗したら warn を出すだけで続ける。`kubo.pid` と `kubo-api.json` は `Daemon::stop` が成功したときだけ消し、失敗したときは残して次回の [`recover_orphan`](kubo/daemon.md#kubopid-と孤児-kubo-の回収managed-のみ) に任せる。

### unmanaged

1. `config.ipfs_client()`（`[ipfs].api` の URL。秘密は付けない）でクライアントを作り、`kubo::wait_healthy`（30 秒、`UNMANAGED_HEALTH_TIMEOUT`）で待つ。
2. そのクライアントを測定の対象として `AppState.stats` に渡し（PID は渡さない）、`agent::run_until` を子トークンで動かす。
3. agent が `Ok(())` なら終了、`Err` ならバックオフして 1 から。cancel されたら agent の終了を最大 15 秒（`AGENT_STOP_TIMEOUT`）待ち、超えたら warn を出して future を捨てて終了する。

### managed

`run_managed` の始めに（`up::run` が呼ばれるたびに）`locate_binary` + `version`（[`kubo.md#kubo-のバージョン`](kubo.md#kubo-のバージョン)）、続けて [`recover_orphan`](kubo/daemon.md#kubopid-と孤児-kubo-の回収managed-のみ) を行う。`recover_orphan` はトークンの cancel と競争させ、cancel が先なら回収を途中でやめて `Ok(())` を返す（`kubo.pid` は残り、次の起動で回収し直す）。以後ループ:

1. 前回の `<state_dir>/kubo-api.json` を消し、repo を用意し（[`ensure_repo`](kubo.md#リポジトリの初期化kuboensure_repo)）、空きポートと新しい RPC の秘密を選んで（[`kubo.md#rpc-の認証managed-のみ`](kubo.md#rpc-の認証managed-のみ)）設定を適用し（[`apply_config`](kubo.md#適用する-kubo-設定kuboapply_config)）、Kubo を起動して `kubo.pid` を書き（[`kubo/daemon.md`](kubo/daemon.md#kubopid-と孤児-kubo-の回収managed-のみ)）、ヘルス待ち（120 秒、`MANAGED_HEALTH_TIMEOUT`。[`kubo/daemon.md`](kubo/daemon.md#ヘルス待ちkubowait_healthy--daemonwait_healthy)）の後で `kubo-api.json` を書く。
   - いずれかの手順が失敗したら（ヘルス待ちか `kubo-api.json` の書き込みが失敗した場合は daemon を止めてから）バックオフして 1 からやり直す。`kubo.pid` の書き込みの失敗だけは warn を出して続ける。
   - ヘルス待ちの間に cancel されたら daemon を止めて `Ok(())` を返す。
2. Kubo の PID と `Daemon` の RPC クライアントを測定の対象として `AppState.stats` に渡す（Kubo が exit したら外す。[`stats.md`](stats.md#測り方)）。`config.ipfs.api` を `IpfsApi::Url("http://127.0.0.1:<api_port>")` に差し替え、`config.ipfs.api_secret` にその起動の RPC の秘密を入れたコピーで agent を子トークンとともに動かす。ダッシュボードは agent の準備ができたときに、agent と同じクライアントを受け取る。
3. 次のいずれかを待つ:
   - Kubo が exit したら、agent を止め（最大 15 秒、`AGENT_STOP_TIMEOUT`。超えたら `abort()`）、`kubo.pid` と `kubo-api.json` を消し、バックオフして 1 からやり直す。
   - agent が `Err` か panic で終わったら、バックオフしてから agent だけを同じ Kubo に対して起こし直す。
   - agent が正常に終わるか、バックオフ中か待っている間に cancel されたら、agent を止め（待っている間の cancel なら最大 15 秒）、`Daemon::stop(20s)` して `Ok(())` を返す。

### バックオフ（`Backoff`）

1 秒から開始し、リトライのたびに倍にして最大 60 秒で頭打ち（`1, 2, 4, 8, 16, 32, 60, 60, ...`）。直前に動いていた期間（Kubo なら healthy になってからの時間、agent なら起動してからの実行時間）が 60 秒以上あれば、次の遅延は 1 秒にリセットする。待機中も cancel に即応する。

## 終了要求と exit code（`shutdown::ExitRequest`, `shutdown::Exit`）

`up::run` は `Result<shutdown::Exit>`（`Exit::Stop` | `Exit::Restart`）を返す。`main.rs` の `Command::Up` ループは、`Exit::Stop` ならプロセスを終了し（exit code 0）、`Exit::Restart` なら `config::Config::load` で設定を読み直してから同じプロセス・同じ PID のまま `up::run` をもう一度呼ぶ。`Err` なら終了コード 1 でプロセスごと終わる。

`ExitRequest` は `up::run` が呼ばれるたびに、その回のトークン（[shutdown](#shutdownshutdownrs)）から作り直し、`dashboard::AppState.exit` にクローンを渡す。サービスマネージャから見た停止と再起動の扱いは [`service.md#共通`](service.md#共通)。`stop()` はトークンを cancel するだけ、`restart()` は内部の `AtomicBool` を立ててから同じトークンを cancel する。

- `stop()` を呼ぶのは `POST /api/shutdown`、`restart()` を呼ぶのは `POST /api/restart`・`POST /api/setup`・`POST /api/signer/reconnect`（[`dashboard/http-api.md`](dashboard/http-api.md)）。シグナル（または Windows の親プロセス終了）で cancel され、誰も `restart()` を呼んでいなければ `Exit::Stop` になる。
- `restart()` の後は、シグナルを受けたときと同じ経路で Kubo と agent がグレースフルに終わり（セットアップモードなら `token.cancelled()` を抜けるだけ）、`up::run` が `Exit::Restart` を返す。

## shutdown（shutdown.rs）

`cancel_on_signal(grace) -> Result<SignalWatch>` がシグナル監視の入口。`main.rs` が `swing up` のループに入る前に `up::FORCE_EXIT_GRACE`（下記「停止の時間予算」）を渡して 1 回だけ呼び、`SignalWatch::token()` のトークンの `child_token()` を `up::run` に毎回渡す。`cancel_on_signal` を使うのは `swing up` だけ。Windows ではもう 1 つ `SignalWatch::cancel_when_parent_exits()` があり、`swing up --exit-with-parent`（隠しオプション、Windows のみ。タスクスケジューラ登録が付ける。[`service.md`](service.md)）のときに呼ぶ。

- unix では SIGINT と SIGTERM を、Windows では `ctrl_c`（`signal = "ctrl-c"`）を、runtime が動いている間ずっと待ち続ける。
- `cancel_when_parent_exits` は親プロセスを開いて専用の OS スレッドで終了を待ち、親が終わったら `reason = "parent exited"` で 1 回目のシグナルと同じ処理に入る。親を開けなければ `swing up` 自体がエラーで終わる。
- 1 回目（シグナルでも親の終了でも、先に来た方）: `shutdown requested` を info で出して `token.cancel()` し、専用の OS スレッドで `grace` 待つ watchdog を起こす。`grace` たってもプロセスが残っていれば error を出して `std::process::exit(1)` する。watchdog は runtime の畳み込み（下記）の間も効く。
- 2 回目以降のシグナル（1 回目の後、runtime が動いている間）: error を出してすぐ `std::process::exit(1)` する。1 回目の後に親が終わっても何もしない（2 回目扱いにしない）。
- ダッシュボード API からの停止（`POST /api/shutdown`、`swing stop`）は `up::run` の子トークンを cancel するだけで `SignalWatch` を通らないので、watchdog も 2 回目の判定も無い。

`main.rs` は tokio の runtime を自前で組み、`run` を抜けた後に `shutdown_timeout(shutdown::RUNTIME_SHUTDOWN_TIMEOUT)`（10 秒）で畳む。終わらない blocking タスクが残っていても、10 秒でプロセスは終わる。

### 停止の時間予算

シグナルを受けてからプロセスが終わるまでの最悪時間は、`up.rs` の定数から次のように決まる（`up::tests::stop_budget_fits_within_force_exit_and_service_manager_limits` で固定）。括弧内は Windows の値（無いものは全 OS 共通）。

| 段階 | 最悪 | 定数 |
|---|---|---|
| agent の停止待ち（超えたら managed は `abort()`、unmanaged は future を捨てる） | 15 秒 | `AGENT_STOP_TIMEOUT` |
| Kubo の停止（猶予 `DAEMON_STOP_GRACE` 20 秒。内訳は [`kubo/daemon.md`](kubo/daemon.md#停止daemonstopgrace)） | 35 秒（25 秒） | `kubo::daemon_stop_budget(DAEMON_STOP_GRACE)` |
| ダッシュボードの停止待ち | 5 秒 | `DASHBOARD_SHUTDOWN_TIMEOUT` |
| 上の合計（managed の最悪。unmanaged は agent + ダッシュボードの 20 秒、セットアップモードはダッシュボードの 5 秒） | 55 秒（45 秒） | `STOP_BUDGET` |
| runtime の畳み込み | 10 秒 | `shutdown::RUNTIME_SHUTDOWN_TIMEOUT` |
| 強制終了までの猶予 = `STOP_BUDGET` + `RUNTIME_SHUTDOWN_TIMEOUT` + 余裕 5 秒 | 70 秒（60 秒） | `FORCE_EXIT_GRACE` |
| サービスマネージャの上限（systemd の `TimeoutStopSec`、launchd の `ExitTimeOut`、[`service.md`](service.md)） | 90 秒 | `service::STOP_TIMEOUT` |

関係は「`STOP_BUDGET` + `RUNTIME_SHUTDOWN_TIMEOUT` < `FORCE_EXIT_GRACE` < `service::STOP_TIMEOUT`」。署名アプリとの接続を閉じる `Signer::shutdown` と、`ensure_repo`・`version` が起動する短命の `ipfs` コマンドは時間の上限を持たず、予算に入れていない（詰まったときは watchdog が止める）。

## 多重起動の防止（lock.rs）

`up::run` は最初に `lock::acquire(state_dir)` で `<state_dir>/swing.lock` を取る。下の `lock::try_acquire(state_dir, file_name)` は、ほかのプロセスが持っていれば `Held { pid }` を返し、`swing-tray` も `swing-tray.lock` に使う（[`tray.md`](tray.md#多重起動の防止)）。

- `state_dir` が無ければ `0700` で作る。
- ロックファイルを作成（無ければ）・読み書きで開く。Unix では `O_NOFOLLOW` で開くので、シンボリックリンクならエラーになり、開いたあとに通常ファイルでなければ（ディレクトリなど）エラーにする。その上で `std::fs::File::try_lock()`（advisory lock。プロセスが `kill -9` で消えても OS が外す）を取る。
- 取れたら中身を空にして自分の PID を書く。
- 既に別のプロセスが取っていれば、ファイルの中身（相手の PID）を読んで `another swing instance is already running on <state_dir> (pid N)` でエラーにする（PID が読めなければ `(pid N)` を省く。Windows ではロックがファイル全体への強制ロックなので常に省かれる）。
- `InstanceLock` を drop してもロックファイル自体は消さない。次回の `acquire` はロックが外れていれば中身を上書きして取り直す。
