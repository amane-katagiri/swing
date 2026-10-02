# 設定と環境変数（`src/config/`, `src/settings/`）

[`../architecture.md`](../architecture.md) の一部。キーごとの意味・既定値・環境変数名は [`../../swing.example.toml`](../../swing.example.toml)（`#field = value` はコメントアウトされた任意設定で、省略すると既定値か他の設定から導く値になる）。`[dashboard]` の各キーは [`dashboard.md#設定dashboard`](dashboard.md#設定dashboard)、`[gateway]` は [`gateway.md#設定gateway`](gateway.md#設定gateway)、Kubo の設定は [`kubo.md`](kubo.md)。

## 設定ファイルの場所

次の順で 1 つのパスに決まる（`config::locate_config`。パスだけが要る呼び出し元は `config::resolve_config_path`）。

1. `--config <path>`
2. 環境変数 `SWING_CONFIG`
3. `<カレントディレクトリ>/swing.toml`（ファイルがあるときだけ）
4. ユーザーごとの既定の場所の `swing.toml`（無くても使う）
5. 4 の場所を決められないときは `<カレントディレクトリ>/swing.toml`（無くても使う）

4 の場所（`default_config_dir`）は OS で決まる。

| OS | 場所 | 決められないとき |
|---|---|---|
| Linux ほか（macOS・Windows 以外） | `$XDG_DATA_HOME/swing`。`XDG_DATA_HOME` が無いか絶対パスでなければ `$HOME/.local/share/swing` | `XDG_DATA_HOME` が使えず、`HOME` が無い・絶対パスでない・既存のディレクトリでない |
| macOS | `$HOME/Library/Application Support/swing` | `HOME` が無い・絶対パスでない・既存のディレクトリでない |
| Windows | `%LOCALAPPDATA%\swing` | `LOCALAPPDATA` が無い・絶対パスでない・既存のディレクトリでない |

設定ファイルの隣の `data`（状態ファイルと Kubo のリポジトリ）もこの下に入る。ホームディレクトリの無いユーザー（Docker イメージのユーザーなど。[`docker.md`](docker.md)）は 5 になる。空文字の環境変数は未設定として扱う（下記）。

1 か 2 で指したファイルが無ければエラー終了。3〜5 で決まったファイルが無ければエラーにせず、既定値と環境変数だけで組み立てる（`Config.config_exists = false`）。ダッシュボードのセットアップ・設定編集はこのパスに新規作成・上書きし、親ディレクトリが無ければ作る（Unix では `0700`。`settings::write_atomic`）。

TOML の構文や型のエラーは `line <行>, column <桁>: <理由>` の形で報告し、該当行を引用しない。

## 設定カタログ

すべての設定キー（TOML のセクションとフィールド、環境変数、種類、`swing.example.toml` 上の見え方、編集可否、英日の説明）は `src/settings/mod.rs` の `SETTINGS`（`Setting` の配列）1 か所に持つ。

- 既定値は `config::build_config`（`src/config/build.rs`）が持つ。
- 環境変数は TOML の値を上書きする。`build_config` はキーごとの解決を `Resolver` にまとめ、値の出どころ（`Env`・`File`・`Default`）を `Config.sources` に記録する。
- パースや検証の失敗は、実際に使った出どころの名前で報告する（環境変数なら `invalid SWING_MAX_PER_SITE`・`SWING_POLL_INTERVAL must be greater than 0`、設定ファイルなら `invalid [policy].max_per_site`・`[agent].poll_interval must be greater than 0`）。例外として、`report_ttl` の検証（`poll_interval` の 2 倍より長いこと・上限）と `[gateway].hosts` の検証（`listen` が有効なら空でないこと・ダッシュボードのホスト名と重ならないこと）は、出どころに関わらず `report_ttl must be ...`・`[gateway].hosts ...` の固定の文言で報告する。
- `swing.example.toml` と `.env.example` はカタログから生成する（[`cli.md#config-example--config-env-example`](cli.md#config-example--config-env-example)）。どちらも `.gitattributes` で改行を LF に固定している。

`src/settings/` のテストは、カタログと次のものが食い違わないことを確かめる: `swing.example.toml`・`.env.example` の内容、example をパースした値と既定値、`Config.sources` のキー集合、`compose.yaml` の `mirror` サービスに固定値で書いた `SWING_*`（`ENV_NO_EFFECT_IN_COMPOSE`）、`settings::raw_value` が扱う編集可能なキー。

## 環境変数

- 空文字の環境変数（`SWING_CONFIG` を含む）は未設定として扱う。例外は `SWING_NO_PORT_SHIFT` で、clap の真偽値として読むので `true`・`false` 以外（空文字を含む）は起動エラーになる。
- カタログに無い `SWING_` 環境変数は `SWING_CONFIG` と `SWING_NO_PORT_SHIFT`（`swing up --no-port-shift` と同じ。[`up.md#セットアップモードでのポートの調整`](up.md#セットアップモードでのポートの調整)）だけ。ほかにテスト用の `SWING_TEST_*` がある。
- `SWING_KUBO_GATEWAY_BIND`・`SWING_GATEWAY_BIND`・`SWING_DASHBOARD_BIND` は `compose.yaml` の変数展開専用で、`swing` は読まない（[`docker.md#composeyaml`](docker.md#composeyaml)）。
- ログレベルは `RUST_LOG`。

## パスの設定

カタログの種類が `Path` のもの（`[agent].state_dir`・`[kubo].binary`・`[kubo].repo`・`[dashboard].custom_css`・`desktop_page`・`desktop_page_css`・`desktop_banner`・`mascots_dir`）の相対パスは、値の出どころで起点が変わる。

- 設定ファイルに書いた値と既定値（`state_dir` の `./data`）: 設定ファイルがあるか、パスがユーザーごとの既定の場所（上記の 4）なら、そのディレクトリを起点に `build_config` が絶対パスにする。
- 環境変数で渡した値と、それ以外で設定ファイルが無いときの既定値: そのまま残り、カレントディレクトリが起点になる。
- 他の値から導く既定値（`[kubo].repo` の `<state_dir>/kubo`）は解決後の `state_dir` に従う。

設定ファイルの書き換え（下記）は TOML の文書を直接編集するので、解決後の絶対パスがファイルに書き戻されることはない。

## 検証

違反はどれもエラー。

- `[nostr].relays` が空（TOML の空配列、または空の要素だけの環境変数）なら既定の relay（`DEFAULT_RELAYS`）に戻し、出どころを `Default` にする。エラーにはしない。
- `poll_interval`・`concurrency`・`max_sites_per_account`・`[publish].keep_versions`・`[agent].fetch_timeout`・`[agent].fetch_idle_timeout`・`[dashboard].max_upload` は 0 不可。
- `report_ttl` は半分が `poll_interval` より大きく、`nostr::MAX_REPORT_AGE`（[`nostr.md`](nostr.md#レプリカ報告の信頼度replicastier)）以下（超えると `report_ttl must be at most 7d`）。
- `mfs_root` は `/` で始まる絶対パス。`/` そのもの、空の要素、`.`、`..` は不可。末尾の `/` は取り除く。
- `[ipfs].api`・`[gateway].upstream`・`[dashboard].public_url`・`[dashboard].gateway` は `http(s)://host[:port]` の形。末尾の `/` を取り除いた値が、`Url` として解釈した http(s) のオリジンの文字列（`origin().ascii_serialization()`）と一致すること。パス・クエリ・フラグメント・`@`・制御文字・大文字のホスト名・既定のポートの明示は不可。`[dashboard].gateway` だけは空文字と空白だけの値も可（空文字として扱う）。
- `[kubo].managed = true` のときに `[ipfs].api` を指定すると `[ipfs].api conflicts with [kubo].managed = true`（環境変数なら `SWING_IPFS_API conflicts with ...`）。
- `[publish].dotfiles_allow` の各要素は前後の空白を除き、空の要素は捨てる。残りは `.` で始まり、`/` を含まず、`.`・`..` そのものでないこと（`config::validate_dotfile_name`）。TOML で空配列にすると何も見逃さない（環境変数では `,` だけを渡す）。未設定なら `config::DEFAULT_DOTFILES_ALLOW`。
- `[gateway].hosts` の規則（文字・`listen` との関係・ダッシュボードのホスト名との重なり）は [`gateway.md#hosts-の検証`](gateway.md#hosts-の検証)。
- `[kubo].provide_strategy` は空文字か空白だけなら不可。値そのものの妥当性は Kubo の起動時に任せる。
- `[kubo].storage_max` は容量、`[kubo].gateway_listen` は `SocketAddr`、`[kubo].swarm_port` は 1〜65535 としてパースする。
- `[kubo].binary` の実在は `config` では確かめない（`swing up` の起動時に `kubo::locate_binary` が確かめる）。

## 値の形式

- 容量: `"100GiB"`・`"512MiB"`・`"1TiB"`・`"512B"`。単位は `KiB`・`MiB`・`GiB`・`TiB` と `KB`・`MB`・`GB`・`TB` で、どちらも 1024 基数、大文字小文字を区別しない（compose の Kubo での違いは [`docker.md#kubo-の設定`](docker.md#kubo-の設定)）。数値部分は数字と `.` だけで、小数はバイト換算後に切り捨て。数値だけならバイト。u64 に収まらなければエラー。
- 時間: `"30s"`・`"10m"`・`"2h"`・`"365d"`。数値部分は整数だけ。数値だけなら秒。秒換算で u64 に収まらなければエラー。
- 秘密鍵は `Debug` 出力で `<redacted>` になる。

## 設定の書き換え（`src/settings/edit.rs`）

設定ファイルを書き換えるのは次の 3 つだけ。

| 関数 | 呼び出し元 | 書くキー |
|---|---|---|
| `settings::update` | [`PUT /api/config`](dashboard/http-api/config.md#put-apiconfig) | 下記の編集可能なキー |
| `settings::setup` | [`POST /api/setup`](dashboard/http-api/config.md#post-apisetup) | 編集可能なキーと `nostr.secret_key` |
| `settings::pin_addrs` | セットアップモードの `swing up`（[`up.md#セットアップモードでのポートの調整`](up.md#セットアップモードでのポートの調整)） | 種類が `SocketAddr` のキー |

### 編集できるキー

カタログで `editable: true` のキーだけを `PUT /api/config`・`POST /api/setup` で受け付ける。ほか（パス・待ち受けアドレス・ポート、`kubo.binary`・`kubo.managed`、`dashboard.ui`・`allowed_hosts`・`public_url`、`ipfs.*`、`gateway.*`、kind 番号、`nostr.secret_key` など）は書けない。

| キー | 種類 |
|---|---|
| `nostr.relays` | list |
| `nostr.mirror_set` | string |
| `policy.max_total_storage` / `max_per_site` / `max_per_account` / `max_update_size` | size |
| `policy.max_sites_per_account` / `keep_versions` / `keep_days` | integer |
| `policy.min_update_interval` / `nip05_cache_ttl` | duration |
| `policy.remove_on_unfollow` | bool |
| `policy.nip05` / `publish.nip05` / `publish.check_dotfiles` / `publish.check_size` / `publish.check_unchanged` | mode（`off`/`warn`/`require`） |
| `agent.poll_interval` / `report_ttl` | duration |
| `agent.concurrency` | integer |
| `publish.keep_versions` | integer |
| `publish.dotfiles_allow` | list |
| `kubo.storage_max` | size |
| `dashboard.gateway` | string |

- 値が環境変数から来ているキー（`source: "env"`）は、カタログで編集可能でも書けない。
- 編集できないキーか環境変数由来のキーが 1 つでも混ざれば要求全体を 400 で拒み、一部だけを適用することはない。

### 書き込みの手順

1. 書き込み先（[設定ファイルの場所](#設定ファイルの場所) で決まるパス）を毎回 `toml_edit::DocumentMut` として読む。無ければ空の文書から始める。
2. 渡された項目だけを書き換える（コメントや他のキーは残る）。
3. `config::build_config_from_str` で組み立て直して検証する。失敗したらファイルには触れない。
4. `settings::write_atomic` で書く。書き込み先がシンボリックリンクならたどった先の実体に書く。親ディレクトリが無ければ `auth::create_private_dir_all` で作り、`auth::write_private_file` で書く（手順は [`dashboard/security.md#トークン`](dashboard/security.md#トークン)）。Unix では既存・新規を問わず `0600` になる。
5. `Config::load` で読み直す。

失敗は `settings::EditError` の 2 種類に分かれる。

- `Invalid`（API は 400）: 編集できないキー・環境変数由来のキー・値の形式違い・組み立て直した設定の検証失敗・既存ファイルの TOML の解析失敗・鍵のある状態での `setup`。
- `Io`（API は 500 で、`error!` でログに出す）: 設定ファイルの読み込み（`NotFound` 以外）・親ディレクトリの作成・一時ファイルの書き込み・`rename`・書き込み後の `Config::load` の失敗。

### ダッシュボードでの直列化と反映

- 設定ファイルを書く API（`PUT /api/config`・`POST /api/setup`）と、`remote-signer.json` を書く `POST /api/signer/reconnect` は `AppState.config_writes: tokio::sync::Mutex<ConfigWrites>` を取ってから、ファイルの読み書き（`Config::load` を含む）を `api::blocking`（`spawn_blocking`）で行う。同じプロセス内では 1 つずつ順に走る。プロセス外からの同時書き込みは対象外。
- `ConfigWrites.setup_done` は、成功後の 2 回目の `POST /api/setup` を 409 にする印（[`dashboard/http-api/config.md#post-apisetup`](dashboard/http-api/config.md#post-apisetup)）。
- `AppState.restart_required: AtomicBool` は `PUT /api/config` か `POST /api/signer/reconnect` が一度でも成功すると `true` になり、プロセス内再起動まで戻らない（`POST /api/setup` は立てない）。`GET`/`PUT /api/config` の `restart_required` はこの値。
- `AppState.display_config: RwLock<Arc<Config>>` は起動時は `AppState.config` と同じで、`PUT /api/config` が成功するたびに書き換え後の設定に差し替わる。`GET /api/config` はこれを返す。relay・Kubo・agent が使う `AppState.config` は再起動まで変わらない。
- [`POST /api/setup`](dashboard/http-api/config.md#post-apisetup) はこの書き込みに、鍵（または `remote-signer.json`）の保存とプロセス内再起動が加わる。
