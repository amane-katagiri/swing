# SWING アーキテクチャ

今のコードが何をしているかのリファレンス。イベント形式は [`protocol.md`](protocol.md)、経緯と理由は [`log/`](log/) を参照。

| 文書 | 内容 |
|---|---|
| このファイル | 構成要素、リポジトリ構成、CLI の一覧、設定と環境変数、テストと各ファイルへの索引 |
| [`architecture/config.md`](architecture/config.md) | 設定ファイルと環境変数の読み込み・解決、設定キーのカタログ（`settings/`）とダッシュボードからの書き込み |
| [`architecture/cli.md`](architecture/cli.md) | 各サブコマンドの動作と出力。子ページは `publish` の [`cli/publish.md`](architecture/cli/publish.md) と、表示系と Follow Set の操作の [`cli/views.md`](architecture/cli/views.md) |
| [`architecture/publish.md`](architecture/publish.md) | `swing publish` とダッシュボードの公開が共有する処理: 保護パス・ローカルの確認（リンクの確認は子ページ [`publish/links.md`](architecture/publish/links.md)）・時計の確認と `created_at`・版の配置と後始末・古い版の削除 |
| [`architecture/mirror.md`](architecture/mirror.md) | `mirror list`/`add`/`remove`・`sites` とダッシュボードの API が共有する処理: 使う Follow Set の選び方・Follow Set の書き換え・sites の集計（レプリカ数、unfollow の一覧） |
| [`architecture/webring.md`](architecture/webring.md) | `webring` と `/api/webring` が共有する処理: Follow Set のたどり方・グラフ・ノードの名前 |
| [`architecture/health.md`](architecture/health.md) | 版の検査と MFS の突き合わせ: agent の起動時の突き合わせと sweep、`status` と `/api/status` が共有する判定と集計 |
| [`architecture/agent.md`](architecture/agent.md) | mirror-agent の動作、`state.json`。子ページはポリシー判定の [`agent/policy.md`](architecture/agent/policy.md) と、レプリカ報告の送信の [`agent/replicas.md`](architecture/agent/replicas.md) |
| [`architecture/signer.md`](architecture/signer.md) | 署名（`signer/`）: 秘密鍵か NIP-46 の署名アプリか、`remote-signer.json`、QR コードでのペアリング |
| [`architecture/nip05.md`](architecture/nip05.md) | NIP-05 検証（agent と publish で共通） |
| [`architecture/nostr.md`](architecture/nostr.md) | Nostr イベントの検証（`nostr/`）、未来ずれの許容、レプリカ報告の信頼度。取得と表示の上限（`nostr::budget`）の子ページもここから |
| [`architecture/kubo.md`](architecture/kubo.md) | Kubo の管理（`kubo/`）: 検出と init、設定、RPC の秘密、Kubo のバージョン。子ページはデーモンの起動・停止と孤児の回収の [`kubo/daemon.md`](architecture/kubo/daemon.md) |
| [`architecture/mfs.md`](architecture/mfs.md) | MFS の使い方（`mfs.rs`）と Kubo RPC クライアント（`ipfs.rs`） |
| [`architecture/up.md`](architecture/up.md) | `swing up`（supervisor）: 起動順、セットアップモード、終了要求・シグナル、停止の時間予算、多重起動の防止 |
| [`architecture/stats.md`](architecture/stats.md) | リソース使用量の記録（`stats.rs`） |
| [`architecture/gateway.md`](architecture/gateway.md) | 内蔵 gateway（`gateway.rs`）: Host 振り分けと Kubo gateway へのプロキシ |
| [`architecture/service.md`](architecture/service.md) | `swing service`（systemd / launchd / タスクスケジューラ）。子ページは登録の持ち主の判定（`--only-from`・`--points-into`）の [`service/ownership.md`](architecture/service/ownership.md) |
| [`architecture/tray.md`](architecture/tray.md) | タスクトレイ（`swing-tray`、Windows と macOS） |
| [`architecture/docker.md`](architecture/docker.md) | Dockerfile、compose、外部 Kubo コンテナの設定 |
| [`architecture/dashboard.md`](architecture/dashboard.md) | Web ダッシュボード兼制御 API（`dashboard/`）: 起動と終了、設定、タイムアウト、静的ファイル。子ページの一覧もここ |
| [`architecture/release.md`](architecture/release.md) | ビルドとリリース（手順のスクリプトを含む）、macOS・Homebrew・Windows の動作確認の CI |
| [`architecture/install-sh.md`](architecture/install-sh.md) | Linux 向けのインストールスクリプト（`packaging/linux/install.sh`）: 入れる物と場所、更新、アンインストール、テスト |
| [`architecture/homebrew.md`](architecture/homebrew.md) | macOS 向けの Homebrew の formula（`packaging/homebrew/`）: 入れる場所、`brew upgrade` とサービス、tap への公開 |
| [`architecture/installer-windows.md`](architecture/installer-windows.md) | Windows のインストーラー（Inno Setup、`packaging/windows/`）: 中身、インストール・上書き・アンインストールの動作、その確認の CI |

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
| ログ | `tracing` + `tracing-subscriber`（`RUST_LOG`。既定は `swing up` が `info`、ほかのコマンドは `info,nostr_sdk=warn,nostr_connect=warn`）。メッセージと各フィールドの値は `logging::SanitizedFields` で `format::Sanitized` を通してから書く（外部由来の文字列でログの行を偽れないように） |
| CID 検証 | `cid` クレート |

クレート `swing` は lib + bin 構成。統合テストは `swing::` としてモジュールを直接使う。

## リポジトリ構成

`src/`（クレート `swing`）:

| パス | 役割 |
|---|---|
| `main.rs` / `lib.rs` | CLI エントリ（clap）/ クレートルート |
| `config/` | 設定の読み込みと値のパーサ（`mod.rs`）、`build_config` と `Resolver`（`build.rs`、テストは `build/tests.rs`）。[config.md](architecture/config.md) |
| `settings/` | 設定キーの型（`mod.rs`）とカタログ `SETTINGS`（`catalog.rs`）、例の生成（`example.rs`）、設定ファイルの書き換え `update`・`setup`・`pin_addrs`・`write_atomic`（`edit.rs`）。[config.md](architecture/config.md) |
| `nostr/` | イベントの検証・組み立て、relay との通信（`client.rs`。取得の内部処理は `client/fetch.rs`、レプリカ報告の送受信の trait は `client/report_relay.rs`、送信結果の表示は `client/send.rs`）、site event（`site.rs`）、follow set（`follow.rs`）、レプリカ報告（`report.rs`）、上限 `budget`（`mod.rs`）。[nostr.md](architecture/nostr.md) |
| `signer/`・`pair.rs` | 署名（秘密鍵か NIP-46 の署名アプリ）と `remote-signer.json`、ペアリング / `swing signer pair`。[signer.md](architecture/signer.md) |
| `nip05.rs` | NIP-05 検証。[nip05.md](architecture/nip05.md) |
| `ipfs.rs`・`ipfs/site.rs`・`mfs.rs` | Kubo RPC クライアント・サイトのディレクトリの一覧と add の multipart・MFS 上のパスの組み立て。[mfs.md](architecture/mfs.md) |
| `kubo/` | Kubo の RPC の秘密と `kubo-api.json`（`access.rs`）・検出と init（`binary.rs`）・設定（`config.rs`）・起動と終了（`daemon.rs`）・`kubo.pid` と孤児回収（`orphan.rs`）、テストは `tests.rs`。[kubo.md](architecture/kubo.md) |
| `agent/` | mirror-agent のループ。[agent.md](architecture/agent.md) |
| `policy.rs`・`state.rs` | 保存ポリシーの判定（純粋関数）・`state.json` の永続化。[agent.md](architecture/agent.md) |
| `health.rs` | 版と MFS の突き合わせ（agent と `status` で共通）と `status`。[health.md](architecture/health.md)、CLI の表示は [cli/views.md](architecture/cli/views.md#status) |
| `publish.rs`・`publish/checks.rs`・`publish/links.rs`（と `publish/links/`）・`publish/clock.rs`・`publish/new_files.rs`・`publish/staged.rs` | `publish` と、その前後の確認（ドットファイル・保護パス・サイズ・リンク・同じ内容・増えたファイル・時計）、告知できなかった版の後始末。[publish.md](architecture/publish.md)、CLI の表示は [cli/publish.md](architecture/cli/publish.md) |
| `mirror/` | `mirror list`/`add`/`remove`・`sites`（`set.rs`: Follow Set の編集、`print.rs`・`time.rs`: 表示）。[mirror.md](architecture/mirror.md)、CLI の表示は [cli/views.md](architecture/cli/views.md#mirror-list--add--remove) |
| `replicas.rs` | レプリカ報告の集計と `replicas`。[nostr.md](architecture/nostr.md#一覧の集計replicascollect)、表示は [cli/views.md](architecture/cli/views.md#replicas) |
| `webring/` | Follow Set のたどり方とグラフ、`webring`（`render.rs`: テキスト・DOT・Mermaid）。[webring.md](architecture/webring.md)、CLI の表示は [cli/views.md](architecture/cli/views.md#webring) |
| `key.rs`・`format.rs` | `key generate`・バイト数と秒数の表示、端末に出す他人由来の文字列の無害化（`Sanitized`: 制御文字を空白 1 つに置き換え、見えない書式文字（`nostr::is_unsafe_char`）を取り除く。`sanitize_display_text`: それに加えて文字数で切り、前後の空白を削る）、エラー終了のときの表示（`error_report`） |
| `logging.rs` | `tracing` のフィールドの書き出し（`SanitizedFields`） |
| `up.rs`・`ports.rs`・`lock.rs`・`shutdown.rs` | `swing up` の supervisor・セットアップモードでのポートのずらし方・多重起動の防止・シグナルと終了要求。[up.md](architecture/up.md) |
| `stop.rs` | `swing stop`。[cli.md](architecture/cli.md#stop) |
| `stats.rs`・`stats/process.rs` | リソース使用量の記録と `stats`、OS ごとのプロセスの CPU とメモリ。[stats.md](architecture/stats.md) |
| `proc.rs` | プロセスの開始時刻・生存確認・シグナル（`kubo/` が使う）と、Linux の `/proc/<pid>/stat` の読み取り（`stats/process.rs` と共有） |
| `service/` | `swing service`（`templates.rs`: unit・plist・タスク XML、`process.rs`: 外部コマンド、`ownership.rs`: 登録が指す実行ファイルの読み取りと判定、`linux.rs`・`macos.rs`・`windows.rs`、ほかの OS は `unsupported.rs`）。[service.md](architecture/service.md)、登録の持ち主の判定は [service/ownership.md](architecture/service/ownership.md) |
| `gateway.rs`・`host.rs` | 内蔵 gateway・Host ヘッダのパーサ（ダッシュボードのガードと共有）。[gateway.md](architecture/gateway.md) |
| `dashboard/` | Web ダッシュボードと制御 API（`mod.rs`・`guard.rs`・`api.rs`・`error.rs`・`publish.rs`・`session.rs`・`setup.rs`・`locks.rs`・`upload.rs`・`dto.rs`・`config_dto.rs`・`assets.rs`・`mascots/`・`test_support.rs`）。[dashboard.md](architecture/dashboard.md) |
| `auth.rs`・`api_client.rs`・`login.rs` | トークン・セッション・ログインコード、CLI のダッシュボード API クライアント、`swing dashboard open`/`rotate-token`。[dashboard/security.md](architecture/dashboard/security.md) |
| `activity.rs` | 最新の publish・レプリカ報告の時刻（`/api/activity` 用、メモリだけ） |
| `test_support.rs` | テスト用のフィクスチャ（`FakeKubo` など） |

ほか:

| パス | 役割 |
|---|---|
| `web/` | ダッシュボードのフロント。ビルド工程なしでバイナリに埋め込む（`fonts/`: 同梱フォント PixelMplus12、`mascots/`: 同梱のマスコットパック）。[dashboard/web.md](architecture/dashboard/web.md) |
| `tests/` | 統合テスト（`#[ignore]`。下記「[テスト](#テスト)」） |
| `tray/` | タスクトレイ `swing-tray` の別クレート（workspace のメンバー。`src/`・`assets/`・`macos/`）。[tray.md](architecture/tray.md) |
| `build.rs`・`assets/swing.ico` | Windows 向けに exe へアイコンとバージョン情報を埋め込む。アイコンは `tray/assets/` の画像とともに `web/favicon.svg` から書き出した派生物（[tray.md](architecture/tray.md#アイコン)） |
| `Dockerfile`・`compose.yaml`・`docker/` | コンテナと compose（`kubo-init.d/`: 外部 Kubo の設定、`release.Dockerfile`: 配布イメージ、`demo/`: デモ環境）。[docker.md](architecture/docker.md)・[docker/demo/README.md](../docker/demo/README.md) |
| `swing.example.toml`・`.env.example` | 設定例（カタログから生成） |
| `packaging/linux/` | Linux 向けの `install.sh` とそのテスト（`test-install.sh`）。[install-sh.md](architecture/install-sh.md) |
| `packaging/homebrew/` | Homebrew の formula のひな形と書き出しスクリプト。[homebrew.md](architecture/homebrew.md) |
| `packaging/windows/` | Windows のインストーラーの Inno Setup スクリプト（`swing.iss`）、それを組み立てる `build.ps1`、同梱する Kubo の固定したチェックサム（`kubo.sha512`）、動作確認の `check-installer.ps1`。[installer-windows.md](architecture/installer-windows.md) |
| `packaging/release/` | リリースの手順のスクリプト（`bump.sh`・`check.sh`・`tag.sh`・`notes.sh`・`update-tap.sh`）とリリースノートのひな形。[release.md](architecture/release.md#リリースの手順) |
| `.github/workflows/` | `release.yml`・`macos-check.yml`・`homebrew-check.yml`・`windows-check.yml`・`windows-installer-check.yml`。[release.md](architecture/release.md)・[installer-windows.md](architecture/installer-windows.md) |
| `docs/` | 役割は AGENTS.md |

`mirror/`・`health.rs`・`replicas.rs`・`webring/` は、relay・Kubo から値を集める関数（`collect_mirror_list`・`collect_sites`・`collect_status`・`replicas::collect`・`webring::collect`）と表示する関数に分かれ、ダッシュボードの API は前者を呼んで DTO にする。集める側の処理は [`architecture/mirror.md`](architecture/mirror.md)・[`architecture/health.md`](architecture/health.md)・[`architecture/webring.md`](architecture/webring.md)（レプリカ報告は [`architecture/nostr.md`](architecture/nostr.md#レプリカ報告の信頼度replicastier)）。`publish.rs` の段階ごとの関数は CLI とダッシュボードで共有する（[`architecture/publish.md`](architecture/publish.md)）。サブコマンドごとに relay・Kubo へ直接つなぐか `swing up` のダッシュボード API を経由するかは [`architecture/cli.md#共通`](architecture/cli.md#共通)。

## CLI

```
swing up      [--config <path>] [--log-file <path>] [--no-port-shift]
swing stop    [--config <path>] [--restart] [--timeout <secs>]
swing service install   [--config <path>] [--system [--run-as <user>] [--allow-root]] [--no-start] [--no-tray]
swing service uninstall [--system] [--only-from <dir>]
swing service start     [--system]
swing service stop      [--system]
swing service status    [--system] [--points-into <dir>]
swing dashboard open         [--config <path>] [--no-browser]
swing dashboard rotate-token [--config <path>]
swing publish [--config <path>] --site <d-tag> [--url <URL>] [--nip05 <off|warn|require>] [--check-dotfiles <off|warn|require>] [--check-size <off|warn|require>] [--check-links <off|warn|require>] [--check-unchanged <off|warn|require>] [--title <TEXT>] [-m, --message <TEXT>] [-y, --yes] <DIR>
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

各サブコマンドの動作と出力は [`architecture/cli.md`](architecture/cli.md)。署名の設定とセットアップモードに入る条件は [`architecture/signer.md`](architecture/signer.md) と [`architecture/up.md#セットアップモード鍵未設定`](architecture/up.md#セットアップモード鍵未設定)。

## 設定と環境変数

設定ファイル（`--config` → `SWING_CONFIG` → あれば `./swing.toml` → ユーザーごとの既定の場所の `swing.toml`）の値を `SWING_` 環境変数が上書きする。すべてのキーは `src/settings/catalog.rs` の `SETTINGS` にカタログとしてまとまり、`swing.example.toml` と `.env.example` はそこから生成する。探し方・検証・値の形式・書き換えの規則は [`architecture/config.md`](architecture/config.md)。

## テスト

- ユニットテスト: `cargo test`。大きいテストモジュールは `foo.rs` に `#[cfg(test)] mod tests;` を置いて `foo/tests.rs` に分ける（`config/build/tests.rs`・`nostr/client/tests.rs`・`nostr/site/tests.rs`・`agent/replicas/tests.rs` など）。agent と health のテストは MFS をメモリ上で真似る `FakeKubo`（`src/test_support.rs`）を使い、agent はこれに `FakeNip05` を組み合わせる。ダッシュボードのテストは axum の `Router` に `oneshot` でリクエストを投げ、TCP で listen しない。署名アプリとのやり取りのテストは [`architecture/signer.md#テスト`](architecture/signer.md#テスト)。
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

`kubo::tests` の `#[ignore]` テスト（Kubo の一生・孤児の回収）と `mfs_kubo_integration`（空白・記号・`.`/`..`・日本語・長い名前を含む `d` が、agent と publish の MFS パスで往復し、`find_garbage` が誤検知せず、削除できること）は Kubo のバイナリを子プロセスとして起動する（repo は一時ディレクトリ）。`SWING_TEST_KUBO_BIN` が無ければ何もせずに通る:

```bash
SWING_TEST_KUBO_BIN=/path/to/ipfs cargo test --lib kubo::tests -- --ignored
SWING_TEST_KUBO_BIN=/path/to/ipfs cargo test --test mfs_kubo_integration -- --ignored
```

ビルドとリリース、macOS・Homebrew・Windows の動作確認は [`architecture/release.md`](architecture/release.md)。
