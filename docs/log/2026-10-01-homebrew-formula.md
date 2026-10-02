# Homebrew の formula

## 背景

macOS 向けの配布は tap `amane-katagiri/swing`（リポジトリ `amane-katagiri/homebrew-swing`）の formula にすると決めていた（[2026-09-21 の log](2026-09-21-distribution-design.md)）。cask にしないのは quarantine 属性が付くため、`depends_on "kubo"` で Kubo も入れる。tap のリポジトリはこのリポジトリを公開するときに作るので、今回はこのリポジトリの中で formula を作れるところまでを用意した。

Homebrew は formula をバージョンごとの keg（`<prefix>/Cellar/swing/<version>`）に入れ、`<prefix>/bin` と `<prefix>/opt/swing` からシンボリックリンクを張る。`brew upgrade` は既定で古い keg を消す。一方 `swing service install` は実行ファイルのパスを LaunchAgent の plist に埋め込み、トレイは `swing` の隣の `SWING.app` を登録する。plist に keg のパスが入ると、`brew upgrade` の後のログインで起動できなくなる。

## 調べたこと（`current_exe()` が返すパス）

- Rust の `std::env::current_exe()` は Apple のプラットフォームでは `_NSGetExecutablePath` の結果をそのまま返し、`realpath` しない（`library/std/src/sys/paths/unix.rs`）。
- dyld の `_NSGetExecutablePath` は macOS では `mainUnrealPath` を返す（`dyld/DyldAPIs.cpp` に「this is not real-path. It may be a symlink」とある）。`mainUnrealPath` はカーネルが apple パラメータの `executable_path` で渡す exec 時のパスで、相対パスのときだけカレントディレクトリを前に付ける（`dyld/DyldProcessConfig.cpp` の `getMainUnrealPath`）。シンボリックリンク経由で起動すればそのリンクのパスになる。
- ところが `service::resolve_service_paths` は `current_exe()` を `canonicalize` していた。どう起動しても plist には keg のパスが入る。Rust を変えずに済ませる案（`post_install` で keg の外に複製する、upgrade のたびに `swing service install` をやり直してもらうだけにする）は、keg の外にファイルを置き去りにするか、やり直しを忘れると次のログインで起動しないので採らなかった。
- Linux の `current_exe()` は `/proc/self/exe` で、カーネルが解決したパスを返す。Linuxbrew で同じ手は使えない。

## 決めたこと

- `swing` と `SWING.app` を `libexec` に入れ、`bin/swing` は `bin.write_exec_script opt_libexec/"swing"` のスクリプト（`exec "<prefix>/opt/swing/libexec/swing" "$@"`）にする。`current_exe()` は `opt` のパスになり、upgrade をまたいで変わらない。`bin/swing` を `libexec/swing` へのシンボリックリンクにする案では、`current_exe()` が `<prefix>/bin/swing` になり、隣に `SWING.app` が無いので採らなかった。
- Rust の変更は 2 つに絞った。
  - `resolve_service_paths` で実行ファイルのパスを `canonicalize` せず `std::path::absolute` にする。設定ファイルのパスはこれまでどおり `canonicalize` する。macOS 以外への影響: Linux はもともと解決済みのパス、Windows の `current_exe()`（`GetModuleFileNameW`）は絶対パスで、`canonicalize` が付けていた `\\?\` が付かなくなるだけ（登録時にはもともと外していた）。
  - `tray_exe_path` は `<exe>` の隣で見つからなければ、`canonicalize` した先の隣も探す。アーカイブを展開した場所の `swing` だけを PATH の通った場所へシンボリックリンクしていた人は、これまで `canonicalize` のおかげでトレイが見つかっていたので、その構成を壊さないため。登録される本体のパスはリンクのパスになる。
- `brew services`（formula の `service do`）は使わない。`swing service install` が本体とトレイの 2 つの LaunchAgent・既定の設定ファイルの作成・`AssociatedBundleIdentifiers` をまとめて扱っており、`brew services` は 1 つの plist しか持てず、トレイの登録と食い違う。同じラベルを二重に登録する恐れもある。
- `caveats` で `swing service install` と `swing dashboard open`、設定とデータとログの場所、`brew upgrade` の後に `swing service install` をやり直すこと、`brew uninstall` の前に `swing service uninstall` を実行することを案内する。`swing stop --restart` はプロセス内の再起動で実行ファイルが入れ替わらないので、upgrade 後の再起動には使えない。`swing service install` のやり直しは両方の LaunchAgent を `bootout`・`bootstrap` するので、本体もトレイも新しい版になる。
- formula からの `post_install` での LaunchAgent の登録はしない。Homebrew のサンドボックスで `~/Library/LaunchAgents` に書けず、利用者の状態を formula が勝手に変えることにもなるため。
- Homebrew の Kubo は `KUBO_VERSION` と違うことがある。今は警告だけなので formula では固定せず、README と architecture に書くだけにした。リポジトリの移行は `ipfs daemon --migrate=true` に任せる。
- Linux（Linuxbrew）向けの `on_linux` は入れなかった。上記のとおり `current_exe()` が keg のパスになり、`swing service install` の systemd unit が upgrade で壊れるため。Linux はインストールスクリプトで配る。
- 現行の Homebrew の `brew audit` は `on_arm`／`on_intel` の中の `url`／`sha256` を受け付けないので、`Hardware::CPU.arm?` で分けた。`version` は URL から読み取れるので書かない（書くと audit が冗長だと指摘する）。
- formula はリリースのたびに `release.yml` の `homebrew` ジョブが `render.sh` で書き出し、リリースの添付ファイルにする。tap への反映はその `swing.rb` を `Formula/` に置くだけにした。tap への自動 push はトークンが要るので、tap のリポジトリができてから考える。
- `homebrew-check` のワークフローは、同じ中身のアーカイブを 2 つのバージョン名で作り、ローカルの tap（`brew tap-new --no-git`）と `file://` の URL で install・upgrade を通す。Homebrew は tap の外の formula ファイルを受け付けない（`brew style ./swing.rb` も「Homebrew requires formulae to be in a tap」で拒む）。

## 署名と Gatekeeper

- formula がダウンロードしたファイルには quarantine 属性が付かないので、公証は要らない。
- Apple Silicon では未署名の実行ファイルは起動できない。`SWING.app` は `bundle.sh` が ad-hoc 署名する。`swing` は arm64 のリンカが付ける ad-hoc 署名だけに頼っている。release ビルドで rustc が走らせる `strip` の後もこの署名が有効かは確かめられていないので、`homebrew-check` で `codesign --verify --strict` し、quarantine 属性とあわせて記録する。無効だった場合は `bundle.sh` と同じく release の Package で `codesign --force --sign -` を足す。
- Intel の `swing` には署名が無いが、Intel の Mac では署名が無くても起動できる。

## 検証

- `render.sh` を、ダミーのアーカイブから作った `SHA256SUMS`（`sha256sum` の通常とバイナリモード）で動かし、正しい SHA-256 が入ること、該当するアーカイブが無いと失敗すること、URL の基点の `|`・`&`・`\` がそのまま入ること、引数の数が違うと使い方を出すことを確かめた。shellcheck を通した。
- 書き出した `swing.rb` で `ruby -c` を通し、一時的なローカルの tap に置いて `brew style` と `brew audit --strict` を通した（Linux の Homebrew で。インストールはしていない）。
- `release.yml` と `homebrew-check.yml` を actionlint（shellcheck 込み）で確かめた。どちらのワークフローも実行していない。
- `tray_exe_path` の単体テストに、ディレクトリのシンボリックリンク越しならリンクのパスのまま、`swing` だけのシンボリックリンクなら実体の隣で見つかることを足した。`cargo fmt`・`cargo clippy`・`cargo test`・Windows 向けの `cargo xwin clippy` を通した。
- macOS での `brew install`・`swing service install` の plist のパス・`brew upgrade` の後の振る舞いは実機で確かめていない。`homebrew-check` を手動で実行して確かめる。
