# Publish の進捗バーの余白と Storage check の表スクロール

見た目の不具合を 2 件直した。

## 1. publish 失敗後の進捗バー

publish が失敗すると赤いエラー状態の `.swing-progress` がアップロード情報の行の下に残り、すぐ下の「Site (d tag)」ラベルと余白なしでくっついていた。

### 原因

`.swing-progress` は `margin-top` しか持たず `margin-bottom` が無かった。バーが `hidden` のとき（display:none）は margin collapsing に参加しないので問題は出ないが、エラーで表示されたままになると、直後の `.swing-field`（margin-top 無し）との間に隙間が無くなる。

### 意図の確認

`data-state="uploading|processing|error"` は `docs/architecture/dashboard/web.md` に既に載っており、同じ節に「publish が成功したら進捗バーを隠し」とだけ書かれている。成功時のみ隠す、失敗時は残すという現状の挙動は元から意図された設計と判断し、バーを消す変更はしなかった。`#publish-status` のエラー文言（原因の説明）と、進捗バーの赤（アップロード自体がどこまで進んだかの視覚的な合図）は役割が違うため、重複としては扱わなかった。

### 変更

- `web/style.css`: `.swing-progress` に `margin-bottom: var(--swing-space-3)` を追加。ほかのフィールド間の余白（`.swing-field` の `margin-bottom`）と揃えた。

### 検証

デモ環境で、URL に `ftp://example.com/` を入れて publish を失敗させ、赤いバーと「Site (d tag)」の間に余白ができることを確認した。続けて正しい URL で publish すると `hideProgress()` でバーが隠れ、次に別サイトを公開しようとして再び失敗させても、バーは新しいエラー状態で正しく表示された（前回の状態を引きずらない）。英語・日本語、1280px・390px で確認。

## 2. ストレージ確認の結果の横スクロール

直前のコミット（`docs/log/2026-09-26-table-cell-nowrap.md`）で `#status-check-result` に `overflow-x: auto` を付けたため、狭い幅で表を右にスクロールすると見出し（h3）や「No problems found.」、合計行まで一緒に横へ流れていた。

### 変更

- `web/style.css`: `#status-check-result` から `overflow-x: auto` を外した。新しく `.swing-table-scroll`（`width: 100%; overflow-x: auto; margin-bottom: var(--swing-space-3)`）を追加し、表 1 つだけを包むラッパーにした（`#status-check-result .swing-table { margin-bottom: ... }` は不要になったので削除し、余白はラッパー側に移した）。テーブル表示の `.swing-site-set` と同じく、スクロールする範囲を表だけに絞る考え方。
- `web/sites.js`: `renderStatusCheck` の 2 つの表（版ごとの判定の表、サイトごとの実容量の表）をそれぞれ `el('div', { class: 'swing-table-scroll' }, table)` で包んだ。
- `docs/architecture/dashboard/web.md`: 安定 class 一覧に `swing-table-scroll` を追加し、表の折り返しの説明を「表 1 つだけの外側」がスクロールする、に書き直した。

### 検証

デモ環境で Storage check を実行し、760px・390px で表だけが横スクロールし、見出し・「問題は見つかりませんでした。」・合計行は動かないことを確認した（表のラッパーの `scrollLeft` を変えても他の行は動かない）。ページの `scrollWidth` は 760px・390px のどちらでもビューポート幅と一致し、はみ出さない。1280px では表が収まるため見た目は変更前と同じ。

## 確認環境

`docker/demo/demo.sh up --seed` のデモ環境、`agent-browser` で操作。両方とも英語・日本語 × 1280px を基本に、1 は 390px、2 は 760px・390px を追加で確認した。
