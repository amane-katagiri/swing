# 英語の文言を見直す

ダッシュボードの英語（`web/i18n.js` の `en` と `web/index.html` の初期テキスト）と、CLI・agent・トレイの英語の出力（ヘルプ・エラー・状態表示・ログ）を、不自然なところがないか見直した。

## 決めたこと

- 直す範囲は、文法（冠詞・一致・前置詞）・綴り・直訳調の言い回し・表記ゆれ・大文字と句読点の不揃いに絞った。
- 砕けた言い方（`Someone new is mirroring {title}` など）は残す。
- 独自の用語（mirror・mirror set・replica・Webring・trust tier など）と、サブコマンド・フラグ・設定キーの名前は変えない。
- 単数と複数の出し分け（`1 sites` になるところ）は足さない。
- 表記を揃えた。
  - ダッシュボードは文頭だけ大文字にし、文になっているものは文末にピリオドを付ける。
  - CLI のエラーは小文字始まりで、文末にピリオドを付けない。
  - 状態表示の行は、同じ処理の中の流儀に合わせた。
  - 製品名は `SWING`、Kubo・CID・HTTP は大文字にし、綴りは米式にした。
- `--check-unchanged` の欄名「Diff from last deploy」は、ここだけ deploy でほかはすべて publish だったので、「Diff from last publish」に揃えた。
- 通知の種類「When a site you follow is newly mirrored」は、「自分のサイトがミラーされたとき」の項目と区別がつかなかった。実際の条件（フォロー中のサイトの新しい版を自分が保存したとき）に合わせて、「When you mirror a new version of a site you follow」にした。
- `warn` モードの説明は「Only report problems found」から「Report problems but publish anyway」にした。問題があっても公開を続けることが分かるようにするため。
- 二重起動のエラーは `already running on <dir>` から `already using <dir>` にした。この文字列で判定しているものは、リポジトリの中にはない。

## 検証

- `cargo fmt`・`cargo clippy`・`cargo test` が通った。
- 文言を引用していた architecture と、テストの期待値を同じ変更で直した。
- `web/index.html` の初期テキストのうち、`en` に同じキーの文があるものは、そのキーの英語と同じにした。プルダウンの短い値（`off`・`warn`・`require`）は、もとから JS で差し替える前提の仮の文字なので、そのままにした。
