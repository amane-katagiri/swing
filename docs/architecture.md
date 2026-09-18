# SWING アーキテクチャ

今のコードが何をしているかのリファレンス。イベント形式は [`protocol.md`](protocol.md)、経緯と理由は [`log/`](log/) を参照。

| 文書 | 内容 |
|---|---|
| このファイル | 構成、CLI の一覧、設定、イベントの検証、テスト |
| [`architecture/cli.md`](architecture/cli.md) | 各サブコマンドの動作と出力 |
| [`architecture/agent.md`](architecture/agent.md) | mirror-agent の動作、ポリシー判定、レプリカ報告の送信、`state.json` |
| [`architecture/nip05.md`](architecture/nip05.md) | NIP-05 検証（agent と publish で共通） |
| [`architecture/kubo.md`](architecture/kubo.md) | MFS の使い方、Kubo RPC、Kubo のバージョン |
| [`architecture/docker.md`](architecture/docker.md) | Dockerfile、compose、Gateway |
| [`architecture/dashboard.md`](architecture/dashboard.md) | `swing agent` 内蔵の Web ダッシュボード（起動と終了、設定、ガード、静的ファイル） |
| [`architecture/dashboard/http-api.md`](architecture/dashboard/http-api.md) | ダッシュボードの HTTP API |
| [`architecture/dashboard/web.md`](architecture/dashboard/web.md) | ダッシュボードの画面と CSS カスタマイズ |

## 構成要素

| 要素 | 実体 |
|---|---|
| 言語・ランタイム | Rust (edition 2024)、`tokio` |
| Nostr | `nostr-sdk` 0.45 |
| Kubo RPC | `reqwest`（rustls、multipart、stream）で直接呼ぶ |
| ダッシュボードの HTTP サーバ | `axum` 0.8、リクエストタイムアウトに `tower-http` |
| 設定 | `toml` + `serde`、環境変数が TOML を上書き |
| CLI | `clap` derive |
| ログ | `tracing` + `tracing-subscriber`（`RUST_LOG`、既定 `info`） |
| CID 検証 | `cid` クレート |

クレート `swing` は lib + bin 構成。統合テストは `swing::` としてモジュールを直接使う。

## リポジトリ構成

```
swing/
  src/
    lib.rs           各モジュールを公開するクレートルート
    main.rs          CLI エントリ (clap)
    config.rs        設定読み込み、サイズ・時間パーサ
    nostr.rs         relay 接続 / follow set 取得 / site event 購読・発行・パース / レプリカ報告の組み立て・パース
    ipfs.rs          Kubo RPC クライアント
    mfs.rs           MFS 上のパスの組み立て
    key.rs           key generate
    policy.rs        保存ポリシー判定（純粋関数）
    state.rs         state.json の永続化
    agent/           mirror-agent ループ。詳細は architecture/agent.md
    health.rs        版と MFS の突き合わせ（agent と status で共通）、status サブコマンド
    publish.rs       publish サブコマンド
    mirror.rs        mirror list/add/remove, sites サブコマンド
    replicas.rs      レプリカ報告の集計、replicas サブコマンド
    webring.rs       Follow Set のたどり方とグラフの組み立て・出力、webring サブコマンド
    nip05.rs         NIP-05 検証
    dashboard/       agent 内蔵の Web ダッシュボード（mod.rs, guard.rs, api.rs, dto.rs, assets.rs）。詳細は architecture/dashboard.md
  web/               ダッシュボードのフロント（index.html, style.css, ES modules 一式）。ビルド工程なしで include_str! によりバイナリへ埋め込む。詳細は architecture/dashboard.md
  tests/
    kubo_integration.rs          Kubo 連携の統合テスト（#[ignore]）
    nostr_relay_integration.rs   relay 連携の統合テスト（#[ignore]）
  docker/kubo-init.d/  Kubo コンテナの起動スクリプト
  docker/caddy/        gateway プロファイルの Caddy 設定
  Dockerfile, compose.yaml, .env.example, swing.example.toml
  docs/                役割は AGENTS.md を参照
```

`mirror.rs`・`health.rs`・`replicas.rs`・`webring.rs`・`publish.rs`・`nostr.rs` は、relay/Kubo とやり取りして値を返す `collect_*` 系の関数と、それを表示する CLI 側の薄い関数とに分かれている。ダッシュボードの API ハンドラは同じ `collect_*` 関数を呼び、DTO に変換する。

## CLI

```
swing agent   [--config <path>]
swing publish [--config <path>] --site <d-tag> [--url <URL>] [--nip05 <off|warn|require>] [-m, --message <TEXT>] <DIR>
swing mirror list                      [--config <path>]
swing mirror add <key>...              [--config <path>]
swing mirror remove <key>...           [--config <path>]
swing sites                            [--config <path>]
swing replicas [<key>...]              [--config <path>]
swing status                           [--config <path>]
swing webring [<key>...] [--depth <N>] [--format <text|dot|mermaid>] [--config <path>]
swing key generate
```

設定ファイルは次の順に探す。1 か 2 で指定したファイルが無ければエラー終了。3 が無ければ環境変数だけで動く。

1. `--config <path>`
2. 環境変数 `SWING_CONFIG`
3. `./swing.toml`
4. 環境変数のみ

各サブコマンドの動作と出力は [`architecture/cli.md`](architecture/cli.md) を参照。

## 設定と環境変数

環境変数は TOML の値を上書きする。

```toml
[nostr]
secret_key = "nsec1..."             # SWING_NOSTR_SECRET_KEY（nsec または hex）
relays = ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://yabu.me", "wss://relay-jp.nostr.wirednet.jp"]
                                    # SWING_NOSTR_RELAYS（カンマ区切り）
mirror_set = "swing"                # SWING_MIRROR_SET（kind 30000 の d タグ）
site_event_kind = 35980             # SWING_SITE_EVENT_KIND
replica_event_kind = 35981          # SWING_REPLICA_EVENT_KIND

[ipfs]
api = "http://127.0.0.1:5001"       # SWING_IPFS_API
mfs_root = "/swing"                 # SWING_MFS_ROOT

[policy]
max_total_storage = "100GB"         # SWING_MAX_TOTAL_STORAGE
max_per_site = "10GB"               # SWING_MAX_PER_SITE
max_per_account = "20GB"            # SWING_MAX_PER_ACCOUNT
max_sites_per_account = 10          # SWING_MAX_SITES_PER_ACCOUNT
max_update_size = "2GB"             # SWING_MAX_UPDATE_SIZE
keep_versions = 5                   # SWING_KEEP_VERSIONS
keep_days = 365                     # SWING_KEEP_DAYS
min_update_interval = "10m"         # SWING_MIN_UPDATE_INTERVAL
remove_on_unfollow = true           # SWING_REMOVE_ON_UNFOLLOW
nip05 = "warn"                      # SWING_NIP05（off / warn / require）
nip05_cache_ttl = "1d"              # SWING_NIP05_CACHE_TTL

[agent]
state_dir = "./data"                # SWING_STATE_DIR
poll_interval = "5m"                # SWING_POLL_INTERVAL
concurrency = 4                     # SWING_CONCURRENCY
report_ttl = "3d"                   # SWING_REPORT_TTL

[publish]
nip05 = "warn"                      # SWING_PUBLISH_NIP05（--nip05 が優先）
keep_versions = 5                   # SWING_PUBLISH_KEEP_VERSIONS

[dashboard]
listen = "127.0.0.1:8082"           # SWING_DASHBOARD_LISTEN（"off" で無効）
allowed_hosts = []                  # SWING_DASHBOARD_ALLOWED_HOSTS（カンマ区切り、ポート抜き）
gateway = "http://127.0.0.1:8080"   # SWING_DASHBOARD_GATEWAY（空文字でリンクを出さない）
#custom_css = "/path/to/custom.css" # SWING_DASHBOARD_CUSTOM_CSS
max_upload = "2GB"                  # SWING_DASHBOARD_MAX_UPLOAD（POST /api/publish/upload のボディ上限。0 はエラー）
```

TOML キーの無い環境変数:

| 環境変数 | 意味 | 既定 |
|---|---|---|
| `SWING_CONFIG` | 設定ファイルのパス | `./swing.toml` |
| `SWING_FETCH_TIMEOUT` | 1 サイト分の取得（`dag/export`）全体のタイムアウト | `15m` |
| `SWING_FETCH_IDLE_TIMEOUT` | `dag/export` で次のデータ（最初のブロックを含む）を待つ上限 | `2m` |

空文字の環境変数は未設定として扱う。`[dashboard]` の各キーの意味と制約は [`architecture/dashboard.md`](architecture/dashboard.md#設定dashboard) を参照。

検証:

- `poll_interval`、`concurrency`、`max_sites_per_account`、`[publish].keep_versions`、`SWING_FETCH_TIMEOUT`、`SWING_FETCH_IDLE_TIMEOUT` は 0 だとエラー。
- `report_ttl` の半分が `poll_interval` 以下ならエラー。
- `mfs_root` は `/` で始まる絶対パス。`/` そのもの、空の要素、`.`、`..` を含むとエラー。末尾の `/` は取り除く。

値の形式:

- 容量: `"100GB"`、`"512MB"`、`"1TB"`、`"512B"`。1024 基数、大文字小文字を区別しない。数値部分は数字と `.` のみで、小数はバイト換算後に切り捨て。数値だけならバイト。u64 に収まらなければエラー。
- 時間: `"30s"`、`"10m"`、`"2h"`、`"365d"`。数値部分は整数のみ。数値だけなら秒。秒換算で u64 に収まらなければエラー。
- 秘密鍵は `Debug` 出力で `<redacted>` になる。

## Nostr イベントの検証（nostr.rs）

形式と MUST/SHOULD は [`protocol.md`](protocol.md)、kind と `d` の予約は [`extensions.md`](extensions.md)。この実装の判定:

- `d`: 空、253 バイト超、制御文字を含む場合はイベント全体を拒否する。
- `cid`: `cid` クレートでパースできなければイベント全体を拒否する。
- `url`: 2048 バイト超、または http(s) としてパースできなければ `url` だけを無視する。
- `content`: 空でなければ `SiteEvent::message` に入れる。検証せず、保存の判断にも使わない。
- Follow Set: relay の author フィルタに加え、受信後にも kind・作者・`d`・署名を確かめる。`content`（暗号化 private 部分）は読まない。
- レプリカ報告: `d` を最初の `:` で分け、作者が小文字 hex の公開鍵でない、サイトの `d` が上の `d` の条件を満たさない、`a` の値が `<site_event_kind>:<作者>:<サイトの d>` と一致しない、`cid` タグのどれかが `cid` クレートでパースできない、のいずれかなら報告全体を拒否する。`cid` タグは 0 個でもよい（取り下げ）。`expiration` は読むだけで、期限切れの判定は使う側が行う。
- 署名は nostr-sdk が受信時に検証する。
- relay からの取得（`fetch_events`）は 30 秒でタイムアウトする。

## テスト

- ユニットテスト: `cargo test`。agent のテストは MFS をメモリ上で真似る `FakeKubo` と `FakeNip05` を使う。ダッシュボードのテストは axum の `Router` に `oneshot` でリクエストを投げ、TCP で listen しない。
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
