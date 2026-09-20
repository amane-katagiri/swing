# 同梱バナーをアニメーション GIF に差し替える

## 決めたこと

- 同梱の 88×31 バナーを、手描きの `yureko_banner_88x31.gif`（「JOIN SWING NETWORK!」、181 コマ・約 16.5 秒ループ・86KB）に差し替えた。`web/desktop-banner.png` は削除し、`web/desktop-banner.gif` を置いた。既定の Content-Type も `image/png` → `image/gif`。
- ルートを `/desktop-banner.png` から拡張子なしの `/desktop-banner` に変えた。バナーは設定（`[dashboard].desktop_banner`）で png/gif/jpeg/webp/svg のどれにでも差し替えられるので、パスに `.png` を残すと既定の状態ですら嘘になる。実際に配る形式は Content-Type が決める。
  - 参照元は `web/desktop-page.html` の `<img>` だけ。旧パスへの後方互換ルートは置いていない（このルート自体が前日に入ったばかりで、外に出ている差し替えページは無いという判断）。
- `alt` を新しい絵柄に合わせて書き直した（アニメーション GIF であることと文字の内容）。

## 分かっていて残した点

GIF のアニメーションは CSS から止められないので、リンク集ページの `prefers-reduced-motion` 対応（マーキーと NEW アイコンの `animation: none`）の網からこのバナーだけ外れる。静止画版をもう 1 枚同梱して `<picture>` + `media="(prefers-reduced-motion: reduce)"` で出し分ける手はあるが、差し替え側には静止画版が無いので設定の形が一段複雑になる。今回は入れていない。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（246 件）。既存のアセット配信テストは `/desktop-banner` + `image/gif` に更新。
- `docker/demo/demo.sh up --seed` のデモ環境で、`GET /desktop-banner` が `image/gif` で 88230 バイトを返し、旧 `/desktop-banner.png` が 404 になることを確認。
- ブラウザで Desktop 画面を開き、iframe 内の `.desk-banner` が `http://127.0.0.1:18082/desktop-banner` を 88×31 で読めていること、フッタで従来どおり描画されていることを見た。
