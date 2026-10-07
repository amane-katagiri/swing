# Homebrew の formula（packaging/homebrew/）

[`../architecture.md`](../architecture.md) の一部。formula が入れるファイルの元は release のアーカイブ（[`release.md`](release.md)）、登録される LaunchAgent は [`service.md`](service.md)、`SWING.app` は [`tray.md`](tray.md#macos-のアプリバンドルswingapp)。

macOS 向けに、tap `amane-katagiri/swing`（リポジトリ `amane-katagiri/homebrew-swing`）の formula（cask ではない）で配る。利用者は `brew install amane-katagiri/swing/swing` で入れる。ソースからはビルドせず、GitHub Releases のビルド済みアーカイブを入れる。

## ファイル

| パス | 役割 |
|---|---|
| `packaging/homebrew/swing.rb.in` | formula のひな形。`@TAG@`・`@URL_BASE@`・`@SHA256_AARCH64@`・`@SHA256_X86_64@` を置き換える |
| `packaging/homebrew/render.sh` | `render.sh <tag> <SHA256SUMS> <出力先> [<ダウンロード URL の基点>]` で `swing.rb` を書き出す（POSIX sh） |

`render.sh` の動作:

- `SHA256SUMS`（`sha256sum`／`shasum -a 256` の形式。バイナリモードの `*` 付きの名前も読む）から `swing-<tag>-aarch64-apple-darwin.tar.gz` と `swing-<tag>-x86_64-apple-darwin.tar.gz` の SHA-256 を取る。どちらかが無い、または 64 桁の 16 進でなければ「no SHA-256 for <file> in <SHA256SUMS>」で終了コード 1。
- URL の基点の既定は `https://github.com/amane-katagiri/swing/releases/download/<tag>`。末尾の `/` は落とす。
- タグが空か英数字と `.`・`_`・`/`・`+`・`-` 以外を含む、または URL の基点が `"`・`\`・`#`・空白を含むなら終了コード 1。
- 置き換え後に `@…@` が残っていれば終了コード 1。

## formula の中身

| 項目 | 値 |
|---|---|
| `url`・`sha256` | `Hardware::CPU.arm?` で `aarch64-apple-darwin` と `x86_64-apple-darwin` のアーカイブを選ぶ。`version` は書かず URL から読み取らせる |
| `depends_on` | `"kubo"`・`:macos`。Linux 向けの指定は無い |
| `install` | `swing` と `SWING.app` を `libexec` に、`swing.example.toml` を `pkgshare` に入れる。`bin/swing` は `bin.write_exec_script opt_libexec/"swing"` で作るスクリプトで、中身は `exec "<prefix>/opt/swing/libexec/swing" "$@"` |
| `caveats` | `swing service install`・`swing dashboard open`、設定とデータ・ログの場所、`brew upgrade` 後に `swing service install` をやり直すこと、`brew uninstall` の前に `swing service uninstall` を実行すること |
| `test` | `swing --version` にバージョンが含まれること |
| `service do` | 無い。LaunchAgent は `swing service install` が登録する（下記） |

## パスと `brew upgrade`

`<prefix>` は `brew --prefix`（Apple Silicon は `/opt/homebrew`、Intel は `/usr/local`）。

| パス | 実体 | upgrade をまたいで |
|---|---|---|
| `<prefix>/bin/swing` | `<prefix>/Cellar/swing/<version>/bin/swing`（上記のスクリプト）へのシンボリックリンク | 変わらない |
| `<prefix>/opt/swing` | `<prefix>/Cellar/swing/<version>` へのシンボリックリンク | 変わらない（指す先が新しい版になる） |
| `<prefix>/opt/swing/libexec/swing` | 本体 | 変わらない |
| `<prefix>/opt/swing/libexec/SWING.app` | トレイ | 変わらない |
| `<prefix>/Cellar/swing/<version>/…` | 実体 | `brew upgrade` の後片付け（既定で有効。`HOMEBREW_NO_INSTALL_CLEANUP` があれば `brew cleanup` まで残る）で古い版は消える |

- `swing` を PATH から実行すると、スクリプトが `opt` のパスで本体を exec し、`current_exe()` はそのパスを返す。`swing service install` はこれをシンボリックリンクを解決せずに登録し、トレイもその隣の `SWING.app` で見つける（[`service.md`](service.md#共通)）ので、どちらの plist にも `Cellar` のパスは入らない。
- `brew upgrade` は動いている `swing up` とトレイを止めない。両方とも古い版のまま動き続け、`swing stop --restart`（プロセス内の再起動）でも入れ替わらない。`swing service install` をもう一度実行すると、両方の LaunchAgent を `bootout` して `bootstrap` し直すので、新しい版で起動し直す。次のログインでも新しい版で起動する。`swing up` が落ちて launchd が起動し直したときも新しい版になる。
- `swing service install` が登録する設定ファイルは [`config.md`](config.md#設定ファイルの場所) の順で決まる（カレントディレクトリの `swing.toml` が既定の場所より先）。
- Kubo は Homebrew の `kubo` の `ipfs` を PATH（LaunchAgent の `PATH` に `/opt/homebrew/bin` と `/usr/local/bin` がある）から使う。そのバージョンが `KUBO_VERSION` と違っても警告だけで動く（[`kubo.md`](kubo.md)）。`brew upgrade kubo` の後も、`swing up` を起動し直すまでは古い `ipfs` が動き続ける。リポジトリの移行は起動時の `--migrate=true` に任せる。
- `brew uninstall swing` は LaunchAgent・設定とデータ（ユーザーごとの既定の場所）・ログ（`~/Library/Logs/swing.log`）を消さない。

## 署名と Gatekeeper

- Homebrew の formula がダウンロードしたファイルには quarantine 属性が付かないので、Gatekeeper の公証の確認は走らない。
- Apple Silicon では実行ファイルに署名が要る。`SWING.app` は `tray/macos/bundle.sh` が ad-hoc 署名する。`swing` には arm64 のリンカが付ける ad-hoc 署名（linker-signed）だけがある。
- 署名と quarantine 属性の有無は `homebrew-check` のワークフローで記録し、署名は `codesign --verify --strict` で確かめる（[`release.md`](release.md#homebrew-の動作確認githubworkflowshomebrew-checkyml)）。

## tap への公開

`release.yml` の `homebrew` ジョブが、`build` ジョブの `*-apple-darwin` のアーカイブから `SHA256SUMS` を作り、`render.sh` で `swing.rb` を書き出して artifact `homebrew` に置く。URL の基点は `$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/releases/download/<ref>`。タグの ref では `release` ジョブがこれもリリースの添付ファイルにする（`SHA256SUMS` にも入る）。ブランチの ref ではアーカイブ名にブランチ名が入るので、バージョンを読み取れない formula になる（中身の確認用）。

リリースを公開した後、その `swing.rb` を tap のリポジトリの `Formula/swing.rb` に置いて push すると利用者に届く。formula のアーカイブはリリースの公開後でないとダウンロードできない（ドラフトの添付ファイルは公開されない）。
