# mirror-agent（agent/, policy.rs, state.rs）

[`../architecture.md`](../architecture.md) の一部。MFS のパスと Kubo RPC は [`mfs.md`](mfs.md)、NIP-05 は [`nip05.md`](nip05.md)、ダッシュボードは [`dashboard.md`](dashboard.md)、内蔵 gateway は [`gateway.md`](gateway.md)、`swing up`（Kubo の起動・監視、agent の再起動）は [`up.md`](up.md)。

子ページ:

- [`agent/store.md`](agent/store.md): サイトの保存の順序と、拒否の記録・取得の試行の間引き
- [`agent/policy.md`](agent/policy.md): ポリシー判定と取得の上限
- [`agent/replicas.md`](agent/replicas.md): レプリカ報告の送信と、他の報告者の報告の時刻の記録
- [`agent/resend.md`](agent/resend.md): 保存した版のサイトイベントの再送
- [`agent/state.md`](agent/state.md): `state.json` の形式と各項目の扱い
- [`agent/anchors.md`](agent/anchors.md): アンカーの広告（kind 35982）と、選んだアンカーへの Kubo の peering

## agent/ の構成

| ファイル | 内容 |
|---|---|
| `agent/mod.rs` | `Agent` 構造体、`poll_once`、メンテナンス（sweep・unfollow・突き合わせ） |
| `agent/lifecycle.rs` | `run_until`（ライフサイクル本体）、内蔵 gateway の起動・終了 |
| `agent/follow.rs` | Follow Set の選択・再送と、購読・過去分の取得 |
| `agent/resend.rs` | 保存した版のサイトイベントの再送（[`agent/resend.md`](agent/resend.md)） |
| `agent/store.rs` | キューイングと直列実行、保存の順序（[`agent/store.md`](agent/store.md)）、NIP-05 検証 |
| `agent/replicas.rs` | レプリカ報告（[`agent/replicas.md`](agent/replicas.md)） |
| `agent/anchors.rs` | アンカーの広告と選び直し（[`agent/anchors.md`](agent/anchors.md)） |
| `agent/test_support.rs` | テスト用の偽の relay・NIP-05 検証と設定（`#[cfg(test)]`） |

## 全体の流れ

1. relay 群に接続し、state を読む。`[gateway].listen` が `off` でなければ内蔵 gateway の `TcpListener` を bind する（失敗したら agent 全体がエラーで終了する）。続けて `Agent` を組み立て、起動時の突き合わせを行う。
2. 突き合わせの完了前にシャットダウンが要求されたら、突き合わせを打ち切って即座に shutdown へ進む（relay を切断するだけで終わる。gateway はまだ起動していない）。
3. 突き合わせを終えたら、`dashboard.set_ready(relay, ipfs)` を呼んで relay・Kubo を使う API のエンドポイントを使えるようにし、bind できていれば `gateway::serve` を別タスクで起動する。
4. `poll_interval` ごとの tick（最初の tick は起動直後）、およびダッシュボードでの mirror 変更（`Notify`）で `poll_once`（次を行う）を実行する。
   1. sweep
   2. Follow Set を決める。決まらなければ警告を出して 3 と 4 を飛ばす。
   3. unfollow
   4. 対象 pubkey 群のサイトイベントを購読し直してから過去分を取得し、`nostr::select_latest`（未来ずれの許容は [`nostr.md`](nostr.md#未来ずれの許容nostrmax_future_skew)）でサイトごとの最新版を選び、pubkey ごとに、保存済みのサイトすべてと、それ以外のサイトを `created_at` の新しい順に合計 `max_sites_per_account` 件まで、タスクに投入する。一時的な取得・保存の失敗はここで再試行される（[取得の試行の間引き](agent/store.md#取得の試行の間引き)により、サイトごとに `min_update_interval` に 1 回まで。[保存の順序](agent/store.md#保存の順序)で試行を取り消す失敗は間引かず、毎 poll）。投入の前に、選んだ最新版のうち state のそのサイトの最新の版と `cid` が一致するものを、その版のサイトイベントとして入れ、変わったら state を保存する（`State::fill_latest_event`。条件は [`agent/state.md`](agent/state.md#sitesevent)）。同じ `cid` を作者が後で署名し直した版は `policy::decide` で `duplicate_cid` になり版としては記録しないが、ここで保存している版のイベントを置き換える。続けて、この poll の対象 pubkey を[再送の確認](agent/resend.md#確認の間隔とタスク)の対象として 5 のタスクに渡す（過去分の取得に失敗しても渡す）。
   5. レプリカ報告の同期（Follow Set が決まらなくても行う）、他の報告者が自分のサイトについて出した報告の時刻の記録（[`agent/replicas.md`](agent/replicas.md)）、間隔が来ていれば再送の確認を、この順に 1 つの別タスク（報告の回）で始める。poll はその終わりを待たない。前の回のタスクがまだ動いていれば今回は始めない。
   6. アンカーの選び直しと Kubo の peering への反映、選んだアンカーへの接続の確認（[`agent/anchors.md`](agent/anchors.md#接続する側)）を、報告の回とは別のタスクで始める。poll はその終わりを待たない。前の選び直しのタスクがまだ動いていれば今回は始めない。
5. `anchor::CHECK_INTERVAL`（60 秒）ごとの tick で、AutoNAT を読んでアンカーの広告を確かめるタスクを始める（前のタスクがまだ動いていれば始めない。[`agent/anchors.md`](agent/anchors.md#広告を出す側)）。
6. 購読で届いたサイトイベントをタスクに投入する（投入前に捨てる条件は下記「並行処理」の `submit`）。購読 ID と kind が一致しない通知は debug、パースできないサイトイベントは warn を出して捨てる。
7. タスクはサイト単位で[保存の順序](agent/store.md#保存の順序)に従って処理する。

relay の切断、Kubo のエラー、不正なイベントはログに出して続ける。relay への再接続と再購読は nostr-sdk が行う。nostr-sdk の通知チャネルから溢れた分は、上の 4 の 4（poll ごとの過去分の取得）で回収される。通知ストリーム自体が終わったらエラーで終了する。

### シグナルと終了

`swing up` はシグナルから作った `CancellationToken`（の子トークン）を `run_until` に渡す（`cancel_on_signal` の詳細は [`up.md#shutdownshutdownrs`](up.md#shutdownshutdownrs)）。`run_until` 自体はシグナルを直接扱わない。起動時の突き合わせと `poll_once` はこのトークンの cancel と競争させ（`race_with_shutdown`）、cancel が先に届いたら処理中の I/O を打ち切って shutdown に進む。

shutdown・poll 中の停止検知・relay 通知ストリーム終了のいずれでループを抜けても（正常終了でもエラーでも）、`run_until` はループを抜けた後に必ず `shutdown_gateway`（gateway の子トークンを cancel してサーバタスクを最大 5 秒待つ。超えたら warn を出して待つのをやめる）→ relay 切断 → `dashboard.set_not_ready()` の順で後始末する。サイトのタスク・レプリカ報告の回・保存した版のサイトイベントの再送の確認・アンカーの選び直し・アンカーの広告の確認は 1 つの `JoinSet` で動かし、`run_until` が返るときに待たずに中断する。ダッシュボードの API サーバは agent の外で動く（[`up.md`](up.md)）。

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

- `remove_on_unfollow = true`: `state.sites` か `state.verifications` にエントリがあり、今の Follow Set にいない pubkey を state から消して保存し、`<mfs_root>/agent/<pubkey hex>` を消す。ただし保存中（[保存の順序](agent/store.md#保存の順序)の 6）の版のパスがその下にあれば、そのアカウントのディレクトリは消さずに info を出して sweep に任せる（作者が外れたままなら保存の順序の 9 で版のパスが消える）。メモリの拒否と試行の記録も、今の Follow Set にいないアカウントの分を消す。前の Follow Set ではなく state と比べるので、agent の停止中に外した相手や、設定を `true` に変える前に外した相手も消える。
- `false`: 外れた相手の保存済みの版を残す。新しい版は取らない。保持期間の適用と容量の集計は続き、最新版は残る。起動時の突き合わせで壊れていた版は取り直さずに消える。

## sweep

1. state のロックの中で、すべてのサイトに `policy::retention_evictions`（`max_per_site`・`keep_versions`・`keep_days`。最新版は残す）を適用する。evict があれば `sites` から消して state を保存し、パスを消す。evict が無ければ state は保存しない。終わったら state を複製してロックを放す。
2. ロックの外で、複製した state を使って `health::find_garbage` で state の版に対応しない MFS の項目を集める（集め方は [`health.md`](health.md#state-に無いパスhealthfind_garbage)）。一覧に失敗したディレクトリの下は消さない。
3. 項目があれば state のロックを取り直し、今の state の版のパスか保存中のパスにあたる項目、またはそれらを下に持つディレクトリの項目を除いて消す。[保存の順序](agent/store.md#保存の順序)の 6 の登録と行き違わないよう、消し終えるまでロックを持つ。

## 起動時の突き合わせ

サイトごとに `health::check_site` で版を確かめる（判定は [`health.md`](health.md#版の検査healthcheck_site)）。壊れた版（パスが無い、CID が違う、ブロックが欠けている）は warn を出して `sites` から消す（次の poll で取り直され、パスは sweep で消える）。確認できなかった版は残す。消した版があれば state を保存する。サイト単位で DAG をたどるので、保存量に比例して時間がかかる。同じ呼び出しで返るサイトの実容量は使わない。

## 並行処理

- `submit`（購読通知・過去分の取得の両方から呼ばれる入口）は、対象判定（Follow Set にいるか）より前に `nostr::plausible_at` で `created_at` を確かめ、未来ずれの許容（[`nostr.md`](nostr.md#未来ずれの許容nostrmax_future_skew)）を超えて先なら `future_created_at` を理由に warn を出してその場で捨てる。キューにある実行中・待機中のイベントを置き換えることはない。続けて対象判定を行い、今の Follow Set にいない pubkey のイベントは info を出してその場で捨てる。[保存の順序](agent/store.md#保存の順序)の 1・6・9 の判定は、`submit` から実行までの間に対象から外れた場合に効く。
- 保存の順序を同時に実行するタスクは最大 `concurrency` 個。
- 同じ pubkey のタスクは同時に `max_sites_per_account` 個まで。超えたイベントは捨て、次の poll で拾い直す。
- 同じサイト（`pubkey:d`）のタスクは同時に 1 つ。実行中に来たイベントは、実行中・待機中のものより `created_at` が新しいときだけ待機に置き（1 件、上書き）、実行後に同じタスクで続けて処理する。
- 保存の順序の 6 と 9〜10 の state の更新、sweep の 1 と 3、unfollow、突き合わせは state のロックの中で直列に行う。sweep の 2（MFS の一覧）はロックの外で行う。保存の順序の Kubo への書き込み（7 と、9・10 のパスの削除）はロックの外で行う。レプリカ報告の同期どうしは報告用のロックで直列になる（取る順は報告用 → state）。取得中の一時的なディスク使用量は最大で `concurrency` × `fetch_limit`。
