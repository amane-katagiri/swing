# サイトの保存（`agent/store.rs`）

[`../agent.md`](../agent.md) の子ページ。購読と過去分の取得で届いたサイトイベントを、サイト単位で次の順に処理する。キューと同時実行は [`../agent.md#並行処理`](../agent.md#並行処理)、判定は [`policy.md`](policy.md)。

## 保存の順序

1. 作者が今の Follow Set にいなければ info を出して終わる（キューに入れる前と 6・9 でも同じ確認をする）。続けて、そのサイトが state の `sites` にあり、イベントの `created_at` が未来ずれの許容の内で、署名が正しく、state のそのサイトの最新の版より新しければ（`State::note_newer_version`。最新の版に `event` があればそれと NIP-01 の置き換え規則で、無ければ版の `created_at` と比べる）、この後で保存しなくても state に記録する。`cid` が最新の版と同じなら、そのイベントを最新の版のイベントにし（`State::fill_latest_event`）、違えば最新の版の取り下げとして記録する（`State::record_withdrawal`）。条件と記録の形は [`state.md`](state.md)。変わったときだけ state を保存する。続けて、同じサイトで同じ CID を拒否した記録（下記）があれば debug を出して終わる。
2. 事前判定: `size` タグ（無い、または読めなければ不明。[`../nostr.md#検証`](../nostr.md#検証)）で `policy::decide` する。skip なら info を出して終わる。同じ state から、4・5 で使う取得の上限 `policy::fetch_budget`（[`policy.md#取得の上限`](policy.md#取得の上限)）も決める。
3. 取得の試行の間引き（下記）に当たれば debug を出して終わる。当たらなければ、ここで試行を記録する。続けて NIP-05 検証（[`../nip05.md#agent-での適用`](../nip05.md#agent-での適用)）。`require` で `Verified` でなければ終わる。間引いたイベントでは NIP-05 の問い合わせも state の保存もしない。
4. 2 で決めた上限が 0（アカウントかノードにもう空きが無い）なら、Kubo に何も問い合わせずに `reason = "no_space_left"` で warn を出して拒否し、終わる。続けて `files/stat /ipfs/<cid>`（ルートのブロックだけを取る）の `Type` を見て、`directory` でなければ `reason = "not_a_directory"` で warn を出して拒否し、終わる。`files/stat` 自体が失敗したら取得の失敗と同じ扱いで終わる。
5. 取得: `dag/export` の CAR を読み捨てながらバイト数を数え、2 で決めた上限を超えたら打ち切って拒否する。`[agent].fetch_idle_timeout` か `[agent].fetch_timeout` を超えたら失敗。いずれも state と MFS は変えない。
6. state のロックを取り、作者が Follow Set から外れていれば終わる。版のパスを「保存中」としてメモリに登録して（9〜10 が終わるまで。sweep と unfollow はこのパスとその親ディレクトリを消さない）、ロックを放す。sweep が「登録より前に消し終えている」か「登録を見て残す」のどちらかになるよう、登録はロックの中で行う。
7. state のロックの外で、版のパスに CID を置く（既存の項目は先に消す）。失敗したら終わる。
8. state のロックの外で、`dag/stat`（`offline=true`、タイムアウト 300 秒）の `TotalSize` を実サイズとする。
9. state のロックを取る。8 が失敗していれば（ブロックが欠けている場合を含む）、ロックを放して 7 のパスを消して終わる。作者が Follow Set から外れていれば同じく 7 のパスを消して終わる。`size` タグより大きければ warn を出す。実サイズで `policy::decide` する。skip ならロックを放して 7 のパスを消し、拒否して終わる。
10. 新版を、受け取った署名済みのサイトイベントと一緒に記録し（[`state.md`](state.md#sitesevent)。nostr-sdk は一度確かめた `id` の署名を確かめ直さないので、ここで署名を確かめ、正しくなければ版だけを記録する）、evict した版を `sites` から消して state を保存し、ロックを放してから evict した版のパスを消す。続けて、同時実行の枠を返してからレプリカ報告の同期を行う（[`replicas.md`](replicas.md)）。

- 4・5 で終えたとき、取得したブロックは Kubo の GC に任せる。
- パスの削除に失敗しても state はそのままにし、sweep に任せる。
- 取得の失敗（タイムアウトを含む）やブロックの欠けで終えたサイトは、3 の試行から `min_update_interval` が過ぎてから取り直す。

## 拒否の記録

取得した内容で拒否した版（4 の `no_space_left` と `not_a_directory`、5 の上限超過、9 の `policy::decide` の skip）の CID を、サイトごとにメモリに覚え（`agent::store::Attempts`）、同じ CID のイベントは 1 で終える。

- 1 サイトに複数の CID を覚える。記録はアカウントごとに合計 50 件まで（`REJECTED_PER_ACCOUNT`）で、超えたらそのアカウントの最も古い記録を捨てる。
- 保存に成功したらそのサイトの記録を消す。取得の失敗やブロックの欠けなど一時的な失敗は覚えない。
- `remove_on_unfollow = true` の unfollow（[`../agent.md#unfollow`](../agent.md#unfollow)）のとき、今の Follow Set にいないアカウントの記録（拒否と試行の間引き）を消す。

## 取得の試行の間引き

`Attempts::try_attempt` が、取得を始めた時刻をサイトごとにメモリに覚えて間引く。保存に成功したらそのサイトの記録を消す。

- 同じサイトは、前の試行から `min_update_interval` の間は取得しない（`fetch_attempt_interval`）。取得の失敗や拒否で終わった場合も、保存済みのサイトの更新でも同じ。
- 同じアカウントで、直近 `min_update_interval` の間に試行したほかのサイトが `max_sites_per_account` 件（1〜50 に丸める）あれば取得しない（`fetch_attempts_per_account`）。保存済みで拒否の記録が無いサイトの更新はこの件数では止めない（試行としては数える）。
- `min_update_interval` が 0 なら効かない。
- 試行を記録するときに、そのアカウントの間隔を過ぎた記録（拒否の記録が無いもの）は消す。

拒否と試行の記録は `state.json` に書かないので、agent を再起動すると消える。
