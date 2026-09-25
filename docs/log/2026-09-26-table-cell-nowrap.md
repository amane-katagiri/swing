# ダッシュボードの表でサイズ・日付が折れる不具合

Sites のテーブル表示と Storage check の表で、サイズ（`763 B`）や日時、見出し（`Size`・`Health`）が語の途中で折れていた。

## 原因

`.swing-table` のセルに一律で `overflow-wrap: anywhere` が付いていた。`anywhere` はセルの min-content を 1 文字幅まで縮めるので、表の自動レイアウトは長い Path 列に幅を回し、短い列は 1〜数文字幅まで押しつぶされて途中で折れる。

## 変更

- セルの既定を `overflow-wrap: break-word` にした。これは min-content を縮めないので、単語の途中で折れる前に表が広がる。
- 見出しと、新しい `.swing-nowrap` を付けたセル（サイズ・日時・保存状態・レプリカ数・短縮した npub と CID）は `white-space: nowrap`。
- Storage check の Path 列だけ `.swing-break-anywhere`（`overflow-wrap: anywhere` と `min-width: 16ch`）にして、この列が縮む役を受け持つ。最小幅が無いと、狭い画面で他の列が固定幅になった分だけ 1 文字幅まで細る。
- テーブル表示のリンク（「サイトを開く」「ゲートウェイで開く」）は 1 つずつ `.swing-nowrap` の `span` で包んだ。日本語は文字ごとに改行できるので、`break-word` でも列が 1 文字幅まで細るため。リンクとリンクの間では折り返せる。
- `#status-check-result` に `overflow-x: auto` を付けた。テーブル表示の `.swing-site-set`、Settings の `.swing-config-section` と同じく、表が収まらないときは外側が横スクロールし、ページ全体ははみ出さない。

## 検証

デモ環境で、英語・日本語 × 幅 1280 / 760 / 390 の Sites（カード・テーブル）、Storage check、Publish を表示した。どの組み合わせでも見出し・サイズ・日時は 1 行に収まり、ページの `scrollWidth` はビューポート幅と一致した（修正前は 390 で Storage check の表がページを 673px まではみ出させていた）。Settings の表の幅は変わらないことも確かめた。
