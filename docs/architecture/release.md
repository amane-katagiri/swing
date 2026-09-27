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
- workspace には `swing` と `swing-tray`（`tray/`）がある。fmt は `cargo fmt --all --check`、clippy / test は `--workspace` で回し、いずれもワークスペース全体（`tray/` を含む）を対象にする。Linux は `-p swing` だけをビルドし、Windows と macOS は `--workspace` でビルドする。
- Windows 向けのビルドでは、`build.rs`（`swing` と `swing-tray` の両方）が `winresource` でファイルアイコン（`swing` は `assets/swing.ico`、`swing-tray` は `tray/assets/swing-tray.ico`）とバージョン情報（`Cargo.toml` の `name`・`version`）を exe に埋め込む。他の target では何もしない。
- macOS の `swing` は素の実行ファイルで、ファイルアイコンは付かない。`swing-tray` は `tray/macos/bundle.sh` で `SWING.app` にまとめ、アイコンと名前はバンドルが持つ（[`tray.md`](tray.md#macos-のアプリバンドルswingapp)）。
- 成果物は `swing-<ref>-<target>.tar.gz`（Windows は `.zip`）で、中身は `swing`（`swing.exe`）・`LICENSE`・`README.md`。Windows には `swing-tray.exe`、macOS には `SWING.app` も入れる。Kubo は同梱しない。
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
| `mxschmitt/action-tmate` | `macos-check`・`windows-check` の最後に、ランナーへ SSH で入れる tmate のセッションを開く（下記） |

サードパーティおよび `actions/*`（`actions/checkout`・`actions/upload-artifact`・`actions/download-artifact`）の action はフルコミット SHA に固定し、末尾に `# vN` コメントでタグ相当のバージョンを添えている。ziglang は pip の `==` でバージョンを固定する。Rust ツールチェインのバージョン自体はこれらのピン留めとは別で、ワークフローの `toolchain:` 入力（環境変数 `RUST_TOOLCHAIN`）で決まる。選定理由と信頼性の評価は [2026-09-25 の log](../log/2026-09-25-release-actions-rationale.md) を参照。

## macOS の動作確認（`.github/workflows/macos-check.yml`）

Mac の実機が無くても `swing-tray` とサービス登録を macOS で動かして確かめるためのワークフロー。手動実行（`workflow_dispatch`）でだけ動く。`macos-latest` のランナー（GUI のログインセッションがあり、画面は 1024×768 の等倍、ロケールは `en_US`）で `--workspace` を release ビルドして `swing` の隣に `SWING.app` を作り、`swing.example.toml` の写しを設定ファイルにして、鍵の無いセットアップモード（[`up.md`](up.md#セットアップモード鍵未設定)）で動かす。Kubo も relay も使わない。CLI は設定ファイルのディレクトリで実行する。

順に次を行い、各段階で `screencapture` で画面全体と、メニューバーの右半分（開いたメニューを含む）を撮る。

1. `swing service install` → ダッシュボードが応答するのを待ち、`launchctl print`（`jp.ne.ama.swing`・`jp.ne.ama.swing-tray`）・`pgrep`・`lsappinfo`（`ApplicationType` が `UIElement` なら Dock に出ない）・`swing service status`・メニューバーでのアイコンの位置と大きさを保存する
2. `lsappinfo` でのトレイのプロセスの名前と `codesign -dv` を保存し、Finder で `SWING.app` を表示して撮る
3. `sudo sfltool dumpbtm`（「ログイン項目と機能拡張」のバックグラウンド項目としての登録。名前・種類・実行ファイルのパス）を保存する
4. 動作中のメニューを開く（ライト → ダーク）。開いたメニューの項目名と有効・無効も保存する
5. `swing stop` で止め、停止中のアイコンとメニューを撮る（ダーク → ライト）
6. トレイのメニューを開いて ↓ → Return（停止中に最初に選べるのは「Start」）で起動し、動作中に戻るのを撮る
7. `swing service uninstall` の後の `~/Library/LaunchAgents` とプロセスを保存する（前の段階が失敗しても行う）

操作はランナーのシェルから `osascript` で行う（`/bin/bash` と `osascript` にはアクセシビリティと Apple Events の許可が付いている）。

- `swing-tray` のアイコンは、System Events では `swing-tray` のプロセス（`SWING.app` に入れたので名前は `SWING` になる。`pgrep -x swing-tray` の PID の `unix id` で探す）の `menu bar 1` の `menu bar item 1` に見える。AX の `click` では `tray-icon` のメニューが開かないので、その位置の中央に `CGEventPost` でマウスのクリックを送る（JXA）。
- ダークモードは System Events の `appearance preferences` で切り替える。

2〜6 は失敗しても続ける（`continue-on-error`）。撮った画像・テキスト・`~/Library/Logs/swing*.log`・TCC の許可の一覧は artifact `macos-check` に残す。入力 `ssh` を true にすると、最後に `mxschmitt/action-tmate` で実行した本人だけが入れる tmate のセッションを開く。ログインし直したときの自動起動と、Retina での表示は、ランナーでは確かめられない。

## Windows の動作確認（`.github/workflows/windows-check.yml`）

英語の表示言語の Windows で `swing-tray` とサービス登録を動かして確かめるためのワークフロー。手動実行（`workflow_dispatch`）でだけ動く。`windows-latest` のランナー（Windows Server 2025、表示言語・ロケールとも `en-US`、`runneradmin` の対話セッションでジョブが動き、画面は 1024×768）で `--workspace` を release ビルドし、`swing.exe`・`swing-tray.exe` と `swing.example.toml` の写しを同じ作業ディレクトリに置いて、鍵の無いセットアップモードで動かす。Kubo も relay も使わない。スクリプトは PowerShell で、画面は `System.Drawing` の `CopyFromScreen` で撮る。

順に次を行う。

1. OS・表示言語・ロケール・セッションの情報と、`schtasks /Query /TN swing` の失敗時の出力を保存し、登録前の `swing service status` が `not installed` になることを確かめる
2. `swing service install` → ダッシュボードが応答するのを待ち、トレイのプロセス・Run キー・`swing service status`（登録済み）を保存する
3. すべてのウィンドウを最小化し、ランナーが起動時に出す「System Properties」のダイアログがあれば閉じる
4. 動作中のメニューを開いて項目名と有効・無効を保存し、「Stop」→ 確認のダイアログで「No」（止まらないこと）、「Quit」→「Cancel」（トレイが残ること）
5. `swing stop` で止め、停止中のメニューを保存する
6. 停止中のメニューの「Start」で起動し、ダッシュボードが応答するのを待つ
7. `swing service uninstall` → トレイが 30 秒以内に自分で閉じ、`swing service status` が `not installed` に戻り、Run キーの値が消えることを確かめる（前の段階が失敗しても行う）

- トレイのアイコンは UI Automation で、タスクバー（`Shell_TrayWnd`）か通知領域のあふれ（`TopLevelWindowForOverflowXamlIsland`）の中の、名前（ツールチップ）が `SWING` で始まるボタンとして探す。あふれに入っていれば「Show Hidden Icons」を押してから探す。
- `muda` のポップアップメニュー（`#32768`）の項目は UI Automation には見えないので、`MN_GETHMENU` で `HMENU` を取り、`GetMenuStringW`・`GetMenuState`・`GetMenuItemRect` で項目名・有効かどうか・位置を読む。クリックは `SetCursorPos` と `mouse_event` で送る。
- 確認のダイアログは名前が `SWING` の `#32770` のウィンドウで、ボタンは UI Automation ではクラス名 `Button` の要素として見える。

ステップは 3 分でタイムアウトし、3・4・5 は失敗しても続ける（`continue-on-error`）。撮った画像・テキスト・`swing.log` は artifact `windows-check` に残す。入力 `ssh` は `macos-check` と同じ。サインインし直したときの自動起動と、日本語の表示言語は、ランナーでは確かめられない。

## ローカルでのクロスビルド

```bash
cargo xwin clippy --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings
cargo xwin build --release --workspace --target x86_64-pc-windows-msvc
```
