# ダッシュボードの静的ファイルを ETag で確かめ直させる

絵文字のフィンガープリントのスプライト `/emoji.svg`（約 440KB）を足してから、平文の http で LAN から開くと読み込みが重くなった。静的ファイルの応答にはキャッシュの指定も検証子も無く、ブラウザが毎回本体を取り直すことがあった。

## 決めたこと

- `/custom.css` 以外の静的ファイル（`STATIC_ASSETS`・Desktop 画面の差し替えできるファイル・マスコットのパック）に、本体の SHA-256 の先頭 16 バイトから作った強い `ETag` と `Cache-Control: no-cache` を付け、`If-None-Match` が一致したら `304` を返す。
- `max-age` や `immutable` での長期のキャッシュは取らなかった。モジュールは `./x.js` のように版を含まない URL で import しているので、更新後に古い JS と新しい JS が混ざる。URL に版を入れるにはビルド工程か import の書き換えが要り、そこまでする利点が無い。
- `ETag` は版番号ではなく中身から作る。開発中は版を上げずに中身が変わるのと、利用者が差し替えたファイル（`desktop_page` など）やマスコットのパックも同じ仕組みで扱えるため。`STATIC_ASSETS` の分は最初のリクエストで一度だけ計算し、`AppState` に持つものは小さい（スプライトでも 1MB まで）のでリクエストのたびに計算する。
- 圧縮（`Accept-Encoding` に応じた gzip など）は今回は入れず、初回の読み込みの重さとして todo に残した。

## 検証

- `cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace` が通る。
- `/emoji.svg`・`/`・`/desktop-page.html`・`/desktop-banner`・`/mascots/index.json`・`/mascots/yureko/sprite.png` で、`ETag` と `no-cache` が付くこと、同じタグ（一覧の中の `W/` 付きを含む）で `304` と空の本体が返ること、違うタグなら `200` になることをテストした。
