# README の画像の手直し

## ロックアップのロゴタイプの円がギザギザだった

`docs/assets/swing-lockup.svg` と `swing-lockup-dark.svg` で、ロゴタイプ側の `<g>`（縞の `rect` と円の両方を含む）に `shape-rendering="crispEdges"` を付けていたため、円（「I」の点など）のアンチエイリアスまで切られて縁がギザギザになっていた。ロゴ側の円には付いていないので綺麗だった。

ダッシュボードの `web/style.css` では `crispEdges` を縞のグループ `.swing-logotype-body` にだけ掛けていて、円は対象外。ロックアップも同じ形にそろえ、属性を縞の `rect` をまとめた内側の `<g>` へ移した。

## スクリーンショットにコントロール パネルを入れた

`docs/assets/dashboard-desktop.png` を、コントロール パネルのダイアログを開いた状態で撮り直した。ダイアログは既定だと画面中央に出てリンク集をほぼ隠すので、SWING Explorer ウィンドウの右下に重なる位置までドラッグし、リンク集の左半分（NEW バッジ・日付・サイト名）が見えるようにした。マーキーの文字が帯の中に入り、NEW バッジが点灯しているコマを選んだ。

撮影条件はデモ環境の `#/desktop`、1280×800・等倍・サイドナビ collapsed・日本語・ライトテーマ。README の alt と AGENTS.md の撮影条件の記述も合わせて更新した。
