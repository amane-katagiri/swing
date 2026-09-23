# 2026-09-23 配布方式の実装

[配布方式の設計](2026-09-21-distribution-design.md)の実装。実装前に細部の仕様を決めてから着手した。本文には、そのうち設計ログにまだ無いものだけを書く。architecture 側の結果は [`up.md`](../architecture/up.md)・[`gateway.md`](../architecture/gateway.md)・[`service.md`](../architecture/service.md)・[`docker.md`](../architecture/docker.md)・[`kubo.md`](../architecture/kubo.md)・[`agent.md`](../architecture/agent.md)・[`cli.md`](../architecture/cli.md)・[`dashboard.md`](../architecture/dashboard.md)・[`../architecture.md`](../architecture.md) に反映済み（このログには経緯だけを書く）。

## きっかけ

設計ログで方針を決めた 4 点（`swing up` supervisor、内蔵 gateway、`swing service`、Kubo 設定の `swing.toml` への集約と RPC の動的ポート化）を実装する回。設計ログは方針とその根拠だけで実装には入っていないので、今回は実装しながら確定した設定の形・型・挙動の細部を決めている。

## 決めたこと（設計ログに無い、今回確定した細部）

- `[ipfs].api` を `IpfsApi::Url(String) | IpfsApi::Managed` の enum にする。`[kubo].managed = true` かつ `[ipfs].api` が明示されていたら設定エラーにする（両方が意味を持つ状態を作らない）。`Config::ipfs_api_url()` が `Managed` を実際の URL に解決する唯一の入口で、Kubo RPC を呼ぶ全モジュールはこれを経由する。
- `[kubo]`/`[gateway]` の 2 セクションを新設。キー名・既定値は `swing.example.toml` のとおり（`managed`・`binary`・`repo`・`storage_max`・`provide_strategy`・`gateway_listen`・`swarm_port` と、`listen`・`hosts`・`upstream`）。`repo` の既定は `<[agent].state_dir>/kubo`、`storage_max` の既定は `[policy].max_total_storage` と同値、`gateway.upstream` の既定は managed なら `http://<[kubo].gateway_listen>`、そうでなければ `http://127.0.0.1:8080`。
- Kubo の RPC アドレスをファイル経由で受け渡す方式にする（Unix ソケットではなく）。`swing up` が起動のたびに `pick_free_port()` で空いている loopback ポートを選んで `Addresses.API` に設定し、Kubo 自身が実際に listen したアドレスを `<repo>/api` に書き出す。`kubo::api_url_from_repo` がこれを読んで他のサブコマンド（`swing agent`・`swing status` 等）に渡す。起動直後はまだこのファイルが無いため、`wait_healthy` は選んだポート番号から直接 URL を組み立てて待つ（`api` ファイルの出現は待たない）。
- `managed` の既定値は `true`（単体で `swing up` を実行したときに Docker 無しで動くことを優先）。compose の `mirror` サービスだけが `SWING_KUBO_MANAGED=false` を明示的に渡して従来どおり外部の `ipfs` コンテナを使う。
- compose の `gateway`（Caddy）サービスと `docker/caddy/` を削除し、内蔵 gateway に置き換える。トレードオフとして、`mirror` コンテナの 8081（gateway 用ホストポート）は `[gateway].listen` の有効・無効に関わらず常に compose がマッピングする（compose 側の `ports:` は起動時固定で、コンテナ内の設定を見て動的に変えられないため）。gateway を使わない構成でもポートだけは空いている。
- `swing up` は managed・unmanaged のどちらも同じサブコマンドで扱う。Kubo が落ちたときは agent も含めて丸ごと再起動し（Kubo の状態と agent の内部状態がずれるのを避ける）、agent 自身がエラーで落ちたときは Kubo はそのままに agent だけ再起動する（Kubo の再起動はコストが高く、agent 側のエラーの大半は relay やパースの一時的な失敗のため）。バックオフは 1 秒から倍々で最大 60 秒、直前の起動が 60 秒以上生きていれば 1 秒にリセットする。
- Windows のサービス登録は `LogonType = S4U` を選ぶ。`InteractiveToken` はログオン時にコンソールウィンドウが開いてしまい常駐サービスとして不自然になるため。`sc.exe` によるサービス登録は管理者権限と UAC が要るため使わない（設計ログの結論どおり）。タスクの XML には環境変数を書けないので、ログファイルの指定だけは `swing up --log-file <path>` という CLI 引数で渡す（`service install` が Windows のときだけこの引数を足す。Linux/macOS は OS 側のログリダイレクト機構があるので付けない）。
- Linux の子プロセス回収に `PR_SET_PDEATHSIG(SIGTERM)` を使う（`target_os = "linux"` のみ有効。macOS は同じ unix でもこの機構が無く、Windows と同様 `kill_on_drop` のみに頼る）。
- Kubo のバージョン不一致は warn に留めてブロックしない（エラーにはしない）。バイナリの配布方法（パッケージマネージャ経由か同梱か）が環境によって違い、パッチバージョンの差で起動を拒否すると配布の柔軟性を損なうため。
- Kubo の自己展開（swing に埋め込んで起動時に展開する方式）は採らない、という設計ログの結論を実装でも維持。`kubo::locate_binary` はあくまで「探す」だけで、取得・展開は行わない。

## やったこと

モジュール別の実装内容は各 architecture ドキュメントに反映済み。要点だけ:

- `config.rs`: `KuboConfig`・`GatewayConfig`・`IpfsApi`・`Listen`（旧 `DashboardListen` を改名し dashboard/gateway で共有）を追加。`resolve_config_path` を `pub`化（`service.rs` から使うため）。`is_valid_gateway_host` は `docker/kubo-init.d/001-swing-config.sh` のホスト名検証と同じ規則。
- `kubo.rs`: バイナリ検出・バージョン確認・repo 初期化・`ipfs config` 適用・子プロセスの起動/停止・ヘルス待ちを新設。
- `up.rs`: supervisor 本体（`Backoff`、managed/unmanaged の 2 ループ）。
- `gateway.rs`: axum ベースの内蔵 gateway（Host 振り分け、ヘッダーの転送/破棄、ストリーミング）。
- `service.rs`: OS ごとの unit/plist/XML を作る純粋関数と、実際に登録・削除・状態確認するコマンド実行部分。
- `shutdown.rs`: `tokio::signal::unix` 直書きをやめ、`CancellationToken` ベースの共通シャットダウン待受けに切り出し（`agent::run` と `up::run` の両方から使う）。
- `agent/lifecycle.rs`: `run` を `cancel_on_signal` + `run_until` の薄いラッパーにし、`run_until` は `CancellationToken` を受け取る形に変更。内蔵 gateway をダッシュボードと同じ枠組み（bind はループ開始前、spawn は reconcile 後、終了は子トークンの cancel + 最大 5 秒待ち）で組み込んだ。
- `main.rs`: `swing up [--log-file]`・`swing service install/uninstall/status` を追加。`Cli::parse()` を `init_tracing` より先に呼ぶよう順序を変更（`--log-file` の値がトレーシング初期化に必要なため）。
- `Dockerfile`/`compose.yaml`: `CMD` を `up` に、`mirror` に `SWING_KUBO_MANAGED=false`・`SWING_GATEWAY_LISTEN`・`SWING_GATEWAY_UPSTREAM` を追加、`gateway`（Caddy）サービスと `docker/caddy/` を削除。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（ユニット）を通した。
- `kubo::tests::full_lifecycle_against_real_kubo`（`#[ignore]`、`SWING_TEST_KUBO_BIN` で実 Kubo 0.43.1 を指定）: init → `profile apply test` → `apply_config` → `spawn` → `wait_healthy` → 書き込んだ `ipfs config` の内容検証 → `stop` を通した。
- スモークテスト（実 Kubo 0.43.1 + ローカル nostr-rs-relay、公開ネットワーク不使用）:
  - `swing up` で repo 初期化 → Kubo 起動（動的ポートが `<repo>/api` に反映される）→ relay 接続 → dashboard・gateway が listen することを確認。
  - `/api/config` の `ipfs.api` が `"managed"` になっていることを確認。
  - gateway が非許可 Host に 404、許可 Host は Kubo gateway にそのまま中継し、応答が Kubo に直接アクセスした場合とバイト一致することを確認。
  - `swing status` が `<repo>/api` から managed な Kubo を見つけて動作することを確認。
  - Kubo の子プロセスを SIGTERM で殺すと約 1 秒後に別ポートで再起動し、agent も再接続することを確認。
  - `swing up` 自体に SIGTERM を送ると 1 秒以内に終了し、`ipfs daemon` プロセスが残留しないことを確認。
- クロスコンパイル確認（`cargo check --target x86_64-pc-windows-msvc` / `aarch64-apple-darwin`）はこの開発環境では `ring` の C ビルドが Windows SDK / macOS SDK を要求して通らなかった。`cfg(target_os = ...)` の分岐は目視で確認したのみで、Windows・macOS での実際のコンパイル・実動作（サービス登録、ファイアウォール、S4U の実際の挙動を含む）は未確認のまま。

## 見送ったこと

- 設計ログで既に見送りを決めている項目（自己展開、GUI インストーラ、Rust 製 IPFS 実装の組み込み、compose 埋め込みランチャー、SignPath の無料証明書）は今回も維持し、実装しなかった。
- Kubo RPC を Unix ソケットにする案は採らず、loopback の動的ポート + `<repo>/api` ファイルの組み合わせにした。Windows は名前付きパイプが必要になり実装が複雑化するのに対し、loopback ポートは全 OS で同じコードパスにできるため。
- Windows/macOS での子プロセスの確実な回収（Job Object や `launchd` の `KeepAlive` 相当の仕組みを swing 自身が持つこと）は見送った。`kill_on_drop` と（Linux のみ）`PR_SET_PDEATHSIG` に留めている。

## 未確認（todo に転記済み、[`../todo.md`](../todo.md) を参照）

- Windows: タスクスケジューラの S4U ログオン（非対話セッション）で Kubo の 4001 に対するファイアウォールのダイアログが出ず、inbound が黙って遮られる可能性。`netsh advfirewall` でのルール追加は管理者権限が要る。
- Windows/macOS: swing が SIGKILL 相当で死んだときの子 Kubo の回収。孤児化した Kubo が repo.lock を持ったままだと次の `swing up` が起動ループしうる。
- Windows/macOS での実際のコンパイル確認（CI か実機）。
- gateway の TLS（`rustls-acme`）は未実装。
- 既存 compose 利用者が `ipfs-data` ボリュームから `swing up`（managed Kubo）へ移行する手順は未作成。
- `swing up` の二重起動検出は Kubo の repo.lock と dashboard の bind 失敗に任せていて、明示的な検出は無い。
