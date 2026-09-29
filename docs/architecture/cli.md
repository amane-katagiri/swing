# CLI（main.rs と各サブコマンド）

サブコマンドの一覧は [`../architecture.md#cli`](../architecture.md#cli)、設定ファイルの探し方は [`../architecture.md#設定と環境変数`](../architecture.md#設定と環境変数)、出力例は README を参照。

## 共通

- `<key>` は npub / hex / nprofile を受け付ける。
- 「Follow Set」は kind 30000、`d = mirror_set` の作者ごとに最新のもの。「サイトごとの最新のサイトイベント」は sites・replicas・webring で `nostr::select_latest` が選ぶもの。どちらも新しさの比べ方は [`nostr.md`](nostr.md#未来ずれの許容nostrmax_future_skew)。
- 自分でファイルを書くのは `up`・`publish`・`service install`/`uninstall`・`signer pair`・`dashboard rotate-token` だけ。`publish` は MFS、`service install`/`uninstall` は OS のサービス登録と、Windows・macOS では `swing-tray` のログイン時の自動起動の登録（[`service.md`](service.md#タスクトレイの自動起動windows-と-macos)）、`signer pair` は `<state_dir>/remote-signer.json`、`dashboard rotate-token` は `swing up` が動いていなければ `<state_dir>/dashboard.token`（[dashboard open / rotate-token](#dashboard-open--rotate-token)）を書く。`mirror add` / `remove` は Follow Set を relay に送り、受理されると動いている agent に即時の poll（sweep と state の保存を含む。[`dashboard.md#概要`](dashboard.md#概要)）を行わせる。`stop` と `service start`/`stop` は動いているプロセスやサービスを起動・停止させるだけ。ほかは読み取り専用。
- `status`・`stats`・`mirror add`・`mirror remove`・`stop`・`dashboard open`・`dashboard rotate-token`（と Windows の `service stop`。[service](#service-install--uninstall--start--stop--status)）は relay/Kubo に直接つながず、動いている `swing up` のダッシュボード API（`[dashboard].listen`、既定 `http://127.0.0.1:8082`）を `src/api_client.rs::ApiClient` 経由で叩く。`<[agent].state_dir>/dashboard.token` を読んで `Authorization: Bearer` で送る（送る前に `POST /api/identity` で相手がそのトークンを知っていることを確かめ、確かめられなければ送らずにエラー終了する。[`dashboard.md#認証srcauthrs-srcdashboardsessionrs`](dashboard.md#認証srcauthrs-srcdashboardsessionrs)）ので、`swing up` と同じ設定（同じ `state_dir`）を読めて、そのファイルを読めるユーザーで実行する必要がある。API が `[dashboard].listen` を未指定アドレス（`0.0.0.0` / `::`）で待ち受けていても、クライアントは接続先と `Host` ヘッダをループバックの同じポートへ正規化する。API に接続できなければ `status`・`stats`・`mirror add`・`mirror remove` は `swing up is not running (cannot connect to <addr>)` でエラー終了し（非ゼロ終了）、`stop` は `not running` を出して正常終了（終了コード 0）する。`sites`・`replicas`・`webring`・`mirror list`・`publish` はこの API を経由せず relay/Kubo に直接つなぐので、`swing up` が動いていなくても使える。
- `[nostr].secret_key`（`SWING_NOSTR_SECRET_KEY`）は必須ではない。`signer::Signer::require` を呼ぶコマンド（`sites`・`replicas`・`webring`・`mirror list`・`publish`）は秘密鍵も `<state_dir>/remote-signer.json`（NIP-46 の署名アプリ。[`signer.md`](signer.md)）も無ければエラー終了するが、`status`・`stats`・`mirror add`・`mirror remove`・`stop`・`dashboard open`・`dashboard rotate-token` はダッシュボード API 経由で鍵を直接使わないので鍵が無くても動く。ただし鍵が無い `swing up` はセットアップモード（[up](#up)）で動いており、そこでは `status`・`mirror add`・`mirror remove` は 503 `agent is not configured` を返す。

## up

`[kubo].managed` に応じて Kubo（子プロセス）と mirror-agent の中身を 1 プロセスの supervisor として動かす。mirror-agent はこの `up` だけが起動でき、単体で動かすサブコマンドは無い。Kubo・agent いずれかが落ちても自動で再起動する（[`up.md`](up.md)）。`--log-file <path>` を指定すると、標準エラーの代わりにそのファイルへ追記でログを出す。

処理を始める前に `<[agent].state_dir>/swing.lock` のインスタンスロックを取り、同じ `state_dir` で既に動いていればエラーで終了する（[`up.md#多重起動の防止lockrs`](up.md#多重起動の防止lockrs)）。

鍵も署名アプリの接続情報も無ければ、ダッシュボードだけを動かすセットアップモードになる（[`up.md#セットアップモード鍵未設定`](up.md#セットアップモード鍵未設定)）。セットアップモードではダッシュボードと Kubo の gateway のポートが使われていればずらして設定ファイルに書き込む。`--no-port-shift`（環境変数 `SWING_NO_PORT_SHIFT`。Docker イメージでは既定で有効）を付けるとずらさず、使われていればこれまでどおりエラーになる（[`up.md#セットアップモードでのポートの調整`](up.md#セットアップモードでのポートの調整)）。

## stop

```
swing stop [--config <path>] [--restart] [--timeout <secs>, 既定 60（service::GRACEFUL_STOP_TIMEOUT）]
```

動いている `swing up` インスタンスに正常終了（グレースフルシャットダウン）、または `--restart` でプロセス内再起動を要求する（[`up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)）。`--timeout` は下記のポーリングの上限で、超えたらエラー終了する。

実装（`src/stop.rs`）はダッシュボード API だけを使う。

1. `ApiClient::identity`（[`POST /api/identity`](dashboard/http-api.md#post-apiidentity)。トークンは送らず、相手の `proof` を確かめて `instance` を読む）で今の `instance` を読んでおく。接続できなければ `not running` で正常終了し、相手がトークンを知っていることを確かめられなければエラー終了する。
2. `POST /api/shutdown`（`--restart` なら `/api/restart`）を叩く。API に接続できなければ（`swing up` 自体が動いていない）`not running` を出して正常終了する。
3. 呼び出しが通れば `ApiClient::identity` を 500ms 間隔でポーリングする（トークンを送らないので、止まった後にポートを取った相手にも渡らない）。`--restart` なしなら、接続できなくなった時点（プロセスが終了した時点）で `stopped` を出して正常終了する。`--restart` なら、確かめられた応答の `instance` が 1. で読んだ値と変わった時点（同じプロセスの中で `up::run` がやり直された時点）で `restarted` を出して正常終了する。確かめられない応答は、どちらでも待ち続ける理由として扱う（`--timeout` でエラー終了する）。

Windows の `swing service stop` もこの `stop::run` を使う（失敗したときの扱いは [`service.md`](service.md#windowsタスクスケジューラ)）。

## dashboard open / rotate-token

実装は `src/login.rs`（URL の組み立ては `login::request_link`、ブラウザで開くのは `login::open_browser`。どちらも `swing-tray` と共通。[`tray.md`](tray.md)）。認証の仕組みは [`dashboard.md#認証srcauthrs-srcdashboardsessionrs`](dashboard.md#認証srcauthrs-srcdashboardsessionrs)。

- `dashboard open [--config] [--no-browser]`: `POST /api/login-code` で使い捨てのログインコードをもらい、`<[dashboard].public_url>/login?code=<code>`（`public_url` 未設定時の URL の決め方は [`dashboard.md#設定dashboard`](dashboard.md#設定dashboard)）とコード（`login code (single use, valid for 5 minutes): ...`）を標準出力に出す。`--no-browser` が無ければ続けて OS の既定ブラウザで URL を開く（Linux は `xdg-open`、macOS は `open`、Windows は `rundll32 url.dll,FileProtocolHandler`）。開けなければ標準エラーに案内を出すだけで正常終了する。`[dashboard].ui = false` ならエラー終了する。`swing up` が動いていなければ `swing up is not running (...)` でエラー終了する。
- `dashboard rotate-token [--config]`: `POST /api/token/rotate` でトークンを作り直す（ブラウザのセッションはすべて無効になる）。`swing up` が動いていなければ `<state_dir>/dashboard.token` を直接書き換える。

## service install / uninstall / start / stop / status

`swing up` を OS のログイン/システムサービスとして登録する（systemd user unit・launchd LaunchAgent・Windows タスクスケジューラ）。`install` は `--config`（省略時は `SWING_CONFIG`、それも無ければ `./swing.toml`。解決したパスにファイルが無ければエラー）・`--system`（Linux のみ）・`--run-as <user>`（`--system` と一緒にだけ使える。system unit を動かすユーザー）・`--no-start`（登録だけで起動しない）・`--no-tray`（Windows と macOS で、`swing-tray` をログイン時に起動する登録をしない）を取る。`start`/`stop`/`status`/`uninstall` は `--system` のみ。`start` は登録済みのサービスを、`stop` は登録を残したままプロセスだけを、`uninstall` は止めてから登録を、サービス機構経由で操作する。上の `swing stop` とは別で、こちらはサービス機構を通す（Windows の `service stop` だけは [stop](#stop) の例外）。OS ごとの実体は [`service.md`](service.md) を参照。

## publish

`DIR` を Kubo に add して MFS に置き、サイトイベントに署名して全 relay に送り、同じサイトの古い版を MFS から消す。

- `--site` は必須で、そのまま `d` になる。[`d` の条件](nostr.md#検証)を満たさなければ `invalid --site` でエラー終了する。
- `--url` は任意。指定すると `url` タグになり、[`url` の条件](nostr.md#検証)を満たさなければ `invalid --url: ...` でエラー終了する。省略すると `url` タグを付けない（IPFS だけで公開するサイト）。
- `--nip05` 省略時は `[publish].nip05`。
- `--check-dotfiles`・`--check-size`・`--check-unchanged` はサイトの確認のモード（`off`/`warn`/`require`）。省略時はそれぞれ `[publish].check_dotfiles`（既定 `require`）・`check_size`（既定 `warn`）・`check_unchanged`（既定 `require`）。`--nip05` を含めた 4 つのモードは表示や処理の前にまとめて解釈し（`publish::resolve_modes`）、不正な値は `invalid --<フラグ名>` でエラー終了する。
- `--title` は任意。指定すると `title` タグになる。空文字・空白のみは付けない扱いにする。256 バイトを超える、または制御文字か見えない書式文字（[`title` の条件](nostr.md#検証)）を含む場合は `invalid --title: ...` でエラー終了する。
- `--message` はサイトイベントの `content` になる。受け取る側の SWING は `MAX_CONTENT_BYTES`（4096 バイト）を超える更新メモを捨てる（[`content` の扱い](nostr.md#検証)）ので、`publish::validate_message` で同じ上限を確かめ、超えたら何もせずにエラー（`invalid --message: must not exceed 4096 bytes (got N bytes)`）。最初に `Site: <d>`、`--url` があれば `URL:`、`--title` があれば `Title:`、`--message` があれば `Message:` を表示する。省略時は空文字。

引数とモードを解釈した後、表示の前に、`DIR` の実体（`canonicalize`）の中に設定ファイル・`[agent].state_dir`・`[kubo].repo` のどれかの実体があれば（`DIR` そのものである場合を含む）、確認のモードにかかわらず何もせずにエラー終了する（`publish::refuse_protected_paths`。存在しないパスは見ない）。どれも Nostr の秘密鍵・ダッシュボードのトークン・Kubo の秘密鍵を持つため。逆向き（`DIR` が `state_dir` の中にある場合。ダッシュボードのアップロード先がこれ）は止めない。

処理順:

1. `--nip05` が `off` でなければ、`d` と自分の pubkey で NIP-05 を検証し、`NIP-05` 見出しの下に結果を表示する（`✓ verified` / `! mismatch: ...` / `- not applicable (d is not a domain)` / `! error: ...`）。`require` で `Verified` 以外（`NotApplicable` を含む）なら add せず終了する。
2. `--check-dotfiles` と `--check-size` のどちらかが `off` でなければ、`DIR` を add と同じ辿り方（`ipfs::list_site`。シンボリックリンクを辿り、ドットファイルも含める。リンク先が `DIR` の外ならこの時点でエラー）で一覧し、`Checks` 見出しの下に 1 行ずつ結果を表示する（`publish::LocalChecks`。`off` の項目は `- dotfiles: off` のように出す）。
   - ドットファイル: 各パスをサイトのルートから順にセグメントごとに見て、`[publish].dotfiles_allow` の名前と一致するセグメントがあればそのパスは見逃し、先に名前が `.` で始まるセグメントがあればそこまでを 1 件とする（ディレクトリは 1 回だけ数え、その下は見ない）。無ければ `✓ dotfiles: none`、あれば `! dotfiles: N found (not in [publish].dotfiles_allow)` の後に先頭 `LISTED_DOTFILES`（10）件のパスを字下げして並べ、残りは `… and N more` にまとめる。
   - サイズ: ファイルの大きさの合計（`metadata().len()` の和。ブロックの共有やディレクトリのノードは数えない）が `SIZE_GUIDELINE`（512 MiB、固定）を超えたら（ちょうどは超えない扱い）`! size: <合計> is over the 512 MiB guideline; each mirror decides by its own limits (max_update_size, default 2 GiB)`、超えなければ `✓ size: <合計> (guideline 512 MiB)`。
   - `require` の項目が引っかかったら、項目ごとの理由（ドットファイルは消す・名前を `[publish].dotfiles_allow` に足す・`--check-dotfiles` か `[publish].check_dotfiles` を `warn`/`off` にする、の案内、サイズは `--check-size` か `[publish].check_size` の案内）を `; ` でつないだメッセージで、add せずにエラー終了する。
3. 現在時刻を `created_at` に決め、`DIR` を CIDv1・pin なしで add し、`<mfs_root>/publish/<pubkey hex>/<site>/<created_at>` に置く（既存の項目は先に消す）。
4. `dag/stat`（`offline=true`）の `TotalSize` を `size` タグにする。失敗したら（ブロックが欠けていたら）エラーで終了する。`IPFS` 見出しの `Size:` はこの値。
5. relay に接続する（ここで 1 回だけつなぎ、6 と 7 で同じ接続を使う）。
6. `--check-unchanged` が `off` でなければ、relay から自分の pubkey・この `d` のサイトイベントを取り（`RelayClient::fetch_own_latest_site`。`parse_site_event` を通り `created_at` が未来すぎないものの最新 1 件）、`Previous version` 見出しの下に結果を表示する（`publish::UnchangedOutcome::decide`）。CID が違えば `✓ changed from the latest version on the relays (<前の CID>)`、同じなら `! unchanged: the CID equals your latest version on the relays`、見つからなければ `- no previous version on the relays`、取得に失敗したら `! could not check: <理由>`。同じで `require` なら、3 で置いた版を MFS から消して `✓ removed <パス>` を出し、署名・送信・古い版の削除をせずに `Unchanged; not published.` で終わる（終了コード 0。消せなければエラー終了）。見つからない・取得に失敗したときは `require` でも続ける。
7. サイトイベント（`alt` は `SWING site announcement: <d>`）を 3 の `created_at` で作って署名し、全 relay に送る。署名アプリを使っているときは、署名の前（`Nostr` 見出しの直後）に `waiting for the signer app to sign the site event...` を表示し、署名アプリの返事（承認）を最大 90 秒待つ。署名できなければエラーで終了し、古い版は消さない。relay ごとの成否（✓/✗）を表示する。どこにも受理されなければエラーで終了し、古い版は消さない。
8. `<mfs_root>/publish/<pubkey hex>/<site>/` の中で名前が整数の項目を新しい順に `[publish].keep_versions` 個残して消す（`Old versions (keeping N)` 見出し）。一覧に失敗したら警告を出して続ける。
9. `Published.` で終わる。

## mirror list / add / remove

自分の Follow Set を操作する。

- `add` / `remove` は `p` 以外のタグの値と `content`（NIP-51 の暗号化 private 部分）を保持して再署名する。タグは `d` → その他 → `p` の順に並べ直す。
- Follow Set が無い状態の `add` は `["title", "SWING mirror list"]` 付きで新規作成する。
- 追加済みの `add`、未登録の `remove` は no-op と報告し、変更が無ければ publish しない。
- 再署名する Follow Set の `created_at` は現在時刻と「元の Follow Set の `created_at` + 1」の大きい方（`mirror::set::next_created_at`）。元が未来ずれの許容内で先の時刻でも、NIP-01 の比較で新しい方になる。
- `add` は結果の `p` タグのうち公開鍵としてパースできたものの数が `MAX_FOLLOW_SET_ENTRIES`（[取得と表示の上限](nostr.md#取得と表示の上限nostrbudget)）を超えるならエラーで終了し、publish しない（ダッシュボード API は 409 を返し、CLI はその `error` の文言 `would grow the follow set to <N> entries, over the 500-entry limit; remove some first` を表示する）。
- `list` は npub と hex を併記する。`title` タグは `sites` の `title:` 行と同じ無害化（下記）をして `Title:` に表示する。
- relay の Follow Set と `state.json` の `follow_set` を比べて新しい方を使う（検証条件は [agent の Follow Set の選び方](agent.md#follow-set-の選び方) と同じ）。保存済みの方を使ったときは `(relays returned an older follow set; ...)` か `(follow set not found on relays; ...)` を表示する。state.json は読むだけ。`sites` も同じ。
- `list` は relay に直接つなぐ（`mirror::collect_mirror_list`）。`add` / `remove` は `POST /api/mirror/add` / `/api/mirror/remove`（body は `{"keys": [...]}`）を叩き、Follow Set の操作は `swing up` 側が保持する relay 接続（`dashboard::AppState`）で行う（[共通](#共通)）。`remove` で Follow Set がそもそも見つからない場合は `(no follow set found); no changes` とだけ表示して終わる。

## sites

- Follow Set の対象者ごとに、サイトごとの最新のサイトイベントを 1 行（`d`、`cid`、`url`、`size`、`created_at`、NIP-05 検証結果、`replicas`、`[stored]` / `[not stored]`）表示する。`title` タグが有効なら次の行に `    title: ` として、`content` が空でなければ続けて `    message: ` として、それぞれ制御文字を空白に置き換え、見えない書式文字（[`nostr::is_unsafe_char`](nostr.md#検証)）を取り除き、前後の空白を削り、200 文字を超える分を `…` に置き換えて表示する（`mirror::print::sanitize_display_text`）。検証結果と保存状況は `state.json` から読む。`replicas` は [replicas](#replicas) と同じ集計の最新版のレプリカ数で、`replicas::format_replica_counts` により `3` または `3 (+12 unverified)`（未検証の報告者がいるとき）の形になる（[「レプリカ報告の信頼度」](nostr.md#レプリカ報告の信頼度replicastier)）。信頼度の判定には Follow Set の対象全員分をまとめて 1 回だけ取得する。レプリカ報告の取得に失敗したら `(fetching replica reports failed: ...)` を表示して `-` にする。
  - `size` 列: 保存済みの版の実測値（`/api/sites` の `stored_size` と同じ値。[`dashboard/http-api.md#get-apisites`](dashboard/http-api.md#get-apisites)）があれば数値で、無ければイベントの自己申告の `size` タグを括弧書き（例 `(12345)`）で、どちらも無ければ `-` を出す。
- 続けて、state に版があるのに Follow Set にいない pubkey を `Unfollowed but still stored` 見出しの下に `[unfollowed]` 付きで、サイトごとに state の最新版を 1 行（`url` は `-`）表示する。Follow Set が見つからなくても表示する。見出しには `remove_on_unfollow` と Follow Set が見つかったかどうかに応じて、次の poll で消えるか・残すか・Follow Set が見つかるまで消さないかを添える（いつ消えるかは [agent の unfollow](agent.md#unfollow)）。
- 表示件数は [取得と表示の上限](nostr.md#取得と表示の上限nostrbudget)（`MAX_FOLLOW_SET_ENTRIES`・`MAX_SITES_PER_AUTHOR_LISTED`）で打ち切る。

## replicas

state は読まない。

- `<key>` を作者として扱う。省略時は自分の pubkey。
- 作者ごとに、サイトごとの最新のサイトイベントについて `d`、`cid`、`replicas=<数> (reports=<有効な報告の数>)` を表示し（`<数>` は `format_replica_counts` の形で、未検証の報告者がいれば ` (+N unverified)` が付く）、続けて報告者ごとに npub と `[latest]` / `[older version]` を 1 行ずつ表示する（並びは [「レプリカ報告の信頼度」](nostr.md#レプリカ報告の信頼度replicastier)）。
- 報告者ごとに信頼度の tier を `[author]`（報告者が作者自身） / `[chosen]`（報告者が作者の Follow Set かこちらの Follow Set のいずれかに入っている） / `[unverified]`（それ以外。自称にすぎない）のいずれかで添える（[「レプリカ報告の信頼度」](nostr.md#レプリカ報告の信頼度replicastier)）。
- 数える報告の条件と、サイトごとの報告の切り詰め（`MAX_REPORTS_PER_SITE`）は [「レプリカ報告の信頼度」](nostr.md#レプリカ報告の信頼度replicastier) と [取得と表示の上限](nostr.md#取得と表示の上限nostrbudget)。`reports=` は切り詰め後の件数で、切り詰めがあれば報告者一覧の後に `… and N more report(s) not shown` を出す。
- サイトイベント・報告・Follow Set のどれかの取得に失敗したらエラーで終了する。

## status

`GET /api/status` を叩く（[共通](#共通)）。API 側（`health::collect_status`）が `state.json` と Kubo だけを見て組み立てた結果を DTO（`dashboard::dto::StatusDto`）として返し、CLI（`src/health.rs::print_status_dto`）はそれをそのまま印字する。agent が未準備なら失敗する（[`dashboard/http-api.md#共通`](dashboard/http-api.md#共通)）。

- `state.json` の版ごとに、版のパス・`cid`・`size`（state に記録された版ごとのサイズ）と判定を 1 行表示する。判定は [起動時の突き合わせ](agent.md#起動時の突き合わせ) と同じ検査で、`[ok]` / `[missing]`（パスが無い）/ `[cid mismatch]` / `[incomplete]`（ブロックが欠けている）/ `[check failed]`（`files/stat` 自体が失敗）のいずれか。`ok` 以外は理由を添える。state のキーが `<pubkey hex>:<d>` として読めない版は検査せず、キーと `cid` に `[invalid site key]` を付けて出す（API の `health` は `invalid_key`）。
- 続けて `Actual size` 見出しの下に、サイトごとの実容量（そのサイトの全版をまとめた `dag/stat` の `TotalSize`。版どうしで共有しているブロックは 1 回だけ数える）と合計を表示する。測れなかったサイトは `unknown` にし、合計も `unknown` にする。
- 続けて `Not in state` 見出しの下に、[sweep](agent.md#sweep) が消す MFS のパスを表示する。ディレクトリごと消えるものはそのディレクトリだけを出す。一覧に失敗したディレクトリは `[list failed]: <理由>` 付きで出す（理由は DTO の `GarbageDto::list_failed_reason`）。
- `ok` 以外の版と `Not in state` の項目が 1 つでもあれば、件数を表示して 0 以外で終了する。
- agent の実行中は、保存途中の版（MFS に置いた後、state を保存する前）が `Not in state` に出ることがある。
- 実行時間は突き合わせと同じく保存量に比例する（[起動時の突き合わせ](agent.md#起動時の突き合わせ)）。API 呼び出しはダッシュボードのリクエストタイムアウト（[`dashboard.md#タイムアウトsrcdashboardmodrs`](dashboard.md#タイムアウトsrcdashboardmodrs)）と `ApiClient` 側のタイムアウト（125 秒）の範囲で待つ。

## stats

```
swing stats [--last <duration>, 既定 1h] [--json] [--config <path>]
```

`swing up` が測って持っているリソース使用量（[`stats.md`](stats.md)）を `GET /api/stats?since=<今 − last>` で取り（[共通](#共通)）、`src/stats.rs::render` で表にする。測るのは `swing up` 側で、このコマンドは読むだけ。`--last` は `30m`・`6h`・`1d` のような長さ（単位なしは秒）で、`swing up` が持つのは 24 時間分まで。セットアップモードでも使える。

- 1 行目にサンプル数・期間・間隔・最新のサンプルが何秒前か。サンプルが無ければ `No samples in the last <期間>; swing up takes one every 1m.` だけを出す。
- 表は `swing CPU`・`swing memory`・`Kubo CPU`・`Kubo memory`・`IPFS in`・`IPFS out` の 6 行で、列は `now`（最新のサンプルの値）・`avg`・`max`（期間内の値のあるサンプルでの平均と最大）。値が無ければ `-`。バイト数は 1024 基数で小数 1 桁に丸める（`format::format_bytes_approx`）。
- 表の下に、最新のサンプルに通信量があれば Kubo 起動からの累計（`IPFS total since Kubo started: in …, out …`）、`kubo_managed` が `false` なら Kubo が SWING の管理外で CPU・メモリを取得できない旨を出す。
- `--json` を付けると API の応答（[`dashboard/http-api.md#get-apistats`](dashboard/http-api.md#get-apistats)）をそのまま整形して出す。

## webring

state は読まない。

- `<key>` を起点にする。複数指定でき、すべて深さ 0 の起点になる。省略時は自分の pubkey。`--depth` の既定は 2、`--format` の既定は `text`。
- たどり方（`webring::crawl`）: 起点を深さ 0 とし、深さ `d` のアカウントについて Follow Set を取得する。`d < depth` なら、その `p` のアカウント（アカウント自身が実際にフォローしている相手）を、まだ見ていなければ深さ `d + 1` にする。深さ `depth` のアカウントも Follow Set は取得するが、先へは広げない。`#p`（自分を名指ししているだけの相手。フォローし返しているとは限らない自称）は深さ 0（起点）についてだけ 1 回取得し、クロールを広げるのには使わない（[「レプリカ報告の信頼度」](nostr.md#レプリカ報告の信頼度replicastier)）。
- グラフ（`webring::build_graph`）: 取得した Follow Set の `p` のうち、見つけたアカウント（`#p` で見つかっただけの、フォロー先ではない相手を除く）を指すものを辺（A → B は A の Follow Set に B がいる）にする。自分自身への辺は捨てる。辺を向きを無視してたどり、起点につながらないアカウントは除く。
- 残ったアカウントのサイトイベントを取得し、サイトごとの最新版の `d` をアカウントの名前にする。
- `text`: 見出しに件数、`Accounts` にアカウントごとの名前（`d` を `, ` でつないだもの。無ければ縮めた npub。同じ名前が複数あれば縮めた npub を添える）・npub・深さ・`[root]` / `[no follow set]`、`Mutual` に双方向の組、`One-way` に片方向の辺を出す。並びは（深さ、名前）の順。続けて `Referencing the root (unverified)` に、起点を名指ししているだけでクロールには加えなかったアカウント（`#p` で見つかったもの）を npub で先頭 `nostr::budget::MAX_REFERENCING_LISTED` 件まで、超えた分は `… and N more` として出す（1 件も無ければ `(none)`）。表示したアカウントの Follow Set に載っているのに crawl に加えなかったアカウントがあれば、その数（`beyond`）を `(accounts beyond depth N, not shown: N)` として出す。
- `dot`: Graphviz の `digraph`。ノード ID は hex、ラベルは名前と縮めた npub。起点は `penwidth=2`、双方向の組は `dir=both` の 1 本にする。`referencing` は含めない（グラフだけを描く）。
- `mermaid`: `graph LR`。ラベルは名前と縮めた npub。起点は `root` クラス、双方向の組は `<-->` にする。`referencing` は含めない。
- Follow Set・サイトイベントのどれかの取得に失敗したらエラーで終了する。
- たどるアカウントの総数は `nostr::budget::MAX_CRAWL_NODES` を超えない。超えて見つかったアカウントは crawl に加えず件数（`over_budget`）だけ数え、`text` の末尾に `(crawl stopped at the N-account budget; not reached: N)` として出す。`beyond` は深さの上限の外にいるものと、この上限で弾いたもののどちらも数えるので、`over_budget` と重なることがある。アカウントの名前に使う `d` も 1 アカウントあたり先頭 `MAX_SITES_PER_AUTHOR_LISTED` 件までに切り詰める。

## signer pair

NIP-46 の署名アプリと、ターミナルに出した QR コードでペアリングし、`<state_dir>/remote-signer.json` に保存する（実装は `src/pair.rs`、ペアリングそのものはダッシュボードと同じ仕組み。[`signer.md#ペアリングpairing`](signer.md#ペアリングpairing)）。`--config` と `--relay <URL>`（署名アプリとのやりとりに使う relay。繰り返し指定で最大 5 個、既定 `wss://relay.primal.net`）を取る。ダッシュボード API は使わず、動いている `swing up` があってもなくても同じように動く。

1. `[nostr].secret_key`（`SWING_NOSTR_SECRET_KEY`）が設定されていれば、QR を出す前にエラーで終了する。
2. QR コードを Unicode のブロック文字で標準出力に出し、続けて `nostrconnect://` のリンクと、10 分待つ旨の案内を出す。
3. 状態を 200ms 間隔で見る。署名アプリが接続したら（`Checking`）`connected as <npub>; asking the signer app to sign a check event...` を出す。失敗（`Failed`、10 分のタイムアウトを含む）ならエラーで終了する。
4. `remote-signer.json` が既にあるときはつなぎ直しとして扱い、そのファイルのユーザーと同じ公開鍵の署名アプリだけを受け付ける。違えば、状態が `Checking` か `Ready` になったのを見た時点で `the signer app signs as <npub>, not as this swing's <npub>; connect the same Nostr account` でエラー終了し、ファイルは変えない（確認の署名のリクエストより前に止まるとは限らない）。
5. `Ready` になったら `remote-signer.json` を書く。確認の署名が通れば `the check event was signed`、通らなければ標準エラーに警告を出す（保存はする）。最後に `paired: swing now signs as <npub> (saved to <path>)` と、動いている `swing up` には再起動するまで反映されないこと（`swing stop --restart` かサービスの再起動）を出す。`remote-signer.json` 以外の設定は書かない。

## key generate

新しい鍵ペアの nsec / npub / hex（秘密鍵・公開鍵）を表示する。設定を読まない。

## config example / config env-example

設定ファイルを読まない（`--config` を取らない）。`src/settings/mod.rs::SETTINGS` の設定カタログから、リポジトリ直下の `swing.example.toml`／`.env.example` と同じ内容を標準出力に印字する（[`../architecture.md#設定と環境変数`](../architecture.md#設定と環境変数)）。両ファイルはこの出力と一致することを `cargo test` が確認し、一致しなければ再生成に使うコマンドをテストの失敗メッセージが示す。
