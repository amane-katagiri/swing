# release ワークフローと kubo の署名確認

配布前の確認のうち、macOS 実機が無くてもできるものを片付けた。macOS のコンパイル確認は GitHub Actions に任せることにし、そのための release ワークフローを作った。

## macOS のコンパイル確認を WSL でやらない理由

`rustup` の `aarch64-apple-darwin` ターゲットは入っているので `cargo check --target aarch64-apple-darwin` を試した。Rust のコードまで届かず、`ring` のビルドスクリプトで止まる。

- 既定の `cc`（gcc）は `-arch` や `-mmacosx-version-min` を知らない。
- `CC_aarch64_apple_darwin=clang` にすると `--target=arm64-apple-macosx` でコンパイルはするが、ヘッダを Linux の `/usr/include` から拾って `bits/libc-header-start.h` が無いと言われる。macOS SDK のヘッダが要る。

osxcross で SDK を入れる手はあるが、SDK の取り出しに macOS か Xcode の .xip が要る。ビルドはどうせ CI でやるので、確認もそちらに寄せた。

## release ワークフロー

`.github/workflows/release.yml`。構成の現状は [architecture.md のビルドとリリース](../architecture.md#ビルドとリリース)。

- **トリガはタグと手動実行だけ**: リポジトリは private なので、macOS ランナーは Linux の何倍も無料枠を減らす。push / PR ごとの CI はまだ作っていない（Linux だけの軽いものなら安い。要るなら別ワークフローで足す）。
- **native ランナーで 3 OS**: macOS と Windows は native で `fmt` / `clippy --all-targets -D warnings` / `test` まで通す。macOS のコンパイル確認をここで兼ねる。`x86_64-apple-darwin` は macos-latest（arm64）からのクロスでビルドだけする。
- **Linux は musl**: ubuntu-latest の glibc でビルドすると、それより古いディストリで動かない。`install.sh` で配ることを考え、静的リンクの musl にした。クロスの linker と C コンパイラの用意を `cargo-zigbuild` にまとめ、x86_64 と aarch64 を同じ手順でビルドする。arm64 の Linux ランナーは private リポジトリで使えるか確かめていないので使っていない。musl の malloc は glibc より遅いが、SWING の負荷は Kubo 側にあるので問題にしていない。
- **ドラフトで止める**: タグで全 target が通ったら `SHA256SUMS` 付きのドラフトリリースを作り、公開は手で行う。タグ名と `Cargo.toml` の version が食い違ったら失敗させる。
- **Kubo は同梱しない**: 同梱するかどうかは配布経路ごとに違う（Homebrew は依存で解決し、winget だけ `ipfs.exe` を同梱する）。なので release の成果物には入れず、インストーラ側で扱う。
- **入れていないもの**: todo にある macOS の ad-hoc 署名（`rcodesign`）と artifact attestation は入れていない。artifact attestation は private リポジトリだと GitHub Enterprise Cloud が要るので、リポジトリを public にするまで使えない。
- アクションは checkout v7 / upload-artifact v7 / download-artifact v8 / rust-cache v2 / taiki-e/install-action v2（それぞれの最新メジャー）を使う。

## kubo darwin-arm64 バイナリの署名

`https://dist.ipfs.tech/kubo/v0.43.1/kubo_v0.43.1_darwin-arm64.tar.gz`（同じ場所の `.sha512` と一致）の `ipfs` を、Linux 版の `rcodesign` 0.29.0 の `print-signature-info` で調べた。

- `Developer ID Application: Protocol Labs, Inc. (7Y229E2YRL)` で署名済み。Apple の timestamp 付き（2026-09-15）で、hardened runtime（`CodeSignatureFlags(RUNTIME)`）付き。
- notarization も済んでいる。CodeDirectory の cdhash（sha256 の先頭 20 バイト、`05857fafc33818d06849ff9ce7ca5688447659fe`）で Apple のチケット配布（CloudKit の `com.apple.gk.ticket-delivery`）を引くと、`DeveloperIDTicket` が返る。ただしチケットは tarball に staple されていない（単体の Mach-O には staple できない）ので、オフラインの初回起動ではチケットを確かめられない。

なので、インストーラがブラウザを通さずに upstream から直接取ってくる経路を作っても、Gatekeeper には引っかからない。そもそも `curl` や Homebrew で取ったファイルには quarantine 属性が付かないので、ブラウザで落とす経路を作らなければこの点は気にしなくていい。

## 検証

- `actionlint`（`rhysd/actionlint` のコンテナ、shellcheck 込み）で指摘なし。
- ワークフロー自体は GitHub で動かしていない（push していない）。
