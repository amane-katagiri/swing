# ダッシュボードの各画面（`web/sites.js`, `web/setup.js`, `web/login.js`）

[`web.md`](web.md) の子ページ。画面の一覧と共通の仕組みは [`web.md`](web.md)、Desktop 画面は [`desktop.md`](desktop.md)。子ページ:

- [`views/publish.md`](views/publish.md): Publish 画面（自分の情報・署名アプリ・フォーム・公開前の確認・結果・My sites）
- [`views/webring.md`](views/webring.md): Webring 画面とグラフの配置（`web/graph.js`）
- [`views/settings.md`](views/settings.md): Settings 画面（設定編集・表示・通知・リソース使用量・プロセス操作）

## Sites 画面

並び順（`.swing-sort-switch`、`localStorage["swing:sites:sort"]`、既定 `updated`）:

- `updated`: 各アカウントの最初のサイトを `created_at` 降順で比較（アカウント内のサイトも同基準）。サイト無しは最後。
- `name`: 最初のサイトの `d` のロケール順（`localeCompare`）。サイト無しは最後。
- `pubkey`: 画面に出す `npub` の文字列順（hex の順とは一致しない）。

`Unfollowed but still stored` セクションも同じ並び順を使う。セクションの注記は `unfollowed.remove_on_unfollow` が `false` なら残ることを、`true` で `follow_set.found` が `true` なら次の更新で消えることを、`true` で `follow_set.found` が `false` なら Follow Set が見つかるまで消えないこと（[agent の unfollow](../agent.md#unfollow)）を示す。

絞り込みは 2 つ。テキスト欄は `d`・`message`・`cid`・`url`・npub・hex の pubkey を大小文字を無視して部分一致で探す。保存状態のフィルター（`#sites-filter-stored` の `<select>`。`all`・`true`（保存済み）・`pending`（更新待ち）・`false`（未保存））の値は `localStorage["swing:sites:stored-filter"]`（既定 `all`）。

NIP-05 の検証結果はバッジで `OK`（`verified`）/ `NG`（`mismatch`）/ `ERR`（`error`）/ `N/A`（`not_applicable`）と短く表示し、意味は `title` 属性に表示言語で出す。保存状態（`ui.js::storedState`）は、`stored` が true なら `true`（保存済み）、false で `previous` があれば `pending`（更新待ち）、どちらでもなければ `false`（未保存）で、バッジとカード・行の `data-stored` に使う。更新待ちのサイトは、カードのメタ情報とテーブルの保存状態の列に保存済みの版の大きさと `created_at` を添え、`gateway_url` が無ければ `previous.gateway_url` へのリンクを出す（カードは「保存済みの版をゲートウェイで開く」、テーブルは「ゲートウェイで開く」で、`title` に前者を出す）。カードはサイト名の下の行に保存状態・NIP-05・レプリカ数のバッジ（`.swing-site-badges`）をまとめ、NIP-05 には `nip05: ` を前に付ける。テーブルでは NIP-05 列にラベルだけを出す。

サイズの表示（`util.js::formatSiteSize`）: `stored_size` があればそのまま（`formatBytes`。1024 基数の `KiB`・`MiB`… で小数 1 桁、`.0` は省く）、無ければ `size` を `(12.3 MiB)` のように括弧書きで、どちらも無ければ `–`（値の意味は [`http-api/status.md#get-apisites`](http-api/status.md#get-apisites)）。

「ミラーに追加」の入力欄は、入力のたびに 250ms 待ってから鍵を [`POST /api/mirror/preview`](http-api/nostr.md#post-apimirrorpreview) に送り、入力欄の下の `#mirror-add-preview`（`.swing-key-preview`）に鍵ごとの行を出す（`ui.js::createKeyPreview`。古い要求の応答は捨てる）。読める鍵は絵文字のフィンガープリント（4 つ）と縮めた npub（`title` に全体）、読めない鍵は `keyPreviewInvalid` の文面を出す。入力が空・件数が上限（`MAX_MIRROR_KEYS`、100）を超える・API が失敗したとき（追加できて入力欄を空にしたときも）は、`keyPreviewEmpty` の 1 行（`.swing-hint`）に戻す。行の高さはフィンガープリントのバッジに合わせて揃え、どの行に替わっても高さが変わらない。

アカウントの見出し（カード表示・テーブル表示とも、`Unfollowed but still stored` も含む）は、縮めた npub の前に絵文字のフィンガープリント（4 つ。[`fingerprint.md`](fingerprint.md)）を出す。

Storage check は [`/api/status`](http-api/status.md#get-apistatus) をボタンを押したときだけ呼び、版ごとの判定の表と、サイトごとの実容量・合計の表を出す。どちらの表もアカウント列の縮めた npub の前に、`/api/status` の各行の `fingerprint` の先頭 1 つを出す。

表の折り返しは [`web/ui.md`](web/ui.md)。

## Setup 画面（`setup.js`）

鍵が未設定の間（[`../up.md#セットアップモード鍵未設定`](../up.md#セットアップモード鍵未設定)）だけ表示できる導入フォーム。`onShow` のたびに `loadOverview()`（キャッシュがあれば取り直さない）で `overview.setup` を見て、`false` なら `#/sites` に移す。そうでなければ `GET /api/config` を毎回読み直してフォームを埋める。

- 鍵: 「新しい鍵を生成する」（既定）・「既存の鍵を使う」（nsec か hex を 1 行で入力。`type="password"`・`autocomplete="new-password"` で、送信が成功したら欄を空にする）・「スマホの署名アプリで署名する（NIP-46）」のラジオ。
- 署名アプリを選ぶと `#setup-signer-field` を出し、「QRコードを表示」ボタンからペアリングを始める（[`web/ui.md`](web/ui.md) のペアリング）。状態は `#setup-signer-status` に出す。ペアリングが `ready` でなければ送信せず、まだ接続していない旨を出す。
- relays（複数行テキストエリア）と保存上限 3 つ（`max_total_storage` / `max_per_site` / `max_per_account`）を `GET /api/config` の `raw` で事前に入れ、各フィールドの下に `item.description[lang]` を `swing-hint` で添える（`setup.js::renderFieldDescriptions`）。
- この 4 項目のうち `editable: false`（env 由来。[`../docker.md`](../docker.md)）のものは disabled にして現在値を表示し、env で固定されている旨を添える。送信する `items` にもそのキーを含めない。Setup 画面は `writable` を見ない。
- 送信すると [`POST /api/setup`](http-api/config.md#post-apisetup) を呼ぶ。`remote_signer` は署名アプリを選んだときだけ `true`。成功したら `npub` と、秘密鍵なら保存先、署名アプリなら接続情報を `remote-signer.json` に保存した旨を表示する。続けて `pollUntil` で、`fetchInstance` の `instance` が送信前の `cache.overview.instance` から変わってから `GET /api/overview` を読み、`setup: false` になったら `#/settings` へ移る。`pollUntil` が尽きたら時間切れのエラーを出す。
- 失敗したらフォームを再び有効にする（env 由来で disabled にしていたフィールドは disabled のまま）。

## Login 画面（`login.js`）

- `swing dashboard open` の案内（コマンドを `<code>` で表示）と、ログインコードの入力欄（`swing-inline-form`）・その下の注記（`--no-browser` で表示されたコードを貼る、1 回限り・5 分）を出す。
- 送信すると [`POST /api/login`](http-api/session.md#post-apilogin) を呼び、成功したら `location.replace('/')` でページごと読み込み直す。401 なら「コードが無効か期限切れ」を、それ以外は `describeError` を `#login-status` に出す。
- ログインリンク（`GET /login?code=`）のリダイレクト先は [`security.md#ブラウザ`](security.md#ブラウザ)。`LoginView.init` は `#/login/code/<code>` のハッシュを見つけると、`history.replaceState` でハッシュを `#/login` に戻してからコードを入力欄に入れるだけで、送信はしない。そのあと Login 画面を表示したときは `#login-status` に `loginLinkReady`（`ok`。「ログイン」を押す旨）を出し、送信ボタンにフォーカスする（それ以外はコードの入力欄にフォーカスする）。
- `#/login/invalid` のときは、表示時に同じ「無効か期限切れ」を出す。
- 状態行の文言は i18n のキーで覚えておき、`swing:langchange` で差し替える（`LoginView.render`）。
