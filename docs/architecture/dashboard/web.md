# ダッシュボードの画面（`web/index.html`, `web/*.js`）

[`dashboard.md`](../dashboard.md) の一部。サーバ側の起動・ガード・タイムアウト・静的ファイル配信は [`dashboard.md`](../dashboard.md)、HTTP API の入出力は [`http-api.md`](http-api.md) を参照。

## 構成

ビルド工程なし・外部依存なしの素の HTML + CSS + ES modules。依存は下から上への一方向で循環 import は無い。

| ファイル | 役割 |
|---|---|
| `storage.js` | `localStorage` の薄いラッパー。他のどのモジュールにも依存しない |
| `i18n.js` | 多言語辞書と `t()`。`storage.js` にだけ依存する |
| `util.js` | 画面間で共有するキャッシュ・DOM/fetch ユーティリティ・表示スタイル切替・非同期ロードのガード（`createLoadGuard`）。`storage.js`・`i18n.js` に依存する |
| `ui.js` | 複数画面で共有する UI 部品（コピーボタン、バッジ、relay 結果表示など）。`util.js`・`i18n.js` に依存する |
| `graph.js` | webring 用の自前 force-directed layout。`util.js` の `clamp` だけに依存する |
| `sites.js` / `webring.js` / `publish.js` / `settings.js` / `desktop.js` | 各画面（それぞれ Sites・Webring・Publish・Settings・Desktop） |
| `boot.js` | 描画前に同期実行する小さな通常スクリプト。サイドナビの折りたたみ状態と、表示言語が英語以外なら翻訳待ちの印を付ける。どのモジュールにも依存しない |
| `app.js` | ルーター兼エントリポイント。`<script type="module" src="/app.js">` から読み込まれ、各画面モジュールを import する |

CSS は `style.css`（全画面共通）に加え、Desktop 画面のウィンドウ枠専用の `desktop.css` を `index.html` が `<link>` で読み込む（読み込み順は下記「CSS カスタマイズのインターフェース」を参照）。Desktop 画面の「リンク集」ページ本文だけは別ドキュメント（`desktop-page.html` + `desktop-page.css`）で、iframe の中で動く（下記「リンク集ページ（iframe）」）。

`#/desktop` `#/sites` `#/webring` `#/publish` `#/settings` の 5 画面をハッシュルーティングで切り替える（既定は `sites`）。現在の画面のナビのリンクには `aria-current="page"` が付く。書き込みリクエストには `X-Swing-Dashboard: 1` と `Content-Type: application/json` を付ける。relay 由来の文字列は DOM API だけで挿入し、`innerHTML` は使わない。`url` は `^https?://` にマッチするときだけリンクにする。

言語切り替え（Settings 画面）は `settings.js` が `swing:langchange` という `CustomEvent` を `document` に投げ、`app.js` がそれを購読して各画面を再描画する。各画面のロードは世代カウンタ（`createLoadGuard`）でガードし、切り替えが速くても古いレスポンスで上書きしない。

サイドナビの並び順は上から Desktop・Sites・Webring・Publish・Settings（`#/desktop` が一番上）。

| 画面 | 内容 | 表示スタイル（`data-style`、localStorage キー `swing:style:<view>`） |
|---|---|---|
| Desktop | [`/api/sites`](http-api.md#get-apisites) から `stored: true` のサイトだけを集め、レトロ調（Win95/98 風デスクトップ＋ブラウザ風ウィンドウ内の「リンク集」ページ）に描画する。詳細は下記「Desktop 画面」 | スタイル切替なし |
| Sites | [`/api/sites`](http-api.md#get-apisites) の一覧、mirror への追加・削除、`Unfollowed but still stored`、[`/api/status`](http-api.md#get-apistatus) を呼ぶ Storage check（版ごとの判定の表と、サイトごとの実容量・合計の表） | `cards`（既定）/ `table` |
| Webring | [`/api/webring`](http-api.md#get-apiwebringrootkeydepthn) を root・depth 指定で取得。ノード選択で [`/api/replicas?key=`](http-api.md#get-apireplicaskeykey) を引き、詳細パネルからミラー操作もできる | `graph`（既定）/ `list` / `ascii` / `source`（dot・mermaid） |
| Publish | [`/api/overview`](http-api.md#get-apioverview)・[`/api/publish/sites`](http-api.md#get-apipublishsites)（My sites）、publish フォーム（常にフォルダアップロード） | スタイル切替なし |
| Settings | [`/api/config`](http-api.md#get-apiconfig) を読み取り専用表示。テーマ・言語・カスタム CSS の設定 | スタイル切替なし |

## Desktop 画面

Win95/98 風デスクトップ（テールグリーンの背景、デスクトップアイコン、タスクバー、Start 風ボタン、`setInterval` で 30 秒ごとに更新する時計）と、その上に浮かぶ「SWING Explorer」ウィンドウ（タイトルバー・メニューバー・ツールバー・アドレス行・ページ本文・ステータスバー）を `#view-desktop` の中に静的 HTML（`index.html`）で組む。ツールバーとアドレス行（`アドレス(D)` ラベル+URL 表示）は別々の行に分け、間に薄い彫り込み線（`box-shadow` のハイライト+シャドウ）を挟む（ラベルとボタン類は `white-space: nowrap; flex: none`、URL 側と本文側は `flex: 1 1 auto; min-width: 0` + 省略記号にして、320×240 まで縮めても折り返さない）。ウィンドウの移動・リサイズ・最小化・最大化・閉じる/再オープンは `desktop.js` の素の JS（pointer events）で動く。それ以外（デスクトップアイコンのうち 2 個、Start ボタン、メニューバー、ツールバー、アドレス行、ステータスバー、タスクバー本体、トレイのアイコン）は完全に装飾で、`cursor: default`・`user-select: none` にして見た目だけクリックできそうに見えないようにしてある。実際に押せるボタン・ハンドル類も含め、カーソルは基本的に矢印のまま変えない（後述）。配色はダッシュボードのテーマ（`--swing-*` 変数）を参照せず、`desktop.css` の `#view-desktop` スコープに直書きしたレトロパレット（`--desk-teal` `--desk-face` `--desk-navy` など）を固定で使う。ライト/ダークどちらのテーマでも見た目は変わらない。

### フォント

`#view-desktop` は同梱の PixelMplus12（Regular/Bold の 2 ウェイトのみ、`@font-face` で `/fonts/pixelmplus12-regular.woff2` `/fonts/pixelmplus12-bold.woff2` を読み込む）をフォントスタックの先頭に置く: `--desk-font-ui: "PixelMplus12", "MS UI Gothic", "MS PGothic", "ＭＳ Ｐゴシック", Osaka, "Yu Gothic", sans-serif;`（`--desk-font-page` も同様、先頭以外は元のフォールバック列のまま）。OS 依存のフォールバック（Windows の MS UI Gothic など）に頼らず、同梱フォントで全 OS の見た目を揃えるための選択。`#view-desktop` に `-webkit-font-smoothing: none; font-smooth: never;` を当ててアンチエイリアスを切り、ビットマップ由来のドットをそのまま出す。

PixelMplus12 はビットマップフォントを元にしたアウトラインフォントで、12px の倍数以外のサイズでは輪郭が滲む。そのため `#view-desktop` 内のテキストは（`.desk-counter` を除き）すべて 12px の倍数: 本文・chrome 系は 12px、`.desk-hero-title`（タイトル）は 24px・`font-weight: 400`、`.desk-panel h2`（見出し）は 12px・`font-weight: 700`。ウェイトは 400/700 の 2 つのみ（合成太字は使わない）。ステータスアイコンの外枠は 12px テキストに合わせて 36×16px。`.desk-counter`（来訪者カウンタ）だけは意図的に対象外で `"Courier New", monospace` の 13px のまま。

ドットを滲ませないため、テキストが載る箱は整数ピクセルに載せる。ウィンドウのジオメトリは `desktop.js` の `applyWindowGeometry()` が `left`/`top`/`width`/`height` を `Math.round` してから当てる（既定ジオメトリの中央寄せ `(画面幅 - 幅) / 2` は画面幅とウィンドウ幅の偶奇が食い違うと 0.5px になり、ウィンドウ全体—タイトルバー・ステータスバー・iframe の中身まで—が半ピクセルずれて滲む）。ページ側も同じ理由で、`desktop-page.css` の `.desk-page-inner` を `margin: 0 auto` の自動中央寄せではなく、幅を 2px 単位に切り下げ（`--desk-col`）、左マージンを 1px 単位に切り下げて置く（CSS の `round()`。未対応のブラウザでは `@supports` が丸ごと落ちて `margin: 0 auto` に戻る）。幅を偶数にするのは、中の `text-align: center` も半ピクセルを踏まないようにするため。マーキー（`.desk-marquee-track`）も同じ理由で、`transform` を直接アニメーションさせず、登録済みカスタムプロパティ（`@property --desk-marquee-x`、`<length-percentage>`）を 0% → -100% で補間し、`transform: translateX(round(var(--desk-marquee-x), 1px))` で 1px に丸めてから当てる。`transform` を直接動かすと合成レイヤごと半端な位置に置かれて滲み、`steps()` を挟んでも（各ステップの位置が整数にならないので）直らない。この差し替えも `@supports (transform: translateX(round(-100%, 1px)))` の中だけなので、`round()` 非対応のブラウザは従来の `@keyframes desk-marquee`（滑らかだが滲む）に戻る。

ページ本文は iframe の別ドキュメントなので `style.css` は一切届かず、`desktop-page.css` が `:root` に同じ `--desk-*`（本文で使う分だけ）と `@font-face` を自前で持つ。`style.css` の `h1, h2, h3` を打ち消すための規則はもう要らない。

フォントが遅れて差し替わるチラつき（CSS 適用前の素の HTML や代替フォントが一瞬見える）は 4 段構えで防ぐ。

1. `index.html` と `desktop-page.html` の両方が 2 つの woff2 を `<link rel="preload" as="font" crossorigin>` で先に取りに行く（`crossorigin` が無いと CORS モードの本番の取得と別扱いになり 2 回落ちる）。
2. `@font-face` は `font-display: block`。代替フォントで描いてから入れ替えるのではなく、フォントが来るまで字を出さない。
3. `.desk-window` は `is-loading`（`visibility: hidden`）付きで出力し、**Desktop 画面を初めて表示したとき**（`DesktopView.onShow()` の 1 回目）に iframe の `load` → `desktop-frame.css` の差し込みと、親と iframe 両方の `document.fonts.load("12px PixelMplus12")`（Regular/Bold）→ `requestAnimationFrame` 2 回、を待ってから外す（最大 1.5 秒で打ち切る）。起動時ではなく初回表示で待つのは Firefox のため: `display: none` の iframe にはレイアウトが無く、スタイルとフォントが実際に効くのは画面に出てからなので、起動時に待って外すと「その後の初回表示」が素通しになる。`visibility: hidden` ならレイアウトは走るので、この待ちの間に中身が固まる。`DesktopView.init()` では同じフォントのロードだけ先に始めておく（他の画面を開いている間に取得を終わらせる）。
4. ページ本文のインライン SVG アイコン（`.desk-sitemenu-icon`）には `width`/`height` 属性を書いてあり、CSS が来る前でも既定サイズ（300×150）ではなく 14×14px で描かれる。

ライセンスは `web/fonts/LICENSE-PixelMplus.txt`（M+ FONT LICENSE）。CSP の `font-src` は個別に設定しておらず `default-src 'self'` にフォールバックする（同一オリジンの `/fonts/*` は許可される）。

### 装飾 vs 実機能

`#view-desktop` に `cursor: default; user-select: none;` を 1 回だけ書いて継承させ、機能する要素だけ選択的に戻す（ページ本文は別ドキュメントなので継承を受けず、ブラウザ既定のまま選択でき、リンクのカーソルも既定の `pointer`）。装飾要素は `aria-hidden="true"` を個別に持ち、スクリーンリーダーやキーボード操作からも外れる（メニューバー・ツールバーのようにマウスオーバーだけ反応するものも同様。テキストを持つ `.desk-sitemenu` はクリック不可でも情報として意味があるので `aria-hidden` にしない）。実際に押せるタイトルバーのボタン・タスクボタン・「更新」ボタン・デスクトップアイコンも `cursor: pointer` にはしない（Windows 95 の実機がそうだったのに合わせている）。矢印以外のカーソルが出るのは、本文中のハイパーリンク（`pointer`。iframe の中）と 8 個の `.desk-resize` ハンドル（各方向のリサイズカーソル）だけ。

装飾ボタン・メニューのうち「ボタン」と「メニュー」に分類できるもの（ツールバーの戻る/進む/中止/ホーム、メニューバーの各項目）はマウスオーバーにだけ反応する（IE4 の「フラットな coolbar」に合わせ、静止時はベベル無しの平面、`:hover` で 1px の立ち上がりベベル（白の上・左 + `#808080` の下・右）が付く。クリックしても何も起きず、`:active` の沈み込みは付けない）。この 2 グループだけ `pointer-events: none` を外してあり（ホバー判定に必要）、それ以外（トレイのアイコン・アドレスバー・ステータスバー・Start ボタン）は元どおり `pointer-events: none` のままホバーにも反応しない。実際に押せる「更新」ボタン（`#desk-reload`）は同じ平面→ホバーベベルの挙動に加えて `:active` で沈み込みベベルも付く（唯一の実機能ボタンなので区別できる）。フォーカス不可・選択不可・カーソル矢印のままという制約はホバー対応後も変わらない。

実際に動く要素:

- ツールバーの「更新」（`#desk-reload`）: `DesktopView.load(true, …)` を呼んで再取得する。
- タイトルバーの最小化・最大化/元に戻す・閉じるボタン（`#desk-btn-min` `#desk-btn-max` `#desk-btn-close`、実際の `<button>`。`aria-label`/`title` は表示言語に関わらず固定の日本語（後述「表示言語」）で、最大化ボタンだけは状態に応じて JS（`desktop.js` の `updateMaxGlyph`）がラベルとグリフ（`.desk-glyph-max` ⇄ `.desk-glyph-restore`）を切り替える）。
- タイトルバー自体（ドラッグで移動。ナロー幅では無効）、ダブルクリックで最大化⇄復元をトグル。最大化中にタイトルバーを 4px 以上ドラッグすると最大化を解き、元の大きさに戻した窓をポインタの下（タイトルバー上の横位置の割合を保つ）に置いてそのまま移動を続ける。
- ウィンドウ四辺+四隅の `.desk-resize` ハンドル（8 個、`data-dir` で方向を持つ）。それぞれ対応するリサイズカーソルを持ち、`setPointerCapture` でドラッグ中の要素を固定する。南東ハンドルは当たり判定だけウィンドウの角の外側にもはみ出す（16×16px）が、何も描画しない。最大化中はハンドルを 8 個とも `display: none` にし、カーソルも変えない。見た目のサイズグリップは別要素の `.desk-grip`（ステータスバー右下、12×12px、インライン SVG の高さ 1px の `<rect>` を並べ、白 1px + 灰色 2px の斜めの筋を 3 本描いた Win98 風グリップ）で、ウィンドウ枠の内側に収まる。最大化中とナロー幅では非表示にする。
- タスクバーのタスクボタン（`#desk-taskbtn`、実際の `<button>`。表示中かつアクティブなら押すと最小化、最小化中なら押すと復元。ウィンドウを閉じると `hidden` になり消える）。
- デスクトップアイコン（`.desk-icon`）は 3 つともシングルクリックで選択でき、選択は常に 1 つだけ（`is-selected`）。デスクトップの余白やウィンドウをクリックすると選択を解く。選択中の見た目は Win95/98 に合わせ、アイコン+ラベル全体を囲む枠は付けず、ラベルだけが紺地に白文字になり（紺地の内側 1px に白の点線矩形が入る）、グリフの白が水色（`#a8c8ff`）寄りに変わる。「マイ コンピュータ」と「ごみ箱」は選択できるだけで、開けず、フォーカスも受けない（`aria-hidden`）。「SWING Explorer」（`#desk-icon-explorer`、`.desk-icon-shortcut`）だけがダブルクリックまたは Enter / Space で開ける。グリフは同じ S+E のマークを白一色で描いた `icon-desk-swing`（`.desk-icon-glyph-solid`。他の 2 つは白い線画）。線で描くマークなので、このクラスは `fill`/`stroke` を `none` にして `color` だけを持ち、シンボル側が `stroke="currentColor"` で受ける。キーボードフォーカス時の点線矩形（`:focus-visible`）はラベルにだけ沿わせ、タイル本体は `outline: none`。ウィンドウを閉じたあとはこのアイコンだけが復帰手段で、既定のジオメトリで開き直す（最小化中なら単に復元する）。フルページリロードでもウィンドウは開いた状態に戻る。

残りは全部装飾: 「マイ コンピュータ」「ごみ箱」アイコン、Start ボタンとそのアイコン、アドレスバー、ステータスバーのテキスト、タスクバーの背景、トレイのアイコン+時計（メニューバーとツールバーはホバーだけ反応する。上記参照）。

### ウィンドウ管理の状態

ジオメトリ（位置・サイズ）と最小化/最大化/クローズの状態は `desktop.js` 内のモジュールスコープ変数だけが持ち、**永続化しない**（`localStorage` には書かない）。既定ジオメトリ（初回表示・アイコンからの再オープン）は `.desk-screen` の中で左右対称に中央寄せする。幅はデスクトップアイコン列がちょうど隠れない余白（108px、`ICON_COLUMN`）を目安に、`min(920, 画面幅 - 108*2)` を上下限（320〜画面幅）でクランプし、余った左右の余白を均等に振り分ける（`.desk-screen` が狭くて 108px ずつ取れないときは、幅 320px を確保できる範囲まで左右の余白を対称に縮める）。縦方向は変更なく、上下とも 18px（`SCREEN_PAD`）で対称。ユーザーがまだ一度も移動・リサイズしていない間は、`ResizeObserver`（`.desk-screen` を監視。サイドバーの折りたたみ/展開やブラウザのリサイズで発火）のたびにこの既定ジオメトリを再計算して中央寄せし直す。一度でもタイトルバードラッグ or リサイズハンドルで実際に動かすと `userPositioned` フラグが立ち、以降はその位置を新しい範囲内にクランプするだけで中央寄せには戻さない（最大化からの復元ジオメトリもクランプのみで、中央寄せの対象外）。最小サイズは 320×240。最大化中は `.desk-screen` の実寸を `Math.floor` した px をそのまま `width`/`height` に入れる（`ResizeObserver` → `reflowWindow()` で追従する）。`100%` だと画面幅が半端なときにウィンドウ幅も半端になり、右端に寄せたタイトルバーのボタンが半ピクセルにかかるため。760px 以下のナロー幅では `desktop.css` が `!important` でウィンドウを常に全画面（アイコン列も覆う）に固定し、移動・リサイズのハンドラも `isNarrowLayout()` チェックで無効化する（最小化・閉じるは動作させたままにして、ユーザーが操作不能にならないようにしている）。

タイトルバーとタスクボタンのアイコンは SWING Explorer のマークの `<symbol id="icon-desk-swing-color">`、Start ボタンは OS 側の記号なので SWING 本体のマークの `<symbol id="icon-desk-start">` を使う。いずれも 16×16px（整数ピクセルの高さに収まる場所だけ）で揃える。

SWING Explorer のマークは S と E を組み合わせたモノグラムで、左上に S・右下に E を置き、S の下のボウルが E の左上の肩に掛かる。図形は `index.html` 冒頭の `<defs>` に線（stroke）のまま 1 組だけ置き（`swing-se-s` / `swing-se-e`、`viewBox="5 2 54 60"` の座標系・線幅 7・`stroke-linecap: butt`）、色違いの 2 つの `<symbol>`（`icon-desk-swing-color` / `icon-desk-swing`）が `<use>` で参照する。重なりの隙間はマスク `swing-se-gap`（白い全面矩形から S の線を線幅 10.5 で抜く）で E 側に開け、S を上に描く。`icon-desk-swing-color` は S を金（`swing-se-gold`: `#ffe680`→`#f0a010`）、E を青（`swing-se-blue`: `#8fd4ff`→`#3a86f0`）のグラデーションで塗る。青は紺のタイトルバーに乗るので暗い側を明るめに取ってある。

Start ボタンの `icon-desk-start` は中心円（黄）・大きい方の腕+円（赤）・180°回転させた小さい方の腕+円（緑）の 3 色に塗り分けた SWING のマークで、`/home/miki/inbox/favicon.svg` の図形をベースに（インラインの `<style>` は使わず）色を直接 `fill` 属性で焼き込んだもの（滑らかなベクター画のまま、ドット絵化はしていない）。

トレイには時計の左に音量・ネットワーク・ミラーリング状態を模した 3 個の装飾アイコン（`icon-desk-tray-volume` / `icon-desk-tray-network` / `icon-desk-tray-mirror`、いずれも `#view-desktop` 専用の SVG `<symbol>`）を並べる。この 3 個は Win95/98 風のドット絵で統一し、`viewBox="0 0 16 16"` の整数グリッドに乗る軸並行の `<rect>`（`shape-rendering="crispEdges"`）だけで構成し、16 色 VGA パレットのみを使い、輪郭は黒 1px、光源は左上（実体はほぼフラットな塗りで表現）。表示サイズも `.desk-tray-icon { width: 16px; height: 16px; image-rendering: pixelated }` で 16×16px に固定し、拡大縮小によるにじみを防ぐ。実在の製品ロゴは使っていない。音量アイコンは黄色いスピーカー（箱+ラッパ形のコーン）から 2 本の黒い段付きの弧が伸びる形。ミラーアイコンは SWING のマークを 16×16 に簡略化したドット絵（黒縁の黄色い中心、上半分を囲む赤い弧、下半分を囲む緑の弧）で、バナー画像のマークと同じ形。ネットワークアイコンは 2 台のモニタを模した図形で変更していない。

ツールバーの「戻る」「進む」「中止」「更新」「ホーム」もアイコン付きに揃えている（`icon-desk-tb-back` / `-forward` / `-stop` / `-reload` / `-home`、いずれも `viewBox="0 0 16 16"` のドット絵で、トレイアイコンと同じ 16 色 VGA パレット・黒 1px 輪郭・`.desk-toolbtn-icon { width: 16px; height: 16px; image-rendering: pixelated }`）。戻る/進むは緑の三角矢印、中止（`中止` ラベル、装飾のみ）は赤い丸に白い ✕、更新は緑の 2 本の円弧矢印、ホームは赤い屋根+白い壁+臙脂の扉の家。レイアウトはアイコン左+ラベル右（IE4 風。ツールバーの高さを抑えるため）。実際に押せるのは「更新」（`#desk-reload`）だけで、他はテキストと同じく `aria-hidden="true"` の装飾。

「マイ コンピュータ」のグリフ（`icon-desk-mycomputer`、`viewBox="0 0 24 24"`）は他のデスクトップアイコンと同じ白い線画スタイル（`.desk-icon-glyph`: `fill: none; stroke: #fff`）で、ブラウン管モニタ（画面のベゼル+スタンド）がデスクトップ型の本体（フロッピースロットの線+小さい LED）の上に乗った形。`icon-desktop`（サイドナビが使う汎用モニタのグリフ）とは別のシンボルで、こちらは変更していない。

タイトルバーの最小化/最大化/閉じるボタンのグリフはボタン 16×14px に対して置く。最小化・最大化・元に戻すは画像を使わず CSS の箱（`.desk-glyph-*`、疑似要素の `border`/`background` で描画、8×8px）で組み、いずれも整数ピクセルの位置に収まる。閉じるだけは `rotate()` で描くと斜め線が滲むため、インライン SVG（8×7px、`shape-rendering="crispEdges"`、軸に沿った 1×1px の `<rect>` を 13 個並べて Win95 の × を再現）にしてある。ボタンの高さ 14px に対しグリフの高さが 7px（奇数)で単純な中央寄せだと半端なピクセルになるため、閉じるボタンだけ `align-items: flex-start` + `margin-top: 3px` で整数オフセットに固定している。どのグリフも `:active` 時は `transform: translate(1px, 1px)` で右下に 1px ずれる。

リンク集ページ（iframe の中。`#view-desktop` で実際にスクロールする唯一の領域）のスクロールバーは Win95/98 風に描き直してある。スクロールバーは中のページではなく**窓の持ち物**なので、差し替えできる `desktop-page.css` ではなく、差し替えできない `desktop-frame.css` に置き、`desktop.js` が iframe のドキュメントに `<link>` として差し込む（親ドキュメントの CSS では子フレームのスクロールバーに触れないため、同一オリジンであることを使って JS で入れる）。`head` の先頭に入れるので、差し替えたページが自前で上書きすることもできる。色は `--desk-sb-*` という専用の変数で持ち、ページ側の `--desk-*` とは名前を分けてある。セレクタは `#view-desktop *` ではなくそのドキュメントの `*`。Chromium/Safari 系は `::-webkit-scrollbar*` 疑似要素（トラックは白黒 2px 市松、つまみとボタンは `--desk-face` に立体ベベルの `box-shadow`、矢印は `data:image/svg+xml` の小さい三角形。片側だけの `:start:decrement`/`:end:increment` のみ表示し、逆側の `:start:increment`/`:end:decrement` は `display:none`）。矢印グリフの `data:` URI は CSP の `img-src 'self' data:` の範囲内。Firefox など `::-webkit-scrollbar` 非対応エンジンは `@supports not selector(::-webkit-scrollbar)` の中でだけ `scrollbar-color`/`scrollbar-width` にフォールバックする（標準プロパティと `::-webkit-scrollbar` を同時に指定すると Chromium 側が無効化してしまうため）。

### レイアウト（隙間なく画面いっぱいに敷く）

広い幅（760px 超）では `body[data-view="desktop"] .swing-shell` に `height: 100vh` を当てて（他画面の `min-height: 100vh` の伸びを止め）、`.swing-main` を `display: flex; flex-direction: column; height: 100%; overflow: hidden;` にする。`#view-desktop` 自体も `flex: 1 1 auto; min-height: 0;` の縦 flex コンテナで、中の `.desk-screen`（テール背景+アイコン+ウィンドウ、`flex: 1 1 auto`）と `.desk-taskbar`（`flex: none`）がぴったり積み重なる。どちらも `.desk-screen` の外の実サイズぴったりに収まるので、ページレベルのスクロールバーや余白は出ない。

760px 以下のナロー幅では `.swing-shell` は（他画面と同じく）ナビが上に来る縦積みに変わる。`body[data-view="desktop"] .swing-page-footer` はこの画面のときだけ非表示にした（footer 分の高さがあると、ページ自体のスクロール＋ウィンドウ内側のスクロールの二重スクロールになり、下に空白も出るため）。`.swing-shell` の高さは `100dvh`（モバイルブラウザのアドレスバー分だけ `100vh` より縮む単位。未対応ブラウザ向けに `100vh` へフォールバック）にし、`.swing-main` を `flex: 1 1 auto` にして、ナビの下の残り全部を `#view-desktop` が埋める（`min-height: 480px` はさらに極端に低いビューポート向けの最終フォールバック）。ページ自体はスクロールせず、`.desk-page` の iframe の中だけがスクロールする。

ウィンドウの中身（`#desk-page` の iframe）は「リンク集」ページのパスティーシュで、`<html lang="ja">` の別ドキュメント（`desktop-page.html`）として読み込む。UI 表示言語設定に関わらず画面に見える文字列は全部固定の日本語（後述「表示言語」）。

- データ: `cache.sites`（`sites.js` と共有）を [`/api/sites`](http-api.md#get-apisites) から取得し、`accounts[].sites[]` と `unfollowed.accounts[].sites[]` の両方から `stored === true` の版だけを集めて 1 本のリストにする（自分のアカウントが follow set に入っていれば `accounts` 経由で同じリストに含まれる。Sites 画面と同じ集計をそのまま再利用するだけで、Desktop 側で自分/他人を区別しない）。`created_at` 降順（新しい順）で並べる。
- 各行のステータスアイコン（`.desk-status-icon`、`data-status`）は次の優先順で 1 個だけ選ぶ。`title`/`aria-label` は `desktop.js` の `DESK_TEXT`（固定の日本語。Sites 画面の `nip05DescMismatch`/`nip05DescError` とは文言は同じでも別の文字列で、i18n の共有キーは参照しない）:
  1. `nip05 === "mismatch"` → `data-status="ng"`（赤地に `✕`。`DESK_TEXT.iconMismatchDesc`）
  2. `nip05 === "error"`、またはこの 5 つ（`mismatch`/`error`/`not_applicable`/`null`/`"verified"`）のどれにも一致しない未知の文字列 → `data-status="err"`（灰色地に `?`。`DESK_TEXT.iconErrorDesc`）
  3. 上記に当たらない（`null`・`"verified"`・`"not_applicable"`）場合は更新の新しさで決める: `created_at` から 7 日以内 → `data-status="new"`（赤字に `NEW`、点滅。`DESK_TEXT.iconNewDesc`）。30 日以内 → `data-status="up"`（青地に `UP`。`DESK_TEXT.iconUpDesc`）。それ以外 → `data-status="default"`（緑地に `★`。`DESK_TEXT.iconDefaultDesc`）。`not_applicable`（`d` がドメイン形でなく検証しようがない）はこの新しさ判定に普通に乗る — 警告ではないので NIP-05 の問題扱いにはしない
- `.desk-status-icon` はどの `data-status` でも同じ外枠（36×16px）を持つので、行ごとにアイコンの見た目が違っても日付・タイトルの開始位置は揃う。`.desk-link-date` も固定幅（72px）+ `font-variant-numeric: tabular-nums`。メッセージ行のインデント（38px）はこのアイコン+隙間の幅に合わせてある。
- 日付は `2026.09.19` 形式（`created_at` をローカル時刻で整形）。
- タイトルは自己申告の `title`（サニタイズ済み、最大 120 文字）があればそれを、無ければ `d` を表示する。`title` を出すときは必ず `d` も括弧付きの小さい文字で併記する（`title` は未検証の自己申告のため）。
- リンク先: `gateway_url` があればそれを一次リンクにし、`url` もあれば `[本家]` という第二リンクを追加する。`gateway_url` が無ければ `url` を一次リンクにする。どちらも無ければリンクにしない（プレーンテキスト）。リンクは `util.js` の `maybeLink()`（`target="_blank" rel="noopener noreferrer"`、`^https?://` のみ）をそのまま使う。npub・cid・size・replicas は表示しない。
- メッセージ（`message`）はサニタイズして「」で括って表示し、無ければ省略する。
- 一番新しい更新はウィンドウ内のマーキー（`.desk-marquee`、CSS アニメーション。位置を 1px に丸める話は上記「フォント」。`prefers-reduced-motion: reduce` で静止）にも出す。何も保存していなければ「工事中」寄りの空メッセージになる。
- マーキーの下にはサイト内メニュー（`.desk-sitemenu`、`<nav>`）を置く。当時のリンク集ページによくある「`[ トップ ] [ プロフィール ] [ 日記 ] [ 掲示板 ] [ リンク集 ]`」形式で、`リンク集` だけが現在地（`.desk-sitemenu-current`、太字・紺色、非リンク）。残り 4 項目は未実装のページで、`<a>` にはせず（デッドリンクを作らない）、それぞれ工事中サインの小さいドット絵アイコン（`icon-desk-construction`、16×16、黄色い三角に黒い「!」、`.desk-sitemenu-icon`）付きのプレーンテキストで表示し、下にゆれ子の声で断り書き（`.desk-sitemenu-note`: 「トップ・プロフィール・日記・掲示板は工事中です。もうしばらくお待ちください m(_ _)m」）を添える。カーソルは `cursor: default` のまま、`aria-hidden` にはしない（意味のある文言なので読み上げは妨げない）。旧 `.desk-construction-bar`（縞模様バーに「工事中」とだけ書いた飾り）は撤去した。
- 読み込み中・エラー・空はどれも `#desk-page-status`（`data-kind="loading"|"error"|"empty"`）にレトロな文面で出すが、実際の技術的な詳細（`describeError` の結果）もエラーメッセージに含める。
- 来訪者カウンタ（`.desk-counter`）は `localStorage["swing:desktop:visits"]`（アプリ起動＝ページ読み込みのたびに +1、`DesktopView.init()` で加算）に、保存中サイト数から求めた基準値（`1000 + サイト数 * 37`）を足して 6 桁ゼロ埋めで表示する。永続化はブラウザの localStorage のみで、サーバには送らない。
- フッタのバナー（`.desk-banner`）は CSS ではなく実物の 88×31 画像（`/desktop-banner`、`<img>`、`image-rendering: pixelated`）。設定で差し替えられる（下記「リンク集ページ（iframe）」）ので、ルートは拡張子を持たない。右クリックから保存でき、「バナーはご自由にお持ち帰りください」の一文がそのまま成り立つようにしている。リンクにはしていない（本家に飛ばず、クリックしても何も起きない）。同梱の画像は「JOIN SWING NETWORK!」のアニメーション GIF（181 コマ・約 16.5 秒ループ・86KB）。`web/desktop-banner.gif` として repo に置き、`assets.rs`（`include_bytes!`）+ `mod.rs`（`GET /desktop-banner`、`image/gif`）で配信する（詳細は [`dashboard.md`](../dashboard.md#静的ファイルの配信srcdashboardassetsrs)）。GIF のアニメーションは CSS で止められないので、ページの `prefers-reduced-motion` 対応（マーキー・NEW アイコン）の対象外になる。

### リンク集ページ（iframe）

ページ本文は `#desk-page`（枠だけの `<div>`）の中の `#desk-page-frame`（同一オリジンの `<iframe src="/desktop-page.html">`）で開く。別ドキュメントなので、ダッシュボードの CSS（`style.css`・`desktop.css`・`/custom.css`・ユーザー CSS）はこのページに一切当たらず、ページ側の CSS も外に漏れない。継承も CSS 変数も越えない。これを成立させるため、ガードのヘッダだけは自分自身からの埋め込みを許してある（`frame-ancestors 'self'` / `X-Frame-Options: SAMEORIGIN`。[`dashboard.md`](../dashboard.md#ガードsrcdashboardguardrs)）。

3 ファイルとも起動時に差し替えられる（`[dashboard].desktop_page` / `desktop_page_css` / `desktop_banner`。[`dashboard.md`](../dashboard.md#設定dashboard)）:

| ルート | 既定 | 中身 |
|---|---|---|
| `/desktop-page.html` | `web/desktop-page.html` | ページ本体。`desktop-page.css` を `<link>` で読み、工事中アイコンの `<symbol>` を自前で持つ |
| `/desktop-page.css` | `web/desktop-page.css` | そのページ専用の CSS。`:root` の `--desk-*`・`@font-face`・`box-sizing` のリセットまで自己完結 |
| `/desktop-banner` | `web/desktop-banner.gif` | フッタの 88×31 バナー |

差し替えの対象外が 1 つだけある。`/desktop-frame.css`（`web/desktop-frame.css`）は窓の持ち物（今はスクロールバーだけ）で、ページの HTML が何であっても `desktop.js` が `head` の先頭に差し込む。「外のスタイルは中に入らない」の唯一の例外で、OS 側の見た目をページ作者の責任にしないための線引き。

`desktop.js` はこのページのスクリプトを前提にしない（ページ側に JS は無い）。親から `iframe.contentDocument` を引いて、決まった `id` を見つけたときだけ書き込む。差し替えるページはこのうち要るものだけ置けばよく、無い `id` は黙って飛ばす:

| `id` | 書き込む内容 |
|---|---|
| `desk-link-list` | リンク行（`.desk-link-row` 以下、上記「Desktop 画面」の構造）を `replaceChildren` で流し込む |
| `desk-page-status` | 読み込み中・エラー・空の文面（`data-kind="loading"\|"error"\|"empty"`） |
| `desk-marquee-text` | マーキーの文面（最新更新 1 件、または空メッセージ） |
| `desk-counter` | 来訪者カウンタの 6 桁 |

ウィンドウ枠側（タイトルバー・ツールバーの「更新」・ステータスバー・タスクバー）は親ドキュメントのままなので、差し替えても操作系は影響を受けない。ページ内のクリックはデスクトップアイコンの選択解除として親にも伝える（iframe の document に click ハンドラを付けている）。リンクは `util.js` の `maybeLink()` が付ける `target="_blank"` のままで、iframe の中で遷移してページを失うことはない。

## Sites 画面

並び順（`.swing-sort-switch`、`localStorage["swing:sites:sort"]`、既定 `updated`）:

- `updated`: 各アカウントの最初のサイトを `created_at` 降順で比較（アカウント内のサイトも同基準）。サイト無しは最後。
- `name`: 最初のサイトの `d` のロケール順（`localeCompare`）。サイト無しは最後。
- `pubkey`: 画面に出す `npub` の文字列順（`/api/sites` は hex 順で返すが、bech32 の順とは一致しない）。

NIP-05 の検証結果はバッジで `OK`（`verified`）/ `NG`（`mismatch`）/ `ERR`（`error`）/ `N/A`（`not_applicable`）と短く表示し、意味は `title` 属性（マウスオーバー）に表示言語で出す。カードはサイト名の下の行に保存状態・NIP-05・レプリカ数のバッジ（`.swing-site-badges`）をまとめ、NIP-05 には `nip05: ` を前に付け、テーブルでは NIP-05 列にラベルだけを出す。

サイズの表示（`util.js` の `formatSiteSize`、カードのメタ行・テーブルの Size 列の両方で使う）: `stored_size`（`state.json` に記録済みの実測値。保存時の `dag/stat` の結果で、この表示のために Kubo を呼び直すことはしない）があればそれをそのまま `formatBytes` で出す。無ければ `size`（イベントの自己申告）を `(12.3 MB)` のように括弧書きで出す。ラベルは付けない。「未確認の自己申告である」ことは同じ行の保存状態バッジ（`[not stored]`）がすでに示しているため。どちらも無ければ `–`。版どうしで共有するブロックを差し引いた重複排除済みの合計はここには出ず、Storage check（`/api/status` の `sites[].actual`）だけが持つ。

`Unfollowed but still stored` セクションも同じ並び順ロジックを共有する。「Stored only」チェックボックスの状態は `localStorage["swing:sites:stored-only"]`。

## Publish 画面

- My sites: 一覧の「Use」ボタンは `site`・`url`・`title`（`message` を除く）をフォームに入れるだけ。
- publish フォームは常にフォルダアップロード（`<input type="file" webkitdirectory multiple>`）。ファイル数・合計サイズを表示し、`max_upload` を超えれば送信ボタンを無効化する。送信は `XMLHttpRequest` で、進捗を `.swing-progress`/`.swing-progress-bar`（`data-state`）に反映する。413 は「上限を超えた」という文言に言い換える。
- 各ファイルの送信名は `webkitRelativePath` から選んだフォルダ名を除いたもの。最後に使ったフォーム内容は `swing:publish:last` に保存する（下記の localStorage 一覧を参照）。

## Webring 画面

- root/depth のクエリは `swing:webring:query` に保存し、次に開いたときに復元する。
- 再取得中、既存の表示は消さず `aria-busy="true"` で薄く表示する。初回だけ「Loading webring…」になる。
- ノード詳細パネルのミラー操作は選んだノードが自分自身かどうかで変える（自分自身: ボタン無し／ミラー済み: 削除ボタン／未ミラー: 追加ボタン）。判定はキャッシュ済みの `/api/sites` か `/api/mirror` の pubkey 集合。
- `beyond`（深さの上限外で表示していないアカウント数）と `over_budget`（クロールの上限に達して到達できなかったアカウント数）は、どちらも 0 より大きければ画面下部にヒント文を 1 行ずつ出す。`list` 表示では `referencing`（起点を名指ししているだけでクロールには加えていないアカウント）を「Mutual」「One-way」と並ぶグループとして出し、`more` が 0 より大きければ末尾に件数のヒント文を添える（[「レプリカ報告の信頼度」](../../architecture.md#レプリカ報告の信頼度replicastier)）。ノード詳細パネルの報告者一覧は npub の後ろに信頼度の tier（`author`/`chosen`/`other`）に応じたタグを付け、`site.dropped` が 0 より大きければ末尾に「…and N more」相当のヒント文を出す（[取得と表示の上限](../../architecture.md#取得と表示の上限nostrbudget)）。

### グラフ（`web/graph.js`）

外部ライブラリを使わない自前の force-directed layout。ドラッグでノードを固定でき、クリックで選択して詳細パネルを開く。キーボード操作（Tab で移動、Enter/Space で選択）に対応する。パン・ホイールズーム（0.15〜4 倍）と全体表示（Fit）ができ、`prefers-reduced-motion: reduce` ではアニメーションせず同期的に 1 回だけ描画する。ノード・辺は class と `data-*` だけを持ち、色は付けない（配色は CSS 側、下記参照）。

## 共通の UI 部品

- busy 表示: `setBusy(button, bool)` で `disabled`・`aria-busy`・`.is-busy` を切り替える。ボタン幅は変わらない。`prefers-reduced-motion: reduce` ではスピナーを止め静止表示にする。
- コピー: 成功で 1.5 秒だけ `data-copied="true"`、失敗で `data-copy-failed="true"`。表示文字列はすべて共通の `copy` キーで、対象の違いは `aria-label` 側で表す。
- ボタン: `.swing-btn`（主要操作）、`.swing-btn-small`（行単位）、`.swing-copy-btn`（インラインコピー）、`.swing-icon-btn`（アイコンのみ）。
- ロゴ: サイドナビ上端の `.swing-logo`（インライン SVG、26px）。色は CSS で付け、本体 `.swing-logo-body` は `--swing-text`、中央と両端の丸 `.swing-logo-node` は `--swing-accent`。`viewBox` は図形の外接矩形に合わせた正方形。
- ロゴタイプ: ロゴの右の `.swing-logotype`（インライン SVG、高さ 26px・幅は縦横比なり）。横縞の文字は `.swing-logotype-body` が `--swing-text`、2 つの丸は `.swing-logotype-node` が `--swing-accent` で、ロゴと同じ 2 トーン。縞は 2px 未満なので本体だけ `shape-rendering: crispEdges` にする。アクセシブルな名前は `role="img"` と `aria-label="SWING"`。
- サイドナビ: 各項目はアイコン（`index.html` 冒頭の SVG `<symbol>`。Desktop `icon-desktop` モニタ、Sites `icon-sites` 星、Webring `icon-webring` 輪、Publish `icon-publish` ロケット、Settings `icon-settings` 歯車。線幅 1.75 の stroke で揃え、現在の画面は 2）＋ラベル。下端右寄せの枠付きトグル（`#nav-toggle`）で畳むとアイコンだけの幅（`--swing-nav-collapsed-width`）になる。畳んでもアイコン・ロゴの横位置は変わらず、幅だけが縮み、ラベル・ロゴタイプ・フッタは不透明度 0 になって切り取られる（DOM には残るのでスクリーンリーダーには読まれる）。畳んだ状態では各リンクの `title` にラベルを入れる。状態は `localStorage["swing:nav:collapsed"]` に保存し、`<body data-nav="collapsed">` で表す。
- 読み込み時: `<body>` 直後の通常の（module でない）`<script src="/boot.js">` が同期的に `data-nav` を付ける。最初の描画から畳んだ幅になるので、読み込み時にアニメーションしない。`app.js` の初期化はトグルのラベルと `title` を整える。
- 表示言語が英語以外に決まるとき（判定は `i18n.js` の `currentLang` と同じ）、`boot.js` が `<html lang>` と `<html data-i18n-pending>` を付け、`[data-i18n]` の要素を `visibility: hidden`、`[data-i18n-placeholder]` のプレースホルダを透明にする。`app.js` が静的な訳を当てた直後にこの属性を外す。`app.js` が動かなかった場合でも、1 秒後に CSS アニメーションで英語のまま表示される。
- モバイル幅: 760px 以下ではナビを横並びにしてトグルとサイドナビのフッタを隠し（畳んだ状態でもラベルを表示する）、ページ最下部の `<footer id="page-footer">` にフッタと同じ内容を表示する。

## localStorage キー一覧

ダッシュボードが使う `localStorage` のキーはこれで全部（他にサーバに送るものは無い）。

| キー | 値の形 | 意味 |
|---|---|---|
| `swing:style:<view>`（`view` は `sites`/`webring`） | 文字列（スタイル名） | 画面ごとの表示スタイル |
| `swing:sites:sort` | `updated` / `name` / `pubkey` | Sites の並び順（既定 `updated`） |
| `swing:sites:stored-only` | `"1"` / `"0"` | Sites の「Stored only」チェックボックスの状態 |
| `swing:webring:query` | JSON `{root, depth}` | Webring の最後のクエリ（起動時に復元） |
| `swing:publish:last` | JSON `{site, url, title, message, nip05}` | Publish フォームの最後の入力（起動時にプリフィル） |
| `swing:nav:collapsed` | `"1"` / `"0"` | サイドナビを畳んでいるか（既定 `0`） |
| `swing:theme` | `auto` / `light` / `dark` | 表示テーマ |
| `swing:lang` | `auto` / `en` / `ja` | 表示言語 |
| `swing:user-css` | 文字列（CSS） | Settings のカスタム CSS 欄の内容 |
| `swing:desktop:visits` | 整数の文字列 | Desktop 画面の来訪者カウンタ。アプリ起動（ページ読み込み）のたびに +1 する |

## 表示言語（i18n）

`web/i18n.js` の `MESSAGES = { en: {...}, ja: {...} }` を `t(key, vars)` で参照する。`localStorage["swing:lang"]`（`auto`/`en`/`ja`。`auto` は `navigator.language` が `ja` で始まるかで判定）で切り替え、再読み込みは不要。訳が無いキーは英語にフォールバックする。

訳さないもの: ナビゲーションの「Webring」、webring の ASCII/DOT/Mermaid 出力、API のエラー文字列、npub・hex・CID・パス、環境変数名・設定キー名、`nip05`/`health` のステータス値、NIP-05 モードの `off`/`warn`/`require`。日本語訳は「mirror」を指す語をすべて「ミラー」に統一している。日時表示は `Intl.DateTimeFormat`（`ja-JP`/`en-US`）を使う。

### Desktop 画面は丸ごと日本語固定

`#view-desktop` の中で画面に見える文字列は、UI 表示言語の設定（`swing:lang`）に関わらず全部固定の日本語。90 年代〜00 年代前半の日本の個人ホームページのパスティーシュなので、UI 言語だけ英語に切り替わって中身が混ざると成立しないための決定。対象は「リンク集」本文（見出し・マーキーの固定文言・サイト内メニュー・カウンタの前後の文・リンクフリー表記などは `desktop-page.html`、`[本家]` ラベルは `desktop.js`）だけでなく、読み込み中・エラー・空状態の文面、ステータスバーの「完了」「エラー」「読み込み中…」、ウィンドウ/タスクボタン/ステータスアイコンの `title`・`aria-label` も含む。

これらの文字列は `i18n.js` の `MESSAGES` には置かず、`desktop.js` 内の `DESK_TEXT`（ページ本文寄りのものはゆれ子の一人称の言葉遣い、ブラウザ/OS のクロームに当たるものは中立的な日本語）と `index.html`・`desktop-page.html` に直書きした固定テキスト／`aria-label`/`title` 属性で持つ。i18n 側にあった `desk*`/`desktop*` キー（`deskLoading`・`deskDoneStatus`・`deskErrorStatus`・`deskError`・`deskEmpty`・`deskMarqueeEmpty`・`deskMarqueeLatest`・`deskIconNewDesc`・`deskIconUpDesc`・`deskIconDefaultDesc`・`deskVisitorCountAriaLabel`・`deskIconExplorerLabel`・`desktopMinimize`・`desktopMaximize`・`desktopRestore`・`desktopClose`）は `en`/`ja` 両方から削除した（サイドナビのラベル `navDesktop` だけは他画面と同じ扱いで i18n のまま残す）。ステータスアイコンの `title`/`aria-label` は Sites 画面が使う共有キー `nip05DescMismatch`/`nip05DescError` を流用せず、`DESK_TEXT.iconMismatchDesc`/`iconErrorDesc` という Desktop 専用の文字列を持つ（共有キー自体は削除していない。Sites 画面が使う）。

ARIA ラベルも同じ理由で固定日本語にそろえている（`aria-label` だけ i18n・本文は固定日本語、という使い分けは一貫性がなく複雑になるため）。ウィンドウの最小化/最大化・元に戻す/閉じるボタン、SWING Explorer アイコン、来訪者カウンタの `aria-label` はすべて `index.html`・`desktop-page.html` に直書きの固定日本語（最大化⇔元に戻すの状態切り替えだけは `desktop.js` の `updateMaxGlyph` が `DESK_TEXT.maximize`/`DESK_TEXT.restore` で書き換える）。この結果 `desktop.js` はもう `swing:langchange` を購読しない（`updateMaxGlyph` は状態変化でだけ呼ばれる）し、`app.js` の `applyLanguage()` も `DesktopView.render()` を呼ばなくなった（言語が変わっても Desktop 画面の見た目は変わらないため）。

## CSS カスタマイズのインターフェース

読み込み順は `style.css` → `desktop.css`（Desktop 画面のウィンドウ枠専用、常に読み込む） → `/custom.css`（サーバ設定、[`dashboard.md`](../dashboard.md#静的ファイルの配信srcdashboardassetsrs)） → `<style id="user-css">`（ブラウザの localStorage、後勝ち）。Desktop 画面のレトロパレットは `desktop.css` の `#view-desktop` セレクタにローカル変数（`--desk-*`）として直書きしてあり、`style.css` の `--swing-*` 変数（テーマ・カスタム CSS）を参照しない。この 4 つはどれもリンク集ページ（iframe）には届かない。そちらを変えたいときは `[dashboard].desktop_page_css` でファイルごと差し替える（上記「リンク集ページ（iframe）」）。iframe のドキュメントに入るのは `desktop-page.css`（差し替え可）と、`desktop.js` が差し込む `desktop-frame.css`（差し替え不可、先に入る）の 2 つだけ。

既定は白黒中立基調＋アクセント 1 色の配色。アクセントはテーマごとに色相が違い、ダークは黄緑の `--swing-dark-accent: #7dff3c`、ライトはリンク色の濃い青 `--swing-accent: #0645ad`。`--swing-ok` はダークのアクセントと見分けられるようにティール（ライト `#2f7a6e`・ダーク `#6fc9b8`）。`--swing-root`（webring の root ノードの色）は `var(--swing-accent)` を参照するので、アクセントを変えるだけで揃って変わる。

- CSS 変数（`web/style.css` の `:root`）: `--swing-bg` `--swing-surface` `--swing-surface-alt` `--swing-surface-raised` `--swing-border` `--swing-border-strong` `--swing-text` `--swing-text-muted` `--swing-text-faint` `--swing-accent` `--swing-accent-strong` `--swing-accent-soft` `--swing-warn` `--swing-warn-soft` `--swing-danger` `--swing-danger-soft` `--swing-ok` `--swing-ok-soft` `--swing-root` `--swing-focus` `--swing-font-display` `--swing-font-ui` `--swing-font-mono` `--swing-space-1`〜`--swing-space-6` `--swing-radius` `--swing-radius-lg` `--swing-nav-width` `--swing-nav-collapsed-width`（既定 64px） `--swing-graph-label-size`（既定 11px）。
- テーマ: 既定は `@media (prefers-color-scheme: dark)` に連動。`<html data-theme="light"|"dark">` で上書き（Settings 画面が `localStorage["swing:theme"]` に保存してこの属性を付け替える）。
- 状態フック: `<body data-view="desktop|sites|webring|publish|settings" data-style="<現在の表示スタイル>" data-nav="collapsed">`（`data-nav` は畳んでいるときだけ）。Desktop 画面は `body[data-view="desktop"] .swing-main` に `max-width: none; padding: 0;` を当てて画面いっぱいに広げる。
- 安定 class（抜粋。`swing-` 接頭辞で統一）: レイアウト系 `swing-shell` `swing-nav` `swing-nav-list` `swing-nav-icon` `swing-nav-label` `swing-nav-toggle` `swing-main` `swing-view` `swing-panel` `swing-toolbar` `swing-style-switch` `swing-sort-switch`。Sites 系 `swing-site` `swing-site-row` `swing-site-badges` `swing-site-meta-cid` `swing-site-meta-info` `swing-account`。共通部品 `swing-badge` `swing-btn`（`swing-btn-accent`/`swing-btn-danger`/`swing-btn-small`）`swing-copy-btn` `swing-icon-btn` `swing-status` `swing-hint` `swing-table` `swing-mono` `swing-pre` `swing-relay-results` `swing-page-footer`。Webring 系 `swing-graph` `swing-node` `swing-edge` `swing-arrow-oneway-fill` `swing-arrow-mutual-fill` `swing-legend` `swing-webring-layout` `swing-node-detail` `swing-source-block`。Publish 系 `swing-my-sites` `swing-progress` `swing-progress-bar` `swing-identity`。Desktop 系（`desk-` 接頭辞。`swing-` とは意図的に分けている。全クラスは `#view-desktop` の中でだけ使う）: `desk-screen` `desk-icons` `desk-icon`（`desk-icon-shortcut` は実機能を持つ「SWING Explorer」アイコンだけに付く。選択中は `is-selected`） `desk-window` `desk-titlebar` `desk-titlebar-text` `desk-titlebar-btns` `desk-tbtn`（`desk-tbtn-min`/`desk-tbtn-max`/`desk-tbtn-close`。全部実際の `<button>`） `desk-glyph`（`desk-glyph-min`/`desk-glyph-max`/`desk-glyph-restore`/`desk-glyph-close`） `desk-menubar` `desk-toolbar` `desk-toolbtn`（`#desk-reload` だけ実際の `<button>`。ホバーで平面→ベベルに変わる） `desk-toolbtn-icon` `desk-address-row`（`desk-address-label` + `desk-address-bar` を持つ、ツールバーとは別行） `desk-page`（iframe の枠） `desk-page-frame`（iframe 本体）。ここから下の本文側は iframe の別ドキュメントにある: `desk-page-inner` `desk-hero` `desk-rainbow-hr` `desk-marquee` `desk-marquee-track` `desk-sitemenu`（`desk-sitemenu-list` + `desk-sitemenu-item`（`desk-sitemenu-current` は現在地） + `desk-sitemenu-icon` + `desk-sitemenu-note`） `desk-panel` `desk-page-status` `desk-link-list` `desk-link-row` `desk-status-icon` `desk-link-date` `desk-link-title` `desk-link-d` `desk-link-honke` `desk-link-message` `desk-footer` `desk-counter` `desk-banner`（88×31 の `<img>`） `desk-statusbar`（`desk-status-text` + `desk-zone` + `desk-grip`） `desk-resize`（`desk-resize-n`/`-s`/`-e`/`-w`/`-ne`/`-nw`/`-se`/`-sw`。当たり判定のみで見た目は持たない） `desk-taskbar` `desk-start` `desk-start-icon` `desk-taskbtn`（実際の `<button>`。表示中は `is-active`） `desk-taskbtn-icon` `desk-tray` `desk-tray-icon`。ドラッグ・リサイズ中は `document.body` に一時的に `desk-no-select` が付く（`#view-desktop` の外まで含めた保険の選択禁止）。SVG `<symbol>` は上記のクラスとは別に `index.html` 冒頭にまとめてある: `icon-desk-mycomputer`（マイ コンピュータ、線画）`icon-desk-trash`（ごみ箱、線画）`icon-desk-swing`（SWING Explorer、S+E モノグラムの白一色）`icon-desk-swing-color`（タイトルバー/タスクボタン、同じ S+E を金と青のグラデーションで）`icon-desk-start`（Start ボタン、SWING のマーク 3 色）`icon-desk-tray-volume`/`-network`/`-mirror`（トレイ、ドット絵）`icon-desk-tb-back`/`-forward`/`-stop`/`-reload`/`-home`（ツールバー、ドット絵）`icon-desk-construction`（サイト内メニューの工事中サイン、ドット絵）だけはページ本文で使うので `desktop-page.html` の側に置いてある。
- 状態は data 属性: `data-stored="true|false"`、`data-nip05="verified|mismatch|not_applicable|error"`、`data-health="ok|missing|cid_mismatch|incomplete|check_failed|invalid_key"`、`data-ok="true|false"`（relay 結果）、`data-kind="loading|error|empty"`（`swing-status`・`desk-page-status`）、`data-root`/`data-has-follow-set`/`data-depth`/`data-selected`（グラフのノード）、`data-mutual`（グラフの辺）、`data-style-value`/`data-sort-value`（切替ボタン自身の値）、`data-mirrored="true"`、`data-detail="true|false"`（詳細パネル表示中か）、`data-copied`/`data-copy-failed`、`data-state="uploading|processing|done|error"`（`swing-progress`）、`aria-busy="true"`（busy 中のボタン、再取得中の webring 表示領域）、`data-status="ng|err|na|new|up|default"`（`desk-status-icon`、Desktop のリンク行のステータスアイコン）。
- SVG グラフは class と `data-*` だけを付け、色は JS に書かない（`style.css` 側で `.swing-node[data-root="true"] circle { ... }` のように当てる）。Desktop 画面のレトロパレットは逆に、テーマに連動させない意図があるため `desktop.css` に直接色を書く（上記「CSS カスタマイズのインターフェース」を参照）。
