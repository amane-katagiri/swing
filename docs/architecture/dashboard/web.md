# ダッシュボードの画面（`web/index.html`, `web/*.js`, `web/*.css`）

[`../dashboard.md`](../dashboard.md) の子ページ。サーバ側の起動・タイムアウト・静的ファイル配信は [`../dashboard.md`](../dashboard.md)、ガードと認証は [`security.md`](security.md)、HTTP API の入出力は [`http-api.md`](http-api.md)、Desktop 画面は並列の [`desktop.md`](desktop.md)、おしらせは [`notices.md`](notices.md) を参照。子ページ:

- [`views.md`](views.md): Sites・Setup・Login の各画面。子ページに Publish（[`views/publish.md`](views/publish.md)）・Webring（[`views/webring.md`](views/webring.md)）・Settings（[`views/settings.md`](views/settings.md)）
- [`web/ui.md`](web/ui.md): 複数の画面で使う UI 部品と共通の見た目（サイドナビ・表の折り返し・モバイル幅など）
- [`css.md`](css.md): CSS カスタマイズのインターフェース
- [`fingerprint.md`](fingerprint.md): アカウントの絵文字のフィンガープリント（`web/emoji/`・`web/emoji.svg`）

## 構成

ビルド工程なし・外部依存なしの素の HTML + CSS + ES modules。表の上のものほど下位で、各モジュールはそれより上に載ったモジュールにだけ依存する（循環 import は無い）。例外は `app.js` で、表に無い `desktop.js`・`desktop-system-settings.js` も import する。

| ファイル | 役割 | 依存 |
|---|---|---|
| `storage.js` | `localStorage` の薄いラッパー | なし |
| `i18n.js` | 多言語辞書と `t()`、`index.html` の静的な訳を当てる `applyStaticI18n`、JS で作る要素に表示言語の `title`・`aria-label`・`lang` を当てる `setUiLabels` | `storage.js` |
| `util.js` | 画面間で共有するキャッシュ（`cache`）・DOM と fetch の部品・表示スタイルと並び順の切替・非同期ロードのガード（`createLoadGuard`）・`pollUntil`（1 秒間隔・最大 120 回）・cookie を付けない確認と失敗時の間隔（[下記](#止まっている間の呼び出し)）・表示前のサニタイズ（[下記](#表示前のサニタイズ)）・表示の整形 | `storage.js`・`i18n.js` |
| `ui.js` | 複数画面で共有する UI 部品（コピーボタン、バッジ、relay 結果表示、サイト名の行 `buildSiteNameRow`、絵文字のフィンガープリント `fingerprintBadge`、入力中の鍵の確認 `createKeyPreview` など） | `util.js`・`i18n.js` |
| `graph.js` | webring 用の自前 force-directed layout（[`views/webring.md#グラフwebgraphjs`](views/webring.md#グラフwebgraphjs)） | `util.js`・`ui.js` |
| `pairing.js` | 署名アプリ（NIP-46）とのペアリングの部品（`createPairing`）。Setup 画面と Publish 画面の「つなぎ直す」が使う | `util.js`・`i18n.js` |
| `notify-settings.js` | おしらせの設定（[`notices.md#設定`](notices.md#設定)）と `swing:desktop:mascot` 全体の読み書き、ブラウザの通知の許可、種類を確認するかどうか（`kindWanted`）、イベント名 `SHOW_SELF_IN_WEBRING`。Settings 画面・Webring 画面・Desktop 画面が使う | `storage.js` |
| `stats.js` | Settings 画面のリソース使用量のパネル（`loadStats`・`renderStats`） | `util.js`・`i18n.js` |
| `network.js` | Settings 画面のネットワークのパネル（`initNetwork`・`loadNetwork`・`renderNetwork`） | `util.js`・`i18n.js` |
| `settings-notify.js` | Settings 画面の「通知」パネル（`BrowserNotifySettings`） | `notify-settings.js`・`util.js`・`i18n.js` |
| `sites.js` / `webring.js` / `publish.js` / `settings.js` / `setup.js` / `login.js` | 各画面（[`views.md`](views.md) と子ページ）。`webring.js` は `graph.js`・`notify-settings.js`、`publish.js` と `setup.js` は `pairing.js`、`settings.js` は `stats.js`・`network.js`・`settings-notify.js`、`setup.js` は `publish.js` の `loadOverview` も使う | 上記 |
| `boot.js` | 描画前に同期実行する小さな通常スクリプト（[`web/ui.md`](web/ui.md) の読み込み時） | なし |
| `app.js` | ルーター兼エントリポイント（`<script type="module" src="/app.js">`） | 各画面と上記の共有モジュール、`desktop.js`・`desktop-system-settings.js`（`startupView`） |

Desktop 画面専用のモジュール（`desktop*.js`）は [`desktop.md#構成`](desktop.md#構成)。

CSS の読み込み順と上書きの仕方は [`css.md#読み込み順`](css.md#読み込み順)。

## ルーティング

サイドナビの並び順（上から Desktop・Sites・Webring・Publish・Settings・Setup）と同じ `#/desktop` `#/sites` `#/webring` `#/publish` `#/settings` `#/setup` の 6 画面と、未ログインのときだけ出す Login 画面をハッシュルーティングで切り替える。ハッシュが無いか知らない画面のときは `sites`（Desktop 画面の「コントロール パネル」→「システム」で `desktop` に変えられる。[`desktop/control-panel.md`](desktop/control-panel.md)）。現在の画面のナビのリンクには `aria-current="page"` が付く。

## API の呼び方

- `/api/*` はすべて `fetch`/`XMLHttpRequest` で呼ぶ（`<img src>` やリンクで読まない）。
- `util.js::apiFetch` はどのメソッドにも `X-Swing-Dashboard: 1` を付け（cookie で認証する読み取りにも要る。[`security.md#ガード`](security.md#ガード)）、GET/HEAD 以外には `Content-Type: application/json` も付ける。
- `apiFetch` を通らないのは、`FormData` を送る publish のアップロード（`XMLHttpRequest`）と公開前の確認（`POST /api/publish/check`。`fetch`）、それに `fetchInstance`（[下記](#止まっている間の呼び出し)）だけ。前の 2 つは `X-Swing-Dashboard: 1` だけを自分で付ける。
- 応答の解釈は `apiFetch` と publish のアップロード・公開前の確認のどれも `parseApiBody`（JSON でなければ `null`）と `apiResponseError`（2xx 以外をエラーにし、401 なら下記の `swing:unauthorized` を投げる）を使う。`apiFetch` は接続できなかったとき `status` が `0` のエラーを投げる。

## 表示前のサニタイズ

他人の Nostr イベント由来のテキスト（サイトの `title`・`d`・`message`、webring のラベル・names など）と、relay や署名アプリから来てサーバがそのまま返すエラー文言（API の `error`・署名アプリのペアリングの `error`・Publish 画面の `last_failure.message`・Sites 画面の `replicas_error`）は、表示直前に `util.js::sanitizeDisplayText` を通してから DOM に入れる。

- 取り除くもの: 制御文字（C0・DEL・C1 と U+2028/U+2029。空白 1 つに置き換える。`stripControlChars`）と、双方向制御文字と isolate・不可視文字・ゼロ幅文字・ソフトハイフン・行間注釈（U+FFF9–U+FFFB）・タグ文字（U+E0000–U+E007F）（`stripUnsafeUnicode`）。範囲の正本はこの 2 つの関数。
- `sanitizeMessage` は既定 200 文字で `…` に切り詰め、空なら `null` にする。
- 対象の要素には `dir="auto"` と CSS 側の `unicode-bidi: isolate` を付け、RTL の文字列を周囲の UI の並びに影響させない。
- 挿入は DOM API だけで行い、`innerHTML` は使わない。`url` は `^https?://` にマッチするときだけリンクにする。
- webring の DOT/Mermaid/ASCII エクスポートは `stripUnsafeUnicode` だけを通す（制御文字は残す）。オペレーター自身のフォーム入力には適用しない。

## 起動とログイン状態

`app.js` の `init()` は `loadOverview()`（[`/api/overview`](http-api/status.md#get-apioverview)）を待ってから `showRoute()` を呼ぶ。`cache.overview.setup` が `true` の間は `currentRoute()` が hash に関わらず `'setup'` を返し（未ログインの判定が優先）、サイドナビも Setup 項目だけを表示する（`false` の間は Setup 項目を隠す）。

未ログイン（`apiResponseError` が `/api/login` 以外の 401 を受けた）のときは `apiResponseError` が `swing:unauthorized` イベントを投げ、`app.js` が以後 `currentRoute()` を（セットアップモードでも）常に `'login'` にしてサイドナビをすべて隠す。ログイン済みのときに `#/login` を開いても既定の画面に落とす。

各画面のロードは世代カウンタ（`createLoadGuard`）でガードし、古いレスポンスで上書きしない。

## 止まっている間の呼び出し

`swing up` が止まっていそうな間は、cookie を付けない確認だけを送る（残る弱点は [`security.md#既知の弱点`](security.md#既知の弱点)）。

- `util.js::fetchInstance` は [`POST /api/identity`](http-api/session.md#post-apiidentity) を `credentials: 'omit'` で呼び（nonce は `crypto.getRandomValues` の 32 バイト）、応答の `instance` を返す（2xx 以外や `instance` が無ければ `null`）。`proof` は見ない。`dashboardAnswers()` はこれが `instance` を返したかどうか（例外も `false`）。
- 再起動を待つとき（Publish 画面のつなぎ直し・Setup 画面の送信後）は、`waitForNewInstance(previous)`（`pollUntil` で `fetchInstance` を呼び、`previous` と違う `instance` が返るまで）か同じ判定を通ってから、認証付きの呼び出しをする。
- 定期的な呼び出し（おしらせの `/api/activity`、Settings のリソース使用量の `/api/stats`）は、接続できなかった後は次の回から `dashboardAnswers()` が `true` になるまで認証付きの呼び出しをしない。失敗が続く間は間隔を `backoffDelay(interval, 失敗回数)`（間隔 × 2^失敗回数。10 分か元の間隔の大きいほうで頭打ち）に延ばし、成功したら元に戻す。

## 画面の一覧

| 画面 | 内容 | 表示スタイル（`data-style`、localStorage キー `swing:style:<view>`） |
|---|---|---|
| Desktop | [`/api/sites`](http-api/status.md#get-apisites) の保存済みと更新待ちのサイト（[`desktop/link-page.md`](desktop/link-page.md)）をレトロ調（Win95/98 風デスクトップ＋ブラウザ風ウィンドウ内の「リンク集」ページ）に描画する（[`desktop.md`](desktop.md)） | なし |
| Sites | [`/api/sites`](http-api/status.md#get-apisites) の一覧、mirror への追加・削除、`Unfollowed but still stored`、Storage check（[`views.md#sites-画面`](views.md#sites-画面)） | `cards`（既定）/ `table` |
| Webring | [`/api/webring`](http-api/nostr.md#get-apiwebringrootkeydepthn) のグラフとノードの詳細・ミラー操作（[`views/webring.md`](views/webring.md)） | `graph`（既定）/ `list` / `ascii` / `source`（dot・mermaid） |
| Publish | 自分の情報、My sites、publish フォーム（[`views/publish.md`](views/publish.md)） | なし |
| Settings | 設定の表示と編集、テーマ・言語・カスタム CSS、通知、リソース使用量、プロセスの停止・再起動（[`views/settings.md`](views/settings.md)） | なし |
| Setup | 鍵が未設定（`overview.setup === true`）の間だけ表示できる導入画面（[`views.md#setup-画面setupjs`](views.md#setup-画面setupjs)） | なし |
| Login | 未ログインのときだけ出すログインコードの入力画面（[`views.md#login-画面loginjs`](views.md#login-画面loginjs)） | なし |

## 共通の UI 部品

[`web/ui.md`](web/ui.md)。絵文字のフィンガープリント・busy 表示・コピー・サイドナビ・ロゴタイプ・ペアリング・読み込み時の `boot.js`・表の折り返し・モバイル幅。

## localStorage キー一覧

ダッシュボードが使う `localStorage` のキーはこれで全部（サーバには送らない）。

| キー | 値の形 | 意味 |
|---|---|---|
| `swing:style:<view>`（`view` は `sites`/`webring`） | 文字列（スタイル名） | 画面ごとの表示スタイル |
| `swing:sites:sort` | `updated` / `name` / `pubkey` | Sites の並び順（既定 `updated`） |
| `swing:sites:stored-filter` | `"all"` / `"true"` / `"pending"` / `"false"` | Sites の保存状態のフィルター |
| `swing:webring:query` | JSON `{root, depth}` | Webring の最後のクエリ（起動時に復元） |
| `swing:publish:last` | JSON `{site, url, title, message, nip05, check_dotfiles, check_size, check_links, check_unchanged}` | Publish フォームの最後の入力（起動時にプリフィル） |
| `swing:nav:collapsed` | `"1"` / `"0"` | サイドナビを畳んでいるか（既定 `0`） |
| `swing:theme` | `auto` / `light` / `dark` | 表示テーマ |
| `swing:lang` | `auto` / `en` / `ja` | 表示言語 |
| `swing:user-css` | 文字列（CSS） | Settings のカスタム CSS 欄の内容 |
| `swing:desktop:visits` | 整数の文字列 | Desktop 画面の来訪者カウンタ（[`desktop/link-page.md`](desktop/link-page.md)） |
| `swing:desktop:wallpaper` | JSON `{color?, image?}`（[`desktop/control-panel.md`](desktop/control-panel.md)） | Desktop 画面の壁紙 |
| `swing:desktop:startup` | `"1"` / `"0"` | パスなしで開いたときに Desktop 画面を出すか（既定 `"0"`。[`desktop/control-panel.md`](desktop/control-panel.md)） |
| `swing:desktop:mascot` | JSON `{packs?, interval, walk, chatter}`（[`mascot.md#マスコットタブ`](mascot.md#マスコットタブ)） | マスコットの設定と更新の確認の間隔（`interval` は [`notices.md#設定`](notices.md#設定)） |
| `swing:desktop:notify` | JSON `{mascot: {…}, browser: {…}}`（[`notices.md#設定`](notices.md#設定)） | おしらせの種類のオン・オフとブラウザの通知を使うかどうか |
| `swing:desktop:seen`・`swing:desktop:seen-published`・`swing:desktop:seen-replicas` | 整数の文字列 | おしらせの既読の位置（[`notices.md#更新の確認`](notices.md#更新の確認)） |
| `swing:desktop:replica-reporters` | JSON `{<d>: {<pubkey>: <初めて見た時刻>}}` | 前回確認した自分のサイトの報告者の集合（[`notices.md#更新の確認`](notices.md#更新の確認)） |
| `swing:desktop:notified` | JSON `{<種類>: <at>}` | ブラウザの通知を出したおしらせの位置（[`notices.md#おしらせの出し分け`](notices.md#おしらせの出し分け)） |

## 表示言語（i18n）

`web/i18n.js` の `MESSAGES = { en: {...}, ja: {...} }` を `t(key, vars)` で参照する。`localStorage["swing:lang"]`（`auto`/`en`/`ja`。`auto` は `navigator.language` が `ja` で始まるかで判定）で切り替え、再読み込みは不要。切り替えると `document` に `swing:langchange` という `CustomEvent` を投げ、`app.js` が各画面を再描画する。訳が無いキーは英語にフォールバックする。

訳さないもの: ナビゲーションの「Webring」、webring の ASCII/DOT/Mermaid 出力、API のエラー文字列、npub・hex・CID・パス、環境変数名・設定キー名、`nip05`/`health` のステータス値、NIP-05 とサイトの確認のモードの `off`/`warn`/`require`。日時表示は `Intl.DateTimeFormat`（`ja-JP`/`en-US`）を使う。

Desktop 画面は、画面に見える文字列は表示言語の設定に関わらず固定の日本語で、ツールチップ（`title`）と対になる `aria-label` だけが表示言語に従う（[`desktop.md#desktop-画面の表示言語`](desktop.md#desktop-画面の表示言語)）。`index.html` の要素に `data-i18n`・`data-i18n-title`・`data-i18n-aria-label`・`data-i18n-placeholder` を付けると `applyStaticI18n` が訳を当て、`data-i18n-lang` を付けるとその要素の `lang` を表示言語にする。
