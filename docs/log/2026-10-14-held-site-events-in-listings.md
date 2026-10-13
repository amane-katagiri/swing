# 一覧で relay の答えを保存したサイトイベントで補う

`state.json` に持つ署名済みのサイトイベント（[保存した版のサイトイベントの再送](2026-10-13-resend-held-site-events.md)）を、一覧で relay の答えを補うのにも使う。残タスクの「relay から取れないときにも透過的に使う」のうち、一覧の部分（1）を行った。中身の照合で忘れた版のイベントを取り直しの元にする部分（2）は行わないことにした（下記）。

## 決めたこと

- **一覧の取得で、relay の答えに保存したイベントを混ぜてから最新を選ぶ。** `RelayClient::fetch_latest_signed_sites` に `held` を足し、relay から取ったイベントと合わせて `select_latest` に渡す。一覧の作り方を変えずに、relay から消えたサイトや relay に古い版しか無いサイトも保存している版で出せる。relay に新しい版があれば、これまでどおりそちらが選ばれる。
- **混ぜるのは `swing sites`・`swing replicas`・`swing webring`（とそれぞれのダッシュボードの API）の 3 つ。** agent の過去分の取得には混ぜない。保存している版のイベントを投げても保存済みとして飛ばされるだけで、得るものが無い。
- **混ぜるのは各サイトの最新の版の `event` のうち、署名を確かめられ、`cid` が版と一致し、`created_at` が版以上のもの（`State::listed_site_events`）。** 条件は送り直しの候補の確かめ（`agent::resend` の `resend_candidates`）から `withdrawn` を除いたもので、そちらと違って理由ごとのログは出さない（一覧は何度も呼ばれるため）。
- **取り下げられた版も一覧に出す（`withdrawn` を見ない）。** `withdrawn` は送り直すかだけに使う記録で、取り下げられた版も中身は保存し続ける。一覧は保存しているものを見せる場所なので、保存していれば出す。外すと、取り下げられたフォロー中のサイトが、保存しているのにどの一覧にも出なくなる（フォローを外したサイトの一覧は state の版から作るので出るが、フォロー中のサイトは relay の答えからしか行ができない）。新しい版で置き換えられたサイトは、relay に新しい版があればそちらが選ばれて今までどおり更新待ちで出る。relay から新しい版まで消えたときは古い版が最新として出るが、手元にあって配っているのはその版なので表示としては正しい。
- **フォローを外したサイトの `url`・`title`・`message` は、版が持つイベントから出す。** 確かめ方は送り直しと同じ `nostr::saved_site_event` で、`cid` が版と一致するものだけを使う。
- **中身の照合で忘れた版のイベントを取り直しの元にする部分（2）は行わず、残タスクからも外す。** 起きるのは、relay からイベントが消え、かつ手元の中身が壊れたときだけで、もともと珍しい。relay から消えても毎時の再送が保存したイベントを戻すので、過去分の取得から取り直しが始まる。再送が効かないのは、受理しない relay・NIP-05 の `require`・イベントを持たない版のときに限られる。さらに、ほかの誰も中身を持っていなければ、イベントがあっても取り戻せない。得るものに比べて、`state.json` に項目を足して古い形式を読む処置を入れ、いつ消すかの規則を足すコストが大きい。

## 作ったもの・変えたもの

- `State::listed_site_events(site_event_kind, authors)` を足した。
- `RelayClient::fetch_latest_signed_sites` と `fetch_latest_sites` に `held` を足した。relay の答えと同じ `id` のものは足さない。
- `mirror::collect_sites` は `state.json` を先に読んで `held` を渡す。`replicas::collect`・`webring::collect` も `state.json` を読むようになった（読めなければ全体がエラー）。
- `mirror::unfollowed_accounts` は版のイベントから `url`・`title`・`message` を埋める。
- ドキュメント: `nostr.md`・`mirror.md`・`webring.md`・`agent/state.md`・`cli/views.md`・`dashboard/http-api/status.md`、`todo.md`。

## 確かめたこと

- relay に無いサイトは保存したイベントで出る。relay に古い版しか無いサイトは保存したイベントの版が、relay に新しい版があるサイトは relay の版が選ばれる（ローカルの relay を相手にしたテスト）。
- `listed_site_events` は、最新の版の、条件を満たすイベントだけを返す。`withdrawn` に当たるものも返す。古い版・`cid` の違うもの・`d` の違うもの・イベントの無い版・ほかの作者のもの・kind の違うものは返さない。
- フォローを外したサイトは、イベントの `cid` が版と一致するときだけ `url` が出る。
- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通る。
