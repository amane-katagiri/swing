# SWING アーキテクチャ

今のコードが何をしているかのリファレンス。イベント形式は [`protocol.md`](protocol.md)、経緯と理由は [`log/`](log/) を参照。

| 文書 | 内容 |
|---|---|
| このファイル | 構成、CLI の一覧、設定、テストと各ファイルへの索引 |
| [`architecture/cli.md`](architecture/cli.md) | 各サブコマンドの動作と出力 |
| [`architecture/agent.md`](architecture/agent.md) | mirror-agent の動作、ポリシー判定、レプリカ報告の送信、`state.json` |
| [`architecture/signer.md`](architecture/signer.md) | 署名（`signer/`）: 秘密鍵か NIP-46 の署名アプリか、`remote-signer.json`、QR コードでのペアリング |
| [`architecture/nip05.md`](architecture/nip05.md) | NIP-05 検証（agent と publish で共通） |
| [`architecture/nostr.md`](architecture/nostr.md) | Nostr イベントの検証（`nostr/`）、取得と表示の上限（`nostr::budget`）、レプリカ報告の信頼度 |
| [`architecture/kubo.md`](architecture/kubo.md) | MFS の使い方、Kubo RPC、Kubo プロセスの管理、Kubo のバージョン |
| [`architecture/up.md`](architecture/up.md) | `swing up`（supervisor）: セットアップモード、終了要求・シグナル、多重起動の防止 |
| [`architecture/stats.md`](architecture/stats.md) | リソース使用量の記録（`stats.rs`）: `swing up` が測る CPU・メモリ・IPFS の通信量、間隔と保持期間、OS ごとの取り方 |
| [`architecture/gateway.md`](architecture/gateway.md) | 内蔵 gateway（`gateway.rs`）: Host 振り分けと Kubo gateway へのプロキシ |
| [`architecture/service.md`](architecture/service.md) | `swing service install / uninstall / start / stop / status`（systemd / launchd / タスクスケジューラ） |
| [`architecture/tray.md`](architecture/tray.md) | タスクトレイ（`swing-tray`、Windows と macOS）: メニュー、状態の表示、ダッシュボード API の使い方 |
| [`architecture/docker.md`](architecture/docker.md) | Dockerfile、compose、外部 Kubo コンテナの設定（compose から `swing up` への移行手順は README） |
| [`architecture/dashboard.md`](architecture/dashboard.md) | `swing up` の寿命で常時動く Web ダッシュボード兼制御 API（起動と終了、設定、ガード、静的ファイル、agent 未準備時の扱い） |
| [`architecture/dashboard/http-api.md`](architecture/dashboard/http-api.md) | ダッシュボードの HTTP API |
| [`architecture/dashboard/web.md`](architecture/dashboard/web.md) | ダッシュボードの画面（Desktop 以外）と CSS カスタマイズ（`dashboard.md` の子ページ） |
| [`architecture/dashboard/desktop.md`](architecture/dashboard/desktop.md) | ダッシュボードの Desktop 画面と、どの画面でも動く更新の確認・おしらせの出し分け・ブラウザの通知（`dashboard.md` の子ページで `web.md` と並列） |
| [`architecture/dashboard/mascot.md`](architecture/dashboard/mascot.md) | Desktop 画面のマスコット: パック形式・ふるまい・当たり判定・吹き出し・おしらせ（`desktop.md` の子ページ） |
| [`architecture/release.md`](architecture/release.md) | ビルド（Windows 向けのクロスビルドを含む）とリリース（`.github/workflows/release.yml`）、macOS と Windows の動作確認（`.github/workflows/macos-check.yml`・`windows-check.yml`） |

## 構成要素

| 要素 | 実体 |
|---|---|
| 言語・ランタイム | Rust (edition 2024)、`tokio` |
| Nostr | `nostr-sdk` 0.45、NIP-46 に `nostr-connect` 0.45 |
| QR コード | `qrcode`（SVG だけ） |
| Kubo RPC | `reqwest`（rustls、multipart、stream）で直接呼ぶ |
| ダッシュボードの HTTP サーバ | `axum` 0.8、リクエストタイムアウトに `tower-http` |
| 設定 | `toml` + `serde`、環境変数が TOML を上書き |
| CLI | `clap` derive |
| ログ | `tracing` + `tracing-subscriber`（`RUST_LOG`。既定は `swing up` が `info`、ほかのコマンドは relay への接続・切断のログが出力に混ざらないよう `info,nostr_sdk=warn,nostr_connect=warn`） |
| CID 検証 | `cid` クレート |

クレート `swing` は lib + bin 構成。統合テストは `swing::` としてモジュールを直接使う。

## リポジトリ構成

```
swing/
  src/
    lib.rs           各モジュールを公開するクレートルート
    activity.rs      agent とダッシュボードが共有する最新の publish・レプリカ報告の時刻（`/api/activity` 用。メモリだけに持つ）
    main.rs          CLI エントリ (clap)
    config/          設定読み込み、サイズ・時間パーサ（mod.rs: 型・パーサ・`Config::load`、build.rs: `Resolver` とセクションごとの解決関数に分けた `build_config`、build/tests.rs: そのテスト）
    nostr/           Nostr イベントまわり（mod.rs: 未来ずれの許容・上限 `budget`・replaceable の新旧比較、client.rs: relay 接続・取得・購読・発行と送信結果、site.rs: site event の検証・組み立て・パース・最新版の選択、follow.rs: follow set の選択と p タグの取り出し、report.rs: レプリカ報告の組み立て・パース）
    ipfs.rs          Kubo RPC クライアント
    mfs.rs           MFS 上のパスの組み立て
    key.rs           key generate
    format.rs        バイト数・秒数の表示用フォーマット（format_bytes / format_bytes_approx / format_duration_secs）
    policy.rs        保存ポリシー判定（純粋関数）
    state.rs         state.json の永続化
    agent/           mirror-agent ループ。詳細は architecture/agent.md
    health.rs        版と MFS の突き合わせ（agent と status で共通）、status サブコマンド
    publish.rs       publish サブコマンド
    publish/checks.rs publish 前後のサイトの確認（ドットファイル・サイズ・同じ内容。純粋関数）
    mirror/          mirror list/add/remove, sites サブコマンド（mod.rs: 値を集める関数と CLI の入口、set.rs: Follow Set の編集 `MirrorSet`、print.rs: CLI の表示、time.rs: UTC 日時の表示）
    replicas.rs      レプリカ報告の集計、replicas サブコマンド
    webring/         Follow Set のたどり方とグラフの組み立て、webring サブコマンド（mod.rs）、テキスト・DOT・Mermaid の出力（render.rs）
    nip05.rs         NIP-05 検証
    signer/          署名（mod.rs: Signer（秘密鍵か NIP-46 の署名アプリ）・RemoteSigner・remote-signer.json、pair.rs: QR コードでのペアリング）。詳細は architecture/signer.md
    pair.rs          `swing signer pair`（ターミナルに QR コードを出して署名アプリとペアリングする）。詳細は architecture/cli.md
    up.rs            `swing up` supervisor（Kubo の起動・監視、agent の起動・再起動、バックオフ）。詳細は architecture/up.md
    stats.rs         リソース使用量の記録（`swing up` の中で 60 秒ごとに測ってメモリに 24 時間分持つ）と stats サブコマンド。stats/process.rs が OS ごとにプロセスの CPU 時間とメモリを読む。詳細は architecture/stats.md
    kubo.rs          Kubo バイナリの検出・init・`ipfs config` 適用・RPC のポートと秘密（kubo-api.json）・子プロセスの起動と終了・ヘルス待ち・kubo.pid と孤児回収。詳細は architecture/kubo.md
    lock.rs          多重起動防止のインスタンスロック（swing.lock、swing-tray の swing-tray.lock）。詳細は architecture/up.md
    ports.rs         セットアップモードでのポートのずらし方（`bind_shifting`・`free_addr`・`may_shift`）。詳細は architecture/up.md
    auth.rs          ダッシュボードのトークンファイル（dashboard.token）、HMAC 署名のセッション値、使い捨てログインコード。詳細は architecture/dashboard.md
    login.rs         `swing dashboard open`・`swing dashboard rotate-token`。詳細は architecture/cli.md
    gateway.rs       内蔵 gateway（axum）。Host 名での振り分けと Kubo gateway へのプロキシ。詳細は architecture/gateway.md
    host.rs          Host ヘッダをホスト名とポートに分ける厳密なパーサ（ダッシュボードのガードと内蔵 gateway が共有）
    service/         `swing service install/uninstall/start/stop/status`（mod.rs が入口、templates.rs が unit / plist / タスク XML の生成、process.rs が外部コマンドの実行、linux.rs・macos.rs・windows.rs が OS ごとの実行部分）。詳細は architecture/service.md
    settings/        全設定キーのカタログ（mod.rs）、`swing.example.toml`/`.env.example` の生成（example.rs）、`PUT /api/config`・`POST /api/setup` の書き込み（edit.rs）。詳細は下記「設定と環境変数」と architecture/dashboard.md
    stop.rs          `swing stop`（動いている `swing up` のダッシュボード API 経由。API に到達できなければ「動いていない」として終了する）。詳細は architecture/up.md, architecture/service.md
    shutdown.rs      `cancel_on_signal(grace)`（SIGINT/SIGTERM → CancellationToken、force-exit watchdog、2 回目のシグナルで即時終了）、`RUNTIME_SHUTDOWN_TIMEOUT`、`ExitRequest`/`Exit`（停止・再起動の要求と、`up::run` が返す `Exit::Stop`/`Exit::Restart`）。up/agent 共通
    dashboard/       `swing up` 常駐の Web ダッシュボード兼制御 API（mod.rs, guard.rs, api.rs, session.rs, setup.rs, dto.rs, config_dto.rs, assets.rs, mascots/, upload.rs, test_support.rs）。詳細は architecture/dashboard.md
    api_client.rs    ダッシュボード API を呼ぶ CLI 共通クライアント（`ApiClient`。`POST /api/identity` で相手を確かめてから `<state_dir>/dashboard.token` を Bearer トークンとして送る）。使うサブコマンドは architecture/cli.md の「共通」
    test_support.rs  `#[cfg(test)]` のクレート共通フィクスチャ（`FakeKubo`、CID・鍵・イベントのテストヘルパ）
  web/               ダッシュボードのフロント（index.html, style.css, ES modules（setup.js を含む）, 画像・フォントなどの静的アセット一式）。ビルド工程なしで include_str!/include_bytes! によりバイナリへ埋め込む。desktop-page.html / desktop-page.css / desktop-banner.gif（Desktop 画面のリンク集ページ）だけは設定で差し替えられ、mascots/（同梱の Desktop マスコットパック）は `[dashboard].mascots_dir` で指すディレクトリのユーザー定義パックを追加できる。詳細は architecture/dashboard.md
    fonts/           Desktop 画面の同梱フォント PixelMplus12（woff2）とそのライセンス
  build.rs           Windows 向けのとき、exe にアイコン（assets/swing.ico）とバージョン情報を埋め込む（winresource）
  assets/swing.ico   swing.exe のファイルアイコン（web/favicon.svg から書き出した 16〜256px）
  tests/
    kubo_integration.rs          Kubo 連携の統合テスト（#[ignore]）
    nostr_relay_integration.rs   relay 連携の統合テスト（#[ignore]）
  tray/              タスクトレイ（`swing-tray`）の別クレート。workspace のメンバーで、`swing` クレートをライブラリとして使う。詳細は architecture/tray.md
    src/main.rs      エントリポイント（`--config` の解釈、多重起動の防止）
    src/status.rs    状態から表示・使える項目・アイコンを決める純粋関数
    src/worker.rs    ダッシュボード API のポーリングとメニュー操作（tokio）
    src/app.rs       tao のイベントループとトレイアイコン（Windows / macOS のみ）
    build.rs         ルートの build.rs と同じ（Windows 向けのとき assets/swing-tray.ico を埋め込む）
    assets/swing-tray.svg, assets/swing-tray.ico  swing-tray.exe のファイルアイコン（ロゴの右下にタスクトレイのバッジ）と、その元の SVG
    assets/icon-64.png           Windows のトレイアイコン
    assets/icon-template-64.png  macOS のメニューバーのアイコン（テンプレート画像）
    assets/SWING.icns            macOS の `SWING.app` のアイコン
    macos/bundle.sh, macos/Info.plist  `swing-tray` を `SWING.app` にまとめるスクリプトと、その Info.plist の雛形
  docker/kubo-init.d/  Kubo コンテナの起動スクリプト（外部 Kubo の設定。compose 専用）
  docker/release.Dockerfile  ghcr.io に push するイメージ。release ワークフローがビルド済みの musl バイナリを入れる
  docker/demo/         外部ネットワークに出ないデモ環境（compose の重ね合わせとシードスクリプト）。詳細は docker/demo/README.md
  Dockerfile, compose.yaml, .env.example, swing.example.toml
  .github/workflows/release.yml  配布用バイナリとコンテナイメージのビルド、ドラフトリリース（[`architecture/release.md`](architecture/release.md)）
  .github/workflows/macos-check.yml  手動実行で macOS のトレイとサービス登録を動かして画面を撮る（[`architecture/release.md`](architecture/release.md#macos-の動作確認githubworkflowsmacos-checkyml)）
  .github/workflows/windows-check.yml  手動実行で英語の Windows のトレイとサービス登録を動かして画面を撮る（[`architecture/release.md`](architecture/release.md#windows-の動作確認githubworkflowswindows-checkyml)）
  docs/                役割は AGENTS.md を参照
```

`mirror/`・`health.rs`・`replicas.rs`・`webring/` は、relay/Kubo とやり取りして値を返す関数（`collect_mirror_list`・`collect_sites`・`collect_status`・`replicas::collect`・`webring::collect`）と、表示する関数とに分かれている。ダッシュボードの API ハンドラは同じ関数を呼び、DTO に変換する。CLI は `sites`・`replicas`・`webring`・`mirror list` ではこれらを直接呼び、`status`・`stats`・`mirror add`/`remove` ではダッシュボード API 経由で `swing up` 側に呼ばせる。`publish.rs` は段階ごとの関数（`resolve_modes`・`check_nip05`・`LocalChecks::run`・`add_and_measure`・`check_unchanged`・`sign_and_send`・`prune_old_versions_collect`）を CLI とダッシュボードで共有する。

サブコマンドごとに relay/Kubo へ直接つなぐか、動いている `swing up` のダッシュボード API を経由するかは [`architecture/cli.md#共通`](architecture/cli.md#共通) を参照。

## CLI

```
swing up      [--config <path>] [--log-file <path>] [--no-port-shift]
swing stop    [--config <path>] [--restart] [--timeout <secs>]
swing service install   [--config <path>] [--system [--run-as <user>]] [--no-start] [--no-tray]
swing service uninstall [--system]
swing service start     [--system]
swing service stop      [--system]
swing service status    [--system]
swing dashboard open         [--config <path>] [--no-browser]
swing dashboard rotate-token [--config <path>]
swing publish [--config <path>] --site <d-tag> [--url <URL>] [--nip05 <off|warn|require>] [--check-dotfiles <off|warn|require>] [--check-size <off|warn|require>] [--check-unchanged <off|warn|require>] [--title <TEXT>] [-m, --message <TEXT>] <DIR>
swing mirror list                      [--config <path>]
swing mirror add <key>...              [--config <path>]
swing mirror remove <key>...           [--config <path>]
swing sites                            [--config <path>]
swing replicas [<key>...]              [--config <path>]
swing status                           [--config <path>]
swing stats  [--last <duration>] [--json] [--config <path>]
swing webring [<key>...] [--depth <N>] [--format <text|dot|mermaid>] [--config <path>]
swing signer pair [--relay <URL>]...   [--config <path>]
swing key generate
swing config example
swing config env-example

swing-tray [--config <path>]
```

`swing up` は Kubo（`[kubo].managed = true` なら）と mirror-agent の中身を 1 プロセスの supervisor として動かす（[`architecture/up.md`](architecture/up.md)）。`managed = false` なら既に動いている Kubo（外部のもの）を待ってから同じことをする。`swing service` は `swing up` を OS の常駐に登録する（[`architecture/service.md`](architecture/service.md)）。`swing stop` は動いている `swing up` にグレースフルな停止・再起動を要求する（[`architecture/cli.md#stop`](architecture/cli.md#stop)）。

署名の設定とセットアップモードに入る条件は [`architecture/signer.md`](architecture/signer.md) と [`architecture/up.md#セットアップモード鍵未設定`](architecture/up.md#セットアップモード鍵未設定) を参照。

各サブコマンドの動作と出力は [`architecture/cli.md`](architecture/cli.md) を参照。

## 設定と環境変数

設定ファイルの TOML の構文や型のエラーは `line <行>, column <桁>: <理由>` の形で報告し、toml の既定の表示のように該当行を引用しない（秘密鍵の行を出さないため）。

設定ファイルは次の順で 1 つのパスに決まる（`config::resolve_config_path`）。1 か 2 を指定してそのファイルが無ければエラー終了。3 は存在確認をせず、そのままファイルの読み書き先になる（無ければ設定は既定値と環境変数だけで組み立て、`Config.config_exists = false` になる。ダッシュボードのセットアップ・設定編集はこのパスに新規作成・上書きする）。

1. `--config <path>`
2. 環境変数 `SWING_CONFIG`
3. `<カレントディレクトリ>/swing.toml`

すべての設定キー（TOML フィールド、環境変数、種類、例、編集可否、説明）は `src/settings/mod.rs` の `SETTINGS`（`Setting` の配列）1 箇所にカタログとして持つ。既定値は `config::build_config`（`src/config/build.rs`）が持つ（`[policy].max_update_size` の既定値は `publish` のサイズ確認の表示でも使うので `config::DEFAULT_MAX_UPDATE_SIZE` として公開している）。環境変数は TOML の値を上書きする。`config::build_config` はキーごとの解決を `Resolver` にまとめ、各キーの env 名をこのカタログから `settings::env_of("<section>.<field>")` で引き、値の出どころ（`Env`・`File`・`Default`）を `Config.sources` に記録する。値のパースや検証の失敗は、実際に使った出どころの名前で報告する（環境変数から来た値なら `invalid SWING_MAX_PER_SITE`・`SWING_POLL_INTERVAL must be greater than 0`、設定ファイルから来た値なら `invalid [policy].max_per_site`・`[agent].poll_interval must be greater than 0`）。

`swing.example.toml` と `.env.example` はこのカタログから生成する（生成するコマンドは [`architecture/cli.md#config-example--config-env-example`](architecture/cli.md#config-example--config-env-example)）。両ファイルは `.gitattributes` で改行を LF に固定している。

`cargo test` は次の 4 点でカタログと生成物の乖離を防ぐ（`src/settings/` のテスト）:

- `swing.example.toml`・`.env.example` の内容が、それぞれ `render_toml_example()`・`render_env_example()` の出力と一致すること。
- `render_toml_example()` の出力を `build_config_from_str` でパースした結果が、空文字列から作った既定値とフィールドごとに一致すること（秘密鍵は有無だけを比べる。例の値がコードの既定値から drift しない）。
- カタログのキー集合が `Config.sources`（`build_config` が実際に解決するキー）の集合と完全に一致すること。
- `render_env_example` が「compose では効果が無い」と注記する `ENV_NO_EFFECT_IN_COMPOSE` の集合が、`compose.yaml` の `mirror` サービスの `environment:` に固定値で書かれている `SWING_*` の集合と一致すること。

パスの設定（カタログの種類が `Path` のもの: `[agent].state_dir`・`[kubo].binary`・`[kubo].repo`・`[dashboard].custom_css`・`desktop_page`・`desktop_page_css`・`desktop_banner`・`mascots_dir`）の相対パスは、値の出どころで起点が変わる。設定ファイルがあるときは、設定ファイルに書いた値と既定値（`state_dir` の `./data`）を、設定ファイルのあるディレクトリ（`Config::load` が設定ファイルのパスを絶対パスにした親ディレクトリ）を起点に `build_config` が絶対パスにする。環境変数で渡した値と、設定ファイルが無いときの既定値はそのまま残り、カレントディレクトリが起点になる。他の値から導く既定値（`[kubo].repo` の `<state_dir>/kubo`）は解決後の `state_dir` に従う。絶対パスはどちらでもそのまま。設定ファイルの書き換え（ダッシュボードの設定編集・セットアップ・ポートの固定）は TOML の文書を直接編集するので、解決後のパスがファイルに書き戻されることはない。

設定例は [`../swing.example.toml`](../swing.example.toml) を参照。`swing.example.toml` の `#field = value` はコメントアウトされた任意設定（省略時は既定値、または他の設定から導かれる値）、`field = value` は有効な行。

Kubo の起動・設定は [`architecture/kubo.md`](architecture/kubo.md)、内蔵 gateway の動作は [`architecture/gateway.md`](architecture/gateway.md) を参照。

`swing` が読む `SWING_` 環境変数のうち TOML キーを持たずカタログに無いのは `SWING_CONFIG`（設定ファイルのパス）と `SWING_NO_PORT_SHIFT`（`swing up --no-port-shift` と同じ。clap の `env` で読み、`false`・`0`・`no`・`off` など以外なら有効。[`architecture/up.md#セットアップモードでのポートの調整`](architecture/up.md#セットアップモードでのポートの調整)）だけ。ほかにテスト用の `SWING_TEST_*` がある。`compose.yaml` の変数展開専用のホストバインド変数（`SWING_KUBO_GATEWAY_BIND`・`SWING_GATEWAY_BIND`・`SWING_DASHBOARD_BIND`）はカタログの外で、`swing` は読まない。ログレベルは `RUST_LOG`。

compose でのコンテナ内の待ち受けとホスト側の公開アドレス（`SWING_*_BIND`）の対応と、内蔵 gateway の有効化は [`architecture/docker.md#composeyaml`](architecture/docker.md#composeyaml)。

空文字の環境変数（`SWING_CONFIG` を含む）は未設定として扱う。`[dashboard]` の各キーの意味と制約は [`architecture/dashboard.md`](architecture/dashboard.md#設定dashboard) を参照。

検証:

- `poll_interval`、`concurrency`、`max_sites_per_account`、`[publish].keep_versions`、`[agent].fetch_timeout`、`[agent].fetch_idle_timeout`、`[dashboard].max_upload` は 0 だとエラー。
- `[publish].dotfiles_allow` の各要素は前後の空白を除き、空になった要素は捨てる。残りは `.` で始まり、`/` を含まず、`.`・`..` そのものでないこと（`config::validate_dotfile_name`）。違反はエラー。TOML で空配列にすると何も見逃さない（環境変数は空文字だと未設定扱いなので、`,` だけを渡す）。未設定なら `config::DEFAULT_DOTFILES_ALLOW`。
- `report_ttl` の半分が `poll_interval` 以下ならエラー。`report_ttl` が `nostr::MAX_REPORT_AGE`（7 日）を超えてもエラー（`report_ttl must be at most 7d`）。
- `mfs_root` は `/` で始まる絶対パス。`/` そのもの、空の要素、`.`、`..` を含むとエラー。末尾の `/` は取り除く。
- `[ipfs].api` と `[gateway].upstream` は `http(s)://host[:port]` の形（末尾の `/` を取り除いた値が、`Url` として解釈した http(s) のオリジンの文字列（`origin().ascii_serialization()`）と一致すること。パス・クエリ・フラグメント・`@`・制御文字・大文字のホスト名・既定のポートの明示は不可。`[dashboard].public_url` と同じ規則）であること。違反はエラー。
- `[kubo].managed = true` のときに `[ipfs].api`（TOML または `SWING_IPFS_API`）が指定されているとエラー（`[ipfs].api conflicts with [kubo].managed = true`。環境変数なら `SWING_IPFS_API conflicts with ...`）。
- `[gateway].listen` が `off` 以外で `[gateway].hosts` が空ならエラー。
- `[gateway].hosts` の各要素は前後の空白を除き、空になった要素は捨てる。残りは `a-z 0-9 . -` のみで構成され、`.` で始まらず・終わらず、`..` を含まないこと（`config::is_valid_gateway_host`。`docker/kubo-init.d/001-swing-config.sh` の `SWING_GATEWAY_HOSTS` 検証と同じ規則）。違反はエラー。
- `[gateway].hosts` の要素がダッシュボードで開けるホスト名（`[dashboard].allowed_hosts` の要素（大文字小文字を区別しない）と `localhost`・`127.0.0.1`）と重なればエラー（[`architecture/gateway.md`](architecture/gateway.md#設定gateway)）。
- `[dashboard].gateway` は空文字か、`[dashboard].public_url` と同じ `http(s)://host[:port]` の形であること。違反はエラー。
- `[kubo].provide_strategy` は空文字か空白だけならエラー。値そのものの妥当性は Kubo 起動時の判定に任せる。
- `[kubo].storage_max` は容量パーサ、`[kubo].gateway_listen` は `SocketAddr`、`[kubo].swarm_port` は 1..=65535（`0` はエラー）としてパースする。
- `[kubo].binary` の実在確認は `config` では行わない（`kubo::locate_binary` が `swing up` 起動時に行う）。

値の形式:

- 容量: `"100GiB"`、`"512MiB"`、`"1TiB"`、`"512B"`。単位は `KiB`・`MiB`・`GiB`・`TiB` と `KB`・`MB`・`GB`・`TB` で、どちらも 1024 基数（`100GB` と `100GiB` は同じ値）、大文字小文字を区別しない。例や既定値の表記は `GiB` 系に揃えている。compose の Kubo は `SWING_KUBO_STORAGE_MAX` の文字列を自分で解釈し `GB` を 10 進として読むので、同じ値にしたいときは `GiB` 系で書く（[`architecture/docker.md#kubo-の設定`](architecture/docker.md#kubo-の設定)）。数値部分は数字と `.` のみで、小数はバイト換算後に切り捨て。数値だけならバイト。u64 に収まらなければエラー。
- 時間: `"30s"`、`"10m"`、`"2h"`、`"365d"`。数値部分は整数のみ。数値だけなら秒。秒換算で u64 に収まらなければエラー。
- 秘密鍵は `Debug` 出力で `<redacted>` になる。

Nostr イベントの検証、取得と表示の上限（`nostr::budget`）、レプリカ報告の信頼度（`replicas::Tier`）は [`architecture/nostr.md`](architecture/nostr.md) を参照。

## テスト

- ユニットテスト: `cargo test`。agent と health のテストは、MFS をメモリ上で真似る `FakeKubo`（`src/test_support.rs`）を使う。agent はこれに `FakeNip05` を組み合わせる。ダッシュボードのテストは axum の `Router` に `oneshot` でリクエストを投げ、TCP で listen しない。署名アプリとのやり取りのテストは [`architecture/signer.md#テスト`](architecture/signer.md#テスト)。
- 統合テスト（`#[ignore]`、ローカルの Kubo / relay が必要。公開ネットワークには接続しない）:

```bash
# test プロファイルは bootstrap とローカル探索を無効にする
docker run -d --rm -e IPFS_PROFILE=test -p 127.0.0.1:15001:5001 ipfs/kubo:v0.43.1
# SWING_TEST_IPFS_API（既定 http://127.0.0.1:15001）、任意で SWING_TEST_EXPECTED_CID
cargo test --test kubo_integration -- --ignored --test-threads=1
# agent の保存・sweep・突き合わせ・unfollow を実物の Kubo で通す（health.rs の検査を含む）
cargo test --lib agent_stores_and_removes_through_real_kubo -- --ignored

docker run -d --rm -p 127.0.0.1:18080:8080 scsibug/nostr-rs-relay
# SWING_TEST_RELAY（既定 ws://127.0.0.1:18080）。サイトイベント・レプリカ報告・Follow Set の送受信と #p での Follow Set の取得
cargo test --test nostr_relay_integration -- --ignored --test-threads=1
```

`kubo::tests` の `#[ignore]` テスト（`full_lifecycle_against_real_kubo`・`recover_orphan_shuts_down_a_leftover_daemon_via_its_api`・`recover_orphan_falls_back_to_signals_when_api_is_unreachable`）は Kubo のバイナリを子プロセスとして起動する（repo は一時ディレクトリ）。`SWING_TEST_KUBO_BIN` が無ければ何もせずに通る:

```bash
SWING_TEST_KUBO_BIN=/path/to/ipfs cargo test --lib kubo::tests -- --ignored
```

ビルド（Windows 向けのクロスビルドを含む）とリリース（`.github/workflows/release.yml`）、macOS と Windows の動作確認（`.github/workflows/macos-check.yml`・`windows-check.yml`）は [`architecture/release.md`](architecture/release.md) を参照。
