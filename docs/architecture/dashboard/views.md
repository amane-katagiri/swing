# ダッシュボードの各画面（`web/sites.js`, `web/webring.js`, `web/graph.js`, `web/settings.js`, `web/settings-notify.js`, `web/stats.js`, `web/setup.js`, `web/login.js`）

[`web.md`](web.md) の子ページ。画面の一覧と共通の仕組みは [`web.md`](web.md)、Desktop 画面は [`desktop.md`](desktop.md)、Publish 画面は [`views/publish.md`](views/publish.md) を参照。

## Sites 画面

並び順（`.swing-sort-switch`、`localStorage["swing:sites:sort"]`、既定 `updated`）:

- `updated`: 各アカウントの最初のサイトを `created_at` 降順で比較（アカウント内のサイトも同基準）。サイト無しは最後。
- `name`: 最初のサイトの `d` のロケール順（`localeCompare`）。サイト無しは最後。
- `pubkey`: 画面に出す `npub` の文字列順（hex の順とは一致しない）。

`Unfollowed but still stored` セクションも同じ並び順を使う。セクションの注記は `unfollowed.remove_on_unfollow` が `false` なら残ることを、`true` で `follow_set.found` が `true` なら次の更新で消えることを、`true` で `follow_set.found` が `false` なら Follow Set が見つかるまで消えないこと（[agent の unfollow](../agent.md#unfollow)）を示す。保存状態のフィルター（`#sites-filter-stored` の `<select>`。`all`・`true`（保存済み）・`pending`（更新待ち）・`false`（未保存））の値は `localStorage["swing:sites:stored-filter"]`（既定 `all`）。

NIP-05 の検証結果はバッジで `OK`（`verified`）/ `NG`（`mismatch`）/ `ERR`（`error`）/ `N/A`（`not_applicable`）と短く表示し、意味は `title` 属性に表示言語で出す。保存状態（`ui.js::storedState`）は、`stored` が true なら `true`（保存済み）、false で `previous` があれば `pending`（更新待ち）、どちらでもなければ `false`（未保存）で、バッジとカード・行の `data-stored` に使う。更新待ちのサイトは、カードのメタ情報とテーブルの保存状態の列に保存済みの版の大きさと `created_at` を添え、`gateway_url` が無ければ `previous.gateway_url` へのリンクを出す（カードは「保存済みの版をゲートウェイで開く」、テーブルは幅を抑えるため「ゲートウェイで開く」で、`title` に前者を出す）。カードはサイト名の下の行に保存状態・NIP-05・レプリカ数のバッジ（`.swing-site-badges`）をまとめ、NIP-05 には `nip05: ` を前に付ける。テーブルでは NIP-05 列にラベルだけを出す。

サイズの表示（`util.js::formatSiteSize`）: `stored_size` があればそのまま（1024 基数の `KiB`・`MiB`… で小数 1 桁）、無ければ `size` を `(12.3 MiB)` のように括弧書きで、どちらも無ければ `–`（値の意味は [`http-api/status.md#get-apisites`](http-api/status.md#get-apisites)）。

Storage check は [`/api/status`](http-api/status.md#get-apistatus) をボタンを押したときだけ呼び、版ごとの判定の表と、サイトごとの実容量・合計の表を出す。

表の折り返しは [`web.md#共通の-ui-部品`](web.md#共通の-ui-部品)。

## Publish 画面

[`views/publish.md`](views/publish.md)。

## Webring 画面

- root/depth のクエリは `swing:webring:query` に保存し、次に開いたときに復元する。
- `document` に `swing:show-self-in-webring`（名前は `notify-settings.js::SHOW_SELF_IN_WEBRING`）を投げると、Webring 画面に移って自分（`cache.overview.pubkey`）のノードを選んだ状態にする。今のクエリのグラフを読み込み（キャッシュがあればそれ）、自分のノードがあればそれを選ぶ。無ければ root を空（＝自分）にしてクエリを保存し直し、読み込み直してから選ぶ（depth は変えない）。ブラウザの通知のクリックが使う（[`notices.md#おしらせの出し分け`](notices.md#おしらせの出し分け)）。
- 再取得中は既存の表示を消さず `aria-busy="true"` で薄く表示する。表示中の内容が無いときだけ「Loading webring…」になる。
- ノードの選択で [`/api/replicas?key=`](http-api/nostr.md#get-apireplicaskeykey) を引き、詳細パネルを出す。報告者一覧は npub の後ろに最新版か古い版かのタグと tier のタグ（`author` は `[author]`、`chosen` は `[chosen]`、それ以外は `[unverified]`。日本語表示では `[作者]`・`[フォロー中]`・`[未検証]`）を付け、`site.dropped` が 0 より大きければ末尾に「…and N more」相当のヒント文を出す（[`../nostr/fetch.md`](../nostr/fetch.md)）。レプリカ数のバッジは `unverified` が 0 より大きければ未検証の件数も添える。
- 詳細パネルには「ルートにする」ボタン（root をそのノードに替えて depth はそのままで読み込み直す）と、ミラー操作を置く。ミラー操作は、選んだノードが自分自身ならボタン無し、ミラー済みなら削除ボタン、未ミラーなら追加ボタン。判定はキャッシュ済みの `/api/sites` か `/api/mirror` の pubkey 集合。
- `beyond` と `over_budget`（意味は [`../webring.md`](../webring.md#グラフwebringbuild_graph)）は、0 より大きければ画面下部にヒント文を 1 行ずつ出す。`list` 表示では `referencing`（起点を名指ししているだけでクロールには加えていないアカウント）を「Mutual」「One-way」と並ぶグループとして出し、`more` が 0 より大きければ末尾に件数のヒント文を添える（[`../webring.md`](../webring.md#たどり方webringcrawl)）。

### グラフ（`web/graph.js`）

外部ライブラリを使わない自前の force-directed layout。ドラッグでノードを固定でき、クリックで選択して詳細パネルを開く。キーボード操作（Tab で移動、Enter/Space で選択）に対応する。パン・ホイールズーム（0.15〜4 倍）と全体表示（Fit）ができる。`prefers-reduced-motion: reduce` ではアニメーションせず 1 回だけ描画する。ノード・辺は class と `data-*` だけを持ち、色は CSS 側で当てる（[`css.md`](css.md)）。

## Settings 画面（`settings.js`）

上から設定表・「表示」・「通知」・「リソース使用量」・「プロセス」のパネルが並ぶ。

### 設定編集

[`/api/config`](http-api/config.md#get-apiconfig) をセクションごとの表（キー・値・env）で出す（`renderConfig`）。

- `item.editable === true`（書き込み範囲に入っていて env 由来でない。[`../config.md#編集できるキー`](../config.md#編集できるキー)）の項目は値のセルが入力欄になる。`kind` が `bool`/`mode` ならセレクト、`list` ならテキストエリア、それ以外はテキスト入力で、初期値は `item.raw`。ほかの項目はテキスト表示。
- 書き込み範囲の項目（`item.raw != null`）には、値のセルの下に `item.description[lang]`（無ければ `.en`）を `swing-hint` で添える。env 由来で編集できないものには、env で固定されている旨も添える。
- セクションごとに 1 つの Save ボタン。そのセクション内で初期値から変わったフィールドだけを [`PUT /api/config`](http-api/config.md#put-apiconfig) に送る（変更が無ければ何もしない）。成功すると応答の `ConfigDto` で描き直し、状態行に再起動が必要な旨を出す。失敗したらそのセクションだけ編集できる状態に戻してエラーを出す。
- `config.writable === false` なら、`editable` な項目も読み取り専用で表示し、書き込めない旨の注記を出す。
- `config.restart_required === true` なら、画面上部に再起動待ちの注記を出す。

### 表示の設定

テーマ・表示言語・カスタム CSS をブラウザの `localStorage` に保存する（キーは [`web.md#localstorage-キー一覧`](web.md#localstorage-キー一覧)、効き方は [`css.md`](css.md) と [`web.md#表示言語i18n`](web.md#表示言語i18n)）。

### 通知

「通知」パネル（`settings-notify.js`）は、Desktop 画面の「コントロール パネル」→「通知」タブの「更新の確認」と「ブラウザの通知」と同じ設定を Desktop 画面の外から変える。値・既定・許可の規則は [`notices.md#設定`](notices.md#設定)。サーバの設定とは独立で、ブラウザにだけ保存される旨の説明を 1 行添える。

- 上から「確認の間隔」のセレクト、「ブラウザの通知を使う」チェックボックス、「通知する内容」の 3 つの種類のチェックボックス（使わないときは無効表示）。間隔を「確認しない」にするとブラウザの通知もマスコットのおしらせも来ない旨を `swing-hint` で添える。
- 変えたその場で保存する（保存ボタンは無い。反映の時機は [`notices.md#設定`](notices.md#設定)）。種類と「使う」はブラウザ側（`browser`）だけを書き換える。
- 許可の理由と保存の失敗は `#settings-notify-status`（`swing-status`、`data-kind="error"`）に出し、言語を切り替えると出し直す。間隔や種類を変えたときは保存の失敗の表示だけを消す。
- 画面を開くたびに `localStorage` から読み直す。

### リソース使用量

`stats.js` が受け持つ。Desktop 画面には無い。

- 画面を開くたびに [`GET /api/stats`](http-api/status.md#get-apistats) を呼び、以後は Settings 画面を表示している間だけ `interval` 秒ごとに呼び直す（画面を離れると次の呼び出しで止まる）。2 回目からは手元の最後の `at` を `since` に渡して新しいサンプルだけを取り、最新のサンプルから 24 時間より古いものを捨てる。
- 表（`.swing-table`）は `swing` の CPU・メモリ、Kubo の CPU・メモリ、IPFS の受信・送信の 6 行。列は「現在」（最新のサンプル）・「1 時間平均」・「1 時間最大」・「24 時間最大」（最新のサンプルから数えた期間）。値が無ければ `–`。バイト数は `formatBytes`、通信量はそれに `/s` を付ける。
- パネルの説明は、1 分ごとの記録を最大 24 時間さかのぼって見られることと、IPFS の通信量が何を数えたものか。
- 表の下に最終記録の時刻、最新のサンプルに通信量があれば Kubo 起動からの累計、`kubo_managed` が `false` なら Kubo が SWING の管理外で CPU・メモリを取得できない旨を出す。サンプルが 1 つも無ければ表の代わりに「まだ記録がありません」を出す。
- 読み込みの失敗は `#stats-status` に出し、次の呼び出しは続ける。接続できなかった後の扱いは [`web.md#止まっている間の呼び出し`](web.md#止まっている間の呼び出し)。

### プロセス操作

ページ末尾の「プロセス」パネル（停止・再起動の 2 ボタン）。Desktop 画面には無い。

- どちらも確認のダイアログの後で [`POST /api/shutdown`](http-api/session.md#post-apishutdown-post-apirestart)／`POST /api/restart` を呼び（`settings.js::runProcessAction`）、呼び出し中はボタンを無効にし、結果かエラーを状態行に出す。
- リクエストが受理された時点で完了とし、プロセスが実際に止まる/再起動するまでは待たない。

## Setup 画面（`setup.js`）

鍵が未設定の間（[`../up.md#セットアップモード鍵未設定`](../up.md#セットアップモード鍵未設定)）だけ表示できる導入フォーム。`onShow` のたびに `loadOverview()`（キャッシュがあれば取り直さない）で `overview.setup` を見て、`false` なら `#/sites` に移す。そうでなければ `GET /api/config` を毎回読み直してフォームを埋める。

- 鍵: 「新しい鍵を生成する」（既定）・「既存の鍵を使う」（nsec か hex を 1 行で入力。`type="password"`・`autocomplete="new-password"` で、送信が成功したら欄を空にする）・「スマホの署名アプリで署名する（NIP-46）」のラジオ。
- 署名アプリを選ぶと `#setup-signer-field` を出し、「QRコードを表示」ボタンからペアリングを始める（[`web.md#共通の-ui-部品`](web.md#共通の-ui-部品)のペアリング）。状態は `#setup-signer-status` に出す。ペアリングが `ready` でなければ送信せず、まだ接続していない旨を出す。
- relays（複数行テキストエリア）と保存上限 3 つ（`max_total_storage` / `max_per_site` / `max_per_account`）を `GET /api/config` の `raw` で事前に入れ、各フィールドの下に `item.description[lang]` を `swing-hint` で添える（`setup.js::renderFieldDescriptions`）。
- この 4 項目のうち `editable: false`（env 由来。[`../docker.md`](../docker.md)）のものは disabled にして現在値を表示し、env で固定されている旨を添える。送信する `items` にもそのキーを含めない。Setup 画面は `writable` を見ない。
- 送信すると [`POST /api/setup`](http-api/config.md#post-apisetup) を呼ぶ。`remote_signer` は署名アプリを選んだときだけ `true`。成功したら `npub` と、秘密鍵なら保存先、署名アプリなら接続情報を `remote-signer.json` に保存した旨を表示する。続けて `pollUntil` で、`fetchInstance` の `instance` が送信前の `cache.overview.instance` から変わってから `GET /api/overview` を読み、`setup: false` になったら `#/settings` へ移る。
- 失敗したらフォームを再び有効にする（env 由来で disabled にしていたフィールドは disabled のまま）。

## Login 画面（`login.js`）

- `swing dashboard open` の案内（コマンドを `<code>` で表示）と、ログインコードの入力欄（`swing-inline-form`）・その下の注記（`--no-browser` で表示されたコードを貼る、1 回限り・5 分）を出す。
- 送信すると [`POST /api/login`](http-api/session.md#post-apilogin) を呼び、成功したら `location.replace('/')` でページごと読み込み直す。401 なら「コードが無効か期限切れ」を、それ以外は `describeError` を `#login-status` に出す。
- ログインリンク（`GET /login?code=`）のリダイレクト先は [`security.md#ブラウザ`](security.md#ブラウザ)。`LoginView.init` は `#/login/code/<code>` のハッシュを見つけると、`history.replaceState` でハッシュを `#/login` に戻してからコードを入力欄に入れるだけで、送信はしない。Login 画面を表示したときは `#login-status` に `loginLinkReady`（`ok`。「ログイン」を押す旨）を出し、送信ボタンにフォーカスする。
- `#/login/invalid` のときは、表示時に同じ「無効か期限切れ」を出す。
- 状態行の文言は i18n のキーで覚えておき、`swing:langchange` で差し替える（`LoginView.render`）。
