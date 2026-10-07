# Follow Set と sites の集計（`src/mirror/mod.rs`, `src/mirror/set.rs`）

CLI の `swing mirror list` / `add` / `remove`・`swing sites`（[`cli/views.md`](cli/views.md)）と、ダッシュボードの `GET /api/mirror`・`POST /api/mirror/add`・`/api/mirror/remove`（[`dashboard/http-api/nostr.md`](dashboard/http-api/nostr.md)）・`GET /api/sites`（[`dashboard/http-api/status.md`](dashboard/http-api/status.md#get-apisites)）が共有する処理。ここには表示に依存しない集計と書き換えだけを書き、出力の書式・ステータスコード・JSON はそれぞれのページに置く。CLI の `add` / `remove` は API を叩くので、書き換えはダッシュボード側でだけ動く。

`state.json` は読むだけで書かない。

## 使う Follow Set

`mirror::current_follow_set`。relay から自分の Follow Set（kind 30000、`d = [nostr].mirror_set`）を取り、`state.json` の `follow_set`（自分の・同じ `mirror_set` のものに限る）と比べて新しい方を使う。比べ方と検証は [agent の Follow Set の選び方](agent.md#follow-set-の選び方)と同じ（`nostr::choose_follow_set`）。

- 保存済みの方を使ったときは注記を付ける。relay に無かったら `(follow set not found on relays; using the one saved by the agent)`、relay の方が古かったら `(relays returned an older follow set; using the newer one saved by the agent)`。
- どの relay も答えなかった（[`nostr/fetch.md`](nostr/fetch.md#1-回の-reqrelayclientfetch)）ときは「見つからない」とは扱わず、エラーにする。

`mirror::collect_mirror_list`（`mirror list`・`GET /api/mirror`）はこの Follow Set の `title` タグと、`p` タグのうち公開鍵としてパースできたものをタグの順に返す。

## Follow Set の書き換え（`mirror::apply_add` / `apply_remove`）

- 入力の鍵は npub・hex・nprofile を受け付け、重複を除いて順を保つ（`mirror::parse_pubkey_inputs`）。
- 元にするのは[使う Follow Set](#使う-follow-set)。元にする版は `created_at` が [`MAX_FUTURE_SKEW`](nostr.md#未来ずれの許容nostrmax_future_skew) 以内のものに限るので、それより先の日付の版は元にせず、置き換えられない。
- `p` 以外のタグと `content`（NIP-51 の暗号化 private 部分）をそのまま残し、`p` タグを足すか消して再署名する。消すのは公開鍵としてパースでき、指定した鍵に一致する `p` タグだけ。
- タグは `d`（`[nostr].mirror_set`）→ その他 → `p` の順に並べ直す。
- `created_at` は現在時刻と「元の Follow Set の `created_at` + 1」の大きい方（`mirror::set::next_created_at`）。
- Follow Set が無い状態の `add` は `["title", "SWING mirror list"]` 付きで新規作成する。
- 追加済みの鍵の `add`、未登録の鍵の `remove` は no-op（`unchanged`）とし、変更が 1 つも無ければ署名も送信もしない。
- `add` の結果の `p` タグのうち公開鍵としてパースできたものの数が `MAX_FOLLOW_SET_ENTRIES`（[取得と表示の上限](nostr/fetch.md#定数)）を超えるときは送信せずに `FollowSetCapExceeded`（`would grow the follow set to <N> entries, over the 500-entry limit; remove some first`）で止める。切り詰めはしない。
- 送信先は `[nostr].relays` で、relay ごとの受理の結果を返す。受理された後に agent をすぐ poll させるのはダッシュボード側（[`dashboard.md#概要`](dashboard.md#概要)）。

## sites の集計（`mirror::collect_sites`）

1. [使う Follow Set](#使う-follow-set) の `p` を対象にする（`nostr::follow_set_pubkeys_capped`。重複を除いてタグ順に `MAX_FOLLOW_SET_ENTRIES` 件まで、切り詰めたら warn を 1 回出す）。Follow Set が無ければ対象は空。
2. 対象者のサイトイベントを取り、サイトごとの最新（`nostr::select_latest`）を、1 作者あたり `d` の昇順で先頭 `MAX_SITES_PER_AUTHOR_LISTED` 件までにする（`nostr::cap_sites_per_author`）。
3. その版のレプリカ数を数える（下記）。
4. `state.json` を読み、サイトごとに次を付ける。
   - `stored_size`・`stored_at`: 同じサイトの版のうち `cid` がイベントと一致するものの `size`（保存時に `dag/stat` で測った値）と保存時刻。一致する版が無ければ無し。`stored` は一致する版があるかどうか。
   - `previous`: 一致する版が無く、同じサイトの版が state にあるとき、そのうち `created_at` が最大の版（`cid`・`size`・`created_at`・`stored_at`）。新しい版を `min_update_interval` などで待っている間も、配っている版が分かるようにする。一致する版があるか、版が 1 つも無ければ無し。
   - `nip05`: `state.json` の `verifications` にあるそのサイトの検証結果。
   - `size`・`url`・`title`・`message` はイベントの値（`size` は作者の自己申告）。
5. `[policy].remove_on_unfollow` の値と、[unfollow の一覧](#unfollow-の一覧)を添える。

アカウントは対象者全員を hex の昇順で並べ、サイトイベントが無い対象者も空の一覧で入る。Follow Set とサイトイベントの取得、`state.json` の読み込みのどれかが失敗したら全体がエラーになる。

### レプリカ数

`replicas::fetch_chosen_with_own` で信頼できる報告者の集合を作り（自分の Follow Set は 1 の対象をそのまま使い、作者たちの Follow Set はまとめて 1 回取る）、`replicas::fetch_for_sites` で 2 の版の報告を取って、最新版の CID を持つ報告を数える。tier・数える報告の条件・取得の仕方は [「レプリカ報告の信頼度」](nostr.md#レプリカ報告の信頼度replicastier)。

レプリカ報告の取得に失敗しても集計は続け、全サイトのレプリカ数を無しにして失敗の理由（`replicas_error`）を添える。

### unfollow の一覧

`state.json` に版があり、キーの pubkey が 1 の対象にいないものを、アカウント（hex の昇順）とサイトごとに、`created_at` が最大の版 1 つで並べる。Follow Set が見つからなかったときは対象が空なので、保存済みの全アカウントがここに入る。

- `size` と `stored_size` はどちらも state に記録した版の大きさ、`stored` は常に true。`url`・`title`・`message`・レプリカ数は無し、`nip05` は state の検証結果。
- キーが `<pubkey hex>:<d>` として読めない版は入れない。
- いつ消えるかは [agent の unfollow](agent.md#unfollow)。
