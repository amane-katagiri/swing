# mirror-agent（agent/, health.rs, policy.rs, state.rs）

[`../architecture.md`](../architecture.md) の一部。子ページはレプリカ報告の [`agent/replicas.md`](agent/replicas.md) とポリシー判定の [`agent/policy.md`](agent/policy.md)。MFS のパスと Kubo RPC は [`mfs.md`](mfs.md)、NIP-05 は [`nip05.md`](nip05.md)、ダッシュボードは [`dashboard.md`](dashboard.md)、内蔵 gateway は [`gateway.md`](gateway.md)、`swing up`（Kubo の起動・監視、agent の再起動）は [`up.md`](up.md)。

## agent/ の構成

| ファイル | 内容 |
|---|---|
| `agent/mod.rs` | `Agent` 構造体、`poll_once`、メンテナンス（sweep・unfollow・突き合わせ） |
| `agent/lifecycle.rs` | `run_until`（ライフサイクル本体）、内蔵 gateway の起動・終了 |
| `agent/follow.rs` | Follow Set の選択・再送と、購読・過去分の取得 |
| `agent/store.rs` | キューイングと直列実行、「保存の順序」、NIP-05 検証 |
| `agent/replicas.rs` | レプリカ報告（[`agent/replicas.md`](agent/replicas.md)） |
| `agent/test_support.rs` | テスト用の偽の relay・NIP-05 検証と設定（`#[cfg(test)]`） |

## 全体の流れ

1. relay 群に接続し、state を読む。`[gateway].listen` が `off` でなければ内蔵 gateway の `TcpListener` を bind する（失敗したら agent 全体がエラーで終了する）。続けて `Agent` を組み立て、起動時の突き合わせを行う。
2. 突き合わせの完了前にシャットダウンが要求されたら、突き合わせを打ち切って即座に shutdown へ進む（relay を切断するだけで終わる。gateway はまだ起動していない）。
3. 突き合わせを終えたら、`dashboard.set_ready(relay, ipfs)` を呼んで relay・Kubo を使う API のエンドポイントを使えるようにし、bind できていれば `gateway::serve` を別タスクで起動する。
4. `poll_interval` ごとの tick（最初の tick は起動直後）、およびダッシュボードでの mirror 変更（`Notify`）で `poll_once`（次を行う）を実行する。
   1. sweep
   2. Follow Set を決める。決まらなければ警告を出して 3 と 4 を飛ばす。
   3. unfollow
   4. 対象 pubkey 群のサイトイベントを購読し直してから過去分を取得し、`nostr::select_latest`（未来ずれの許容は [`nostr.md`](nostr.md#未来ずれの許容nostrmax_future_skew)）でサイトごとの最新版を選び、pubkey ごとに、保存済みのサイトすべてと、それ以外のサイトを `created_at` の新しい順に合計 `max_sites_per_account` 件まで、タスクに投入する。一時的な取得・保存の失敗はここで再試行される（取得の試行は下記「保存の順序」の間引きにより、サイトごとに `min_update_interval` に 1 回まで）。
   5. レプリカ報告の同期（Follow Set が決まらなくても行う）と、他の報告者が自分のサイトについて出した報告の時刻の記録（[`agent/replicas.md`](agent/replicas.md)）を、この順に別タスクで始める。前の回のタスクがまだ動いていれば今回は始めない。
5. 購読で届いたサイトイベントをタスクに投入する（投入前に捨てる条件は下記「並行処理」の `submit`）。購読 ID と kind が一致しない通知は debug ログで捨てる。
6. タスクはサイト単位で「保存の順序」に従って処理する。新版を記録したら、同時実行の枠を返してからレプリカ報告の同期を行う。

relay の切断、Kubo のエラー、不正なイベントはログに出して続ける。relay への再接続と再購読は nostr-sdk が行う。nostr-sdk の通知チャネルから溢れた分は、上の 4 の 4（poll ごとの過去分の取得）で回収される。通知ストリーム自体が終わったらエラーで終了する。

### シグナルと終了

`swing up` はシグナルから作った `CancellationToken`（の子トークン）を `run_until` に渡す（`cancel_on_signal` の詳細は [`up.md#shutdownshutdownrs`](up.md#shutdownshutdownrs)）。`run_until` 自体はシグナルを直接扱わない。起動時の突き合わせと `poll_once` はこのトークンの cancel と競争させ（`race_with_shutdown`）、cancel が先に届いたら処理中の I/O を打ち切って shutdown に進む。

shutdown・poll 中の停止検知・relay 通知ストリーム終了のいずれでループを抜けても（正常終了でもエラーでも）、`run_until` はループを抜けた後に必ず `shutdown_gateway`（gateway の子トークンを cancel してサーバタスクを最大 5 秒待つ。超えたら warn を出して待つのをやめる）→ relay 切断 → `dashboard.set_not_ready()` の順で後始末する。ダッシュボードの API サーバは agent の外で動く（[`up.md`](up.md)）。

ダッシュボードとの共有のしかたは [`dashboard.md#概要`](dashboard.md#概要)。内蔵 gateway は agent の state や Kubo RPC を一切使わず、Kubo の gateway へ透過的にプロキシするだけ（[`gateway.md`](gateway.md)）。

## Follow Set の選び方

relay から取得した版と `state.follow_set` の版を比べて使う方を決める。

- 候補は kind 30000・作者が自分・`d` が `mirror_set`・署名が正しいものだけ。保存済みの版も同じ条件で確かめる（`mirror_set` を変えると古い版は使わない）。
- 新しさは NIP-01 の置き換え規則で比べ、未来ずれの許容を超えて先の版はどちらも「無い」として扱う（[`nostr.md`](nostr.md#未来ずれの許容nostrmax_future_skew)）。

| relay から | 保存済み | 使う版 | state に保存 | relay に再送 |
|---|---|---|---|---|
| 取れた | 無い | 取れた版 | する | しない |
| 取れた（保存済みと同じ `id`） | ある | 保存済み | しない | しない |
| 取れた（保存済みより新しい） | ある | 取れた版 | する | しない |
| 取れた（保存済みより古い） | ある | 保存済み | しない | する |
| 見つからない（先の時刻で捨てた場合を含む） | ある | 保存済み | しない | する |
| 取得に失敗 | ある | 保存済み | しない | しない |
| 見つからない、または失敗 | 無い、または先の時刻で捨てた | 決まらない | — | — |

再送は署名済みのイベントをそのまま全 relay に送り、どこにも受理されなければ warn を出す。NIP-09 で relay から Follow Set が消えても、保存済みの版の再送は続く。

対象 pubkey は決まった Follow Set の `p` のうち先頭 `MAX_FOLLOW_SET_ENTRIES` 件で、超えたら `resubscribe_and_backfill` が warn を出す（[取得と表示の上限](nostr/fetch.md#定数)）。

## unfollow

Follow Set が決まった tick で行う。Follow Set の更新はこれより先に反映する。決まらない tick では何もせず、warn を出す（state に版か検証結果のあるアカウントがあれば、その数と、鍵か `mirror_set` を変えていないかの確認を添える）。state に保存した Follow Set は今の鍵と `mirror_set` のものしか使わないので、鍵か `mirror_set` を変えて起動すると、新しい Follow Set ができるまで以前のアカウントは消えずに残る。

- `remove_on_unfollow = true`: `state.sites` か `state.verifications` にエントリがあり、今の Follow Set にいない pubkey を state から消して保存し、`<mfs_root>/agent/<pubkey hex>` を消す。ただし保存中（下記の保存の順序の 6）の版のパスがその下にあれば、そのアカウントのディレクトリは消さずに info を出して sweep に任せる。保存の間に作者がフォローし直されていれば版はそのまま記録され、外れたままなら 9 で版のパスが消え、残りは保存が終わった後の sweep が消す。state と比べるので、agent の停止中に外した相手や、設定を `true` に変える前に外した相手も消える。
- `false`: 外れた相手の保存済みの版を残す。新しい版は取らない。保持期間の適用と容量の集計は続き、最新版は残る。起動時の突き合わせで壊れていた版は取り直さずに消える。

## 保存の順序

1. 作者が今の Follow Set にいなければ info を出して終わる（キューに入れる前と 6・9 でも同じ確認をする）。同じサイトで同じ CID が以前 4・5・9 で拒否されていれば（下記）、debug を出して終わる。
2. 事前判定: `size` タグ（無い、または読めなければ不明。[`nostr.md#検証`](nostr.md#検証)）で `policy::decide` する。skip なら終わる。同じ state から、5 で使う取得の上限 `policy::fetch_budget` も決める（`policy::fetch_limit`（`max_update_size`・`max_per_site`・`max_per_account` の最小値）、`max_per_account` からそのアカウントのほかのサイトの合計を引いた残り、`max_total_storage` からほかのサイトの合計を引いた残りの最小値。新しい版はどれを evict しても残るので、これを超える内容は 9 で必ず拒否される）。
3. 取得の試行の間引き（下記）に当たれば debug を出して終わる。当たらなければ、ここで試行を記録する。
   続けて NIP-05 検証（[`nip05.md#agent-での適用`](nip05.md#agent-での適用)）。`require` で `Verified` でなければ終わる。間引いたイベントでは NIP-05 の問い合わせも state の保存もしない。
4. 2 で決めた上限が 0（アカウントかノードにもう空きが無い）なら、Kubo に何も問い合わせずに `reason = "no_space_left"` で warn を出して拒否し、終わる。
   ディレクトリ確認: 取得の前に `files/stat /ipfs/<cid>`（ルートのブロックだけを取る）の `Type` を見る。`directory` でなければ `reason = "not_a_directory"` で warn を出して終わる。`files/stat` 自体が失敗したら取得の失敗と同じ扱いで終わる（取り直すのは 3 の試行から `min_update_interval` が過ぎてから）。
5. 取得: `dag/export` の CAR を読み捨てながらバイト数を数え、2 で決めた上限を超えたら打ち切る。`[agent].fetch_idle_timeout` か `[agent].fetch_timeout` を超えたら失敗。いずれも state と MFS は変えない。
6. state のロックを取り、作者が Follow Set から外れていれば終わる。版のパスを「保存中」としてメモリに登録して（9〜10 が終わるまで。sweep と unfollow はこのパスとその親ディレクトリを消さない）、ロックを放す。ロックの中で登録するので、sweep は登録より前に消し終えているか、登録を見て残すかのどちらかになる。
7. state のロックの外で、版のパスに CID を置く（既存の項目は先に消す）。失敗したら終わる。
8. state のロックの外で、`dag/stat`（`offline=true`、タイムアウト 300 秒）の `TotalSize` を実サイズとする。
9. state のロックを取る。8 でブロックが欠けていればエラーになるので、ロックを放して 7 のパスを消して終わる。作者が Follow Set から外れていれば（7 の間に unfollow があった場合を含む）同じく 7 のパスを消して終わる。`size` タグより大きければ warn を出す。実サイズで `policy::decide` する。skip ならロックを放して 7 のパスを消して終わる。
10. 新版を記録し、evict した版を `sites` から消して state を保存し、ロックを放してから evict した版のパスを消す。

4・5 で拒否したとき、取得したブロックは Kubo の GC に任せる。

パスの削除に失敗しても state はそのままにし、sweep に任せる。

取得した内容で拒否した版（4 の `no_space_left` と `not_a_directory`、5 の上限超過、9 の `policy::decide` の skip）:

- その CID をサイトごとにメモリに覚え（`agent::store::Attempts`）、同じ CID のイベントは 1 で終える。1 サイトに複数の CID を覚える。
- 記録はアカウントごとに合計 50 件まで（`REJECTED_PER_ACCOUNT`）。超えたらそのアカウントの最も古い記録を捨てる。
- 保存に成功したらそのサイトの記録を消す。取得の失敗やブロックの欠けなど一時的な失敗は拒否としては覚えない。
- `remove_on_unfollow = true` の unfollow（上記）のとき、今の Follow Set にいないアカウントの記録（拒否と試行の間引き）を消す。

取得の試行の間引き（`Attempts::try_attempt`）: 取得を始めた時刻をサイトごとにメモリに覚えて間引く。保存に成功したらそのサイトの記録を消す。

- 同じサイトは、前の試行から `min_update_interval` の間は取得しない（`fetch_attempt_interval`）。取得の失敗（タイムアウトを含む）や拒否で終わった場合も、保存済みのサイトの更新でも同じ。
- 同じアカウントで、直近 `min_update_interval` の間に試行したほかのサイトが `max_sites_per_account` 件（1〜50 に丸める）あれば取得しない（`fetch_attempts_per_account`）。保存済みで拒否の記録が無いサイトの更新はこの件数では止めない（試行としては数える）。
- `min_update_interval` が 0 なら効かない。
- 試行を記録するときに、そのアカウントの間隔を過ぎた記録（拒否の記録が無いもの）は消す。

これらの記録は `state.json` に書かないので、agent を再起動すると消える。

## sweep

state のロックの中で行う。

1. すべてのサイトに `policy::retention_evictions`（`max_per_site`・`keep_versions`・`keep_days`。最新版は残す）を適用する。evict があれば `sites` から消して state を保存し、パスを消す。evict が無ければ state は保存しない。
2. `health::find_garbage` で `<mfs_root>/agent` を `<pubkey hex>/<site>/<created_at>` の 3 階層たどり、state の版に対応しない項目を消す。版が 1 つも残らない `<site>`・`<pubkey hex>` のディレクトリや、想定外の階層のファイルも消す。一覧に失敗したディレクトリの下は消さない。

## 起動時の突き合わせ

サイトごとに `health::check_site` で確かめる。まず版ごとに、版のパスの CID（`files/stat`）が記録と一致するかを見る。一致した版の CID をまとめて 1 回の `dag/stat`（`offline=true`）に渡し、成功すればその版はすべて完全とする。失敗したときだけ版ごとに `dag/stat` をやり直して、どの版が欠けているかを決める。欠けているとするのは Kubo がブロックを手元に見つけられないと答えたとき（エラーに `ipld: could not find` を含む）だけで、タイムアウトなどそれ以外の失敗は確認できなかった版として扱う。

パスが無い、CID が違う、ブロックが欠けている版は warn を出して `sites` から消す（次の poll で取り直され、パスは sweep で消える）。確認できなかった版（`files/stat` の失敗や、欠けている以外の理由での `dag/stat` の失敗）は残す。消した版があれば state を保存する。サイト単位で DAG をたどるので、保存量に比例して時間がかかる（版どうしで共有しているブロックは 1 回しかたどらない）。

`check_site` は同じ呼び出しでサイトの実容量（まとめた `dag/stat` の `TotalSize`）も返す。突き合わせでは使わず、`swing status` とダッシュボードの表示に使う。state には記録しない。

## 並行処理

- `submit`（購読通知・過去分の取得の両方から呼ばれる入口）は、対象判定（Follow Set にいるか）より前に `nostr::plausible_at` で `created_at` を確かめ、未来ずれの許容（[`nostr.md`](nostr.md#未来ずれの許容nostrmax_future_skew)）を超えて先なら `future_created_at` を理由に warn を出してその場で捨てる。キューにある実行中・待機中のイベントを置き換えることはない。続けて対象判定を行い、今の Follow Set にいない pubkey のイベントも warn を出してその場で捨てる。「保存の順序」1・6 の判定は、`submit` から実行までの間に対象から外れた場合に効く。
- 「保存の順序」を同時に実行するタスクは最大 `concurrency` 個。
- 同じ pubkey のタスクは同時に `max_sites_per_account` 個まで。超えたイベントは捨て、次の poll で拾い直す。
- 同じサイト（`pubkey:d`）のタスクは同時に 1 つ。実行中に来たイベントは、実行中・待機中のものより `created_at` が新しいときだけ待機に置き（1 件、上書き）、実行後に同じタスクで続けて処理する。
- 保存の順序の 6 と 9〜10 の state の更新、sweep、unfollow、突き合わせは state のロックの中で直列に行う。保存の順序の Kubo への書き込み（7 と、9・10 のパスの削除）はロックの外で行う。レプリカ報告の同期どうしは報告用のロックで直列になる（取る順は報告用 → state）。取得中の一時的なディスク使用量は最大で `concurrency` × `fetch_limit`。

## レプリカ報告

レプリカ報告の同期と、他の報告者からの報告の時刻の記録は [`agent/replicas.md`](agent/replicas.md)。

## ポリシー判定（policy.rs）

`policy::decide` の判定の順は [`agent/policy.md`](agent/policy.md)。

## state.json（state.rs）

`[agent].state_dir` 直下。`auth::write_private_file` で書く（手順とパーミッションは [`dashboard/security.md#トークン`](dashboard/security.md#トークン)）。

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
