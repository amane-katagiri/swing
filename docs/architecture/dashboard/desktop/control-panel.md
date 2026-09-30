# コントロール パネル（`web/desktop-settings.js`, `web/desktop-dialog.js`, `web/desktop-wallpaper.js`, `web/desktop-mascot-settings.js`, `web/desktop-notify-settings.js`, `web/desktop-system-settings.js`）

[`../desktop.md`](../desktop.md) の子ページ。

デスクトップアイコン「コントロール パネル」をダブルクリックまたは Enter/Space でモーダルダイアログ「コントロール パネル」を開く。タブは「背景」「マスコット」「通知」「システム」の 4 つ。ロジックは 3 層（ダイアログ一般の機能を持つ `desktop-dialog.js`、タブ切り替えの殻 `desktop-settings.js`、各タブの中身の `desktop-wallpaper.js::WallpaperPage`・`desktop-mascot-settings.js::MascotSettingsPage`・`desktop-notify-settings.js::NotifySettingsPage`・`desktop-system-settings.js::SystemSettingsPage`）に分かれる。

**Page のインターフェース**: `desktop-settings.js` の `PAGES` の各要素は、`id`（`data-tab` と一致）・`init({changed, dialog, updates})`（`updates` は `desktop.js` の `desktopUpdates`）・`open()`・`isDirty()`・`save()`・`discard()` を持つオブジェクト（任意で `onKey(ev)`・`boot()`）。ドラッグ・Tab トラップ・Esc/Enter・OK/キャンセル/適用ボタンは `createDialog` が全ページ共通で持つ（フォーカス表示は CSS）。

**タブ**: タブのボタンはクリックか、フォーカスしたタブで ←/→（端で折り返す）・Home/End で切り替える。Tab で止まるのは選択中のタブだけ（他は `tabindex="-1"`）。選んだタブはダイアログを閉じて開き直しても保つ（ページを読み込み直すと「背景」に戻る）。

**ボタン**: OK は変更のあるページを順に保存し、保存に失敗したページがあればそこで止めて閉じない（残りのページは保存しない）。すべて保存できたら閉じる。キャンセル/×/Esc は全ページ破棄してから閉じる。適用は変更のあるページだけ保存し、ダイアログは開いたまま。

## 各タブ

### 背景タブ（`WallpaperPage`）

- 背景色（Win95 風パレット＋任意の色＋既定に戻す）と壁紙画像（表示方法: `center`/`tile`/`contain`/`cover`/`stretch`）は独立した項目で、互いに影響しない。
- 色・画像とも保存前にハイカラー量子化を通す。画像は長辺を 1920 px 以下に縮め、保存サイズの上限（data URL で 2.5 Mi 文字）に収まるまで 1280・1024・…・240 px と段階的にダウンスケールする。1 辺 16384 px か合計 8192×8192 画素を超える画像は canvas に描かずにエラーにする。
- **永続化**: `web/storage.js` 経由で `swing:desktop:wallpaper` キー（`desktop-wallpaper.js` で定義）に JSON で保存する（`color`・`image` は独立で、どちらか一方・両方・どちらも無し、いずれも正当な状態）。壊れた/想定外の値はキー単位で既定へ落とす。
- **画像の形**: `image` は `{dataUrl, width, height, display, filename}`（`dataUrl` は `data:image/` で始まる文字列、`display` は上の 5 つのどれか）。
- **適用先**: `#desk-wallpaper`（アイコンより下・ウィンドウより下の層）。保存値はページ読み込み時に 1 回 `localStorage` から読み、以後はメモリ上の保存値を Desktop 画面の表示のたびに当て直す（別のタブで保存した値は読み直さない）。保存に失敗したらエラーを出し、保存値は変えない。

### マスコットタブ（`MascotSettingsPage`）

表示するマスコットの選択・動き（「歩きまわる」「ひとりごとを言う」）を選び、`swing:desktop:mascot` キーに保存する。詳細は [`mascot.md#マスコットタブ`](../mascot.md#マスコットタブ)。

### 通知タブ（`NotifySettingsPage`）

値・既定・保存先・許可の規則は [`notices.md#設定`](../notices.md#設定)。同じ設定は Settings 画面の「通知」パネルにもある（[`views.md#通知`](../views.md#通知)）。

- **更新の確認**: 確認の間隔をコンボボックスで選ぶ（当て方は [`notices.md#設定`](../notices.md#設定)）。
- **デスクトップでおしらせする内容**: マスコットの吹き出しに出す 3 つの種類のチェックボックス（「フォロー中のサイトを新しくミラーしたとき」「サイトを SWING に公開（publish）したとき」「自分のサイトが SWING で新しくミラーされたとき」）。
- **ブラウザの通知**: 「デスクトップを見ていないときはブラウザの通知を使う」チェックボックスと、その下の同じ 3 つの種類のチェックボックス（使わないときは無効表示）。許可の理由は fieldset の末尾に出す。

### システムタブ（`SystemSettingsPage`）

実際に働く項目は「起動時にデスクトップを表示する」チェックボックスだけ。オンにすると、ハッシュが無い（または知らない画面の）URL で開いたときの画面が `sites` ではなく `desktop` になる。`web/storage.js` 経由で `swing:desktop:startup` キー（`desktop-system-settings.js` で定義）に `"1"`/`"0"` で保存し、`"1"` 以外はオフ扱い。ほかに並ぶ項目（起動音・パフォーマンス・ネットワークなど）は飾りで、HTML で `disabled` にしてあり保存もしない。
