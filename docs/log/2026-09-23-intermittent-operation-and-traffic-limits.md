# 2026-09-23 間欠運用と通信量の制限

実装は行っていない。カジュアル利用（常時起動のサーバを持たない利用者）を想定したときに何が成立して何が成立しないかを、今のコードに照らして確かめた記録と、通信量の制限を入れるとしたらどこに置くかの検討。

同じ日の [relay から取得するイベントと表示件数の上限](2026-09-23-fetch-and-display-budgets.md) とは別の話題。あちらは relay から受け取る「件数」の上限（資源保護のための固定値）、ここは IPFS でやり取りする「バイト数」の上限（運用者が回線事情に合わせて決める値）。

## きっかけ

「カジュアルに使ってもらうとして、サーバで常時起動じゃないと使い物にならないか」という問い。

## 調べたこと

### 間欠運用で壊れないもの

- `swing publish` は一発のコマンドで完結する（[`cli.md`](../architecture/cli.md#publish)）。add → MFS へ配置 → サイトイベントの署名と送信 → 古い版の削除。常駐は要らない。発行したサイトイベントは relay に残るので、自分のノードが落ちていても「このサイトの最新はこの CID」という情報自体は生き続ける。
- `swing agent` は停止・再起動に耐える。
  - 起動時の突き合わせ（`health::check_site`）で、パスが無い・CID が違う・ブロックが欠けている版を state から落とす。次の poll で取り直される。
  - `poll_once` は購読だけに頼らず、毎回サイトイベントの過去分も取得し直す（[`agent.md`](../architecture/agent.md) 全体の流れ 4.4）。停止中に流れた更新はまとめて追いつく。
  - unfollow は relay の Follow Set ではなく state と比べるので、停止中に外した相手も復帰後の tick で消える（[`agent.md`](../architecture/agent.md#unfollow)）。

つまり「1 日に数時間だけ起動するノート PC」でもミラーとしては機能する。

### 間欠運用で劣化するもの

| 事象 | 影響 | 根拠 |
|---|---|---|
| 自分の Kubo が落ちている間は自分からは配れない | ミラーが 0 人のサイトは誰からも読めなくなる。始めたてが最も弱い | IPFS の性質。SWING の相互ミラーはこれを緩和するための仕組みそのもの |
| 停止が `report_ttl`（既定 `3d`）を超えるとレプリカ報告が expire する | 他者の `swing replicas`・ダッシュボードから、自分がミラー保持者として数えられなくなる。復帰すれば次の同期で送り直されて戻る | 送り直しは `report_ttl / 2`（既定 1.5 日）経過後（[`agent.md`](../architecture/agent.md#レプリカ報告)） |
| 受信側は 7 日より古い報告を数えない | 上と同じ症状を、報告が expire しない設定でも起こす | `nostr::MAX_REPORT_AGE` |

「1〜2 日に 1 回は起動する」なら報告は切れない。常時起動が要るのは「自分」ではなく「相互ミラーの網のうち誰か」である、という整理になる。

### 通信量の内訳と、今ある制御点

| 経路 | 量 | 今の制御 |
|---|---|---|
| relay の WebSocket | 小 | `poll_interval` |
| `dag/export` によるサイトの取得（下り） | 大。1 サイトあたり最大 `policy::fetch_limit`（`max_update_size` / `max_per_site` / `max_per_account` の最小値、既定 2GB） | `src/policy.rs:88`。バイト数は取得中に数えている（`src/ipfs.rs:318`） |
| bitswap で他者へ配る（上り） | 大。上限なし | 無し。Kubo の領分で SWING からは見えない |
| DHT の provide / reprovide | 中。保存 CID 数に比例 | `Provide.Strategy`（`docker/kubo-init.d/001-swing-config.sh`）のみ |

現状の上限はすべて「1 回あたり」「1 サイトあたり」で、累積を縛るものが無い。`min_update_interval`（既定 `1h`）が同一サイトの取り直し間隔を抑えるだけで、フォロー数が増えれば 1 tick の総取得量は際限なく増える。月あたりの上限がある回線では、これだけでは足りない。

## 検討した案

実装順の候補。上ほど安く、効きが大きい。

1. **累積の取得予算**（`[agent] max_fetch_per_day` / `max_fetch_per_month`、環境変数は `SWING_MAX_FETCH_PER_DAY` / `SWING_MAX_FETCH_PER_MONTH`）。`src/ipfs.rs:318` で既にバイト数を数えているので、合計を state に持たせて `policy::decide` の事前判定に渡す。超過なら `reason = "budget_exhausted"` で skip し、`dag/export` を張る前に止める。窓が開けば次の poll で普通に拾い直されるので、復帰処理は別途要らない。窓の開始時刻と累積バイト数を state に持つ形になる。
2. **一時停止トグル**（ダッシュボードのスイッチと設定・環境変数）。テザリングに切り替えたら止める、という手動運用。ダッシュボードの mirror 変更が `poll_once` を起こす `Notify` の経路に相乗りできる。
3. **取得の時間帯ウィンドウ**（`active_hours = "02:00-07:00"` のような指定）。カジュアル利用には予算より直感的。
4. **メータード回線の自動判定**。OS ごとに実装が分かれる（Linux は NetworkManager の D-Bus プロパティ `Metered`、Windows は WinRT の `NetworkCostType`、macOS は `NWPathMonitor.isExpensive`）。1〜3 があれば優先度は低い。

### 上り（bitswap）は SWING 側では絞れない

下りは上記で完全に縛れるが、配る側は Kubo が行っており SWING からは観測も制御もできない。モバイル回線で確実に通信を止める方法は Kubo ごと落とすことしかない。今の compose 構成では agent を止めても `ipfs` コンテナは配り続ける。

これは [配布方式の設計](2026-09-21-distribution-design.md) の `swing up`（Kubo を子プロセスとして面倒を見る supervisor）と噛み合う。swing の停止が Kubo の停止になれば、一時停止トグルが上りにも効く。

間接的な緩和としては `Swarm.ConnMgr.HighWater` を下げて接続相手を減らす、`Reprovider.Interval` を延ばす、といった Kubo 設定がある。今の `docker/kubo-init.d/001-swing-config.sh` は `Datastore.StorageMax` と `Provide.Strategy` しか触っていないので、追加する余地はある。

## 未確認

- Kubo v0.43.1（`compose.yaml` が固定しているバージョン）にバイトレートの帯域制限があるか。`Swarm.ResourceMgr` は接続数・ストリーム数・メモリの制限であって帯域ではない、という理解で検討したが、実物の設定項目では確かめていない。上りの緩和策はこの確認が前提になる。
- `report_ttl` を既定より大きくしたときの実際の見え方。制約は `report_ttl / 2 > poll_interval` だけなので伸ばせるが、他クライアントの集計側（`MAX_REPORT_AGE` = 7 日）との兼ね合いで、伸ばしても 7 日以上は意味が無い。

## 次にやること

- `docs/todo.md` に累積の取得予算・一時停止と時間帯ウィンドウ・上りの抑制を追加した。
- README に「常時起動しない場合に何が起きるか」の節を足すかは未決。protocol ではなく運用の話なので README か architecture のどちらに置くかも含めて保留（`docs/todo.md` に項目としては入れた）。
