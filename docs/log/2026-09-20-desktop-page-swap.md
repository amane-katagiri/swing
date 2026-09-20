# リンク集ページ・CSS・バナーの差し替え

## 決めたこと

- Desktop 画面の「リンク集」ページ本文・そのスタイル・88×31 バナーの 3 つを、起動時に外部ファイルへ差し替えられるようにした（`[dashboard].desktop_page` / `desktop_page_css` / `desktop_banner`、`SWING_DASHBOARD_DESKTOP_PAGE` ほか）。未設定なら従来どおり同梱のものを使う。
- 「外の CSS がページに当たらず、ページの CSS も外に漏れない」を厳密に満たすため、Shadow DOM ではなく**同一オリジンの iframe**（`/desktop-page.html`）にした。Shadow DOM では継承プロパティ（`font-family`・`color` など）と CSS 変数が外から中へ入るので、`:host { all: initial }` を書いても「一切当たらない」は保証できない。
  - 代償としてガードのヘッダを `frame-ancestors 'none'` → `'self'`、`X-Frame-Options: DENY` → `SAMEORIGIN` に緩めた。他オリジンからの埋め込みは引き続き不可なので、クリックジャッキング耐性は実質変わらない。
  - 差し替えるファイルが「完結した 1 枚の HTML」になるのも iframe を選んだ理由。断片 + 親の CSS 前提より書きやすい。
- ページ側に JS は置かない。親（`desktop.js`）が `iframe.contentDocument` から決まった `id`（`desk-link-list` / `desk-page-status` / `desk-marquee-text` / `desk-counter`）を探し、あるものにだけ書き込む。差し替えたページに無い `id` は黙って飛ばす。
- 読み込みは起動時 1 回だけ（`AppState::new` → `DesktopAssets::load`）。`/custom.css` のようにリクエストごとにディスクを見る方式は採らず、読めないパスは同梱版へフォールバックせずに agent の起動を止める。タイポに気付けないまま既定の見た目で動き続けるのを避けるため。
- バナーの Content-Type は拡張子から決める（png / gif / jpeg / webp / svg）。当時のバナーはアニメーション GIF が普通なので許容した。未対応の拡張子は起動時エラー。
- `desktop.css` はウィンドウ枠だけの CSS になり、ページ本文の規則（配色・マーキー・リンク行・スクロールバー・`prefers-reduced-motion`）は `desktop-page.css` へ移した。ページ側は `:root` に自前の `--desk-*`・`@font-face`・`box-sizing` リセットを持ち、親から何も受け取らない。

## チラつき対策

デモ環境で確認したところ、リロード直後に 2 種類のチラつきが出たので合わせて直した。

- iframe の CSS が当たる前に素の HTML が見え、工事中アイコンの `<svg>` が既定サイズ（300×150）で一瞬描かれる → ページ本文のインライン SVG に `width`/`height` 属性を付けた。
- 他の画面でリロードしてから Desktop に移ると、PixelMplus12 が遅れて適用されて字が入れ替わる → フォントは描画されるまで取得が始まらないので、`DesktopView.init()` で `document.fonts.load("12px PixelMplus12")`（Regular/Bold、親と iframe の両方）を明示的に呼ぶようにした。
- そのうえで `.desk-window` を `is-loading`（`visibility: hidden`）付きで出力し、iframe の `load` とフォントのロードが終わってから外す（最大 1.5 秒で打ち切り）。ウィンドウごと出現を少し遅らせるほうが、途中経過が見えるより自然だという判断。

### Firefox で残っていた分

Chrome では消えたが、Firefox では「他の画面でリロード → Desktop に移動」でまだチラついた。Firefox は `display: none` の iframe にレイアウトを作らないため、iframe のスタイルとフォントが実際に効くのは画面に出た後で、起動時にだけ待つ上記の作りでは初回表示が素通しになっていた。

- 待つタイミングを起動時から `DesktopView.onShow()` の 1 回目に移した。`visibility: hidden` はレイアウトを止めないので、隠したまま初回レイアウトとフォント適用を済ませられる。待ちには `requestAnimationFrame` 2 回分を足した。
- `index.html` と `desktop-page.html` に woff2 の `<link rel="preload" as="font" crossorigin>` を足し、レイアウトの有無に関係なく最初のページ読み込みで取得を終わらせる。
- `@font-face` を `font-display: swap` から `block` に変えた。代替フォントで一度描いてから入れ替える（＝チラつきそのもの）より、来るまで出さないほうがこの画面には合う。
- Firefox はこの環境に無く（`agent-browser` は Chromium のみ）、実機確認はオタクくん側にお願いした。Chromium では初回表示から 31ms（rAF 2 回分）でウィンドウが出ることを計測した。

## スクロールバーは窓側に残す

最初はスクロールバーの規則もページ側の CSS（`desktop-page.css`）に入れていたが、スクロールバーは OS/ブラウザの見た目でページの持ち物ではない。CSS を差し替えた人が Win95 風スクロールバーまで書き直す羽目になるのはおかしいので、`desktop-frame.css` に分けた。

- 親ドキュメントの CSS からは子フレームのスクロールバーに触れられないので、同一オリジンであることを使って `desktop.js` が iframe のドキュメントに `<link>` を差し込む（`head` の先頭。差し替えたページが上書きできる余地を残す）。「外のスタイルは中に入らない」の唯一の例外。
- 色は `--desk-sb-*` という専用の変数にして、ページ側の `--desk-*` と名前が衝突しないようにした。
- ウィンドウの表示待ちに、この差し込みの完了も含めた。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（新規 3 件: 設定したファイルが配信されること、読めないパスとバナーの未対応拡張子が起動時エラーになること）。
- `docker/demo/demo.sh up --seed` のデモ環境をブラウザで確認。リンク一覧・マーキー・カウンタ・バナーが従来どおり描画され、`frame-ancestors 'self'` でも iframe が開くこと、スクロールとウィンドウ操作（ドラッグ・リサイズ）が iframe 越しでも動くことを見た。
- iframe の中身を最小限のページ（`desk-link-list` と `desk-counter` だけを持つ、serif・ピンク地の自作 HTML）に差し替えて、リンク一覧とカウンタが描画され、ダッシュボードのスタイルが一切当たらず、ページ側のスタイルも外に漏れないことを確認した。
- リロード後の `performance` を見て、フォントの取得完了が約 100ms、ウィンドウの表示がその直後になることを確認した。
