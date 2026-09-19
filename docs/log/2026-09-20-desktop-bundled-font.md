# Desktop 画面にビットマップフォントを同梱

## 問題

- Desktop 画面のフォントスタックは `"MS UI Gothic", "MS PGothic", ...` で、Windows 以外では先頭のフォントが無く現代的な滑らかなゴシック体にフォールバックしてしまい、当時の個人サイトらしいドット感が失われていた。
- `.desk-page` の中の見出し（`h1`/`h2`/`h3`）は `style.css` のダッシュボード全体向け `h1, h2, h3` ルール（フォント・字間・`h2` のボーダー）が要素セレクタの詳細度で `.desk-page-inner` からの継承に勝ってしまい、レトロなフォント指定が効いていなかった。

## 決めたこと

| 決定 | 理由 |
|---|---|
| PixelMplus12 の Regular・Bold の 2 ウェイトを同梱する（M+ FONT LICENSE。woff2 で約 276KB + 約 264KB、バイナリは合計で約 540KB 増える） | ビットマップ風のドットフォントで、当時の個人サイトの雰囲気に合う。ライセンスが自由（改変・商用可、表示義務なし） |
| JIS X 0208 相当のサブセットにはしない（試したところ削減はサイズの約 2% のみ） | サイト名・メッセージは利用者が自由に入力する任意の文字列で、事前に文字集合を絞れない |
| 同梱フォントをフォントスタックの先頭に置く（`"PixelMplus12", "MS UI Gothic", ...`） | Windows で比較した際に既定の MS UI Gothic より好ましいと判断されたため、OS 依存のフォールバックに頼らず全 OS で同じ見た目にする |
| `#view-desktop` 内のテキストは（`.desk-counter` を除き）すべて 12px の倍数にする | PixelMplus12 はビットマップ由来のアウトラインフォントで、12px の倍数以外のサイズでは輪郭が滲む |
| タイトルは 24px・Regular、見出しは 12px・Bold にする（36px/24px Bold、DotGothic16 32px も試した） | 見た目で選んだ。DotGothic16 はウェイトが Regular のみで太字が合成太字になり、ファイルサイズも約 500KB 増える。PixelMplus10 は小さすぎた |
| ステータスアイコンの外枠を 32×14px → 36×16px に広げる | 12px テキストが枠に収まるようにするため |
| CSP は変更しない | `font-src` を個別設定しておらず `default-src 'self'` にフォールバックする。同一オリジンの `/fonts/*` は元々許可されている |
| 見出しの半角 `&` を全角 `＆` に変える（「更新履歴 & リンク集」→「更新履歴 ＆ リンク集」） | 見た目で選んだ |

## 作ったもの

- `web/fonts/pixelmplus12-regular.woff2` `web/fonts/pixelmplus12-bold.woff2` `web/fonts/LICENSE-PixelMplus.txt`。
- `assets.rs`: `FONT_PIXELMPLUS12_REGULAR` / `FONT_PIXELMPLUS12_BOLD`（`include_bytes!`）とハンドラ。`mod.rs`: `/fonts/pixelmplus12-regular.woff2` `/fonts/pixelmplus12-bold.woff2` のルートとコンテンツタイプのテスト。
- `desktop.css`: `@font-face`（`PixelMplus12`、400/700、`font-display: swap`）、`--desk-font-ui`/`--desk-font-page` の先頭に追加、`-webkit-font-smoothing: none; font-smooth: never;`、chrome・本文・見出し・タイトル・ステータスアイコンのサイズ調整（すべて 12px の倍数に）。
- `index.html`: 見出しの `&amp;` を `＆` に変更。
- `README.md`・`docs/architecture.md`・`docs/architecture/dashboard.md`・`docs/architecture/dashboard/web.md`: 同梱フォントの記載を追加、古いサイズ（32×14px のステータスアイコンなど）の記述を更新。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -j 3 -- -D warnings` / `cargo test -j 3`（243 passed, 11 ignored）。
- `grep -rn "trial\|Trial\|dotgothic\|DotGothic\|pixelmplus10" web/ src/` は空。
- デモ環境（`docker/demo/demo.sh up`）を再ビルドし、1440×900 で表示。`document.fonts.check('12px PixelMplus12')` と `bold 12px` がともに `true`、コンソールに CSP 違反やエラーは無し、`localStorage` にトライアル用のキーは残っていない（`swing:desktop:visits` のみ）、Start ボタンの `pointer-events` は `none`（`id="desk-start"` も削除済み）。
- 変更前（トライアル状態、`data-trial-font="pixelmplus12"` 既定）と変更後を 1440×900 でスクリーンショットし、時計・マーキー・来訪者カウンタ・見出し文言（`&`→`＆`）・NEW アイコンの点滅の 6 箇所をマスクして PIL で pixel diff を取った。差分は 0（マスク外は完全一致）で、フォールドインが忠実であることを確認した。
- 390×844 でページ全体のスクロールが出ないこと、タスクバーが折り返さないことを確認。320×240（1440×900 のビューポートから変更）でもツールバー・アドレス行・ステータスバー・タスクバーの `offsetHeight` が広い画面と同じ値のままであることを確認した。
