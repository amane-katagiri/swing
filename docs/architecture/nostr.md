# Nostr イベントの検証とレプリカ報告の数え方（nostr/, replicas.rs）

[`../architecture.md`](../architecture.md) の一部。子ページは relay からの取得の打ち切り方と取得・表示の件数の上限の [`nostr/fetch.md`](nostr/fetch.md)。形式と MUST/SHOULD は [`../protocol.md`](../protocol.md)、kind と `d` の予約は [`../extensions.md`](../extensions.md)。

## 検証

この実装の判定:

- `d`: 空、253 バイト超、制御文字か見えない書式文字（範囲は [`../protocol.md`](../protocol.md#4-サイトイベント)。判定は `nostr::is_unsafe_char`。ダッシュボードの `web/util.js` の `stripControlChars` と `stripUnsafeUnicode` を合わせた範囲と同じ）を含む場合はイベント全体を拒否する。
- `cid`: 規則・正規化・以後の扱いは次の 3 点。
  - 規則: `cid` クレートでパースできない、またはコーデックが dag-pb（`0x70`）でなければイベント全体を拒否する（理由は [`../protocol.md`](../protocol.md)）。これはパース時の構文チェックで、取得した root が UnixFS のディレクトリであることは別に確かめる（[`agent.md` の「保存の順序」5](agent.md#保存の順序)）。
  - 正規化: 通った値を `nostr::canonical_cid` で CIDv1・base32 に変換する。レプリカ報告の `cid` タグ（下記）と、`swing publish` が Kubo から受け取った CID（`publish::add_and_measure`）も同じ関数を通す。
  - 以後: `SiteEvent::cid`・`ReplicaReport::cids`・`policy::decide`・`replicas_of`・`state.json` は正規形の文字列だけを扱う。
- `url`: 2048 バイト超、制御文字を含む、または http(s) としてパースできなければ `url` だけを無視する。`swing publish --url` も同じ判定で拒否する。
- `title`: 空、256 バイト超、制御文字か見えない書式文字（`d` と同じ）を含む場合は `title` だけを無視する。保存の判断には使わない。
- `size`: ASCII の数字だけからなり `u64` に収まる値だけを読む（`nostr::parse_decimal`。`+` や空白、符号付きの値は `size` だけを無視する）。
- `content`: 空でなく `MAX_CONTENT_BYTES`（4096 バイト）以下なら `SiteEvent::message` に入れる。超えたら更新メモだけを捨て、イベントは受け入れる。保存の判断には使わない。
- Follow Set: relay のフィルタに加え、受信後にも kind・作者・`d` を確かめる（`nostr::is_follow_set_of`）。relay から受け取ったものの署名は下の「署名」のとおり確かめ済みなので繰り返さない。`state.json` から読んだ自分の Follow Set だけは `nostr::is_saved_follow_set_of` で署名も確かめ直す（`agent::follow`・`agent::replicas`・`mirror`）。`content`（暗号化 private 部分）は読まない。
- フィルタとの照合: `RelayClient` の nostr-sdk クライアントは `verify_subscriptions` を有効にして作る（nostr-sdk の既定は無効）。nostr-sdk は受信したイベントが REQ のフィルタのどれにも一致しなければ、取得のストリームや購読の通知に渡す前に捨てる。フィルタが 1 つの REQ では、EOSE より前に `limit` を超えて届いた分も捨てる（数えるのはフィルタとの照合より前なので、フィルタを無視する relay からは本来の答えも欠け得る）。`bounded_client`（signer アプリとの通信）には付けない。さらに `RelayClient` の取得関数は受信後にもフィルタの条件で照合して捨てる。

  | 取得関数 | 受信後に確かめる条件 |
  |---|---|
  | `fetch_follow_set` | kind が 30000、作者が自分の公開鍵、`d` が要求した名前（`is_follow_set_of`）、`created_at` が未来ずれの許容内（`plausible_at`。下記）。残ったものから最新 1 件 |
  | `fetch_follow_sets` | 作者が要求した `authors` に含まれ、kind が 30000・`d` が要求した名前（`is_follow_set_of`）。残ったものを作者ごとに最新 1 件にする |
  | `fetch_site_events` | kind が `[nostr].site_event_kind`、作者が要求した `authors` に含まれる |
  | `fetch_own_latest_site`（`swing publish` の同じ内容かの確認） | 作者が自分の公開鍵、サイトイベントとしてパースでき（kind も確かめる）、`d` が要求した名前。残ったものから `select_latest` で最新 1 件 |
  | `fetch_replica_reports` | kind が `[nostr].replica_event_kind`、`a` タグのどれかが要求した座標のいずれかと一致する |
  | `fetch_replica_reports_by` | `fetch_replica_reports` の条件に加え、作者が要求した報告者に含まれる |
  | `fetch_follow_set_authors_referencing` | kind が 30000・`d` が要求した名前（`is_follow_set_of`）で、`p` タグのどれかが要求した相手に含まれる（作者を指定しない取得） |
  | `fetch_own_reports`（agent のレプリカ報告の同期） | 呼び出し側（`agent::replicas`）が作者が自分であることを、`parse_replica_report` が kind を確かめる |
  | `fetch_reports_about`（agent が他の報告者からの報告の時刻を記録する） | kind が `[nostr].replica_event_kind`、作者が要求した報告者に含まれ、`p` タグに要求した相手がある。呼び出し側（`agent::replicas`）はさらに報告者が `Chosen` であること（自分は除く）を確かめ、作者は `parse_replica_report` の結果で確かめる |
  | 購読（`subscribe_site_events`） | agent の `submit` が Follow Set の対象かを確かめて捨てる（[`agent.md` の「並行処理」](agent.md#並行処理)） |

- レプリカ報告: [`../protocol.md`](../protocol.md#8-レプリカ報告) の「無視しなければならない」条件に当たる報告は、報告全体を拒否する。`expiration` タグは `size` と同じ規則（`nostr::parse_decimal`）で読み、読めなければ拒否する。この実装ではさらに、`content` が `MAX_CONTENT_BYTES` を超える報告も拒否する。`expiration` が無ければ `None` として読み、期限切れかどうかは使う側（`ReplicaReport::counts_at`）が判定する。
- 署名は nostr-sdk が受信時に検証する。nostr-sdk は一度検証した `id` を覚えていて、同じ `id` を名乗るイベントは署名を確かめずに通す。`RelayClient` と `bounded_client` の nostr-sdk クライアントには `AdmitPolicy`（`MatchingIds`。`Event::verify_id` が偽なら拒否）を付け、`id` が中身から計算した値と一致しないイベントを、取得・購読・署名アプリとの通信のどれでも、nostr-sdk がデータベースに入れて通知する前に捨てる。
- 大きさ: `RelayClient` の nostr-sdk クライアントは、relay から受け取るメッセージを 128 KiB（`MAX_RELAY_MESSAGE_BYTES`）、イベントを 16 KiB（`MAX_EVENT_BYTES`）・タグ 600 個（`MAX_EVENT_TAGS`）までに制限する。kind 30000（Follow Set）だけはイベントの上限を 64 KiB（`MAX_FOLLOW_SET_EVENT_BYTES`）にする。サイトイベントとレプリカ報告は既定の 16 KiB。超えたものは nostr-sdk が受信時に捨てる。署名アプリとの通信（[`signer.md`](signer.md)。常駐の接続とペアリング）は `nostr::bounded_client` で作り、メッセージもイベントも 128 KiB（`MAX_SIGNER_EVENT_BYTES`）・タグ 600 個までにする。

サイトイベントの「取得 → パース → サイトごとの最新を選ぶ」は `RelayClient::fetch_latest_sites(site_event_kind, authors)` にまとめてあり、agent の過去分の取り込み・`mirror::collect_sites`・`replicas::collect`・`webring::collect` が使う。パースに失敗したイベントは debug ログを出して捨てる。

## 未来ずれの許容（`nostr::MAX_FUTURE_SKEW`）

`created_at` の未来ずれ許容は `nostr::MAX_FUTURE_SKEW`（900 秒）。`nostr::plausible_at(created_at, now)` がこれを超えるかどうかを判定する。保存の可否（`policy::decide`）だけでなく、「現在の版」やその時点で有効な Follow Set をどれとして選ぶかにも同じ基準を使う（`select_latest`・`choose_follow_set`・`newest_by_address`・`fetch_follow_set(s)` など。Follow Set の選び方は [`agent.md`](agent.md#follow-set-の選び方)）。

許容内の版どうしの新しさは、Follow Set もサイトごとの最新のサイトイベント（`select_latest`）も NIP-01 の置き換え規則（[`../protocol.md`](../protocol.md#4-サイトイベント)）で比べる。比べ方は `nostr/mod.rs` の 1 か所（`is_newer_replaceable` と `select_latest` が共有）にある。`SiteEvent::id` にイベントの `id` を持つ。

## レプリカ報告の信頼度（`replicas::Tier`）

`replicas::Tier` は作者本人（`Author`）と、作者かオペレータの Follow Set に明示的に選ばれている報告者（`Chosen`）を信頼し、それ以外を `Other`（未検証）とする（`replicas::tier_of`）。

- `replicas::Chosen`: 作者ごとの Follow Set とオペレータ自身の Follow Set をまとめて持つ。作者ごとの Follow Set は、どちらの作り方でも 1 回の `fetch_follow_sets(mirror_set, authors)` でまとめて取る。
  - `replicas::fetch_chosen`（`replicas::collect`。`swing replicas` / `/api/replicas`）: オペレータの Follow Set を relay から取得する。`authors` は呼び出し元が渡したもの（ダッシュボード API では最大 100 件（`MAX_KEYS`）、CLI の `swing replicas` には上限なし）。
  - `replicas::fetch_chosen_with_own`（`mirror::collect_sites`）: 取得済みの Follow Set の対象（`targets`。`MAX_FOLLOW_SET_ENTRIES` 件まで）をそのまま作者とオペレータの集合に使い、オペレータの Follow Set は取り直さない。
- 集計（`replicas::fetch_for_sites` が取得し `replicas::collect_reports` が数える）:
  - 取得: 次の 2 つを並行して行い、合わせる。片方が失敗したら warn を出してもう片方だけで数え、両方失敗したときだけエラーにする。
    - サイトイベントの座標（`<[nostr].site_event_kind>:<作者>:<d>`）を `#a` に入れた取得。
    - 信頼できる報告者（`Chosen::trusted_reporters`: サイトの作者、オペレータの Follow Set、その作者たちの Follow Set の順に重複を除き、`MAX_TRUSTED_REPORTERS`（1000）人まで）を `authors` に入れた取得（`fetch_replica_reports_by`）。
  - 報告者・`d` ごとに最新の 1 件だけを残す（`newest_by_address`。未来ずれの許容を超えるものは先に捨てる）。
  - パースに失敗したもの（[検証](#検証)）と `cid` タグが無いものは数えない。
  - 残りは `ReplicaReport::counts_at(now)` が true のもの（`created_at` が未来ずれの許容以内、`now - created_at` が `nostr::MAX_REPORT_AGE`（7 日）以内、`expiration` が無いか `now` より先）だけを数える。
- `collect_reports` は、`MAX_REPORTS_PER_SITE` で切り詰める前に、サイトごとの報告を `(tier, created_at 降順, reporter の hex)` の順に並べ替える。tier が高い（`Author` → `Chosen` → `Other`）報告者ほど、`created_at` が古くても切り詰めで残る。
- 表示用の報告者一覧（`replicas::replicas_of`。`swing replicas` と `/api/replicas` の `reporters`）は `(tier, 最新版を持つものが先, reporter の hex)` の順。
- カウント（`replicas::count_replicas`）: 現在の版の CID を持つ報告を tier で分け、`Author`・`Chosen` の数を `ReplicaCounts.trusted`、`Other` の数を `ReplicaCounts.unverified` にする。`replicas::collect` は `trusted` を `SiteReplicas.replicas`（CLI・DTO では単に `replicas`）に、`unverified` を `SiteReplicas.unverified` に入れる。表示は `replicas::format_replica_counts`（`"3"` / `"3 (+12 unverified)"`。`unverified` が 0 なら括弧を出さない）。
