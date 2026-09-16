# mirror-agent（agent.rs, policy.rs, state.rs）

[`architecture.md`](../architecture.md) の一部。MFS のパスと Kubo RPC は [`kubo.md`](kubo.md)、NIP-05 は [`nip05.md`](nip05.md)。

## 全体の流れ

1. relay 群に接続し、state を読み、起動時の突き合わせを行う。
2. `poll_interval` ごとの tick（最初の tick は起動直後）で次を行う。
   1. sweep
   2. Follow Set を決める。決まらなければ警告を出して次の tick を待つ。
   3. unfollow
   4. 対象 pubkey 群のサイトイベント（過去分を含む）を取得して購読し直し、pubkey ごとに、保存済みのサイトすべてと、それ以外のサイトを `created_at` の新しい順に合計 `max_sites_per_account` 件まで、サイトごとの最新版をタスクに投入する。一時的な取得・保存の失敗はここで再試行される。
3. 購読で届いたサイトイベントをタスクに投入する。送信元が今の Follow Set にいなければ warn を出して無視する。購読 ID と kind が一致しない通知は debug ログで捨てる。
4. タスクはサイト単位で「保存の順序」に従って処理する。

relay の切断、Kubo のエラー、不正なイベントはログに出して続ける。relay への再接続と再購読は nostr-sdk が行う。nostr-sdk の通知チャネル（容量 2048）から溢れた分は 2.4 の取り直しで回収される。通知ストリーム自体が終わったらエラーで終了する。Ctrl-C で終了すると実行中のタスクは中断される。

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
2. `<mfs_root>/agent` を `<pubkey hex>/<site>/<created_at>` の 3 階層たどり、state の版に対応しない項目を消す。版が 1 つも残らない `<site>`・`<pubkey hex>` のディレクトリや、想定外の階層のファイルも消す。一覧に失敗したディレクトリの下は消さない。

## 起動時の突き合わせ

state の各版について、版のパスの CID（`files/stat`）が記録と一致し、`dag/stat`（`offline=true`）が成功するかを確かめる。パスが無い、CID が違う、ブロックが欠けている版は warn を出して `sites` から消す（次の poll で取り直され、パスは sweep で消える）。`files/stat` 自体が失敗した版は残す。消した版があれば state を保存する。全版の DAG をたどるので、保存量に比例して時間がかかる。

## 並行処理

- 「保存の順序」を同時に実行するタスクは最大 `concurrency` 個。
- 同じ pubkey のタスクは同時に `max_sites_per_account` 個まで。超えたイベントは捨て、次の poll で拾い直す。
- 同じサイト（`pubkey:d`）のタスクは同時に 1 つ。実行中に来たイベントは、実行中・待機中のものより `created_at` が新しいときだけ待機に置き（1 件、上書き）、実行後に同じタスクで続けて処理する。
- 保存の順序の 4〜8、sweep、unfollow、突き合わせは state のロックの中で直列に行う。取得中の一時的なディスク使用量は最大で `concurrency` × `fetch_limit`。

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
