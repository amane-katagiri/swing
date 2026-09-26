# README の画面素材の撮り方

どちらも同じスクリプトで、デモ環境（`docker/demo/demo.sh up --seed`）の `#/desktop` を、1280×800・等倍・サイドナビ collapsed・日本語で撮る。画面の見た目を変えたら撮り直す。

```sh
docker/demo/demo.sh up --seed
docs/assets/capture-desktop.sh            # docs/assets/dashboard-desktop.webp と .png を上書きする
docs/assets/capture-desktop.sh out.webp   # out.webp と out.png に出す
```

`agent-browser` と、`libwebp_anim` 入りの `ffmpeg` が要る。

## `dashboard-desktop.webp`（README の本体）

- 流れ: 3 秒ほどそのまま（マーキー・NEW バッジ・マスコット）→ コントロール パネルのアイコンをダブルクリック → ダイアログを SWING Explorer ウィンドウの右下に重なる位置（ダイアログの左上がビューポートの (804, 196)）までドラッグ → 5 秒ほどそのまま。
- マウスカーソルは映さない。
- マスコットがダイアログの下を歩かないよう、撮影するブラウザだけ `Math.random` をシード固定で 0〜0.45 の値を返すものに差し替えて、出現位置も歩く先もデスクトップの左側に寄せる。アプリのコードには手を入れない。
- 録画（webm）の先頭 1 秒を落とし、10fps・品質 80 のアニメーション WebP（無限ループ）にする。約 10 秒で 1.3MB 前後。可逆にすると録画のノイズで 4MB を超えるので使わない。

撮れたらコマを並べて、マスコットがダイアログに被っていないか、ダイアログが最後に上の位置で止まっているかを確かめる。

## `dashboard-desktop.png`（動きを減らす設定のときの静止画）

README では `prefers-reduced-motion: reduce` のときにこちらを出す。録画を止めた直後の、WebP の最後のコマと同じ配置で撮る。マーキーの文字がダイアログに隠れず全部見えていて、NEW バッジが点灯しているコマになるまで撮り直す。
