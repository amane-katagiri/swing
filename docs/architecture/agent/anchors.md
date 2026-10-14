# アンカー（agent/anchors.rs, anchor/）

[`../agent.md`](../agent.md) の子ページ。イベントの形式と受信側の規則は [`../../protocol.md`](../../protocol.md#10-アンカー広告)（kind 35982）。

外から直接接続を受けられる Kubo を持つ参加者は、そのノードの PeerID をアンカーとして広告する。agent は、ミラー対象の参加者が出したアンカーの広告から数個を選び、Kubo の peering でそこへの接続を張り続ける。NAT の内側のノードから張った接続は双方向に使えるので、アンカーはその接続でホールパンチ無しにこちらのサイトを取得できる。

| ファイル | 内容 |
|---|---|
| `nostr/anchor.rs` | 広告の組み立て（`build_anchor_ad_builder`）・解釈（`parse_anchor_ad`）・PeerID の検査（`validate_peer_id`）・宛先ごとの最新（`latest_anchor_ads`） |
| `nostr/client/anchor_relay.rs` | `AnchorRelay` トレイト（広告・自分の広告・自分を含む Follow Set の取得と広告の送信）と `RelayClient` の実装、そのテスト。レプリカ報告の取得は `ReportRelay::fetch_reports_by`（`nostr/client/report_relay.rs`） |
| `anchor/mod.rs` | 定数、到達性のヒステリシス（`Hysteresis`）、広告を出すかの判断（`ad_to_publish`）、ダッシュボードと共有する状態（`AnchorStatus`） |
| `anchor/select.rs` | 使える広告の絞り込み（`usable_ads`）、候補の組み立て（`candidates`）と選び方（`select_anchors`）。どれも純粋関数 |
| `anchor/peering.rs` | peering の差分（`plan_peering`、純粋関数）と Kubo への反映（`apply_peering`） |
| `ipfs/peering.rs` | Kubo RPC の `swarm/peering/ls`・`add`・`rm` と `swarm/peers`、`KuboNode` トレイト（`id`・AutoNAT・peering・接続中のピア） |
| `agent/anchors.rs` | agent 側の状態（`AnchorAgentState`: `AnchorStatus`・出した広告の記録・PeerID のキャッシュ・ミラー対象のサイト・2 つのタスクの `AbortHandle`）と、agent からの呼び出し（接続の確認・広告の確認・アンカーの選び直し） |

## 定数（`anchor/mod.rs`）

| 名前 | 値 | 意味 |
|---|---|---|
| `CHECK_INTERVAL` | 60 秒 | AutoNAT を読んで広告を確かめる間隔 |
| `PUBLIC_AFTER` | 10 分 | 届く（`public`）と見なすまでに、`public` が続かなければならない時間 |
| `NOT_PUBLIC_AFTER` | 20 分 | 届かないと見なすまでに、`public` 以外が続かなければならない時間 |
| `AD_TTL` | 3 日 | 広告の `expiration`（`created_at` からの長さ） |
| `AD_REFRESH` | 1 日 | `public` の広告を出し直す間隔 |
| `AD_RETRY` | 10 分 | 広告の送信か自分の広告の取得に失敗した後、次に試すまでの間隔 |
| `MAX_PEERS` | 16 | `max_peers` に設定できる最大の値 |

## 設定（`[anchor]`）

| キー | 環境変数 | 既定 | 意味 |
|---|---|---|---|
| `advertise` | `SWING_ANCHOR_ADVERTISE` | `true` | 届くと判定されている間、このノードを広告する。`false` にすると、出していた `public` の広告を `private` で置き換える |
| `max_peers` | `SWING_ANCHOR_MAX_PEERS` | `4` | 接続を張り続けるアンカーの数の上限。0〜16（`MAX_PEERS`）で、範囲外は設定の読み込みでエラーになる。`0` なら選ばず、relay にも問い合わせず、SWING が足した peering を外す |

どちらもダッシュボードから編集でき、反映には再起動が要る（[`../config.md`](../config.md#編集できるキー)）。

## 広告を出す側

`run_until` のループが `CHECK_INTERVAL` ごとに `Agent::start_anchor_check` を呼び、広告の確認（`check_anchor_ad`）を別タスクで動かす（前のタスクがまだ動いていれば今回は始めない。NIP-46 の署名アプリの承認待ちでループを止めないため）。

自分の PeerID は `AnchorAgentState` にキャッシュし、無いときだけ Kubo の `id` で読む。Kubo の RPC（AutoNAT・`swarm/peers`・`swarm/peering/ls`）に失敗したら、Kubo がリポジトリを替えて起動し直した場合に備えてキャッシュを捨て、次に要るときに読み直す。`id` に失敗したら、その回は広告の確認やアンカーの選び直しをしない。

1. `advertise = false` で、出した広告の記録が「無し」か期限内の `public` 以外なら、取り下げるものが無いので、Kubo にも relay にも問い合わせずに終わる。
2. `advertise = true` なら Kubo の AutoNAT（`IpfsClient::autonat`）を読み、ヒステリシスに通す。`public` 以外（`private`・`unknown`・`offline`）と、RPC の失敗は「届かない」として数える。状態は「未定」から始まり、同じ観測が `PUBLIC_AFTER`／`NOT_PUBLIC_AFTER` 続いたときだけ「届く」「届かない」に移る。観測が変わると続いた時間は 0 からになり、決まった状態はそのまま残る。起動直後の「未定」の間は何もしないので、再起動のたびに広告を取り下げて出し直すことはない。`advertise = false` なら AutoNAT は読まない（どの状態でも取り下げるだけなので）。
3. `advertise = true` で「未定」なら、ここで終わる。それ以外は自分の PeerID（`d` に使う）を得る。PeerID が出した広告の記録の PeerID と違えば、記録を「不明」に戻す。
4. 出した広告の記録が「不明」なら、relay から自分の、その PeerID の広告を取得する（`fetch_own_anchor_ads`。kind 35982・作者が自分・`#d` が PeerID）。最新の 1 件を記録にする。無ければ「無し」。取得に失敗したら `AD_RETRY` の後にまた取得する。
5. 次の表で決める（`ad_to_publish`）。

| `advertise` | 到達性 | 出した広告 | 出すもの |
|---|---|---|---|
| `true` | 届く | 無い・不明・`private`・`public` で `AD_REFRESH` 以上前か期限切れ | `public` |
| `true` | 届く | 期限内の `public` で `AD_REFRESH` 未満前 | なし |
| `true` | 届かない | 期限内の `public` | `private` |
| `false` | （判定しない） | 期限内の `public` | `private` |
| ほか | | | なし |

6. 出す広告は `d` = PeerID・`reachability`・`expiration` = `created_at + AD_TTL`・`alt` = `SWING anchor: <PeerID>`・`content` は空。`created_at` は今の時刻と、前の広告の `created_at + 1` の大きい方（同じ秒に置き換えても新しい方として扱われるように）。署名は `RelayClient::sign`（秘密鍵でも NIP-46 の署名アプリでも同じ経路）で、設定した relay 全部へ送る。
7. 1 つでも relay が受理すれば記録を更新する。受理されなかったときと、署名や送信に失敗したときは記録を変えず、`AD_RETRY` の後まで送り直さない（NIP-46 の署名アプリに 1 分ごとに要求を出さないため）。
8. 記録が期限内の `public` なら「広告中」として `AnchorStatus` に入れる。

`private` の広告は 1 度だけ出し、出し直さない。期限が切れれば、前の `public` の広告とともに使われなくなる。別の PeerID で出した広告（Kubo のリポジトリを作り直した場合や、同じ鍵の別のノード）には触れない。

## 接続する側

アンカーの選び直し（`refresh_anchors`）は、poll ごとに `Agent::start_anchor_refresh` が報告の回とは別のタスクで始める（Follow Set の取得と過去分の取得の後、[報告の回](../agent.md#全体の流れ)を始めた直後）。前の選び直しのタスクがまだ動いていれば今回は始めない。relay の取得に時間がかかっても、報告の回を遅らせたり飛ばしたりしない。対象は今の Follow Set の pubkey（`Agent::targets`）。

1. `max_peers` が 0 なら選ばずに 5 へ進む（選ぶものは空）。Follow Set がまだ決まっていなければ（対象が空）選ぶものは空。
2. 自分の PeerID を得て、対象の pubkey が出した kind 35982 を取得する（`fetch_anchor_ads`。作者ごとのフィルタで 1 人あたり 8 件まで。1 人の広告が多くてもほかの作者の広告を押し出さない）。どちらかに失敗したら、今の選択と peering をそのまま残して終わる。
3. 宛先（作者・`d`）ごとに NIP-01 の置き換え規則で最新の 1 件を選び（未来ずれの許容を超える広告は除く）、それを解釈する。最新が壊れていれば、その宛先は使わない（古い方に戻らない）。解釈できないもの（`d` が PeerID でない、`reachability` が無いか `public`・`private` 以外、`expiration` が無いか 10 進整数でない）を除き、`anchor::usable_ads` で `private`・期限切れ・未来の日付・自分の PeerID のものを除く。作者は取得の段階で対象に限っている。PeerID は base58btc の multihash で、identity（42 バイト以下）か sha2-256（32 バイト）であり、書き直しても同じ文字列になること。
4. 残った広告の作者について、次の 2 つを relay から同時に取得する。
   - 相互の関係: 作者の Follow Set（`mirror_set` が同じもの）に自分が入っているか（`fetch_followers_among`。`#d` に `mirror_set`、`#p` に自分を入れて relay に絞らせ、作者ごとに最新の Follow Set の `p` タグに自分があるかを確かめ直す）。失敗したら誰も相互でないとして続ける。
   - 保存しているサイト: 作者が出したレプリカ報告のうち、自分のミラー対象のサイト（直前の poll で取得したサイトイベントのうち、`max_sites_per_account` で絞った後の、実際にミラーするものの宛先）についてのもの（`ReportRelay::fetch_reports_by`）。`nostr::counted_replica_reports`（`replicas::collect` と共通）で、宛先ごとに最新の報告を選んで、正しく解釈でき、`cid` が 1 つ以上あり、数えてよい（`ReplicaReport::counts_at`）ものを、その作者が保存しているサイトとする。失敗したら誰も何も保存していないとして続ける。
5. 選ぶ（下記）。選んだ PeerID の集合を Kubo に反映し（[Kubo への反映](#kubo-への反映)）、選んだ結果を `AnchorStatus` に入れる。

### 保存しているサイトの数え方

報告がサイトの今の版を挙げているかは問わず、どれかの版を挙げていれば保存しているとして数える。アンカーはそのサイトの作者をミラーしているので、新しい版も間もなく取りに行く。今の版に限ると、作者が公開するたびに選び直しが起きて接続が入れ替わる。

### 選び方（`anchor::select_anchors`）

候補は PeerID ごとに 1 つ（`anchor::candidates`）。同じ PeerID を複数の作者が広告していれば、相互の作者、次に pubkey の小さい作者の広告を使う。1 人が複数の PeerID を広告していれば、それぞれが候補になる。保存しているサイトは作者ごとの値なので、同じ作者の候補は 1 つの集合を共有する。

1. 相互の作者の候補を、前回選んだもの・保存しているサイトの多いもの・PeerID の順に並べ、上限まで選ぶ。1 人の作者からは 1 つだけ選ぶ。
2. 残りの枠は、まだ選んだ候補で覆われていないミラー対象のサイトを最も多く保存している候補から 1 つずつ選ぶ。同じ数なら前回選んだもの、次に PeerID の小さいもの。新しく覆うサイトが 1 つも無い候補は、枠が余っていても選ばない（張り続ける接続はアンカーの資源も使うため）。同じ作者の候補は同じサイトを保存しているので、1 人の作者からは 1 つしか選ばれない。そのため作者ごとに代表（前回選んだもの、次に PeerID の小さいもの）を 1 つ決めてから比べる。1 の段で選んだ作者は除く。

前回選んだもの（`AnchorStatus` の選択）を優先するので、同点の候補の間で接続が入れ替わり続けることはない。結果の各要素は PeerID・作者・理由（`mutual`／`coverage`）・保存しているミラー対象のサイトの数を持つ。

## Kubo への反映

Kubo の peering（`swarm/peering/add`・`rm`・`ls`）を実行時に使う。Kubo の設定（`Peering.Peers`）には書かないので、Kubo を再起動すると消える。

- `add` には `/p2p/<PeerID>` だけを渡す。Kubo v0.43.1 の libp2p ホストは routed host（`core/node/libp2p/host.go` が `libp2p.Routing` を渡し、go-libp2p が `routedhost.Wrap` で包む）で、peering が接続するときにアドレスが無ければ peer routing（DHT）で引く。3 つのコンテナでの確認: A と B の間に直接の接続が無い状態で A に `/p2p/<B>` だけを足すと 10 秒ほどで B につながり、A の `Routing.Type` を `none` にすると 45 秒待ってもつながらなかった。アドレスを付けずに済むので、広告に multiaddr を入れない仕様（[`../../protocol.md`](../../protocol.md#10-アンカー広告)）のままで動く。
- SWING が足した peering の集合（managed）を `AnchorStatus` に持つ。反映のたびに `ls` で今の peering を読み、次のように差分をとる（`plan_peering`）。
  - 足す: 選んだもののうち `ls` に無いもの。成功したら managed に入れる。
  - 外す: managed のうち、選んでおらず `ls` にあるもの。失敗したら managed に残し、次の poll でまた外す。
  - 選んでいて、`ls` にあるが managed でないもの（`Peering.Peers` に書いたもの）は足さず、managed にも入れないので、外すこともない。
- `ls` に失敗したら（Kubo が `--offline` で動いているときも）、何も変えずに次の poll でやり直す。

### 再起動

- 管理下の Kubo（[`../up.md`](../up.md)）が落ちると、`swing up` は agent を止めて Kubo を起動し直し、agent を新しく作る。新しい agent の最初の poll の選び直しで、`ls` に無い選んだ PeerID を足し直す。
- 外部の Kubo（`[kubo].managed = false`）が agent の知らないうちに再起動しても、反映のたびに `ls` と突き合わせるので、次の poll で足し直す。
- managed の集合はダッシュボードの状態（`AppState::anchors`）に持つので、同じ `swing up` の中で agent だけが再起動しても失われない。`swing up` 自体を止めると失われる。管理下の Kubo は一緒に止まるので問題にならないが、外部の Kubo では前の `swing up` が足した peering が Kubo の再起動まで残る。

## 接続の確認

アンカーの選び直し（`refresh_anchors`）の最後に、選び直しが途中で終わったときも含めて、選んだアンカーがあれば Kubo の `swarm/peers` を読み、選んだ PeerID のうち接続中のものの集合を `AnchorStatus` に入れる。選んだものが無ければ RPC を呼ばず、集合を「不明」にする。RPC に失敗したとき（`--offline` の Kubo など）も「不明」にする。新しく選んだアンカーは、Peering に足した直後でまだ接続が張れていないことが多く、次の選び直し（poll の間隔以内）まで未接続として出ることがある。

## 表示

[`GET /api/network`](../dashboard/http-api/status.md#get-apinetwork) の `anchor` に、広告中かどうかと選んだアンカー、それぞれの接続（[接続の確認](#接続の確認)の結果。API は Kubo に問い合わせない）を出す。Settings 画面の「ネットワーク」パネル（[`../dashboard/views/settings.md`](../dashboard/views/settings.md#ネットワーク)）と `swing status`（[`../cli/views.md`](../cli/views.md)）はこれを使う。
