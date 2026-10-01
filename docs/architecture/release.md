# ビルドとリリース・動作確認の CI（`.github/workflows/`）

[`../architecture.md`](../architecture.md) の一部。配布物に入る `swing-tray` は [`tray.md`](tray.md)、コンテナイメージの土台は [`docker.md`](docker.md)。

`.github/workflows/release.yml` が配布用のバイナリとコンテナイメージを作る。`v*` タグの push と手動実行（`workflow_dispatch`）で動き、push や PR では動かない。

| target | ランナー | fmt / clippy / test | ビルド |
|---|---|---|---|
| `x86_64-unknown-linux-musl` | ubuntu-latest | する（ホストの gnu で） | `cargo zigbuild` |
| `aarch64-unknown-linux-musl` | ubuntu-latest | しない | `cargo zigbuild` |
| `aarch64-apple-darwin` | macos-latest | する | `cargo build` |
| `x86_64-apple-darwin` | macos-latest | しない | `cargo build`（クロス） |
| `x86_64-pc-windows-msvc` | windows-latest | する | `cargo build` |

- Rust のバージョンは Dockerfile の builder と同じ（[`docker.md#dockerfile`](docker.md#dockerfile)）。Linux は musl の静的バイナリにする。
- workspace には `swing` と `swing-tray`（`tray/`）がある。fmt は `cargo fmt --all --check`、clippy / test は `--workspace` で回し、いずれもワークスペース全体（`tray/` を含む）を対象にする。Linux は `-p swing` だけをビルドし、Windows と macOS は `--workspace` でビルドする。
- Windows 向けのビルドでは、`build.rs`（`swing` と `swing-tray` の両方）が `winresource` でファイルアイコン（`swing` は `assets/swing.ico`、`swing-tray` は `tray/assets/swing-tray.ico`）とバージョン情報（`Cargo.toml` の `name`・`version`）を exe に埋め込む。他の target では何もしない。
- macOS の `swing` は素の実行ファイルで、ファイルアイコンは付かない。`swing-tray` は `tray/macos/bundle.sh` で `SWING.app` にまとめ、アイコンと名前はバンドルが持つ（[`tray.md`](tray.md#macos-のアプリバンドルswingapp)）。
- 成果物は `swing-<ref>-<target>.tar.gz`（Windows は `.zip`）で、中身は `swing`（`swing.exe`）・`LICENSE`・`web/fonts/LICENSE-PixelMplus.txt`・`swing.example.toml` と、`docs/release/README.md` を `README.md` に改名したもの。Windows には `swing-tray.exe`、macOS には `SWING.app` も入れる。Kubo は同梱しない。
- `release` ジョブは、`packaging/linux/install.sh`（[`install-sh.md`](install-sh.md)）を `install.sh` としてアーカイブと並べてリリースに添える。`SHA256SUMS` はこれを含めて作る。
- タグの ref で動いたとき（タグの push と、タグを選んだ手動実行）は、タグ名と `Cargo.toml` の `version` が一致しないと失敗する（`v0.1.0` と `0.1.0`）。全 target が通ると `SHA256SUMS` を付けた**ドラフト**のリリースを作る。公開は GitHub 上で手動で行う。
- ブランチで手動実行したときはリリースを作らず、バイナリは Actions の artifact に残す。イメージは下記のとおり push する。
- `image` ジョブが `ghcr.io/<owner>/<repo>`（小文字）のコンテナイメージを `linux/amd64`・`linux/arm64` で作る。中身は `build` ジョブの `x86_64-unknown-linux-musl`・`aarch64-unknown-linux-musl` の `swing` を `docker/release.Dockerfile`（[`docker.md`](docker.md#dockerfile)）に入れたもの。QEMU は `RUN`（ユーザー作成）にだけ使う。
- イメージのタグは、タグの ref なら `v` を除いたバージョン（`0.1.0`）と `latest`（バージョンに `-` を含む `0.2.0-rc.1` などでは `latest` を付けない）。タグのイメージは、ドラフトのリリースを公開する前に push される。ブランチの ref ならブランチ名（`main` など）のタグだけを付けて push する。ブランチ名に `/` があると `image` ジョブが失敗する。ブランチ名が `latest` か数字で始まるときは push せずに失敗する。`org.opencontainers.image.source` ラベルでパッケージをこのリポジトリに紐づけ、パッケージの公開範囲はリポジトリに合わせる。

サードパーティの action・ツール:

| 名前 | 役割 |
|---|---|
| `dtolnay/rust-toolchain` | 指定バージョンの Rust ツールチェインをインストール |
| `Swatinem/rust-cache` | Cargo のビルドキャッシュ |
| `taiki-e/install-action` | `cargo-zigbuild` をビルド済みバイナリからインストール |
| ziglang（PyPI、`pip3 install`） | `cargo zigbuild` が使う Zig 本体 |
| `docker/setup-qemu-action`・`docker/setup-buildx-action`・`docker/login-action`・`docker/build-push-action` | マルチアーキテクチャのイメージのビルドと ghcr.io への push |
| `mxschmitt/action-tmate` | `macos-check`・`windows-check` の最後に、ランナーへ SSH で入れる tmate のセッションを開く（下記） |

サードパーティおよび `actions/*`（`actions/checkout`・`actions/upload-artifact`・`actions/download-artifact`）の action はフルコミット SHA に固定し、末尾に `# vN` コメントでタグ相当のバージョンを添えている。ziglang は pip の `==` でバージョンを固定する。Rust ツールチェインのバージョン自体はこれらのピン留めとは別で、ワークフローの `toolchain:` 入力（環境変数 `RUST_TOOLCHAIN`）で決まる。選定理由と信頼性の評価は [2026-09-25 の log](../log/2026-09-25-release-actions-rationale.md) を参照。

## macOS の動作確認（`.github/workflows/macos-check.yml`）

手動実行（`workflow_dispatch`）でだけ動く。`macos-latest` のランナー（ロケールと画面の情報は artifact に記録する）で `--workspace` を release ビルドして `swing` の隣に `SWING.app` を作り、`swing.example.toml` の写しを設定ファイルにして、鍵の無いセットアップモード（[`up.md`](up.md#セットアップモード鍵未設定)）で動かす。Kubo も relay も使わない。手順と操作の仕方の正本は [`macos-check.yml`](../../.github/workflows/macos-check.yml)。

確かめること（多くは判定せずに画面とテキストを残し、artifact で確かめる）:

- `swing service install` で本体とトレイの LaunchAgent が登録されて動き、トレイが Dock に出ないこと。Finder と「ログイン項目と機能拡張」での名前と署名。
- メニューの項目と有効・無効（動作中と停止中、ライトとダーク、英語と日本語）。
- `swing stop` と、メニューからの起動・停止・終了。確認のダイアログの文言とボタン、各ボタンで `swing up` とトレイが止まる・残ること。
- 「ダッシュボードを開く」で Safari がログインリンクを開き、ログインできること。
- `swing service uninstall` の後に LaunchAgent とプロセスが残らないこと（前の段階が失敗しても行う）。

失敗時の見方:

- `service install`・`swing stop`・日本語への切り替え・`service uninstall` 以外の段階は失敗しても続ける（`continue-on-error`）ので、ジョブの成否だけでなく各ステップの結果を見る。
- 各段階の画面（全体とメニューバー）・保存したテキスト・`~/Library/Logs/swing*.log`・TCC の許可の一覧は artifact `macos-check` に残る。
- 入力 `ssh` を true にすると、最後に `mxschmitt/action-tmate` で実行した本人だけが入れる tmate のセッションを開く。
- ログインし直したときの自動起動と、Retina での表示は、ランナーでは確かめられない。

## Windows の動作確認（`.github/workflows/windows-check.yml`）

手動実行（`workflow_dispatch`）でだけ動く。`windows-latest` のランナー（表示言語・ロケールと画面の情報は artifact に記録する）で `--workspace` を release ビルドし、`swing.exe`・`swing-tray.exe` と `swing.example.toml` の写しを同じ作業ディレクトリに置いて、鍵の無いセットアップモードで動かす。Kubo も relay も使わない。手順と操作の仕方の正本は [`windows-check.yml`](../../.github/workflows/windows-check.yml)。

確かめること:

- 登録前の `swing service status` が `not installed` で、`swing service install` の後は登録済みになること（トレイのプロセスと Run キーの値は保存するだけで判定しない）。
- 動作中のメニューの項目と有効・無効。「Stop」→「No」で止まらず、「Quit」→「Cancel」でトレイが残ること。
- `swing stop` で止まっても登録が残り、停止中のメニューの「Start」で起動すること。
- `swing service uninstall` の後、トレイが 30 秒以内に自分で閉じ、`swing service status` が `not installed` に戻ること（前の段階が失敗しても行う。Run キーは保存するだけで判定しない）。

失敗時の見方:

- `Prepare` から `Uninstall the service` までの各ステップは 3 分でタイムアウトする。タスクバーの準備・動作中のメニューとダイアログ・停止中のメニューのステップは失敗しても続ける（`continue-on-error`）。
- 撮った画像・テキスト・`swing.log` は artifact `windows-check` に残る。入力 `ssh` は `macos-check` と同じ。
- サインインし直したときの自動起動と、日本語の表示言語は、ランナーでは確かめられない。

## ローカルでのクロスビルド

Windows 向けの clippy とビルドのコマンドは [`AGENTS.md`](../../AGENTS.md#コーディング規則)。
