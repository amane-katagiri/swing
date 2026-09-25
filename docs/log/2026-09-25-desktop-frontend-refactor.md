# 2026-09-25 Desktop 画面フロントエンドの整理リファクタ（フェーズ A〜C）

## 対応した項目

「コントロール パネル」ダイアログの追加とキーボードフォーカスの統一（直前の 2 回の作業）を経て、`web/desktop-settings.js` が 798 行の単一ファイルになり、フォーカス/アクティブ状態の管理がウィンドウ・ダイアログ・デスクトップ本体の 3 箇所にそれぞれ少しずつ違う実装で散らばっていた。ドキュメント（`docs/architecture/dashboard/web.md`）の import 関係の記述も実コードとずれ、新しい静的ファイルを 1 つ足すたびに Rust 側（`assets.rs` の `const`・ハンドラ関数、`mod.rs` のルート、content-type を検証する手書きのテストの 4 箇所）を手で揃える必要があった。今後「設定項目」や「小さなウィンドウ/ダイアログ」を足しやすくすることを目的に、挙動と見た目を変えずに 1 ファイルだったものをモジュールへ分割する整理を進め、フェーズ C（JS の残り整理・CSS 分割・ドキュメント反映）で仕上げた。

## 決めたこと

| 決定 | 理由 |
|---|---|
| モジュール構成を「殻」と「部品」に分ける: `desktop.js` の `LAUNCHERS` テーブル（アイコン起動元の一覧）とデスクトップ本体、`desktop-focus.js`（フォーカストラップ・アクティブ同期の共有部品）、`desktop-drag.js`（ポインタ操作の共有部品）、`desktop-dialog.js` の `createDialog()`（モーダルダイアログ一般の機能）、`desktop-settings.js` の `PAGES` 配列とページのインターフェース（`id`/`init`/`open`/`isDirty`/`save`/`discard`/`onKey`/`boot`）、`desktop-wallpaper.js`（「背景」タブの状態）、`desktop-wallpaper-image.js`（量子化・ダウンスケールの純粋関数）、`desktop-combobox.js`（汎用コンボボックス部品） | 「どのダイアログ/ページにも要る」処理と「このタブ/この機能だけの」処理を別ファイルに分けることで、新しいタブやダイアログを足すときに触るファイルが限定される。依存は下から上への一方向に揃え、循環 import を作らない |
| CSS も同じ考えで 3 分割: `desktop.css`（フォント・シェル・`--desk-*` 変数・ウィンドウとダイアログが共有する枠のクロム・タスクバー・アイコン・共通の focus-visible・メディアクエリ）、`desktop-dialog.css`（オーバーレイ・ダイアログの枠・タブ・本文・ボタン行と、どのダイアログからも使える汎用部品）、`desktop-wallpaper.css`（「背景」タブ固有の見た目） | JS と同じ境界（共通クロム／ダイアログ一般／個別タブ）を CSS 側にも通すことで、次のタブやダイアログを足すときにどのファイルへ何を書けばよいかが機械的に決まる |
| 汎用部品のクラス名を改名: `.desk-wp-fieldset` → `.desk-fieldset`、`.desk-wp-combobox*` → `.desk-combobox*` | どちらも壁紙タブ固有の見た目ではなく、`<fieldset>` の min-width 挙動の打ち消しと Win95 風コンボボックスという、他のタブ/ダイアログからも使い回せる部品だったため。改名前に `web.md` の「安定 class」一覧に載っていないことを確認した |
| 同じ 4 層の盛り上がりベベル・1px の盛り上がり・1px の沈み込みの `box-shadow` を、それぞれ `#view-desktop` のカスタムプロパティ `--desk-bevel-raised`/`--desk-bevel-raised-1`/`--desk-bevel-sunken-1` にまとめる。値が微妙に違うもの（`--desk-shadow` を使う版、非対称な版など）は無理にまとめない | 見た目を変えずに同一パターンの重複を消す。値が違うものを無理に共通化すると、後で片方だけ変えたいときに事故る |
| `outline: 1px dotted #000000; outline-offset: -3px` が完全に同じ値で 5 箇所（タイトルバーボタン・ツールバー「更新」・タスクボタン・ダイアログのタブ・コンボボックスのフィールド）にあったのを 1 つのセレクタ群にまとめ、`desktop.css` に置く | 「共通の focus-visible ルール」として、ウィンドウ・ダイアログどちらの要素も参照する。デスクトップアイコン（白・内側）とスウォッチ/カラー入力（外側）の 2 系統は値も理由も別なのでまとめない |
| `DesktopSettings.applyStoredWallpaper()` を廃止し、ページ側に任意の `boot()` を持たせて `DesktopSettings.boot()` が `PAGES` を舐めて `page.boot` があれば呼ぶだけにする | 「保存済みの状態を表示に反映する」処理を壁紙タブ専用の名前で殻に持たせていると、次のタブがそれを必要としたときにまた殻を触ることになる。`boot` は無ければスキップするだけなので、必要なページだけが持てばよい形にした |
| `desktop-settings.js` のタブボタン・タブパネル・適用ボタンの参照を、`document` 全体からではなくダイアログの root（`#desk-dialog-control-panel`）配下の `querySelector`/`querySelectorAll` に変えた。適用ボタンも `id` ではなく `createDialog` 側と同じ `[data-dialog-action="apply"]` で拾う | 今は `.desk-dialog-tab`/`.desk-tabpanel` を持つダイアログが 1 つしか無いので実害は無かったが、2 つ目のダイアログ（今回追加はしていない）を足したときに `document.querySelectorAll` が他のダイアログの要素まで拾ってしまう作りだったため、先に直しておいた |

## 採らなかった案

- コンボボックス（`desktop-combobox.js`）の外側クリック判定を、`click` から `pointerdown` に変える案。ダイアログのタイトルバーをドラッグしている最中は `click` が飛ばないため外側クリックでの自動クローズが効かないことがあり、`pointerdown` なら防げるように見えるが、候補（`role="option"` の `<li>`）自体の選択確定も `click` で行っている。外側判定だけを `pointerdown` にすると、候補をクリックしたときに確定用の `click` より先に外側判定の `pointerdown` が発火して選択前に閉じてしまい、クリックでの選択自体が壊れる。挙動が変わってしまうため不採用とし、ドラッグ中に限ってダイアログのタイトルバー側の `pointerdown` でコンボボックスを閉じる、という今のピンポイントな対処のままにした。
- ウィンドウのリサイズ（8 方向のハンドル、最小サイズのクランプ）を `desktop-drag.js` の汎用部品として切り出す案。リサイズ可能な非モーダルウィンドウは「SWING Explorer」1 つしか無く、ダイアログはリサイズを持たないため、今 2 箇所で使われているポインタ操作（`trackPointer`/`clampPosition`/`screenSize`）だけを共有部品にし、リサイズのハンドル自体とその最小サイズ判定は `desktop-window.js` に残した。使う場所が 1 つしかないコードを一般化すると、将来 2 つ目が来たときの実際の要件が分からないまま抽象化することになる。
- disable された直後にフォーカスが外へ落ちる問題を、`desktop-focus.js` の汎用の保険（`keepFocusOnDisable`。無効化が原因で外へ出た場合だけコンテナへ戻す）だけに任せる案。「削除」ボタンは押した後の自然な戻り先が「参照...」、ダイアログの「適用」は OK ボタンというように、ボタンごとに Win95 として自然な戻り先が違う。汎用の保険はダイアログ「コンテナ」へ戻すだけで具体的な戻り先を知らないため、`refocusIfDropped(root, fallback, fn)` による明示的なフォーカス移動をボタンごとに残し、汎用の保険は「まだ具体的な戻り先を用意していない disable」のための最後の网（フォールバック）という位置づけのままにした。

## 内部的な差分（フェーズ A〜C 全体、`git diff` ベース）

- ウィンドウのタイトルバードラッグは、リサイズハンドルだけがやっていた「`pointerdown` で `pointermove`/`pointerup`/`pointercancel` を張り、ジェスチャの終わりに剥がす」方式に統一した（`desktop-drag.js` の `trackPointer()`）。従来のタイトルバードラッグは起動時に永続的にリスナーを張ったままだった。
- 背景色パレット（`PALETTE`）にあった `#808080`（グレー）の重複を削除し、24 色 → 23 色になった。
- `applyBackground(target, state, opts)` の第 3 引数を `{preview: boolean}` から、プレビュー枠と実画面の縮尺そのものを表す数値 `scale`（既定 1）に変えた。呼び出し側が `previewScale()` を渡すかどうかを選ぶだけの単純な形になった。
- `focusableIn()` を `desktop-focus.js` に一本化したことで、iframe 内（リンク集ページ）のフォーカス候補の抽出にも `!node.disabled` の条件が付くようになった（以前は親ドキュメント側の候補抽出にしか付いていなかった）。
- Rust 側は `src/dashboard/assets.rs` の `STATIC_ASSETS` というテーブル + 2 つの小さな `macro_rules!`（`text_asset!`/`bytes_asset!`）にまとめ、`assets::register()` が 1 行ずつループしてルートを登録する形に変えた。新しい静的ファイルを 1 つ足すのに要る変更は、以前の 4 箇所（`const` 定義・ハンドラ関数・`mod.rs` のルート・content-type を検証する手書きの一覧）から、この 1 つの配列への 1 行足しだけになった。`mod.rs` の `ui_router()` も個別の `.route()` の羅列から `assets::register(Router::new())` への 1 行に縮んだ。

## 確認したこと

- `cargo fmt` / `cargo clippy --workspace --all-targets -- -D warnings`（`-j 4`） / `cargo test --workspace`（`-j 4`）すべて成功（477 passed, 16 ignored）。既知の flaky（`signer::tests::answers_from_a_signer_whose_clock_runs_behind_are_received`）はこの実行では発生しなかった。
- `web/*.js` すべてを `node --check` で構文確認。
- デモ環境（Chromium）で `#/desktop` を 1280×800・390×700 の両方で確認: 初期表示、SWING Explorer を開いた状態、コントロール パネルを開いた状態、コンボボックスを開いた状態、Tab でのフォーカス移動（タイトルバーボタン・スウォッチの外側点線・デスクトップアイコンの白い点線・コントロール パネルを閉じた直後にアイコンへ戻るフォーカス表示）を確認。コンソールエラー・ページエラーとも無し。同じ画面の以前のスクリーンショット（本リファクタ前のもの）とピクセル差分（ImageMagick `compare`）を取り、時計・来訪者カウンタ・マーキーのスクロール位置・文字のサブピクセル位置ゆらぎ（連続 2 回撮っただけでも生じる）を除いて、スウォッチの色やコンボボックスの文字など静的な領域は完全一致することを確認した。
- 壁紙（背景色・壁紙画像）はテスト後に既定（`localStorage` にキーが無い状態）へ戻した。
- Firefox では確認していない。

## 最終的な行数

`desktop*.js`: `desktop.js` 443・`desktop-window.js` 269・`desktop-settings.js` 68・`desktop-dialog.js` 156・`desktop-wallpaper.js` 317・`desktop-wallpaper-image.js` 107・`desktop-combobox.js` 131・`desktop-focus.js` 65・`desktop-drag.js` 39。
`desktop*.css`: `desktop.css` 680・`desktop-dialog.css` 208・`desktop-wallpaper.css` 146。
