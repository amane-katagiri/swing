# CLI（main.rs と各サブコマンド）

サブコマンドの一覧と設定ファイルの探し方は [`../architecture.md`](../architecture.md#cli)、出力例は README を参照。

共通:

- `<key>` は npub / hex / nprofile を受け付ける。
- 「Follow Set」は kind 30000、`d = mirror_set` のうち、作者ごとに NIP-01 の置き換え規則で最新のもの。`created_at` が現在時刻より 900 秒（`nostr::MAX_FUTURE_SKEW`）を超えて先のものは、それが relay から取れた最新であっても無いものとして扱う（`RelayClient::fetch_follow_set` / `fetch_follow_sets`）。「サイトごとの最新のサイトイベント」（sites・replicas・webring で使う `nostr::select_latest`）も同じ基準で、先すぎる `created_at` のイベントは選ばない。
- `up`・`publish`・`service install`/`uninstall` 以外は読み取り専用で、`state.json` も MFS も OS のファイルも変えない（`mirror add` / `remove` は Follow Set を relay に送る。`stop`／`service stop` は動いているプロセスに停止・再起動を要求するだけで、ファイルは変えない）。`service install`/`uninstall` は OS のサービス定義ファイル（systemd unit / launchd plist / タスクスケジューラのタスク）を書く・消す。
- `up` は処理を始める前に `<[agent].state_dir>/swing.lock` のインスタンスロックを取る（[`up.md#多重起動の防止lockrs`](up.md#多重起動の防止lockrs)）。同じ `state_dir` に対して既に動いていれば、起動側のエラーで即座に終了する。
- `status`・`mirror add`・`mirror remove`・`stop`／`service stop` は relay/Kubo に直接つながず、動いている `swing up` のダッシュボード API（`[dashboard].listen`、既定 `http://127.0.0.1:8082`）を `src/api_client.rs::ApiClient` 経由で叩く。API が `[dashboard].listen` を未指定アドレス（`0.0.0.0` / `::`）で待ち受けていても、クライアントは接続先と `Host` ヘッダをループバックの同じポートへ正規化する。API に接続できなければ `status`・`mirror add`・`mirror remove` は `swing up is not running (cannot connect to <addr>)` でエラー終了し（非ゼロ終了）、`stop`／`service stop` は `not running` を出して正常終了（終了コード 0）する。`sites`・`replicas`・`webring`・`mirror list`・`publish` はこの API を経由せず relay/Kubo に直接つなぐので、`swing up` が動いていなくても使える。
- `[nostr].secret_key`（`SWING_NOSTR_SECRET_KEY`）は必須ではなくなった。`config::Config::require_secret_key()` を呼ぶコマンド（`sites`・`replicas`・`webring`・`mirror list`・`publish`）は鍵が無ければエラー終了するが、`status`・`mirror add`・`mirror remove`・`stop`／`service stop` はダッシュボード API 経由で鍵を直接使わないので鍵が無くても動く。ただし鍵が無い `swing up` はセットアップモードで動いており（[`up.md#セットアップモード鍵未設定`](up.md#セットアップモード鍵未設定)）、そこでは `status`・`mirror add`・`mirror remove` は 503 `agent is not configured` を返す。

## up

`[kubo].managed` に応じて Kubo（子プロセス）と mirror-agent の中身を 1 プロセスの supervisor として動かす。mirror-agent はこの `up` だけが起動でき、単体で動かすサブコマンドは無い。Kubo・agent いずれかが落ちても自動で再起動する（[`up.md`](up.md)）。`--log-file <path>` を指定すると、標準エラーの代わりにそのファイルへ追記でログを出す。`[nostr].secret_key` が設定ファイル・環境変数のどちらにも無ければ、Kubo も agent も起動せずダッシュボードだけを動かすセットアップモードになる（[`up.md#セットアップモード鍵未設定`](up.md#セットアップモード鍵未設定)）。ダッシュボードのセットアップ画面（`POST /api/setup`）が鍵を書き込むと、プロセスを終了させずに（同じ PID のまま）設定を読み直して通常モードで動き直す（[`up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）。

## stop

動いている `swing up` インスタンスに正常終了（グレースフルシャットダウン）を要求する（[`up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)、[`service.md#swing-stopstoprs`](service.md#swing-stopstoprs)）。`--config`（省略時は `SWING_CONFIG` または `./swing.toml`）・`--restart`（止めるのではなく再起動を要求する）・`--timeout <秒>`（既定 60。この秒数だけ停止を待ち、超えたらエラー）を取る。

実装（`src/stop.rs`）はダッシュボード API だけを使う。まず `POST /api/shutdown`（`--restart` なら `/api/restart`）を叩く。API に接続できなければ（`swing up` 自体が動いていない）`not running` を出して正常終了する。呼び出しが通れば `GET /api/overview` を 500ms 間隔でポーリングし、接続できなくなった時点（プロセスが終了した時点）で `stopped` を出して正常終了する。`--timeout` はこのポーリングの上限で、超えたらエラー終了する。

## service install / uninstall / status / stop

`swing up` を OS のログイン/システムサービスとして登録する（systemd user unit・launchd LaunchAgent・Windows タスクスケジューラ、[`service.md`](service.md)）。`install` は `--config`（省略時は `SWING_CONFIG` または `./swing.toml`。どちらも無ければエラー）・`--system`（Linux のみ）・`--no-start`（登録だけで起動しない）を取る。`uninstall`/`status`/`stop` は `--system` のみ。`stop` は登録を残したままプロセスだけ止める。OS ごとの実体（`systemctl stop` / `launchctl kill` / Windows は `swing stop` と同じ API 経由）は [`service.md`](service.md) の各 OS の節を参照。上の `stop`（`swing stop`）とは別で、こちらはサービス機構を通す。`uninstall` はどの OS でも止めてから登録を消す。Linux（`systemctl disable --now`）と macOS（`launchctl bootout`）はサービス機構が SIGTERM を送るのでそれ自体がグレースフル、Windows は `schtasks /End` が強制終了なので、その前に `swing stop` と同じ手順で止めてから `/End` → `/Delete` する。

## publish

`DIR` を Kubo に add して MFS に置き、サイトイベントに署名して全 relay に送り、同じサイトの古い版を MFS から消す。

- `--site` は必須で、そのまま `d` になる。[`d` の条件](../architecture.md#nostr-イベントの検証nostrrs)を満たさなければ何もせず終了する。
- `--url` は任意。指定すると `url` タグになり、http / https の URL でなければ何もせず終了する。省略すると `url` タグを付けない（IPFS だけで公開するサイト）。
- `--nip05` 省略時は `[publish].nip05`。
- `--title` は任意。指定すると `title` タグになる。空文字・空白のみは付けない扱いにする。256 バイトを超える、または制御文字を含む場合は何もせず終了する。
- `--message` はサイトイベントの `content` になる。最初に `Site: <d>`、`--url` があれば `URL:`、`--title` があれば `Title:`、`--message` があれば `Message:` を表示する。省略時は空文字。

処理順:

1. `--nip05` が `off` でなければ、`d` と自分の pubkey で NIP-05 を検証し、`NIP-05` 見出しの下に結果を表示する（`✓ verified` / `! mismatch: ...` / `- not applicable (d is not a domain)` / `! error: ...`）。`require` で `Verified` 以外（`NotApplicable` を含む）なら add せず終了する。
2. 現在時刻を `created_at` に決め、`DIR` を CIDv1・pin なしで add し、`<mfs_root>/publish/<pubkey hex>/<site>/<created_at>` に置く（既存の項目は先に消す）。
3. `dag/stat`（`offline=true`）の `TotalSize` を `size` タグにする。失敗したら（ブロックが欠けていたら）エラーで終了する。
4. サイトイベント（`alt` は `SWING site announcement: <d>`）を 2 の `created_at` で作って署名し、全 relay に送る。relay ごとの成否（✓/✗）を表示する。どこにも受理されなければエラーで終了し、古い版は消さない。
5. `<mfs_root>/publish/<pubkey hex>/<site>/` の中で名前が整数の項目を新しい順に `[publish].keep_versions` 個残して消す（`Old versions (keeping N)` 見出し）。一覧に失敗したら警告を出して続ける。
6. `Published.` で終わる。

## mirror list / add / remove

自分の Follow Set を操作する。

- `add` / `remove` は `p` 以外のタグの値と `content`（NIP-51 の暗号化 private 部分）を保持して再署名する。タグは `d` → その他 → `p` の順に並べ直す。
- Follow Set が無い状態の `add` は `["title", "SWING mirror list"]` 付きで新規作成する。
- 追加済みの `add`、未登録の `remove` は no-op と報告し、変更が無ければ publish しない。
- `add` は結果の `p` タグ数が `nostr::budget::MAX_FOLLOW_SET_ENTRIES`（500）を超えるならエラーで終了し、publish しない（黙って切り詰めない）。
- `list` は npub と hex を併記する。
- relay の Follow Set と `state.json` の `follow_set` を比べて新しい方を使う（検証条件は [agent の Follow Set の選び方](agent.md#follow-set-の選び方) と同じ）。保存済みの方を使ったときは `(relays returned an older follow set; ...)` か `(follow set not found on relays; ...)` を表示する。state.json は読むだけ。`sites` も同じ。
- `list` は relay に直接つなぎ（`mirror::collect_mirror_list`）、`swing up` が動いていなくても使える。`add` / `remove` は動いている `swing up` のダッシュボード API を経由する（`POST /api/mirror/add` / `/api/mirror/remove`、body は `{"keys": [...]}`）。実際の Follow Set 操作は agent が保持する relay 接続（`dashboard::AppState`）で行われ、CLI プロセス自身は relay につながない。`remove` で Follow Set がそもそも見つからない場合は `(no follow set found); no changes` とだけ表示して終わる（`add` に同じ制限は無い。無い状態からの新規作成を許すため）。API に接続できない場合の挙動は上の共通節を参照。

## sites

- Follow Set の対象者ごとに、サイトごとの最新のサイトイベントを 1 行（`d`、`cid`、`url`、`size`、`created_at`、NIP-05 検証結果、`replicas`、`[stored]` / `[not stored]`）表示する。`title` タグが有効なら次の行に `    title: ` として、`content` が空でなければ続けて `    message: ` として、それぞれ制御文字を空白に置き換え、前後の空白を削り、200 文字を超える分を `…` に置き換えて表示する。検証結果と保存状況は `state.json` から読む。`replicas` は [replicas](#replicas) と同じ集計の最新版のレプリカ数で、`replicas::format_replica_counts` により `3` または `3 (+12 unverified)`（未検証の報告者がいるとき）の形になる（[「レプリカ報告の信頼度」](../architecture.md#レプリカ報告の信頼度replicastier)）。信頼度の判定には Follow Set の対象全員分をまとめて 1 回だけ取得する。レプリカ報告の取得に失敗したら `(fetching replica reports failed: ...)` を表示して `-` にする。
  - `size` 列: そのイベントの `cid` と一致する `VersionRecord`（保存時に `dag/stat` で測って `state.json` に記録した値。改めて Kubo は呼ばない）があればその値をそのまま数値で出す。無ければイベントの自己申告の `size` タグを括弧書き（例 `(12345)`）で出す。どちらも無ければ `-`。括弧書きは `[not stored]` と対で「申告のみで未確認」を表す（ラベルは付けない）。
- 続けて、state に版があるのに Follow Set にいない pubkey を `Unfollowed but still stored` 見出しの下に `[unfollowed]` 付きで、サイトごとに state の最新版を 1 行（`url` は `-`）表示する。見出しには `remove_on_unfollow` に応じて、次の poll で消えるか残しているかを添える。Follow Set が見つからなくても表示し、そのとき `remove_on_unfollow = true` なら「Follow Set が見つかるまで agent は消さない（鍵か `mirror_set` を変えたなら、戻せば残り、新しいセットに誰かを足せば消える）」という見出しにする（[agent の unfollow](agent.md#unfollow)）。
- 表示する対象は [取得と表示の上限](../architecture.md#取得と表示の上限nostrbudget) の対象になる: Follow Set の `p` は先頭 500 件まで（超えたら warn を出す）、1 作者あたりの `d` は `d` の昇順で先頭 50 件まで。

## replicas

state は読まない。

- `<key>` を作者として扱う。省略時は自分の pubkey。
- 作者ごとに、サイトごとの最新のサイトイベントについて `d`、`cid`、`replicas=<trusted な報告者数>（`format_replica_counts` により未検証がいれば ` (+N unverified)` を添える） (reports=<有効な報告の数>)` を表示し、続けて報告者ごとに npub と `[latest]` / `[older version]` を 1 行ずつ表示する。並びは（信頼度の tier、最新版を持つか、hex）の順。
- 報告者ごとに信頼度の tier を `[author]`（報告者が作者自身） / `[chosen]`（報告者が作者の Follow Set かこちらの Follow Set のいずれかに入っている） / `[unverified]`（それ以外。自称にすぎない）のいずれかで添える（[「レプリカ報告の信頼度」](../architecture.md#レプリカ報告の信頼度replicastier)）。
- 集計（`replicas::collect_reports`）: サイトイベントの座標（`35980:<作者>:<d>`）を `#a` に入れて `replica_event_kind` の報告を取得し、報告者・`d` ごとに最新の 1 件だけを残す。パースに失敗したもの（検証は下の [Nostr イベントの検証](../architecture.md#nostr-イベントの検証nostrrs)）、`cid` タグが無いものは数えない。残りは `ReplicaReport::counts_at(now)` が true のものだけを数える: `created_at` が `now + 900` 秒以内、`now - created_at` が 7 日（`nostr::MAX_REPORT_AGE`）以内、かつ `expiration` が無いか `now` より先。
- サイトごとの報告は（信頼度の tier、`created_at` の新しい順）で先頭 200 件（`nostr::budget::MAX_REPORTS_PER_SITE`）までに切り詰める。`reports=` は切り詰め後の件数で、切り詰めがあれば報告者一覧の後に `… and N more report(s) not shown` を出す（[取得と表示の上限](../architecture.md#取得と表示の上限nostrbudget)）。作者ごとに表示する `d` も先頭 50 件までに切り詰める。
- サイトイベント・報告・Follow Set のどれかの取得に失敗したらエラーで終了する。

## status

relay には接続せず、動いている `swing up` のダッシュボード API（`GET /api/status`）を叩く。API 側（`health::collect_status`）が `state.json` と Kubo だけを見て組み立てた結果を DTO（`dashboard::dto::StatusDto`）として返し、CLI（`src/health.rs::print_status_dto`）はそれをそのまま印字する。`swing up` が動いていない、または agent 未準備（Kubo の URL が未確定）なら失敗する（上の共通節を参照）。

- `state.json` の版ごとに、版のパス・`cid`・`size`（state に記録された版ごとのサイズ）と判定を 1 行表示する。判定は [起動時の突き合わせ](agent.md#起動時の突き合わせ) と同じ検査で、`[ok]` / `[missing]`（パスが無い）/ `[cid mismatch]` / `[incomplete]`（ブロックが欠けている）/ `[check failed]`（`files/stat` 自体が失敗）のいずれか。`ok` 以外は理由を添える。
- 続けて `Actual size` 見出しの下に、サイトごとの実容量（そのサイトの全版をまとめた `dag/stat` の `TotalSize`。版どうしで共有しているブロックは 1 回だけ数える）と合計を表示する。測れなかったサイトは `unknown` にし、合計も `unknown` にする。
- 続けて `Not in state` 見出しの下に、[sweep](agent.md#sweep) が消す MFS のパスを表示する。ディレクトリごと消えるものはそのディレクトリだけを出す。一覧に失敗したディレクトリは `[list failed]: <理由>` 付きで出す（理由は DTO の `GarbageDto::list_failed_reason`）。
- `ok` 以外の版と `Not in state` の項目が 1 つでもあれば、件数を表示して 0 以外で終了する。
- agent の実行中は、保存途中の版（MFS に置いた後、state を保存する前）が `Not in state` に出ることがある。
- サイト単位で DAG をたどるので、保存量に比例して時間がかかる。API 呼び出しはダッシュボードのリクエストタイムアウト（120 秒、`src/dashboard/mod.rs::REQUEST_TIMEOUT`）と `ApiClient` 側のタイムアウト（125 秒）の範囲で待つ。

## webring

state は読まない。

- `<key>` を起点にする。省略時は自分の pubkey。`--depth` の既定は 2、`--format` の既定は `text`。
- たどり方（`webring::crawl`）: 起点を深さ 0 とし、深さ `d` のアカウントについて Follow Set を取得する。`d < depth` なら、その `p` のアカウント（アカウント自身が実際にフォローしている相手）を、まだ見ていなければ深さ `d + 1` にする。深さ `depth` のアカウントも Follow Set は取得するが、先へは広げない。`#p`（自分を名指ししているだけの相手。フォローし返しているとは限らない自称）は深さ 0（起点）についてだけ 1 回取得し、クロールを広げるのには使わない（[「レプリカ報告の信頼度」](../architecture.md#レプリカ報告の信頼度replicastier)）。
- グラフ（`webring::build_graph`）: 取得した Follow Set の `p` のうち、見つけたアカウント（`#p` で見つかっただけの、フォロー先ではない相手を除く）を指すものを辺（A → B は A の Follow Set に B がいる）にする。自分自身への辺は捨てる。辺を向きを無視してたどり、起点につながらないアカウントは除く。
- 残ったアカウントのサイトイベントを取得し、サイトごとの最新版の `d` をアカウントの名前にする。
- `text`: 見出しに件数、`Accounts` にアカウントごとの名前（`d` を `, ` でつないだもの。無ければ縮めた npub。同じ名前が複数あれば縮めた npub を添える）・npub・深さ・`[root]` / `[no follow set]`、`Mutual` に双方向の組、`One-way` に片方向の辺を出す。並びは（深さ、名前）の順。続けて `Referencing the root (unverified)` に、起点を名指ししているだけでクロールには加えなかったアカウント（`#p` で見つかったもの）を npub で先頭 50 件（`nostr::budget::MAX_REFERENCING_LISTED`）まで、超えた分は `… and N more` として出す（1 件も無ければ `(none)`）。深さの上限の外にいて表示しなかった、残ったアカウントの Follow Set に載っているアカウントがあれば、その数を最後に出す。
- `dot`: Graphviz の `digraph`。ノード ID は hex、ラベルは名前と縮めた npub。起点は `penwidth=2`、双方向の組は `dir=both` の 1 本にする。`referencing` は含めない（グラフだけを描く）。
- `mermaid`: `graph LR`。ノード ID は `n<番号>`（hex 順）、ラベルは名前と縮めた npub で、`#` `&` `"` `<` `>` はエンティティにする。起点は `root` クラス、双方向の組は `<-->` にする。`referencing` は含めない。
- Follow Set・サイトイベントのどれかの取得に失敗したらエラーで終了する。
- たどるアカウントの総数は `nostr::budget::MAX_CRAWL_NODES`（1000）を超えない。超えて見つかったアカウントは crawl に加えず件数だけ数え、`text` の末尾に `(crawl stopped at the 1000-account budget; not reached: N)` として出す（深さの上限外で表示していない `beyond` とは別のカウンタ）。アカウントの名前に使う `d` も 1 アカウントあたり先頭 50 件までに切り詰める。

## key generate

新しい鍵ペアの nsec / npub / hex（秘密鍵・公開鍵）を表示する。設定を読まない。

## config example / config env-example

設定ファイルを読まない（`--config` を取らない）。`src/settings.rs::SETTINGS` の設定カタログから、リポジトリ直下の `swing.example.toml`／`.env.example` と同じ内容を標準出力に印字する（[`../architecture.md#設定と環境変数`](../architecture.md#設定と環境変数)）。両ファイルはこの出力と一致することを `cargo test` が確認する（一致しなければどちらのコマンドで再生成すべきかをテストの失敗メッセージが示す）。設定キーを追加・変更したら、このコマンドの出力をそのファイルに書き直してコミットする:

```bash
swing config example     > swing.example.toml
swing config env-example > .env.example
```
