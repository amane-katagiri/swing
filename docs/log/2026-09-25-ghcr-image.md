# リリース時に ghcr.io へコンテナイメージを push する

release ワークフローに `image` ジョブを足し、`v*` タグの push で `ghcr.io/<owner>/<repo>` にマルチアーキテクチャ（`linux/amd64`・`linux/arm64`）のイメージを push するようにした。現状の構成は [architecture/release.md](../architecture/release.md)。

## 決めたこと

- **イメージの中で Rust をビルドしない。** ルートの `Dockerfile` を `build-push-action` にそのまま渡すと、arm64 のビルドが QEMU 上の `cargo build` になり、1 回のリリースで 1 時間近くかかる。代わりに `build` ジョブが作った musl の静的バイナリを runtime だけの `docker/release.Dockerfile` に入れる。リリースの tar.gz とイメージの中身が同じバイナリになる利点もある。
- **ルートの `Dockerfile` は残す。** `compose.yaml` の `build: .` とデモ環境は、リリースを待たずに手元のソースから作れる必要がある。runtime の内容（ユーザー・`/data`・`ENTRYPOINT`・`CMD`）は 2 つの Dockerfile で揃える。
- **ベースは musl でも `debian:trixie-slim`。** 静的バイナリなので `scratch` や distroless でも動くが、`groupadd`/`useradd` と `docker compose exec` でのシェル操作をルートの Dockerfile と同じにしておくため。TLS は `rustls` の webpki roots を使うので `ca-certificates` は要らない。
- **タグ。** バージョン（`v` を除く）と `latest`。`-` を含むプレリリースには `latest` を付けない。`docker/metadata-action` は使わず、シェルで組み立てる（action を 1 つ減らすため）。
- **手動実行ではブランチ名のタグで push する。** 手動実行（`workflow_dispatch`）でもリリースを作らずに ghcr への push とパッケージの紐づけまで試せるように、実行したブランチ名をタグにして push する。`latest` やバージョンのタグとは衝突しない。ただしバージョンと同じ形のブランチ名（`0.1.0` など）にはしない。リリースはドラフトで作って手動で公開する運用だが、イメージはタグの push 時点で公開される。`release` ジョブを待たないのは、イメージの push とリリースの作成が互いに依存しないため。
- **action の選定。** `docker/*` の 4 つは Docker 社の公式 org が管理しているもので、GitHub Actions でイメージを作る際の標準。既存の action と同じくフルコミット SHA に固定した（各メジャーの最新リリースのコミット）。
- **認証は `GITHUB_TOKEN`。** ジョブに `packages: write` だけを足す。`org.opencontainers.image.source` ラベルで ghcr のパッケージをリポジトリに紐づけ、公開範囲とアクセス権をリポジトリから引き継がせる。

## 検証

- `docker/release.Dockerfile` を amd64 で手元でビルドし、`<TARGETARCH>/swing` が `/usr/local/bin/swing` に入ること、uid 1000 の `swing` ユーザーで `/data` から `swing up` が起動することを確かめた（中身はダミーのスクリプト）。
- テスト用のブランチで手動実行し、全ジョブが通って `ghcr.io/<owner>/<repo>:<ブランチ名>` が push されることを確かめた。パッケージはリポジトリに紐づき、公開範囲はリポジトリと同じ private になった。
- push したイメージは `linux/amd64` と `linux/arm64` の 2 つを持つ（ほかに `unknown/unknown` の attestation のマニフェストが 2 つ付く。buildx が既定で付ける provenance）。amd64 では `swing --version` が動き、uid 1000 の `swing` ユーザーで `/data` から動くことを確かめた。arm64 はエミュレーションの無い x86_64 の環境で確かめたので起動はできず、`/usr/local/bin/swing` が aarch64 の静的バイナリであることだけを確かめた。
- `v*` タグでの実行（バージョンと `latest` のタグ）はまだ試していない。
