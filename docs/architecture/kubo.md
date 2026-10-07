# Kubo（kubo/）

[`../architecture.md`](../architecture.md) の一部。`swing up`（[`up.md`](up.md)）が managed の Kubo を扱うときのバイナリの検出・repo・設定・RPC の認証と、Kubo の版。呼ぶ順序は [`up.md#managed`](up.md#managed)。デーモンの起動・ヘルス待ち・停止・孤児回収は子ページの [`kubo/daemon.md`](kubo/daemon.md)、MFS の使い方と RPC（managed／unmanaged 共通）は [`mfs.md`](mfs.md)、内蔵 gateway は [`gateway.md`](gateway.md)。

## バイナリの検出（`kubo::locate_binary`）

優先順位:

1. `[kubo].binary` が指定されていればそのパス。ファイルが無ければエラー。
2. `swing` 実行ファイル（`current_exe()`）と同じディレクトリの `ipfs`（Windows は `ipfs.exe`）。
3. `PATH` 上の `ipfs`（Windows も `PATHEXT` は見ず `ipfs.exe` 固定）。絶対パスでない要素（空や `.` など）は飛ばす（カレントディレクトリの `ipfs` を実行しないため）。
4. どれも無ければ `Kubo binary not found: ...` のエラー。

2・3 で見つけたときは解決したパスを info ログに出す。

## リポジトリの初期化（`kubo::ensure_repo`）

`<repo>` ディレクトリが無ければ作り（unix は 0700。既にあるディレクトリの権限は変えない）、`<repo>/config` が無ければ `IPFS_PATH=<repo>` で `ipfs init` を実行する。

SWING が起動する Kubo のプロセス（`ipfs init`・`ipfs version`・`ipfs daemon`）には、設定カタログ（`settings::SETTINGS`）で種類が秘密のキーの環境変数（`SWING_NOSTR_SECRET_KEY`）を渡さない。

## 適用する Kubo 設定（`kubo::apply_config`）

`swing up` は Kubo を起動するたびに `<repo>/config` の JSON を読み、次のキーを書き換えて 1 回で書き戻す。

| キー | 値 | 備考 |
|---|---|---|
| `Datastore.StorageMax` | `[kubo].storage_max`（10 進バイト数の文字列、例 `"107374182400"`） | |
| `Provide.Strategy` | `[kubo].provide_strategy`（文字列） | `mfs` か `all` を含めないと、MFS にしか無いサイトが DHT に告知されない。知らない値を与えると daemon が起動しない |
| `Gateway.NoFetch` | `true` | 下記「[Kubo の Gateway](#kubo-の-gatewaynofetch)」 |
| `Gateway.NoDNSLink` | `true` | |
| `Gateway.PublicGateways` | `[gateway].hosts` を `{"<host>": {"Paths": [], "UseSubdomains": false, "NoDNSLink": false}}` に変換したもの | hosts が空なら `{}` |
| `Addresses.Gateway` | `[kubo].gateway_listen` を multiaddr（`/ip4/.../tcp/...` か `/ip6/.../tcp/...`）にした 1 要素の配列 | |
| `Addresses.Swarm` | `[kubo].swarm_port` があるときだけ、Kubo の既定の Swarm リスト 8 本（TCP・QUIC・WebTransport・WebRTC の IPv4／IPv6）のポートをすべてこの値にしたもの | 無ければ触らない |
| `Addresses.API` | `["/ip4/127.0.0.1/tcp/<api_port>"]` | 下記「[動的な API ポート](#動的な-api-ポートと-repoapi)」 |
| `API.Authorizations` | `{"swing": {"AuthSecret": "bearer:<secret>", "AllowedPaths": ["/api/v0"]}}` | 下記「[RPC の認証](#rpc-の認証managed-のみ)」 |

- 親のオブジェクト（`Datastore`・`Provide`・`Gateway`・`Addresses`・`API`）が無いか `null` なら空のオブジェクトを作り、ほかのキーはそのまま残す。親がオブジェクト以外ならそのキーを名指しするエラーにする。
- RPC の秘密を他のユーザーが読めるコマンドラインに載せないため、`ipfs config` は使わない。書き戻しは `settings::write_atomic`（一時ファイル（unix は 0600）からの rename。`<repo>/config` がシンボリックリンクならリンク先を置き換え、リンクは残す）。

compose の外部 Kubo コンテナでの同等の設定は [`docker.md#kubo-の設定`](docker.md#kubo-の設定)。

### Kubo の Gateway（`NoFetch`）

`Gateway.NoFetch=true` により、Kubo の Gateway はローカルにあるブロックだけを返し、ネットワークからは取りに行かない。`Gateway.PublicGateways` に入れたホストでは DNSLink（`_dnslink.<host>`）の内容だけを返す。

Kubo は `Host` と `X-Forwarded-Host` をそのまま信じるので、Kubo の Gateway ポートを外部に直接公開しない。内蔵 gateway（[`gateway.md`](gateway.md)）や compose の `mirror` コンテナを前段に置く。

## RPC の認証（managed のみ）

managed の Kubo の RPC は `API.Authorizations` で Bearer トークンを要求する。

- 秘密は 32 バイトの乱数の 16 進（`kubo::ApiSecret`。`Debug` は `<redacted>`）。API のポートと組にした `kubo::ApiAccess { port, secret }` として扱い、Kubo を起動するたびに作り直す。
- `ApiAccess::client()` が `http://127.0.0.1:<port>` 宛てで、すべてのリクエストに `Authorization: Bearer <secret>`（sensitive 指定）を付けるクライアントを作る。
- ほかのプロセス（`swing publish`）への受け渡し口は `<state_dir>/kubo-api.json`（JSON: `port`・`secret`。`auth::write_private_file` で書く。unix は 0600、`state_dir` が無ければ 0700 で作る）。同じプロセスの agent とダッシュボードにはメモリで渡す（agent には `config.ipfs.api_secret`）。書く・消すタイミングは [`up.md#managed`](up.md#managed)。クライアントはポートと秘密を必ずこのファイルから組で読む。読むときは、無ければ「無い」、JSON でないか `secret` が 16 進でなければエラー。

unmanaged（`[ipfs].api`）の Kubo には秘密を送らず、認証は SWING では設定しない。Docker Compose の構成での RPC の公開範囲は [`docker.md`](docker.md)。

### 既知の制限: API ポートの認証のないエンドポイント

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

`swing up` は Kubo を起動するたびに `kubo::pick_free_port()`（`127.0.0.1:0` を bind してすぐ解放する）で空いている TCP ポートを選び、`Addresses.API` に設定する。Kubo は API を listen できた後で、そのアドレスを `<repo>/api` に multiaddr（例 `/ip4/127.0.0.1/tcp/54321`）で書き出す。

- `<repo>/api` を読むのは managed のヘルス待ち（[`kubo/daemon.md`](kubo/daemon.md#ヘルス待ちkubowait_healthy--daemonwait_healthy)）だけで、listen したのが自分の起動した Kubo であることの確認に使う。ほかのプロセスは `kubo-api.json`（上記）を読む。
- `kubo::multiaddr_to_http_url(addr)`: `/ip4/<ip>/tcp/<port>` → `http://<ip>:<port>`、`/ip6/<ip>/tcp/<port>` → `http://[<ip>]:<port>`。`/dns4`・`/dns6`・`/dns` はホスト名をそのまま使う。それ以外の形やホストが空ならエラー。

## RPC クライアントの作り方（`Config::ipfs_client()`）

agent・`swing publish`・unmanaged のヘルス待ち（[`up.md#unmanaged`](up.md#unmanaged)）が RPC のクライアントをこれで作る。

- `[ipfs].api` が URL: その URL と `config.ipfs.api_secret` で作る。`api_secret` は設定ファイルからは入らず、`swing up` が managed の agent に渡すコピー（`[ipfs].api` を実際の URL に差し替えたもの）にだけ入る。
- `managed`: `kubo::managed_client` が `kubo-api.json` のポートと秘密でクライアントを作り、`kubo::ensure_own_daemon` で、その API の `id` の `ID` が `<repo>/config` の `Identity.PeerID` と一致することを確かめる。一致しない、または PeerID が読めなければエラー。ファイルが無ければ `Kubo is not running: ...`（`swing up` を起動するか、`[kubo].managed = false` と `[ipfs].api` で外部の Kubo を指すよう案内する）のエラー。

## Kubo のバージョン

`swing up`（managed）は `ipfs version --number` を実行し（呼ぶ時点は [`up.md#managed`](up.md#managed)）、`kubo::KUBO_VERSION` と異なれば warn を出して続行する。`ipfs version` 自体が実行できなければ `swing up` はエラー終了する。

上げるときに揃える場所:

- `compose.yaml` の `ipfs` サービスのイメージタグ（`ipfs/kubo:v0.43.1`）
- `kubo::KUBO_VERSION`（`src/kubo/mod.rs`）
- `packaging/linux/install.sh` の `KUBO_VERSION` と `KUBO_SHA512_AMD64`・`KUBO_SHA512_ARM64`（<https://dist.ipfs.tech/kubo/> の `kubo_v<版>_linux-<amd64|arm64>.tar.gz.sha512` の値。[`install-sh.md`](install-sh.md)）
- `packaging/windows/kubo.sha512`（`kubo_v<版>_windows-amd64.zip.sha512` をそのまま置く。[`installer-windows.md`](installer-windows.md#kubo-の取得と検証)）
- README と docs の版表記（`v0.43.1` で検索できる）

2 つのインストーラーの更新漏れは `kubo::tests` の `install_sh_pins_the_same_kubo_version`（版の一致と固定値が 128 桁の 16 進であること）と `windows_installer_pins_the_same_kubo_version`（ファイル名の版の一致）が検出する。

次の Kubo の挙動に依存しているので、上げると壊れうる。

- `file does not exist` の文面での判定（変わると、突き合わせが MFS から消えた版を取り直さず警告を出し続ける）
- `files/rm` が失敗時も 200 を返すこと
- 各 RPC の JSON の形（`TotalSize`、`Hash`、`Type`（`files/stat` は文字列、`Entries[].Type` は数値）など）と、`add` の multipart・`to-files`
- MFS の保護・GC・`offline=true` の挙動

これを確かめるテストは、統合テスト（`kubo_integration`・`agent_stores_and_removes_through_real_kubo`）と、`SWING_TEST_KUBO_BIN` が要る `kubo::tests` の `#[ignore]` テストと `mfs_kubo_integration`。コマンドは [`../architecture.md#テスト`](../architecture.md#テスト)。
