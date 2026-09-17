# SWING アーキテクチャ

今のコードが何をしているかのリファレンス。イベント形式は [`protocol.md`](protocol.md)、経緯と理由は [`log/`](log/) を参照。

| 文書 | 内容 |
|---|---|
| このファイル | 構成、CLI、設定、イベントの検証、テスト |
| [`architecture/agent.md`](architecture/agent.md) | mirror-agent の動作、ポリシー判定、レプリカ報告の送信、`state.json` |
| [`architecture/nip05.md`](architecture/nip05.md) | NIP-05 検証（agent と publish で共通） |
| [`architecture/kubo.md`](architecture/kubo.md) | MFS の使い方、Kubo RPC、Kubo のバージョン |
| [`architecture/docker.md`](architecture/docker.md) | Dockerfile、compose、Gateway |

## 構成要素

| 要素 | 実体 |
|---|---|
| 言語・ランタイム | Rust (edition 2024)、`tokio` |
| Nostr | `nostr-sdk` 0.45 |
| Kubo RPC | `reqwest`（rustls、multipart、stream）で直接呼ぶ |
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
    agent.rs         mirror-agent ループ
    health.rs        版と MFS の突き合わせ（agent と status で共通）、status サブコマンド
    publish.rs       publish サブコマンド
    mirror.rs        mirror list/add/remove, sites サブコマンド
    replicas.rs      レプリカ報告の集計、replicas サブコマンド
    webring.rs       Follow Set のたどり方とグラフの組み立て・出力、webring サブコマンド
    nip05.rs         NIP-05 検証
  tests/
    kubo_integration.rs          Kubo 連携の統合テスト（#[ignore]）
    nostr_relay_integration.rs   relay 連携の統合テスト（#[ignore]）
  docker/kubo-init.d/  Kubo コンテナの起動スクリプト
  docker/caddy/        gateway プロファイルの Caddy 設定
  Dockerfile, compose.yaml, .env.example, swing.example.toml
  docs/                役割は AGENTS.md を参照
```

## CLI

```
swing agent   [--config <path>]
swing publish [--config <path>] [--site <d-tag>] --url <URL> [--nip05 <off|warn|require>] <DIR>
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

### agent

Follow Set の対象者のサイトを MFS に保存・削除し続ける常駐プロセス。[`architecture/agent.md`](architecture/agent.md) を参照。

### publish

`DIR` を Kubo に add して MFS に置き、サイトイベントに署名して全 relay に送り、同じサイトの古い版を MFS から消す。

- `--site` 省略時は `--url` のホスト名を `d` にする。
- `--nip05` 省略時は `[publish].nip05`。

処理順:

1. `--nip05` が `off` でなければ、`d` と自分の pubkey で NIP-05 を検証し、`NIP-05` 見出しの下に結果を表示する（`✓ verified` / `! mismatch: ...` / `- not applicable (d is not a domain)` / `! error: ...`）。`require` で `Verified` 以外（`NotApplicable` を含む）なら add せず終了する。
2. 現在時刻を `created_at` に決め、`DIR` を CIDv1・pin なしで add し、`<mfs_root>/publish/<pubkey hex>/<site>/<created_at>` に置く（既存の項目は先に消す）。
3. `dag/stat`（`offline=true`）の `TotalSize` を `size` タグにする。失敗したら（ブロックが欠けていたら）エラーで終了する。
4. サイトイベント（`alt` は `SWING site announcement: <d>`）を 2 の `created_at` で作って署名し、全 relay に送る。relay ごとの成否（✓/✗）を表示する。どこにも受理されなければエラーで終了し、古い版は消さない。
5. `<mfs_root>/publish/<pubkey hex>/<site>/` の中で名前が整数の項目を新しい順に `[publish].keep_versions` 個残して消す（`Old versions (keeping N)` 見出し）。一覧に失敗したら警告を出して続ける。
6. `Published.` で終わる。

出力例は README にある。

### mirror list / add / remove

自分の Follow Set（kind 30000、`d = mirror_set`）を操作する。

- `<key>` は npub / hex / nprofile を受け付ける。
- `add` / `remove` は `p` 以外のタグの値と `content`（NIP-51 の暗号化 private 部分）を保持して再署名する。タグは `d` → その他 → `p` の順に並べ直す。
- Follow Set が無い状態の `add` は `["title", "SWING mirror list"]` 付きで新規作成する。
- 追加済みの `add`、未登録の `remove` は no-op と報告し、変更が無ければ publish しない。
- `list` は npub と hex を併記する。
- relay の Follow Set と `state.json` の `follow_set` を比べて新しい方を使う（検証条件は [agent の Follow Set の選び方](architecture/agent.md#follow-set-の選び方) と同じ）。保存済みの方を使ったときは `(relays returned an older follow set; ...)` か `(follow set not found on relays; ...)` を表示する。state.json は読むだけ。`sites` も同じ。

### sites

読み取り専用。state は作らない。

- Follow Set の対象者ごとに、サイトごとの最新のサイトイベントを 1 行（`d`、`cid`、`url`、`size`、`created_at`、NIP-05 検証結果、`replicas`、`[stored]` / `[not stored]`）表示する。検証結果と保存状況は `state.json` から読む。`replicas` は [replicas](#replicas) と同じ集計の最新版のレプリカ数。レプリカ報告の取得に失敗したら `(fetching replica reports failed: ...)` を表示して `-` にする。
- 続けて、state に版があるのに Follow Set にいない pubkey を `Unfollowed but still stored` 見出しの下に `[unfollowed]` 付きで、サイトごとに state の最新版を 1 行（`url` は `-`）表示する。見出しには `remove_on_unfollow` に応じて、次の poll で消えるか残しているかを添える。Follow Set が見つからなくても表示する。

### replicas

読み取り専用。state は読まない。

- `<key>`（npub / hex / nprofile）を作者として扱う。省略時は自分の pubkey。
- 作者ごとに、サイトごとの最新のサイトイベントについて `d`、`cid`、`replicas=<最新版を持つ報告者数> (reports=<有効な報告の数>)` を表示し、続けて報告者ごとに npub と `[latest]` / `[older version]` を 1 行ずつ表示する。最新版を持つ報告者を先に、同じ中では hex の順に並べる。
- 報告者が作者なら `[author]`、報告者の Follow Set（kind 30000、`d = mirror_set`、NIP-01 の置き換え規則で最新のもの）に作者がいなければ `[not following]` を添える。
- 集計（`replicas::collect_reports`）: サイトイベントの座標（`35980:<作者>:<d>`）を `#a` に入れて `replica_event_kind` の報告を取得し、報告者・`d` ごとに最新の 1 件だけを残す。パースに失敗したもの（検証は下の [Nostr イベントの検証](#nostr-イベントの検証nostrrs)）、`cid` タグが無いもの、`expiration` を過ぎたものは数えない。
- サイトイベント・報告・Follow Set のどれかの取得に失敗したらエラーで終了する。

### status

読み取り専用。relay には接続せず、`state.json` と Kubo だけを見る。state も MFS も変えない。

- `state.json` の版ごとに、版のパス・`cid`・`size` と判定を 1 行表示する。判定は [起動時の突き合わせ](architecture/agent.md#起動時の突き合わせ) と同じ検査で、`[ok]` / `[missing]`（パスが無い）/ `[cid mismatch]` / `[incomplete]`（ブロックが欠けている）/ `[check failed]`（`files/stat` 自体が失敗）のいずれか。`ok` 以外は理由を添える。
- 続けて `Not in state` 見出しの下に、[sweep](architecture/agent.md#sweep) が消す MFS のパスを表示する。ディレクトリごと消えるものはそのディレクトリだけを出す。一覧に失敗したディレクトリは `[list failed]` 付きで出す。
- `ok` 以外の版と `Not in state` の項目が 1 つでもあれば、件数を表示して 0 以外で終了する。
- agent の実行中は、保存途中の版（MFS に置いた後、state を保存する前）が `Not in state` に出ることがある。
- 全版の DAG をたどるので、保存量に比例して時間がかかる。

### webring

読み取り専用。state は読まない。

- `<key>`（npub / hex / nprofile）を起点にする。省略時は自分の pubkey。`--depth` の既定は 2、`--format` の既定は `text`。
- たどり方（`webring::crawl`）: 起点を深さ 0 とし、深さ `d` のアカウントについて Follow Set（kind 30000、`d = mirror_set`、作者ごとに NIP-01 の置き換え規則で最新のもの）を取得する。`d < depth` なら、その `p` のアカウントと、`#p` にそのアカウントを含む Follow Set の作者を、まだ見ていなければ深さ `d + 1` にする。`#p` での取得は作者を見つけるためだけに使い、辺は作者で取得した Follow Set からだけ作る。深さ `depth` のアカウントも Follow Set は取得するが、先へは広げない。
- グラフ（`webring::build_graph`）: 取得した Follow Set の `p` のうち、見つけたアカウントを指すものを辺（A → B は A の Follow Set に B がいる）にする。自分自身への辺は捨てる。辺を向きを無視してたどり、起点につながらないアカウント（`#p` で見つけたが、最新の Follow Set ではもう指していない作者など）は除く。
- 残ったアカウントのサイトイベントを取得し、サイトごとの最新版の `d` をアカウントの名前にする。
- `text`: 見出しに件数、`Accounts` にアカウントごとの名前（`d` を `, ` でつないだもの。無ければ縮めた npub。同じ名前が複数あれば縮めた npub を添える）・npub・深さ・`[root]` / `[no follow set]`、`Mutual` に双方向の組、`One-way` に片方向の辺を出す。並びは（深さ、名前）の順。深さの上限の外にいて表示しなかった、残ったアカウントの Follow Set に載っているアカウントがあれば、その数を最後に出す。
- `dot`: Graphviz の `digraph`。ノード ID は hex、ラベルは名前と縮めた npub。起点は `penwidth=2`、双方向の組は `dir=both` の 1 本にする。
- `mermaid`: `graph LR`。ノード ID は `n<番号>`（hex 順）、ラベルは名前と縮めた npub で、`#` `&` `"` `<` `>` はエンティティにする。起点は `root` クラス、双方向の組は `<-->` にする。
- Follow Set・サイトイベントのどれかの取得に失敗したらエラーで終了する。

### key generate

新しい鍵ペアの nsec / npub / hex（秘密鍵・公開鍵）を表示する。設定を読まない。

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
```

TOML キーの無い環境変数:

| 環境変数 | 意味 | 既定 |
|---|---|---|
| `SWING_CONFIG` | 設定ファイルのパス | `./swing.toml` |
| `SWING_FETCH_TIMEOUT` | 1 サイト分の取得（`dag/export`）全体のタイムアウト | `15m` |
| `SWING_FETCH_IDLE_TIMEOUT` | `dag/export` で次のデータ（最初のブロックを含む）を待つ上限 | `2m` |

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
- Follow Set: relay の author フィルタに加え、受信後にも kind・作者・`d`・署名を確かめる。`content`（暗号化 private 部分）は読まない。
- レプリカ報告: `d` を最初の `:` で分け、作者が小文字 hex の公開鍵でない、サイトの `d` が上の `d` の条件を満たさない、`a` の値が `<site_event_kind>:<作者>:<サイトの d>` と一致しない、`cid` タグのどれかが `cid` クレートでパースできない、のいずれかなら報告全体を拒否する。`cid` タグは 0 個でもよい（取り下げ）。`expiration` は読むだけで、期限切れの判定は使う側が行う。
- 署名は nostr-sdk が受信時に検証する。
- relay からの取得（`fetch_events`）は 30 秒でタイムアウトする。

## テスト

- ユニットテスト: `cargo test`。agent のテストは MFS をメモリ上で真似る `FakeKubo` と `FakeNip05` を使う。
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
