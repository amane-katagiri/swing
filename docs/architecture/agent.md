# mirror-agent（agent/, health.rs, policy.rs, state.rs）

[`architecture.md`](../architecture.md) の一部。MFS のパスと Kubo RPC は [`kubo.md`](kubo.md)、NIP-05 は [`nip05.md`](nip05.md)、ダッシュボードは [`dashboard.md`](dashboard.md)、内蔵 gateway は [`gateway.md`](gateway.md)、`swing up`（Kubo の起動・監視、agent の再起動）は [`up.md`](up.md)。

## agent/ の構成

| ファイル | 内容 |
|---|---|
| `agent/mod.rs` | `Agent` 構造体の定義、`new`、`poll_once`、メンテナンス系（`sweep`・`collect_garbage`・`reconcile`・`remove_unfollowed`）、state 保存の共通ヘルパー（`save`） |
| `agent/lifecycle.rs` | `run_until`（プロセスのライフサイクル本体。`CancellationToken` を受け取る）、ダッシュボード・内蔵 gateway タスクの起動・終了 |
| `agent/follow.rs` | `refresh_follow_set`（Follow Set の取得・保存・再送、対象の切り替え、サイトイベントの購読・取得）、`limit_sites_per_account` |
| `agent/store.rs` | `Agent::submit`/`drain`（キューイングと直列実行）、`apply_site_event`（「保存の順序」の中核）、NIP-05 検証、`decide`/`version_infos` |
| `agent/replicas.rs` | レプリカ報告の差分計算・送信（`SentReport`・`ReportBook`・`Held`・`reports_to_send`・`held`・`load_sent_reports`・`sync_reports`） |
| `agent/test_support.rs` | ユニットテスト共通のフィクスチャ（`Fixture`・`FakeKubo`・`FakeNip05`・`FakeRelay`）。`#[cfg(test)]` |

外部からは `swing::agent::run_until` だけを公開する（`swing up` が Kubo・agent を協調させて起動・再起動するために使う。[`up.md`](up.md)）。テストは対応するモジュールの `#[cfg(test)] mod tests` に置く。

## 全体の流れ

1. relay 群に接続し、state を読み、`[dashboard].listen` が `off` でなければダッシュボードの、`[gateway].listen` が `off` でなければ内蔵 gateway の `TcpListener` をそれぞれ bind する（どちらも失敗したら agent 全体がエラーで終了する）。続けて `Agent` を組み立て、起動時の突き合わせを行う。
2. 突き合わせの完了前にシャットダウンが要求されたら、突き合わせを打ち切って即座に shutdown へ進む（ダッシュボード・gateway はまだ起動していないので、relay を切断するだけで終わる）。
3. 突き合わせを終えたら、bind できていればダッシュボードの `AppState` を作って `dashboard::serve` を、bind できていれば `gateway::serve` を、それぞれ別タスクで起動する（ダッシュボードは起動前に `<state_dir>/upload/` を掃除する。[`dashboard/http-api.md`](dashboard/http-api.md#post-apipublishupload)）。
4. `poll_interval` ごとの tick（最初の tick は起動直後）、およびダッシュボードでの mirror 変更（`Notify`）で `poll_once`（次を行う）を実行する。
   1. sweep
   2. Follow Set を決める。決まらなければ警告を出して 3 と 4 を飛ばす。
   3. unfollow
   4. 対象 pubkey 群のサイトイベント（過去分を含む）を取得して購読し直し、`nostr::select_latest`（`created_at` が現在時刻より 900 秒（`nostr::MAX_FUTURE_SKEW`）を超えて先のイベントは無視する）でサイトごとの最新版を選び、pubkey ごとに、保存済みのサイトすべてと、それ以外のサイトを `created_at` の新しい順に合計 `max_sites_per_account` 件まで、タスクに投入する。一時的な取得・保存の失敗はここで再試行される。
   5. レプリカ報告の同期（Follow Set が決まらなくても行う）
5. 購読で届いたサイトイベントをタスクに投入する。送信元が今の Follow Set にいなければ warn を出して無視する。購読 ID と kind が一致しない通知は debug ログで捨てる。
6. タスクはサイト単位で「保存の順序」に従って処理する。新版を記録したら、同時実行の枠を返してからレプリカ報告の同期を行う。

relay の切断、Kubo のエラー、不正なイベントはログに出して続ける。relay への再接続と再購読は nostr-sdk が行う。nostr-sdk の通知チャネル（容量 2048）から溢れた分は 2.4 の取り直しで回収される。通知ストリーム自体が終わったらエラーで終了する。

### シグナルと終了

`swing up` は `shutdown::cancel_on_signal()`（[`up.md`](up.md#shutdownshutdownrs)）で作った `CancellationToken`（の子トークン）を `run_until(config, token)` に渡す。`run_until` 自体はシグナルを直接扱わない。

- `cancel_on_signal`: SIGINT（`ctrl_c`）を待つ。unix ではさらに SIGTERM も待ち、どちらか先に届いた方で `token.cancel()` する。受信から `FORCE_EXIT_GRACE_PERIOD`（10 秒）たってもプロセスが終わっていなければ `std::process::exit(1)` する watchdog を兼ねる。
- 起動時の突き合わせと `poll_once` は `race_with_shutdown` でトークンの cancel と競争させ、cancel が先に届いたら処理中の I/O を打ち切って shutdown に進む。
- shutdown: `shutdown_dashboard`（ダッシュボードに終了を通知してサーバタスクを最大 5 秒待つ。超えたら warn を出して待つのをやめる）と `shutdown_gateway`（gateway の子トークンを cancel してサーバタスクを最大 5 秒待つ。同じく超えたら warn）を両方行ってから relay を切断し、ループを抜ける。
- `main.rs` はランタイムを明示的に組み立て、`run()` の後に `shutdown_timeout(10s)` で畳む（ブロッキング呼び出しで詰まったスレッドがあっても drop で止まらない）。

ダッシュボードは agent のメモリ上の state を触らず、`state.json` を読み直す（[`dashboard.md`](dashboard.md)）。内蔵 gateway は agent の state や Kubo RPC を一切使わず、Kubo の gateway へ透過的にプロキシするだけ（[`gateway.md`](gateway.md)）。

## Follow Set の選び方

relay から取得した版と `state.follow_set` の版を比べて使う方を決める。

- 候補は kind 30000・作者が自分・`d` が `mirror_set`・署名が正しいものだけ。保存済みの版も同じ条件で確かめる（`mirror_set` を変えると古い版は使わない）。
- 新しさは NIP-01 の置き換え可能イベントの規則（`created_at` が大きい方、同じなら `id` が小さい方）で比べる。
- `nostr::choose_follow_set` は、relay から取得した版・保存済みの版のどちらも、`created_at` が現在時刻より 900 秒（`nostr::MAX_FUTURE_SKEW`）を超えて先なら「無い」として扱ってから下の表の判定に入る。保存済みの版まで捨てるのは、先の時刻の Follow Set が一度 state に保存されると、以後まともな時刻の版が二度と「新しい」と判定されず、ミラー対象が凍結するのを防ぐため（保存済みを捨てても、relay から改めてまともな時刻の版が取れれば、それが保存されて復旧する）。`RelayClient::fetch_follow_set` / `fetch_follow_sets` 自身も取得直後に同じ基準でふるい落とすので、relay から先の時刻の版しか取れなかった場合は「見つからない」と同じ扱いになる。

| relay から | 保存済み | 使う版 | state に保存 | relay に再送 |
|---|---|---|---|---|
| 取れた | 無い | 取れた版 | する | しない |
| 取れた（保存済みと同じ `id`） | ある | 保存済み | しない | しない |
| 取れた（保存済みより新しい） | ある | 取れた版 | する | しない |
| 取れた（保存済みより古い） | ある | 保存済み | しない | する |
| 見つからない（先の時刻で捨てた場合を含む） | ある | 保存済み | しない | する |
| 取得に失敗 | ある | 保存済み | しない | しない |
| 見つからない、または失敗 | 無い、または先の時刻で捨てた | 決まらない | — | — |

再送は署名済みのイベントをそのまま全 relay に送り、どこにも受理されなければ warn を出す。NIP-09 で Follow Set を削除しても再送は続くので、ミラーをやめるときは `swing mirror remove` を使う。

決まった Follow Set から対象 pubkey を取り出すのは `nostr::follow_set_pubkeys_capped`（`extract_follow_set_pubkeys` はこれの薄いラッパ）で、`p` タグの重複を除いた先頭 `nostr::budget::MAX_FOLLOW_SET_ENTRIES`（500）件だけを対象にする。500 件を超える Follow Set（自分のものを含む）は、超えた分が対象から静かに外れるのではなく、`refresh_follow_set` が `warn!` を 1 回出してから続行する（[取得と表示の上限](../architecture.md#取得と表示の上限nostrbudget)）。

## unfollow

Follow Set が決まった tick で行う（決まらない tick では何もしない）。Follow Set の更新はこれより先に反映する。

- `remove_on_unfollow = true`: `state.sites` か `state.verifications` にエントリがあり、今の Follow Set にいない pubkey を state から消して保存し、`<mfs_root>/agent/<pubkey hex>` を消す。state と比べるので、agent の停止中に外した相手や、設定を `true` に変える前に外した相手も消える。
- `false`: 外れた相手の保存済みの版を残す。新しい版は取らない。保持期間の適用と容量の集計は続き、最新版は残る。起動時の突き合わせで壊れていた版は取り直さずに消える。

## 保存の順序

1. 事前判定: `size` タグ（無ければ不明）で `policy::decide` する。skip なら終わる。
2. NIP-05 検証（`[policy].nip05` が `off` 以外）。`require` で `Verified` でなければ終わる。
3. 取得: `dag/export` の CAR を読み捨てながらバイト数を数え、`policy::fetch_limit`（`max_update_size`・`max_per_site`・`max_per_account` の最小値）を超えたら打ち切る。`SWING_FETCH_IDLE_TIMEOUT` か `SWING_FETCH_TIMEOUT` を超えたら失敗。いずれも state と MFS は変えない。
4. ディレクトリ確認: `files/stat /ipfs/<cid>` の `Type` を見る。`directory` でなければ `reason = "not_a_directory"` で warn を出して終わる（MFS にはまだ何も置いていないので消すものは無く、取得したブロックは Kubo の GC に任せる）。`files/stat` 自体が失敗したら取得の失敗と同じ扱いで終わる（次の poll で取り直す）。
5. 以降は state のロックの中で行う。作者が Follow Set から外れていれば終わる。
6. 版のパスに CID を置く（既存の項目は先に消す）。失敗したら終わる。
7. `dag/stat`（`offline=true`）の `TotalSize` を実サイズとする。ブロックが欠けていればエラーになるので、6 のパスを消して終わる。`size` タグより大きければ warn を出す。
8. 実サイズで `policy::decide` する。skip なら 6 のパスを消して終わる。
9. 新版を記録し、evict した版を `sites` から消して state を保存してから、evict した版のパスを消す。

パスの削除に失敗しても state はそのままにし、sweep に任せる。

## sweep

state のロックの中で行う。

1. すべてのサイトに `policy::retention_evictions`（`max_per_site`・`keep_versions`・`keep_days`。最新版は残す）を適用する。evict があれば `sites` から消して state を保存し、パスを消す。evict が無ければ state は保存しない。
2. `health::find_garbage` で `<mfs_root>/agent` を `<pubkey hex>/<site>/<created_at>` の 3 階層たどり、state の版に対応しない項目を消す。版が 1 つも残らない `<site>`・`<pubkey hex>` のディレクトリや、想定外の階層のファイルも消す。一覧に失敗したディレクトリの下は消さない。

## 起動時の突き合わせ

サイトごとに `health::check_site` で確かめる。まず版ごとに、版のパスの CID（`files/stat`）が記録と一致するかを見る。一致した版の CID をまとめて 1 回の `dag/stat`（`offline=true`）に渡し、成功すればその版はすべて完全とする（1 つでもブロックが欠けていれば失敗するので、成功は全版の完全性を意味する）。失敗したときだけ版ごとに `dag/stat` をやり直して、どの版が欠けているかを決める。

パスが無い、CID が違う、ブロックが欠けている版は warn を出して `sites` から消す（次の poll で取り直され、パスは sweep で消える）。`files/stat` 自体が失敗した版は残す。消した版があれば state を保存する。サイト単位で DAG をたどるので、保存量に比例して時間がかかる（版どうしで共有しているブロックは 1 回しかたどらない）。

`check_site` は同じ呼び出しでサイトの実容量（まとめた `dag/stat` の `TotalSize`）も返す。突き合わせでは使わず、`swing status` とダッシュボードの表示に使う。state には記録しない。

## 並行処理

- `submit`（購読通知・過去分の取得の両方から呼ばれる入口）は、対象判定（Follow Set にいるか）より前に `nostr::plausible_at` で `created_at` を確かめ、900 秒（`nostr::MAX_FUTURE_SKEW`）を超えて先なら `future_created_at` を理由に warn を出してその場で捨てる。キューにある実行中・待機中のイベントを置き換えることはない。続けて対象判定を行い、対象外の pubkey のイベントもその場で捨てる。relay がフィルタを無視して対象外のイベントを大量に送っても、キューやタスクは増えない。「保存の順序」5 の判定は、`submit` から実行までの間に対象から外れた場合に効く。
- 「保存の順序」を同時に実行するタスクは最大 `concurrency` 個。
- 同じ pubkey のタスクは同時に `max_sites_per_account` 個まで。超えたイベントは捨て、次の poll で拾い直す。
- 同じサイト（`pubkey:d`）のタスクは同時に 1 つ。実行中に来たイベントは、実行中・待機中のものより `created_at` が新しいときだけ待機に置き（1 件、上書き）、実行後に同じタスクで続けて処理する。
- 保存の順序の 5〜9、sweep、unfollow、突き合わせは state のロックの中で直列に行う。レプリカ報告の同期どうしは報告用のロックで直列になる（取る順は報告用 → state）。取得中の一時的なディスク使用量は最大で `concurrency` × `fetch_limit`。

## レプリカ報告

イベント形式は [`protocol.md`](../protocol.md#8-レプリカ報告)。同期は報告用のロックの中で直列に行う。state のロックは保存している CID を読む間だけ取る。

保存している CID（サイト `<pubkey hex>:<d>` ごと）:

- `state.sites` の各版の CID。
- `<mfs_root>/publish/<自分の pubkey hex>/` の下のディレクトリ名を `d` に戻し（`site_name` で同じ名前に戻らないもの、`d` の条件を満たさないものは無視）、その下の名前が整数の項目の CID（`files/ls` の `Hash`）。同じサイトが `state.sites` にもあれば合わせる。同じ Kubo で `swing publish` した自分のサイトだけが対象で、別の Kubo で publish したサイトは報告しない。
- `publish/<自分>/` の一覧に失敗したら自分が作者のサイトすべてを、`publish/<自分>/<site>/` の一覧に失敗したらそのサイトを「不明」とし、今回は送らない。

送信済みの記録はメモリにだけ持つ（サイトごとに `cid` の集合と `created_at`）。まだ読めていなければ、同期のたびに relay から自分の報告（`replica_event_kind`、作者が自分）を取得し、`d` ごとの最新を記録に入れる（記録にある方が新しければそのまま）。取得に失敗したら warn を出し、送信は続ける。

送るもの:

| 保存している CID | 記録 | 送る内容 |
|---|---|---|
| ある | 無い、または `cid` が違う | 保存している CID |
| ある | 同じ `cid` で、`created_at` から `report_ttl / 2` 以上たった | 保存している CID |
| 無い | `cid` が 1 つ以上 | `cid` 無し（取り下げ） |
| 不明 | — | 送らない |

- `created_at` は現在時刻。記録の `created_at` 以下になるときは記録の `created_at + 1` にする。`expiration` は `created_at + report_ttl`。
- 全 relay に送り、どこかに受理されたら記録を更新する。受理されなければ warn を出し、次の同期で送り直す。
- 取り下げた記録は `cid` 無しで残り、出し直さない。

受信側で報告を数える規則（`replicas::collect_reports` / `ReplicaReport::counts_at`）は agent 自身の動作ではなく [`cli.md`](cli.md#replicas) を参照。`report_ttl` の既定 `3d` に対し、数えるのをやめる期間（`nostr::MAX_REPORT_AGE`、7 日）はそれより長い。TTL を伸ばした他クライアントの報告も、期限切れ前に数えられなくなることがないようにするため。

## ポリシー判定（policy.rs）

`policy::decide` は純粋関数。入力は同サイトの既存版、使用量（他サイトの合計容量、同じ pubkey の他サイトの合計容量とサイト数）、候補（cid, size, created_at）、ポリシー設定、現在時刻。出力は `Decision { store: Option<String>, evict: Vec<String>, reason: String }`。

1. `nostr::plausible_at` が false、つまり `created_at` が現在時刻より先で、ずれが 15 分（`nostr::MAX_FUTURE_SKEW`。`policy.rs` ではなく `nostr.rs` にある定数で、`select_latest` や Follow Set の選択（[Follow Set の選び方](#follow-set-の選び方)）でも同じ値を使う）を超えるなら skip（`future_created_at`）。
2. 同じ CID が同サイトに記録済みなら skip（`duplicate_cid`）。
3. 新しいサイトで、同じ pubkey の記録済みのサイトが `max_sites_per_account` 個以上あれば skip（`max_sites_per_account`）。
4. `created_at` が同サイトの最新版以下なら skip（`stale`）。
5. 現在時刻が同サイトの最大の `stored_at` から `min_update_interval` 未満なら skip（`min_update_interval`）。
6. `size` が `max_update_size` を超えるなら skip。
7. 同サイト合計が `max_per_site` を超えるなら古い版から evict する。新版単体で超えるなら skip。
8. `keep_versions`（最低 1 に丸める）を超える古い版を evict する。
9. `keep_days` より古い版を evict する。最新版は残す。
10. evict 後、同じ pubkey の全サイト合計が `max_per_account` を超えるなら skip（`max_per_account`）。他のサイトは削らない。
11. evict 後の全サイト合計が `max_total_storage` を超えるなら skip。他のサイトは削らない。

`size` 不明の事前判定では 6 を飛ばし、新版を 0 バイトとして 7〜11 を評価する。

`created_at` は作者の自己申告なので、間隔の判定（5）だけは自分が保存した実時刻（`stored_at`）で測る。1 と合わせて、`created_at` を先に振っても取り込みの頻度は上げられない。5 で見送った版は、`min_update_interval` が経ったあとの poll（全体の流れ 4.4）で同じイベントが改めて評価されて受理される。relay には `pubkey + kind + d` ごとに最新の 1 件しか残らないので、見送っている間の中間の版は取れないが、最新の内容には必ず追いつく。

## state.json（state.rs）

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

- キーは最初の `:` で `<pubkey hex>` と `<d>` に分ける。
- `sites[].size` は `dag/stat` の `TotalSize`。
- `status` は `verified` / `mismatch` / `not_applicable` / `error`。`verifications` の扱いは [`nip05.md`](nip05.md)。
- `follow_set` は最後に保存した署名済みの Follow Set。無ければ `null`（キーが無くても `null`）。
- `sites` と `verifications` は必須キー。ファイルが無い、または空白だけなら空の state として扱う。
- state を消すと、次の sweep で `<mfs_root>/agent` の下がすべて消え、次の poll で取り直しになる。
