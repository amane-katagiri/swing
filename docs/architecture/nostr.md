# Nostr イベントの検証と上限（nostr.rs）

[`../architecture.md`](../architecture.md) の一部。形式と MUST/SHOULD は [`../protocol.md`](../protocol.md)、kind と `d` の予約は [`../extensions.md`](../extensions.md)。

## 検証

この実装の判定:

- `d`: 空、253 バイト超、制御文字を含む場合はイベント全体を拒否する。
- `cid`: 規則・正規化・以後の扱いは次の 3 点。
  - 規則: `cid` クレートでパースできない、またはコーデックが dag-pb（`0x70`）でなければイベント全体を拒否する（理由は [`../protocol.md`](../protocol.md)）。これはパース時の構文チェックで、取得した root が UnixFS のディレクトリであることは別に確かめる（[`agent.md` の「保存の順序」5](agent.md#保存の順序)）。
  - 正規化: 通った値を `nostr::canonical_cid` で CIDv1・base32 に変換する。レプリカ報告の `cid` タグ（下記）と、`swing publish` が Kubo から受け取った CID（`publish::add_and_measure`）も同じ関数を通す。
  - 以後: `SiteEvent::cid`・`ReplicaReport::cids`・`policy::decide`・`replicas_of`・`state.json` は正規形の文字列だけを扱う。
- `url`: 2048 バイト超、制御文字を含む、または http(s) としてパースできなければ `url` だけを無視する。`swing publish --url` も同じ判定で拒否する。
- `title`: 空、256 バイト超、制御文字を含む場合は `title` だけを無視する。保存の判断には使わない。
- `content`: 空でなければ `SiteEvent::message` に入れる。検証せず、保存の判断にも使わない。
- Follow Set: relay のフィルタに加え、受信後にも kind・`d`・署名を確かめる（`nostr::is_follow_set_of`）。`content`（暗号化 private 部分）は読まない。
- フィルタとの照合: nostr-sdk は受信したイベントが REQ のフィルタに一致するかを確かめない（`verify_subscriptions` が既定で無効）ので、relay がフィルタを無視して他人のイベントを返してきても、`RelayClient` の取得関数が受信後にフィルタの条件で照合して捨てる。
  - `fetch_follow_set`: 作者が自分の公開鍵であること。
  - `fetch_follow_sets`: 作者が要求した `authors` に含まれること。残ったものを作者ごとに最新 1 件にする。
  - `fetch_site_events`: kind が `[nostr].site_event_kind` で、作者が要求した `authors` に含まれること。
  - `fetch_replica_reports`: kind が `[nostr].replica_event_kind` で、`a` タグのどれかが要求した座標のいずれかと一致すること。
  - `fetch_follow_set_authors_referencing`: 作者を指定しない取得なので、`p` タグのどれかが要求した相手に含まれること。
  - `fetch_own_reports`（agent のレプリカ報告の同期）: 呼び出し側（`agent::replicas`）が作者が自分であることを確かめ、kind は `parse_replica_report` が確かめる。
  - `fetch_reports_about`（agent が他の報告者からの報告の時刻を記録する）: 呼び出し側（`agent::replicas`）が `p` タグに自分があること・報告者が自分でないことを確かめ、kind と作者は `parse_replica_report` の結果で確かめる。
  - 購読（`subscribe_site_events`）で届くサイトイベントは、agent の `submit` が Follow Set の対象かを確かめて捨てる（[`agent.md` の「並行処理」](agent.md#並行処理)）。
- レプリカ報告: `d` を最初の `:` で分け、作者が小文字 hex の公開鍵でない、サイトの `d` が上の `d` の条件を満たさない、`a` の値が `<site_event_kind>:<作者>:<サイトの d>` と一致しない、`cid` タグのどれかが上の `cid` の判定を満たさない、`expiration` タグがあるのに `u64` としてパースできない、のいずれかなら報告全体を拒否する。`cid` タグは 0 個でもよい（取り下げ）。`expiration` が無ければ `None` として読み、期限切れかどうかの判定は使う側（`ReplicaReport::counts_at`）が行う。
- 署名は nostr-sdk が受信時に検証する。

## 未来ずれの許容（`nostr::MAX_FUTURE_SKEW`）

`created_at` の未来ずれ許容は `nostr::MAX_FUTURE_SKEW`（900 秒）。`nostr::plausible_at(created_at, now)` がこれを超えるかどうかを判定する（`nostr.rs` にあり、`policy.rs` はここから読む）。保存の可否（`policy::decide`）だけでなく、「現在の版」やその時点で有効な Follow Set をどれとして選ぶかにも同じ基準を使う（`select_latest`・`choose_follow_set`・`newest_by_address`・`fetch_follow_set(s)` など。Follow Set の選び方は [`agent.md`](agent.md#follow-set-の選び方)）。

許容内の版どうしの新しさは次のように比べる。

- Follow Set は NIP-01 の置き換え可能イベントの規則（`created_at` が大きい方、同じなら `id` が小さい方）。
- サイトごとの最新のサイトイベント（`select_latest`）は `created_at` が大きい方。同じ `created_at` の版どうしを `id` で決着させる処理は無く、先に見た方が残る。

## 取得と表示の上限（`nostr::budget`）

サイトイベント・レプリカ報告（既定の kind 35980・35981。`[nostr].site_event_kind`・`replica_event_kind` で変えられる）と Follow Set（kind 30000）は誰でも捨て鍵で出せるので、relay から読む経路は `nostr::budget` の定数で件数を打ち切る。どの定数がどの経路に効くかは下の表のとおりで、表示用の経路（`swing sites` / `replicas` / `webring`、ダッシュボードの `/api/sites` / `/api/replicas` / `/api/webring`）だけでなく agent の取り込みと `/api/publish/sites` にも一部が効く。relay からの取得（`fetch_events`）は 30 秒でタイムアウトする。

| 定数 | 値 | 適用箇所 |
|---|---|---|
| `MAX_FOLLOW_SET_ENTRIES` | 500 | `nostr::follow_set_pubkeys_capped`。tag 順で最初の 500 件の重複しない `p` を残す。自分の Follow Set も対象で、`agent::follow::resubscribe_and_backfill` と `mirror::collect_sites` は切り詰めたら warn を 1 回出す。`swing mirror add`（`POST /api/mirror/add`）は上限を超える追加を 409 のエラーにし、切り詰めない |
| `MAX_SITES_PER_AUTHOR_LISTED` | 50 | `nostr::cap_sites_per_author`（`select_latest` の直後に呼ぶ）。作者ごとに `d` の昇順で先頭 50 件だけを残す。`mirror::collect_sites`・`replicas::collect`・`webring::collect`・`/api/publish/sites`（`dashboard::api::publish_sites`）で使う。agent の取り込み側の件数制限（`agent::follow::limit_sites_per_account`、ポリシー値 `max_sites_per_account`）とは別物で、この表の上限とは独立に効く |
| `MAX_REPORTS_PER_SITE` | 200 | `replicas::collect_reports`。数える報告を [「レプリカ報告の信頼度」](#レプリカ報告の信頼度replicastier) の順に並べ替えて、サイトごとに先頭 200 件を残す。`SiteReplicas.reports` は残した件数、`SiteReplicas.dropped` は切り捨てた件数（`swing replicas`・`/api/replicas`・ダッシュボードはここから「…and N more」を出す） |
| `MAX_CRAWL_NODES` | 1000 | `webring::crawl`。`Crawl.depths` がこれを超えないように新規ノードの追加を止め、弾いた件数を `Crawl.over_budget`（テキスト出力・`/api/webring` の `over_budget`）に積む。追加しなかったノードは次のレベルの取得にも現れない |
| `MAX_REFERENCING_LISTED` | 50 | `webring::crawl`。`#p` で見つかる「起点を名指ししているだけの相手」（`Crawl.referencing`）の一覧を先頭 50 件までに切り詰める。超えた件数は `Crawl.referencing_dropped` に積む。たどり方は [`cli.md#webring`](cli.md#webring) |
| `MAX_RELAY_FETCH_LIMIT` | 20,000 | `nostr::capped_limit` の上限値。個々の `limit()` 計算がどれだけ大きくなっても、relay 1 台への 1 回の REQ に付ける `limit` はこれを超えない |

表の外に、数える報告の古さの上限 `nostr::MAX_REPORT_AGE`（7 日。`budget` モジュールではなく `nostr` 直下）がある。`ReplicaReport::counts_at` が使い、`created_at` からこれを超えて古い報告は `expiration` に関わらず数えない。自分が出す報告の有効期間 `[agent].report_ttl` の上限でもある（[`../architecture.md`](../architecture.md#設定と環境変数)の検証）。

relay への `Filter::limit` は `nostr::capped_limit(count, per)`（= `min(count * per, MAX_RELAY_FETCH_LIMIT)`）で、取得先の件数（作者数・サイト数など）に経路ごとの倍率を掛けて決める。

`limit` は relay ごとに付くので、合計の取得件数は relay 数倍になり得る。`fetch_replica_reports` は複数サイトの座標を 1 つのフィルタにまとめるため、1 サイトの報告が多いと同じ問い合わせの他のサイトの報告が押し出されることがある（ダッシュボードは `key`/`root` を 1 リクエストあたり 100 件までに絞る）。

## レプリカ報告の信頼度（`replicas::Tier`）

レプリカ報告も Follow Set も捨て鍵で誰でも出せるため、`replicas::Tier` は作者本人（`Author`）と、作者かオペレータの Follow Set に明示的に選ばれている報告者（`Chosen`）だけを信用し、それ以外は自称にすぎない `Other` として扱う（`replicas::tier_of` が判定する）。

- `replicas::Chosen`: 作者ごとの Follow Set とオペレータ自身の Follow Set をまとめて持つ。作者ごとの Follow Set は、どちらの作り方でも 1 回の `fetch_follow_sets(mirror_set, authors)` でまとめて取る。
  - `replicas::fetch_chosen`（`replicas::collect`。`swing replicas` / `/api/replicas`）: オペレータの Follow Set を relay から取得する。`authors` は呼び出し元が渡したもの（ダッシュボード API では最大 100 件（`MAX_KEYS`）、CLI の `swing replicas` には上限なし）。
  - `replicas::fetch_chosen_with_own`（`mirror::collect_sites`）: 取得済みの Follow Set の対象（`targets`。`MAX_FOLLOW_SET_ENTRIES` 件まで）をそのまま作者とオペレータの集合に使い、オペレータの Follow Set は取り直さない。
- 集計（`replicas::fetch_for_sites` が取得し `replicas::collect_reports` が数える）: サイトイベントの座標（`<[nostr].site_event_kind>:<作者>:<d>`）を `#a` に入れて報告を取得し、報告者・`d` ごとに最新の 1 件だけを残す（`newest_by_address`。未来ずれの許容を超えるものは先に捨てる）。パースに失敗したもの（[検証](#検証)）と `cid` タグが無いものは数えない。残りは `ReplicaReport::counts_at(now)` が true のもの（`created_at` が未来ずれの許容以内、`now - created_at` が `MAX_REPORT_AGE` 以内、`expiration` が無いか `now` より先）だけを数える。
- `collect_reports` は、`MAX_REPORTS_PER_SITE` で切り詰める前に、サイトごとの報告を `(tier, created_at 降順, reporter の hex)` の順に並べ替える。tier が高い（`Author` → `Chosen` → `Other`）報告者ほど、`created_at` が古くても切り詰めで残る。
- 表示用の報告者一覧（`replicas::replicas_of`。`swing replicas` と `/api/replicas` の `reporters`）は `(tier, 最新版を持つものが先, reporter の hex)` の順。
- カウント（`replicas::count_replicas`）: 現在の版の CID を持つ報告のうち、tier が `Author`・`Chosen` のものが `SiteReplicas.replicas`（CLI・DTO では単に `replicas`）、tier が `Other` のものが `SiteReplicas.unverified`。表示は `replicas::format_replica_counts`（`"3"` / `"3 (+12 unverified)"`。`unverified` が 0 なら括弧を出さない）。

webring の `#p` で見つかる「起点を名指ししているだけの相手」（`referencing`）も自称にすぎないので、クロールにも Mutual の判定にも使わず別枠で出す（たどり方は [`cli.md#webring`](cli.md#webring)）。
