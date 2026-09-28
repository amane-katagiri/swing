# リリース用の README を分ける

## 決めたこと

- リリースのアーカイブには、リポジトリの `README.md` ではなく、バイナリを使う人向けに別に書いた `docs/release/README.md` を `README.md` として入れる。
  - リポジトリの README は冒頭の画像（`docs/assets/`）と、`docs/architecture/` などへの相対リンクがアーカイブ内ではすべて切れる。`<picture>` の HTML もテキストで読むとノイズになる。
  - Docker Compose・Compose からの移行・ソースからのビルド・デモ環境の話は、バイナリを落とした人には要らない。
  - リポジトリの README の「バイナリで動かす」は `cp swing.example.toml swing.toml` と `./target/release/swing` を前提にしていて、アーカイブの中ではそのまま使えなかった。
- 本家の README にマーカーを埋めて削る生成方式は採らなかった。パスの書き方や読み手が違って結局書き分けが要り、機械的な加工は README の書き換えで壊れやすい。二重管理になるので、AGENTS.md のドキュメント表に更新の条件を書いた。
- リリース用の README ではリポジトリ内への相対リンクと画像を使わず、詳細はリポジトリの URL に送る。
- アーカイブに `swing.example.toml` と `web/fonts/LICENSE-PixelMplus.txt` を足した。前者は手順で使うため、後者はバイナリに埋め込んだフォントのライセンスを同梱するため。

## 変更

- `docs/release/README.md` を追加。
- `.github/workflows/release.yml` の Package で、上記を `README.md` として入れ、`swing.example.toml`・`LICENSE-PixelMplus.txt` を足した。
- `docs/architecture/release.md` の成果物の中身を更新。
- `AGENTS.md` のドキュメント表に `docs/release/README.md` を追加。
