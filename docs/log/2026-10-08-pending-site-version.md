# 新しい版を待っている間も保存済みの版を見せる

サイト一覧の各行は relay から取った最新のサイトイベントで、保存状態・大きさ・ゲートウェイのリンクは、イベントと CID が同じ版からしか取っていなかった。そのため、新しい版を `min_update_interval` などで取り込んでいない間は、前の版を保存して配っていても「未保存」と出て、大きさもリンクも消え、Desktop 画面のリンク集からもサイトが消えていた。

## 決めたこと

- 状態を 3 つに分ける。イベントの版を保存していれば「保存済み」、していないが同じサイトの別の版を保存していれば「更新待ち」、1 つも保存していなければ「未保存」。
- 「更新待ち」のときに見せる版は、state にある同じサイトの版のうち `created_at` が最大のもの。イベントより新しい版を持っている場合（relay が古いイベントを返したとき）も同じ扱いにする。
- `GET /api/sites` のサイトに `previous` を足す。イベントの版の欄（`stored_size`・`stored_at`・`gateway_url`）の意味は変えず、前の版の値は別の欄にまとめる。フロントが「イベントの版」と「配っている版」を取り違えないようにするため。
- Desktop 画面のリンク集では、更新待ちのサイトを前の版で載せる。新しい版の `message` はその版の更新メモなので外す。保存の通知は前の版のときに出ているので、足さない。
- Sites 画面の「Stored only」チェックボックスを、すべて・保存済み・更新待ち・未保存の `<select>` にした。保存先の `localStorage` のキーも `swing:sites:stored-filter` に変え、前のキーは読まない（表示の好みだけなので、移行は入れない）。
- テーブルではリンクの文言を短い「ゲートウェイで開く」にし、前の版を開くことは `title` で示す。長い文言だと列がはみ出したため。

## 変えたもの

- `mirror::SiteRow` に `previous: Option<VersionRecord>` を足し、`collect_sites` で埋める。`swing sites` は `[update pending]` と、`title`・`message` の行の後に `stored version:` の行を出す。
- `dto::SiteDto` に `previous`（`cid`・`created_at`・`stored_at`・`stored_size`・`gateway_url`）を足す。
- Web UI: `ui.js::storedState` とバッジの `data-stored="pending"`、カードとテーブルの保存済みの版の表示とリンク、Desktop 画面のリンク集、保存状態のフィルター。

## 検証したこと

- `cargo fmt`・`cargo clippy -D warnings`・`cargo test --workspace` が通る。`followed_accounts` が、保存済み・更新待ち・未保存のそれぞれで `stored` と `previous` を正しく埋めるテストを足した。
- デモ環境で、保存済みの `bob-zine` に新しい版を publish し、agent が `min_update_interval` で取り込まない状態を作った。`swing sites` が `[update pending]` と保存済みの版を出すこと、Sites 画面のカード（英語）とテーブル（日本語）で「更新待ち」と保存済みの版の大きさ・日時・リンクが出てはみ出さないこと、フィルターの「更新待ち」でそのサイトだけになること、Desktop 画面のリンク集に前の版で載ること、前の版のゲートウェイのリンクが前の版の中身を返すことを確かめた。
