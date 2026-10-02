# インストーラーが自分の入れた swing の登録だけを扱う

## 背景

Windows のインストーラー（Inno Setup）・Linux の `install.sh` のアンインストールは `swing service uninstall` を無条件に実行し、Windows の上書きはタスクが登録済みなら `{app}` から `swing service install --no-start` をやり直していた。利用者がインストーラーとは別に、zip や tarball を展開した `swing` から `swing service install` していた場合、アンインストールがその登録を消し、上書きがその登録を `{app}` の `swing.exe` に付け替えてしまう。`install.sh` の更新も、動いているユーザー unit を誰のものかを見ずに止めて起動し直していた。

## 決めたこと

- 方針: インストーラーは、登録が起動する実行ファイルが自分のインストール先（Windows は `{app}`、`install.sh` は `<prefix>/lib/swing`）の下にある登録だけを消したり登録し直したりする。別の場所の登録は残し、残したことを知らせる。
- CLI は 2 つのオプションにした。
  - `swing service uninstall --only-from <dir>`: `<dir>` の下を指す登録だけを消す。別の場所を指す登録ごとに 1 行出し、終了コードは 0。
  - `swing service status --points-into <dir>`: 登録ごとに 1 行出し、終了コードで答える（0: 登録があってすべて下、3: 未登録、4: 下でないものがある）。上書きのときに「登録し直してよいか」「止めてよいか」を聞くのに使う。
  - `uninstall` に「確かめるだけ」のモードを足す案もあったが、問い合わせと削除は別の操作なので分けた。判定そのもの（`service/ownership.rs`）は 1 つで、両方が使う。終了コードは、anyhow のエラーの 1 と clap の使い方の誤りの 2 を避けて 3 と 4 にした。
- 判定は、登録の定義から実行ファイルを読み直す。Linux は unit の `ExecStart`、macOS は plist の `ProgramArguments[0]`（本体とトレイ）、Windows はタスクの XML（`schtasks /Query /XML`）の `conhost.exe` の引数と、Run キーの値 `swing-tray` の先頭の引数。どれも `service/templates.rs` が書くエスケープの逆をたどり、テストはテンプレートで作ったものを読み戻して確かめる。
- パスは要素ごとに比べる（文字列の前置きだと `SWING-old` が `SWING` の下になる）。Windows は大文字小文字を区別せず、`\\?\` を外す。シンボリックリンク経由に備えて、`canonicalize` したものどうしも比べる。
- 混ざった状態（本体とトレイが別々の場所を指す）の扱い。
  - `uninstall --only-from` は本体とトレイを別々に判定し、下にあるものだけを消す。本体が別の場所でトレイだけが自分のものなら、トレイの登録だけを消し、本体は止めもしない。消すインストール先のトレイを残すと、ログインのたびに消えた実行ファイルを起動しようとするため。
  - `status --points-into` は、どちらか一方でも別の場所なら 4 にする。Windows の上書きはそのとき何も登録し直さない。`--no-tray` で登録し直すとトレイの値を消し、付けなければ書き換えるので、どちらにしても他人の登録に触れてしまう。自分の本体の登録は前の版のまま残るが、パスは同じ `{app}` なので動く。
  - 実行ファイルを読み取れない登録は「下にない」として扱い、消さない。
- `--system`（Linux）でも同じように使える。`install.sh` のアンインストールが system unit を見て拒否する判定にも使い、別の場所の `swing` の system unit（ディストリビューションのパッケージなど）では拒否しないようにした。
- 古い版との組み合わせ（v0.1.0 の `swing` は新しいオプションを知らない）。互換のための処置は入れず、確認には常に新しい版の `swing` を使う順にした。
  - Windows の上書き: ファイルを置く前の `PrepareToInstall` で、新しい `swing.exe` を `ExtractTemporaryFile` で `{tmp}` に取り出して確かめる。置いた後に確かめる案もあったが、それでは止める前に持ち主が分からず、別のコピーのタスクを `schtasks /End` で止めうる。持ち主が別なら、`{app}` のプロセスはダッシュボード経由でだけ止める `swing stop` で止める（`service stop` は失敗すると `schtasks /End` に落ちる）。
  - Windows のアンインストール: アンインストーラーと `{app}\swing.exe` は同じ版なので問題にならない。
  - `install.sh` の更新: 取得した新しい版の `swing` を `lib/swing/.swing-check.<pid>` に写して確かめる。展開先の一時ディレクトリは `noexec` のことがあるので、そこからは実行しない。
  - `install.sh` のアンインストールは入っている `swing` を使う。v0.1.0 が入っているところで新しい `install.sh --uninstall` を実行すると、`--only-from` を知らずに失敗し、何も消さずに終わる（入っている `swing-uninstall.sh` は同じ版の組なので動く）。安全側に倒れるので、そのままにした。
- Windows の GUI のアンインストールでは、残した登録があれば完了のメッセージの前にその行を出す（`KeptRegistrations`）。Inno Setup では完了のメッセージ（`UninstalledAll`）の中身を実行時に変えられないので、別のメッセージにした。サイレントでは出さない。残した登録があるときはタスクが消えずトレイが自分で閉じないので、トレイを待たずに強制終了する。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo xwin clippy --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings`・`cargo test --workspace` が通ることを確かめた。macOS 向けはこの環境ではビルドできず（`ring` の C コンパイル）、コードを読んで確かめただけ。
- 単体テスト: テンプレート（unit・plist・タスク XML・Run の値）から実行ファイルを読み戻せること（空白・引用符・`$`・`%`・`&` を含むパス、Task Scheduler が `&quot;` を `"` に書き直した XML）、パスの比べ方（大文字小文字・`\\?\`・UNC・`/`・`..`・似た名前のディレクトリ・相対パス）、シンボリックリンク経由の一致、終了コードのまとめ方。
- Linux で、一時的な `XDG_CONFIG_HOME` に手で書いた unit を置き、ビルドした `swing` の `status --points-into` が未登録で 3、別の場所で 4、下にあれば 0 になること、`uninstall --only-from` が別の場所の unit を残して 0 で終わることを確かめた。
- `sh packaging/linux/test-install.sh`: 偽の `swing` が unit の `ExecStart` を見て答えるようにし、更新で持ち主を新しい版で確かめること、別の場所の `swing` のサービスを止めず確認用のファイルも残らないこと、アンインストールが別の場所の unit を残すこと、別の場所の system unit で拒否しないことを足して、すべて通ることを確かめた。`shellcheck` も警告なし。
- `swing.iss` を Wine で動かす Inno Setup 6 の ISCC で、中身の無いファイルを置いた `stage` でコンパイルし、警告もエラーも無く通ることを確かめた（`AppVersion` が `0.2.0` と `0.2.0-rc.1`）。
- `check-installer.ps1` に `foreign` の段階を足し（`windows-installer-check` にも手順を足した）、PowerShell の構文解析が通ることだけを確かめた。ワークフローはまだ走らせていない（残タスクの「インストーラの実機での確認」に含まれる）。
