# release ワークフローのサードパーティ action・ツールの選定理由

[release ワークフロー](2026-09-24-release-workflow-and-kubo-signature.md)の回では「それぞれの action の最新メジャーを使う」としか書いておらず、選定理由と信頼性の評価を記録していなかった。今回それを事後的にまとめる。現状の構成（一覧・ピン留めの方針）は [architecture/release.md](../architecture/release.md) を参照。

## 選定理由

- **dtolnay/rust-toolchain**: David Tolnay（serde・syn・anyhow の作者）による action。メンテナンスが止まった `actions-rs/toolchain` の事実上の後継で、Rust の CI では広く使われている。中身は `rustup` を薄くラップしているだけで、監査しやすい。以前は `@master` を直接参照していたが、今回そのコミットが当時 `v1` タグと同一であることを確認した上で SHA 固定に変えた。ツールチェインのバージョン自体は `toolchain:` 入力（環境変数 `RUST_TOOLCHAIN`）で決めており、この変更による影響はない。
- **Swatinem/rust-cache**: Arpad Borsos による action。Cargo のビルドキャッシュ action の中で最も広く使われているもの。
- **taiki-e/install-action**: Taiki Endo（pin-project・cargo-hack の作者、tokio エコシステムのメンテナの一人）による action。マニフェストにチェックサム付きのビルド済みバイナリをダウンロードしてインストールするだけなので、`cargo install` でのビルドより速い。
- **cargo-zigbuild と ziglang**: cargo-zigbuild は messense（maturin/PyO3 のメンテナ）によるツールで、PyPI の ziglang（Zig 公式バイナリを pip 向けに配布したもの）と組み合わせて使う。x86_64・aarch64 の両方の musl 向けクロスビルドを 1 つの手順で揃えられる。musl を選んだ理由自体は [release ワークフローの回のログ](2026-09-24-release-workflow-and-kubo-signature.md)に既に書いてあるので、ここでは繰り返さない。

## リスク評価と決定

いずれも Rust エコシステムで長く使われ、利用実績も広い一方、組織ではなく個人アカウントが管理している。アカウント乗っ取りのリスクはゼロにはならないが、SHA（ziglang はバージョン）固定により、そのリスクを「意図した更新のタイミングでだけ影響を受ける」形に抑えた。引き換えに、更新のたびに手作業でピンを上げ直す手間が要る。

Docker のベースイメージ（`debian:trixie-slim` など）はこれとは逆に、意図してタグ参照のままにしてある。公式イメージは同じタグのままセキュリティパッチ込みで再ビルドされ続けるため、ピン留めするとむしろ更新が止まってしまう。Kubo（`ipfs/kubo`）だけはパッチバージョンまで固定している（[Kubo のバージョンをピン留めした理由](2026-09-16-pin-kubo-version.md)）。
