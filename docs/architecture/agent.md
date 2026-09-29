# mirror-agent（agent/, health.rs, policy.rs, state.rs）

[`../architecture.md`](../architecture.md) の一部。MFS のパスと Kubo RPC は [`kubo.md`](kubo.md)、NIP-05 は [`nip05.md`](nip05.md)、ダッシュボードは [`dashboard.md`](dashboard.md)、内蔵 gateway は [`gateway.md`](gateway.md)、`swing up`（Kubo の起動・監視、agent の再起動）は [`up.md`](up.md)。

## agent/ の構成

| ファイル | 内容 |
|---|---|
| `agent/mod.rs` | `Agent` 構造体の定義、`new`、`poll_once`、メンテナンス系（`sweep`・`collect_garbage`・`reconcile`・`remove_unfollowed`）、state 保存の共通ヘルパー（`save`） |
| `agent/lifecycle.rs` | `run_until`（プロセスのライフサイクル本体。`CancellationToken`・`swing up` から渡される共有の `Arc<dashboard::AppState>`・`Arc<Notify>` を受け取る。`AppState.activity` を `Agent` に渡して共有する）、内蔵 gateway の bind（`bind_gateway`）とタスクの起動・終了、購読の通知からのサイトイベントの取り出し（`site_event_of`）、ダッシュボードへの準備完了・未準備の通知（`AppState::set_ready`/`set_not_ready`） |
| `agent/follow.rs` | `refresh_follow_set`（Follow Set の取得・保存・再送は `choose_and_apply_follow_set`、対象の切り替え・サイトイベントの購読・取得は `resubscribe_and_backfill` に分かれた薄い呼び出し元）、`limit_sites_per_account` |
| `agent/store.rs` | `Agent::submit`/`drain`（キューイングと直列実行）、`apply_site_event`（「保存の順序」の中核。事前判定の `worth_fetching`・取得とディレクトリ判定の `fetch_directory`・MFS への保存と state への記録の `store_fetched` を順に呼ぶ）、NIP-05 検証、`decide`/`version_infos` |
| `agent/replicas.rs` | レプリカ報告の差分計算・送信（`SentReport`・`ReportBook`・`Held`・`reports_to_send`・`held`・`load_sent_reports`・`sync_reports`）、他の報告者からの報告の時刻の記録（`record_replica_reports`） |
| `agent/test_support.rs` | ユニットテスト共通のフィクスチャ（`Fixture`・`FakeNip05`・`FakeRelay`、`test_config`）。`#[cfg(test)]`。`FakeKubo` は `src/test_support.rs` のものを `pub(super) use` で再公開する |

外部からは `swing::agent::run_until` だけを公開する（`swing up` が Kubo・agent を協調させて起動・再起動するために使う。[`up.md`](up.md)）。テストは対応するモジュールの `#[cfg(test)] mod tests` に置く。

## 全体の流れ

1. relay 群に接続し、state を読む。`[gateway].listen` が `off` でなければ内蔵 gateway の `TcpListener` を bind する（失敗したら agent 全体がエラーで終了する）。続けて `Agent` を組み立て、起動時の突き合わせを行う。
2. 突き合わせの完了前にシャットダウンが要求されたら、突き合わせを打ち切って即座に shutdown へ進む（relay を切断するだけで終わる。gateway はまだ起動していない）。
3. 突き合わせを終えたら、`dashboard.set_ready(relay, ipfs)` を呼んで relay・Kubo を使う API のエンドポイントを使えるようにし、bind できていれば `gateway::serve` を別タスクで起動する。
4. `poll_interval` ごとの tick（最初の tick は起動直後）、およびダッシュボードでの mirror 変更（`Notify`）で `poll_once`（次を行う）を実行する。
   1. sweep
   2. Follow Set を決める。決まらなければ警告を出して 3 と 4 を飛ばす。
   3. unfollow
   4. 対象 pubkey 群のサイトイベントを購読し直してから過去分を取得し、`nostr::select_latest`（未来ずれの許容は [`nostr.md`](nostr.md#未来ずれの許容nostrmax_future_skew)）でサイトごとの最新版を選び、pubkey ごとに、保存済みのサイトすべてと、それ以外のサイトを `created_at` の新しい順に合計 `max_sites_per_account` 件まで、タスクに投入する。一時的な取得・保存の失敗はここで再試行される。
   5. レプリカ報告の同期（Follow Set が決まらなくても行う）
   6. 他の報告者が自分のサイトについて出した報告の時刻を記録する（下記「レプリカ報告」）
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
- 新しさの比べ方（NIP-01 の置き換え規則）は [`nostr.md`](nostr.md#未来ずれの許容nostrmax_future_skew)。
- `nostr::choose_follow_set` は、relay から取得した版・保存済みの版のどちらも、未来ずれの許容（[`nostr.md`](nostr.md#未来ずれの許容nostrmax_future_skew)）を超えて先なら「無い」として扱ってから下の表の判定に入る。`RelayClient::fetch_follow_set` / `fetch_follow_sets` 自身も取得直後に同じ基準でふるい落とすので、relay から先の時刻の版しか取れなかった場合は「見つからない」と同じ扱いになる。

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

対象 pubkey は決まった Follow Set の `p` のうち先頭 `MAX_FOLLOW_SET_ENTRIES` 件で、超えたら `resubscribe_and_backfill` が warn を出す（[取得と表示の上限](nostr.md#取得と表示の上限nostrbudget)）。

## unfollow

Follow Set が決まった tick で行う。Follow Set の更新はこれより先に反映する。決まらない tick では何もせず、warn を出す（state に版か検証結果のあるアカウントがあれば、その数と、鍵か `mirror_set` を変えていないかの確認を添える）。state に保存した Follow Set は今の鍵と `mirror_set` のものしか使わないので、鍵か `mirror_set` を変えて起動すると、新しい Follow Set ができるまで以前のアカウントは消えずに残る。

- `remove_on_unfollow = true`: `state.sites` か `state.verifications` にエントリがあり、今の Follow Set にいない pubkey を state から消して保存し、`<mfs_root>/agent/<pubkey hex>` を消す。state と比べるので、agent の停止中に外した相手や、設定を `true` に変える前に外した相手も消える。
- `false`: 外れた相手の保存済みの版を残す。新しい版は取らない。保持期間の適用と容量の集計は続き、最新版は残る。起動時の突き合わせで壊れていた版は取り直さずに消える。

## 保存の順序

1. 作者が今の Follow Set にいなければ warn を出して終わる。同じサイトで同じ CID が以前 4・5・9 で拒否されていれば（下記）、debug を出して終わる。
2. 事前判定: `size` タグ（無い、または `u64` としてパースできなければ不明）で `policy::decide` する。skip なら終わる。
3. NIP-05 検証（`[policy].nip05` が `off` 以外）。`require` で `Verified` でなければ終わる。
   その後、取得の試行の間引き（下記）に当たれば debug を出して終わる。当たらなければ、ここで試行を記録する。
4. 取得: `dag/export` の CAR を読み捨てながらバイト数を数え、`policy::fetch_limit`（`max_update_size`・`max_per_site`・`max_per_account` の最小値）を超えたら打ち切る。`[agent].fetch_idle_timeout` か `[agent].fetch_timeout` を超えたら失敗。いずれも state と MFS は変えない。
5. ディレクトリ確認: `files/stat /ipfs/<cid>` の `Type` を見る。`directory` でなければ `reason = "not_a_directory"` で warn を出して終わる（MFS にはまだ何も置いていないので消すものは無く、取得したブロックは Kubo の GC に任せる）。`files/stat` 自体が失敗したら取得の失敗と同じ扱いで終わる（次の poll で取り直す）。
6. 版のパスを「保存中」としてメモリに登録する（8〜10 が終わるまで。sweep はこのパスとその親ディレクトリを消さない）。state のロックを取り、作者が Follow Set から外れていれば終わる。
7. 版のパスに CID を置く（既存の項目は先に消す）。失敗したら終わる。ここで state のロックを放す。
8. `dag/stat`（`offline=true`）の `TotalSize` を実サイズとする。DAG をたどるので時間がかかる（タイムアウト 300 秒）ため、state のロックの外で行う。7 で MFS に置いてからたどるので、その間に Kubo の GC がブロックを消すことはない。
9. state のロックを取り直す。8 でブロックが欠けていればエラーになるので、7 のパスを消して終わる。作者が Follow Set から外れていれば 7 のパスを消して終わる。`size` タグより大きければ warn を出す。実サイズで `policy::decide` する。skip なら 7 のパスを消して終わる。
10. 新版を記録し、evict した版を `sites` から消して state を保存してから、evict した版のパスを消す。

パスの削除に失敗しても state はそのままにし、sweep に任せる。

取得した内容で拒否した版（4 の上限超過、5 の `not_a_directory`、9 の `policy::decide` の skip）は、その CID をサイトごとにメモリに覚え（`agent::store::Attempts`。1 サイトに複数の CID を覚える。アカウントごとに合計 50 件まで（`REJECTED_PER_ACCOUNT`）で、超えたらそのアカウントの最も古い記録を捨てる）、同じ CID のイベントは 1 で終える。上限まで取得し直すのを poll ごとに繰り返さないためである（2 つの大きな CID を交互に出されても、どちらも覚えている）。保存に成功したらそのサイトの記録を消す。取得の失敗やブロックの欠けなど一時的な失敗は拒否としては覚えない。

取得の試行の間引き（`Attempts::try_attempt`）: `policy::decide` の `min_update_interval` は保存済みの版の `stored_at` でしか効かないので、まだ保存していないサイトと、拒否した CID を覚えているサイトについては、取得を始めた時刻もメモリに覚えて間引く。

- 同じサイトは、前の試行から `min_update_interval` の間は取得しない（`fetch_attempt_interval`）。取得の失敗で終わった場合も同じで、次の試行は間隔が空いてからになる。
- 同じアカウントで、直近 `min_update_interval` の間に試行したほかのサイトが `max_sites_per_account` 件（最大 50 件）あれば取得しない（`fetch_attempts_per_account`）。`d` を変えて新しいサイトを次々に出されても、取得の回数はアカウントごとにこの数で頭打ちになる。
- 保存済みで拒否の記録が無いサイトには効かない（`min_update_interval` は `policy::decide` が見る）。`min_update_interval` が 0 なら効かない。
- 試行を記録するときに、そのアカウントの間隔を過ぎた記録（拒否の記録が無いもの）は消す。

これらの記録は `state.json` に書かないので、agent を再起動すると消える（容量が空いたあとなどに取り直させたいときは再起動する）。

## sweep

state のロックの中で行う。

1. すべてのサイトに `policy::retention_evictions`（`max_per_site`・`keep_versions`・`keep_days`。最新版は残す）を適用する。evict があれば `sites` から消して state を保存し、パスを消す。evict が無ければ state は保存しない。
2. `health::find_garbage` で `<mfs_root>/agent` を `<pubkey hex>/<site>/<created_at>` の 3 階層たどり、state の版に対応しない項目を消す。版が 1 つも残らない `<site>`・`<pubkey hex>` のディレクトリや、想定外の階層のファイルも消す。一覧に失敗したディレクトリの下は消さない。

## 起動時の突き合わせ

サイトごとに `health::check_site` で確かめる。まず版ごとに、版のパスの CID（`files/stat`）が記録と一致するかを見る。一致した版の CID をまとめて 1 回の `dag/stat`（`offline=true`）に渡し、成功すればその版はすべて完全とする（1 つでもブロックが欠けていれば失敗するので、成功は全版の完全性を意味する）。失敗したときだけ版ごとに `dag/stat` をやり直して、どの版が欠けているかを決める。

パスが無い、CID が違う、ブロックが欠けている版は warn を出して `sites` から消す（次の poll で取り直され、パスは sweep で消える）。`files/stat` 自体が失敗した版は残す。消した版があれば state を保存する。サイト単位で DAG をたどるので、保存量に比例して時間がかかる（版どうしで共有しているブロックは 1 回しかたどらない）。

`check_site` は同じ呼び出しでサイトの実容量（まとめた `dag/stat` の `TotalSize`）も返す。突き合わせでは使わず、`swing status` とダッシュボードの表示に使う。state には記録しない。

## 並行処理

- `submit`（購読通知・過去分の取得の両方から呼ばれる入口）は、対象判定（Follow Set にいるか）より前に `nostr::plausible_at` で `created_at` を確かめ、未来ずれの許容（[`nostr.md`](nostr.md#未来ずれの許容nostrmax_future_skew)）を超えて先なら `future_created_at` を理由に warn を出してその場で捨てる。キューにある実行中・待機中のイベントを置き換えることはない。続けて対象判定を行い、今の Follow Set にいない pubkey のイベントも warn を出してその場で捨てる。relay がフィルタを無視して対象外のイベントを大量に送っても、キューやタスクは増えない。「保存の順序」1・6 の判定は、`submit` から実行までの間に対象から外れた場合に効く。
- 「保存の順序」を同時に実行するタスクは最大 `concurrency` 個。
- 同じ pubkey のタスクは同時に `max_sites_per_account` 個まで。超えたイベントは捨て、次の poll で拾い直す。
- 同じサイト（`pubkey:d`）のタスクは同時に 1 つ。実行中に来たイベントは、実行中・待機中のものより `created_at` が新しいときだけ待機に置き（1 件、上書き）、実行後に同じタスクで続けて処理する。
- 保存の順序の 6〜7 と 9〜10、sweep、unfollow、突き合わせは state のロックの中で直列に行う。レプリカ報告の同期どうしは報告用のロックで直列になる（取る順は報告用 → state）。取得中の一時的なディスク使用量は最大で `concurrency` × `fetch_limit`。

## レプリカ報告

イベント形式は [`../protocol.md`](../protocol.md#8-レプリカ報告)。同期は報告用のロックの中で直列に行う。state のロックは保存している CID を読む間だけ取る。

保存している CID（サイト `<pubkey hex>:<d>` ごと）:

- `state.sites` の各版の CID。
- `<mfs_root>/publish/<自分の pubkey hex>/` の下のディレクトリ名を `mfs::site_from_name` で `d` に戻し（エンコードし直して同じ名前にならないもの、`d` の条件を満たさないものは無視）、その下の名前が整数の項目の CID（`files/ls` の `Hash`）。同じサイトが `state.sites` にもあれば合わせる。同じ Kubo で `swing publish` した自分のサイトだけが対象で、別の Kubo で publish したサイトは報告しない。
- `publish/<自分>/` の一覧に失敗したら自分が作者のサイトすべてを、`publish/<自分>/<site>/` の一覧に失敗したらそのサイトを「不明」とし、今回は送らない。

送信済みの記録はメモリにだけ持つ（サイトごとに `cid` の集合と `created_at`）。まだ読めていなければ、同期のたびに relay から自分の報告（`replica_event_kind`、作者が自分）を取得し、`d` ごとの最新を記録に入れる（記録にある方が新しければそのまま）。取得に失敗したら warn を出し、送信は続ける。

送るもの:

| 保存している CID | 記録 | 送る内容 |
|---|---|---|
| ある | 無い、または `cid` が違う | 保存している CID |
| ある | 同じ `cid` で、`created_at` から `report_ttl / 2` 以上たった | 保存している CID |
| 無い | `cid` が 1 つ以上 | `cid` 無し（取り下げ） |
| 不明 | — | 送らない |

- `created_at` は現在時刻。記録の `created_at` 以下になるときは記録の `created_at + 1` にする。`expiration` は `created_at + report_ttl`（足し算は飽和させる）。
- 全 relay に送り、どこかに受理されたら記録を更新する。受理されなければ warn を出し、次の同期で送り直す。
- 署名（NIP-46 の署名アプリへのリクエストを含む）か送信がエラーになったら warn を出して、その回の残りの報告は送らずに打ち切る。残りは次の同期で送り直す（署名アプリがオフラインのときの扱いは [`signer.md#署名アプリがオフラインのとき`](signer.md#署名アプリがオフラインのとき)）。
- 取り下げた記録は `cid` 無しで残り、出し直さない。

ダッシュボードの `/api/activity`（[`dashboard/http-api.md`](dashboard/http-api.md#get-apiactivity)）のために、次の 2 つをメモリ上の `activity::Activity`（ダッシュボードの `AppState` と共有する。値は最大値を取るだけで下がらず、`state.json` には書かない）に記録する。

- publish の時刻: 保存している CID を集めるときに一覧した `publish/<自分>/<site>/` の整数名（CID が空でないもの）の最大値。整数名はサイトイベントの `created_at` なので、同じ Kubo で `swing publish` した分も次の同期で拾う。一覧がすべて成功したら、版が無くても「確かめた」印を付ける（`/api/activity` で `0` になる）。
- 他の報告者の報告の時刻: CID は誰でも見られるので、自分が選んだ報告者の報告だけを数える。信頼できる報告者は `state.json` の `follow_set`（自分の Follow Set）から `replicas::Chosen::from_own` で作り、`Chosen::trusted_reporters` から自分を除いたもの（[「レプリカ報告の信頼度」](nostr.md#レプリカ報告の信頼度replicastier)の `Chosen`）。poll ごとに relay から `replica_event_kind` で `#p` が自分、作者がその報告者（`AUTHORS_PER_FILTER` 人ずつ）の報告を、前回までに記録した最大値を `since` に付けて取得する（まだ無ければ `since` 無し。`limit` は組の人数 × `MAX_SITES_PER_AUTHOR_LISTED` × 2）。報告者が `replicas::tier_of` で `Chosen`（自分は `Author` なので外れる）、`p` タグに自分がある、`parse_replica_report` でパースでき作者が自分、`ReplicaReport::counts_at(now)` が true（未来ずれの許容・`MAX_REPORT_AGE`・`expiration`）、`created_at` が今以前、`cid` タグのどれかが自分のそのサイトで保存している CID（直前のレプリカ報告の同期で集めたもの。一覧に失敗したサイトは前回の値を使う）と一致する、のすべてを満たすものの `created_at` の最大値を記録する。`cid` 無し（取り下げ）の報告は数えない。`created_at` が今より先の報告は、未来ずれの許容内でも記録せず、`since` 以降なので時刻が追いついた後の poll で取り直して記録する。記録する値は今の時刻を超えないので、先の時刻を入れた報告 1 件で以後の報告が `since` から外れることはない。選ばれていない報告者の報告は数えないので、その `created_at` で記録（と次の `since`）が進むこともない。信頼できる報告者がいなければ取得はせず、成功として扱う。取得に成功したら、数える報告が無くても「確かめた」印を付ける（`/api/activity` で `0` になる）。`since` は記録した最大値が 0 なら付けない。取得に失敗したら warn を出し、値はそのまま。購読は増やさない。

受信側で報告を数える規則（`replicas::collect_reports` / `ReplicaReport::counts_at`）は [「レプリカ報告の信頼度」](nostr.md#レプリカ報告の信頼度replicastier)。受信側は `created_at` から `nostr::MAX_REPORT_AGE`（7 日）を過ぎた報告を数えない（[取得と表示の上限](nostr.md#取得と表示の上限nostrbudget)）ので、`report_ttl` はそれ以下でないと設定の検証でエラーになる（[`../architecture.md`](../architecture.md#設定と環境変数)）。出し直しは `report_ttl / 2` ごとなので、上限の 7 日でも最新の報告は常に 3.5 日以内に出ている。

## ポリシー判定（policy.rs）

`policy::decide` は純粋関数。入力は同サイトの既存版、使用量（他サイトの合計容量、同じ pubkey の他サイトの合計容量とサイト数）、候補（cid, size, created_at）、ポリシー設定、現在時刻。出力は `Decision { store: Option<String>, evict: Vec<String>, reason: String }`。

1. `nostr::plausible_at` が false（`created_at` が未来ずれの許容を超えて先。[`nostr.md`](nostr.md#未来ずれの許容nostrmax_future_skew)）なら skip（`future_created_at`）。
2. 同じ CID が同サイトに記録済みなら skip（`duplicate_cid`）。
3. 新しいサイトで、同じ pubkey の記録済みのサイトが `max_sites_per_account` 個以上あれば skip（`max_sites_per_account`）。
4. `created_at` が同サイトの最新版以下なら skip（`stale`）。
5. 現在時刻が同サイトの最大の `stored_at` から `min_update_interval` 未満なら skip（`min_update_interval`）。
6. `size` が `max_update_size` を超えるなら skip（`max_update_size`）。
7. 同サイト合計が `max_per_site` を超えるなら古い版から evict する。新版単体で超えるなら skip（`max_per_site_exceeded_alone`）。
8. `keep_versions`（最低 1 に丸める）を超える古い版を evict する。
9. `keep_days` より古い版を evict する。最新版は残す。`keep_days = 0` なら何もしない。
10. evict 後、同じ pubkey の全サイト合計が `max_per_account` を超えるなら skip（`max_per_account`）。他のサイトは削らない。
11. evict 後の全サイト合計が `max_total_storage` を超えるなら skip（`max_total_storage`）。他のサイトは削らない。

`size` 不明の事前判定では 6 を飛ばし、新版を 0 バイトとして 7〜11 を評価する。

間隔（5）は `stored_at` で測る。見送った版は次の poll で再評価される。

## state.json（state.rs）

`[agent].state_dir` 直下。ダッシュボードのトークンや `remote-signer.json` と同じ `auth::write_private_file` で、一時ファイル（unix では `0o600`）に書いて `fsync` してから rename する。`state_dir` が無ければ `auth::create_private_dir_all` で作る（unix では新しく作るディレクトリだけ `0o700`。既にあるディレクトリのモードは変えない）。

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
