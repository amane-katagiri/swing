# Kubo（kubo.rs）

[`../architecture.md`](../architecture.md) の一部。MFS の使い方と RPC（managed／unmanaged 共通）は [`mfs.md`](mfs.md)、内蔵 gateway は [`gateway.md`](gateway.md)。

## Kubo プロセスの管理（kubo.rs）

`swing up`（[`up.md`](up.md)）が managed の Kubo を扱うときの検出・起動・設定・停止・孤児回収。呼ぶ順序は [`up.md#managed`](up.md#managed)。

### バイナリの検出（`kubo::locate_binary`）

優先順位:

1. `[kubo].binary` が指定されていればそのパス。存在しなければエラー。
2. `swing` 実行ファイル（`current_exe()`）と同じディレクトリの `ipfs`（Windows は `ipfs.exe`）。
3. `PATH` 上の `ipfs`（Windows も `PATHEXT` は見ず `ipfs.exe` 固定）。
4. どれも無ければエラー（`Kubo binary not found: ...`）。

2・3 で見つけたときは解決したパスを info ログに出す。

### リポジトリの初期化（`kubo::ensure_repo`）

`<repo>/config` が無ければ `IPFS_PATH=<repo>` で `ipfs init` を実行する（あれば何もしない）。`<repo>` ディレクトリ自体は無ければ先に作る。

### 適用する Kubo 設定（`kubo::apply_config`）

`swing up` は Kubo を起動するたびに `IPFS_PATH=<repo>` で次の `ipfs config` を順に実行する。

| キー | 値 | 備考 |
|---|---|---|
| `Datastore.StorageMax` | `[kubo].storage_max`（10 進バイト数の文字列、例 `"107374182400"`） | |
| `Provide.Strategy` | `[kubo].provide_strategy` | `mfs` か `all` を含めないと、MFS にしか無いサイトが DHT に告知されない。知らない値を与えると daemon が起動しない |
| `Gateway.NoFetch` | `true` | 下記「[Kubo の Gateway](#kubo-の-gatewaynofetch)」 |
| `Gateway.NoDNSLink` | `true` | |
| `Gateway.PublicGateways` | `[gateway].hosts` を `{"<host>": {"Paths": [], "UseSubdomains": false, "NoDNSLink": false}}` に変換したもの | hosts が空なら `{}` |
| `Addresses.Gateway` | `[kubo].gateway_listen` を multiaddr（`/ip4/.../tcp/...` か `/ip6/.../tcp/...`）にした 1 要素の配列 | |
| `Addresses.Swarm` | `[kubo].swarm_port` が `Some` のときだけ、Kubo の既定の Swarm リスト 8 本のポートをすべてこの値に置き換えたもの | `None` なら触らない（Kubo の既定のまま） |
| `Addresses.API` | `["/ip4/127.0.0.1/tcp/<api_port>"]` | `api_port` は起動のたびに動的に選ぶ（下記）。`API.Authorizations` と一緒に書く（下記） |
| `API.Authorizations` | `{"swing": {"AuthSecret": "bearer:<secret>", "AllowedPaths": ["/api/v0"]}}` | 値は下記「[RPC の認証](#rpc-の認証managed-のみ)」 |

- `Datastore.StorageMax`・`Provide.Strategy` 以外は `ipfs config --json` で JSON の値として書く。
- 最後の `Addresses.API` と `API.Authorizations` の 2 つは `ipfs config` を使わず（`kubo::set_api_access`）、`<repo>/config` の JSON を読んで書き換える。`Addresses`・`API` が無いか `null` なら空のオブジェクトを作り、ほかのキーはそのまま残す。書き戻しは `settings::write_atomic`（一時ファイル（unix は 0600）からの rename。`<repo>/config` がシンボリックリンクならリンク先のファイルを置き換え、リンクは残す）。
- `ipfs config` の実行が失敗したら stderr を含めてエラーにする。

compose の外部 Kubo コンテナでの同等の設定は [`docker.md#kubo-の設定`](docker.md#kubo-の設定)。

### Kubo の Gateway（`NoFetch`）

`Gateway.NoFetch=true`（上記）により、Kubo の Gateway はローカルにあるブロックだけを返し、ネットワークからは取りに行かない。`Gateway.PublicGateways` に入れたホストでは DNSLink（`_dnslink.<host>`）の内容だけを返す。

Kubo は `Host` と `X-Forwarded-Host` をそのまま信じるので、Kubo の Gateway ポートを外部に直接公開しない。内蔵 gateway（[`gateway.md`](gateway.md)）や compose の `mirror` コンテナを前段に置く。

### RPC の認証（managed のみ）

managed の Kubo の RPC は Kubo の `API.Authorizations` で Bearer トークンを要求する。

- 秘密は 32 バイトの乱数の 16 進（`kubo::ApiSecret`。`Debug` は `<redacted>`）。API のポートと組にした `kubo::ApiAccess { port, secret }` として扱い、Kubo を起動するたびに `ApiAccess::generate(port)` で作り直す。
- `apply_config` が `Addresses.API` と `API.Authorizations` に書き（上表）、`Daemon` はヘルス待ちと RPC シャットダウンでこれを送る。agent には `config.ipfs.api_secret` として渡す（[`up.md#managed`](up.md#managed)）。
- `<state_dir>/kubo-api.json`（JSON: `port`・`secret`）がほかのプロセス（`swing publish`）への受け渡し口。同じプロセスの agent とダッシュボードにはメモリで渡す。`kubo::write_api_access` が `auth::write_private_file` で書く（unix は 0600、`state_dir` が無ければ 0700 で作る）。書く・消すタイミングとメモリでの渡し方は [`up.md#managed`](up.md#managed)。クライアントはポートと秘密を必ずこのファイルから組で読む。
- `kubo::read_api_access(state_dir)` はファイルを読む。無ければ `None`、JSON でないか `secret` が 16 進でなければエラー。
- クライアントは `ipfs::kubo_http_client(Some(&secret))` で作り、すべてのリクエストに `Authorization: Bearer <secret>`（sensitive 指定）を付ける。

unmanaged（`[ipfs].api`）の Kubo には秘密を送らず、認証は SWING では設定しない。Docker Compose の構成での RPC の公開範囲は [`docker.md`](docker.md)。

#### 既知の制限: API ポートの認証のないエンドポイント

`API.Authorizations` が守るのは `/api/v0` の RPC だけで、Kubo は同じ API のリスナーで次のパスを秘密なしで返す（確認した版は Kubo 0.43.1。ステータスは `Authorization` ヘッダーなしの GET／POST）。Kubo の設定でこれらを止める項目は無い。

| パス | GET | POST | 中身 |
|---|---|---|---|
| `/api/v0/id`（比較用） | 403 | 403 | |
| `/debug/pprof/`・`/debug/pprof/heap` | 200 | 405 | Go のプロファイル（CPU プロファイルの取得で負荷をかけられる） |
| `/debug/vars` | 200 | 405 | expvar（コマンドライン・メモリ統計） |
| `/debug/metrics/prometheus` | 200 | 200 | Prometheus のメトリクス |
| `/debug/stack` | 200 | 200 | 全 goroutine のスタック |
| `/debug/pprof-mutex/`・`/debug/pprof-block/` | 405 | 400（引数なし） | 引数付きの POST で mutex・block プロファイルの採取率を変えられる |
| `/logs` | 応答が続く（ストリーム） | 同左 | Kubo のログのストリーム |
| `/version` | 200 | 200 | バージョン |
| `/webui` | 503 | 503 | |

どれも RPC の秘密・鍵・MFS の中身は返さず、MFS やピンを変えることもできないが、同じマシンのほかのユーザーはループバックのポートからこれらを読める（ログやメトリクスから、ミラーしているサイトの CID や通信先が分かりうる）。

### 動的な API ポートと `<repo>/api`

`kubo::pick_free_port()` が `127.0.0.1:0` を bind してすぐ解放し、空いている TCP ポートを 1 つ返す。`swing up` は Kubo を起動するたびにこれで API ポートを選び、`Addresses.API` に設定する。Kubo は API のポートを listen できた後で、そのアドレスを `<repo>/api` に multiaddr（例 `/ip4/127.0.0.1/tcp/54321`）で書き出す。

- `<repo>/api` を読むのは `Daemon::wait_healthy`（下記）だけで、listen したのが自分の起動した Kubo であることの確認に使う。ほかのプロセスが managed の Kubo を探すときは `<state_dir>/kubo-api.json`（上記）を読む。
- `kubo::multiaddr_to_http_url(addr)`: `/ip4/<ip>/tcp/<port>` → `http://<ip>:<port>`、`/ip6/<ip>/tcp/<port>` → `http://[<ip>]:<port>`（`[::1]` のように角括弧を付ける）。`/dns4`・`/dns6`・`/dns` も同様にホスト名をそのまま使う。それ以外のプロトコルや `tcp` 以外はエラー。

### RPC クライアントの作り方（`Config::ipfs_client()`）

`Config::ipfs_client()`（`src/config/mod.rs`）が RPC のクライアントを作る。agent・`swing publish`・unmanaged のヘルス待ちと統計（[`up.md#unmanaged`](up.md#unmanaged)）がこれを使う。

- `[ipfs].api` が `Url`: その URL と `config.ipfs.api_secret` で作る。`api_secret` は設定ファイルからは入らず、`swing up` が managed の agent に渡すコピー（`[ipfs].api` を実際の URL に差し替えたもの。[`up.md#managed`](up.md#managed)）にだけ入る。
- `Managed`: `kubo::managed_client(state_dir, repo)` が `kubo-api.json` のポートと秘密でクライアントを作り、`kubo::ensure_own_daemon` で、その API の `id` の `ID` が `<repo>/config` の `Identity.PeerID` と一致することを確かめる。一致しない、または PeerID が読めなければエラー。ファイルが無ければ `Kubo is not running: ...`（`swing up` を起動するか、`[kubo].managed = false` と `[ipfs].api` で外部の Kubo を指すよう案内する）のエラー。

### デーモンの起動（`kubo::Daemon::spawn`）

```
<bin> daemon --migrate=true --enable-gc --agent-version-suffix=swing
```

`Daemon::spawn(bin, repo, api)` は起動する Kubo の `ApiAccess`（`Addresses.API` に設定したポートと RPC の秘密）を受け取り、`http://127.0.0.1:<api_port>` と、秘密を付ける `IpfsClient` を `Daemon` に持たせる。`Daemon::wait_healthy` と `Daemon::stop`（下記）がこれを使い、`Daemon::ipfs()` で外にも渡す。

- 起動の前に `<repo>/api` があれば消す。
- `IPFS_PATH=<repo>`。stdin は `/dev/null` 相当、stdout/stderr は pipe。
- すべての OS で `kill_on_drop(true)` を設定する。
- Linux（`cfg(target_os = "linux")`）のみ、`pre_exec` で `PR_SET_PDEATHSIG(SIGTERM)` を設定する。swing プロセスが SIGKILL 等で消えても、Linux では子の Kubo に SIGTERM が届く。
- Windows（`cfg(windows)`）のみ、`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` の Job Object に子プロセスを割り当てる。ハンドルは `Daemon` が持ち `Drop` で閉じるので、swing が強制終了されても Kubo は一緒に落ちる。
- macOS では `kill_on_drop(true)` と次回起動時の孤児回収だけになる。
- 標準出力・標準エラーは 1 行ずつ `target: "kubo"` のログ（`stream` フィールド付き）に流す。標準エラーに `repo.lock` か `someone else has the lock` を含む行が出たら覚えておく（`Daemon::saw_repo_lock_error()`。下記「repo lock のヒント」）。

### ヘルス待ち（`kubo::wait_healthy` / `Daemon::wait_healthy`）

どちらも `IpfsClient::peer_id()`（`POST <api_url>/api/v0/id`、リクエストタイムアウト 10 秒）を 1 秒間隔で呼び、指定した `timeout` を超えたらエラー。

- `kubo::wait_healthy(ipfs, timeout)`（unmanaged）: `Config::ipfs_client()` のクライアントで、`id` が成功すれば成功。時間切れのエラーには最後の失敗の理由を付ける。
- `Daemon::wait_healthy(repo, timeout)`（managed）: 始めに `<repo>/config` の `Identity.PeerID` を 1 回読み、読めなければ何も送らずにエラーにする。毎回、まず子プロセスが終わっていないか（`try_wait`）を見て、終わっていれば待たずに `Kubo exited (<status>) before becoming healthy` でエラーにする。次に `<repo>/api` が自分の URL（`http://127.0.0.1:<api_port>`）を指しているかを見て、指していなければその回は何も送らない。`id` の `ID` が読んだ PeerID と一致したときだけ成功とし、別の ID を返す相手には成功しない（時間切れのエラーにその ID と期待した PeerID を入れる）。

`timeout` の値と cancel の扱いは [`up.md`](up.md#swing-up-のループuprs)。

### 停止（`Daemon::stop(grace)`）

まず Kubo の RPC（`ipfs shutdown` と同じ）を叩き、それでも `grace` 以内に終わらなければ段階的に強制する 3 段構え（1・2 段目は全 OS 共通、3 段目が OS 依存）:

1. `IpfsClient::shutdown`（`POST <api_url>/api/v0/shutdown`）をリクエストタイムアウト 5 秒（`SHUTDOWN_RPC_TIMEOUT`）で送る。レスポンスの成功・失敗・接続エラーのどれであっても「シャットダウンを要求した」ものとして次に進む（リトライしない）。
2. 子プロセスの終了を `grace` 秒まで待つ。終了すればここで成功。
3. まだ生きていれば: unix は SIGTERM を送って 10 秒（`SIGTERM_GRACE`）待ち、それでも終わらなければ `kill()`（SIGKILL）。Windows（`cfg(unix)` に入らない経路）は待たずに直接 `kill()`。

`kill()` 自体の失敗はエラーとして返す。

`kubo::daemon_stop_budget(grace)` はこの 3 段の最悪時間（5 秒 + `grace` + unix は 10 秒、Windows は 0 秒。SIGKILL 後の終了待ちは数えない）を返す。`swing up` が渡す `grace` と停止全体の時間予算は [`up.md#停止の時間予算`](up.md#停止の時間予算)。

### `kubo.pid` と孤児 Kubo の回収（managed のみ）

`kubo::write_pid_file` が `Daemon::spawn` の直後に `<state_dir>/kubo.pid`（JSON: `pid`・`api_port`・`started_at`）を書く（[`up.md#managed`](up.md#managed)）。`started_at` はその `pid` の開始時刻を OS ごとの方法（Linux は `/proc/<pid>/stat`、macOS は `ps -o lstart=`、Windows は `GetProcessTimes`）で取った比較専用の文字列。書けなければ warn を出して続行する（その回は孤児回収の対象にならない）。

`kubo::recover_orphan` は `swing.lock` を持っている間に 1 回呼ぶ（呼ぶ時点は [`up.md#managed`](up.md#managed)）。

- `kubo.pid` が無ければ何もしない。読めなければ warn を出してファイルを消すだけで、何も kill しない。
- まずその `pid` の今の開始時刻を取り直し、記録と比べる。プロセスがもう無い、または開始時刻が一致しない（PID の再利用）なら、API にも何も送らず kill もせずにファイルを消す。
- 一致したら記録の `api_port` に API でのシャットダウンを送り（タイムアウト 3 秒、`ORPHAN_SHUTDOWN_RPC_TIMEOUT`）、2xx が返ればその `pid` の終了を最大 30 秒（`ORPHAN_SHUTDOWN_GRACE`）待つ。終われば完了。秘密は `<state_dir>/kubo-api.json` の `port` が記録の `api_port` と同じときだけ付ける（読めなければ warn を出し、秘密なしで送る。秘密なしの要求が拒まれたら次の強制終了に進む）。
- API で終わらなければ強制終了する（unix は SIGTERM → 最大 30 秒（`ORPHAN_SIGTERM_GRACE`）→ SIGKILL → 最大 10 秒（`ORPHAN_KILL_WAIT`）、Windows は `taskkill /T /F` → 最大 10 秒（`ORPHAN_KILL_WAIT`））。
- 強制終了しても終わらなければエラーを返し、`swing up` は Kubo を起動せずに終了する。

`kubo.pid` と `kubo-api.json` をいつ消すかは [`up.md#swing-up-のループuprs`](up.md#swing-up-のループuprs)。

#### repo lock のヒント

`recover_orphan` が拾えるのは自分が書いた `kubo.pid` だけで、`swing up` の管理下に無い Kubo が同じ repo を使っていると起動が失敗し続ける。`Daemon::wait_healthy` が失敗した時点で daemon が exit していて、標準エラーに repo lock の行（上記）が出ていたら、`another ipfs daemon seems to hold the Kubo repo lock; ...` を warn で出してから通常のバックオフに入る。

## Kubo のバージョン

`swing up`（managed）は `ipfs version --number` を実行し（呼ぶ時点は [`up.md#managed`](up.md#managed)）、`kubo::KUBO_VERSION` と異なれば warn を出して続行する。`ipfs version` 自体が実行できなければ `swing up` はエラー終了する。

上げるときに揃える場所:

- `compose.yaml` の `ipfs` サービスのイメージタグ（`ipfs/kubo:v0.43.1`）
- `kubo::KUBO_VERSION`（`src/kubo.rs`）
- `packaging/linux/install.sh` の `KUBO_VERSION`（[`install-sh.md`](install-sh.md)）
- Windows のインストーラーに同梱する Kubo のチェックサム `packaging/windows/kubo.sha512`（<https://dist.ipfs.tech/kubo/> の `kubo_v<版>_windows-amd64.zip.sha512` をそのまま置く。ファイル名が `KUBO_VERSION` と合っていることを `kubo::tests` が確かめる。[`installer-windows.md`](installer-windows.md#kubo-の取得と検証)）
- README と docs の版表記（`v0.43.1` で検索できる）

次の Kubo の挙動に依存しているので、上げると壊れうる。

- `file does not exist` の文面での判定（変わると、突き合わせが MFS から消えた版を取り直さず警告を出し続ける）
- `files/rm` が失敗時も 200 を返すこと
- 各 RPC の JSON の形（`TotalSize`、`Hash`、`Type`（`files/stat` は文字列、`Entries[].Type` は数値）など）と、`add` の multipart・`to-files`
- MFS の保護・GC・`offline=true` の挙動

これを確かめるテストは、統合テスト（`kubo_integration`・`agent_stores_and_removes_through_real_kubo`）と、`SWING_TEST_KUBO_BIN` が要る `kubo::tests` の `#[ignore]` テストと `mfs_kubo_integration`。コマンドは [`../architecture.md#テスト`](../architecture.md#テスト)。
