# SWING アーキテクチャ

現在のコードが何をしているかのリファレンス。実装非依存のプロトコル定義は [`protocol.md`](protocol.md)、設計の経緯や理由は [`log/`](log/) の実装ログ、元の計画は [`plan.md`](plan.md)、将来の拡張規約は [`extensions.md`](extensions.md) を参照。

## 構成要素

| 要素 | 実体 |
|---|---|
| 言語・ランタイム | Rust (edition 2024)、`tokio` |
| Nostr | `nostr-sdk` 0.45（署名検証、NIP-51、addressable event） |
| Kubo RPC | `reqwest`（rustls、multipart、stream）で `POST /api/v0/...` を直接呼ぶ。保存は pin ではなく MFS に置く |
| 設定 | `toml` + `serde`、環境変数が TOML を上書き |
| CLI | `clap` derive |
| ログ | `tracing` + `tracing-subscriber`（`RUST_LOG`、既定 `info`） |
| CID 検証 | `cid` クレート |

クレート `swing` は lib + bin 構成。`src/lib.rs` が各モジュールを公開し、`src/main.rs` は CLI エントリ。統合テストは `swing::` としてモジュールを直接使う。

`IpfsClient` と `HttpNip05Verifier` はそれぞれ自前の `reqwest::Client` を持つ。NIP-05 側はリダイレクトを追わない設定。agent は `HttpNip05Verifier::public_only()`、publish は `HttpNip05Verifier::new()` を使う（違いは「NIP-05 検証」を参照）。

## リポジトリ構成

```
swing/
  Cargo.toml, Cargo.lock
  src/
    lib.rs           各モジュールを公開するクレートルート
    main.rs          CLI エントリ (clap)
    config.rs        設定読み込み（TOML + 環境変数上書き）、サイズ・時間パーサ
    nostr.rs         relay 接続 / follow set 取得 / site event 購読・発行・パース
    ipfs.rs          Kubo RPC クライアント (add / dag export / dag stat / files mkdir・cp・rm・ls・stat)
    mfs.rs           MFS 上のパスの組み立て
    key.rs           swing key generate 用の鍵ペア生成（純粋関数）
    policy.rs        保存ポリシー判定（純粋関数）
    state.rs         state.json の永続化
    agent.rs         mirror-agent ループ
    publish.rs       publish サブコマンド
    mirror.rs        mirror list/add/remove, sites サブコマンド
    nip05.rs         NIP-05 検証（fetch + 判定）
  tests/
    kubo_integration.rs          Kubo 連携の統合テスト（#[ignore]）
    nostr_relay_integration.rs   relay 連携の統合テスト（#[ignore]）
  docker/
    kubo-init.d/     Kubo コンテナの /container-init.d にマウントする起動スクリプト
  Dockerfile         multi-stage（rust:1.97-slim-trixie → debian:trixie-slim）
  .dockerignore
  compose.yaml
  .env.example
  swing.example.toml
  README.md
  AGENTS.md, CLAUDE.md
  LICENSE            MIT
  docs/
    architecture.md  このファイル
    protocol.md      実装非依存のプロトコル定義
    plan.md          初期実装計画
    extensions.md    拡張時の命名規約と予約表
    todo.md          残タスク
    log/             実装ログ
    examples/publish.sh  ipfs CLI + nak によるプロトコルの参考実装
```

## CLI

```
swing agent   [--config <path>]
swing publish [--config <path>] [--site <d-tag>] --url <URL> [--nip05 <off|warn|require>] <DIR>
swing mirror list                      [--config <path>]
swing mirror add <key>...              [--config <path>]
swing mirror remove <key>...           [--config <path>]
swing sites                            [--config <path>]
swing key generate
```

### 設定ファイルの解決

1. `--config <path>`
2. 環境変数 `SWING_CONFIG`
3. `./swing.toml`（存在すれば）
4. 環境変数のみ

1 か 2 で指定したファイルが存在しない場合はエラー終了。3 が無い場合は環境変数だけで動く。

### agent

Follow Set の対象者のサイトイベントを購読し、ポリシーに従ってサイトを MFS に保存・削除し続ける常駐プロセス。動作の詳細は「mirror-agent の動作」。

### publish

`DIR` を Kubo に add して MFS の publish 用の場所に置き、サイトイベントに署名して全 relay に送る。送信に成功したら、同じサイトの古い版を MFS から消す。

- `--site` 省略時は `--url` のホスト名を `d` タグにする。
- `--nip05` 省略時は `[publish].nip05` / `SWING_PUBLISH_NIP05`（既定 `warn`）。
- `d` にドメイン名を使う場合は、そのドメインのルートを自分で管理していることが前提になる（NIP-05 がそのドメインの `/.well-known/nostr.json` を見に行くため）。サブパス配信や共有ホスティングなどでルートを管理していない場合は、`--site` にドメイン名の形をしない識別子を渡して NIP-05 を「対象外」にするか、`--nip05 off` で検証自体を無効にする。

処理順:

1. `--nip05` が `off` でなければ、`d` と自分の pubkey で NIP-05 を検証し、`Site:` の直後に `NIP-05` 見出しで結果を表示する（`✓ verified` / `! mismatch: ...` / `- not applicable (d is not a domain)` / `! error: ...`）。`require` で `Verified` 以外なら（`NotApplicable` を含め）IPFS への add を行わず終了する。
2. 現在時刻をイベントの `created_at` に決め、`DIR` を Kubo に add する（CIDv1、pin なし、`to-files` で `<mfs_root>/publish/<pubkey hex>/<site>/<created_at>` に置く。`<site>` は「MFS の使い方」を参照。同じパスに既にあれば先に消す）。
3. `dag/stat`（`offline=true`）の `TotalSize` を `size` タグにする。失敗したら（ブロックが欠けていたら）エラーで終了する。pin なしの add は Kubo の GC ロックを取らないので、add 中の GC でブロックが消えた場合をここで検出する。
4. サイトイベントを 2 の `created_at` で作り、自鍵で署名し全 relay に送る。relay ごとの成否（✓/✗）を表示する。
5. どの relay にも受理されなければエラーで終了し、古い版は消さない。
6. `<mfs_root>/publish/<pubkey hex>/<site>/` の中で名前が整数の項目を新しい順に並べ、`[publish].keep_versions` 個を残して消す。一覧に失敗したら警告を出して続ける。
7. `Published.` で終わる。

出力例:

```text
Site: https://ama.ne.jp/

NIP-05
  ✓ verified

IPFS
  CID: bafy...
  ✓ added to /swing/publish/<pubkey hex>/ama.ne.jp/1700000000
  Size: 12345 bytes

Nostr
  ✓ wss://relay.damus.io
  ✓ wss://nos.lol

Old versions (keeping 5)
  ✓ removed /swing/publish/<pubkey hex>/ama.ne.jp/1690000000

Published.
```

### mirror list / add / remove

自分の Follow Set（kind 30000, `d = mirror_set`）を操作する。

- `<key>` は npub / hex / nprofile を受け付ける。
- `add` / `remove` は既存の Follow Set を relay から取得し、`p` タグ以外の既存タグ（`title` 等）と `content`（NIP-51 の暗号化 private 部分）を保持したまま再署名して publish する。保持されるのはタグの値と `content` で、タグの並び順は `d` → その他のタグ → `p` の順に再構築される。
- 既存の Follow Set が無い状態で `add` すると、`["title", "SWING mirror list"]` タグ付きで新規作成する。
- 追加済みの鍵の `add`、未登録の鍵の `remove` は no-op として報告し、変更が無ければ publish しない。
- `list` は読み取り専用で、npub と hex を併記する。
- `list` / `add` / `remove` と `sites` は、relay から取得した Follow Set と `state.json` の `follow_set`（agent が保存したもの。検証条件は「Follow Set の選び方」と同じ）を比べ、新しい方を使う。保存済みの方を使ったときは、`(relays returned an older follow set; ...)` か `(follow set not found on relays; ...)` を表示する。relay から古い版しか取れないときに、それを元に編集して新しい版を上書きしないため。state.json は読むだけで書かない。

### sites

Follow Set の対象者ごとに、サイト単位で最新のサイトイベント 1 行（`d`、`cid`、`url`、`size`、`created_at`、NIP-05 検証結果、`[stored]` / `[not stored]`）を表示する。バージョンごとの一覧ではない。`state.json`（`[agent].state_dir`）があれば読んで、その CID を agent が保存しているかと検証結果に使う。読み取り専用で、state は作らない。

続けて、state に保存済みの版があるのに Follow Set にいない pubkey を `Unfollowed but still stored` の見出しの下に `[unfollowed]` 付きで表示する。サイトごとに state の最新の版を 1 行（`url` は `-`）出す。見出しには `remove_on_unfollow` に応じて、次の poll で消えるか、残している理由を添える。Follow Set が見つからない場合もこの部分は表示する。

### key generate

`nostr_sdk::Keys::generate()` で新しい鍵ペアを作り、nsec / npub / hex（秘密鍵・公開鍵）を表示する。設定ファイルも `SWING_NOSTR_SECRET_KEY` も不要で、`Config::load` を呼ばない。ロジックは `src/key.rs` の純粋関数（`Keys` を受け取り 4 つの文字列を返す）に切り出してあり、CLI 側はそれを表示するだけ。

## 設定と環境変数

環境変数は TOML の値を上書きする。

```toml
[nostr]
secret_key = "nsec1..."             # SWING_NOSTR_SECRET_KEY（nsec または hex）
relays = ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://yabu.me", "wss://relay-jp.nostr.wirednet.jp"]
                                    # SWING_NOSTR_RELAYS（カンマ区切り。未設定ならこの既定値）
mirror_set = "swing"                # SWING_MIRROR_SET（kind 30000 の d タグ）
site_event_kind = 35980             # SWING_SITE_EVENT_KIND

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

[publish]
nip05 = "warn"                      # SWING_PUBLISH_NIP05（off / warn / require、--nip05 が優先）
keep_versions = 5                   # SWING_PUBLISH_KEEP_VERSIONS
```

TOML キーの無い環境変数:

| 環境変数 | 意味 | 既定 |
|---|---|---|
| `SWING_CONFIG` | 設定ファイルのパス | `./swing.toml` |
| `SWING_FETCH_TIMEOUT` | 1 サイト分のコンテンツ取得（`dag/export`）全体のタイムアウト | `15m` |
| `SWING_FETCH_IDLE_TIMEOUT` | `dag/export` で次のデータが届かないまま待つ上限。最初のブロックが届くまでも含む | `2m` |

`poll_interval`、`concurrency`、`max_sites_per_account`、`[publish].keep_versions`、`SWING_FETCH_TIMEOUT`、`SWING_FETCH_IDLE_TIMEOUT` は 0 だと設定エラーになる。

`mfs_root` は `/` で始まる絶対パスで、ルート（`/`）そのもの、空の要素、`.`、`..` を含むものは設定エラーになる。末尾の `/` は取り除く。

### 値の形式

- 容量: `"100GB"`, `"512MB"`, `"1TB"`, `"512B"` のような文字列。1024 基数、大文字小文字を区別しない。数値部分は数字と `.` のみ（符号・指数表記・`inf`/`NaN` は不可）。`"1.5GB"` のような小数はバイト換算後に切り捨て。数値だけならバイト。u64 に収まらなければエラー。
- 時間: `"10m"`, `"2h"`, `"365d"`, `"30s"`。数値部分は整数のみ。数値だけなら秒。秒換算で u64 に収まらなければエラー。
- 秘密鍵は `Debug` 出力で `<redacted>` になる。

## Nostr イベント

イベント形式そのもの（kind 35980 サイトイベントのタグ定義、kind 30000 ミラー対象リストの定義、MUST/SHOULD の検証規則）は実装非依存の仕様として [`protocol.md`](protocol.md) にまとめてある。kind と `d` タグの予約・命名規約は [`extensions.md`](extensions.md)。ここには、この実装（`nostr.rs`）がその検証を具体的にどう行っているかだけを記す。

kind・`d` タグの既定値は「設定と環境変数」を参照（サイトイベント kind は `SWING_SITE_EVENT_KIND` 既定 `35980`、ミラー対象リストの `d` は `SWING_MIRROR_SET` 既定 `swing`）。

- `d` タグ: 空、253 バイト超、制御文字を含む場合はイベント全体を拒否する。
- `cid` タグ: `cid` クレートでパースできなければイベント全体を拒否する。
- `url` タグ: 2048 バイト超、または http(s) としてパースできない場合は `url` タグだけを無視し、イベント自体は受理する。
- ミラー対象リスト: agent は自分の pubkey が author のイベントだけを読む。relay の author フィルタに加えて受信後にも author を再確認する（二重チェック）。
- 暗号化された private 部分（`content`）は読まない。`mirror add/remove` はそのまま保持する。

## mirror-agent の動作

1. relay 群に接続する。
2. 自分の Follow Set を決める（後述「Follow Set の選び方」）。決まらなければ警告を出し、`poll_interval` ごとに再試行する。
3. 対象 pubkey 群のサイトイベントを過去分も含めて取得し（`kinds=[site_event_kind], authors=targets`）、以後は購読で新着を受ける。同じ `pubkey + d` は `created_at` 最大のものを最新とみなす。
4. 受理ゲート: 送信元 pubkey が現在の Follow Set に含まれないイベントは warn を出して無視する。購読 ID と kind が一致しない通知は debug ログで捨てる。
5. サイトイベントはサイト単位のタスクに渡して並行に処理する（後述「並行処理」）。各タスクは次の「保存の順序」に従って処理し、`state.json` を保存する。
6. `poll_interval` ごとに、まず sweep（後述）を行い、続いて Follow Set を再取得する。同時に対象全員のサイトイベントを取り直し、サイトごとの最新版を再投入する。投入するのは pubkey ごとに、保存済みのサイトすべてと、それ以外のサイトを `created_at` の新しい順に合計 `max_sites_per_account` 件まで（保存済みだけで上限を超えていれば保存済みのみ）。これにより一時的な取得失敗や保存失敗は次の tick で再試行される。
7. `remove_on_unfollow = true` のとき、Follow Set が決まるたびに、`state.sites` か `state.verifications` にエントリがある pubkey のうち今の Follow Set にいないものを削除して state を保存し、`<mfs_root>/agent/<pubkey hex>` を MFS から消す。前回の Follow Set との差分ではなく state と比べるので、agent の停止中に外した相手や、`false` から `true` に変えた時点で残っていた相手も消える。Follow Set が決まらない tick では何もしない。`false` のときは、外れた相手の保存済みの版を残す（新しい版は取らない。保持期間の適用は続くので、最新版は残り続ける。容量の集計にも入り続ける。起動時の突き合わせで壊れていた版は取り直さずに消える）。Follow Set の更新はこの削除より先に反映する。
8. 起動時に突き合わせ（後述）を行う。

relay の切断や Kubo のエラー、不正なイベントはログに出して処理を続ける。relay への再接続と再購読は nostr-sdk が自動で行う（再試行間隔 10 秒から最大 60 秒）。通知チャネル（容量 2048）が溢れた分は nostr-sdk が黙って捨てるが、6 の定期取り直しで回収される。通知ストリーム自体が終わった場合（relay プールの shutdown）はエラーで終了する。

### Follow Set の選び方

relay から取得した Follow Set と、`state.follow_set` に保存した Follow Set を比べて使う方を決める。relay が古い版を返したり、Follow Set を失ったりしても、外していない相手のサイトを消さないため。

- 取得では、kind 30000、作者が自分、`d` が `mirror_set`、署名が正しいものだけを候補にし、その中で最も新しいものを選ぶ。
- 新しさは NIP-01 の置き換え可能イベントの規則で比べる（`created_at` が大きい方、同じなら `id` が小さい方）。
- 保存済みの版も同じ条件（kind・作者・`d`・署名）を満たすときだけ使う。`mirror_set` を変えた場合、古い `d` の保存済みの版は使わない。

| relay から | 保存済み | 使う版 | state に保存 | relay に再送 |
|---|---|---|---|---|
| 取れた | 無い | 取れた版 | する | しない |
| 取れた（保存済みと同じ `id`） | ある | 保存済み | しない | しない |
| 取れた（保存済みより新しい） | ある | 取れた版 | する | しない |
| 取れた（保存済みより古い） | ある | 保存済み | しない | する |
| 見つからない | ある | 保存済み | しない | する |
| 取得に失敗 | ある | 保存済み | しない | しない |
| 見つからない、または失敗 | 無い | 決まらない | — | — |

再送は署名済みのイベントをそのまま全 relay に送る。どの relay にも受理されなければ warn を出す。自分で Follow Set を NIP-09 で削除しても、agent は保存済みの版を再送し続ける。ミラーをやめるときは `swing mirror remove` で対象を外す。

### MFS の使い方

agent も publish も Kubo の pin を使わず、MFS（Kubo 内のファイルシステム）にサイトの CID を置いて GC から守る。MFS に置いた CID は元の CID のままで、同じ CID を複数の場所に置いても、すべての場所から消えるまで GC されない。運用者が手動で付けた pin や、`mfs_root` の外に置いたものには触れない。

| パス | 持ち主 |
|---|---|
| `<mfs_root>/agent/<pubkey hex>/<site>/<created_at>` | agent。サイトの版ごとに 1 つ |
| `<mfs_root>/publish/<pubkey hex>/<site>/<created_at>` | publish。自分のサイトの版ごとに 1 つ |

- `<site>` は `d` をパーセントエンコードしたもの（`A-Z a-z 0-9 - . _ ~` 以外を `%XX`）。`d` が `.` または `..` のときはドットも `%2E` にする。
- `<created_at>` はサイトイベントの `created_at`（10 進）。
- `<mfs_root>/agent` の下は agent だけが使う。state が参照していない項目は、sweep ですべて消す。

MFS の保護は pin と違って、DAG が欠けていても登録でき、GC も欠けた部分を無視して進む。`block/rm` による直接の削除も止めない（Kubo 0.43.1 で確認）。そのため、置いた後の完全性は `dag/stat`（`offline=true`）で確認する。

MFS から消したコンテンツや、打ち切った取得のブロックは Kubo の blockstore に残り、GC で消える。compose の Kubo は `--enable-gc` で起動する（GC は repo が `Datastore.StorageMax` × `StorageGCWatermark` を超えたときに `Datastore.GCPeriod` ごとに走る）。

### 保存の順序

1. 事前判定: `size` タグの値（無ければ不明）で `policy::decide` する。skip ならここで終わり、NIP-05 検証も取得もしない。
2. NIP-05 検証（`[policy].nip05` が `off` 以外のとき）。`require` で `Verified` でなければ終わり。
3. 取得: `dag/export` で CAR を流し読みし、受信バイト数を数える。`policy::fetch_limit`（`max_update_size`・`max_per_site`・`max_per_account` の最小値）を超えた時点で打ち切る。`SWING_FETCH_IDLE_TIMEOUT` の間データが来ない、または `SWING_FETCH_TIMEOUT` を超えたら失敗。いずれも state と MFS は変えない。`size` タグは使わないので、小さく偽った `size` でも上限を超えて取得されない。
4. ここから先は state のロックを持ったまま行う。作者が Follow Set から外れていれば終わる。
5. 版のパスに CID を置く（既存の項目は先に消す。`files/cp` は `offline=true`）。失敗したら終わる。
6. `dag/stat`（`offline=true`）の `TotalSize` を実サイズとする。DAG のブロックが 1 つでも欠けていれば即エラーになるので、5 のパスを消して終わる。`size` タグより大きければ warn を出す。
7. 実サイズで `policy::decide` する。skip なら 5 のパスを消して終わる。
8. 新版を記録し、evict 対象を `sites` から消して state を保存してから、evict した版のパスを消す。

5 を 6 より先に行うのは、3 で取得したブロックは MFS に置くまで GC から守られないため。置いた後に確認すれば、確認の結果が GC で崩れない。

パスの削除（`files/rm`）に失敗しても state はそのままにし、次の sweep が消す。

### sweep

`poll_interval` の各 tick で Follow Set を再取得する前に、state のロックを持ったまま行う。最初の tick は起動直後（突き合わせの後）に来るので、起動時にも一度走る。

1. すべてのサイトに `policy::retention_evictions`（`max_per_site`・`keep_versions`・`keep_days`。最新版は必ず残す）を適用する。evict した版があれば `sites` から消して state を保存し、そのパスを消す。新しい版が来ないサイトや、設定で上限を下げたサイトにも効く。
2. `<mfs_root>/agent` を `<pubkey hex>/<site>/<created_at>` の 3 階層たどり、state の版に対応しない項目を消す。対応する版が 1 つも残らない `<site>` や `<pubkey hex>` のディレクトリ、想定外の階層にあるファイルも消す。一覧に失敗したディレクトリの下は消さない。
3. 1 で evict が無ければ state は保存しない。

### 起動時の突き合わせ

state の各版について、版のパスの CID（`files/stat`）が記録と一致し、かつ `dag/stat`（`offline=true`）が成功するかを確かめる。パスが無い、CID が違う、ブロックが欠けている場合は warn を出してその版を `sites` から消す（次の poll で取り直される。パスの項目は次の sweep で消える）。`files/stat` 自体が失敗した場合はその版を残す。消した版があれば state を保存する。すべての版の DAG をたどるので、保存量が多いと起動に時間がかかる。

### 並行処理

- タスクは同時に最大 `concurrency` 個が「保存の順序」の 1〜8 を実行する（セマフォ）。
- 同じ pubkey のサイトのタスクは同時に `max_sites_per_account` 個まで。超えた分のイベントは捨て、次の poll で拾い直す。
- 同じサイト（`pubkey:d`）のタスクは同時に 1 つだけ。実行中に同じサイトのイベントが来たら、実行中のものと待機中のものより `created_at` が新しいときだけ待機に置き（待機は 1 件で、新しいもので上書き）、実行が終わったら同じタスクで続けて処理する。
- MFS への配置・削除、ポリシー判定、state の更新（4〜8）と sweep・unfollow・突き合わせは state のロックの中で直列に行うので、並行に取得しても容量の判定は既に確定した版を必ず見る。取得中の一時的なディスク使用量は最大で `concurrency` × `fetch_limit` になる。
- Ctrl-C で終了するとき、実行中のタスクは中断される。state は一時ファイル経由で保存するので壊れない。

### ポリシー判定（policy.rs）

入力: 同サイトの既存版、使用量（他サイトの合計容量、同じ pubkey の他サイトの合計容量とサイト数）、候補イベント（cid, size, created_at）、ポリシー設定、現在時刻。出力: `Decision { store: Option<String>, evict: Vec<String>, reason: String }`。純粋関数。

判定順:

1. 同じ CID が同サイトに記録済みなら skip（`duplicate_cid`）。
2. 同サイトに記録済みの版が無く、同じ pubkey の他のサイト（記録済みの版があるもの）が `max_sites_per_account` 個以上あれば skip（`max_sites_per_account`）。既存サイトの更新には適用しない。
3. `created_at` が同サイトの最新受理版以下なら skip（`stale`）。`min_update_interval` の値によらず適用される。
4. `created_at` が最新受理版から `min_update_interval` 未満なら skip。
5. `size` が `max_update_size` を超えるなら skip。
6. 同サイト合計が `max_per_site` を超えるなら古い版から evict する。新版単体で超えるなら skip。
7. `keep_versions` 超過分の古い版を evict する。`keep_versions` は最低 1 に丸められる。
8. `keep_days` より古い版を evict する。最新版は残す。
9. 6〜8 の evict 後、同じ pubkey の全サイト合計が `max_per_account` を超えるなら skip（`max_per_account`）。同じアカウントの他サイトは削らない。
10. 6〜8 の evict 後の全サイト合計が `max_total_storage` を超えるなら skip。他サイトは削らない。

`size` が不明な事前判定では 5 を飛ばし、新版のサイズを 0 として 6〜10 を評価する。

### NIP-05 検証（nip05.rs）

`d` をドメインとして扱う条件: ラベルが 2 つ以上、`/ : @ ? #` を含まない、各ラベルが `[a-z0-9-]`（大文字は小文字化）で 1〜63 バイトかつ先頭・末尾が `-` でない、全体 253 バイト以下、WHATWG URL のホストとしてパースしたときに IP アドレスではなくドメインになる（`127.0.0.1`、`0x7f.1`、`example.123` などは不可）。満たさなければ `NotApplicable`。NIP-05 はそのドメインのルートを作者が管理していることを前提にした検証であり、ルートを管理していないサイトはドメイン名の形をしない `d`（`--site` で指定）を使うことで `NotApplicable` 扱いにできる。

ドメインなら `https://{d}/.well-known/nostr.json?name=_` を取得する。

| 条件 | 結果 |
|---|---|
| `names["_"]` がイベントの pubkey hex と一致（大文字小文字無視） | `Verified` |
| `_` が無い、または値が異なる | `Mismatch` |
| タイムアウト（10 秒）、非 2xx、64 KiB 超のボディ、非 UTF-8、JSON パース失敗 | `Error` |

リダイレクトは追わない。

agent 用の `public_only()` は、他人のイベントの `d` を宛先にするため内部ネットワークへのリクエストを防ぐ:

- 名前解決の結果から公開アドレス以外を除き、残らなければ `Error`。除外するのは IPv4 の unspecified・loopback・private・link-local・broadcast・documentation・multicast・`0.0.0.0/8`・`240.0.0.0/4`・`100.64.0.0/10`・`198.18.0.0/15`・`192.0.0.0/24`、IPv6 の unspecified・loopback・multicast・`fc00::/7`・`fe80::/10`・`2001:db8::/32`、および IPv4-mapped アドレスで中身が前述の IPv4 のもの。
- プロキシ環境変数を無視する（プロキシ側で名前解決されると上のフィルタを通らないため）。

publish 用の `new()` は自分の `d` を検証するだけなので、アドレスの制限もプロキシの無視もしない。

agent 側の適用（事前判定の後、取得の前）:

| モード | 動作 |
|---|---|
| `off` | 検証しない。記録しない |
| `warn` | 検証して結果を記録し、`Verified` 以外は warn ログ。取得に進む |
| `require` | 検証して結果を記録し、`Verified` のときだけ取得に進む |

結果は `state.verifications` をキャッシュとして使う。`checked_at` から `nip05_cache_ttl`（既定 1 日、`error` は 15 分とのうち短い方）が経つまでは再検証せず、記録済みの `status` が `verified` かどうかで判断する。`nip05_cache_ttl = 0` なら毎回検証する。検証結果を記録するたびに、その pubkey の保存されていないサイトの記録を `checked_at` の新しい順に `max_sites_per_account` 件だけ残して消す。

`Nip05Verify` トレイトとして定義され、テストではインメモリの fake を使う。

### state.json

`[agent].state_dir` 直下。一時ファイルに書いて rename する。

```json
{
  "sites": {
    "<pubkey hex>:<d>": [
      { "cid": "bafy...", "size": 12345, "created_at": 1700000000, "stored_at": 1700000100 }
    ]
  },
  "verifications": {
    "<pubkey hex>:<d>": { "status": "verified", "detail": null, "checked_at": 1700000100 }
  },
  "follow_set": { "id": "...", "pubkey": "...", "created_at": 1700000000, "kind": 30000, "tags": [["d", "swing"], ["p", "..."]], "content": "", "sig": "..." }
}
```

- `status` は `verified` / `mismatch` / `not_applicable` / `error`。
- `follow_set` は「Follow Set の選び方」で最後に保存した署名済みイベント（NIP-01 の JSON）。まだ無ければ `null`。
- `sites` と `verifications` は必須キー（空なら `{}`）。`follow_set` は serde の `Option` なので、キーが無くても `null` として読む。state.json が存在しない、または空白だけのときは空の state として扱う。
- キーの `<pubkey hex>` と `<d>` は最初の `:` で分ける（`d` に `:` が含まれてもよい）。
- `sites` の `size` は `dag/stat` の `TotalSize`（`size` タグの値ではない）。
- サイズは keep_versions × フォロー中サイト数に比例し、evict や unfollow でエントリは消える。`verifications` は保存されなかったサイトの分も pubkey ごとに `max_sites_per_account` 件まで残り、unfollow で消える。
- state を消すと、次の sweep で `<mfs_root>/agent` の下がすべて消え、次の poll で取り直しになる。

## Kubo RPC

| 操作 | リクエスト | タイムアウト |
|---|---|---|
| 取得 | `dag/export?arg={cid}&progress=false`（CAR をストリームで読み捨て、バイト数を数える） | 全体 `SWING_FETCH_TIMEOUT`（既定 15 分）、無通信 `SWING_FETCH_IDLE_TIMEOUT`（既定 2 分） |
| 実サイズ・完全性 | `dag/stat?arg={cid}&progress=false&offline=true` → `TotalSize` | 300 秒 |
| ディレクトリ作成 | `files/mkdir?arg={path}&parents=true` | 60 秒 |
| 配置 | `files/cp?arg=/ipfs/{cid}&arg={path}&offline=true`（親を作り、既存の項目を消してから） | 60 秒 |
| 削除 | `files/rm?arg={path}&recursive=true&force=true` | 60 秒 |
| 一覧 | `files/ls?arg={path}&long=true` → `Entries`（`Type` 1 がディレクトリ） | 60 秒 |
| CID の確認 | `files/stat?arg={path}&hash=true` → `Hash` | 60 秒 |
| add（publish） | `add?recursive=true&cid-version=1&pin=false&quieter=true&wrap-with-directory=false&to-files={path}` | 300 秒 |

すべて `POST /api/v0/...`。CID と MFS のパスはクエリに入れる前にパーセントエンコードする（パスは `/` を残して要素ごと。Kubo はクエリをデコードしてから使うので、`<site>` のエンコードと合わせて二重になる）。非 2xx はボディ付きのエラーになる。

- `dag/export` は最初のブロックが取れるまでレスポンスヘッダーも返さないので、無通信タイムアウトは送信からヘッダー受信までにも適用する。
- `offline=true` は Kubo の RPC 全体に共通のオプションで、ローカルに無いブロックをネットワークから探さずに即エラーにする。`files/cp` はルートのブロックだけを使う。
- `files/rm` は失敗しても 200 を返し、ボディにメッセージの文字列を入れる。成功時はボディが空なので、空でなければ失敗として扱う。`force=true` なので存在しないパスの削除は成功になる。
- `files/ls` と `files/stat` の `file does not exist` エラーは、それぞれ空の一覧、「無い」として扱う。`files/ls` は `long=true` でないと `Type` が常に 0 になる。
- `files/cp` は、同じ名前の項目があると `directory already has entry by that name` で失敗するので、配置の前に消す。

`add` の multipart:

- 各ファイルは `name="file"` パート。`filename` はルートディレクトリ自身の名前を先頭に付けた相対パス（例: ルートが `public/` なら `public/css/style.css`、URL エンコード）。
- ファイルは `Content-Type: application/octet-stream`、ディレクトリは空ボディの `Content-Type: application/x-directory`。
- ファイル内容は `tokio::fs::File` からストリーミングで送る。
- シンボリックリンクはリンク先を辿る。循環はエラー。
- 最後の JSON 行の `Hash` がルート CID。`ipfs add -Qr --cid-version=1` と同じ CID になる。
- `to-files` のパスにはルートディレクトリそのもの（ルートの名前ではなく指定したパス名）が置かれる。親ディレクトリは先に作っておく。

## Docker

### Dockerfile

- builder `rust:1.97-slim-trixie`、runtime `debian:trixie-slim`。同じ Debian コードネームに固定して glibc を一致させる。
- runtime には `/usr/local/bin/swing` だけを置く。
- ユーザー `swing`（uid/gid 1000）を作り `USER swing` で実行する。`/data` はそのユーザー所有の `VOLUME`。
- `ENTRYPOINT ["swing"]`、`CMD ["agent"]`。

### compose.yaml

| サービス | 内容 |
|---|---|
| `ipfs` | `ipfs/kubo:latest`。`command` はイメージ既定（`daemon --migrate=true --agent-version-suffix=docker`）に `--enable-gc` を足したもの。環境変数 `SWING_KUBO_STORAGE_MAX`（compose の変数展開で `SWING_KUBO_STORAGE_MAX`、無ければ `SWING_MAX_TOTAL_STORAGE`、無ければ `100GB`）と `SWING_KUBO_PROVIDE_STRATEGY`（無ければ `pinned+mfs`）。volume `ipfs-data:/data/ipfs` と `./docker/kubo-init.d:/container-init.d:ro`。公開ポートは `4001/tcp` と `4001/udp` のみ（RPC 5001 と Gateway 8080 は非公開）。healthcheck は `ipfs id` |
| `mirror` | `build: .`。`env_file: .env`。`SWING_IPFS_API=http://ipfs:5001`、`SWING_STATE_DIR=/data`、`RUST_LOG=info`。volume `swing-data:/data`。`depends_on: ipfs` を `condition: service_healthy` で待つ |

Kubo イメージの起動スクリプト（`start_ipfs`）は、repo の初期化の有無にかかわらず毎回の起動時に `/container-init.d/*.sh` を実行してから daemon を起動する。`docker/kubo-init.d/001-swing-config.sh` はそこで `ipfs config Datastore.StorageMax "$SWING_KUBO_STORAGE_MAX"` と `ipfs config Provide.Strategy "$SWING_KUBO_PROVIDE_STRATEGY"` を実行する。変数が空ならスクリプトが失敗し、コンテナは起動しない。

`Provide.Strategy` は DHT に「このノードが持っている」と告知する範囲。`pinned` だけにすると MFS にしか無い SWING のサイトが告知されず、他のノードから見つけてもらえなくなるので、`mfs` か `all` を含める。Kubo は知らない値を与えると daemon を起動しない（`all`・`pinned`・`roots`・`mfs`・`pinned+mfs` が通ることを確認した）。

両サービスとも `restart: unless-stopped`。名前付き volume は `ipfs-data` と `swing-data`。`swing-data` は初回マウント時にイメージ側の `/data` の所有者（`swing`）を引き継ぐ。

`.env.example` は `SWING_NOSTR_SECRET_KEY`、`SWING_NOSTR_RELAYS`、`SWING_MIRROR_SET`、`SWING_MAX_TOTAL_STORAGE` の 4 つと、コメントアウトした `SWING_KUBO_STORAGE_MAX`、`SWING_KUBO_PROVIDE_STRATEGY`。`.env` は mirror の `env_file` であると同時に、compose の変数展開（ipfs の環境変数）にも使われる。

## docs/examples/publish.sh

`ipfs` CLI と `nak` だけでプロトコル全体を再現する参考実装。サポート対象の CLI ではない。

- 引数: `publish.sh <site> <url> <dir>`
- 環境変数: `NOSTR_SEC`（必須、nsec または hex）、`NOSTR_RELAYS`（省略可、スペース区切り、既定は上記 5 relay）、`SITE_EVENT_KIND`（省略可、既定 35980）
- 処理: `ipfs add -Qr --cid-version=1` → `ipfs files stat --size /ipfs/<cid>` → `nak event -k <kind> -d <site> -t cid=... -t url=... -t size=... --sec $NOSTR_SEC <relays...>`
- 出力: `added` / `pinned` の素のテキスト行と `Event ID: <id>`

## テスト

- ユニットテスト: `cargo test`。`policy.rs`、`config.rs` のパーサ、`nostr.rs` のイベントパース、`agent.rs` の保存の順序・sweep・突き合わせ・並行処理の合流・NIP-05 キャッシュ（MFS をメモリ上で真似る `FakeKubo`、`FakeNip05`）、`mfs.rs` のパス、`mirror.rs` のタグ再構築、`nip05.rs` のホスト名判定と JSON 比較、`publish.rs` の NIP-05 判定など。
- 統合テスト（`#[ignore]`、ローカルの Kubo / relay が必要）:

```bash
# test プロファイルは bootstrap とローカル探索を無効にし、公開ネットワークに接続しない
docker run -d --rm -e IPFS_PROFILE=test -p 127.0.0.1:15001:5001 ipfs/kubo:latest
# SWING_TEST_IPFS_API（既定 http://127.0.0.1:15001）、任意で SWING_TEST_EXPECTED_CID
cargo test --test kubo_integration -- --ignored --test-threads=1
# agent の保存・sweep・突き合わせ・unfollow を実物の Kubo で通す
cargo test --lib agent_stores_and_removes_through_real_kubo -- --ignored

docker run -d --rm -p 127.0.0.1:18080:8080 scsibug/nostr-rs-relay
# SWING_TEST_RELAY（既定 ws://127.0.0.1:18080）
cargo test --test nostr_relay_integration -- --ignored
```

公開 relay や公開 IPFS にはテストで接続しない。
