# ダッシュボードの Desktop 画面（`web/desktop*.js`）

[`../dashboard.md`](../dashboard.md) の子ページで、[`web.md`](web.md) と並列。ダッシュボード全体の構成・ルーティング・他画面は [`web.md`](web.md)、サーバ側は [`../dashboard.md`](../dashboard.md)、おしらせは [`notices.md`](notices.md)。子ページは [`mascot.md`](mascot.md)（マスコット）と [`desktop/control-panel.md`](desktop/control-panel.md)（コントロール パネル）。

## 構成

Desktop 画面専用のモジュール（各モジュールは表で自分より上のものだけに依存する。[`web.md#構成`](web.md#構成) の共通モジュールにも依存する）:

| ファイル | 役割 |
|---|---|
| `desktop-focus.js` | ウィンドウ/ダイアログの登録（`registerFrame`）とタイトルバーのアクティブ表示の同期（`scheduleActiveSync`）、開いているモーダルの判定（`isModalOpen`）、Tab の折り返し（`tabAcrossEdge`）、フォーカスを失ったときの戻し先の補助 |
| `desktop-scale.js` | 表示倍率の整数倍への補正と左上のデバイスピクセルへの位置合わせ（`initDeskScale`）、ビューポート座標から `#view-desktop` 内の座標への換算（`toDeskPx`）、リンク集ページ（iframe）の中の座標からトップの文書のビューポート座標への換算（`frameToViewport`） |
| `desktop-drag.js` | ウィンドウ/ダイアログのドラッグ・リサイズで共有するポインタ操作と座標のクランプ |
| `desktop-combobox.js` | Win95 風コンボボックス（`createCombobox`）。囲むウィンドウのタイトルバーを押したときにも一覧を閉じる |
| `desktop-wallpaper-image.js` | 壁紙の色・画像の量子化とダウンスケール（ファイルの読み込み・canvas への描画を含む） |
| `desktop-dialog.js` | 任意のモーダルダイアログの共通の殻（`createDialog`。開閉・オーバーレイのクリックでのタイトルバー明滅・ドラッグ・Tab トラップ・フォーカス復帰）と、各タブが使う保存失敗の表示（`showStorageError`） |
| `desktop-window.js` | 「SWING Explorer」ウィンドウの移動・リサイズ・最小化・最大化・ジオメトリ計算（`DesktopWindow`） |
| `desktop-wallpaper.js` | 「コントロール パネル」の「背景」タブ本体（`WallpaperPage`）。保存キーもここで定義する |
| `desktop-system-settings.js` | 「コントロール パネル」の「システム」タブ本体（`SystemSettingsPage`）と、パスなしで開いたときの画面を返す `startupView()`（`app.js` が使う）。保存キーもここで定義する |
| `desktop-updates.js` | 更新の確認（`createUpdateWatcher`）。[`notices.md#更新の確認`](notices.md#更新の確認) |
| `desktop-notify.js` | ブラウザの通知（`createBrowserNotifier`）。[`notices.md#おしらせの出し分け`](notices.md#おしらせの出し分け) |
| `desktop-mascot-pack.js`・`desktop-mascot-sprite.js`・`desktop-mascot-behavior.js`・`desktop-mascot-balloon.js`・`desktop-mascot.js`・`desktop-mascot-settings.js` | マスコット（分担は [`mascot.md#概要`](mascot.md#概要)）。`desktop.js` が `DesktopMascots` の `init`・`start`・`onShow`・`watchFrame`・`showing` を呼ぶ |
| `desktop-notify-settings.js` | 「コントロール パネル」の「通知」タブ本体（`NotifySettingsPage`） |
| `desktop-settings.js` | 「コントロール パネル」の殻（`PAGES` を `desktop-dialog.js` に結び付ける、`DesktopSettings`） |
| `desktop.js` | 画面のロジック本体（デスクトップアイコンの選択・起動、リンク集ページへの書き込み、共通のクリック/フォーカス処理） |

そのほかのファイル:

| ファイル | 役割 |
|---|---|
| `desktop.css`・`desktop-dialog.css`・`desktop-wallpaper.css`・`desktop-mascot-settings.css`・`desktop-system-settings.css`・`desktop-mascot.css` | `index.html` がこの順に読む CSS（それぞれフォント・シェル・`--desk-*` 変数、ダイアログの枠・汎用部品（無効状態を含むチェックボックス）、「背景」タブ固有、「マスコット」タブ固有、「システム」タブ固有、マスコットと吹き出し）。読み込み順は [`css.md#読み込み順`](css.md#読み込み順) |
| `desktop-frame.css` | 窓側のスクロールバーの見た目。`desktop.js` がリンク集ページ（iframe）に差し込む（下記「[リンク集ページ（iframe）](#リンク集ページiframe)」） |
| `desktop-page.html`・`desktop-page.css`・`desktop-banner.gif` | リンク集ページ・その専用 CSS・88×31 バナーの同梱版。設定で差し替えられる（[`../dashboard.md#設定dashboard`](../dashboard.md#設定dashboard)） |
| `desktop-icons.svg` | ピクセルアートアイコンのスプライト。`index.html` から `<use>` で参照する |
| `fonts/` | 同梱フォント PixelMplus12 とそのライセンス |
| `mascots/` | 同梱のマスコットのパック（`yureko/`・`mochi/`・`neko/`。`index.json` は静的ファイルではなく実行時に生成する。[`mascot/pack.md`](mascot/pack.md)） |

## 画面

Win95/98 風デスクトップ（背景・デスクトップアイコン・タスクバー・Start 風ボタン・時計）と、その上の「SWING Explorer」ウィンドウ（タイトルバー・メニューバー・ツールバー・アドレス行・ページ本文・ステータスバー）を `#view-desktop` に静的 HTML で組む。実際に操作できる要素と装飾の切り分けは下記「装飾 vs 実機能」を参照。配色はダッシュボードのテーマ（`--swing-*`）を参照せず、`#view-desktop` スコープの固定レトロパレット（`--desk-*`）を使う（ライト/ダークで見た目は変わらない）。

### フォント

PixelMplus12（`web/fonts/`）を使う。フォントの読み込みが終わるまで `.desk-window` を隠し、Desktop 画面を初めて表示したときだけ待つ（上限あり）。

### 表示倍率

`#view-desktop` の 1 CSS px が常に整数個のデバイスピクセルになるよう、`desktop-scale.js` が `#view-desktop` の CSS `zoom` を決める。

- 整数倍率 `n = max(1, floor(devicePixelRatio))` に対して `zoom = n / devicePixelRatio`。125%・150% は 1 倍、200%〜299% は 2 倍で描く（100% 未満も 1 倍）。ブラウザのズームと OS の表示スケールのどちらにも効く。
- リンク集ページ（iframe）: 中の `devicePixelRatio` が整数倍率で、中の文書の高さ（`documentElement.clientHeight`）が `innerHeight` から縮んでいなければ何もしない。どちらかが崩れていれば、iframe 要素に `zoom = 1 / 補正値` をかけて親の `zoom` を打ち消し、iframe の `<html>` に同じ補正値の `zoom` をかける。
- `#view-desktop` と iframe の左上がデバイスピクセルの境目に来るよう、両方の `left`/`top` を 1 デバイスピクセル未満だけずらす（このずらしのため `.desk-page-frame` は `position: relative`）。
- 倍率の変化・大きさの変化・iframe の読み込みのたびにやり直す。

`zoom` の内側では `style.left` などの長さは補正前の値、`getBoundingClientRect()` とポインタイベントの座標は補正後の値になる。ドラッグ・リサイズ・コンボボックスのリストの配置は、ビューポート座標を `toDeskPx()` で `zoom` の内側の座標に換算してから使う。スクリーンの大きさ（`desktop-drag.js::screenSize`）は補正前の `clientWidth`/`clientHeight` から取る。

### 装飾 vs 実機能

装飾要素は `aria-hidden="true"` を持ち操作できない（ツールバー/メニューバーの「ボタン」「メニュー」扱いの項目だけホバー表示は効く）。例外として、`aria-hidden` の「マイ コンピュータ」「ごみ箱」アイコンもクリックで選択でき、ステータスバーの文言は読み込み状態に応じて `desktop.js` が書き換える。実際に押せる・操作できるのは次のみ:

| 要素 | 動作 |
|---|---|
| ツールバーの「更新」（`#desk-reload`） | 再取得 |
| タイトルバーの最小化/最大化/閉じる | 最小化・最大化⇄復元・閉じる |
| タイトルバー本体 | ドラッグで移動（ナロー幅では無効）。ダブルクリックで最大化⇄復元 |
| `.desk-resize` ハンドル（8 個） | ウィンドウのリサイズ。最大化中は非表示 |
| タスクバーのタスクボタン | 最小化⇄復元。ウィンドウを閉じると消える |
| デスクトップアイコン（4 個） | シングルクリックで選択（常に 1 つだけ）。「SWING Explorer」「コントロール パネル」はダブルクリック/Enter/Space で開ける。「マイ コンピュータ」「ごみ箱」は選択のみ |
| マスコット・吹き出し | 絵の不透明な画素だけがドラッグで移動、クリックでせりふ（透明な画素のところは下の要素に届く）。吹き出しのリンク・「とじる」（[`mascot.md`](mascot.md)） |

残りは全部装飾（Start ボタン、アドレスバー、ステータスバー、タスクバーの背景、トレイのアイコン+時計）。

### ウィンドウ管理の状態

ジオメトリ・最小化/最大化/クローズの状態はメモリにだけ持つ。他の画面に移って戻っても保つが、ページを読み込み直すと既定（横方向に中央寄せ）に戻る。一度動かした（ドラッグ・リサイズした）ウィンドウとダイアログは、閉じて開き直してもその位置を保つ。ウィンドウの最小サイズは 320×240。760px 以下のナロー幅では常に全画面固定（最小化・閉じるだけは動作）。最小化・閉じるで操作したボタンが隠れるときは、フォーカスをタスクボタン（最小化）か SWING Explorer アイコン（閉じる）へ移す。スクリーン（`#desk-screen`）ははみ出した部分を `overflow: clip` で切り取るだけでスクロールしないので、画面外に出たウィンドウの中の要素にフォーカスが移ってもデスクトップはずれない。

### キーボードフォーカス

SWING Explorer ウィンドウ・「コントロール パネル」ダイアログ・デスクトップアイコン・タスクバーは同じフォーカス表示を共有する。フォーカス表示は実機能コントロールだけに付け、ウィンドウ/ダイアログのアクティブ・非アクティブはタイトルバーの配色で示す（フォーカスを含むもの、モーダルが開いていればそれだけがアクティブ）。フォーカスがウィンドウ/ダイアログの中に入るとアイコンの選択を解除する。Tab は、モーダルが開いていなければ `#view-desktop` の実機能コントロール全体（マスコットの吹き出しのリンクと iframe 内のリンクを含む）の先頭/末尾で、開いていればそのダイアログの中で折り返す。モーダルが開いていないときの Esc はサイドナビの「Desktop」リンクへフォーカスを移す。

### コントロール パネル

「コントロール パネル」ダイアログは [`desktop/control-panel.md`](desktop/control-panel.md)。

### レイアウト

Desktop 画面では `.swing-main` の幅制限と余白を外して画面いっぱいに広げる。ページ全体はスクロールせず、スクロールするのはリンク集ページ（iframe）だけ。760px 以下のナロー幅ではナビが上に来る縦積みになり、ページ最下部のフッタ（`#page-footer`）は出さない。

### リンク集ページ（iframe）

ウィンドウの中身（`#desk-page-frame`）は同一オリジンの iframe で開く `/desktop-page.html` の別ドキュメント。ルートと差し替え設定は [`../dashboard.md#静的ファイルの配信srcdashboardassetsrs`](../dashboard.md#静的ファイルの配信srcdashboardassetsrs)。

- CSS: `index.html` のダッシュボードの CSS は当たらず、ページ側の CSS も外に漏れない。窓側の `desktop-frame.css` だけは `desktop.js` が iframe の `<head>` の先頭に `<link>` で差し込む。ページ専用 CSS は `--desk-*`・`@font-face`・リセットまで自己完結する。
- 読み込みの時機: このページとバナー・マスコットはログインが要る（[`security.md#ガード`](security.md#ガード)）。`index.html` の iframe には `src` を書かず、`app.js` が `/api/overview` の読み込みを終えた後（失敗しても 401 でなければ）に呼ぶ `DesktopView.start()` が `src` を入れ、`DesktopMascots.start()` でパックを読み始める。ログイン画面を出すときは呼ばない。
- `src` の無い間の `about:blank` の `load` は無視する。

iframe には `sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"` を付ける。差し替えたページの `<script>`・イベントハンドラ属性・フォーム送信・親ウィンドウの遷移は動かず、`target="_blank"` のリンクは制限の無い新しいタブで開く。親の `desktop.js` は `contentDocument` を直接読み書きする。

`desktop.js` はページ側の JS を前提にせず、決まった `id` を見つけたときだけ書き込む（無い `id` は黙って飛ばす）。差し替えるページはこの `id` と、`desktop.js` が差し込む要素のクラス・属性を契約として使える:

| `id` | 書き込む内容 |
|---|---|
| `desk-link-list` | リンク行（下表）を `replaceChildren` で流し込む |
| `desk-page-status` | 読み込み中・エラー・空の文面。`data-kind` に `loading` / `error` / `empty` を付ける（リンクを出せたときは文面を消して属性ごと外す） |
| `desk-marquee-text` | マーキーの文面 |
| `desk-counter` | 来訪者カウンタ |

| 差し込む要素 | 内容 |
|---|---|
| `li.desk-link-row` | 1 サイトの行 |
| `.desk-status-icon` | ステータスアイコン。`data-status` は `ng` / `err` / `new` / `up` / `default`（下記） |
| `.desk-link-date` | 日付（`YYYY.MM.DD`） |
| `.desk-link-title` | タイトル（リンクか `span`） |
| `.desk-link-d` | `title` を出すときに併記する `(d)` |
| `.desk-link-honke` | `[本家]` の第二リンク |
| `.desk-link-message` | 更新メモ |

データは `cache.sites`（[`web.md`](web.md) の Sites 画面と共有）の `accounts` と `unfollowed.accounts` の両方から `stored === true` の版だけを集め、`created_at` 降順で並べる。各行のステータスアイコン（`.desk-status-icon`、`data-status`）は次の優先順で 1 個だけ選ぶ: (1) `nip05 === "mismatch"` → `ng`。(2) `nip05 === "error"`・未知の文字列 → `err`。(3) それ以外は更新の新しさで `new`（7 日以内）/`up`（30 日以内）/`default`（`not_applicable` は NIP-05 の問題扱いにせず新しさ判定に乗る）。

タイトルは自己申告の `title`（サニタイズ済み、最大 120 文字）があればそれを、無ければ `d` を表示し、`title` を出すときは必ず `d` も括弧付きで併記する。リンク先は `gateway_url` を一次リンク（無ければ `url`）にし、両方あれば `url` を `[本家]` の第二リンクにする。npub・cid・size・replicas は表示しない。来訪者カウンタは、ページ読み込みのたびに 1 増やす `localStorage["swing:desktop:visits"]` に、保存中サイト数から求めた基準値を足して表示する。

### おしらせ

更新の確認とおしらせの出し分け（マスコットの吹き出しかブラウザの通知か）は [`notices.md`](notices.md)。

### Desktop 画面は丸ごと日本語固定

`#view-desktop` の中で画面に見える文字列は、UI 表示言語の設定（`swing:lang`）に関わらず全部固定の日本語。ページ本文だけでなく、読み込み中/エラー/空状態の文面、ステータスバー、`title`・`aria-label` も含む（サイドナビのラベルだけは他画面と同じく i18n）。

### 内部クラス（非安定）

上の「リンク集ページ（iframe）」の表に載せたもの以外の `desk-` 接頭辞のクラスは `#view-desktop` とリンク集ページの実装に閉じた内部のもので、`swing-` 接頭辞のクラス（[`css.md#安定-class`](css.md#安定-class)）と違って安定インターフェースではない。
