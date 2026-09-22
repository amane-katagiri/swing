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
  web/               ダッシュボードのフロント（index.html, style.css, ES modules, 画像・フォントなどの静的アセット一式）。ビルド工程なしで include_str!/include_bytes! によりバイナリへ埋め込む。desktop-page.html / desktop-page.css / desktop-banner.gif（Desktop 画面のリンク集ページ）だけは設定で差し替えられる。詳細は architecture/dashboard.md
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
swing publish [--config <path>] --site <d-tag> [--url <URL>] [--nip05 <off|warn|require>] [--title <TEXT>] [-m, --message <TEXT>] <DIR>
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
min_update_interval = "1h"          # SWING_MIN_UPDATE_INTERVAL
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
gateway = "http://localhost:8080"   # SWING_DASHBOARD_GATEWAY（空文字でリンクを出さない）
#custom_css = "/path/to/custom.css" # SWING_DASHBOARD_CUSTOM_CSS
#desktop_page = "/path/to/links.html"     # SWING_DASHBOARD_DESKTOP_PAGE（Desktop 画面のリンク集ページ）
#desktop_page_css = "/path/to/links.css"  # SWING_DASHBOARD_DESKTOP_PAGE_CSS（そのページ専用の CSS）
#desktop_banner = "/path/to/banner.gif"   # SWING_DASHBOARD_DESKTOP_BANNER（88×31 バナー。png/gif/jpeg/webp/svg）
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
- `cid`: `cid` クレートでパースできなければイベント全体を拒否する。パースできても、コーデックが dag-pb（`0x70`）でなければイベント全体を拒否する（UnixFS のディレクトリ root は必ず dag-pb であり、CIDv0 は常に dag-pb なので通る）。ここまで通れば `nostr::canonical_cid` で CIDv1・base32 の正規形に変換し、以後（`SiteEvent::cid`・`ReplicaReport::cids`・`policy::decide`・`replicas_of`・`state.json`・MFS パス）はこの文字列だけを扱う。`swing publish` が Kubo から受け取った CID も `add_and_measure` で同じ関数に通す（Kubo は既定で base32 v1・dag-pb を返すのでほぼ無変換）。コーデックの検証はパース時の構文チェックに過ぎず、実際に取得した root が UnixFS のディレクトリであることは別に確かめる（`agent.md` の「保存の順序」4）。
- `url`: 2048 バイト超、制御文字を含む、または http(s) としてパースできなければ `url` だけを無視する。`swing publish --url` も同じ判定で拒否する。
- `title`: 空、256 バイト超、制御文字を含む場合は `title` だけを無視する。検証せず、保存の判断にも使わない。
- `content`: 空でなければ `SiteEvent::message` に入れる。検証せず、保存の判断にも使わない。
- Follow Set: relay の author フィルタに加え、受信後にも kind・作者・`d`・署名を確かめる。`content`（暗号化 private 部分）は読まない。
- レプリカ報告: `d` を最初の `:` で分け、作者が小文字 hex の公開鍵でない、サイトの `d` が上の `d` の条件を満たさない、`a` の値が `<site_event_kind>:<作者>:<サイトの d>` と一致しない、`cid` タグのどれかが `cid` クレートでパースできない、`expiration` タグがあるのに `u64` としてパースできない、のいずれかなら報告全体を拒否する。`cid` タグは 0 個でもよい（取り下げ）。`expiration` が無ければ `None` として読み、期限切れかどうかの判定は使う側（`ReplicaReport::counts_at`）が行う。
- 署名は nostr-sdk が受信時に検証する。
- relay からの取得（`fetch_events`）は 30 秒でタイムアウトする。
- `created_at` の未来ずれ許容（`nostr::MAX_FUTURE_SKEW`、900 秒）と、それを超えるかどうかを判定する `nostr::plausible_at(created_at, now)` は `nostr.rs` にある（`policy.rs` はここから読む）。保存の可否（`policy::decide`）だけでなく、「現在の版」やその時点で有効な Follow Set をどれとして選ぶかにも同じ基準を使う: `select_latest`（サイトイベント）、`choose_follow_set` と `RelayClient::fetch_follow_set` / `fetch_follow_sets`（Follow Set）、`newest_by_address`（Follow Set 以外にも使う住所ごとの最新選び）。`choose_follow_set` と `mirror.rs` の `newest_follow_set` は、relay から取得した版だけでなく保存済みの版も同じ基準でふるいにかける（先の時刻で一度保存された Follow Set が永久に勝ち続けるのを防ぐため）。詳細は [`agent.md`](architecture/agent.md#follow-set-の選び方) と [`cli.md`](architecture/cli.md)。

### 取得と表示の上限（`nostr::budget`）

kind 35980・35981・30000 は誰でも捨て鍵で出せるので、relay から読んで表示する読み取り専用の経路（`swing sites` / `replicas` / `webring`、ダッシュボードの `/api/sites` / `/api/replicas` / `/api/webring`）はすべて `nostr::budget` の定数で件数を打ち切る。

| 定数 | 値 | 適用箇所 |
|---|---|---|
| `MAX_FOLLOW_SET_ENTRIES` | 500 | `nostr::follow_set_pubkeys_capped`（`extract_follow_set_pubkeys` はこれの非切り詰め情報を捨てた薄いラッパ）。tag 順で最初の 500 件の重複しない `p` を残す。自分の Follow Set も例外ではなく、`agent::follow::refresh_follow_set` と `mirror::collect_sites` は切り詰められたら `warn!` を 1 回出す。`swing mirror add`（`mirror::apply_add`）はこの上限を超えて追加しようとするとエラーで終了し、黙って切り詰めない |
| `MAX_SITES_PER_AUTHOR_LISTED` | 50 | `nostr::cap_sites_per_author`（`select_latest` の直後に呼ぶ）。作者ごとに `d` の昇順で先頭 50 件だけを残す。`mirror::collect_sites`・`replicas::collect`・`webring::collect`・`/api/publish/sites`（`dashboard::api::publish_sites`）で使う。agent の取り込み側の件数制限（`agent::follow::limit_sites_per_account`、ポリシー値 `max_sites_per_account`）とは別物で、そちらはそのまま |
| `MAX_REPORTS_PER_SITE` | 200 | `replicas::collect_reports`。`newest_by_address` と `ReplicaReport::counts_at` でふるった後、サイトごとに `created_at` の新しい順で先頭 200 件を残す。`SiteReplicas.reports` は残した件数、`SiteReplicas.dropped` は切り捨てた件数（CLI の `swing replicas` と `/api/replicas` の `dropped`、ダッシュボードの reporter 一覧はここを見て「…and N more」を出す） |
| `MAX_CRAWL_NODES` | 1000 | `webring::crawl`。`Crawl.depths` がこれを超えて増えないように新規ノードの追加を止め、弾いた件数を `Crawl.over_budget`（→ `Graph.over_budget`）に積む。frontier に含まれないノードは `follow_sets` / `referencing` の relay 呼び出しにも現れないので、1 レベルあたりの件数も自然に抑えられる。テキスト出力・`/api/webring` の `over_budget` に表れる（`beyond`—深さの上限外で表示していないアカウント数—とは別のカウンタ） |
| `MAX_RELAY_FETCH_LIMIT` | 20,000 | `nostr::capped_limit` の上限値。個々の `limit()` 計算がどれだけ大きくなっても、relay 1 台への 1 回の REQ に付ける `limit` はこれを超えない |

relay への `Filter::limit`（`nostr::capped_limit(count, per)` = `min(count * per, MAX_RELAY_FETCH_LIMIT)`）:

- `fetch_site_events`: `authors.len() * MAX_SITES_PER_AUTHOR_LISTED`
- `fetch_replica_reports`: `sites.len() * MAX_REPORTS_PER_SITE`
- `fetch_follow_sets`: `authors.len() * 2`（1 作者につき有効な Follow Set は置き換え規則で 1 件のはずだが、relay が古い版を返す場合に備えて 2 倍）
- `fetch_follow_set_authors_referencing`: `targets.len() * 100`

`limit` は relay ごとの REQ に付くヒントであり、nostr-sdk は接続中の relay それぞれに同じフィルタを送るので、複数 relay を使う構成では合計の取得件数が `limit` の relay 数倍になり得る。また `fetch_replica_reports` の `limit` は複数サイトの座標をまとめて 1 つのフィルタに入れているため、1 サイトがレプリカ報告で埋め尽くされていると、relay 側の `limit` 適用で同じ問い合わせに混ざる他のサイトの報告が押し出されることがある。座標の数（≒ 問い合わせに含める作者数 × `MAX_SITES_PER_AUTHOR_LISTED`）を絞ることでしか被害の範囲は抑えられない（ダッシュボードは `key`/`root` を 1 リクエストあたり 100 件までに絞っている）。

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
