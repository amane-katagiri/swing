# ビルドとリリース（`.github/workflows/release.yml`）

[`../architecture.md`](../architecture.md) の一部。配布物に入る `swing-tray` は [`tray.md`](tray.md)、コンテナイメージの土台は [`docker.md`](docker.md)。

`.github/workflows/release.yml` が配布用のバイナリとコンテナイメージを作る。`v*` タグの push と手動実行（`workflow_dispatch`）で動き、push や PR では動かない。

| target | ランナー | fmt / clippy / test | ビルド |
|---|---|---|---|
| `x86_64-unknown-linux-musl` | ubuntu-latest | する（ホストの gnu で） | `cargo zigbuild` |
| `aarch64-unknown-linux-musl` | ubuntu-latest | しない | `cargo zigbuild` |
| `aarch64-apple-darwin` | macos-latest | する | `cargo build` |
| `x86_64-apple-darwin` | macos-latest | しない | `cargo build`（クロス） |
| `x86_64-pc-windows-msvc` | windows-latest | する | `cargo build` |

- Rust は 1.97（Dockerfile と同じ）。Linux は glibc のバージョンに依存しないよう musl の静的バイナリにする。
- workspace には `swing` と `swing-tray`（`tray/`）がある。clippy / test は `--workspace` で回し、fmt は `cargo fmt --check`（ルートパッケージのみ）。Linux は `-p swing` だけをビルドし、Windows と macOS は `--workspace` でビルドする。
- Windows 向けのビルドでは、`build.rs`（`swing` と `swing-tray` の両方）が `winresource` でファイルアイコン（`swing` は `assets/swing.ico`、`swing-tray` は `tray/assets/swing-tray.ico`）とバージョン情報（`Cargo.toml` の `name`・`version`）を exe に埋め込む。他の target では何もしない。
- macOS の `swing`・`swing-tray` は `.app` バンドルではない素の実行ファイルで、ファイルアイコンは付かない。
- 成果物は `swing-<ref>-<target>.tar.gz`（Windows は `.zip`）で、中身は `swing`（`swing.exe`）・`LICENSE`・`README.md`。Windows と macOS には `swing-tray`（`swing-tray.exe`）も入れる。Kubo は同梱しない。
- タグの ref で動いたとき（タグの push と、タグを選んだ手動実行）は、タグ名と `Cargo.toml` の `version` が一致しないと失敗する（`v0.1.0` と `0.1.0`）。全 target が通ると `SHA256SUMS` を付けた**ドラフト**のリリースを作る。公開は GitHub 上で手動で行う。
- ブランチで手動実行したときはリリースを作らず、バイナリは Actions の artifact に残す。イメージは下記のとおり push する。
- `image` ジョブが `ghcr.io/<owner>/<repo>`（小文字）のコンテナイメージを `linux/amd64`・`linux/arm64` で作る。中身は `build` ジョブの `x86_64-unknown-linux-musl`・`aarch64-unknown-linux-musl` の `swing` を `docker/release.Dockerfile`（[`docker.md`](docker.md#dockerfile)）に入れたもの。QEMU は `RUN`（ユーザー作成）にだけ使う。
- イメージのタグは、タグの ref なら `v` を除いたバージョン（`0.1.0`）と `latest`（バージョンに `-` を含む `0.2.0-rc.1` などでは `latest` を付けない）。タグのイメージは、ドラフトのリリースを公開する前に push される。ブランチの ref ならブランチ名（`main` など）のタグだけを付けて push する。ブランチ名に `/` があるとイメージのタグに使えないので `image` ジョブが失敗する。`org.opencontainers.image.source` ラベルでパッケージをこのリポジトリに紐づけ、パッケージの公開範囲はリポジトリに合わせる。

サードパーティの action・ツール:

| 名前 | 役割 |
|---|---|
| `dtolnay/rust-toolchain` | 指定バージョンの Rust ツールチェインをインストール |
| `Swatinem/rust-cache` | Cargo のビルドキャッシュ |
| `taiki-e/install-action` | `cargo-zigbuild` をビルド済みバイナリからインストール |
| ziglang（PyPI、`pip3 install`） | `cargo zigbuild` が使う Zig 本体 |
| `docker/setup-qemu-action`・`docker/setup-buildx-action`・`docker/login-action`・`docker/build-push-action` | マルチアーキテクチャのイメージのビルドと ghcr.io への push |

サードパーティおよび `actions/*`（`actions/checkout`・`actions/upload-artifact`・`actions/download-artifact`）の action はフルコミット SHA に固定し、末尾に `# vN` コメントでタグ相当のバージョンを添えている。ziglang は pip の `==` でバージョンを固定する。Rust ツールチェインのバージョン自体はこれらのピン留めとは別で、ワークフローの `toolchain:` 入力（環境変数 `RUST_TOOLCHAIN`）で決まる。選定理由と信頼性の評価は [2026-09-25 の log](../log/2026-09-25-release-actions-rationale.md) を参照。

## ローカルでのクロスビルド

```bash
cargo xwin clippy --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings
cargo xwin build --release --workspace --target x86_64-pc-windows-msvc
```
