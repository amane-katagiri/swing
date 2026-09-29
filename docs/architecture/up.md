# swing up（up.rs, shutdown.rs, lock.rs）

[`../architecture.md`](../architecture.md) の一部。設定キーは [`../architecture.md#設定と環境変数`](../architecture.md#設定と環境変数)、内蔵 gateway は [`gateway.md`](gateway.md)、Kubo プロセスの管理は [`kubo.md`](kubo.md#kubo-プロセスの管理kubors)、OS への常駐登録は [`service.md`](service.md)、コンテナでの起動は [`docker.md`](docker.md)。

`swing up`（`up::run`）は起動順が固定されている: `swing.lock` の取得（[多重起動の防止](#多重起動の防止lockrs)）→ 終了要求（`ExitRequest`、[下記](#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）の作成 → `<state_dir>/upload/` の掃除（`dashboard::cleanup_upload_dir`）→ `signer::Signer::load` で署名の方法を決める（下記「セットアップモード」、[`signer.md`](signer.md)）→ ダッシュボードの `TcpListener::bind`（`up::bind_dashboard`。セットアップモードではポートをずらすことがある。下記「セットアップモードでのポートの調整」）→ ダッシュボードのトークン（`<state_dir>/dashboard.token`）の読み込み・作成（`auth::load_or_create_token`）→ ダッシュボードの `AppState` 作成・`dashboard::serve` の起動 → リソース使用量の測定（`stats::run`、[`stats.md`](stats.md)）の起動 → 鍵の有無・`[kubo].managed` に応じた Kubo / agent の起動ループ → 終わったら測定とダッシュボードを止め、署名アプリとの接続を閉じる（`Signer::shutdown`）。ダッシュボードは bind した時点で応答を始める（agent の準備が整うまで 503 を返す API は [`dashboard/http-api.md#共通`](dashboard/http-api.md#共通)）。`up::run` は呼ばれるたびにこの順で最初からやり直す（署名の方法の扱いは [`signer.md`](signer.md#signer)）。

鍵が設定されていれば、Kubo / agent のループは `[kubo].managed` に応じて 2 通りに分かれる。

- `managed = true`: Kubo を子プロセスとして起動・設定・監視し、その上で `agent::run_until`（[`agent.md`](agent.md)）を動かす。
- `managed = false`: 外部の Kubo（`[ipfs].api`）のヘルスを待ってから `agent::run_until` を動かす。

どちらも `up::run` が受け取ったトークン（[shutdown](#shutdownshutdownrs)）の下で動く。Kubo の起動・ヘルス待ちの失敗、Kubo の異常終了、agent の異常終了（managed は `Err` と panic、unmanaged は `Err` だけ）ではバックオフして再起動し、`up::run` は動き続ける。ただし managed の `locate_binary`・`version`・`recover_orphan` の失敗、unmanaged で Kubo API の URL を決められない場合（`Config::ipfs_api_url` のエラー）、managed の停止時の `Daemon::stop` の失敗（下記）では `up::run` ごとエラーで終わる。ダッシュボードの API サーバは agent の終了では止まらず（agent は抜けるときに `AppState::set_not_ready` を呼ぶだけ。後始末の順序は [`agent.md#シグナルと終了`](agent.md#シグナルと終了)）、`up::run` の終わりに止める。止まるのを最大 5 秒（`DASHBOARD_SHUTDOWN_TIMEOUT`）待ち、超えたら warn を出して待つのをやめる。

## セットアップモード（鍵未設定）

秘密鍵（`[nostr].secret_key` / `SWING_NOSTR_SECRET_KEY`）も `<state_dir>/remote-signer.json` も無く、`signer::Signer::load` が `None` を返すと（読み込み規則は [`signer.md`](signer.md)）、`up::run` は Kubo も agent も起動せず、ダッシュボードだけを動かして `token.cancelled()` を待つ（`dashboard::AppState::new` には `signer: None` を渡す。[`dashboard.md`](dashboard.md#セットアップモードと-appstatesetup_mode)）。このモードでの API の応答は [`dashboard/http-api.md#共通`](dashboard/http-api.md#共通) を参照。

セットアップモードを抜ける経路は 2 つある。`POST /api/setup`（[`dashboard/http-api.md#post-apisetup`](dashboard/http-api.md#post-apisetup)）は鍵（または署名アプリの接続情報）と初期設定を書き込んだ後、下記の「終了要求と exit code」のプロセス内再起動をスケジュールする。`swing signer pair`（[`cli.md#signer-pair`](cli.md#signer-pair)）は `remote-signer.json` だけを書き、次に `up::run` が始まったとき（`swing stop --restart` などによる再起動）に `Signer::load` がそれを読んでセットアップモードを抜ける。`swing up` は鍵が無くても起動でき、`swing.toml` が無くても書き込み先は用意される（設定ファイルの探索順は [`../architecture.md`](../architecture.md#設定と環境変数) 参照）ので、初回起動はこのモードで待ち、ダッシュボードのセットアップ画面から鍵（または署名アプリとのペアリング）・relays・保存上限を書き込んで自分自身を再起動する（[`dashboard.md`](dashboard.md)）。

### セットアップモードでのポートの調整

セットアップモードの間は、既定のポートがほかのプロセスに使われていても起動と初回の起動後の Kubo が止まらないよう、ダッシュボードと管理下の Kubo の gateway のポートをずらして設定ファイルに書き込む。`swing up --no-port-shift`（環境変数 `SWING_NO_PORT_SHIFT` でも指定できる。`up::run` の `port_shift = false`。Docker イメージは既定で有効にしている。[`docker.md#dockerfile`](docker.md#dockerfile)）のときと、値が環境変数由来（`ports::may_shift`。Docker のポート公開と食い違わないように）のときは一切ずらさず書き込みもしない。設定ファイルに書いてある値はずらす対象に含める（セットアップ前のファイルは `swing.example.toml` の写しであることが多いため）。

ずらし方（`ports::bind_shifting`）: 設定のアドレスから始めて、同じ IP でポートを 1 ずつ上げながら最大 20 個（`ports::PROBE_COUNT`）先まで `tokio::net::TcpListener::bind` を試し、それでも駄目ならポート 0（OS が選ぶ空きポート）で bind する。次の候補に進むのは bind が `AddrInUse` か `PermissionDenied`（Windows の除外ポート範囲）で失敗したときだけで、それ以外の失敗はそのままエラーにする。

どちらも `up::bind_dashboard` が、ダッシュボードの bind のときにまとめて行い、書き込んだ後の設定で `AppState` を作る。セットアップを終える経路（`POST /api/setup` と `swing signer pair`）は、どちらもこのとき書き込んだ値をそのまま引き継ぐ。

- ダッシュボード: 上の手順で bind し、ずらしたときは warn（`dashboard port is in use; listening on another port`）を出す。`swing dashboard open`・`swing-tray`・`swing stop` は設定ファイルの `listen` を見てつなぐので、セットアップを終える前のこの時点で書く。
- Kubo の gateway: `[kubo].managed = true` なら、`[kubo].gateway_listen` から同じ手順で空いているアドレスを探す（`ports::free_addr`。bind してすぐ閉じるだけ。セットアップモードでは Kubo を起動しないので、この時点で空いているかを確かめられる）。ずらしたときは warn（`Kubo gateway port is in use; using another port`）を出す。探すこと自体に失敗したら warn を出して書き込まない。
- ずらしたかどうかに関わらず、両方のアドレスを `settings::pin_addrs` で 1 回で書き込む。書き込んだ後は設定を読み直すので、`[dashboard].gateway` が既定値のままなら、そのリンク先のポートもずらした Kubo の gateway のポートになる（[`dashboard.md#設定dashboard`](dashboard.md#設定dashboard)）。書き込みに失敗したら warn を出し、メモリ上の設定のダッシュボードのアドレスだけを bind したものに直して続ける。
- セットアップモードで起動するたびにやり直すので、前回書き込んだポートが使われていても、セットアップを終えるまではまたずらせる。書き込んでからセットアップを終えるまでの間に Kubo の gateway のポートが使われた場合は、通常モードで Kubo の起動に失敗する（下記）。

セットアップモードでないときは、ダッシュボードの bind に失敗すると `swing up` の起動自体がエラーで終わり、Kubo の gateway が使えなければ Kubo の起動失敗としてバックオフして再起動を繰り返す（上記）。

## shutdown（shutdown.rs）

`cancel_on_signal(grace) -> Result<SignalWatch>` がシグナル監視の入口。`main.rs` が `swing up` のループに入る前に `up::FORCE_EXIT_GRACE`（下記「停止の時間予算」）を渡して 1 回だけ呼び、`SignalWatch::token()` のトークンの `child_token()` を `up::run(config, token)` に毎回渡す。`cancel_on_signal` を使うのは `swing up` だけ。Windows ではもう 1 つ `SignalWatch::cancel_when_parent_exits()` があり、`swing up --exit-with-parent`（隠しオプション、Windows のみ。タスクスケジューラ登録が付ける。[`service.md`](service.md)）のときに呼ぶ。

- unix では SIGINT と SIGTERM（どちらも `tokio::signal::unix::signal`）を、Windows では `ctrl_c`（`tokio::signal::windows::ctrl_c`、`signal = "ctrl-c"`）を、runtime が動いている間ずっと待ち続ける。
- `cancel_when_parent_exits` は親プロセスを開いて専用の OS スレッドで終了を待ち、親が終わったら `reason = "parent exited"` で 1 回目のシグナルと同じ処理に入る。親を開けなければ `swing up` 自体がエラーで終わる。
- 1 回目（シグナルでも親の終了でも、先に来た方）: `info!(reason, grace_period, "shutdown requested")` を出して `token.cancel()` し、専用の OS スレッドで `grace` 待つ watchdog を起こす。`grace` たってもプロセスが残っていれば `error!` を出して `std::process::exit(1)` する。watchdog は tokio のタスクではないので、`run` を抜けた後の runtime の畳み込み（下記 `RUNTIME_SHUTDOWN_TIMEOUT`）の間も効き続け、シグナルからプロセス終了までの時間全体を `grace` で抑える。正常終了時はプロセスごと終わるので到達しない。
- 2 回目以降のシグナル（1 回目の後、runtime が動いている間）: `error!(signal, "received another signal during graceful shutdown; exiting immediately")` を出してすぐ `std::process::exit(1)` する。1 回目の後に親が終わっても何もしない（2 回目扱いにしない）。
- ダッシュボード API からの停止（`POST /api/shutdown`、`swing stop`）は `up::run` の子トークンを cancel するだけで `SignalWatch` を通らないので、watchdog も 2 回目の判定も無い。

`main.rs` は tokio の runtime を自前で組み、`run` を抜けた後に `shutdown_timeout(shutdown::RUNTIME_SHUTDOWN_TIMEOUT)`（10 秒）で畳む。終わらない blocking タスクが残っていても、10 秒でプロセスは終わる。

### 停止の時間予算

シグナルを受けてからプロセスが終わるまでの最悪時間は、`up.rs` の定数から次のように決まる（`up::tests::stop_budget_fits_within_force_exit_and_service_manager_limits` で固定）。

| 段階 | 最悪 | 定数 |
|---|---|---|
| agent の停止待ち（超えたら managed は `abort()`、unmanaged は future を捨てる） | 15 秒 | `AGENT_STOP_TIMEOUT` |
| Kubo の停止（`Daemon::stop`、[`kubo.md`](kubo.md#停止daemonstopgrace)）: RPC 5 秒 + 猶予 20 秒 + unix は SIGTERM 後 10 秒（Windows は 0 秒） | 35 秒 | `kubo::daemon_stop_budget(DAEMON_STOP_GRACE)` |
| ダッシュボードの停止待ち | 5 秒 | `DASHBOARD_SHUTDOWN_TIMEOUT` |
| 上の合計（managed の最悪。unmanaged は agent + ダッシュボードの 20 秒、セットアップモードはダッシュボードの 5 秒） | 55 秒 | `STOP_BUDGET` |
| runtime の畳み込み | 10 秒 | `shutdown::RUNTIME_SHUTDOWN_TIMEOUT` |
| 強制終了までの猶予 = `STOP_BUDGET` + `RUNTIME_SHUTDOWN_TIMEOUT` + 余裕 5 秒 | 70 秒 | `FORCE_EXIT_GRACE` |
| サービスマネージャの上限（systemd の `TimeoutStopSec`、launchd の `ExitTimeOut`、[`service.md`](service.md)） | 90 秒 | `service::STOP_TIMEOUT` |

関係は「`STOP_BUDGET` + `RUNTIME_SHUTDOWN_TIMEOUT` < `FORCE_EXIT_GRACE` < `service::STOP_TIMEOUT`」。停止手順が予算内で終われば exit 0 で終わり、watchdog の exit 1 は予算を超えて何かが詰まったときだけになる。サービスマネージャはそれより後にしか SIGKILL を送らない。署名アプリとの接続を閉じる `Signer::shutdown` と、`ensure_repo`・`apply_config` などが起動する短命の `ipfs` コマンドは時間の上限を持たず、予算に入れていない（詰まったときは watchdog が止める）。

## 多重起動の防止（lock.rs）

公開しているのは `lock::acquire(state_dir) -> Result<InstanceLock>`、その下の `lock::try_acquire(state_dir, file_name) -> Result<TryAcquire>`（`Acquired(InstanceLock)` か、ほかのプロセスが持っていれば `Held { pid }`。`swing-tray` が `swing-tray.lock` に使う。[`tray.md`](tray.md#多重起動の防止)）と、ロックファイルのパスを返す `InstanceLock::path`。`up::run` は処理を始める前に最初に `acquire` を呼ぶ。`acquire` は `try_acquire(state_dir, "swing.lock")` の `Held` を下記のエラーにしたもの。

- `state_dir` が無ければ `auth::create_private_dir_all` で `0700` で作る。
- `<state_dir>/swing.lock` を作成（無ければ）・読み書きで開き、`std::fs::File::try_lock()`（advisory lock、OS が管理し、プロセスが `kill -9` で消えても自動的に外れる）を取る。
- 取れたら中身を空にして自分の PID（`std::process::id()`）を書く。
- 既に別のプロセスが取っていれば（`TryLockError::WouldBlock`）、ファイルの中身（相手の PID）を読んで `another swing instance is already running on <state_dir> (pid N)` でエラーにする（PID が読めなければ `(pid N)` を省く。Windows ではロックがファイル全体への強制ロックのため、常に省かれる）。
- `InstanceLock` を drop してもロックファイル自体は消さない。ファイルは残り続け、次回の `acquire` はロックが外れていれば中身を上書きして取り直す。

Kubo バイナリの検出・バージョン確認・リポジトリの初期化・設定の適用・起動・停止・孤児回収は [`kubo.md`](kubo.md#kubo-プロセスの管理kubors) を参照。

## `swing up` のループ（up.rs）

`up::run(config, token)` は `config.kubo.managed` で `run_unmanaged` / `run_managed` に分かれる。

### unmanaged

```
loop {
    wait_healthy(config.ipfs_api_url(), 30s)   // cancel されたら即終了
    AppState.stats に RPC の URL を渡す（PID は分からないので渡さない。stats.md）
    agent::run_until(config, token.child_token())
    // cancel されたら agent の終了を最大 15 秒（AGENT_STOP_TIMEOUT）待ち、超えたら warn を出して future を捨てて終了
    // Ok(()) なら終了。Err ならバックオフして最初から
}
```

### managed

`run_managed` の始めに（`up::run` が呼ばれるたびに）`locate_binary` + `version`（不一致は warn）、続けて [`recover_orphan`](kubo.md#kubopid-と孤児-kubo-の回収managed-のみ) を行う。どれかが失敗したら `up::run` ごとエラーで終わる。`recover_orphan` はトークンの cancel と競争させ、cancel が先なら回収を途中でやめて `Ok(())` を返す（`kubo.pid` は残るので、次の起動で回収し直す）。以後ループ:

1. `ensure_repo` → `pick_free_port` → `apply_config` → `Daemon::spawn(..., "http://127.0.0.1:<api_port>")` → `Daemon::wait_healthy(repo, 120s)`（PeerID の一致と子プロセスの終了も見る。[`kubo.md`](kubo.md#ヘルス待ちkubowait_healthy--daemonwait_healthy)）。
   - いずれかの手順が失敗したら（ヘルス待ちが失敗した場合は daemon を `stop` してから）バックオフして 1 からやり直す。
2. Kubo の PID と RPC の URL を測定の対象として `AppState.stats` に渡す（`Recorder::set_kubo`。Kubo が exit したら外す。[`stats.md`](stats.md#測り方)）。`config.ipfs.api` を `IpfsApi::Url(api_url)` に差し替え、`config.ipfs.api_secret` にその起動の RPC の秘密（[`kubo.md#rpc-の認証managed-のみ`](kubo.md#rpc-の認証managed-のみ)）を入れたコピーで `agent::run_until`（共有の `Arc<dashboard::AppState>` と `Arc<Notify>` を渡す）を子トークンとともに `tokio::spawn` する。`agent::run_until` は `Result<()>` を返すだけで、終了要求の種別（stop/restart）は持たない（下記「終了要求と exit code」）。
3. `tokio::select!` で次のいずれかを待つ:
   - **Kubo が exit** → `error!` を出し、agent を cancel して最大 15 秒（`AGENT_STOP_TIMEOUT`）待つ（超えたら `abort()`）。`kubo.pid` を消し、バックオフして 1 からやり直す（Kubo・agent の両方を再起動）。
   - **agent が Err（または panic）** → `warn!`／`error!` を出し、バックオフしてから **agent だけ**を同じ Kubo に対して再起動する（Kubo はそのまま）。バックオフ中に cancel されたら `Daemon::stop(20s)` して `Ok(())` を返す。
   - **agent が Ok**（cancel による正常終了） → `Daemon::stop(20s)` して `Ok(())` を返す。
   - **親トークンが cancel** → agent を cancel して最大 15 秒待ち（超えたら `abort()`）、`Daemon::stop(20s)` して `Ok(())` を返す。

Kubo の停止（`stop_daemon`）は、1 のヘルス待ちの失敗・その待機中の cancel も含めどの経路でも同じ扱いで、`Daemon::stop` が失敗しても warn を出すだけで経路どおりに続ける（エラーにはしない）。親トークンの cancel は SIGTERM のほか `swing stop`・トレイ（`/api/shutdown`）、`/api/restart`、setup 後の再起動も通るので、停止の失敗で exit 1（サービスマネージャによる再起動）になったり、プロセス内再起動がプロセス終了になったりしない。`kubo.pid` は `Daemon::stop` が成功したときだけ消し、失敗したときは残して次回の [`recover_orphan`](kubo.md#kubopid-と孤児-kubo-の回収managed-のみ) に回収を任せる。

`run_managed` が `Ok(())` を返したときの `swing up` 全体の終わり方は `up::run` が別途持つ `ExitRequest` から決める（下記）。

### バックオフ（`Backoff`）

1 秒から開始し、リトライのたびに倍にして最大 60 秒で頭打ち（`1, 2, 4, 8, 16, 32, 60, 60, ...`）。直前に動いていた期間（Kubo なら healthy になってからの時間、agent なら起動してからの実行時間）が 60 秒以上あれば、次の遅延は 1 秒にリセットする。待機中も cancel に即応する。

## 終了要求と exit code（`shutdown::ExitRequest`, `shutdown::Exit`）

`up::run` は `Result<shutdown::Exit>`（`Exit::Stop` | `Exit::Restart`）を返す。これを消費するのは `main.rs` の `Command::Up` ループで、`Exit::Stop` ならそこで `Ok(())` を返してプロセスを終了させ（exit code 0）、`Exit::Restart` なら `config::Config::load` で設定を読み直してから同じプロセス内でもう一度 `up::run` を呼ぶ（`continue`）。エラー時（`Err`）は終了コード 1 でプロセスごと終わる。

`ExitRequest` は `up::run` が呼ばれるたびに、その回のトークン（[shutdown](#shutdownshutdownrs)）から作り直し、`dashboard::AppState.exit` にクローンを渡す。`stop()` はトークンを cancel するだけ、`restart()` は内部の `AtomicBool` を立ててから同じトークンを cancel する。ダッシュボード API の `POST /api/shutdown`／`POST /api/restart`（[`dashboard/http-api.md`](dashboard/http-api.md#post-apishutdown-post-apirestart)）はこれを呼ぶ。シグナル（または Windows の親プロセス終了）で cancel され、誰も `restart()` を呼んでいなければ `Exit::Stop` になる。セットアップモードでも同じで、`POST /api/setup` の後は `Exit::Restart` になる。

`restart()` が呼ばれたときのプロセス内再起動の流れ:

1. `POST /api/restart`・`POST /api/setup`・`POST /api/signer/reconnect`（[`dashboard/http-api.md`](dashboard/http-api.md)）が `ExitRequest::restart()` を呼ぶ。
2. その回のトークンが cancel される。Kubo・`agent::run_until` のトークンはその子なので、シグナルを受けたときと同じ経路でグレースフルシャットダウンが始まる。
3. `run_managed`／`run_unmanaged` と `agent::run_until` がグレースフルに終了する（managed なら `Daemon::stop(20s)` で Kubo も止める。上記「managed」参照）。セットアップモードなら `token.cancelled()` を抜けるだけ。
4. `up::run` が `exit.exit()`（`restart_requested()` を見て `Exit::Restart`／`Exit::Stop` を組み立てる）を返す。
5. `main.rs` のループが `config::Config::load` で設定を読み直し、同じプロセス・同じ PID のまま `up::run` を再度呼ぶ（新しい `swing.lock` の取得からやり直し）。

### サービスマネージャとの関係

`swing stop --restart` やダッシュボードの再起動ボタンはプロセスを終了させずに済ませるので、systemd/launchd/タスクスケジューラの再起動ポリシーには一切関与しない（各マネージャの `stop` の扱いは [`service.md`](service.md) を参照）。

### Windows の制約

Windows には OS シャットダウン・ログオフをタスクスケジューラのプロセスへ通知する仕組みが無く、`swing up` はグレースフルな停止を経ずに（Job Object に割り当てられた Kubo ごと）kill される（受け入れている制約。[`../log/2026-09-23-graceful-stop.md`](../log/2026-09-23-graceful-stop.md)）。
