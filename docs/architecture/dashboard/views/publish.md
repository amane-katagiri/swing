# Publish 画面（`web/publish.js`）

[`../views.md`](../views.md) の子ページ。使う API は [`../http-api/publish.md`](../http-api/publish.md)。

## 自分の情報と署名アプリ

- 自分の情報（`.swing-identity`）: npub・hex・ミラーセット・relays と、署名の方式（設定ファイルの秘密鍵／署名アプリ）を `overview.signer` から出す（`publish.js::renderSigner`）。`last_failure` があれば警告を出す。署名アプリのときは画面表示のたびと publish 完了時に `GET /api/overview` を読み直してこの行を更新する。
- 署名アプリのときは「つなぎ直す」ボタンから relay 欄と QR（`pairing.js`）とキャンセルボタンを出す。ペアリングが `ready` になると「つなぎ直して再起動」ボタン（`#pub-signer-save`）が有効になり、押すと [`POST /api/signer/reconnect`](../http-api/config.md#post-apisignerreconnect) を呼び、`waitForNewInstance` で `instance` が押す前の `cache.overview.instance` から変わるのを待ってページを読み直す（[`../web.md#止まっている間の呼び出し`](../web.md#止まっている間の呼び出し)）。

## フォーム

- 常にフォルダアップロード（`<input type="file" webkitdirectory multiple>`）。ファイル数・合計サイズを表示し、`max_upload` を超えれば送信ボタンを無効にする。各ファイルの送信名は `webkitRelativePath` から選んだフォルダ名を除いたもの。
- NIP-05 の下に、同じ形のセレクトを「ドットファイルの確認」（`check_dotfiles`）・「サイズの確認」（`check_size`）・「同じ内容の確認」（`check_unchanged`）の順に並べる。どれも先頭の選択肢は値が空（パートを送らず設定の既定値に任せる）で、残りは `off`・`warn`・`require`（NIP-05 も同じ）。空でなければ同名のパートで送る（`publish.js::MODE_FIELDS`）。
- 最後に使ったフォーム内容は `swing:publish:last` に保存し、次に開いたときに入れる。

## 増えたファイルの確認

- 送信する前に [`GET /api/publish/previous-files`](../http-api/publish.md#get-apipublishprevious-filessited) を呼ぶ（その間 `#publish-status` に `loading`）。API がエラー（503 など）ならアップロードせずにエラー文を出す。
- 選んだファイルの送信名のうち `files` に無いものを増えたファイルとする（`publish.js::findNewFiles`。`status` が `listed` 以外なら全ファイル）。無ければそのままアップロードする。
- あれば `#publish-confirm`（フォームの直後の `swing-panel`）を出してフォームを無効にし、`#publish-status` に `warn` で確かめるよう案内する。パネルには、比べた相手（CID と作成日時／前の版が無い／読めなかった理由）、取り消せない旨、増えたファイルの全件（`.swing-file-list`。並べ方は CLI と同じで、フォルダは入れ子の `ul`。`publish.js::groupByFolder`）、「このファイルを含めて公開」「キャンセル」を置く。
- 「このファイルを含めて公開」でアップロードし、「キャンセル」では何も送らずに `#publish-status` に公開を中止した旨を `ok` で出す。

## 送信と結果

- 送信は `XMLHttpRequest` で、進捗を `.swing-progress`/`.swing-progress-bar`（`data-state`）に出す。署名アプリのときは処理中の表示に、承認を求められたら承認するよう案内を出す。
- 成功したら進捗バーを隠し、`#publish-status` に結果（relay N つのうち M つが受け付けたか。全部なら `ok`、一部だけなら `warn`）を出す。結果のパネルには応答の各項目（サイトの確認の行は `publish.js::addCheckRows`、署名の行は署名アプリのときだけ）とゲートウェイのリンクを並べる。
- 応答の `published` が `false`（`check_unchanged` が `require` で同じ内容だった）なら、`#publish-status` に同じ内容なので publish しなかった旨を `ok` で出し、結果のパネルから署名・作成日時・MFS パス・relay の行を省く。`swing:published` イベントは投げない。

## エラーの表示

- 413: 上限を超えた旨。
- 422 で本文に `checks` があるもの（ドットファイル・サイズの `require`）: `require` で引っかかった項目ごとに、画面の選択と設定のキーで直し方を案内する文を出し（どれにも当たらなければ API のエラー文）、結果のパネルに NIP-05 とドットファイル・サイズの行だけを出す。
- 422 で本文に `nip05` があるもの: NIP-05 の確認に失敗した旨を出し、結果のパネルに NIP-05 の行を出す。
- 409: 別の publish を実行中である旨。
- それ以外: API のエラー文。

## My sites

[`/api/publish/sites`](../http-api/publish.md#get-apipublishsites) を、画面を開いたときにキャッシュが無ければ取得し、publish が成功したときと「再読み込み」で取り直す。一覧の「Use」ボタンは `site`・`url`・`title`（`message` を除く）をフォームに入れるだけ。
