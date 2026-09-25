# ダッシュボードの画面（`web/index.html`, `web/*.js`, `web/*.css`）

[`../dashboard.md`](../dashboard.md) の一部。サーバ側の起動・ガード・タイムアウト・静的ファイル配信は [`../dashboard.md`](../dashboard.md)、HTTP API の入出力は [`http-api.md`](http-api.md)、Desktop 画面の詳細は [`desktop.md`](desktop.md) を参照。

## 構成

ビルド工程なし・外部依存なしの素の HTML + CSS + ES modules。依存は下から上への一方向で循環 import は無い。

| ファイル | 役割 |
|---|---|
| `storage.js` | `localStorage` の薄いラッパー。他のどのモジュールにも依存しない |
| `i18n.js` | 多言語辞書と `t()`。`storage.js` にだけ依存する |
| `util.js` | 画面間で共有するキャッシュ・DOM/fetch ユーティリティ・表示スタイル切替・非同期ロードのガード（`createLoadGuard`）・`sleep()`・`pollUntil(predicate)`（1 秒間隔・最大 120 回の汎用ポーリング）・他人のイベント由来テキストの表示前サニタイズ（`stripUnsafeUnicode` / `sanitizeDisplayText` / `sanitizeMessage`）。`storage.js`・`i18n.js` に依存する |
| `ui.js` | 複数画面で共有する UI 部品（コピーボタン、バッジ、relay 結果表示、サイト名の行 `buildSiteNameRow` など）。`util.js`・`i18n.js` に依存する |
| `graph.js` | webring 用の自前 force-directed layout。`util.js` の `clamp`・`sanitizeDisplayText` だけに依存する |
| `pairing.js` | 署名アプリ（NIP-46）とのペアリングの部品（`createPairing`）。QR の表示・状態のポーリング・状態表示を受け持ち、Setup 画面と Publish 画面の「署名アプリとつなぎ直す」の両方が使う。`util.js`・`i18n.js` に依存する |
| `sites.js` / `webring.js` / `publish.js` / `settings.js` / `setup.js` / `login.js` | 各画面（それぞれ Sites・Webring・Publish・Settings・Setup・Login）。`setup.js` は `publish.js` の `loadOverview` も使う |
| `boot.js` | 描画前に同期実行する小さな通常スクリプト（下記「共通の UI 部品」の読み込み時）。どのモジュールにも依存しない |
| `app.js` | ルーター兼エントリポイント。`<script type="module" src="/app.js">` から読み込まれ、各画面モジュールを import する |

Desktop 画面専用のモジュール（`desktop*.js`）とその CSS は [`desktop.md#構成`](desktop.md#構成) を参照。

`index.html` が読む CSS は `style.css`（全画面共通）と Desktop 画面用の 3 ファイル（[`desktop.md#構成`](desktop.md#構成)）。アイコンは `index.html` の `<link>` で `favicon-32.png`・`favicon.svg`・`apple-touch-icon.png` の 3 つを指定する。Desktop 画面のアイコンは `web/desktop-icons.svg`（[`desktop.md#構成`](desktop.md#構成)）。

### ルーティング

サイドナビの並び順（上から Desktop・Sites・Webring・Publish・Settings・Setup）と同じ `#/desktop` `#/sites` `#/webring` `#/publish` `#/settings` `#/setup` の 6 画面と、未ログインのときだけ出す Login 画面をハッシュルーティングで切り替える（既定は `sites`）。現在の画面のナビのリンクには `aria-current="page"` が付く。書き込みリクエストは `util.js::apiFetch` が `X-Swing-Dashboard: 1` と `Content-Type: application/json` を付ける。publish のアップロード（`XMLHttpRequest` + `FormData`）だけは `apiFetch` を通らず、`X-Swing-Dashboard: 1` だけを自分で付ける。relay 由来の文字列は DOM API だけで挿入し、`innerHTML` は使わない。`url` は `^https?://` にマッチするときだけリンクにする。

### 表示前のサニタイズ

他人の Nostr イベント由来のテキスト（サイトの `title`・`d`・`message`、webring のラベル・names など）は、表示直前の 1 箇所（`util.js::sanitizeDisplayText`。内部で `stripUnsafeUnicode` と `stripControlChars` を呼ぶ。`sanitizeMessage` は既定 200 文字で `…` に切り詰め、空なら `null` にするラッパー）だけで、制御文字（空白 1 つに置き換える）・双方向制御文字と isolate・不可視文字・ゼロ幅文字を取り除いてから DOM に入れる（対象の範囲は `stripControlChars`・`stripUnsafeUnicode` が正本）。対象の要素には `dir="auto"` と CSS 側の `unicode-bidi: isolate` を付け、正当な RTL（アラビア語・ヘブライ語タイトルなど）はそのまま表示しつつ周囲の UI の並びには影響しないようにする。webring の DOT/Mermaid/ASCII エクスポートは `stripUnsafeUnicode` だけを通す（制御文字は残す）。オペレーター自身のフォーム入力（Publish フォームへの再入力など）には適用しない。

### 起動とログイン状態

`app.js` の `init()` は `loadOverview()`（[`/api/overview`](http-api.md#get-apioverview)）を待ってから `showRoute()` を呼ぶ。`cache.overview.setup` が `true` の間は `currentRoute()` が hash に関わらず `'setup'` を返し（下記の未ログインの判定が優先）、サイドナビも Setup 項目だけを表示する（`false` の間は Setup 項目を隠し、残りの項目を出す）。

未ログイン（`apiFetch` が `/api/login` 以外で 401 を受けた）のときは `util.js::apiFetch` が `swing:unauthorized` イベントを投げ、`app.js` が以後 `currentRoute()` を（セットアップモードでも）常に `'login'` にしてサイドナビをすべて隠す。ログイン済みのときに `#/login` を開いても `sites` に落とす。

各画面のロードは世代カウンタ（`createLoadGuard`）でガードし、切り替えが速くても古いレスポンスで上書きしない。

### 画面の一覧

| 画面 | 内容 | 表示スタイル（`data-style`、localStorage キー `swing:style:<view>`） |
|---|---|---|
| Desktop | [`/api/sites`](http-api.md#get-apisites) から `stored: true` のサイトだけを集め、レトロ調（Win95/98 風デスクトップ＋ブラウザ風ウィンドウ内の「リンク集」ページ）に描画する。詳細は [`desktop.md`](desktop.md) | スタイル切替なし |
| Sites | [`/api/sites`](http-api.md#get-apisites) の一覧、mirror への追加・削除、`Unfollowed but still stored`、[`/api/status`](http-api.md#get-apistatus) を呼ぶ Storage check（重いのでボタンを押したときだけ呼ぶ。版ごとの判定の表と、サイトごとの実容量・合計の表） | `cards`（既定）/ `table` |
| Webring | [`/api/webring`](http-api.md#get-apiwebringrootkeydepthn) を root・depth 指定で取得。ノード選択で [`/api/replicas?key=`](http-api.md#get-apireplicaskeykey) を引き、詳細パネルからミラー操作もできる | `graph`（既定）/ `list` / `ascii` / `source`（dot・mermaid） |
| Publish | [`/api/overview`](http-api.md#get-apioverview)・[`/api/publish/sites`](http-api.md#get-apipublishsites)（My sites）、publish フォーム（常にフォルダアップロード） | スタイル切替なし |
| Settings | 設定の表示と編集、テーマ・言語・カスタム CSS、プロセスの停止・再起動（下記「Settings 画面」） | スタイル切替なし |
| Setup | 鍵が未設定（`overview.setup === true`）の間だけ表示できる導入画面。鍵の生成／貼り付け／署名アプリ（NIP-46）とのペアリング、relays、保存上限 3 つを入力して [`POST /api/setup`](http-api.md#post-apisetup) を送る（下記「Setup 画面」） | スタイル切替なし |

## Sites 画面

並び順（`.swing-sort-switch`、`localStorage["swing:sites:sort"]`、既定 `updated`）:

- `updated`: 各アカウントの最初のサイトを `created_at` 降順で比較（アカウント内のサイトも同基準）。サイト無しは最後。
- `name`: 最初のサイトの `d` のロケール順（`localeCompare`）。サイト無しは最後。
- `pubkey`: 画面に出す `npub` の文字列順（`/api/sites` は hex 順で返すが、bech32 の順とは一致しない）。

NIP-05 の検証結果はバッジで `OK`（`verified`）/ `NG`（`mismatch`）/ `ERR`（`error`）/ `N/A`（`not_applicable`）と短く表示し、意味は `title` 属性（マウスオーバー）に表示言語で出す。カードはサイト名の下の行に保存状態・NIP-05・レプリカ数のバッジ（`.swing-site-badges`）をまとめ、NIP-05 には `nip05: ` を前に付け、テーブルでは NIP-05 列にラベルだけを出す。

サイズの表示（`util.js` の `formatSiteSize`）: `stored_size` があれば `formatBytes`（1024 基数で `KiB`・`MiB`・`GiB`・`TiB`、小数 1 桁で `.0` は省く）で、無ければ `size` を `(12.3 MiB)` のように括弧書きで、どちらも無ければ `–`（値の意味は [`http-api.md#get-apisites`](http-api.md#get-apisites)）。

`Unfollowed but still stored` セクションも同じ並び順ロジックを共有する。セクションの注記は `unfollowed.remove_on_unfollow` が `false` なら `unfollowedKeepNote`、`true` で `follow_set.found` が `true` なら `unfollowedRemoveNote`（次の更新で消える）、`false` なら `unfollowedNoFollowSetNote`（Follow Set が見つかるまで消えない。[agent の unfollow](../agent.md#unfollow)）。「Stored only」チェックボックスの状態は `localStorage["swing:sites:stored-only"]`。

## Publish 画面

- 自分の情報（`.swing-identity`）: npub・hex・ミラーセット・relays に加えて署名の方式（設定ファイルの秘密鍵／署名アプリ）を `overview.signer` から出す（`publish.js::renderSigner`）。`last_failure` があれば警告を出す。署名アプリのときは画面表示のたびと publish 完了時に `GET /api/overview` を読み直してこの行を更新する。
- 署名アプリのときは「つなぎ直す」ボタンから relay 欄と QR（`pairing.js`）とキャンセルボタンを出す。ペアリングが `ready` になると「つなぎ直して再起動」ボタン（`#pub-signer-save`）が有効になり、押すと [`POST /api/signer/reconnect`](http-api.md#post-apisignerreconnect) を呼び、`pollUntil` で `GET /api/overview` を読んで `instance` が変わったらページを読み直す。
- publish のアップロード後、署名アプリのときは処理中の表示を `processingOnAgentSigner`（承認を求められたら承認して、という案内）にする。
- publish が成功したら進捗バーを隠し、`#publish-status` に結果（relay N つのうち M つが受け付けたか。全部受け付ければ `ok`、一部だけなら `warn`）を出す。結果のパネルにはサイト・URL・タイトル・NIP-05・署名（署名アプリのときだけ）・CID・サイズ・作成日時・MFS パス・ファイル数・消した古い版・relay ごとの結果・ゲートウェイのリンクを並べる。
- My sites: 一覧は画面を開いたときにキャッシュが無ければ取得し、publish が成功したときと「再読み込み」で取り直す。一覧の「Use」ボタンは `site`・`url`・`title`（`message` を除く）をフォームに入れるだけ。
- publish フォームは常にフォルダアップロード（`<input type="file" webkitdirectory multiple>`）。ファイル数・合計サイズを表示し、`max_upload` を超えれば送信ボタンを無効化する。送信は `XMLHttpRequest` で、進捗を `.swing-progress`/`.swing-progress-bar`（`data-state`）に反映する。413 は「上限を超えた」という文言に言い換える。
- 各ファイルの送信名は `webkitRelativePath` から選んだフォルダ名を除いたもの。最後に使ったフォーム内容は `swing:publish:last` に保存する（下記の localStorage 一覧を参照）。

## Webring 画面

- root/depth のクエリは `swing:webring:query` に保存し、次に開いたときに復元する。
- 再取得中、既存の表示は消さず `aria-busy="true"` で薄く表示する。表示中の内容が無いとき（初回やエラーの後など）だけ「Loading webring…」になる。
- ノード詳細パネルのミラー操作は選んだノードが自分自身かどうかで変える（自分自身: ボタン無し／ミラー済み: 削除ボタン／未ミラー: 追加ボタン）。判定はキャッシュ済みの `/api/sites` か `/api/mirror` の pubkey 集合。
- `beyond` と `over_budget`（意味は [`../cli.md#webring`](../cli.md#webring)）は、どちらも 0 より大きければ画面下部にヒント文を 1 行ずつ出す。`list` 表示では `referencing`（起点を名指ししているだけでクロールには加えていないアカウント）を「Mutual」「One-way」と並ぶグループとして出し、`more` が 0 より大きければ末尾に件数のヒント文を添える（[「レプリカ報告の信頼度」](../nostr.md#レプリカ報告の信頼度replicastier)）。ノード詳細パネルの報告者一覧は npub の後ろに信頼度の tier（`author`/`chosen`/`other`）に応じたタグを付け、`site.dropped` が 0 より大きければ末尾に「…and N more」相当のヒント文を出す（[取得と表示の上限](../nostr.md#取得と表示の上限nostrbudget)）。

### グラフ（`web/graph.js`）

外部ライブラリを使わない自前の force-directed layout。ドラッグでノードを固定でき、クリックで選択して詳細パネルを開く。キーボード操作（Tab で移動、Enter/Space で選択）に対応する。パン・ホイールズーム（0.15〜4 倍）と全体表示（Fit）ができ、`prefers-reduced-motion: reduce` ではアニメーションせず同期的に 1 回だけ描画する。ノード・辺は class と `data-*` だけを持ち、色は付けない（配色は CSS 側、下記参照）。

## Settings 画面（`settings.js`）

### 設定編集

[`/api/config`](http-api.md#get-apiconfig) を表示し、書き込み範囲（ホワイトリスト。[`../dashboard.md#設定の読み込みと編集srcsettings`](../dashboard.md#設定の読み込みと編集srcsettings)）に入っていて env 由来でない項目はその場で編集できる。ホワイトリストの項目の目印は `item.raw != null`。`renderConfig` はセクションごとに表（キー・値・env）を描く。`item.editable === true`（ホワイトリストにあり env 由来でない）項目は値のセルが入力欄になる（`kind` で分岐: `bool`/`nip05` はセレクト、`list` はテキストエリア、それ以外はテキスト入力。値の初期値は `item.raw`）。載っていない項目・env 由来の項目はそのままテキスト表示。env 由来で編集できないホワイトリスト項目には `configLockedByEnv` の注記を添える。ホワイトリストの項目には値のセルの下に `item.description[lang]`（無ければ `.en`）を `swing-hint` として添え、`swing:langchange` の再描画でも更新される。

- セクションごとに 1 つの Save ボタン。押すと、そのセクション内で初期値から変わったフィールドだけを集めて [`PUT /api/config`](http-api.md#put-apiconfig) に送る（変更が無ければ何もしない）。
- `config.writable === false`（設定ファイルが書けない。[`http-api.md#get-apiconfig`](http-api.md#get-apiconfig)）のときは、`editable` な項目も読み取り専用表示にし、`configNotWritable` の注記を出す（Setup 画面は `writable` を見ない）。
- 保存が成功すると `renderConfig` を更新後の `ConfigDto` で描き直し、状態行に `configSaved`（再起動が必要な旨）を出す。失敗時はそのセクションだけ編集可能に戻し、エラーを出す。
- `config.restart_required === true`（今のプロセスでの保存が 1 回でもあった）なら、画面上部に `configRestartRequiredNotice` の注記を常に出す。

### 表示の設定

テーマ（`swing:theme`。`<html data-theme>` を付け替える）・表示言語（`swing:lang`）・カスタム CSS（`swing:user-css`。`<style id="user-css">` に入れる）をブラウザの `localStorage` に保存する（下記「CSS カスタマイズのインターフェース」「表示言語」）。言語を切り替えると `swing:langchange` という `CustomEvent` を `document` に投げ、`app.js` がそれを購読して各画面を再描画する。

### プロセス操作

設定表の下に「プロセス」パネル（停止・再起動の 2 ボタン）がある。Desktop 画面には無い。

- 停止・再起動はどちらも確認のダイアログの後で `apiFetch` により [`POST /api/shutdown`](http-api.md#post-apishutdown-post-apirestart)／`POST /api/restart` を呼び（`settings.js::runProcessAction`）、呼び出し中はボタンを無効にし、結果かエラーを状態行に出す。
- リクエストが受理された時点で操作としては完了で、実際にプロセスが止まる/再起動するまで画面側では待たない。

## Setup 画面（`setup.js`）

鍵が未設定の間（[`../dashboard.md#セットアップモードと-appstatesetup_mode`](../dashboard.md#セットアップモードと-appstatesetup_mode)）だけ表示できる導入フォーム。`onShow` のたびに `loadOverview()`（キャッシュがあれば取り直さない）で `overview.setup` を見て、`false` なら（既にセットアップ済みなら）`#/sites` に移す。そうでなければ `GET /api/config` を毎回読み直してフォームを埋める。

- 鍵: 「新しい鍵を生成する」（既定）・「既存の鍵を使う」（nsec か hex を 1 行で入力）・「スマホの署名アプリで署名する（NIP-46）」のラジオ。
- 署名アプリを選ぶと `#setup-signer-field` を出し、「QRコードを表示」ボタンからペアリングを始める（下記「共通の UI 部品」のペアリング）。状態は `#setup-signer-status` に出す。ペアリングが `ready` でなければ送信せず `setupSignerNotReady` を出す。
- relays（複数行テキストエリア）と保存上限 3 つ（`max_total_storage` / `max_per_site` / `max_per_account`）を `GET /api/config` の `raw` で事前入力する。各フィールドの下に対応する `item.description[lang]` を `swing-hint` として添える（`setup.js::renderFieldDescriptions`）。
- これら 4 項目のうち `GET /api/config` 上で `editable: false`（＝ env 由来。[`../docker.md`](../docker.md)）のものは disabled にして現在値を表示し、`configLockedByEnv` を添える。送信する `items` にもそのキーは含めない（含めるとサーバ側が env 由来として 400 で拒否するため）。
- 送信すると `POST /api/setup`（[`http-api.md#post-apisetup`](http-api.md#post-apisetup)）を叩く。`remote_signer` は署名アプリを選んだときだけ `true`。成功したら `npub` と、秘密鍵なら保存先、署名アプリなら接続情報を `remote-signer.json` に保存したことの案内を表示し、`pollUntil` で `GET /api/overview` を読んで `setup: false` になったら `#/settings` へ移る。
- 失敗したらフォームを再度有効にする（env 由来で disabled にしていたフィールドはそのまま disabled に戻す）。

## Login 画面（`login.js`）

- `swing dashboard open` の案内（コマンドを `<code>` で表示）と、ログインコードの入力欄（`swing-inline-form`）・その下の注記（`--no-browser` で表示されたコードを貼る、1 回限り・5 分）を出す。
- 送信すると [`POST /api/login`](http-api.md#post-apilogin) を呼び、成功したら `location.replace('/')` でページごと読み込み直す（cookie が付いた状態で `init()` からやり直す）。401 なら「コードが無効か期限切れ」を、それ以外は `describeError` を `#login-status` に出す。
- `GET /login?code=` が無効なコードで `/#/login/invalid` にリダイレクトしてきた場合は、表示時に同じ「無効か期限切れ」を出す。
- 状態行の文言は i18n のキーで覚えておき、`swing:langchange` で差し替える（`LoginView.render`）。

## 共通の UI 部品

- busy 表示: `setBusy(button, bool)` で `disabled`・`aria-busy`・`.is-busy` を切り替える。
- コピー: 成功で 1.5 秒だけ `data-copied="true"`、失敗で `data-copy-failed="true"`。表示文字列はすべて共通の `copy` キーで、対象の違いは `aria-label` 側で表す。
- サイドナビ: 下端のトグル（`#nav-toggle`）で畳むとアイコンだけの幅（`--swing-nav-collapsed-width`）になり、各リンクの `title` にラベルを入れる。状態は `localStorage["swing:nav:collapsed"]` に保存し、`<body data-nav="collapsed">` で表す。フッタ（`.swing-nav-footer`）はミラーセット名とバージョンを出す（`publish.js::updateNavFooter`）。
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
| `swing:publish:last` | JSON `{site, url, title, message, nip05}` | Publish フォームの最後の入力（起動時にプリフィル） |
| `swing:nav:collapsed` | `"1"` / `"0"` | サイドナビを畳んでいるか（既定 `0`） |
| `swing:theme` | `auto` / `light` / `dark` | 表示テーマ |
| `swing:lang` | `auto` / `en` / `ja` | 表示言語 |
| `swing:user-css` | 文字列（CSS） | Settings のカスタム CSS 欄の内容 |
| `swing:desktop:visits` | 整数の文字列 | Desktop 画面の来訪者カウンタ（[`desktop.md`](desktop.md#リンク集ページiframe)） |
| `swing:desktop:wallpaper` | JSON（`{color?, image?}`、両方省略可。詳細は [`desktop.md`「コントロール パネル」](desktop.md#コントロール-パネル)） | Desktop 画面の壁紙設定 |

## 表示言語（i18n）

`web/i18n.js` の `MESSAGES = { en: {...}, ja: {...} }` を `t(key, vars)` で参照する。`localStorage["swing:lang"]`（`auto`/`en`/`ja`。`auto` は `navigator.language` が `ja` で始まるかで判定）で切り替え、再読み込みは不要。訳が無いキーは英語にフォールバックする。

訳さないもの: ナビゲーションの「Webring」、webring の ASCII/DOT/Mermaid 出力、API のエラー文字列、npub・hex・CID・パス、環境変数名・設定キー名、`nip05`/`health` のステータス値、NIP-05 モードの `off`/`warn`/`require`。日時表示は `Intl.DateTimeFormat`（`ja-JP`/`en-US`）を使う。

Desktop 画面は UI 表示言語の設定に関わらず全部固定の日本語（[`desktop.md`「Desktop 画面は丸ごと日本語固定」](desktop.md#desktop-画面は丸ごと日本語固定)）。

## CSS カスタマイズのインターフェース

読み込み順は `style.css` → Desktop 系 3 ファイル（[`desktop.md#構成`](desktop.md#構成)） → `/custom.css`（サーバ設定、[`../dashboard.md`](../dashboard.md#静的ファイルの配信srcdashboardassetsrs)） → `<style id="user-css">`（ブラウザの `localStorage`、後勝ち）の順。Desktop 画面は `--swing-*` 変数を参照せず、iframe のリンク集ページにはどれも届かない（[`desktop.md`](desktop.md)）。

`--swing-root`（webring の root ノードの色）と `--swing-focus` は `var(--swing-accent)` を参照するので、アクセントを変えるだけで揃って変わる。

- CSS 変数（`web/style.css` の `:root`）: `--swing-bg` `--swing-surface` `--swing-surface-alt` `--swing-surface-raised` `--swing-border` `--swing-border-strong` `--swing-text` `--swing-text-muted` `--swing-text-faint` `--swing-accent` `--swing-accent-strong` `--swing-accent-soft` `--swing-warn` `--swing-warn-soft` `--swing-danger` `--swing-danger-soft` `--swing-ok` `--swing-ok-soft` `--swing-root` `--swing-focus` `--swing-font-display` `--swing-font-ui` `--swing-font-mono` `--swing-space-1`〜`--swing-space-6` `--swing-radius` `--swing-radius-lg` `--swing-nav-width` `--swing-nav-collapsed-width`（既定 64px） `--swing-graph-label-size`（既定 11px）。ほかに `--swing-dark-*`（`--swing-dark-bg` など、`--swing-root`・`--swing-focus` 以外の色変数と同名の組）がある。ダークテーマのときに色変数へ代入される元の値で、ダークテーマの色だけを変えるならこちらを上書きする。
- テーマ: 既定は `@media (prefers-color-scheme: dark)` に連動。`<html data-theme="light"|"dark">` で上書き（Settings 画面が `localStorage["swing:theme"]` に保存してこの属性を付け替える）。
- 状態フック: `<body data-view="desktop|sites|webring|publish|settings|setup|login" data-style="<現在の表示スタイル>" data-nav="collapsed">`（`data-nav` は畳んでいるときだけ）。Desktop 画面でのレイアウトの違いは [`desktop.md#レイアウト`](desktop.md#レイアウト)。
- 安定 class（抜粋。`swing-` 接頭辞で統一。Desktop 画面専用の `desk-` 接頭辞クラスは、リンク集ページ向けの契約（[`desktop.md#リンク集ページiframe`](desktop.md#リンク集ページiframe)）を除いて安定インターフェースではない。[`desktop.md#内部クラス非安定`](desktop.md#内部クラス非安定)）: レイアウト系 `swing-shell` `swing-nav` `swing-nav-list` `swing-nav-icon` `swing-nav-label` `swing-nav-toggle` `swing-main` `swing-view` `swing-panel` `swing-toolbar` `swing-style-switch` `swing-sort-switch`。Sites 系 `swing-site` `swing-site-row` `swing-site-badges` `swing-site-meta-cid` `swing-site-meta-info` `swing-account`。共通部品 `swing-badge` `swing-btn`（`swing-btn-accent`/`swing-btn-danger`/`swing-btn-small`）`swing-copy-btn` `swing-icon-btn` `swing-status` `swing-hint` `swing-table` `swing-mono` `swing-pre` `swing-relay-results` `swing-page-footer`。Webring 系 `swing-graph` `swing-node` `swing-edge` `swing-arrow-oneway-fill` `swing-arrow-mutual-fill` `swing-legend` `swing-webring-layout` `swing-node-detail` `swing-source-block`。Publish 系 `swing-my-sites` `swing-progress` `swing-progress-bar` `swing-identity`。Setup 系 `swing-signer-qr`（署名アプリ接続用の QR と注記・コピーボタンを縦に並べる `<figure>`）。
- 状態は data 属性: `data-stored="true|false"`、`data-nip05="verified|mismatch|not_applicable|error"`、`data-health="ok|missing|cid_mismatch|incomplete|check_failed|invalid_key"`、`data-ok="true|false"`（relay 結果）、`data-kind="loading|error|empty|ok|warn"`（`swing-status`）、`data-root`/`data-has-follow-set`/`data-depth`/`data-selected`（グラフのノード）、`data-mutual`（グラフの辺）、`data-style-value`/`data-sort-value`（切替ボタン自身の値）、`data-mirrored="true"`、`data-detail="true|false"`（詳細パネル表示中か）、`data-copied`/`data-copy-failed`、`data-state="uploading|processing|error"`（`swing-progress`。完了時は属性ごと外して隠す）、`aria-busy="true"`（busy 中のボタン、再取得中の webring 表示領域）。リンク集ページの `data-status`・`data-kind` は [`desktop.md#リンク集ページiframe`](desktop.md#リンク集ページiframe) を参照。
- SVG グラフは class と `data-*` だけを付け、色は JS に書かない（`style.css` 側で `.swing-node[data-root="true"] circle { ... }` のように当てる）。
