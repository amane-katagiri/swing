# swing up と Kubo の管理（up.rs, kubo.rs, shutdown.rs）

[`../architecture.md`](../architecture.md) の一部。設定キーは [`../architecture.md#設定と環境変数`](../architecture.md#設定と環境変数)、内蔵 gateway は [`gateway.md`](gateway.md)、OS への常駐登録は [`service.md`](service.md)。

`swing up`（`up::run`）は起動順が固定されている: `swing.lock` の取得（[多重起動の防止](#多重起動の防止lockrs)）→ シグナルハンドラの設定（`shutdown::cancel_on_signal()`、下記）→ `<state_dir>/upload/` の掃除（`dashboard::cleanup_upload_dir`）→ `config.require_secret_key()` を試す（下記「セットアップモード」）→ ダッシュボードの `AppState` 作成・`TcpListener::bind`・`dashboard::serve` の起動 → 鍵の有無・`[kubo].managed` に応じた Kubo / agent の起動ループ。ダッシュボードはこの時点で bind・応答を始めるが、relay・Kubo を使うエンドポイントは agent が起動して `AppState::set_ready` を呼ぶまで 503 を返す（[`dashboard.md`](dashboard.md#起動)）。

鍵が設定されていれば、Kubo / agent のループは `[kubo].managed` に応じて 2 通りに分かれる。

- `managed = true`: Kubo を子プロセスとして起動・設定・監視し、その上で `agent::run_until`（[`agent.md`](agent.md)）を動かす。
- `managed = false`: 外部の Kubo（`[ipfs].api`）のヘルスを待ってから `agent::run_until` を動かす。

どちらも `shutdown::cancel_on_signal()`（下記）で作った `CancellationToken` の下で動き、Kubo・agent いずれかが落ちても `up::run` 自身は動き続けて再起動する（ダッシュボードの API サーバも `up::run` のプロセスの寿命でずっと動き続ける）。ダッシュボードのサーバタスク自体は `up::run` が `dashboard_shutdown_tx`／`dashboard_task` として直接持ち、Kubo・agent のループとは別に、`up::run` 全体の終了時（下記）に最大 5 秒待って止める。

## セットアップモード（鍵未設定）

`config.require_secret_key()`（`[nostr].secret_key` も `SWING_NOSTR_SECRET_KEY` も無い場合にエラーを返す）が失敗すると、`up::run` は Kubo も agent も起動せず、ダッシュボードだけを動かして `token.cancelled()` を待つ（`dashboard::AppState::new` には `keys: None` を渡す。`AppState::setup_mode()` は `own_pubkey.is_none()` で判定する）。relay・Kubo を使う API エンドポイントは `ApiError::NotConfigured`（503、`{"error": "agent is not configured"}`）を返す（`dashboard::api::not_ready` が `setup_mode()` なら `NotConfigured`、そうでなければ通常の `NotReady` を返す）。`GET /api/overview` は `setup: true`、`pubkey`／`npub` は `null` になる（[`dashboard.md`](dashboard.md)、[`dashboard/http-api.md`](dashboard/http-api.md)）。

セットアップモードを抜けるのは `POST /api/setup`（[`dashboard/http-api.md`](dashboard/http-api.md#post-apisetup)）だけで、鍵と初期設定を書き込んだ後、下記の「終了要求と exit code」と同じ `ExitRequest::restart()` を約 300ms 後に呼んでプロセス内再起動をスケジュールする（HTTP レスポンスを返してからにすることで、リクエスト自体は成功として返る）。`swing up` はコマンドラインからは常に起動でき、`swing.toml` が無くても `resolve_config_path` は `<cwd>/swing.toml` という書き込み先を常に返す（存在しなければ `Config::config_exists = false`。[`../architecture.md#設定と環境変数`](../architecture.md#設定と環境変数)）ので、初回起動はこのモードで待ち、ダッシュボードのセットアップ画面から鍵・relays・保存上限を書き込んで自分自身を再起動する（[`dashboard.md`](dashboard.md)）。

## shutdown（shutdown.rs）

`cancel_on_signal() -> Result<CancellationToken>` が唯一の公開関数。`up::run(config)` がこれで作ったトークンを使う。

- SIGINT（`tokio::signal::ctrl_c`）を待つ。unix ではさらに SIGTERM（`tokio::signal::unix::signal(SignalKind::terminate())`）も待つ。どちらか先に届いた方で進む。
- 受信したら `info!(signal, "shutdown requested")` を出して `token.cancel()` する。
- 続けて `FORCE_EXIT_GRACE_PERIOD`（10 秒）待ち、まだプロセスが生きていれば（＝グレースフルシャットダウンが終わらず main の runtime が畳まれていなければ）`error!` を出して `std::process::exit(1)` する。正常終了時はプロセスごと終わるのでこのコードには到達しない。

## 多重起動の防止（lock.rs）

`lock::acquire(state_dir) -> Result<InstanceLock>` が唯一の公開関数。`up::run` が処理を始める前に最初に呼ぶ。

- `<state_dir>/swing.lock` を作成（無ければ）・読み書きで開き、`std::fs::File::try_lock()`（advisory lock、OS が管理し、プロセスが `kill -9` で消えても自動的に外れる）を取る。
- 取れたら中身を空にして自分の PID（`std::process::id()`）を書く。
- 既に別のプロセスが取っていれば（`TryLockError::WouldBlock`）、ファイルの中身（相手の PID）を読んで `another swing instance is already running on <state_dir> (pid N)` でエラーにする（PID が読めなければ `(pid N)` を省く）。
- `InstanceLock` を drop してもロックファイル自体は消さない（消すと、別プロセスが開いた直後にこちらが消すレースがあり得るため）。ファイルは残り続け、次回の `acquire` はロックが外れていれば中身を上書きして取り直す。

## Kubo バイナリの検出（`kubo::locate_binary`）

優先順位:

1. `[kubo].binary` が指定されていればそのパス。存在しなければエラー。
2. `swing` 実行ファイル（`current_exe()`）と同じディレクトリの `ipfs`（Windows は `ipfs.exe`）。
3. `PATH` 上の `ipfs`（Windows も `PATHEXT` は見ず `ipfs.exe` 固定）。
4. どれも無ければエラー（`Kubo binary not found: ...`）。

## バージョン確認

`kubo::KUBO_VERSION = "0.43.1"`（compose の Kubo イメージと同じ、[`kubo.md#kubo-のバージョン`](kubo.md#kubo-のバージョン)）。`swing up` は起動時に一度だけ `ipfs version --number` を実行し、`KUBO_VERSION` と異なれば `warn!` するだけで続行する（エラーにしない）。実行できない（バイナリが壊れている等）場合は `version()` 自体が失敗し、`swing up` はエラー終了する。

## リポジトリの初期化（`kubo::ensure_repo`）

`<repo>/config` が無ければ `IPFS_PATH=<repo>` で `ipfs init` を実行する（あれば何もしない）。呼び出し元にリポジトリを新規作成したかどうかを bool で返す。`<repo>` ディレクトリ自体は無ければ先に作る。

## 適用する Kubo 設定（`kubo::apply_config`）

`swing up` は Kubo を起動するたびに `IPFS_PATH=<repo>` で次の `ipfs config` を順に実行する（`docker/kubo-init.d/001-swing-config.sh` の設定を移植したもので、内容はそちらと同じ。[`docker.md#kubo-の設定`](docker.md#kubo-の設定)）。

| キー | 値 | 備考 |
|---|---|---|
| `Datastore.StorageMax` | `[kubo].storage_max`（10 進バイト数の文字列、例 `"107374182400"`） | |
| `Provide.Strategy` | `[kubo].provide_strategy` | |
| `Gateway.NoFetch` | `true` | 毎回 `--json` で設定 |
| `Gateway.NoDNSLink` | `true` | 毎回 `--json` で設定 |
| `Gateway.PublicGateways` | `[gateway].hosts` を `{"<host>": {"Paths": [], "UseSubdomains": false, "NoDNSLink": false}}` に変換したもの | hosts が空なら `{}` |
| `Addresses.API` | `["/ip4/127.0.0.1/tcp/<api_port>"]` | `api_port` は起動のたびに動的に選ぶ（下記） |
| `Addresses.Gateway` | `[kubo].gateway_listen` を multiaddr にしたもの（`/ip4/.../tcp/...` か `/ip6/.../tcp/...`） | |
| `Addresses.Swarm` | `[kubo].swarm_port` が `Some` のときだけ、Kubo の既定の Swarm リスト 8 本のポートをすべてこの値に置き換えたもの | `None` なら触らない（Kubo の既定のまま） |

`ipfs config` の実行が失敗したら stderr を含めてエラーにする。

## 動的な API ポートと `<repo>/api`

`kubo::pick_free_port()` が `127.0.0.1:0` を bind してすぐ解放し、空いている TCP ポートを 1 つ返す。`swing up` は Kubo を起動するたびにこれで API ポートを選び、`Addresses.API` に設定する。Kubo は起動時に実際に listen したアドレスを `<repo>/api` に multiaddr（例 `/ip4/127.0.0.1/tcp/54321`）で書き出す。

- `kubo::api_url_from_repo(repo)`: `<repo>/api` を読んで `multiaddr_to_http_url` で HTTP URL に変換する。ファイルが無ければ「Kubo is not running（`swing up` を起動するか、`[kubo].managed = false` にして `[ipfs].api` で外部の Kubo を指すよう案内する）」という趣旨のエラーにする。
- `kubo::multiaddr_to_http_url(addr)`: `/ip4/<ip>/tcp/<port>` → `http://<ip>:<port>`、`/ip6/<ip>/tcp/<port>` → `http://[<ip>]:<port>`（`[::1]` のように角括弧を付ける）。`/dns4`・`/dns6`・`/dns` も同様にホスト名をそのまま使う。それ以外のプロトコルや `tcp` 以外はエラー。
- `Config::ipfs_api_url()`（[`../architecture.md`](../architecture.md#設定と環境変数)）は `[ipfs].api` が `Url` ならそのまま返し、`Managed` なら `api_url_from_repo(&config.kubo.repo)` を呼ぶ。`swing status` など他のサブコマンドはこれを経由して、`swing up` が管理している Kubo の実際のポートを見つける。

`kubo::wait_healthy` はこのファイルを読まない。起動直後はまだ `<repo>/api` が存在しないため、`swing up` は選んだポート番号から直接 `http://127.0.0.1:<api_port>` を組み立ててヘルスチェックする。

## デーモンの起動（`kubo::Daemon::spawn`）

```
<bin> daemon --migrate=true --enable-gc --agent-version-suffix=swing
```

`Daemon::spawn(bin, repo, api_url)` は起動する Kubo の API URL（`Addresses.API` に設定したものと同じ、`http://127.0.0.1:<api_port>`）を受け取って `Daemon` に持たせる。`Daemon::stop`（下記）がこれを使って RPC シャットダウンを呼ぶ。

- `IPFS_PATH=<repo>`。stdin は `/dev/null` 相当、stdout/stderr は pipe。
- Linux（`cfg(target_os = "linux")`）のみ、`pre_exec` で `PR_SET_PDEATHSIG(SIGTERM)` を設定する。swing プロセスが SIGKILL 等で消えても、Linux では子の Kubo に SIGTERM が届く。
- Windows（`cfg(windows)`）のみ、Job Object を作って `SetInformationJobObject`（`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`）で「最後のハンドルが閉じたら中のプロセスを道連れに kill する」設定にし、`AssignProcessToJobObject` で子プロセスを割り当てる。Job Object の HANDLE は `Daemon` が持ち、`Drop` で `CloseHandle` する（swing が `TerminateProcess` 相当で消えれば、OS がハンドルを閉じて Kubo も一緒に落ちる。Linux の `PR_SET_PDEATHSIG` の Windows での相当品）。**この経路は未検証**（[`../../docs/architecture.md`](../architecture.md) の実行環境では Windows SDK が無く `cargo check --target x86_64-pc-windows-msvc` すら通らないため、API シグネチャを `windows-sys` のソースで確認しただけ）。macOS にはこの種の機構が無く、`kill_on_drop(true)` と次回起動時の孤児回収に頼る。
- 標準出力・標準エラーは 1 行ごとに `tracing::info!`（stdout）/ `tracing::warn!`（読み取りエラー時）で `target: "kubo"`、`stream = "stdout" | "stderr"` フィールド付きで転送する（Kubo 自身のログレベルは反映せず、行の内容をそのまま info として流す）。標準エラーの行に `"lock"` が含まれていたら `Daemon` 内の `Arc<AtomicBool>` を立てる（`Daemon::saw_repo_lock_error()`）。Kubo は他のデーモンが同じ repo の lock を持っているとき、この語を含むメッセージ（`someone else has the lock` 等）を標準エラーに出すため。

## ヘルス待ち（`kubo::wait_healthy`）

`POST <api_url>/api/v0/id` を 1 秒間隔で叩き、2xx が返れば成功。1 回ごとのリクエストタイムアウトは 5 秒。指定した `timeout` を超えたらエラー。

- unmanaged: 30 秒（`UNMANAGED_HEALTH_TIMEOUT`）。
- managed: 120 秒（`MANAGED_HEALTH_TIMEOUT`）。

待機中も `CancellationToken` の cancel に即座に応答する（`tokio::select!` で `wait_healthy` と `token.cancelled()` を競走させる）。

## 停止（`Daemon::stop(grace)`）

Kubo 自身に「行儀よく終わる」機会を与えるため、まず Kubo の RPC（`ipfs shutdown` と同じ）を叩き、それでも `grace` 以内に終わらなければ段階的に強制する 3 段構え（全 OS 共通の 1 段目 + OS 依存の 2〜3 段目）:

1. `POST <api_url>/api/v0/shutdown` をリクエストタイムアウト 5 秒で送る。Kubo は自分で終了処理を始めてから接続を切る（またはエラーを返す）ことがあるため、レスポンスの成功・失敗・接続エラーのどれであっても「シャットダウンを要求した」ものとして次に進む（リトライしない）。
2. 子プロセスの終了を `grace` 秒まで待つ。終了すればここで成功。
3. まだ生きていれば: unix は SIGTERM を送って 10 秒待ち、それでも終わらなければ `kill()`（SIGKILL）。Windows（`cfg(unix)` に入らない経路）は SIGTERM に相当するものが無いため、待たずに直接 `kill()`。

各段階の成否は `tracing::debug!` に出す（1 段目で終われば SIGTERM/kill には進まない）。

`swing up` は `DAEMON_STOP_GRACE = 30s` を渡す。`swing stop`（[`service.md`](service.md#swing-stopstoprs)）が呼ぶ経路もこの `Daemon::stop` を通るので、CLI からの停止も同じ 3 段構えになる。

## `kubo.pid` と孤児 Kubo の回収（managed のみ）

`swing up` を SIGKILL 等で強制終了すると、`Daemon` の drop も `Daemon::stop` も走らないため、子の Kubo が残り得る（Linux は `PR_SET_PDEATHSIG` で大抵は道連れになるが保証ではない。Windows の Job Object は未検証、macOS には対抗手段が無い）。残った Kubo は同じ repo の lock を握ったままなので、次の `swing up` がそこに気づかずポートだけ変えて起動しようとしても `ipfs config`/起動が repo lock で失敗し続けうる。

- `kubo::write_pid_file(state_dir, pid)` / `read_pid_file(state_dir)` / `remove_pid_file(state_dir)`: `<state_dir>/kubo.pid` に子の PID（10 進）を読み書きする薄いヘルパー。`run_managed` が `Daemon::spawn` 成功直後に書き、`Daemon::stop` が完了したら（通常のシャットダウンでも、agent 再起動に伴う経路でも）消す。Kubo が自分で落ちた（`daemon.wait()` が先に返った）場合も、`stop` を呼ばずに直接消す。
- `kubo::recover_orphan(state_dir, repo) -> Result<()>`: `run_managed` が起動時に 1 回、[多重起動の防止](#多重起動の防止lockrs) のロックを取った直後・デーモンループに入る前に呼ぶ（ロックが取れている＝他の swing は生きていないので、この repo に対する Kubo がもしいればそれは孤児か無関係の別プロセスのどちらかでしかない）。
  1. `kubo.pid` が無ければ何もせず終了。
  2. あれば、その PID が生きていて `ipfs` プロセスかどうかを確かめる（unix: `ps -p <pid> -o comm=` の出力に `ipfs` を含むか。windows: `tasklist /FI "PID eq <pid>" /FO CSV /NH` の出力に `ipfs` を含むか）。生きていない・`ipfs` でなければ `warn!("stale kubo.pid")` を出してファイルを消すだけで終わる。
  3. 生きていて `ipfs` なら `warn!("terminating orphaned Kubo left by a previous swing")` を出し、終了を試みる: unix は SIGTERM → 500ms 間隔で最大 30 秒待ち → まだいれば SIGKILL → 最大 10 秒待つ（それでも終わらなければエラーで `swing up` 自体を止める）。windows は `taskkill /PID <pid> /T /F`（この経路にはグレースフルな段階が無い）→ 最大 10 秒待つ。生存確認は unix が `kill(pid, 0)`（`ESRCH` の判定は `raw_os_error()` を見る。`io::ErrorKind::NotFound` に必ずしもマップされないため）、windows は上と同じ `tasklist` チェック。終わったら `kubo.pid` を消す。
  - `recover_orphan` が失敗すると `run_managed` はそのままエラーを返し、`swing up` は起動せずに終了する（バックオフして黙って再試行しない。repo lock を握ったままの孤児がいるのに新しい Kubo を起動しても意味が無いため）。

### repo lock のヒント

`recover_orphan` が拾えるのは自分が書いた `kubo.pid` だけで、`swing up` の管理下に無い Kubo（手動で起動した、別の swing 管理下にある等）が同じ repo を使っていた場合は検出できない。この場合 `apply_config`/`Daemon::spawn` 後の起動が失敗し続ける。`Daemon` が標準エラーで [`"lock"` を含む行を見た](#デーモンの起動kubodaemonspawn) ことを覚えているので、`wait_healthy` が失敗した直後に daemon が既に exit していて（`Daemon::try_wait()`）かつ `saw_repo_lock_error()` が true なら、`warn!("another ipfs daemon seems to hold the Kubo repo lock; stop it or point [kubo].repo elsewhere")` を追加で出してから通常のバックオフに入る。

## `swing up` のループ（up.rs）

`up::run(config)` は `config.kubo.managed` で `run_unmanaged` / `run_managed` に分かれる。

### unmanaged

```
loop {
    wait_healthy(config.ipfs_api_url(), 30s)   // cancel されたら即終了
    agent::run_until(config, token.child_token())
    // Ok(()) なら終了。Err ならバックオフして最初から
}
```

### managed

起動時に 1 回だけ `locate_binary` + `version`（不一致は warn）、続けて [`recover_orphan`](#kubopid-と孤児-kubo-の回収managed-のみ)。以後ループ:

1. `ensure_repo` → `pick_free_port` → `apply_config` → `Daemon::spawn` → `wait_healthy("http://127.0.0.1:<api_port>", 120s)`。
   - いずれかの手順が失敗したら（`wait_healthy` が失敗した場合は daemon を `stop` してから）バックオフして 1 からやり直す。
2. `config.ipfs.api` を `IpfsApi::Url(api_url)` に差し替えたコピーで `agent::run_until`（共有の `Arc<dashboard::AppState>` と `Arc<Notify>` を渡す）を子トークンとともに `tokio::spawn` する。`agent::run_until` は `Result<()>` を返すだけで、終了要求の種別（stop/restart）は持たない（下記「終了要求と exit code」）。
3. `tokio::select!` で次のいずれかを待つ:
   - **Kubo が exit** → `error!` を出し、agent を cancel して最大 15 秒（`AGENT_STOP_TIMEOUT`）待つ（超えたら `abort()`）。バックオフして 1 からやり直す（Kubo・agent の両方を再起動）。
   - **agent が Err（または panic）** → `warn!`／`error!` を出し、バックオフしてから **agent だけ**を同じ Kubo に対して再起動する（Kubo はそのまま）。
   - **agent が Ok**（cancel による正常終了）、または**親トークンが cancel** → agent を cancel/待ち、`Daemon::stop(30s)` して `run_managed` 自体は `Ok(())` を返す（`swing up` 全体の exit code は `up::run` が別途持つ `ExitRequest` から決める。下記）。

### バックオフ（`Backoff`）

1 秒から開始し、リトライのたびに倍にして最大 60 秒で頭打ち（`1, 2, 4, 8, 16, 32, 60, 60, ...`）。直前に起動していた期間（Kubo なら daemon の生存時間、agent なら `run_until` の実行時間）が 60 秒以上あれば、次の遅延は 1 秒にリセットする。待機中も cancel に即応する。

## 終了要求と exit code（`shutdown::ExitRequest`, `shutdown::Exit`）

`up::run` は `Result<shutdown::Exit>`（`Exit::Stop` | `Exit::Restart`）を返す。これを消費するのはプロセスではなく `main.rs` の `Command::Up` ループで、`Exit::Stop` ならそこで `Ok(())` を返してプロセスを終了させ（exit code 0）、`Exit::Restart` なら `config::Config::load` で設定を読み直してから同じプロセス内でもう一度 `up::run` を呼ぶ（`continue`）。つまり再起動はプロセスの終了・再起動を伴わない。以前あった「`Exit::Restart` は exit code 3 で終了し、サービスマネージャの再起動ポリシーに任せる」という経路は無くなった。エラー終了（`Err`）はこれまでどおり anyhow 由来の非 0（通常 1）でプロセスごと落ちる。

`ExitRequest` は `up::run` が呼ばれるたびに新しく作り直され（`shutdown::cancel_on_signal()` で作った、その回の `CancellationToken` から `ExitRequest::new(token.clone())`）、`dashboard::AppState`（常に有効。`Option` ではない）にクローンを渡す。ダッシュボード API の `POST /api/shutdown`／`POST /api/restart`（[`dashboard/http-api.md`](dashboard/http-api.md)）はこの `ExitRequest` の `stop()`／`restart()` を呼ぶだけ（`restart()` は内部の `AtomicBool` を立ててから同じトークンを cancel する。`stop()` はトークンを cancel するだけ）。このトークンは Kubo・`agent::run_until` が使う子トークンの親でもあるので、`stop()`／`restart()` は SIGINT/SIGTERM を受けたときと同じ経路でグレースフルシャットダウンを開始させる。`up::run` はループ（`run_managed`／`run_unmanaged`。いずれも `Result<()>` を返すだけで stop/restart の区別を持たない）が終わった後、`exit.exit()`（`restart_requested()` を見て `Exit::Restart`／`Exit::Stop` を組み立てる）を戻り値にする。SIGINT/SIGTERM 経由（誰も `restart()` を呼んでいない）の場合は常に `Exit::Stop` になる。セットアップモードの終了要求（`token.cancelled().await` を抜けるだけ）も同じ `exit.exit()` を経由する。

`swing up`（managed）が Kubo をグレースフルに止めてから終了するのは、`run_managed` が `token.cancelled()` を検知したときに `Daemon::stop(30s)` を呼んでから `Ok(())` を返すため（上記「managed」のループ参照）。ダッシュボードから `restart` を要求すると: `POST /api/restart`（または `POST /api/setup` が内部で行う `exit.restart()`）→ `ExitRequest.restart()` → その回の最上位トークンを cancel → `run_managed`／`run_unmanaged` と `agent::run_until` がグレースフルに終了 → `up::run` が `exit.exit()` で `Exit::Restart` を返す → `main.rs` のループが設定を読み直して `up::run` を再度呼ぶ（新しい `swing.lock` の取得からやり直し。同じプロセス・同じ PID のまま）。

### サービスマネージャとの関係

`swing stop --restart` やダッシュボードの再起動ボタンは、上記のとおりプロセスを終了させずに済ませるので、systemd/launchd/タスクスケジューラの再起動ポリシー（`Restart=on-failure` など）は一切関与しない。これらのポリシーが働くのは、プロセスがシグナルや panic・`Err` で実際に終了したとき（グレースフルな `stop`＝exit code 0 は再起動条件に当たらず、クラッシュ＝非 0 だけが対象になる）に限られる。各マネージャの `stop` の扱いは [`service.md`](service.md) を参照。

### Windows の制約

Windows にはタスクスケジューラのプロセスへ「OS シャットダウン/ログオフ」を通知する標準的な仕組みが無い（SCM サービスなら `SERVICE_CONTROL_SHUTDOWN` が届くが、swing は UAC を避けるためタスクスケジューラ登録にしている。[`service.md`](service.md)）。そのため OS シャットダウンやログオフでは `swing up` に何の通知も来ず、プロセスは（Job Object に割り当てられた Kubo ごと）ただ kill される。これは `swing stop` によるグレースフルな停止とは別の経路で、今回のグレースフルストップの対象外（受け入れている制約。[`../log/2026-09-23-graceful-stop.md`](../log/2026-09-23-graceful-stop.md)）。実害が小さい理由:

- Kubo は Job Object の `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` により道連れで終了するので、孤児化して repo lock を握ったまま残ることはない（次回起動時の `recover_orphan` にも頼らずに済む）。
- `state.json` は一時ファイルに書いてから `rename` する形（tmp + rename）で保存しており、書き込み途中の kill で壊れたファイルが残ることはない（[`architecture.md`](../architecture.md) の `state::State::save`）。
- Kubo 自身のデータストアも突然の kill に対して壊れない前提で作られている（badger/flatfs いずれも書き込み中断からの復旧を想定した実装）。

## Dockerfile / compose との関係

`Dockerfile` の `CMD` は `["up"]`。compose の `mirror` サービスは `SWING_KUBO_MANAGED=false` を固定で渡し、外部の `ipfs` コンテナ（`docker/kubo-init.d/001-swing-config.sh` で設定）を使う。詳細は [`docker.md`](docker.md)。
