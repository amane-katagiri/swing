# ミラーに追加（`web/desktop-mirror-add.js`, `web/desktop-fingerprint.js`）

[`../desktop.md`](../desktop.md) の子ページ。ツールバーの「ミラー」（星のアイコン）で開くモーダルダイアログ（`#desk-dialog-mirror-add`、殻は `createDialog`）。Sites 画面の「ミラーに追加」と同じく、入力欄の鍵を空白・カンマで区切って（`ui.js::parseMirrorKeys`、上限 `MAX_MIRROR_KEYS` = 100 件）`POST /api/mirror/add` に送る。

- 開くたびに、送信中でなければ入力欄と状態表示を空にし、入力欄へフォーカスする。入力が空のあいだと送信中は OK を押せず、送信中は入力欄を読み取り専用にする。Enter は OK と同じ。
- 1 件以上追加できたら、ダイアログを閉じて `cache.mirror` を捨て、`/api/sites` を取り直してリンク集を描き直したあと、ステータスバーに件数（ミラー済みだった件数と、Follow Set を送ったリレーの成功数/総数を括弧で添える）を出す。
- 入力のたびに、Sites 画面と同じく鍵を `POST /api/mirror/preview` に送り（`ui.js::createKeyPreview`。[`../views.md#sites-画面`](../views.md#sites-画面)）、入力欄の下（`#desk-mirror-add-preview`、枠は無くダイアログの地のまま。高さ 120px を超えたらその中でスクロール）に鍵ごとの行を出す。読める鍵はドット絵のフィンガープリント（下記）と縮めた npub（`title` に全体）、読めない鍵は「鍵として読めません」と縮めた入力を暗い赤で出す。入力が空のときなど鍵の行が無いとき（開いたときも）は「ここにフィンガープリントとnpubが表示されます。」の 1 行を出す。行の高さはどれも 16px の絵文字に揃える。
- 何も追加されなかったとき（すべてミラー済み）・件数が上限を超えたとき・API が失敗したときは、ダイアログを開いたまま入力欄の下（`#desk-mirror-add-status`）に文面を出す。
- 送信中にキャンセルで閉じても要求は止めず、追加できればリンク集とステータスバーは同じように更新する。

## フィンガープリントのドット絵

`desktop-fingerprint.js::deskFingerprint(ids, title)` は、Sites 画面などの `fingerprintBadge`（[`../web/ui.md`](../web/ui.md)）の代わりに、Desktop 画面の見た目に合わせて絵文字を 16×16 のドット絵にして出す。絵文字の表とスプライトは同じもの（[`../fingerprint.md`](../fingerprint.md)）で、別の画像は持たない。

- 初めて使うときに `/emoji.svg` を取って `DOMParser` で読み（取れたらページの寿命のあいだ使い回し、失敗したら次に使うときに取り直す）、絵文字ごとに `<symbol>` の中身を 36×36 の SVG の `data:` URL にして `Image` で読む。16×16 の canvas に縮めて描き、不透明度を 128 未満は 0、それ以上は 255 の 2 値にしてふちのぼかしを消し、PNG の `data:` URL にする。結果は絵文字ごとにページの寿命のあいだ覚える（失敗したものは覚えない）。
- `.desk-fingerprint`（`role="img"`、`aria-label` と `title` は `fingerprintBadge` と同じ作り）の中に 16×16 の `<img>` を 4 つ並べ、`image-rendering: pixelated` で拡大する（Desktop 画面の整数倍の拡大は [`../desktop.md#表示倍率`](../desktop.md#表示倍率)）。
- スプライトを取れない・canvas を読めないときは、その絵文字だけ `<svg><use href="/emoji.svg#e-…"/></svg>` のベクタで出す。
- `data:` の画像は CSP の `img-src 'self' data:` の範囲（[`../security.md`](../security.md)）。
