# Kubo と MFS（ipfs.rs, mfs.rs, kubo.rs）

[`../architecture.md`](../architecture.md) の一部。内蔵 gateway は [`gateway.md`](gateway.md)。[RPC](#rpc) と [MFS の使い方](#mfs-の使い方) は managed／unmanaged どちらの Kubo にも共通する。

## Kubo プロセスの管理（kubo.rs）

`swing up`（[`up.md`](up.md)）による Kubo の検出・起動・設定・監視・停止・孤児回収はここに集約する。

### バイナリの検出（`kubo::locate_binary`）

優先順位:

1. `[kubo].binary` が指定されていればそのパス。存在しなければエラー。
2. `swing` 実行ファイル（`current_exe()`）と同じディレクトリの `ipfs`（Windows は `ipfs.exe`）。
3. `PATH` 上の `ipfs`（Windows も `PATHEXT` は見ず `ipfs.exe` 固定）。
4. どれも無ければエラー（`Kubo binary not found: ...`）。

2・3 で見つけたときは解決したパスを `tracing::info!` で 1 行残す（`[kubo].binary` を明示した場合は出さない）。

### リポジトリの初期化（`kubo::ensure_repo`）

`<repo>/config` が無ければ `IPFS_PATH=<repo>` で `ipfs init` を実行する（あれば何もしない）。呼び出し元にリポジトリを新規作成したかどうかを bool で返す。`<repo>` ディレクトリ自体は無ければ先に作る。

### 適用する Kubo 設定（`kubo::apply_config`）

`swing up` は Kubo を起動するたびに `IPFS_PATH=<repo>` で次の `ipfs config` を順に実行する。

| キー | 値 | 備考 |
|---|---|---|
| `Datastore.StorageMax` | `[kubo].storage_max`（10 進バイト数の文字列、例 `"107374182400"`） | |
| `Provide.Strategy` | `[kubo].provide_strategy` | `mfs` か `all` を含めないと、MFS にしか無いサイトが DHT に告知されない。知らない値を与えると daemon が起動しない |
| `Gateway.NoFetch` | `true` | 毎回 `--json` で設定。応答の詳細は下記「[Kubo の Gateway](#kubo-の-gatewaynofetch)」 |
| `Gateway.NoDNSLink` | `true` | 毎回 `--json` で設定 |
| `Gateway.PublicGateways` | `[gateway].hosts` を `{"<host>": {"Paths": [], "UseSubdomains": false, "NoDNSLink": false}}` に変換したもの | hosts が空なら `{}` |
| `Addresses.API` | `["/ip4/127.0.0.1/tcp/<api_port>"]` | `api_port` は起動のたびに動的に選ぶ（下記） |
| `Addresses.Gateway` | `[kubo].gateway_listen` を multiaddr にしたもの（`/ip4/.../tcp/...` か `/ip6/.../tcp/...`） | |
| `Addresses.Swarm` | `[kubo].swarm_port` が `Some` のときだけ、Kubo の既定の Swarm リスト 8 本のポートをすべてこの値に置き換えたもの | `None` なら触らない（Kubo の既定のまま） |
| `API.Authorizations` | `{"swing": {"AuthSecret": "bearer:<secret>", "AllowedPaths": ["/api/v0"]}}` | `ipfs config` を使わず、最後に `<repo>/config` の JSON を読んで書き換える（秘密をコマンドラインに載せないため）。書き戻しは一時ファイル（unix は 0600）からの rename。値は下記「[RPC の認証](#rpc-の認証managed-のみ)」 |

`ipfs config` の実行が失敗したら stderr を含めてエラーにする。compose の外部 Kubo コンテナは `docker/kubo-init.d/001-swing-config.sh` で、このうち `Datastore.StorageMax`・`Provide.Strategy`・`Gateway.NoFetch`・`Gateway.NoDNSLink`・`Gateway.PublicGateways` の 5 つのキーを設定する（`Addresses.*` は設定しない。値の渡し方は [`docker.md#kubo-の設定`](docker.md#kubo-の設定)）。

### Kubo の Gateway（`NoFetch`）

`Gateway.NoFetch=true`（上記）により、Kubo の Gateway はローカルにあるブロックだけを返し、ネットワークからは取りに行かない。`Gateway.PublicGateways` に入れたホストでは DNSLink（`_dnslink.<host>`）の内容だけを返す。

Kubo は `Host` と `X-Forwarded-Host` をそのまま信じるので、Kubo の Gateway ポートを外部に直接公開しない。内蔵 gateway（[`gateway.md`](gateway.md)）や compose の `mirror` コンテナを前段に置く。

### RPC の認証（managed のみ）

managed の Kubo の RPC は Kubo の `API.Authorizations` で Bearer トークンを要求する。同じマシンの別のユーザーが RPC を叩いて、MFS の `publish/<自分>/...` に任意の CID を置く（agent がそれをレプリカ報告に載せて署名する）・設定を変える・ミラーを消す・止める、といったことを防ぐため。

- 秘密は 32 バイトの乱数の 16 進（`kubo::ApiSecret`。`Debug` は `<redacted>`）。`start_kubo`（`up.rs`）が Kubo を起動するたびに `kubo::rotate_api_secret` で作り直し、`<state_dir>/kubo-api.secret` に書く（`auth::write_private_file`。unix は 0600、`state_dir` が無ければ 0700 で作る）。書けなければその回の起動を失敗としてバックオフする。
- 作った秘密は `KuboSettings.api_secret` として `apply_config` が `API.Authorizations` に書き（上表）、`Daemon::spawn` に渡す。`Daemon` はヘルス待ちと RPC シャットダウンでこれを送る。agent に渡す設定のコピーには `config.ipfs.api_secret` として入れ（[`up.md#managed`](up.md#managed)）、ダッシュボードの統計（`KuboTarget`）の `IpfsClient` にも持たせる。
- クライアントは `ipfs::kubo_http_client(Some(&secret))` で作り、すべてのリクエストに既定のヘッダー `Authorization: Bearer <secret>`（sensitive 指定）を付ける。`IpfsClient::with_secret(api, secret)` がこれを使う（`IpfsClient::new(api)` は秘密なし）。
- `kubo::read_api_secret(state_dir)` はファイルを読む。無ければ `None`、中身が 16 進でなければエラー。孤児回収（下記）はこれで読んだ秘密を付けて RPC シャットダウンを送る（読めなければ warn を出して秘密なしで送る）。
- 秘密は Kubo を起動するたびに変わるので、止まった Kubo の古い `<repo>/api` のポートを別のプロセスが先に取っていて、`swing publish` がそこへ秘密を送ってしまっても、その秘密は次に起動する Kubo では使えない。

unmanaged（`[ipfs].api`）の Kubo には秘密を送らず、認証は SWING では設定しない。同じマシンの他のユーザーからその RPC に届くなら、それらのユーザーは上記の操作ができる。Docker Compose の構成では、Kubo の RPC（5001）はホストに公開せず compose の内部ネットワークだけで待ち受けるので、届くのは同じネットワークのコンテナ（`mirror`）だけ（[`docker.md`](docker.md)）。

### 動的な API ポートと `<repo>/api`

`kubo::pick_free_port()` が `127.0.0.1:0` を bind してすぐ解放し、空いている TCP ポートを 1 つ返す。`swing up` は Kubo を起動するたびにこれで API ポートを選び、`Addresses.API` に設定する。Kubo は起動時に実際に listen したアドレスを `<repo>/api` に multiaddr（例 `/ip4/127.0.0.1/tcp/54321`）で書き出す。

- `kubo::api_url_from_repo(repo)`: `<repo>/api` を読んで `multiaddr_to_http_url` で HTTP URL に変換する。ファイルが無ければ「Kubo is not running（`swing up` を起動するか、`[kubo].managed = false` にして `[ipfs].api` で外部の Kubo を指すよう案内する）」という趣旨のエラーにする。
- `kubo::multiaddr_to_http_url(addr)`: `/ip4/<ip>/tcp/<port>` → `http://<ip>:<port>`、`/ip6/<ip>/tcp/<port>` → `http://[<ip>]:<port>`（`[::1]` のように角括弧を付ける）。`/dns4`・`/dns6`・`/dns` も同様にホスト名をそのまま使う。それ以外のプロトコルや `tcp` 以外はエラー。
- `Config::ipfs_api_url()`（`src/config/mod.rs`）は `[ipfs].api` が `Url` ならそのまま返し、`Managed` なら `api_url_from_repo(&config.kubo.repo)` を呼ぶ。unmanaged の `swing up` のヘルス待ちはこれで URL を得る。
- `Config::ipfs_client()`（async）は RPC のクライアントを作る。`Url` ならその URL と `config.ipfs.api_secret`（設定ファイルからは入らず、`swing up` が managed の agent に渡すコピーにだけ入る）で作る。`Managed` なら `api_url_from_repo` の URL と `read_api_secret(<state_dir>)` の秘密で作り、`kubo::ensure_own_daemon` で、その API の `id` の `ID` が `<repo>/config` の `Identity.PeerID` と一致することを確かめる（一致しなければ `<repo>/api` が古いとしてエラー。PeerID が読めなくてもエラー）。CLI の `swing publish` はこれを経由して、`swing up` が管理している Kubo の実際のポートを見つける。agent（`agent/lifecycle.rs`）も同じ関数でクライアントを作る（managed の agent には `swing up` が `[ipfs].api` を実際の URL に差し替え、`api_secret` を入れた設定を渡す。[`up.md#managed`](up.md#managed)）。

`Daemon::wait_healthy` はこのファイルを読まない。起動直後はまだ `<repo>/api` が存在しないため、`swing up` は選んだポート番号から直接 `http://127.0.0.1:<api_port>` を組み立てて `Daemon` に渡し、それでヘルスチェックする。

Kubo の RPC を呼ぶ HTTP クライアントはすべて `ipfs::kubo_http_client(secret)`（`IpfsClient`・ヘルス待ち・RPC シャットダウン・孤児回収）で作り、プロキシの環境変数（`HTTP_PROXY` など）やシステムのプロキシ設定を使わない（`no_proxy`）。プロキシが `add` の `Hash` や `dag/stat` の応答を差し替えて、publish に別の CID へ署名させることを防ぐため。

### デーモンの起動（`kubo::Daemon::spawn`）

```
<bin> daemon --migrate=true --enable-gc --agent-version-suffix=swing
```

`Daemon::spawn(bin, repo, api_url, api_secret)` は起動する Kubo の API URL（`Addresses.API` に設定したものと同じ、`http://127.0.0.1:<api_port>`）と RPC の秘密を受け取り、秘密を付けるクライアントとともに `Daemon` に持たせる。`Daemon::wait_healthy` と `Daemon::stop`（下記）がこれを使う。

- `IPFS_PATH=<repo>`。stdin は `/dev/null` 相当、stdout/stderr は pipe。
- Linux（`cfg(target_os = "linux")`）のみ、`pre_exec` で `PR_SET_PDEATHSIG(SIGTERM)` を設定する。swing プロセスが SIGKILL 等で消えても、Linux では子の Kubo に SIGTERM が届く。
- Windows（`cfg(windows)`）のみ、`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` の Job Object に子プロセスを割り当てる。ハンドルは `Daemon` が持ち `Drop` で閉じるので、swing が強制終了されても Kubo は一緒に落ちる。macOS にはこの種の機構が無く、`kill_on_drop(true)` と次回起動時の孤児回収に頼る。
- 標準出力・標準エラーは 1 行ずつ `target: "kubo"` のログ（`stream` フィールド付き）に流す。標準エラーに `repo.lock` か `someone else has the lock` を含む行が出たら覚えておく（`"lock"` だけだと `block` に当たるため）（`Daemon::saw_repo_lock_error()`。下記「repo lock のヒント」）。

### ヘルス待ち（`kubo::wait_healthy` / `Daemon::wait_healthy`）

どちらも `POST <api_url>/api/v0/id` を 1 秒間隔で叩き、1 回ごとのリクエストタイムアウトは 5 秒。指定した `timeout` を超えたらエラー。

- `kubo::wait_healthy(api_url, timeout)`（unmanaged）: 2xx が返れば成功。
- `Daemon::wait_healthy(repo, timeout)`（managed）: 始めに `<repo>/config` の `Identity.PeerID` を 1 回読む。毎回、応答の後に子プロセスが終わっていないか（`try_wait`）を見て、終わっていれば待たずに `Kubo exited (<status>) before becoming healthy` でエラーにする。2xx の応答の `ID` が読んだ PeerID と一致したときだけ成功とし、別の ID を返す相手（同じポートの別プロセス）には成功しない（時間切れのエラーにその ID と期待した PeerID を入れる）。PeerID が読めなければ warn を出し、unmanaged と同じく 2xx だけで成功とする。

`timeout` は `up.rs` の定数で決まる。

- unmanaged: 30 秒（`UNMANAGED_HEALTH_TIMEOUT`）。
- managed: 120 秒（`MANAGED_HEALTH_TIMEOUT`）。

待機中も `CancellationToken` の cancel に即座に応答する（`tokio::select!` でヘルス待ちと `token.cancelled()` を競走させる）。

### 停止（`Daemon::stop(grace)`）

まず Kubo の RPC（`ipfs shutdown` と同じ）を叩き、それでも `grace` 以内に終わらなければ段階的に強制する 3 段構え（1・2 段目は全 OS 共通、3 段目が OS 依存）:

1. `POST <api_url>/api/v0/shutdown` をリクエストタイムアウト 5 秒（`SHUTDOWN_RPC_TIMEOUT`）で送る。レスポンスの成功・失敗・接続エラーのどれであっても「シャットダウンを要求した」ものとして次に進む（リトライしない）。
2. 子プロセスの終了を `grace` 秒まで待つ。終了すればここで成功。
3. まだ生きていれば: unix は SIGTERM を送って 10 秒（`SIGTERM_GRACE`）待ち、それでも終わらなければ `kill()`（SIGKILL）。Windows（`cfg(unix)` に入らない経路）は待たずに直接 `kill()`。

ログは SIGTERM の送信失敗と SIGTERM 後も終わらないことだけが warn で、他の段階は debug。`kill()` 自体の失敗はエラーとして返す。

`kubo::daemon_stop_budget(grace)` はこの 3 段の最悪時間（5 秒 + `grace` + unix は 10 秒、Windows は 0 秒。SIGKILL 後の終了待ちは数えない）を返す。`swing up` は `DAEMON_STOP_GRACE = 20s`（`up.rs`）を渡すので最悪 35 秒（Windows は 25 秒）。`swing stop`（[`cli.md#stop`](cli.md#stop)）による停止もシグナルによる停止もこの `Daemon::stop` を通り、どちらもこの 3 段を待ち切れる（シグナル時の時間予算は [`up.md#停止の時間予算`](up.md#停止の時間予算)）。

### `kubo.pid` と孤児 Kubo の回収（managed のみ）

`start_kubo`（`up.rs`）は `Daemon::spawn` の直後に `<state_dir>/kubo.pid`（JSON: `pid`・`api_port`・`started_at`）を書く。`started_at` はその `pid` の開始時刻を OS ごとの方法（Linux は `/proc/<pid>/stat`、macOS は `ps -o lstart=`、Windows は `GetProcessTimes`）で取った比較専用の文字列。書けなければ warn を出して続行する（その回は孤児回収の対象にならない）。

`kubo::recover_orphan` は `run_managed` の冒頭（バイナリの検出・バージョン確認の後、デーモンループの前）に 1 回呼ぶ。`swing.lock` を持っている間なので、記録にある Kubo が生きていればそれは前回の swing の孤児である。

- `kubo.pid` が無ければ何もしない。読めなければ warn を出してファイルを消すだけで、何も kill しない。
- まずその `pid` の今の開始時刻を取り直し、記録と比べる。プロセスがもう無い、または開始時刻が一致しない（PID の再利用）なら、API にも何も送らず kill もせずにファイルを消す。
- 一致したら記録の `api_port` に API でのシャットダウンを `<state_dir>/kubo-api.secret` の秘密を付けて送り（タイムアウト 3 秒、`ORPHAN_SHUTDOWN_RPC_TIMEOUT`）、応答があればその `pid` の終了を最大 30 秒（`ORPHAN_SHUTDOWN_GRACE`）待つ。終われば完了。
- API で終わらなければ強制終了する（unix は SIGTERM → 最大 30 秒（`ORPHAN_SIGTERM_GRACE`）→ SIGKILL → 最大 10 秒（`ORPHAN_KILL_WAIT`）、Windows は `taskkill /T /F` → 最大 10 秒（`ORPHAN_KILL_WAIT`））。
- 強制終了しても終わらなければエラーを返し、`swing up` は Kubo を起動せずに終了する。
- `swing up` が Kubo を止めるとき（`up.rs` の `stop_daemon`）は、`Daemon::stop` が成功したときだけ `kubo.pid` を消す。失敗したら（Kubo が残っているかもしれないので）warn を出してファイルを残し、次の起動の `recover_orphan` に任せる。Kubo が自分で exit したとき（[`up.md#managed`](up.md#managed)）は消す。
- `run_managed` は回収中にトークンが cancel されたら（シグナルなど）回収を途中でやめて終わる。`kubo.pid` は残り、次の起動でもう一度回収する。

#### repo lock のヒント

`recover_orphan` が拾えるのは自分が書いた `kubo.pid` だけで、`swing up` の管理下に無い Kubo が同じ repo を使っていると起動が失敗し続ける。`Daemon::wait_healthy` が失敗した時点で daemon が exit していて、標準エラーに repo lock の行（上記）が出ていたら、`another ipfs daemon seems to hold the Kubo repo lock; ...` を warn で出してから通常のバックオフに入る。

## MFS の使い方

agent も publish も pin を使わず、MFS にサイトの CID を置いて GC から守る。同じ CID を複数の場所に置いても、すべての場所から消えるまで GC されない。`mfs_root` の外や手動の pin には触れない。

| パス | 持ち主 |
|---|---|
| `<mfs_root>/agent/<pubkey hex>/<site>/<created_at>` | agent。版ごとに 1 つ。この下は agent だけが使い、state が参照しない項目は sweep で消す |
| `<mfs_root>/publish/<pubkey hex>/<site>/<created_at>` | publish。自分のサイトの版ごとに 1 つ |

- `<site>` は `d` のパーセントエンコード（`A-Z a-z 0-9 - . _ ~` 以外を `%XX`）。`d` が `.` か `..` ならドットも `%2E` にする。
- `<created_at>` はサイトイベントの `created_at`（10 進）。

MFS は DAG が欠けていても置け、GC も `block/rm` も止めない。そのため置いた後の完全性は `dag/stat`（`offline=true`）で確かめる。

MFS から消したコンテンツや打ち切った取得のブロックは、Kubo の GC で消える。

## RPC

すべて `POST /api/v0/...`。CID とパスはクエリに入れる前にパーセントエンコードする（パスは要素ごと。`<site>` のエンコードと合わせて二重になる）。非 2xx はボディ付きのエラーにする。`dag/export` 以外の応答のボディ（エラーのボディを含む）は 16 MiB（`ipfs::MAX_RESPONSE_BYTES`）までしか読まず、超えたらエラーにする。`dag/export` の成功時のボディは読み捨てながら `max_bytes` で打ち切る。

| 操作 | リクエスト | タイムアウト |
|---|---|---|
| 取得 | `dag/export?arg={cid}&progress=false` | 全体 `[agent].fetch_timeout`、無通信 `[agent].fetch_idle_timeout` |
| 実サイズ・完全性 | `dag/stat?arg={cid}[&arg={cid}...]&progress=false&offline=true` → `TotalSize` | 300 秒 |
| ディレクトリ作成 | `files/mkdir?arg={path}&parents=true` | 60 秒 |
| 配置 | `files/cp?arg=/ipfs/{cid}&arg={path}&offline=true` | 60 秒 |
| 削除 | `files/rm?arg={path}&recursive=true&force=true` | 60 秒 |
| 一覧 | `files/ls?arg={path}&long=true` → `Entries`（`Type` 1 がディレクトリ） | 60 秒 |
| CID の確認 | `files/stat?arg={path}&hash=true` → `Hash` | 60 秒 |
| ディレクトリ判定 | `files/stat?arg=/ipfs/{cid}` → `Type`（`directory` か `file`） | 60 秒 |
| PeerID | `id` → `ID` | 10 秒 |
| add（publish） | `add?recursive=true&cid-version=1&pin=false&quieter=true&wrap-with-directory=false&to-files={path}` | 300 秒 |

- `dag/stat` に CID を複数渡すと、`TotalSize` はそれらをまとめた重複排除後のサイズ（同じブロックを 1 回だけ数えた合計）になる。1 つでもブロックが欠けていれば呼び出し全体が失敗する。CID を 1 つも渡さないときは呼ばずに 0 を返す。
- `dag/export` は最初のブロックが取れるまでヘッダーを返さないので、無通信タイムアウトはヘッダー受信までにも適用する。
- 配置は親ディレクトリを作り、同名の項目を消してから行う（同名があると `files/cp` が失敗する）。`offline=true` なのでルートのブロックがローカルに無ければ即エラー。
- `files/rm` は失敗しても 200 でボディにメッセージを返すので、ボディが空でなければ失敗とする。存在しないパスは成功。
- `files/ls` と `files/stat` の `file does not exist` は、それぞれ空の一覧、「無い」として扱う。
- ディレクトリ判定は MFS のパスではなく `/ipfs/{cid}` を `files/stat` に渡す。agent は取得の直後に呼ぶ（[`agent.md` の「保存の順序」](agent.md#保存の順序)）ので、ルートブロックはローカルにある。

`add` の multipart:

- 各ファイルは `name="file"` パート。`filename` はルートディレクトリ名を先頭に付けた相対パス（例: `public/css/style.css`、URL エンコード）。
- ファイルは `application/octet-stream` でストリーミング送信、ディレクトリは空ボディの `application/x-directory`。
- シンボリックリンクは辿る。ただし、リンク先を `canonicalize` した実パスが、`canonicalize` したルートディレクトリの下に無ければエラーにして何も追加しない（`DIR` の外のファイルを公開しないため）。循環もエラー。
- 最後の JSON 行の `Hash` がルート CID（`ipfs add -Qr --cid-version=1` と同じ）。
- `to-files` のパスにルートディレクトリそのものが置かれる。add の前に親ディレクトリを作り、同じパスの既存の項目を `files/rm` で消す。

## Kubo のバージョン

`swing up`（managed）は `run_managed` の始め（`up::run` が呼ばれるたび）に `ipfs version --number` を実行し、`kubo::KUBO_VERSION` と異なれば warn を出して続行する。`ipfs version` 自体が実行できなければ `swing up` はエラー終了する。

上げるときに揃える場所:

- `compose.yaml` の `ipfs` サービスのイメージタグ（`ipfs/kubo:v0.43.1`）
- `kubo::KUBO_VERSION`（`src/kubo.rs`）
- README と docs の版表記（`v0.43.1` で検索できる）

次の Kubo の挙動に依存しているので、上げると壊れうる。

- `file does not exist` の文面での判定（変わると、突き合わせが MFS から消えた版を取り直さず警告を出し続ける）
- `files/rm` が失敗時も 200 を返すこと
- 各 RPC の JSON の形（`TotalSize`、`Hash`、`Type`（`files/stat` は文字列、`Entries[].Type` は数値）など）と、`add` の multipart・`to-files`
- MFS の保護・GC・`offline=true` の挙動

これを確かめるテストは、統合テスト（`kubo_integration`・`agent_stores_and_removes_through_real_kubo`）と `kubo::tests` の `#[ignore]` テスト（`SWING_TEST_KUBO_BIN` が必要）。コマンドは [`../architecture.md#テスト`](../architecture.md#テスト)。
