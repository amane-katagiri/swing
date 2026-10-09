# Publish 画面（`web/publish.js`）

[`../views.md`](../views.md) の子ページ。使う API は [`../http-api/publish.md`](../http-api/publish.md)。

## 自分の情報と署名アプリ

- 自分の情報（`.swing-identity`）: npub・hex・ミラーセット・relays と、署名の方式（設定ファイルの秘密鍵／署名アプリ）を `overview.signer` から出す（`publish.js::renderSigner`）。`last_failure` があれば警告を出す。署名アプリのときは画面表示のたびと publish 完了時に `GET /api/overview` を読み直してこの行を更新する。
- 署名アプリのときは「つなぎ直す」ボタンから relay 欄と QR（`pairing.js`）とキャンセルボタンを出す。ペアリングが `ready` になると「つなぎ直して再起動」ボタン（`#pub-signer-save`）が有効になり、押すと [`POST /api/signer/reconnect`](../http-api/config.md#post-apisignerreconnect) を呼び、`waitForNewInstance` で `instance` が押す前の `cache.overview.instance` から変わるのを待ってページを読み直す（[`../web.md#止まっている間の呼び出し`](../web.md#止まっている間の呼び出し)）。

## フォーム

- 常にフォルダアップロード（`<input type="file" webkitdirectory multiple>`）。ファイル数・合計サイズを表示し、`max_upload` を超えれば送信ボタンを無効にする。各ファイルの送信名は `webkitRelativePath` から選んだフォルダ名を除いたもの。
- NIP-05 の下に、同じ形のセレクトを「ドットファイルの確認」（`check_dotfiles`）・「サイズの確認」（`check_size`）・「動作性の確認」（`check_links`）・「前回のデプロイとの差分の確認」（`check_unchanged`）の順に並べる。どれも先頭の選択肢は値が空（パートを送らず設定の既定値に任せる）で、残りは `off`・`warn`・`require`（NIP-05 も同じ）。空でなければ同名のパートで送る（`publish.js::MODE_FIELDS`）。
- メッセージの下に「サイトイベントに加えて更新情報をノート（kind 1）として投稿する」チェックボックス（`note`。`.swing-field` の中の `.swing-checkbox` と、URL が要る旨の `.swing-hint`）を置く。URL 欄が空白だけの間は無効にし（`publish.js::refreshSubmitState`）、送信するのは URL があってチェックされているときだけ（パート `note=true`）。
- 最後に使ったフォーム内容（チェックボックスの状態を含む）は `swing:publish:last` に保存し、次に開いたときに入れる。

## 公開前の確認

- 送信する前に、[`GET /api/publish/previous-files`](../http-api/publish.md#get-apipublishprevious-filessited) と [`POST /api/publish/check`](../http-api/publish.md#post-apipublishcheck) を同時に呼ぶ（その間 `#publish-status` に `loading`）。どちらかがエラー（503 など）ならアップロードせずにエラー文を出す。
- check には `site`・`url`・`check_dotfiles`・`check_size`・`check_links`（フォームで選んだものだけ）と、選んだ全ファイルの送信名と `File.size` の一覧（`files`）、そのうち拡張子が `.html`・`.htm`・`.css`・`.js`・`.mjs`（大小文字無視）で 4 MiB 以下のファイルの中身（`file`）を送る（`publish.js::buildCheckFormData`。拡張子と上限はサーバー側の[読むファイル](../../publish/links.md#読むファイル)と同じ）。画像などの中身は送らない。
- 選んだファイルの送信名のうち previous-files の `files` に無いものを増えたファイルとする（`publish.js::findNewFiles`。`status` が `listed` 以外なら全ファイル）。
- 確認で何か見つかった（`dotfiles` が `found`・`size` が `over`・`links` が `found` のどれか）か、増えたファイルがあれば、`#publish-confirm`（フォームの直後の `swing-panel`）を出してフォームを無効にする。どちらも無ければそのままアップロードする。
- パネルの上側（`#publish-confirm-checks`）は確認で何か見つかったときだけ出し、見出し「公開前の確認」と、結果のパネルと同じ行（`publish.js::addCheckRows`。リンクの行には修正ガイドのリンクも出る）を `.swing-result-grid` で並べる。下側（`#publish-confirm-new`）は増えたファイルがあるときだけ出し、比べた相手（CID と作成日時／前の版が無い／読めなかった理由）、取り消せない旨、増えたファイルの全件（`.swing-file-list`。並べ方は CLI と同じで、フォルダは入れ子の `ul`。`publish.js::groupByFolder`）を置く。両方あれば 1 つのパネルに上下で並べる。最後にボタン（`.swing-css-actions`）を置く。
- 確認の結果が `require` で止まるもの（`publish.js::describeCheckBlock` が理由を返す）なら、公開のボタンを隠し、`#publish-status` に `error` で公開できない旨と、結果の 422 と同じ直し方の案内を出す。「キャンセル」だけを選べる。
- そうでなければ `#publish-status` に `warn` で確かめるよう案内し（確認で見つかったものがあればその旨、無ければ増えたファイルの旨）、公開のボタン（確認で見つかったものがあれば「このまま公開」、増えたファイルだけなら「このファイルを含めて公開」）と「キャンセル」を出す。
- 公開のボタンでアップロードし、「キャンセル」では何も送らずに `#publish-status` に公開を中止した旨を `ok` で出す。
- 事前の確認は案内で、本番の判定はアップロードした後のサーバー側（[`POST /api/publish/upload`](../http-api/publish.md#post-apipublishupload)）で改めて行う。

## 送信と結果

- 送信は `XMLHttpRequest` で、進捗を `.swing-progress`/`.swing-progress-bar`（`data-state`）に出す。署名アプリのときは処理中の表示に、承認を求められたら承認するよう案内を出す。
- 成功したら進捗バーを隠し、`#publish-status` に結果（relay N つのうち M つが受け付けたか。全部なら `ok`、一部だけなら `warn`）を出す。結果のパネルには応答の各項目（サイトの確認の行は `publish.js::addCheckRows`、署名の行は署名アプリのときだけ、投稿の行は応答の `note` があるときだけで、`error` があれば `warn` の `swing-status` の後に relay ごとの成否）とゲートウェイのリンクを並べる。
- リンクの行（`publish.js::describeLinks`）は、`off` なら確認していない旨、`ok` なら無い旨を 1 行で出す。`found` なら、件数（`blocking` が 1 以上ならそのうち崩れる件数も）の 1 行の後に、種別ごとに「種別の説明（件数）」の 1 行と、その下に参照を 1 件 1 項目の `ul.swing-file-list`（増えたファイルの一覧と同じ見た目。等幅で、長い参照は折り返す）で並べ（並びはサーバーが返した順で、増えたファイルの一覧と同じく最上位のファイルが先、続けてフォルダごと。サーバーの比べ方は UTF-8 のバイト順で、画面の増えたファイルの一覧（`groupByFolder` の UTF-16 のコード単位順）とは、U+E000 以降と U+10000 以降の文字が混ざったときだけ食い違うことがある。各項目は `<file>: <reference>`。`files` が 2 以上なら `<file>` の後に「ほか N ファイル」を付ける。並べなかった残りは最後の項目に「ほか N 件」と付ける）、最後に `guide` へのリンク（「修正ガイド」。`maybeLink` で `target="_blank" rel="noopener noreferrer"`）を 1 行だけ出す。行は `dd` の中の `div` で余白を足さず、リストの上下の余白は `.swing-file-list` のもの（上 `--swing-space-2`・下 `--swing-space-3`）。4 MiB を超えて読まなかったファイルがあれば 1 行目にその数を足す。`_redirects` があるときの `broken` は、補えるかもしれない旨を説明に付ける。
- 応答の `published` が `false`（`check_unchanged` が `require` で同じ内容だった）なら、`#publish-status` に同じ内容なので publish しなかった旨を `ok` で出し、結果のパネルから署名・作成日時・MFS パス・relay の行を省く。`swing:published` イベントは投げない。

## エラーの表示

- 413: 上限を超えた旨。
- 422 で本文に `checks` があるもの（ドットファイル・サイズ・リンクの `require`）: `require` で引っかかった項目ごとに、画面の選択と設定のキーで直し方を案内する文を出し（どれにも当たらなければ API のエラー文。リンクは `blocking` が 1 以上のとき）、結果のパネルに NIP-05 とドットファイル・サイズ・リンクの行だけを出す。
- 422 で本文に `nip05` があるもの: NIP-05 の確認に失敗した旨を出し、結果のパネルに NIP-05 の行を出す。
- 409: 別の publish を実行中である旨。
- それ以外: API のエラー文。

## My sites

[`/api/publish/sites`](../http-api/publish.md#get-apipublishsites) を、画面を開いたときにキャッシュが無ければ取得し、publish が成功したときと「再読み込み」で取り直す。一覧の「Use」ボタンは `site`・`url`・`title`（`message` を除く）をフォームに入れるだけ。
