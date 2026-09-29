# ダッシュボードの画面（`web/index.html`, `web/*.js`, `web/*.css`）

[`../dashboard.md`](../dashboard.md) の一部。サーバ側の起動・ガード・タイムアウト・静的ファイル配信は [`../dashboard.md`](../dashboard.md)、HTTP API の入出力は [`http-api.md`](http-api.md)、Desktop 画面の詳細は [`desktop.md`](desktop.md)（マスコットは [`mascot.md`](mascot.md)）を参照。

## 構成

ビルド工程なし・外部依存なしの素の HTML + CSS + ES modules。依存は下から上への一方向で循環 import は無い。

| ファイル | 役割 |
|---|---|
| `storage.js` | `localStorage` の薄いラッパー。他のどのモジュールにも依存しない |
| `i18n.js` | 多言語辞書と `t()`。`storage.js` にだけ依存する |
| `util.js` | 画面間で共有するキャッシュ・DOM/fetch ユーティリティ・表示スタイル切替・非同期ロードのガード（`createLoadGuard`）・`sleep()`・`pollUntil(predicate)`（1 秒間隔・最大 120 回の汎用ポーリング）・cookie を付けない再起動・復帰の確認（`fetchInstance`・`dashboardAnswers`・`waitForNewInstance`）と失敗時の間隔（`backoffDelay`。下記「止まっている間の呼び出し」）・他人のイベント由来テキストの表示前サニタイズ（`stripUnsafeUnicode` / `sanitizeDisplayText` / `sanitizeMessage`、サイトの表示名 `siteTitle`）。`storage.js`・`i18n.js` に依存する |
| `ui.js` | 複数画面で共有する UI 部品（コピーボタン、バッジ、relay 結果表示、サイト名の行 `buildSiteNameRow` など）。`util.js`・`i18n.js` に依存する |
| `graph.js` | webring 用の自前 force-directed layout。`util.js` の `clamp`・`sanitizeDisplayText` だけに依存する |
| `pairing.js` | 署名アプリ（NIP-46）とのペアリングの部品（`createPairing`）。QR の表示・状態のポーリング・状態表示を受け持ち、Setup 画面と Publish 画面の「署名アプリとつなぎ直す」の両方が使う。`util.js`・`i18n.js` に依存する |
| `sites.js` / `webring.js` / `publish.js` / `settings.js` / `setup.js` / `login.js` | 各画面（それぞれ Sites・Webring・Publish・Settings・Setup・Login）。`setup.js` は `publish.js` の `loadOverview` も使う |
| `stats.js` | Settings 画面のリソース使用量のパネル（`loadStats`・`renderStats`）。`util.js`・`i18n.js` に依存し、`settings.js` と `app.js` から使う |
| `notify-settings.js` | おしらせの設定（`swing:desktop:notify`）の読み書き（`readNotifySettings`・`writeNotifySettings`）、確認の間隔の選択肢（`CHECK_INTERVALS`）と読み書き（`readCheckInterval`・`writeCheckInterval`。`swing:desktop:mascot` の `interval`。キー全体の読み書き `readMascotSettings`・`writeMascotSettings` もここにある）、ブラウザの通知の許可の要求（`requestBrowserPermission`）と使えない理由の判定（`notificationSupport`・`unavailableReason`・`blockedReason`）、使えるかどうか（`browserNotifyReady`）と種類を確認するかどうか（`kindWanted`）。`storage.js` にだけ依存し、Settings 画面と Desktop 画面の両方が使う |
| `settings-notify.js` | Settings 画面の「通知」パネル（`BrowserNotifySettings`）。`notify-settings.js`・`util.js`・`i18n.js` に依存し、`settings.js` と `app.js` から使う |
| `boot.js` | 描画前に同期実行する小さな通常スクリプト（下記「共通の UI 部品」の読み込み時）。どのモジュールにも依存しない |
| `app.js` | ルーター兼エントリポイント。`<script type="module" src="/app.js">` から読み込まれ、各画面モジュールを import する |

Desktop 画面専用のモジュール（`desktop*.js`）とその CSS は [`desktop.md#構成`](desktop.md#構成) を参照。

`index.html` が読む CSS は `style.css`（全画面共通）と Desktop 画面用の 5 ファイル（[`desktop.md#構成`](desktop.md#構成)）。アイコンは `index.html` の `<link>` で `favicon-32.png`・`favicon.svg`・`apple-touch-icon.png` の 3 つを指定する。Desktop 画面のアイコンは `web/desktop-icons.svg`（[`desktop.md#構成`](desktop.md#構成)）。

### ルーティング

サイドナビの並び順（上から Desktop・Sites・Webring・Publish・Settings・Setup）と同じ `#/desktop` `#/sites` `#/webring` `#/publish` `#/settings` `#/setup` の 6 画面と、未ログインのときだけ出す Login 画面をハッシュルーティングで切り替える（既定は `sites`。Desktop 画面の「コントロール パネル」→「システム」で `desktop` に変えられる。[`desktop.md`](desktop.md#コントロール-パネル)）。現在の画面のナビのリンクには `aria-current="page"` が付く。`/api/*` はすべて `fetch`/`XMLHttpRequest` で呼ぶ（`<img src>` やリンクで読まない）。`util.js::apiFetch` はどのメソッドにも `X-Swing-Dashboard: 1` を付け（cookie で認証する読み取りにも要る。[`../dashboard.md#ガードsrcdashboardguardrs`](../dashboard.md#ガードsrcdashboardguardrs)）、GET/HEAD 以外には `Content-Type: application/json` も付ける。publish のアップロード（`XMLHttpRequest` + `FormData`）だけは `apiFetch` を通らず、`X-Swing-Dashboard: 1` だけを自分で付ける。応答の解釈は `apiFetch` と同じ `parseApiBody`（JSON でなければ `null`）と `apiResponseError`（2xx 以外をエラーにし、401 なら下記の `swing:unauthorized` を投げる）を使う。サーバーが返すエラー文言（`error`）は `sanitizeDisplayText` を通してから表示する。relay 由来の文字列は DOM API だけで挿入し、`innerHTML` は使わない。`url` は `^https?://` にマッチするときだけリンクにする。

### 表示前のサニタイズ

他人の Nostr イベント由来のテキスト（サイトの `title`・`d`・`message`、webring のラベル・names など）は、表示直前の 1 箇所（`util.js::sanitizeDisplayText`。内部で `stripUnsafeUnicode` と `stripControlChars` を呼ぶ。`sanitizeMessage` は既定 200 文字で `…` に切り詰め、空なら `null` にするラッパー）だけで、制御文字（C0・DEL・C1 と行区切り・段落区切り U+2028/U+2029。空白 1 つに置き換える）・双方向制御文字と isolate・不可視文字・ゼロ幅文字・ソフトハイフン・行間注釈（U+FFF9–U+FFFB）・タグ文字（U+E0000–U+E007F）を取り除いてから DOM に入れる（対象の範囲は `stripControlChars`・`stripUnsafeUnicode` が正本）。対象の要素には `dir="auto"` と CSS 側の `unicode-bidi: isolate` を付け、正当な RTL（アラビア語・ヘブライ語タイトルなど）はそのまま表示しつつ周囲の UI の並びには影響しないようにする。webring の DOT/Mermaid/ASCII エクスポートは `stripUnsafeUnicode` だけを通す（制御文字は残す）。オペレーター自身のフォーム入力（Publish フォームへの再入力など）には適用しない。relay や署名アプリから来てサーバーがそのまま返すエラー文言（API の `error`・署名アプリのペアリングの `error`・Publish 画面の署名の失敗 `last_failure.message`・Sites 画面の `replicas_error`）も同じく `sanitizeDisplayText` を通す。

### 起動とログイン状態

`app.js` の `init()` は `loadOverview()`（[`/api/overview`](http-api.md#get-apioverview)）を待ってから `showRoute()` を呼ぶ。`cache.overview.setup` が `true` の間は `currentRoute()` が hash に関わらず `'setup'` を返し（下記の未ログインの判定が優先）、サイドナビも Setup 項目だけを表示する（`false` の間は Setup 項目を隠し、残りの項目を出す）。

未ログイン（`apiFetch` か publish のアップロードが `/api/login` 以外で 401 を受けた）のときは `util.js::apiResponseError` が `swing:unauthorized` イベントを投げ、`app.js` が以後 `currentRoute()` を（セットアップモードでも）常に `'login'` にしてサイドナビをすべて隠す。ログイン済みのときに `#/login` を開いても既定の画面に落とす。

各画面のロードは世代カウンタ（`createLoadGuard`）でガードし、切り替えが速くても古いレスポンスで上書きしない。

### 止まっている間の呼び出し

`swing up` が止まっている間にポートを取った別のプログラムへセッション cookie を送る機会を減らすため（残る弱点は [`../dashboard.md#既知の弱点`](../dashboard.md#既知の弱点)）、止まっていそうな間は cookie を付けない確認だけを送る。

- `util.js::fetchInstance` は [`POST /api/identity`](http-api.md#post-apiidentity) を `credentials: 'omit'` で呼び（nonce は `crypto.getRandomValues` の 32 バイト）、応答の `instance` を返す（2xx 以外や `instance` が無ければ `null`）。`proof` はブラウザでは確かめられないので見ない。`dashboardAnswers()` はこれが `instance` を返したかどうか（例外も `false`）。
- 再起動を待つとき（Publish 画面のつなぎ直し・Setup 画面の送信後）は `waitForNewInstance(previous)`（`pollUntil` で `fetchInstance` を呼び、`previous` と違う `instance` が返るまで）か、同じ判定を通ってから認証付きの呼び出しをする。
- 定期的な呼び出し（Desktop のおしらせの `/api/activity`、Settings のリソース使用量の `/api/stats`）は、接続できなかった（`apiFetch` の `status === 0`）後は、次の回から `dashboardAnswers()` が `true` になるまで認証付きの呼び出しをしない。失敗が続く間は間隔を `backoffDelay(interval, 失敗回数)`（間隔 × 2^失敗回数。10 分か元の間隔の大きいほうで頭打ち）に延ばし、成功したら元に戻す。

### 画面の一覧

| 画面 | 内容 | 表示スタイル（`data-style`、localStorage キー `swing:style:<view>`） |
|---|---|---|
| Desktop | [`/api/sites`](http-api.md#get-apisites) から `stored: true` のサイトだけを集め、レトロ調（Win95/98 風デスクトップ＋ブラウザ風ウィンドウ内の「リンク集」ページ）に描画する。詳細は [`desktop.md`](desktop.md) | スタイル切替なし |
| Sites | [`/api/sites`](http-api.md#get-apisites) の一覧、mirror への追加・削除、`Unfollowed but still stored`、[`/api/status`](http-api.md#get-apistatus) を呼ぶ Storage check（重いのでボタンを押したときだけ呼ぶ。版ごとの判定の表と、サイトごとの実容量・合計の表） | `cards`（既定）/ `table` |
| Webring | [`/api/webring`](http-api.md#get-apiwebringrootkeydepthn) を root・depth 指定で取得。ノード選択で [`/api/replicas?key=`](http-api.md#get-apireplicaskeykey) を引き、詳細パネルからミラー操作もできる | `graph`（既定）/ `list` / `ascii` / `source`（dot・mermaid） |
| Publish | [`/api/overview`](http-api.md#get-apioverview)・[`/api/publish/sites`](http-api.md#get-apipublishsites)（My sites）、publish フォーム（常にフォルダアップロード） | スタイル切替なし |
| Settings | 設定の表示と編集、テーマ・言語・カスタム CSS、通知（確認の間隔・ブラウザの通知）、リソース使用量、プロセスの停止・再起動（下記「Settings 画面」） | スタイル切替なし |
| Setup | 鍵が未設定（`overview.setup === true`）の間だけ表示できる導入画面。鍵の生成／貼り付け／署名アプリ（NIP-46）とのペアリング、relays、保存上限 3 つを入力して [`POST /api/setup`](http-api.md#post-apisetup) を送る（下記「Setup 画面」） | スタイル切替なし |

## Sites 画面

並び順（`.swing-sort-switch`、`localStorage["swing:sites:sort"]`、既定 `updated`）:

- `updated`: 各アカウントの最初のサイトを `created_at` 降順で比較（アカウント内のサイトも同基準）。サイト無しは最後。
- `name`: 最初のサイトの `d` のロケール順（`localeCompare`）。サイト無しは最後。
- `pubkey`: 画面に出す `npub` の文字列順（`/api/sites` は hex 順で返すが、bech32 の順とは一致しない）。

NIP-05 の検証結果はバッジで `OK`（`verified`）/ `NG`（`mismatch`）/ `ERR`（`error`）/ `N/A`（`not_applicable`）と短く表示し、意味は `title` 属性（マウスオーバー）に表示言語で出す。カードはサイト名の下の行に保存状態・NIP-05・レプリカ数のバッジ（`.swing-site-badges`）をまとめ、NIP-05 には `nip05: ` を前に付け、テーブルでは NIP-05 列にラベルだけを出す。

サイズの表示（`util.js` の `formatSiteSize`）: `stored_size` があれば `formatBytes`（1024 基数で `KiB`・`MiB`・`GiB`・`TiB`、小数 1 桁で `.0` は省く。丸めると 1024 になる値（1023.95 以上）は次の単位に繰り上げる）で、無ければ `size` を `(12.3 MiB)` のように括弧書きで、どちらも無ければ `–`（値の意味は [`http-api.md#get-apisites`](http-api.md#get-apisites)）。

表（`.swing-table`。Sites のテーブル表示、Storage check の 2 つの表、Settings の設定表）の折り返し: 見出しと `.swing-nowrap` を付けたセル（サイズ・日時・保存状態・レプリカ数・短縮した npub と CID・リンクの 1 つずつ）は折り返さない。ほかのセルは単語の途中では折らず（`overflow-wrap: break-word`）、Storage check の Path 列だけは `.swing-break-anywhere`（最小幅 16ch で、どこでも折り返す）で縮む。収まらないときは表 1 つだけの外側（テーブル表示の `.swing-site-set`、Storage check の 2 つの表それぞれを包む `.swing-table-scroll`、`.swing-config-section`）が横スクロールし、見出しや「No problems found.」・合計行は流れず、ページ全体は横にはみ出さない。

`Unfollowed but still stored` セクションも同じ並び順ロジックを共有する。セクションの注記は `unfollowed.remove_on_unfollow` が `false` なら `unfollowedKeepNote`、`true` で `follow_set.found` が `true` なら `unfollowedRemoveNote`（次の更新で消える）、`false` なら `unfollowedNoFollowSetNote`（Follow Set が見つかるまで消えない。[agent の unfollow](../agent.md#unfollow)）。「Stored only」チェックボックスの状態は `localStorage["swing:sites:stored-only"]`。

## Publish 画面

- 自分の情報（`.swing-identity`）: npub・hex・ミラーセット・relays に加えて署名の方式（設定ファイルの秘密鍵／署名アプリ）を `overview.signer` から出す（`publish.js::renderSigner`）。`last_failure` があれば警告を出す。署名アプリのときは画面表示のたびと publish 完了時に `GET /api/overview` を読み直してこの行を更新する。
- 署名アプリのときは「つなぎ直す」ボタンから relay 欄と QR（`pairing.js`）とキャンセルボタンを出す。ペアリングが `ready` になると「つなぎ直して再起動」ボタン（`#pub-signer-save`）が有効になり、押すと [`POST /api/signer/reconnect`](http-api.md#post-apisignerreconnect) を呼び、`waitForNewInstance` で cookie を付けない `POST /api/identity` の `instance` が押す前の `cache.overview.instance` から変わるのを待ってページを読み直す（上記「止まっている間の呼び出し」）。
- publish のアップロード後、署名アプリのときは処理中の表示を `processingOnAgentSigner`（承認を求められたら承認して、という案内）にする。
- publish フォームの NIP-05 の下に、同じ形のセレクトを「ドットファイルの確認」（`check_dotfiles`）・「サイズの確認」（`check_size`）・「同じ内容の確認」（`check_unchanged`）の順に並べる。どれも先頭の選択肢は `modeDefault`（値は空。パートを送らず設定の既定値に任せる）で、残りは `modeOff`（検証しない）・`modeWarn`（見つかった問題の報告のみ行う）・`modeRequire`（問題が見つかったら中止する）の文言にモード名を添えて出す（NIP-05 も同じ）。空でなければ同名のパートで送る（`publish.js::MODE_FIELDS`）。
- publish が成功したら進捗バーを隠し、`#publish-status` に結果（relay N つのうち M つが受け付けたか。全部受け付ければ `ok`、一部だけなら `warn`）を出す。結果のパネルにはサイト・URL・タイトル・NIP-05・サイトの確認（ドットファイル・サイズ・同じ内容。`publish.js::addCheckRows`）・署名（署名アプリのときだけ）・CID・サイズ・作成日時・MFS パス・ファイル数・消した古い版・relay ごとの結果・ゲートウェイのリンクを並べる。ドットファイルは見つかった件数と `paths`（残りは「ほか N 件」）を、`warn` で公開したときは「公開はしています」を添えて出す。
- レスポンスの `published` が `false`（同じ内容で `require` のため公開しなかった）なら、`#publish-status` に `publishUnchanged` を `ok` で出し、結果のパネルから署名・作成日時・MFS パス・relay の行を省く。`swing:published` イベントは出さない（Desktop 画面のおしらせは増えない）。
- 422 のうち本文に `checks` があるもの（ドットファイル・サイズの `require`）は、`require` で引っかかった項目ごとの文（`siteCheckDotfilesBlocked`・`siteCheckSizeBlocked`。CLI のフラグではなく画面の選択と設定のキーで直し方を案内する）を `siteCheckFailed` に入れて出し（どれにも当たらなければ API のエラー文）、結果のパネルに NIP-05 とドットファイル・サイズの行だけを出す。`checks` の無い 422 はNIP-05 の失敗として扱う。
- My sites: 一覧は画面を開いたときにキャッシュが無ければ取得し、publish が成功したときと「再読み込み」で取り直す。一覧の「Use」ボタンは `site`・`url`・`title`（`message` を除く）をフォームに入れるだけ。
- publish フォームは常にフォルダアップロード（`<input type="file" webkitdirectory multiple>`）。ファイル数・合計サイズを表示し、`max_upload` を超えれば送信ボタンを無効化する。送信は `XMLHttpRequest` で、進捗を `.swing-progress`/`.swing-progress-bar`（`data-state`）に反映する。413 は「上限を超えた」という文言に言い換える。
- 各ファイルの送信名は `webkitRelativePath` から選んだフォルダ名を除いたもの。最後に使ったフォーム内容は `swing:publish:last` に保存する（下記の localStorage 一覧を参照）。

## Webring 画面

- root/depth のクエリは `swing:webring:query` に保存し、次に開いたときに復元する。
- 外から自分のノードを開く入口: `document` に `swing:show-self-in-webring`（名前は `notify-settings.js::SHOW_SELF_IN_WEBRING`）を投げると、Webring 画面に移って（すでに表示中ならそのまま）自分（`cache.overview.pubkey`）のノードを選んだ状態にする。今のクエリ（保存済みの `swing:webring:query`）のグラフを読み込み（キャッシュがあればそれ）、自分のノードがあればそのまま選ぶ。無ければ root を空（＝自分）にしてクエリを保存し直し、読み込み直してから選ぶ（depth は変えない）。ブラウザの通知のクリックが使う（[`desktop.md#おしらせの出し分け`](desktop.md#おしらせの出し分け)）。
- 再取得中、既存の表示は消さず `aria-busy="true"` で薄く表示する。表示中の内容が無いとき（初回やエラーの後など）だけ「Loading webring…」になる。
- ノード詳細パネルのミラー操作は選んだノードが自分自身かどうかで変える（自分自身: ボタン無し／ミラー済み: 削除ボタン／未ミラー: 追加ボタン）。判定はキャッシュ済みの `/api/sites` か `/api/mirror` の pubkey 集合。
- `beyond` と `over_budget`（意味は [`../cli.md#webring`](../cli.md#webring)）は、どちらも 0 より大きければ画面下部にヒント文を 1 行ずつ出す。`list` 表示では `referencing`（起点を名指ししているだけでクロールには加えていないアカウント）を「Mutual」「One-way」と並ぶグループとして出し、`more` が 0 より大きければ末尾に件数のヒント文を添える（[「レプリカ報告の信頼度」](../nostr.md#レプリカ報告の信頼度replicastier)）。ノード詳細パネルの報告者一覧は npub の後ろに信頼度の tier（`author`/`chosen`/`other`）に応じたタグを付け、`site.dropped` が 0 より大きければ末尾に「…and N more」相当のヒント文を出す（[取得と表示の上限](../nostr.md#取得と表示の上限nostrbudget)）。

### グラフ（`web/graph.js`）

外部ライブラリを使わない自前の force-directed layout。ドラッグでノードを固定でき、クリックで選択して詳細パネルを開く。キーボード操作（Tab で移動、Enter/Space で選択）に対応する。パン・ホイールズーム（0.15〜4 倍）と全体表示（Fit）ができ、`prefers-reduced-motion: reduce` ではアニメーションせず同期的に 1 回だけ描画する。ノード・辺は class と `data-*` だけを持ち、色は付けない（配色は CSS 側、下記参照）。

## Settings 画面（`settings.js`）

### 設定編集

[`/api/config`](http-api.md#get-apiconfig) を表示し、書き込み範囲（ホワイトリスト。[`../dashboard.md#設定の読み込みと編集srcsettings`](../dashboard.md#設定の読み込みと編集srcsettings)）に入っていて env 由来でない項目はその場で編集できる。ホワイトリストの項目の目印は `item.raw != null`。`renderConfig` はセクションごとに表（キー・値・env）を描く。`item.editable === true`（ホワイトリストにあり env 由来でない）項目は値のセルが入力欄になる（`kind` で分岐: `bool`/`mode` はセレクト、`list` はテキストエリア、それ以外はテキスト入力。値の初期値は `item.raw`）。載っていない項目・env 由来の項目はそのままテキスト表示。env 由来で編集できないホワイトリスト項目には `configLockedByEnv` の注記を添える。ホワイトリストの項目には値のセルの下に `item.description[lang]`（無ければ `.en`）を `swing-hint` として添え、`swing:langchange` の再描画でも更新される。

- セクションごとに 1 つの Save ボタン。押すと、そのセクション内で初期値から変わったフィールドだけを集めて [`PUT /api/config`](http-api.md#put-apiconfig) に送る（変更が無ければ何もしない）。
- `config.writable === false`（設定ファイルが書けない。[`http-api.md#get-apiconfig`](http-api.md#get-apiconfig)）のときは、`editable` な項目も読み取り専用表示にし、`configNotWritable` の注記を出す（Setup 画面は `writable` を見ない）。
- 保存が成功すると `renderConfig` を更新後の `ConfigDto` で描き直し、状態行に `configSaved`（再起動が必要な旨）を出す。失敗時はそのセクションだけ編集可能に戻し、エラーを出す。
- `config.restart_required === true`（今のプロセスでの保存が 1 回でもあった）なら、画面上部に `configRestartRequiredNotice` の注記を常に出す。

### 表示の設定

テーマ（`swing:theme`。`<html data-theme>` を付け替える）・表示言語（`swing:lang`）・カスタム CSS（`swing:user-css`。`<style id="user-css">` に入れる）をブラウザの `localStorage` に保存する（下記「CSS カスタマイズのインターフェース」「表示言語」）。言語を切り替えると `swing:langchange` という `CustomEvent` を `document` に投げ、`app.js` がそれを購読して各画面を再描画する。

### 通知

「表示」パネルの下にある「通知」パネル（`settings-notify.js`）。Desktop 画面の「コントロール パネル」→「通知」タブの「更新の確認」と「ブラウザの通知」と同じ設定（`swing:desktop:mascot` の `interval` と `swing:desktop:notify` の `browser`。[`desktop.md`「コントロール パネル」](desktop.md#コントロール-パネル)）を、Desktop 画面の外から変えるためのもの。サーバの設定（上記「設定編集」）とは独立で、ブラウザにだけ保存される旨の説明を 1 行添える。

- 中身は上から「確認の間隔」のセレクト（1 分 / 5 分 / 15 分 / 30 分 / 確認しない。「通知」タブと同じ選択肢で、値の検証と既定は `notify-settings.js` の `CHECK_INTERVALS`・`readCheckInterval`。「確認しない」にするとブラウザの通知もマスコットのおしらせも来ない旨の注記を `swing-hint` で添える）、「ブラウザの通知を使う」チェックボックス、「通知する内容」の 3 つの種類のチェックボックス（Desktop 画面の文言と同じ意味。使わないときは無効表示）。
- 「表示」パネルのテーマ・言語と同じく、変えたその場で `localStorage` に書く（保存ボタンは無い）。間隔は `writeCheckInterval`（今の値を読み直して `interval` だけを書き換える）で書き、書けたら `desktopUpdates.setInterval` で watcher に当てる。種類と「使う」は、今の値を読み直して `browser` だけを書き換えるので、Desktop 画面側の `mascot` は変えない。watcher とブラウザの通知は使うたびに読むので、その場で効く。
- 「使う」をオンにした操作の中で、許可がまだなら `Notification.requestPermission()` を呼ぶ（`notify-settings.js::requestBrowserPermission`。Desktop 画面と共通）。許可されなければチェックを外して理由を出す。安全なコンテキストでないか `Notification` が無いブラウザではチェックボックスを無効にして理由を出す。保存済みの値がオンでも、画面を開いたときに許可が無くなっていれば理由を出す。理由と保存の失敗は「使う」の下の `#settings-notify-status`（`swing-status`、`data-kind="error"`）に i18n の文で出し、言語を切り替えると出し直す。間隔や種類の変更では、保存の失敗の表示だけを消す（許可の理由は残す）。
- 画面を開くたびに `localStorage` から読み直して表示する（Desktop 画面や別のタブで変えた値を出すため）。

### リソース使用量

「通知」パネルと「プロセス」パネルの間にある（`stats.js`）。Desktop 画面には無い。

- 画面を開くたびに [`GET /api/stats`](http-api.md#get-apistats) を呼び、以後は Settings 画面を表示している間だけ `interval` 秒ごとに呼び直す（画面を離れると次の呼び出しで止まる）。2 回目からは手元の最後の `at` を `since` に渡して新しいサンプルだけを取り、最新のサンプルから 24 時間より古いものを捨てる。
- 表（`.swing-table`）は `swing` の CPU・メモリ、Kubo の CPU・メモリ、IPFS の受信・送信の 6 行で、列は「現在」（最新のサンプル）・「1 時間平均」・「1 時間最大」・「24 時間最大」（最新のサンプルから数えた期間）。値が無ければ `–`。バイト数は `formatBytes`、通信量はそれに `/s` を付ける。
- パネルの説明は 1 分ごとの記録を最大 24 時間さかのぼって見られることと、IPFS の通信量が何を数えたものかだけにする。
- 表の下に最終記録の時刻、最新のサンプルに通信量があれば Kubo 起動からの累計、`kubo_managed` が `false` なら Kubo が SWING の管理外で CPU・メモリを取得できない旨を出す。サンプルが 1 つも無ければ表の代わりに「まだ記録がありません」を出す。
- 読み込みの失敗は `#stats-status` に出し、次の呼び出しは続ける。接続できなかった後の確認と間隔の延ばし方は上記「止まっている間の呼び出し」。

### プロセス操作

設定表の下に「プロセス」パネル（停止・再起動の 2 ボタン）がある。Desktop 画面には無い。

- 停止・再起動はどちらも確認のダイアログの後で `apiFetch` により [`POST /api/shutdown`](http-api.md#post-apishutdown-post-apirestart)／`POST /api/restart` を呼び（`settings.js::runProcessAction`）、呼び出し中はボタンを無効にし、結果かエラーを状態行に出す。
- リクエストが受理された時点で操作としては完了で、実際にプロセスが止まる/再起動するまで画面側では待たない。

## Setup 画面（`setup.js`）

鍵が未設定の間（[`../dashboard.md#セットアップモードと-appstatesetup_mode`](../dashboard.md#セットアップモードと-appstatesetup_mode)）だけ表示できる導入フォーム。`onShow` のたびに `loadOverview()`（キャッシュがあれば取り直さない）で `overview.setup` を見て、`false` なら（既にセットアップ済みなら）`#/sites` に移す。そうでなければ `GET /api/config` を毎回読み直してフォームを埋める。

- 鍵: 「新しい鍵を生成する」（既定）・「既存の鍵を使う」（nsec か hex を 1 行で入力。`type="password"`・`autocomplete="new-password"` で、送信が成功したら欄を空にする）・「スマホの署名アプリで署名する（NIP-46）」のラジオ。
- 署名アプリを選ぶと `#setup-signer-field` を出し、「QRコードを表示」ボタンからペアリングを始める（下記「共通の UI 部品」のペアリング）。状態は `#setup-signer-status` に出す。ペアリングが `ready` でなければ送信せず `setupSignerNotReady` を出す。
- relays（複数行テキストエリア）と保存上限 3 つ（`max_total_storage` / `max_per_site` / `max_per_account`）を `GET /api/config` の `raw` で事前入力する。各フィールドの下に対応する `item.description[lang]` を `swing-hint` として添える（`setup.js::renderFieldDescriptions`）。
- これら 4 項目のうち `GET /api/config` 上で `editable: false`（＝ env 由来。[`../docker.md`](../docker.md)）のものは disabled にして現在値を表示し、`configLockedByEnv` を添える。送信する `items` にもそのキーは含めない（含めるとサーバ側が env 由来として 400 で拒否するため）。
- 送信すると `POST /api/setup`（[`http-api.md#post-apisetup`](http-api.md#post-apisetup)）を叩く。`remote_signer` は署名アプリを選んだときだけ `true`。成功したら `npub` と、秘密鍵なら保存先、署名アプリなら接続情報を `remote-signer.json` に保存したことの案内を表示し、`pollUntil` で、cookie を付けない `fetchInstance` の `instance` が送信前の `cache.overview.instance` から変わってから `GET /api/overview` を読み、`setup: false` になったら `#/settings` へ移る。
- 失敗したらフォームを再度有効にする（env 由来で disabled にしていたフィールドはそのまま disabled に戻す）。

## Login 画面（`login.js`）

- `swing dashboard open` の案内（コマンドを `<code>` で表示）と、ログインコードの入力欄（`swing-inline-form`）・その下の注記（`--no-browser` で表示されたコードを貼る、1 回限り・5 分）を出す。
- 送信すると [`POST /api/login`](http-api.md#post-apilogin) を呼び、成功したら `location.replace('/')` でページごと読み込み直す（cookie が付いた状態で `init()` からやり直す）。401 なら「コードが無効か期限切れ」を、それ以外は `describeError` を `#login-status` に出す。
- ログインリンク（`GET /login?code=`）は `/#/login/code/<code>` にリダイレクトしてくる。`LoginView.init` はこの形のハッシュを見つけると、`history.replaceState` でハッシュを `#/login` に戻してからコードを入力欄に入れるだけで、送信はしない（JavaScript を実行するリンクのプレビューにコードを使われないため）。Login 画面を表示したときは `#login-status` に「リンクのコードを入れたので「ログイン」を押す」旨（`loginLinkReady`、`ok`）を出し、入力欄ではなく送信ボタンにフォーカスする。押すと上と同じ `POST /api/login`。
- `GET /login?code=` が hex でないコードで `/#/login/invalid` にリダイレクトしてきた場合は、表示時に同じ「無効か期限切れ」を出す。
- 状態行の文言は i18n のキーで覚えておき、`swing:langchange` で差し替える（`LoginView.render`）。

## 共通の UI 部品

- busy 表示: `setBusy(button, bool)` で `disabled`・`aria-busy`・`.is-busy` を切り替える。
- コピー: 成功で 1.5 秒だけ `data-copied="true"`、失敗で `data-copy-failed="true"`。表示文字列はすべて共通の `copy` キーで、対象の違いは `aria-label` 側で表す。
- サイドナビ: 下端のトグル（`#nav-toggle`）で畳むとアイコンだけの幅（`--swing-nav-collapsed-width`）になり、各リンクの `title` にラベルを入れる。状態は `localStorage["swing:nav:collapsed"]` に保存し、`<body data-nav="collapsed">` で表す。フッタ（`.swing-nav-footer`）はミラーセット名とバージョンを出す（`publish.js::updateNavFooter`）。
- ロゴタイプ（`.swing-logotype`）: `docs/assets/swing-lockup.svg` の形を、表示する高さ 26px の整数 px の格子に描き直したもの（帯の太さは上から 1,1,1,1,2,2,3px、隙間 1px）。`viewBox` の 1 単位が 1 CSS px で、帯の端がすべて整数 px に乗るので、ナビが 1px 未満ずれて描かれても `crispEdges` の丸めで帯の太さが変わらない。
- ペアリング（`pairing.js::createPairing`）: [`POST /api/setup/signer`](http-api.md#post-apisetupsigner) が返した SVG を `data:` URI の `<img>` で出し、以後 [`GET /api/setup/signer`](http-api.md#get-apisetupsigner) を 1.5 秒間隔でポーリングして状態を状態行に出す（`idle` は `waiting` と同じ表示）。`ready`/`failed` で QR を隠してポーリングを止める。`ready` で確認の署名が通ったときは「確認の署名が通った」とだけ伝え、自動で署名されるとは言わない。通らなかったときは許可を促す警告にする。もう一度押すと新しいペアリングに置き換わり、前のポーリングは捨てる（`createLoadGuard`）。
- 読み込み時: `<body>` 直後の同期スクリプト `boot.js` が `data-nav` を先に付ける。表示言語が英語以外に決まるときは `<html lang>` と `<html data-i18n-pending>` も付けて `[data-i18n]` 要素を隠し、`app.js` が静的な訳を当てた直後にこの属性を外す（動かなかった場合は 1 秒後に英語のまま表示される）。
- モバイル幅: 760px 以下ではナビを横並びにしてトグルとサイドナビのフッタを隠し、ページ最下部の `<footer id="page-footer">` に同じ内容を表示する（Desktop 画面を除く）。

## localStorage キー一覧

ダッシュボードが使う `localStorage` のキーはこれで全部（他にサーバに送るものは無い）。

| キー | 値の形 | 意味 |
|---|---|---|
| `swing:style:<view>`（`view` は `sites`/`webring`） | 文字列（スタイル名） | 画面ごとの表示スタイル |
| `swing:sites:sort` | `updated` / `name` / `pubkey` | Sites の並び順（既定 `updated`） |
| `swing:sites:stored-only` | `"1"` / `"0"` | Sites の「Stored only」チェックボックスの状態 |
| `swing:webring:query` | JSON `{root, depth}` | Webring の最後のクエリ（起動時に復元） |
| `swing:publish:last` | JSON `{site, url, title, message, nip05, check_dotfiles, check_size, check_unchanged}` | Publish フォームの最後の入力（起動時にプリフィル） |
| `swing:nav:collapsed` | `"1"` / `"0"` | サイドナビを畳んでいるか（既定 `0`） |
| `swing:theme` | `auto` / `light` / `dark` | 表示テーマ |
| `swing:lang` | `auto` / `en` / `ja` | 表示言語 |
| `swing:user-css` | 文字列（CSS） | Settings のカスタム CSS 欄の内容 |
| `swing:desktop:visits` | 整数の文字列 | Desktop 画面の来訪者カウンタ（[`desktop.md`](desktop.md#リンク集ページiframe)） |
| `swing:desktop:wallpaper` | JSON（`{color?, image?}`、両方省略可。詳細は [`desktop.md`「コントロール パネル」](desktop.md#コントロール-パネル)） | Desktop 画面の壁紙設定 |
| `swing:desktop:startup` | `"1"` / `"0"` | パスなしで開いたときに Desktop 画面を出すか（既定 `"0"`。[`desktop.md`「コントロール パネル」](desktop.md#コントロール-パネル)） |
| `swing:desktop:mascot` | JSON（`{packs?, interval, walk, chatter}`。詳細は [`mascot.md`「マスコットタブ」](mascot.md#マスコットタブ)。`interval` は「通知」タブが書く） | Desktop 画面のマスコットと更新の確認の間隔の設定（`interval` は Settings 画面の「通知」パネルも書く） |
| `swing:desktop:notify` | JSON（`{mascot: {stored, published, replica}, browser: {enabled, stored, published, replica}}`。詳細は [`desktop.md`「コントロール パネル」](desktop.md#コントロール-パネル)） | おしらせの種類のオン・オフ（マスコットとブラウザの通知で別々）とブラウザの通知を使うかどうか。Desktop 画面の「通知」タブと Settings 画面の「通知」パネルの両方が読み書きする |
| `swing:desktop:seen`・`swing:desktop:seen-published`・`swing:desktop:seen-replicas` | 整数の文字列 | 保存・公開・ミラーする人の増加のおしらせの既読の位置（[`desktop.md`「更新の確認」](desktop.md#更新の確認)） |
| `swing:desktop:replica-reporters` | JSON（`{<d>: {<pubkey>: <初めて見た時刻>}}`） | 前回確認した自分のサイトの報告者の集合（[`desktop.md`「更新の確認」](desktop.md#更新の確認)） |
| `swing:desktop:notified` | JSON（`{<種類>: <at>}`） | ブラウザの通知を出したおしらせの位置（[`desktop.md`「おしらせの出し分け」](desktop.md#おしらせの出し分け)） |

## 表示言語（i18n）

`web/i18n.js` の `MESSAGES = { en: {...}, ja: {...} }` を `t(key, vars)` で参照する。`localStorage["swing:lang"]`（`auto`/`en`/`ja`。`auto` は `navigator.language` が `ja` で始まるかで判定）で切り替え、再読み込みは不要。訳が無いキーは英語にフォールバックする。

訳さないもの: ナビゲーションの「Webring」、webring の ASCII/DOT/Mermaid 出力、API のエラー文字列、npub・hex・CID・パス、環境変数名・設定キー名、`nip05`/`health` のステータス値、NIP-05 とサイトの確認のモードの `off`/`warn`/`require`。日時表示は `Intl.DateTimeFormat`（`ja-JP`/`en-US`）を使う。

Desktop 画面は UI 表示言語の設定に関わらず全部固定の日本語（[`desktop.md`「Desktop 画面は丸ごと日本語固定」](desktop.md#desktop-画面は丸ごと日本語固定)）。

## CSS カスタマイズのインターフェース

読み込み順は `style.css` → Desktop 系 6 ファイル（[`desktop.md#構成`](desktop.md#構成)） → `/custom.css`（サーバ設定、[`../dashboard.md`](../dashboard.md#静的ファイルの配信srcdashboardassetsrs)） → `<style id="user-css">`（ブラウザの `localStorage`、後勝ち）の順。Desktop 画面は `--swing-*` 変数を参照せず、iframe のリンク集ページにはどれも届かない（[`desktop.md`](desktop.md)）。

`--swing-root`（webring の root ノードの色）と `--swing-focus` は `var(--swing-accent)` を参照するので、アクセントを変えるだけで揃って変わる。

- CSS 変数（`web/style.css` の `:root`）: `--swing-bg` `--swing-surface` `--swing-surface-alt` `--swing-surface-raised` `--swing-border` `--swing-border-strong` `--swing-text` `--swing-text-muted` `--swing-text-faint` `--swing-accent` `--swing-accent-strong` `--swing-accent-soft` `--swing-warn` `--swing-warn-soft` `--swing-danger` `--swing-danger-soft` `--swing-ok` `--swing-ok-soft` `--swing-root` `--swing-focus` `--swing-font-display` `--swing-font-ui` `--swing-font-mono` `--swing-space-1`〜`--swing-space-6` `--swing-radius` `--swing-radius-lg` `--swing-nav-width` `--swing-nav-collapsed-width`（既定 64px） `--swing-graph-label-size`（既定 11px）。ほかに `--swing-dark-*`（`--swing-dark-bg` など、`--swing-root`・`--swing-focus` 以外の色変数と同名の組）がある。ダークテーマのときに色変数へ代入される元の値で、ダークテーマの色だけを変えるならこちらを上書きする。
- テーマ: 既定は `@media (prefers-color-scheme: dark)` に連動。`<html data-theme="light"|"dark">` で上書き（Settings 画面が `localStorage["swing:theme"]` に保存してこの属性を付け替える）。
- 状態フック: `<body data-view="desktop|sites|webring|publish|settings|setup|login" data-style="<現在の表示スタイル>" data-nav="collapsed">`（`data-nav` は畳んでいるときだけ）。Desktop 画面でのレイアウトの違いは [`desktop.md#レイアウト`](desktop.md#レイアウト)。
- 安定 class（抜粋。`swing-` 接頭辞で統一。Desktop 画面専用の `desk-` 接頭辞クラスは、リンク集ページ向けの契約（[`desktop.md#リンク集ページiframe`](desktop.md#リンク集ページiframe)）を除いて安定インターフェースではない。[`desktop.md#内部クラス非安定`](desktop.md#内部クラス非安定)）: レイアウト系 `swing-shell` `swing-nav` `swing-nav-list` `swing-nav-icon` `swing-nav-label` `swing-nav-toggle` `swing-main` `swing-view` `swing-panel` `swing-toolbar` `swing-style-switch` `swing-sort-switch`。Sites 系 `swing-site` `swing-site-row` `swing-site-badges` `swing-site-meta-cid` `swing-site-meta-info` `swing-account`。共通部品 `swing-badge` `swing-btn`（`swing-btn-accent`/`swing-btn-danger`/`swing-btn-small`）`swing-copy-btn` `swing-icon-btn` `swing-status` `swing-hint` `swing-table`（セル用に `swing-nowrap` `swing-break-anywhere`）`swing-table-scroll`（表 1 つだけを包む横スクロール用ラッパー）`swing-mono` `swing-pre` `swing-relay-results` `swing-page-footer`。Webring 系 `swing-graph` `swing-node` `swing-edge` `swing-arrow-oneway-fill` `swing-arrow-mutual-fill` `swing-legend` `swing-webring-layout` `swing-node-detail` `swing-source-block`。Publish 系 `swing-my-sites` `swing-progress` `swing-progress-bar` `swing-identity`。Setup 系 `swing-signer-qr`（署名アプリ接続用の QR と注記・コピーボタンを縦に並べる `<figure>`）。
- 状態は data 属性: `data-stored="true|false"`、`data-nip05="verified|mismatch|not_applicable|error"`、`data-health="ok|missing|cid_mismatch|incomplete|check_failed|invalid_key"`、`data-ok="true|false"`（relay 結果）、`data-kind="loading|error|empty|ok|warn"`（`swing-status`）、`data-root`/`data-has-follow-set`/`data-depth`/`data-selected`（グラフのノード）、`data-mutual`（グラフの辺）、`data-style-value`/`data-sort-value`（切替ボタン自身の値）、`data-mirrored="true"`、`data-detail="true|false"`（詳細パネル表示中か）、`data-copied`/`data-copy-failed`、`data-state="uploading|processing|error"`（`swing-progress`。完了時は属性ごと外して隠す）、`aria-busy="true"`（busy 中のボタン、再取得中の webring 表示領域）。リンク集ページの `data-status`・`data-kind` は [`desktop.md#リンク集ページiframe`](desktop.md#リンク集ページiframe) を参照。
- SVG グラフは class と `data-*` だけを付け、色は JS に書かない（`style.css` 側で `.swing-node[data-root="true"] circle { ... }` のように当てる）。
