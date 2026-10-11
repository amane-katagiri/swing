# ダッシュボードの静的ファイルを gzip で配る

[ETag での確かめ直し](2026-10-11-static-asset-etag.md) で 2 回目からの読み込みは軽くなったが、初回はテキスト系のファイル（合計約 935KB、うち `/emoji.svg` が約 430KB）をそのまま送っていた。

## 決めたこと

- 埋め込んだテキスト系のファイル（`text/*` と `image/svg+xml`）を最初のリクエストで 1 回だけ gzip して持ち、`Accept-Encoding` が gzip を受け付けるときに返す。リクエストのたびに圧縮する `tower-http` の `compression-gzip` は使わなかった。中身は起動後に変わらないので毎回圧縮する CPU が無駄で、`async-compression` まで依存に増える。ビルド時に圧縮する（`build.rs`）方法は、ビルドを複雑にする割に実行時の差が最初の 1 回だけなので取らなかった。
- 圧縮は pure Rust の `flate2`（既定の `miniz_oxide`）にした。Windows・macOS のクロスビルドで C コンパイラを要らない。
- 圧縮版と元の版で `ETag` を分ける。同じ値にすると、`Accept-Encoding` の違うクライアントやプロキシのキャッシュで 2 つの版が混ざるため。`Vary: accept-encoding` は圧縮版を持つファイルの応答すべてに付け、`Content-Encoding` は `200` のときだけ付ける。
- 元より小さくならないファイルは圧縮版を持たない。PNG・GIF・woff2 はもともと圧縮されているので対象外にした。`AppState` に持つ差し替えできるファイル（Desktop 画面のリンク集ページ・マスコット）は小さいので圧縮しない。

## 検証

- `cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace` が通る。
- テストで、`/emoji.svg`・`/`・`/app.js`・`/style.css` が `gzip;q=0.8` を含む `Accept-Encoding` で圧縮版になり、展開すると元と一致すること、圧縮版の `ETag` で `304` になること、`gzip;q=0` では元の版になること、PNG・woff2 は圧縮せず `Vary` も付かないことを確かめた。
- デモ環境で `curl` から、`/emoji.svg` が `Accept-Encoding: gzip` で約 159KB（元は約 436KB）になり、圧縮版の `ETag` を付けると `304` で本体が空になること、`/api/` は `no-store` のままであることを確かめた。
