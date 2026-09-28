# Desktop 画面のタスクバーの音量アイコンをミュート表示にする

## 決めたこと

- Desktop 画面は音を鳴らさないので、タスクバーの通知領域の音量アイコン（`icon-desk-tray-volume`）をミュートの見た目にする。アイコンは飾りのままで、押しても何も起きない。

## 作ったもの

- `web/desktop-icons.svg`: `icon-desk-tray-volume` のスピーカー本体（x ≤ 7）はそのまま残し、右側の音の波 3 本を消して、x 9〜15・y 5〜10 に 2px 幅の赤（`#ff0000`）の × を描いた。
- `docs/assets/dashboard-desktop.webp`・`dashboard-desktop.png`: `docs/assets/capture-desktop.sh` で撮り直した。

## 検証

- デモ環境（`docker/demo/demo.sh up --seed`）の `#/desktop` でタスクバーを表示し、音量アイコンがスピーカー＋赤い × になっていることを確かめた。
- 撮り直した WebP のコマを並べ、マスコットを持ち上げたコマがあること・マスコットがダイアログに被らないこと・ダイアログが最後に所定の位置で止まっていることを確かめた。
