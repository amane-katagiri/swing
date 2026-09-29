# ダッシュボードの Desktop 画面（`web/desktop*.js`）

[`../dashboard.md`](../dashboard.md) の子ページで、[`web.md`](web.md) と並列。ダッシュボード全体の構成・ルーティング・他画面は [`web.md`](web.md)、サーバ側は [`../dashboard.md`](../dashboard.md)、デスクトップのマスコットは子ページの [`mascot.md`](mascot.md) を参照。

## 構成

Desktop 画面専用のモジュール（依存は下から上への一方向。[`web.md#構成`](web.md#構成) の共通モジュールにも依存する）:

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
| `desktop-updates.js` | 更新の確認（`createUpdateWatcher`・`collectNotices`・`collectPublished`・`diffReporters`）。下記「[更新の確認](#更新の確認)」 |
| `desktop-notify.js` | ブラウザの通知（`createBrowserNotifier`）。下記「[おしらせの出し分け](#おしらせの出し分け)」。設定の読み書きと許可の判定は Settings 画面と共有する `notify-settings.js`（[`web.md#構成`](web.md#構成)） |
| `desktop-mascot-pack.js` | マスコットのパックの読み込み（`loadPacks`）。`/mascots/index.json` と各パックの `manifest.json` を取得し、検証・正規化してスプライトシートと当たり判定用のマスクを読み込む。以下マスコットの詳細は [`mascot.md`](mascot.md) |
| `desktop-mascot-sprite.js` | 1 体の描画（`createSprite`）。土台と重ね絵のレイヤー、コマ送り（`createPlayer`）、左右反転、配置、不透明な画素での当たり判定、吹き出しを向ける点 |
| `desktop-mascot-behavior.js` | 1 体のふるまいの状態機械（`createBehavior`）。DOM に触れず、乱数を差し込める。範囲内の乱数 `between` は `desktop-mascot.js` も使う |
| `desktop-mascot-balloon.js` | 吹き出し（`createBalloon`）。文字送り・リンク・「とじる」・配置 |
| `desktop-mascot.js` | マスコット全体の進行（`DesktopMascots`）。表示するパックごとに 1 体を出し、共有の `requestAnimationFrame` ループ・おしらせの振り分け・ポインタ操作と当たり判定の切り替えを持つ。`desktop.js` の `init`/`onShow` から呼ぶ。読み込んだパックの一覧（`packs()`・`whenLoaded()`）と設定の反映（`applySettings`）を「マスコット」タブに出す |
| `desktop-mascot-settings.js` | 「コントロール パネル」の「マスコット」タブ本体（`MascotSettingsPage`）。保存キー（`swing:desktop:mascot`）とその読み書きは、確認の間隔と共有するので `notify-settings.js` で定義する |
| `desktop-notify-settings.js` | 「コントロール パネル」の「通知」タブ本体（`NotifySettingsPage`）。保存キーは `notify-settings.js` で定義する |
| `desktop-system-settings.js` | 「コントロール パネル」の「システム」タブ本体（`SystemSettingsPage`）と、パスなしで開いたときの画面を返す `startupView()`（`app.js` が使う）。保存キーもここで定義する |
| `desktop-settings.js` | 「コントロール パネル」の殻（`PAGES` を `desktop-dialog.js` に結び付ける、`DesktopSettings`） |
| `desktop.js` | 画面のロジック本体（デスクトップアイコンの選択・起動、リンク集ページへの書き込み、共通のクリック/フォーカス処理） |

そのほかのファイル:

| ファイル | 役割 |
|---|---|
| `desktop.css`・`desktop-dialog.css`・`desktop-wallpaper.css`・`desktop-mascot-settings.css`・`desktop-system-settings.css`・`desktop-mascot.css` | `index.html` がこの順に読む CSS（それぞれフォント・シェル・`--desk-*` 変数、ダイアログの枠・汎用部品（無効状態を含むチェックボックス）、「背景」タブ固有、「マスコット」タブ固有、「システム」タブ固有、マスコットと吹き出し）。読み込み順は [`web.md#css-カスタマイズのインターフェース`](web.md#css-カスタマイズのインターフェース) |
| `desktop-frame.css` | 窓側のスクロールバーの見た目。`desktop.js` がリンク集ページ（iframe）に差し込む（下記「[リンク集ページ（iframe）](#リンク集ページiframe)」） |
| `desktop-page.html`・`desktop-page.css`・`desktop-banner.gif` | リンク集ページ・その専用 CSS・88×31 バナーの同梱版。設定で差し替えられる（[`../dashboard.md#設定dashboard`](../dashboard.md#設定dashboard)） |
| `desktop-icons.svg` | ピクセルアートアイコンのスプライト。`index.html` から `<use>` で参照する。SE ロゴの E は S との隙間を切り欠いた塗りの図形で持つ（外部ファイルの `<symbol>` の `mask` は、Firefox では CSS `zoom` の内側でずれるため使わない） |
| `fonts/` | 同梱フォント PixelMplus12 とそのライセンス |
| `mascots/` | 同梱のマスコットのパック（`yureko/`・`mochi/`・`neko/`。`index.json` は静的ファイルではなく実行時に生成する。[`mascot.md`](mascot.md)） |

## 画面

Win95/98 風デスクトップ（背景・デスクトップアイコン・タスクバー・Start 風ボタン・時計）と、その上の「SWING Explorer」ウィンドウ（タイトルバー・メニューバー・ツールバー・アドレス行・ページ本文・ステータスバー）を `#view-desktop` に静的 HTML で組む。実際に操作できる要素と装飾の切り分けは下記「装飾 vs 実機能」を参照。配色はダッシュボードのテーマ（`--swing-*`）を参照せず、`#view-desktop` スコープの固定レトロパレット（`--desk-*`）を使う（ライト/ダークで見た目は変わらない）。

### フォント

PixelMplus12（`web/fonts/`）を使う。フォントの読み込みが終わるまで `.desk-window` を隠し、Desktop 画面を初めて表示したときだけ待つ（上限あり）。

### 表示倍率

ドット絵フォントと 1px のベベルを崩さないため、`#view-desktop` の 1 CSS px が常に整数個のデバイスピクセルになるよう、`desktop-scale.js` が `devicePixelRatio` から `#view-desktop` の CSS `zoom` を決める。整数倍率 `n = max(1, floor(devicePixelRatio))` に対して `zoom = n / devicePixelRatio`。たとえば 125%・150% は 1 倍、200%〜299% は 2 倍で描く（100% 未満のときも 1 倍）。ブラウザのズームと OS の表示スケールのどちらにも効き、倍率が変わったら（`resolution` メディアクエリの変化と `resize`）計算し直す。リンク集ページ（iframe）は、中の `devicePixelRatio` が整数倍率になっていて、中の文書の高さ（`documentElement.clientHeight`）が `innerHeight` から縮んでいなければ（Chromium のように親の `zoom` がそのまま伝わっていれば）何もしない。どちらかが崩れていれば（Firefox は前者、WebKit は後者）、iframe 要素に `zoom = 1 / 補正値` をかけて親の `zoom` を打ち消し、iframe の文書のルート（`<html>`）に同じ補正値の `zoom` をかける。判定と適用は倍率の変化・大きさの変化・iframe の読み込みのたびにやり直す。

整数倍にしても、`#view-desktop` と iframe の左上がデバイスピクセルの途中にあると全体がにじむ（サイドバーの幅や iframe より上の文字の高さによって起きる）。そのため、左上がデバイスピクセルの境目に来るよう、両方の `left`/`top` を 1 デバイスピクセル未満だけずらす。`.desk-page-frame` が `position: relative` なのはこのため。倍率が変わったときと、どちらかの大きさが変わったとき（`ResizeObserver`）にやり直す。

`zoom` の内側では `style.left` などの長さは補正前の値、`getBoundingClientRect()` やポインタイベントの座標は補正後の値になる。ドラッグ・リサイズ・コンボボックスのリストの配置は、ビューポート座標を `toDeskPx()` で換算してから使う。スクリーンの大きさ（`desktop-drag.js::screenSize`）は `clientWidth`/`clientHeight`（補正前の値）から取る。

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

デスクトップアイコン「コントロール パネル」をダブルクリックまたは Enter/Space でモーダルダイアログ「コントロール パネル」を開く。タブは「背景」「マスコット」「通知」「システム」の 4 つ。ロジックは 3 層（ダイアログ一般の機能を持つ `desktop-dialog.js`、タブ切り替えの殻 `desktop-settings.js`、各タブの中身の `desktop-wallpaper.js::WallpaperPage`・`desktop-mascot-settings.js::MascotSettingsPage`・`desktop-notify-settings.js::NotifySettingsPage`・`desktop-system-settings.js::SystemSettingsPage`）に分かれる。

**Page のインターフェース**: `desktop-settings.js` の `PAGES` の各要素は、`id`（`data-tab` と一致）・`init({changed, dialog, updates})`（`updates` は `desktop.js` の `desktopUpdates`）・`open()`・`isDirty()`・`save()`・`discard()` を持つオブジェクト（任意で `onKey(ev)`・`boot()`）。ドラッグ・Tab トラップ・Esc/Enter・OK/キャンセル/適用ボタンは `createDialog` が全ページ共通で持つ（フォーカス表示は CSS）。

**タブ**: タブのボタンはクリックか、フォーカスしたタブで ←/→（端で折り返す）・Home/End で切り替える。Tab で止まるのは選択中のタブだけ（他は `tabindex="-1"`）。選んだタブはダイアログを閉じて開き直しても保つ（ページを読み込み直すと「背景」に戻る）。

**ボタン**: OK は変更のあるページを全部保存してから閉じる（保存に失敗したページがあれば閉じない）。キャンセル/×/Esc は全ページ破棄してから閉じる。適用は変更のあるページだけ保存し、ダイアログは開いたまま。

**背景タブ（`WallpaperPage`）**:

- 背景色（Win95 風パレット＋任意の色＋既定に戻す）と壁紙画像（表示方法: `center`/`tile`/`contain`/`cover`/`stretch`）は独立した項目で、互いに影響しない。
- 色・画像とも保存前にハイカラー量子化を通す。画像は長辺を 1920 px 以下に縮め、保存サイズの上限（data URL で 2.5 Mi 文字）に収まるまで 1280・1024・…・240 px と段階的にダウンスケールする。1 辺 16384 px か合計 8192×8192 画素を超える画像は canvas に描かずにエラーにする。
- **永続化**: `web/storage.js` 経由で `swing:desktop:wallpaper` キー（`desktop-wallpaper.js` で定義）に JSON で保存する（`color`・`image` は独立で、どちらか一方・両方・どちらも無し、いずれも正当な状態）。壊れた/想定外の値はキー単位で既定へ落とす。
- **画像の形**: `image` は `{dataUrl, width, height, display, filename}`（`dataUrl` は `data:image/` で始まる文字列、`display` は上の 5 つのどれか）。
- **適用先**: `#desk-wallpaper`（アイコンより下・ウィンドウより下の層）。保存値はページ読み込み時に 1 回 `localStorage` から読み、以後はメモリ上の保存値を Desktop 画面の表示のたびに当て直す（別のタブで保存した値は読み直さない）。保存に失敗したらエラーを出し、保存値は変えない。

**マスコットタブ（`MascotSettingsPage`）**: 表示するマスコットの選択・動き（「歩きまわる」「ひとりごとを言う」）を選び、`swing:desktop:mascot` キーに保存する。詳細は [`mascot.md#マスコットタブ`](mascot.md#マスコットタブ)。

**通知タブ（`NotifySettingsPage`）**:

- **更新の確認**: 確認の間隔をコンボボックスで 1 分 / 5 分 / 15 分 / 30 分 / 確認しない（既定 1 分）から選ぶ。下記「[更新の確認](#更新の確認)」の間隔になる。同じ設定は Settings 画面の「通知」パネルにもある（[`web.md#通知`](web.md#通知)）。
- **デスクトップでおしらせする内容**: 「フォロー中のサイトを新しくミラーしたとき」「サイトを SWING に公開（publish）したとき」「自分のサイトが SWING で新しくミラーされたとき」の 3 つのチェックボックス（既定はすべてオン）。マスコットの吹き出しに出す種類。
- **ブラウザの通知**: 「デスクトップを見ていないときはブラウザの通知を使う」チェックボックス（既定オフ）と、その下の同じ 3 つの種類のチェックボックス（既定はすべてオン。使わないときは無効表示）。オンにした操作の中で、許可がまだなら `Notification.requestPermission()` を呼ぶ。許可されなければチェックを外し、理由（ブロックされている・許可が得られなかった）を fieldset の末尾に出す。安全なコンテキストでない（`window.isSecureContext` が偽。LAN の `http://` で開いたときなど）か `Notification` が無いブラウザでは、チェックボックスを無効にして理由を出す。保存済みの値がオンでも、開いたときに許可が無くなっていれば理由を出す。同じ設定は Settings 画面の「通知」パネルにもある（[`web.md#通知`](web.md#通知)）。
- 種類ごとのオン・オフの効き方は下記「[更新の確認](#更新の確認)」の「種類のオン・オフ」と「[おしらせの出し分け](#おしらせの出し分け)」。
- **永続化**: 間隔は `swing:desktop:mascot` キーの `interval`（`60`/`300`/`900`/`1800` 秒か `null`（確認しない）。形は [`mascot.md#マスコットタブ`](mascot.md#マスコットタブ)）に、残りは `swing:desktop:notify` キーに `{"mascot": {"stored": true, "published": true, "replica": true}, "browser": {"enabled": false, "stored": true, "published": true, "replica": true}}` の形の JSON で保存する。`swing:desktop:notify` の読み書き・間隔の選択肢と読み書き・許可の要求・使えない理由の判定は `notify-settings.js`（`readNotifySettings`・`writeNotifySettings`・`CHECK_INTERVALS`・`readCheckInterval`・`writeCheckInterval`・`requestBrowserPermission`・`blockedReason`・`unavailableReason`）にあり、Settings 画面と共有する。間隔は「マスコット」タブと同じキーを使うので、どちらのタブも（Settings 画面も）保存のたびに今の値を読み直して自分の項目だけを書き換える（`writeMascotSettings`）。壊れた/想定外の値は項目ごとに既定へ落とす。
- **適用**: 間隔はページ読み込み時に 1 回読み、OK/適用で変わったときに `desktopUpdates.setInterval` で当てる（Settings 画面で変えたときは Settings 画面が当てる）。`swing:desktop:notify` は watcher・マスコット・ブラウザの通知が使うたびに `localStorage` から読むので、保存した時点で（Settings 画面で変えたときも）効く。タブを開くたびに間隔と `swing:desktop:notify` を読み直す（Settings 画面や別のタブで変えた値を出すため。間隔が変わっていればそのとき当て直す）。保存に失敗したらエラーを出し、保存値は変えない。

**システムタブ（`SystemSettingsPage`）**: 実際に働く項目は「起動時にデスクトップを表示する」チェックボックスだけ。オンにすると、ハッシュが無い（または知らない画面の）URL で開いたときの画面が `sites` ではなく `desktop` になる。`web/storage.js` 経由で `swing:desktop:startup` キー（`desktop-system-settings.js` で定義）に `"1"`/`"0"` で保存し、`"1"` 以外はオフ扱い。ほかに並ぶ項目（起動音・パフォーマンス・ネットワークなど）は飾りで、HTML で `disabled` にしてあり保存もしない。

### レイアウト

Desktop 画面では `.swing-main` の幅制限と余白を外して画面いっぱいに広げる。ページ全体はスクロールせず、スクロールするのはリンク集ページ（iframe）だけ。760px 以下のナロー幅ではナビが上に来る縦積みになり、ページ最下部のフッタ（`#page-footer`）は出さない。

### リンク集ページ（iframe）

ウィンドウの中身（`#desk-page-frame`）は同一オリジンの `<iframe src="/desktop-page.html">` で開く別ドキュメントで、`index.html` のダッシュボードの CSS は当たらず、ページ側の CSS も外に漏れない。窓側の `desktop-frame.css` だけは `desktop.js` が iframe の `<head>` の先頭に `<link>` で差し込む。ページ専用 CSS は `--desk-*`・`@font-face`・リセットまで自己完結する。ルートと差し替え設定は [`../dashboard.md#静的ファイルの配信srcdashboardassetsrs`](../dashboard.md#静的ファイルの配信srcdashboardassetsrs)。

iframe には `sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"` を付ける。差し替えたページの `<script>`・イベントハンドラ属性・フォーム送信・親ウィンドウの遷移は動かず、`target="_blank"` のリンクは制限の無い新しいタブで開く。`allow-same-origin` は親の `desktop.js` が `contentDocument` を読み書きするためのもので、ページ側でスクリプトは動かないので同一オリジンの権限を使われることはない。

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

### 更新の確認

`desktop-updates.js::createUpdateWatcher` が、新しく保存した版・自分の publish・自分のサイトをミラーする人の増加を見つけておしらせ（イベント）を流す。`desktop.js` が 1 つだけ作って `desktopUpdates` として公開し、`DesktopView.init` の中で（どの画面を開いていても）動かし始める。受け手（マスコット。[`mascot.md#おしらせ`](mascot.md#おしらせ)。ブラウザの通知。下記「[おしらせの出し分け](#おしらせの出し分け)」）は `subscribe(fn)` で登録する。

- **確認の時機**: 一定の間隔ごと（「通知」タブで選ぶ。既定 60 秒。`setInterval(ms)` で変える）、ページを開いて最初の画面を出した直後、Desktop 画面を表示したとき（`/api/sites` の初回読み込みの後）、タブが前面に戻ったとき（`visibilitychange`）。どの画面でも確認するが、ログイン画面・セットアップ画面では確認しない。タブが裏にある（`document.hidden`）ときは、ブラウザの通知が使える（オンにしていて、種類が 1 つ以上オンで、許可もある。`notify-settings.js::browserNotifyReady`）ときだけ確認する。同時には 1 回しか走らない。「確認しない」（`setInterval(null)`）のときはタイマーを止め、表示時・前面に戻ったときも含めて一切確認しない（`/api/activity` を呼ばない）。失敗が続く間は間隔を延ばし、接続できなかった後は cookie を付けない確認が通るまで `/api/activity` を呼ばない（[`web.md#止まっている間の呼び出し`](web.md#止まっている間の呼び出し)）。
- **確認の中身**: [`GET /api/activity`](http-api.md#get-apiactivity) の 3 つの値（カーソル）を順に見る。それぞれ既読の値を `localStorage` に別のキーで持ち、おしらせ済みの値（メモリだけ）はその先を走る。ページを読み込み直すとおしらせ済みは既読まで戻るので、おしらせしたが既読になっていないものはもう一度おしらせする。

| カーソル | 既読のキー | 進んだときにすること | 流すイベント |
|---|---|---|---|
| `latest_stored_at` | `swing:desktop:seen` | 手元の `cache.sites` に同じかより新しい `stored_at` が無ければ `DesktopView.load(true)` で `/api/sites` を取り直し（Explorer の一覧と Sites 画面の表示もこの結果で更新される）、`collectNotices` で集める | `sites-stored` |
| `latest_published_at` | `swing:desktop:seen-published` | [`GET /api/publish/sites`](http-api.md#get-apipublishsites) を取り直して `cache.publishSites` に入れ、`collectPublished` で `created_at` がおしらせ済みより新しい版を集める | `published` |
| `latest_replica_report_at` | `swing:desktop:seen-replicas` | [`GET /api/replicas`](http-api.md#get-apireplicaskeykey)（自分）を取り直して `cache.replicasByKey` に入れ、`diffReporters` でサイトごとの報告者の集合を前回と比べる（下記） | `replicas-added` |

- **初めてのとき**: 既読の値が無ければ、その時点の値を既読として記録するだけでおしらせはしない。`latest_stored_at` は `null`（版が無い）なら 0 を記録する。`latest_published_at`・`latest_replica_report_at` の `null` は「まだ分からない」なので、値が出るまで何も記録しない。`0`（確かめたら無かった）はそのまま既読として記録するので、その後の最初の publish や最初にミラーする人はおしらせする（`latest_replica_report_at` が `0` のときは `/api/replicas` を取らずに空の集合を保存する）。
- **ミラーする人の増加**: `swing:desktop:replica-reporters` に、サイトの `d` ごとに「`latest: true` で tier が `author`・`chosen`（信頼できる tier。[`http-api.md#get-apireplicaskeykey`](http-api.md#get-apireplicaskeykey)）の報告者（作者本人を除く）の pubkey → 初めて見たときの `latest_replica_report_at`」を JSON で持つ。確認のたびに今の集合をこの形で保存し直す（集合から消えた報告者は落ちる）。前回の集合に無い報告者か、記録した値がおしらせ済みより新しい（おしらせしたが既読になっていない）報告者がいるサイトだけをおしらせする。報告の出し直しや撤回では集合に新しく加わる人がいないので鳴らない。tier が `other`（自称にすぎない）の報告者は集合に入れないので、使い捨ての鍵で報告しても鳴らない（あとで `chosen` になれば、その時点で新しく加わったものとして扱う）。いったん消えてまた加わった報告者は新しく加わったものとして扱う。キーが無いときは初めてとして保存だけする。サイトのタイトルとリンク先は `cache.publishSites`（足りなければ `/api/publish/sites` を取り直す。失敗したらタイトル・リンク無し）から引く。
- **ダッシュボードからの publish**: Publish 画面の publish が成功すると `publish.js` が `document` に `swing:published`（`detail` は [`POST /api/publish/upload`](http-api.md#post-apipublishupload) の応答）を投げ、watcher がその場で `published` を流して publish のおしらせ済みを応答の `created_at` まで進める（同じ版を `latest_published_at` 経由で二度流さないため）。既読は進めない。既読の値がまだ無ければ `created_at - 1` を既読として記録する。
- **種類のオン・オフ**: 設定は確認のたびに読む。ある種類を確認するのは、マスコット側でオンか、ブラウザ側でオンでブラウザの通知が使える（上記 `browserNotifyReady`）ときだけ（`kindWanted`）。どちらでもない種類は、確認のたびに既読とおしらせ済みを今の値まで進めるだけで、取り直しもおしらせもしない（オンに戻しても、外していた間の分はおしらせしない）。ミラーする人の増加がそうなったときは、既読と報告者の集合のキーを消し、オンに戻したら初めてとして扱う。
- **マスコット側がオフの種類の既読**: マスコット側がオフの種類は、確認して取り直したら（おしらせを流したときはその直後に）既読を確認したカーソルの値まで進める（受け手が出したかどうかに関わらない。ダッシュボードからの publish では応答の `created_at` まで）。マスコットが後から言い直すことは無いので、既読を残すと、ページを読み込み直すたびに重い API を取り直して同じおしらせを流すことになるため。ブラウザの通知はこの前に（流すのと同じ処理の中で）出し終えている。
- **おしらせ**: 各要素は `{kind, key, at, site, href, ...}`。`kind` は `stored`・`published`・`replica`、`key` は合流に使う識別子、`at` は既読に使う値（それぞれ `stored_at`・`created_at`・報告者を初めて見たときの `latest_replica_report_at`）、`href` はリンク先（`gateway_url`、無ければ `url`。`replica` では無いこともある）。
  - `stored`（`collectNotices`）: `accounts` と `unfollowed.accounts` のうち `stored === true` で、リンク先が `http://`・`https://` で始まり（`util.js::isHttpUrl`）、`stored_at` がおしらせ済みより新しい版。ほかに `pubkey`・`npub`・`storedAt` を持ち、`site` は `/api/sites` のサイト。
  - `published`（`collectPublished`）: `site` は `/api/publish/sites` のサイト（ダッシュボードからの publish では応答から同じ形に組む）。
  - `replica`: `site` は `{d, title}`、ほかに `added`（新しく加わった報告者 `{pubkey, npub, tier}` の配列）・`replicas`・`unverified`（今の数）を持つ。`key` はサイトごとなので、同じサイトのおしらせは 1 つにまとまる。
  - どれも `at` の昇順に並べる。
- **既読**: 受け手が `acknowledge(notices)` を呼ぶと、種類ごとに `at` の最大値まで既読を進める（戻しはしない）。`isAcknowledged(notice)` は `localStorage` の今の値で判定するので、別のタブで既読にしたものも既読になる。
- **流すイベント**: `{kind: 'sites-stored' | 'published' | 'replicas-added', notices}`、`{kind: 'fetch-error', error}`（どれかの取得に失敗したとき。失敗が続く間は最初の 1 回だけ）、`{kind: 'recovered'}`（失敗の後に最初に確認し終えたとき）。

### おしらせの出し分け

1 回ぶんのおしらせ（`sites-stored`・`published`・`replicas-added`）は、Desktop 画面が表示中でタブが前面にあり、マスコットが 1 体以上表示されている（`DesktopMascots.showing()`）ならマスコットの吹き出しで、それ以外ならブラウザの通知で出す。種類のオン・オフは、マスコットが見えているときはマスコット側、見えていないときはブラウザ側で判定する。マスコットが見えているときにマスコット側がオフの種類は、どちらにも出さない（ブラウザの通知は、デスクトップを見ていないとき専用）。

- **マスコット**: [`mascot.md#おしらせ`](mascot.md#おしらせ)。マスコット側がオフの種類は受けない。Desktop 画面が表示されていない間に来たおしらせは預かっておき（マスコット側がオフの種類は預からない）、次に Desktop 画面を表示したとき（またはタブが前面に戻ったとき）に、既読になったものとその時点でマスコット側がオフの種類を除いて出す。
- **ブラウザの通知**（`desktop-notify.js::createBrowserNotifier`）: 「ブラウザの通知を使う」がオンで種類が 1 つ以上オン、`Notification.permission` が `granted` のときだけ、ブラウザ側でオンの種類だけを出す。既読のものを除き、さらに `swing:desktop:notified`（種類ごとに、通知を出した `at` の最大値の JSON）より新しいものだけを出してその値を進める（おしらせし直しで同じ通知を何度も出さないため。既読とは別で、出しただけでは既読にしない。ただしマスコット側がオフの種類は watcher が流した直後に既読にする。上記「更新の確認」）。1 回ぶんを 1 つの `Notification` にし、題は `SWING`、本文は 1 件ならサイトのタイトル（サニタイズして最大 60 文字、無ければ `d`）入りの文、2 件以上なら件数入りの文（表示言語の設定に従う i18n）。`tag` は `swing:<kind>:<最も新しい at>` で、複数のタブが同じおしらせを出しても重ならない。クリックすると通知を閉じてタブを前面に出し、通知を出した時点でのその種類のマスコット側のオン・オフで次のように動く（通知は 1 回ぶん＝1 種類）。クリックでは既読を進めない（マスコット側がオンならマスコットに任せ、オフならおしらせした時点で既読になっている）。

| 種類 | マスコット側オン | マスコット側オフ |
|---|---|---|
| `stored` | Desktop 画面（`#/desktop`） | 1 件ならリンク（`href`）を新しいタブで開く。2 件以上（またはリンクが `http://`・`https://` でない）なら起動時の画面（`desktop-system-settings.js::startupView()`。`desktop` か `sites`） |
| `published` | Desktop 画面 | Publish 画面（`#/publish`） |
| `replica` | Desktop 画面 | Webring 画面で自分のノードを選んだ状態（`swing:show-self-in-webring` を投げる。[`web.md#webring-画面`](web.md#webring-画面)）。詳細パネルは watcher が `cache.replicasByKey` に入れた最新の `/api/replicas` をそのまま使う |

`new Notification` が例外を投げる環境（ページからの通知を作れないモバイルのブラウザなど）では何もしない。

### Desktop 画面は丸ごと日本語固定

`#view-desktop` の中で画面に見える文字列は、UI 表示言語の設定（`swing:lang`）に関わらず全部固定の日本語。ページ本文だけでなく、読み込み中/エラー/空状態の文面、ステータスバー、`title`・`aria-label` も含む（サイドナビのラベルだけは他画面と同じく i18n）。

### 内部クラス（非安定）

上の「リンク集ページ（iframe）」の表に載せたもの以外の `desk-` 接頭辞のクラスは `#view-desktop` とリンク集ページの実装に閉じた内部のもので、`swing-` 接頭辞のクラス（[`web.md`](web.md#css-カスタマイズのインターフェース)）と違って安定インターフェースではない。
