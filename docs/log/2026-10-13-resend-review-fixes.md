# 保存した版のサイトイベントの再送の見直し

v0.1.6 からの「保存した版のサイトイベントの再送」をレビューし、セキュリティ上の穴をふさぎ、重複したコードをまとめた。

## 決めたこと

- **どれかの relay に新しい版があれば、どこにも送らず、取り下げとして覚える。** 作者が新しい版を出しても、こちらの保存ポリシー（大きさ・NIP-05 など）で保存しなかったときは、state の最新の版は古いまま残る。今までは relay ごとに比べていたので、新しい版を持たない relay へ古い版を送り、作者が置き換えて取り下げた版を生き返らせていた（protocol.md の「確実に取り下げたい作者は新しい版で置き換える」が成り立たない）。どれかの relay が署名の正しい、未来ずれの許容内の新しい版を持っていれば、どの relay にも送らない。後で relay がその新しい版を落としても送り直さないように、既存の `withdrawn` の `through` を新しい版の `created_at` − 1 にして記録する。state.json の形は変えない。
- **同じ秒の置き換えは `id` で覚える。** NIP-01 では同じ `created_at` なら `id` の小さい方が新しい。このとき `through` を新しい版の `created_at` にすると新しい版も含んでしまい、後でその新しい版を保存しても送り直さなくなる。`created_at` − 1 では古い版を含まない。そこで同じ秒のときは古い版の `id` を `events` に足す。
- **`e` タグの削除要求は `created_at` によらず使う。** `e` は版の `id` そのものを指し、未来の時刻を付けても範囲は広がらない。今までは未来ずれの許容を超える削除要求を丸ごと捨てていたので、作者が時計のずれた環境で出した取り下げを見落としていた。`a` タグは範囲（その `created_at` 以下の版）を決めるので、捨てずに今の時刻 + 許容で頭打ちにする。
- **state に入れる前に署名を確かめる。** nostr-sdk は一度確かめた `id` の署名を確かめ直さないので、relay は同じ `id` で署名だけ違う写しを渡せる。それを保存すると、送り直す前の確認（署名を確かめ直す）で落ちて、そのサイトは送り直されなくなる。保存の順序で記録するときと、過去分の取得でイベントを埋めるときに署名を確かめ、正しくなければイベントは記録しない（版は今までどおり記録する）。埋めるときは、署名の正しくないイベントを持つ版も埋め直す。relay ごとの最新を選ぶとき（`keep_newest_site`）も署名を確かめる。上の「新しい版があれば取り下げ」の根拠になるので、偽の新しい版で送り直しを止められないようにするため。
- **確認を始めるときの対象で絞る。** 対象 pubkey は poll の時点のもので、報告の回が終わるまでに Follow Set が変わることがある。確認を始めるときに agent の今の対象と重ねる。
- **`nip05 = "require"` では検証済みのサイトだけ送る。** 保存はできても後で NIP-05 の検証が通らなくなったサイトを、この参加者が広めないようにする。state の `verifications` の `status` が `verified` のサイトだけを対象にする。キャッシュの期限は見ない（期限切れでも最後の結果を使う）。
- **受理するのに保存しない relay への送り直しは今回は直さない。** そうした relay には確認のたびに全部を送り直すことになるが、バックオフを受理の後の確認にまで広げる必要があり、別の作業にする（todo に足した）。

## 作ったもの・変えたもの

- `nostr::plan_site_resends` は `&[&Event]` を受け取り、サイトごとに `ResendPlan::Superseded(新しい版)` か `ResendPlan::SendTo(relay の列)` を返す。`RelayAnswers.sites` は `SiteAnswer`（`Unanswered`・`Missing`・`Found`）の列にした。
- `nostr::is_saved_site_event` を `saved_site_event`（パースした `SiteEvent` を返す）にし、作者・パース・`d` の確認を `keep_newest_site` と共有した。agent は返った `SiteEvent` で `cid`・`created_at` を比べ、二度パースしない。
- `Withdrawals::note` は `e` を常に索引に入れ、`a` の `created_at` を頭打ちにする。
- agent の再送は、保存したイベントを state から 1 回だけ複製し、候補・送信の予定へは値で渡す。取り下げの記録は置き換えと削除要求で共通の関数にした。
- `State::fill_latest_event` と `store_fetched` で署名を確かめる。
- `RelayClient::read_relays`・`publish_to`（`publish_to_relays` はこれに任せる）を `nostr/client.rs` に移し、ほかの取得でも使う。`FetchCap::per_relay` と、期限付きの relay ごとのストリーム（`bounded_relay_stream`）を足し、重複を除いた。
- `fetch_site_deletions` の後処理（relay ごと・サイトごとの「答えたか」）と、受け取った削除要求の畳み込みを関数に分けた。`fetch_sites_per_relay` は削除要求の取得と同じく、REQ の先頭のサイトの位置を畳み込みの状態に持つ。削除要求のフィルタはイベントから直接作り、16 進の文字列を経由しない。
- `refresh_follow_set`・`resubscribe_and_backfill` の引数を `&RelayClient` に戻した。
- テストの補助（削除要求・サイトイベントの組み立て、relay の起動と投入、REQ を断る relay）を `crate::test_support` にまとめた。
- ドキュメント: `docs/protocol.md` 第 9 節に、新しい版による取り下げと、未来の時刻の削除要求の扱いを足した。`docs/architecture/agent/resend.md`（手順の 1・4・8 と表。削除要求の境の規則は `nostr/fetch.md` へのリンクだけにした）、`agent.md`・`agent/state.md`・`nostr.md`・`nostr/fetch.md`、`docs/guide/publish.md` の取り下げの説明を更新した。

## 確かめたこと

- どれかの relay に新しい版があると、どの relay にも送らず、`through` を新しい版の `created_at` − 1 で記録し、その relay が無くなった後の確認でも送らない。同じ秒の置き換えでは古い版の `id` を記録し、新しい版は取り下げに含まれない。
- 署名の正しくない写しは relay ごとの最新に選ばれない。署名の正しくないイベントは保存の順序でも過去分の取得でも記録されず、記録済みのものは正しいイベントで埋め直される。
- 未来の時刻の `e` の削除要求は取り下げになり、`a` の範囲は今の時刻 + 許容で頭打ちになる。
- 確認を始める前に対象から外れた作者のサイトは送らない。`nip05 = "require"` では検証済みのサイトだけが対象になる。
- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通る。

## 二度目の見直し

### 決めたこと

- **記録済みのイベントは埋め直さない。** `State::fill_latest_event` は、署名の正しくないイベントを持つ版も埋め直していた。署名の正しくないイベントを記録するのはこの作業より前の未公開の版だけなので、実質は移行のための処置で、poll のたびに state のロックの中でサイトごとに署名を確かめる手間がかかっていた。`event` の無い最新の版だけを埋め、受け取ったイベントの署名は今までどおり確かめる。
- **`#e` の答えが `limit` で切られたら、その作者（組）のサイトはどれも「答えていない」とする。** 今までは `#a` と同じく境（`limit` 件目の `created_at`）より新しい版を「答えた」としていた。`e` の削除要求は自身の `created_at` によらず版を取り下げ、時計のずれた作者は版より前の時刻を付けることもある（保存する版も未来ずれの許容の分だけ先の時刻であり得る）ので、境より新しい版を指す削除要求が切られていることがある。境から `MAX_FUTURE_SKEW` を引いて比べる案もあったが、削除要求の時刻に前提を置かない方を選んだ。`#e` は保存している版の `id` だけを引くので、`limit` に達するのは作者がそれらを大量に削除したときに限られる。protocol.md 9.3 にも書いた。
- **新しい版を記録したら、そのサイトの `withdrawn.events` を空にする。** 古い版は `event` を持たないので、その `id` が送り直しの対象になることはなく、`events` は増える一方だった。`through` は範囲なので残す。最新の版が消えて古い版が最新に戻り、過去分の取得でその `event` を埋めたときは、その版の `e` の削除要求を次の確認で取り直す。

### 作ったもの・変えたもの

- 取り下げの型を `state::Withdrawn` にまとめた。`Withdrawals::of` が `Withdrawn` を返し、agent は置き換えでも削除要求でもそれをそのまま `State::record_withdrawal(key, withdrawn)` に渡す。`nostr::Withdrawal` と agent の `Withdrawing` はやめた。`DeletionMatch` と `Withdrawals::note`・`merge` は `nostr` の中だけに見えるようにした。
- kind 5 の確認は `Withdrawals::note` だけで行う（`DeletionReq::note` の重複をやめた）。
- `held_site_events` は NIP-05 の確認の後でイベントを複製する。

### 確かめたこと

- 署名の正しくないイベントは埋めず、記録済みのイベントは埋め直さない。
- `#e` の答えが `limit` に達すると、境より新しい版のサイトも「答えていない」になる（境より古い、切られた削除要求がその版を指していても送らない）。`#a` の境の規則は変わらない。
- 新しい版を記録すると `events` が空になり `through` は残る。同じ秒や古い版の記録では変わらない。
- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通る。

## 三度目の見直し

### 決めたこと

- **途中で切れた答えの新しい版も、置き換えの根拠にする。** 今までは期限切れ・上限超え・途中の切断で「答えていない」になった REQ の答えを丸ごと捨てていたので、その前に署名の正しい新しい版を受け取っていても、ほかの relay へ古い版を送り直していた。新しい版があることは答えが最後まで届いたかによらないので、`SiteAnswer::Unfinished` として返し、`plan_site_resends` は置き換えの判定にだけ数える。送る先（「無い」や古い版を返した relay）には数えない。削除要求の取得が、答えていない REQ の取り下げも索引に合わせるのと同じ考え方。protocol.md 9.1 にも書いた。
- **送る直前に対象を確かめ直す。** 確認を始めるときに対象で絞っても、2 回の取得の間に Follow Set が変わることがある。`send_held` で作者が今も対象にいるサイトだけを送る。
- **取り下げは、記録する時点の最新の版が確認を始めたときの版のままのときだけ記録する。** 候補は確認を始めたときの state から作るので、取得の間に新しい版を保存すると（`apply_store` が `events` を空にする）、古い版の `id` を `events` に足し直していた。`cid` と `created_at` で比べ、変わっていたら記録しない。
- **relay が黙って切る削除要求は見落とし得る、と書いておく。** relay の上限が `MAX_DELETIONS_PER_FILTER` より小さいと、切られた答えが `limit` に達しない。コードは変えず、前提を `nostr/fetch.md` と protocol.md 9.3 に書き、todo に足した。

### 作ったもの・変えたもの

- `fold_per_relay` の REQ ごとの受け取りと上限の確かめを `fold_request` に分けた。`fetch_sites_per_relay`・`fetch_site_deletions` の REQ ごとの範囲の計算を `chunk` にまとめた。動きは変えていない。
- `held_site_events` は保存したイベントを 1 回だけ取り出す。
- `nostr/fetch.md` の削除要求の REQ のまとめ方を、作者ごとではなくフィルタの組（`UNITS_PER_REQ`）ごとの記述に直した（サイトの多い作者は複数の組になる）。

### 確かめたこと

- 新しい版を返した後に上限超えで「答えていない」になった relay の答えは `Unfinished` になり、ほかの relay が「無い」と答えても `Superseded` になる。途中で切れた答えの古い版は送る先にも置き換えにも数えない。
- 送る直前に対象から外れた作者のサイトは送らない。
- 確認の間に新しい版を保存したサイトには、古い版の取り下げを記録しない。
- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通る。
