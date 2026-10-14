# Desktop 画面のツールチップを UI 表示言語に従わせる

## 決めたこと

- Desktop 画面は「中の文字列は全部固定の日本語」という決まりだったが、トレイのネットワークアイコンのツールチップだけを先に例外にした（[`2026-10-14-desktop-tray-network.md`](2026-10-14-desktop-tray-network.md)）。同じ理由を残りのツールチップにも広げ、`title` と、それと対になる `aria-label` を UI 表示言語に従わせる。ツールチップはブラウザが Desktop 画面の絵の外に出すもので、Win95 風の見た目の一部ではないので、訳しても見た目は崩れない。英語の利用者は、アイコンだけのボタンや状態アイコンの意味をツールチップで読むことになるので、ここが日本語だと読めない。
- 対象: タイトルバーの最小化・最大化（最大化中は「元に戻す (縮小)」）・閉じる（ウィンドウ・「コントロール パネル」・「ミラーに追加」）、ツールバーの「更新」「ミラーに追加」、リンク集ページの iframe の `title`、リンク集ページの各行（ステータスアイコン・タイトルのリンクと更新待ちのタイトル・`[本家]`・`[キャッシュ]`）、「ミラーに追加」ダイアログのフィンガープリントの `title`。
- `[本家]` はツールチップを持たないが、`aria-label` の「<サイトのタイトル> — サイトを開く」は `[キャッシュ]` の `aria-label` と同じ形で、片方だけ訳すと揃わないので一緒に訳した。サイトのタイトルの部分はそのまま。
- 対象外にしたもの:
  - ツールチップの無い `aria-label` だけの名前（デスクトップアイコンの「SWING Explorer を開く」など・タブの一覧・色見本・表示方法・表示するマスコット・確認の間隔）。画面に見える日本語の部品に付けた名前で、見える文字と読み上げをそろえるほうを優先した。
  - 画面に見える文字。決まりの中心で、変えない。
  - 中身から来る `title`（壁紙のファイル名や色・マスコットのパック名・npub・吹き出しのリンク）。訳す文言が無い。
  - フィンガープリントの `aria-label`（絵文字の文字）。言語に依らない。
- 訳した要素は `lang` も UI 表示言語にする。`#view-desktop` は `lang="ja"` なので、そのままだと英語の `title` に `ja` が付く。トレイのアイコンですでにそうしていた。
- `lang` を変えた要素の中に見える日本語（「更新」「ミラー」「[本家]」「[キャッシュ]」、リンクにしたサイトのタイトル）は `<span lang="ja">` で包み、今までどおり日本語として扱わせる。サイトのタイトルは本当の言語が分からないが、ページの中のほかの文字と同じく `ja` を受け継いでいた扱いを変えないことにした。
- 文言は `i18n.js` に `desk` 接頭辞のキーで移し、日本語は今までの文言のままにした。「サイトを開く」「ゲートウェイで開く」は既存の `openSite`・`openGateway` と日本語が同じなのでそれを使う。フィンガープリントの文言は Sites 画面の `fingerprintTitle`（「npub で確かめてください」が付く）と違うので、`deskFingerprintTitle` を別に作った。
- 静的な要素は既存の `data-i18n-title`・`data-i18n-aria-label` に乗せ、`lang` は新しい `data-i18n-lang` 属性を見て `applyStaticI18n` が当てる。どの要素の `lang` を変えるかを、祖先の `lang` から推し量るより、印で明示するほうが読みやすい。
- JS で作る要素には `i18n.js::setUiLabels(elm, { title, label })` を足し、`title`・`aria-label`・`lang` をまとめて当てる。トレイのネットワークアイコンもこれを使うようにした。
- 言語の切り替えで当て直す方法:
  - 最大化ボタンは状態で文言が変わるので `data-i18n-*` を付けない。付けると `app.js` の `applyStaticI18n` が、`desktop-window.js` の当て直しの後に「最大化」で上書きしてしまう（`app.js` の `swing:langchange` の登録が Desktop 画面の初期化より後のため）。`desktop-window.js` が `swing:langchange` で当て直す。
  - リンク集ページの行は `cache.sites` から行だけ作り直す。`DesktopView.render()` はステータスバーを「完了」に戻すので、ミラーに追加した直後の結果の文言を消さないよう、行の一覧だけを作り直す `renderLinks` に分けた。
  - 「ミラーに追加」ダイアログのプレビューは、最後に受け取った鍵の一覧を覚えておいて作り直す。`/api/mirror/preview` は呼び直さない。

## 作ったもの

- `web/i18n.js`: `desk` 接頭辞のキー 15 個（英語と日本語）、`setUiLabels`、`applyStaticI18n` の `data-i18n-lang`。
- `web/index.html`: タイトルバーの最小化・閉じる（3 か所）、ツールバーの「更新」「ミラーに追加」、iframe に `data-i18n-*` と `data-i18n-lang` を付け、ツールバーの見える文字を `<span lang="ja">` で包んだ。
- `web/desktop-window.js`: 最大化ボタンの文言を `t()` にし、初期化時と `swing:langchange` で当てる。
- `web/desktop.js`: `DESK_TEXT` からツールチップの文言を外して `t()` にし、ステータスアイコンの選び方を `statusIconKind` にまとめた。`renderLinks` を分け、`swing:langchange` で行を作り直す。
- `web/desktop-mirror-add.js`・`web/desktop-fingerprint.js`: フィンガープリントの `title` を `t()` と `setUiLabels` にし、言語の切り替えでプレビューを作り直す。
- `web/desktop-network.js`: `setUiLabels` を使う。
- `src/dashboard/assets/mod.rs`: `index.html` の `data-i18n`・`data-i18n-title`・`data-i18n-aria-label`・`data-i18n-placeholder` のキーが英語と日本語の両方にあることを確かめるテストを足した。
- `docs/architecture/dashboard/desktop.md`: 「Desktop 画面は丸ごと日本語固定」の節を「Desktop 画面の表示言語」に書き直し、トレイのアイコンの例外をその決まりに含めた。`desktop/link-page.md` にツールチップの表、`desktop/mirror-add.md` にフィンガープリントの `title`、`web.md` に `data-i18n-lang` と `setUiLabels`。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通った。足したテストは、`index.html` のキーを辞書に無い名前に変えると失敗することを確かめた。
- デモ環境の Desktop 画面で、英語と日本語のそれぞれで対象の要素の `title`・`aria-label`・`lang` を確かめ、再読み込みせずに言語を切り替えて（Settings 画面の言語の選択に `change` を送る）、最大化中の「元に戻す」・リンク集ページの行・ダイアログのフィンガープリントまで当て直されること、見える文字が日本語のままであることを確かめた。
- デモ環境のサイトには更新待ち・NIP-05 の不一致と失敗・古い更新が無いので、`window.fetch` を包んで `/api/sites` の応答にそれらのサイトを足し、ツールバーの「更新」で読み直して、すべての種類の行を確かめた。
- 対象外の `aria-label`（デスクトップアイコン・タブの一覧・色見本・表示するマスコット）が英語のときも日本語のままであることを確かめた。
