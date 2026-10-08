# publish で通常の投稿（kind 1）もする

## 決めたこと

- `swing publish --note` とダッシュボードの公開画面のチェックボックスで、サイトイベントに加えて kind 1 の投稿もできるようにした。サイトイベントは SWING に対応していないクライアントではほぼ見えないので、フォロワーへ更新を知らせる手段として足した。
- 投稿は `url` があるときだけ。CLI は clap の `requires = "url"`、API は `invalid note: needs a URL` の 400 で止める。画面では URL 欄が空の間はチェックボックスを無効にする。
- 本文は `title`・`url`・メッセージのうち空でないものを半角スペースでつないだ固定の形にした。別の本文を指定するオプションは作らない。
- サイトを指す `a` タグ（`35980:<pubkey>:<d>`）を付け、対応するクライアントがサイトイベントと結び付けられるようにした。protocol.md には任意の投稿として MAY/SHOULD で書き、受信側が保存や現在の版の判断に使わないことを MUST NOT にした。extensions.md に「標準の kind」の表を足した。
- 投稿するのはサイトイベントがどれかの relay に受理された後だけ。投稿の失敗（署名・送信・どこにも受理されない）は publish の成否を変えず、警告だけを出す。サイトはもう公開済みなので、ここで失敗扱いにすると再実行で同じ内容の版を作り直すことになるため。
- `created_at` は署名する時点の現在時刻。サイトイベントの `created_at` は版の名前のために現在時刻より先になることがあるが、投稿にはその制約が無い。
- NIP-46 のペアリングで求める perms に `sign_event:1` を足した。ペアリング済みの接続には反映されないので、利用者向けの手引き（security.md）に、署名アプリで許可するかつなぎ直すよう書いた。
- ダッシュボードのチェックボックスの状態は、既存の `swing:publish:last`（localStorage）に他のフォーム内容と一緒に保存する。

## 作ったもの

- `nostr::site_note_content`・`nostr::build_site_note_builder`（`src/nostr/site.rs`）
- `publish::post_site_note`・`publish::check_note_request`、CLI の `Note` の見出しと表示（`src/publish.rs`）
- `POST /api/publish/upload` の `note` パートと、応答の `note`（`relays`・`error`）（`src/dashboard/`）
- 公開画面のチェックボックスと、結果のパネルの投稿の行（`web/`）

## 検証

- `cargo fmt --all --check`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通る。本文の組み立てと `a` タグ、URL なしの `note` が 400 になることを単体テストで確かめた。
- デモ環境で、ダッシュボードから投稿付きで公開し、結果に投稿の relay の成否が出ること、URL が空の間はチェックボックスが無効なこと、読み直してもチェックの状態が戻ることを確かめた。コンテナの中で `swing publish --note` を実行して `Note` の見出しの下に relay の成否が出ること、`--url` なしでは引数の誤りで止まることを確かめた。
