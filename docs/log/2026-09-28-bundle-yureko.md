# 同梱マスコット `yureko` の追加と既定の表示

Desktop 画面の同梱マスコットに `yureko`（64×64・`scale: 2`）を足し、既定で出すマスコットを `yureko` だけにした。README の画面素材も撮り直した。

## 決めたこと

- **並びは `yureko` → `mochi` → `neko`。** `/mascots/index.json` の順がそのまま「マスコット」タブの一覧の順になるので、`BUNDLED` の先頭に置いた。
- **既定の表示は `["yureko"]`。** これまでは `packs` が無いことを「全部表示」としていたが、既定を全部以外にしたので、全部表示は `packs: null` として明示的に保存するようにした。`packs` が無い保存値は既定（`yureko` だけ）として読む。以前の形式で「全部表示」を保存していた人も `yureko` だけに戻るが、`localStorage` の表示設定なので移行の処置は入れなかった。
- **口パクは重ね絵の `mouth` にした。** `talk` を口の開閉のコマ送りにすると、文字送りが終わってから吹き出しが閉じるまでの間も口が動き続ける。`mouth` は文字送りの間だけ動くので、`talk` は口を閉じた 1 コマにした。
- **README のアニメーションにマスコットを持ち上げて離す操作を入れた。** つまむ・落ちるというマスコットの操作が見えるようにするため。持ち上げる先はダイアログを置く右下から離れた位置にした。

## 作ったもの

- `web/mascots/yureko/`（`manifest.json`・`sprite.png`）と、`src/dashboard/mascots.rs` の `BUNDLED` への追加。
- `web/desktop-mascot-settings.js`・`web/desktop-mascot.js` の既定の `packs` の変更と、`packs: null` の保存・読み込み。
- `docs/assets/capture-desktop.sh` に持ち上げて離す操作を足し、`dashboard-desktop.webp`・`dashboard-desktop.png` を撮り直した（約 12 秒で 1.9MB）。

## 確かめたこと

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace`。
- デモ環境で `/mascots/index.json` が `yureko`・`mochi`・`neko` の順に並ぶこと、設定の無いブラウザで `yureko` だけが出ることを撮影で確かめた。撮れた WebP のコマを並べて、持ち上げたコマがあること、マスコットがダイアログに被っていないことを見た。
