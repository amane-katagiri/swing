# レプリカ報告（`agent/replicas.rs`）

[`../agent.md`](../agent.md) の子ページ。受信側で報告を数える規則は [`../nostr.md#レプリカ報告の信頼度replicastier`](../nostr.md#レプリカ報告の信頼度replicastier)。

イベント形式は [`../protocol.md`](../../protocol.md#8-レプリカ報告)。ロックの取り方は [`../agent.md#並行処理`](../agent.md#並行処理)。state のロックは保存している CID を読む間だけ取る。

保存している CID（サイト `<pubkey hex>:<d>` ごと）:

- `state.sites` の各版の CID。
- `<mfs_root>/publish/<自分の pubkey hex>/` の下のディレクトリ名を `mfs::site_from_name` で `d` に戻し（エンコードし直して同じ名前にならないもの、`d` の条件を満たさないものは無視）、その下の名前が整数の項目の CID（`files/ls` の `Hash`。`nostr::canonical_cid` で CIDv1 の dag-pb に正規化し、正しい CID でないものは warn を出して無視する）。同じサイトが `state.sites` にもあれば合わせる。同じ Kubo で `swing publish` した自分のサイトだけが対象で、別の Kubo で publish したサイトは報告しない。
- `publish/<自分>/` の一覧に失敗したら自分が作者のサイトすべてを、`publish/<自分>/<site>/` の一覧に失敗したらそのサイトを「不明」とし、今回は送らない。

送信済みの記録はメモリにだけ持つ（サイトごとに `cid` の集合と `created_at`）。まだ読めていなければ、同期のたびに relay から自分の報告（`replica_event_kind`、作者が自分）をページに分けて全部取得し（[`nostr/fetch.md`](../nostr/fetch.md#ページに分ける取得relayclientfetch_pages)）、`d` ごとの最新を記録に入れる（記録にある方が新しければそのまま）。取得に失敗したら（最初のページに答えた relay が 1 つも無い場合を含む）warn を出し、読めたことにはしない。取れた分を記録に入れても、最後までたどれた relay が 1 つも無かったとき（期限切れ・途中で切れた。`Paged::complete` が false）も warn を出して読めたことにはしない。同期は poll ごとの報告の回（`start_report_round`）のほか、サイトを保存するたびにも走り、取得は最大 120 秒かかるので、取り直すのは報告の回だけにする。最初の同期（どちらの契機でも）は必ず取得し、読めなかったあとは次の報告の回まで取り直さない（報告の回 1 回につき最大 1 回）。その間の同期は記録にある分（取れた分を含む）で送信を続ける。

送るもの:

| 保存している CID | 記録 | 送る内容 |
|---|---|---|
| ある | 無い、または `cid` が違う | 保存している CID |
| ある | 同じ `cid` で、`created_at` から `report_ttl / 2` 以上たった | 保存している CID |
| 無い | `cid` が 1 つ以上 | `cid` 無し（取り下げ） |
| 不明 | — | 送らない |

- 自分の報告をまだ読めていないあいだは、relay に記録より新しい報告（再起動の直前に送ったものなど）があるかもしれないので、`created_at` を現在時刻より前にしない。記録の無いサイトは現在時刻、記録のあるサイトは現在時刻と記録の `created_at + 1` の大きい方にする（その回の中で同じ値になってもずらさない。ずらすと未来ずれの許容を超えうる）。そのため、その回に送る報告は（記録の `created_at + 1` が上回るもの以外）すべて同じ現在時刻になる。relay は `created_at` でページを区切り、同じ 1 秒のうちページに入らなかった分は取れない（[`nostr/fetch.md`](../nostr/fetch.md#ページに分ける取得relayclientfetch_pages)）ので、1 回に relay の 1 ページより多くの報告を送ると、あとで（再起動後など）読み直すときにその一部を読めないまま読めたことになりうる。読めなかった報告のサイトは、もう保存していなければ取り下げが送られず、保存していれば記録の無いサイトとして送り直すが、現在時刻 − i がその秒以前になるとき（読み直しがその回の直後のとき）は relay に古い方として捨てられうる。どちらもその報告が `report_ttl` で期限切れになるまでの間に限られる。
- 読めたあとは、`created_at` は、その回に送る報告の並び（`reports_to_send` の順。保存している CID のサイトをキー順に、続けて取り下げをキー順に）の i 番目（0 始まり）で現在時刻 − i（0 で止める）。relay はページを `created_at` で区切るので、1 回分の報告を同じ 1 秒に詰めない（[`nostr/fetch.md`](../nostr/fetch.md#ページに分ける取得relayclientfetch_pages)）。記録の `created_at` 以下になるときは記録の `created_at + 1` にし、その回ですでに使った値なら空くまで 1 ずつ足す。`expiration` は `created_at + report_ttl`（足し算は飽和させる。`report_ttl` の上限は [`../config.md#検証`](../config.md#検証)）。
- 全 relay に送り、どこかに受理されたら記録を更新する。受理されなければ warn を出し、次の同期で送り直す。
- 署名（NIP-46 の署名アプリへのリクエストを含む）か送信がエラーになったら warn を出して、その回の残りの報告は送らずに打ち切る。残りは次の同期で送り直す（署名アプリがオフラインのときの扱いは [`signer.md#署名アプリがオフラインのとき`](../signer.md#署名アプリがオフラインのとき)）。
- 取り下げた記録は `cid` 無しで残り、出し直さない。

ダッシュボードの `/api/activity`（値の意味は [`dashboard/http-api/status.md`](../dashboard/http-api/status.md#get-apiactivity)）のために、次の 2 つをメモリ上の `activity::Activity`（ダッシュボードの `AppState` と共有する）に記録する。

- publish の時刻: 保存している CID を集めるときに一覧した `publish/<自分>/<site>/` の整数名（CID が正しいもの）の最大値。一覧がすべて成功したら、版が無くても「確かめた」印を付ける。
- 他の報告者の報告の時刻:
  - 数えるのは信頼できる報告者の報告だけ。信頼できる報告者は `state.json` の `follow_set`（自分の Follow Set）から `replicas::Chosen::from_own` で作り、`Chosen::trusted_reporters` から自分を除いたもの（[「レプリカ報告の信頼度」](../nostr.md#レプリカ報告の信頼度replicastier)）。いなければ取得せず、成功として扱う。
  - poll ごとに relay から `replica_event_kind` で `#p` が自分、作者がその報告者の報告を取得する。`since` は前回までに記録した最大値（0 なら付けない）。報告者ごとに別のフィルタを置き、`AUTHORS_PER_SPLIT_REQ` 人分ずつ 1 つの REQ にまとめる（[`nostr/fetch.md`](../nostr/fetch.md#limit-と組の大きさ)）。購読は増やさない。
  - 次のすべてを満たす報告の `created_at` の最大値を記録する。`cid` 無し（取り下げ）の報告は数えない。
    - 報告者が `replicas::tier_of` で `Chosen`
    - `p` タグに自分がある
    - `parse_replica_report` でパースでき、作者が自分
    - `ReplicaReport::counts_at(now)` が true
    - `created_at` が今以前
    - `cid` タグのどれかが自分のそのサイトで保存している CID（直前のレプリカ報告の同期で集めたもの。一覧に失敗したサイトは前回の値）と一致する
  - 取得に成功したら、数える報告が無くても「確かめた」印を付ける。失敗したら warn を出し、値はそのまま。

