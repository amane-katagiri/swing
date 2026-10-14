# ダッシュボードの共通の UI 部品（`web/ui.js`, `web/util.js`, `web/pairing.js`, `web/boot.js`）

[`../web.md`](../web.md) の子ページ。複数の画面で使う部品と、全画面に共通の見た目・振る舞い。

- 絵文字のフィンガープリント（`ui.js::fingerprintBadge(ids, count)`）: `.swing-fingerprint`（`role="img"`）の中に、絵文字ごとの `<svg><use href="/emoji.svg#e-<コードポイント>"/></svg>` を並べる。`count` を渡すと先頭からその数だけ。`aria-label` は絵文字の文字そのもの（既定がテキスト表示のものも絵文字として読まれるよう、それぞれに異体字セレクタ U+FE0F を付ける）を空白でつないだもの、`title` は `count` に関わらず 4 つ全部を同じ形でつなぎ、「本人の証明ではない」旨を添えた文言（`fingerprintTitle`）。形式に合わない ID は捨て、1 つも残らなければ何も出さない（[`fingerprint.md`](../fingerprint.md)）。
- busy 表示: `setBusy(button, bool)` で `disabled`・`aria-busy`・`.is-busy` を切り替える。
- コピー（`util.js::copyWithFeedback`）: 押すと `data-copied` と `data-copy-failed` の両方を付け（成功なら `"true"`/`"false"`、失敗なら逆）、1.5 秒後に両方外す。文言はどのボタンも共通の `copy`（押した後は `copied`・`copyFailed`）で、対象の違いは `aria-label` 側で表す。
- サイドナビ: 下端のトグル（`#nav-toggle`）で畳むとアイコンだけの幅（`--swing-nav-collapsed-width`）になり、各リンクの `title` にラベルを入れる。状態は `localStorage["swing:nav:collapsed"]` に保存し、`<body data-nav="collapsed">` で表す。フッタ（`.swing-nav-footer`）はミラーセット名とバージョン（`publish.js::updateNavFooter`）と、上から <https://nostter.app/search?q=%23swingpublish> への「#swingpublish」と <https://yureko.samoyed.moe/> への「ゆれ子のSWING日記」のリンクを出す。リンクは `index.html` に固定で書いて `[data-home-link]` を付け（新しいタブで開く）、最初は隠しておいて `updateNavFooter` で出し、ログイン画面に移ったら `app.js::showRoute` で隠す。
- ロゴタイプ（`.swing-logotype`）: `docs/assets/swing-lockup.svg` の形を、表示する高さ 26px の整数 px の格子に描き直したもの（帯の太さは上から 1,1,1,1,2,2,3px、隙間 1px）。`viewBox` の 1 単位が 1 CSS px で、帯の端がすべて整数 px に乗る（`crispEdges`）。
- ペアリング（`pairing.js::createPairing`）: [`POST /api/setup/signer`](../http-api/config.md#post-apisetupsigner) が返した SVG を `data:` URI の `<img>` で出し、以後 [`GET /api/setup/signer`](../http-api/config.md#get-apisetupsigner) を 1.5 秒間隔でポーリングして状態を状態行に出す（`idle` は `waiting` と同じ表示）。`ready`/`failed` で QR を隠してポーリングを止める。`ready` で確認の署名が通ったときは「確認の署名が通った」とだけ伝え、通らなかったときは許可を促す警告にする。もう一度押すと新しいペアリングに置き換わり、前のポーリングは捨てる（`createLoadGuard`）。
- 読み込み時: `<body>` 直後の同期スクリプト `boot.js` が `data-nav` を先に付ける。表示言語が英語以外に決まるときは `<html lang>` と `<html data-i18n-pending>` も付けて `[data-i18n]` 要素を隠し、`app.js` が静的な訳を当てた直後にこの属性を外す（動かなかった場合は 1 秒後に英語のまま表示される）。
- 表（`.swing-table`。Sites のテーブル表示、Storage check の 2 つの表、Settings の設定表とリソース使用量の表）の折り返し:
  - 見出しと `.swing-nowrap` を付けたセル（サイズ・日時・保存状態・レプリカ数・短縮した npub と CID・リンクの 1 つずつ、リソース使用量の値）は折り返さない。
  - ほかのセルは単語の途中では折らない（`overflow-wrap: break-word`）。Storage check の Path 列だけは `.swing-break-anywhere`（最小幅 16ch で、どこでも折り返す）で縮む。
  - 収まらないときは表 1 つだけの外側（テーブル表示の `.swing-site-set`、Storage check とリソース使用量の表を包む `.swing-table-scroll`、`.swing-config-section`）が横スクロールする。見出しや「No problems found.」・合計行は流れず、ページ全体は横にはみ出さない。
- モバイル幅: 760px 以下ではナビを横並びにしてトグルとサイドナビのフッタを隠し、ページ最下部の `<footer id="page-footer">` に同じ内容（ミラーセット名とバージョンは `#page-footer-text`、その下に 2 つのリンク）を表示する（Desktop 画面を除く）。
