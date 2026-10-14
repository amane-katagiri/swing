# アンカーの広告と peering の実装

[アンカー広告の仕様案](2026-10-14-anchor-advertisement-spec.md) の kind 35982 を実装した。外から届くノードは自分をアンカーとして広告し、agent はミラー対象の参加者のアンカーから数個を選んで Kubo の peering で接続を張り続ける。現状の記述は [`architecture/agent/anchors.md`](../architecture/agent/anchors.md)。

## 決めたこと

- 設定は新しいセクション `[anchor]` に `advertise`（既定 `true`）と `max_peers`（既定 4、0 で接続しない）を置いた。どちらもダッシュボードから編集できる。`[agent]` や `[kubo]` に混ぜるより、広告する側と接続する側の 2 つがまとまって見える。
- 到達性の判定には揺れがあるので、ヒステリシスを入れた。AutoNAT を 60 秒ごとに読み、`public` が 10 分続いたら「届く」、`public` 以外が 20 分続いたら「届かない」とする。`unknown`・`offline` と RPC の失敗も「届かない」に数える（仕様の「確かめられなくなったら取り下げる」に合わせた）。取り下げの方を長くしたのは、`public` の広告が少し長く残っても受信側は接続に失敗するだけなのに対し、取り下げと再広告を繰り返すと署名の回数が増えるため（NIP-46 の署名アプリでは承認の手間になる）。
- 起動直後は「未定」として何もしない。前回の `public` の広告が relay に残っていても、届くと分かれば出し直さずに済み、届かないと分かったときだけ取り下げる。再起動のたびに取り下げて出し直すことはない。
- 広告の有効期限は 3 日、出し直しは 1 日ごと。取り下げの `private` の広告にも 3 日の期限を付け、出し直さない。
- 出した広告の記録は agent のメモリに持ち、起動後に必要になったとき relay から自分の広告（今の PeerID の分）を読み直す。レプリカ報告が送った報告を relay から読み直すのと同じ考え方で、`state.json` の形は変えていない。
- 署名や送信に失敗したら 10 分待ってから送り直す。60 秒ごとの確認のたびに署名アプリへ要求を出さないため。確認は別タスクで動かし、署名アプリの承認待ちで agent のループを止めない。
- アンカーの選び直しは poll の報告の回の最後に入れた。レプリカ報告と再送の取得と relay への REQ が重ならない。
- 相互の関係は、広告の作者の Follow Set に自分が入っているかで判断する。webring やレプリカの tier と同じ `fetch_follow_sets` を使う。
- 保存しているサイトは、作者のレプリカ報告がどれかの版を挙げていれば数える（今の版に限らない）。アンカーはそのサイトの作者をミラーしているので新しい版も間もなく取りに行き、今の版に限ると公開のたびに選び直しが起きるため。
- 選び方は仕様の例どおり、相互の作者を先に（1 人 1 つまで）、残りを貪欲法で、まだ覆われていないミラー対象のサイトを最も多く保存しているものから選ぶ。新しく覆うサイトが無い候補は枠が余っても選ばない（アンカー側の接続の負担を増やさないため）。同点は前回選んだもの、次に PeerID の順で、接続が入れ替わり続けないようにした。
- 表示する「サイト数」は、作者が保存しているミラー対象のサイトの総数にした（選んだ時点で新しく覆った数ではない）。表で読んだときに意味が分かりやすい。
- 作者の名前（kind 0）はコードのどこでも引いていないので、表示はフィンガープリントと npub にした。
- peering は Kubo の設定（`Peering.Peers`）を書き換えず、実行時の RPC（`swarm/peering/add`・`rm`・`ls`）だけを使う。SWING が足した PeerID の集合をダッシュボードの状態に持ち、反映のたびに `ls` と突き合わせる。`ls` にあって SWING が足していないもの（利用者が `Peering.Peers` に書いたもの）は外さない。突き合わせるので、Kubo が再起動して peering が消えても、管理下・外部どちらの Kubo でも次の poll で足し直す。
- 足した集合をダッシュボードの状態（`AppState`）に持つのは、同じ `swing up` の中で agent だけが再起動しても失われないようにするため。`swing up` を止めると失われ、外部の Kubo では前の peering が残る。ファイルに持つかは残タスクにした。

## Kubo v0.43.1 の peering がアドレス無しで動くか

`swarm/peering/add` に `/p2p/<PeerID>` だけを渡してよいかを確かめた。

- ソース: `core/commands/swarm.go` の `peering add` は引数を multiaddr として読み `peer.AddrInfosFromP2pAddrs` で `AddrInfo` にする（`/p2p/<id>` だけならアドレスは空）。`core/node/libp2p/host.go` は `libp2p.Routing` でルーティングを渡し、go-libp2p はそのときホストを `routedhost.Wrap` で包む。peering サービスはこのホストの `Connect` を使うので、アドレスが無ければ peer routing で引く。
- 実験: `ipfs/kubo:v0.43.1` のコンテナ 3 つを、外に出られない Docker ネットワークに置いた（Bootstrap 空・mDNS 無効）。C を DHT サーバにし、A と B は C にだけつないだ（A と B は DHT クライアントで、互いの接続は無い）。A で `swarm peering add /p2p/<B>` を実行すると 10 秒ほどで A と B がつながった。A の `Routing.Type` を `none` にして同じことをすると、45 秒待ってもつながらなかった。
- RPC の応答: `add`・`rm` は `{"ID":"…","Status":"success"}`。同じ PeerID を 2 回足しても、無いものを外しても成功する。`/p2p/` の無い PeerID を `add` に渡すと 500。`ls` は `{"Peers":[{"ID":"…","Addrs":[]}]}`。`--offline` の Kubo ではどれも `ErrNotOnline`。

アドレスを付けずに済むので、`routing/findpeer` で先に引く処理は入れなかった。広告に multiaddr を入れない仕様のままで動く。

## 作ったもの

- `nostr/anchor.rs`: 広告の組み立て・解釈・PeerID の検査（base58btc の identity か sha2-256 の multihash）・宛先ごとの最新。
- `nostr/client/anchor_relay.rs`: `AnchorRelay` トレイトと `RelayClient` の実装。
- `ipfs/peering.rs`: peering と `swarm/peers` の RPC、`KuboNode` トレイト。
- `anchor/`: 定数、ヒステリシス、広告の判断、候補と選び方、peering の差分と反映、共有する状態。
- `agent/anchors.rs`: 広告の確認と選び直し。`run_until` に 60 秒ごとの tick を足した。
- 設定 `[anchor]`（カタログ・`swing.example.toml`・`.env.example`）。
- `GET /api/network` に `anchor`。Settings 画面の「ネットワーク」パネルに「アンカー」の段（Kubo が `offline` でも出す）、`swing status` に `Anchor:`・`Anchors:` の行。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通る。
- ユニットテスト: 広告の組み立てと解釈・不正な広告（`d`・`reachability`・`expiration` の欠落や不正、壊れた最新が古い広告を隠すこと、`private`・期限切れ・未来の日付）、ヒステリシス、広告の判断表、選び方（相互の優先・貪欲法・上限・覆わない候補・同点の安定）、peering の差分と偽の Kubo への反映（利用者の peering を外さない・再起動後の足し直し・失敗時の再試行）、agent の広告（再起動後の読み直し・未定の間は何もしない・設定でオフ・署名の失敗）と選び直し、プロセス内の relay（nostr-sdk の `LocalRelay`）への取得と送信、`/api/network` の形、`swing status` の行。
- デモ環境: Kubo が `--offline` なので AutoNAT は `offline` になり、広告も peering の反映（`ls` が `ErrNotOnline`）も試せない。デモの relay に、alice（相互）と bob の `public` の広告、carol の `private` の広告、mallory（ミラー対象でない）の `public` の広告、bob が alice と carol のサイトを保存しているレプリカ報告を入れ、agent が alice を「相互にミラー」、bob を「ミラー対象を 3 サイト保存」で選び、carol と mallory を選ばないこと、Settings 画面のパネル（ライト・ダーク）と `swing status` の `Anchors: 2 selected` を確かめた。
- peering の実際の接続は、上の 3 つのコンテナで確かめた。

## 追記: 確認を受けて決めたこと

- Settings の「ネットワーク」パネルは、`offline` のときアンカーの部分を出さない。Kubo がネットワークにつながっていなければ広告もつなぎもしないので、「外から届くと判定し続けると広告します」の案内がかえって紛らわしいため。
- `/api/network` の `anchor` が無い古い `swing up` への後方互換は入れない。`swing status` と `swing up` は同じ版を使うのが前提で、版が違っても `Reachability: unknown (<理由>)` が出るだけで終了コードは変わらないため。
- アンカーとして広告しているかの段落は、「アンカー」の節から判定の説明の後へ移し、`public` のときだけ出す。届かないときに「届くと判定し続けると広告します」と続くのは回りくどく、判定の説明で足りるため。「アンカー」の節には接続先の一覧だけを残す。
- フィンガープリントを先頭の 1 つだけ出す表（アンカーの表・Storage check の表）でも、ツールチップには 4 つ全部を出す（`ui.js::fingerprintBadge`）。1 つだけでは見分けにくく、ツールチップなら場所を取らないため。

## 追記: レビューを受けて直したこと

- アンカー広告の取得を、作者ごとのフィルタと `limit`（1 人 8 件）に変えた（`fetch_per_author`）。組全体で 1 つの `limit` だと、広告の多い 1 人がほかの作者の広告を押し出せる。
- `/api/network` は Kubo の `swarm/peers` を呼ばなくした。Desktop 画面のトレイは確認のたびに `/api/network` を読むが、使うのは到達性だけで、アンカーを選んでいる間は毎回 RPC が増えていた。接続の確認は agent の 60 秒ごとの tick で行い、選んだ PeerID のうち接続中のものを `AnchorStatus` に入れる。選び直しの時点（poll ごと）でなく tick にしたのは、選んだ直後は接続がまだ張れておらず、poll の間隔（既定 5 分）のあいだ「未接続」が残るため。
- `advertise = false` で取り下げるものが無いと分かった後は、tick で AutoNAT も `id` も呼ばない。オフのときは到達性によらず取り下げるだけなので、AutoNAT は読まない。自分の PeerID はキャッシュし、Kubo の RPC が失敗したときだけ捨てて読み直す（Kubo がリポジトリを替えて起動し直した場合に備える）。
- `max_peers` を 0〜16 に限った。張り続ける接続は相手のアンカーの資源も使うので、数十といった値を許す理由が無い。
- ミラー対象のサイト（被覆を数える対象）を、`max_sites_per_account` で絞った後の、実際にミラーするサイトにした。絞る前の全サイトで数えると、自分が保存しないサイトを覆うアンカーを選びうる。
- アンカーの選び直しを報告の回から外し、別のタスク（前のタスクが動いていれば始めない）にした。選び直しは relay の取得を 3 回続けて行い、最悪で数分かかるので、報告の回に入れておくと次の poll の報告の回が飛ぶ。相互の関係とレプリカ報告の取得は同時に行う。報告の回と REQ が重ならないという前の判断は、報告を遅らせない方を優先して取りやめた。
- 相互の関係の取得に `#p`（自分）を足し、relay に Follow Set を絞らせた。返った Follow Set の `p` は確かめ直す。`fetch_follow_set_authors_referencing` は作者を絞らず、作者ごとの最新も選ばないので使わなかった。`#p` で絞ると、ある relay に新しい（自分を外した）Follow Set があり別の relay に古い（自分を含む）ものしか無いとき、古い方で相互と見なしうる。相互は選ぶ順の優先に使うだけなので、そのままにした。
- 重複をまとめた。広告と報告の送信（署名して全 relay へ送り、1 つでも受理されたか）は `RelayClient::sign_and_publish`、「宛先ごとに最新の報告を選んで、解釈でき、`cid` があり、数えてよいもの」は `nostr::counted_replica_reports`（`replicas::collect_reports` と共通）にした。レプリカ報告の取得（`fetch_reports_by`）は `AnchorRelay` から `ReportRelay` に移した。`AnchorPeerDto` は `AccountDto` を `flatten` で持つ（JSON の形は変わらない）。出した広告の記録は `impl From<&AnchorAd> for Published` で作り、`ad_to_publish` の判定は `Published::is_live_public` を使う。
- 使える広告の絞り込み（`private`・期限切れ・未来の日付・自分の PeerID）は `anchor::usable_ads` の 1 か所にした。`candidates` の中ではなく、相互の関係とレプリカ報告を取る前に絞る。使える広告が無ければ、その 2 つを取りに行かずに済むため。作者を対象に限る絞り込みは、取得が作者を限っているので外した。
- 選び方の貪欲法を作者ごとに比べる形にした。同じ作者の PeerID は同じサイトを保存しているので、1 人から 2 つ目が選ばれることはもともと無く、結果は変わらない。作者の保存しているサイトの集合は候補の間で共有する（`Arc`）。並べ替えのキーは PublicKey と PeerID の `&str` を直接比べる（hex 文字列を作らない。並び順は同じ）。
- 大きくなったファイルから、`/api/network` の処理（`dashboard/api/network.rs`）と DTO（`dashboard/dto/network.rs`）、`swing status` の到達性とアンカーの行（`health/network.rs`）、アンカーの relay のテスト（`nostr/client/anchor_relay.rs`）を分けた。agent のアンカーの状態は `AnchorAgentState` 1 つにまとめた。
- 接続の確認は、その後 tick から選び直しの最後に移した。tick で行うと、ダッシュボードを誰も見ていない間もアンカーを選んでいる限り毎分 `swarm/peers`（接続中の全ピアの一覧）を読むことになり、トレイの問題を常時の負荷に置き換えただけだったため。新しく選んだアンカーが次の選び直しまで「未接続」と出ることは許容した。
