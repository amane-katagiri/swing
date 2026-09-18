# mirror-agent（agent/, health.rs, policy.rs, state.rs）

[`architecture.md`](../architecture.md) の一部。MFS のパスと Kubo RPC は [`kubo.md`](kubo.md)、NIP-05 は [`nip05.md`](nip05.md)、ダッシュボードは [`dashboard.md`](dashboard.md)。

## agent/ の構成

| ファイル | 内容 |
|---|---|
| `agent/mod.rs` | `Agent` 構造体の定義、`new`、`poll_once`、メンテナンス系（`sweep`・`collect_garbage`・`reconcile`・`remove_unfollowed`）、state 保存の共通ヘルパー（`save`） |
| `agent/lifecycle.rs` | `run`（プロセスのライフサイクル）、SIGINT/SIGTERM・watchdog の待ち受け、ダッシュボードタスクの起動・終了 |
| `agent/follow.rs` | `refresh_follow_set`（Follow Set の取得・保存・再送、対象の切り替え、サイトイベントの購読・取得）、`limit_sites_per_account` |
| `agent/store.rs` | `Agent::submit`/`drain`（キューイングと直列実行）、`apply_site_event`（「保存の順序」の中核）、NIP-05 検証、`decide`/`version_infos` |
| `agent/replicas.rs` | レプリカ報告の差分計算・送信（`SentReport`・`ReportBook`・`Held`・`reports_to_send`・`held`・`load_sent_reports`・`sync_reports`） |
| `agent/test_support.rs` | ユニットテスト共通のフィクスチャ（`Fixture`・`FakeKubo`・`FakeNip05`・`FakeRelay`）。`#[cfg(test)]` |

外部からは `swing::agent::run` のみを公開する。テストは対応するモジュールの `#[cfg(test)] mod tests` に置く。

## 全体の流れ

1. relay 群に接続し、state を読み、`[dashboard].listen` が `off` でなければダッシュボードの `TcpListener` を bind する（失敗したら agent 全体がエラーで終了する）。続けて `Agent` を組み立て、SIGINT・SIGTERM のリスナーを作ってから起動時の突き合わせを行う。
2. 突き合わせの完了前にシグナルが届いたら、突き合わせを打ち切って即座に shutdown へ進む（ダッシュボードはまだ起動していないので、relay を切断するだけで終わる）。
3. 突き合わせを終えたら、bind できていればダッシュボードの `AppState` を作って `dashboard::serve` を別タスクで起動する（起動前に `<state_dir>/upload/` を掃除する。[`dashboard/http-api.md`](dashboard/http-api.md#post-apipublishupload)）。
4. `poll_interval` ごとの tick（最初の tick は起動直後）、およびダッシュボードでの mirror 変更（`Notify`）で `poll_once`（次を行う）を実行する。
   1. sweep
   2. Follow Set を決める。決まらなければ警告を出して 3 と 4 を飛ばす。
   3. unfollow
   4. 対象 pubkey 群のサイトイベント（過去分を含む）を取得して購読し直し、pubkey ごとに、保存済みのサイトすべてと、それ以外のサイトを `created_at` の新しい順に合計 `max_sites_per_account` 件まで、サイトごとの最新版をタスクに投入する。一時的な取得・保存の失敗はここで再試行される。
   5. レプリカ報告の同期（Follow Set が決まらなくても行う）
5. 購読で届いたサイトイベントをタスクに投入する。送信元が今の Follow Set にいなければ warn を出して無視する。購読 ID と kind が一致しない通知は debug ログで捨てる。
6. タスクはサイト単位で「保存の順序」に従って処理する。新版を記録したら、同時実行の枠を返してからレプリカ報告の同期を行う。

relay の切断、Kubo のエラー、不正なイベントはログに出して続ける。relay への再接続と再購読は nostr-sdk が行う。nostr-sdk の通知チャネル（容量 2048）から溢れた分は 2.4 の取り直しで回収される。通知ストリーム自体が終わったらエラーで終了する。

### シグナルと終了

SIGINT・SIGTERM のどちらでも同じように終了する。

- 起動時の突き合わせと `poll_once` は `race_with_shutdown` でシグナルの受信と競争させ、シグナルが先に届いたら処理中の I/O を打ち切って shutdown に進む。
- shutdown（`shutdown_dashboard`）: ダッシュボードに終了を通知してサーバタスクを最大 5 秒待ち（超えたら warn を出して待つのをやめる）、relay を切断してループを抜ける。
- watchdog: 別タスクが独立したリスナーでシグナルを待ち、受信から 10 秒たっても終了していなければ `std::process::exit(1)` する。
- `main.rs` はランタイムを明示的に組み立て、`run()` の後に `shutdown_timeout(10s)` で畳む（ブロッキング呼び出しで詰まったスレッドがあっても drop で止まらない）。

ダッシュボードは agent のメモリ上の state を触らず、`state.json` を読み直す（[`dashboard.md`](dashboard.md)）。

## Follow Set の選び方

relay から取得した版と `state.follow_set` の版を比べて使う方を決める。

- 候補は kind 30000・作者が自分・`d` が `mirror_set`・署名が正しいものだけ。保存済みの版も同じ条件で確かめる（`mirror_set` を変えると古い版は使わない）。
- 新しさは NIP-01 の置き換え可能イベントの規則（`created_at` が大きい方、同じなら `id` が小さい方）で比べる。

| relay から | 保存済み | 使う版 | state に保存 | relay に再送 |
|---|---|---|---|---|
| 取れた | 無い | 取れた版 | する | しない |
| 取れた（保存済みと同じ `id`） | ある | 保存済み | しない | しない |
| 取れた（保存済みより新しい） | ある | 取れた版 | する | しない |
| 取れた（保存済みより古い） | ある | 保存済み | しない | する |
| 見つからない | ある | 保存済み | しない | する |
| 取得に失敗 | ある | 保存済み | しない | しない |
| 見つからない、または失敗 | 無い | 決まらない | — | — |

再送は署名済みのイベントをそのまま全 relay に送り、どこにも受理されなければ warn を出す。NIP-09 で Follow Set を削除しても再送は続くので、ミラーをやめるときは `swing mirror remove` を使う。

## unfollow

Follow Set が決まった tick で行う（決まらない tick では何もしない）。Follow Set の更新はこれより先に反映する。

- `remove_on_unfollow = true`: `state.sites` か `state.verifications` にエントリがあり、今の Follow Set にいない pubkey を state から消して保存し、`<mfs_root>/agent/<pubkey hex>` を消す。state と比べるので、agent の停止中に外した相手や、設定を `true` に変える前に外した相手も消える。
- `false`: 外れた相手の保存済みの版を残す。新しい版は取らない。保持期間の適用と容量の集計は続き、最新版は残る。起動時の突き合わせで壊れていた版は取り直さずに消える。

## 保存の順序

1. 事前判定: `size` タグ（無ければ不明）で `policy::decide` する。skip なら終わる。
2. NIP-05 検証（`[policy].nip05` が `off` 以外）。`require` で `Verified` でなければ終わる。
3. 取得: `dag/export` の CAR を読み捨てながらバイト数を数え、`policy::fetch_limit`（`max_update_size`・`max_per_site`・`max_per_account` の最小値）を超えたら打ち切る。`SWING_FETCH_IDLE_TIMEOUT` か `SWING_FETCH_TIMEOUT` を超えたら失敗。いずれも state と MFS は変えない。
4. 以降は state のロックの中で行う。作者が Follow Set から外れていれば終わる。
5. 版のパスに CID を置く（既存の項目は先に消す）。失敗したら終わる。
6. `dag/stat`（`offline=true`）の `TotalSize` を実サイズとする。ブロックが欠けていればエラーになるので、5 のパスを消して終わる。`size` タグより大きければ warn を出す。
7. 実サイズで `policy::decide` する。skip なら 5 のパスを消して終わる。
8. 新版を記録し、evict した版を `sites` から消して state を保存してから、evict した版のパスを消す。

パスの削除に失敗しても state はそのままにし、sweep に任せる。

## sweep

state のロックの中で行う。

1. すべてのサイトに `policy::retention_evictions`（`max_per_site`・`keep_versions`・`keep_days`。最新版は残す）を適用する。evict があれば `sites` から消して state を保存し、パスを消す。evict が無ければ state は保存しない。
2. `health::find_garbage` で `<mfs_root>/agent` を `<pubkey hex>/<site>/<created_at>` の 3 階層たどり、state の版に対応しない項目を消す。版が 1 つも残らない `<site>`・`<pubkey hex>` のディレクトリや、想定外の階層のファイルも消す。一覧に失敗したディレクトリの下は消さない。

## 起動時の突き合わせ

state の各版について、`health::check_version` で版のパスの CID（`files/stat`）が記録と一致し、`dag/stat`（`offline=true`）が成功するかを確かめる。パスが無い、CID が違う、ブロックが欠けている版は warn を出して `sites` から消す（次の poll で取り直され、パスは sweep で消える）。`files/stat` 自体が失敗した版は残す。消した版があれば state を保存する。全版の DAG をたどるので、保存量に比例して時間がかかる。

## 並行処理

- `submit`（購読通知・過去分の取得の両方から呼ばれる入口）は、キューへの登録やタスク生成より前に対象判定（Follow Set にいるか）を行い、対象外の pubkey のイベントはその場で捨てる。relay がフィルタを無視して対象外のイベントを大量に送っても、キューやタスクは増えない。「保存の順序」4 の判定は、`submit` から実行までの間に対象から外れた場合に効く。
- 「保存の順序」を同時に実行するタスクは最大 `concurrency` 個。
- 同じ pubkey のタスクは同時に `max_sites_per_account` 個まで。超えたイベントは捨て、次の poll で拾い直す。
- 同じサイト（`pubkey:d`）のタスクは同時に 1 つ。実行中に来たイベントは、実行中・待機中のものより `created_at` が新しいときだけ待機に置き（1 件、上書き）、実行後に同じタスクで続けて処理する。
- 保存の順序の 4〜8、sweep、unfollow、突き合わせは state のロックの中で直列に行う。レプリカ報告の同期どうしは報告用のロックで直列になる（取る順は報告用 → state）。取得中の一時的なディスク使用量は最大で `concurrency` × `fetch_limit`。

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

## ポリシー判定（policy.rs）

`policy::decide` は純粋関数。入力は同サイトの既存版、使用量（他サイトの合計容量、同じ pubkey の他サイトの合計容量とサイト数）、候補（cid, size, created_at）、ポリシー設定、現在時刻。出力は `Decision { store: Option<String>, evict: Vec<String>, reason: String }`。

1. 同じ CID が同サイトに記録済みなら skip（`duplicate_cid`）。
2. 新しいサイトで、同じ pubkey の記録済みのサイトが `max_sites_per_account` 個以上あれば skip（`max_sites_per_account`）。
3. `created_at` が同サイトの最新版以下なら skip（`stale`）。
4. `created_at` が最新版から `min_update_interval` 未満なら skip。
5. `size` が `max_update_size` を超えるなら skip。
6. 同サイト合計が `max_per_site` を超えるなら古い版から evict する。新版単体で超えるなら skip。
7. `keep_versions`（最低 1 に丸める）を超える古い版を evict する。
8. `keep_days` より古い版を evict する。最新版は残す。
9. evict 後、同じ pubkey の全サイト合計が `max_per_account` を超えるなら skip（`max_per_account`）。他のサイトは削らない。
10. evict 後の全サイト合計が `max_total_storage` を超えるなら skip。他のサイトは削らない。

`size` 不明の事前判定では 5 を飛ばし、新版を 0 バイトとして 6〜10 を評価する。

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
