# macOS のアプリバンドル（SWING.app）

`docs/todo.md` の「macOS: `swing-tray` を `.app` バンドルにして、Finder と「ログイン項目」にアイコンと名前を出す」に着手した。構成の現状は [`../architecture/tray.md`](../architecture/tray.md#macos-のアプリバンドルswingapp)・[`../architecture/service.md`](../architecture/service.md)・[`../architecture/release.md`](../architecture/release.md)。

## 背景

[macOS の動作確認ワークフロー](2026-09-27-macos-check-workflow.md) の `sfltool dumpbtm` では、2 つの LaunchAgent が実行ファイル名（`swing`・`swing-tray`）の legacy agent として登録されていた。素の Mach-O にはアイコンを持たせる場所が無い（[アプリのアイコン](2026-09-24-app-icons.md)）。

## 決めたこと

| 決定 | 理由 |
|---|---|
| バンドルに入れるのは `swing-tray` だけにし、`swing` は tarball の直下に素の実行ファイルのまま置く | `swing` は CLI として PATH に置いて使う。バンドルの中に入れるとシンボリックリンクなどが要る |
| `service install` は `swing` の隣の `SWING.app/Contents/MacOS/swing-tray` だけを見て、素の `swing-tray` は見ない | 判定を 1 つにする。前の形で登録した人は `service install` し直せば新しい形になる |
| 本体とトレイの両方の LaunchAgent に `AssociatedBundleIdentifiers = [jp.ne.ama.swing]` を付ける | 「ログイン項目と機能拡張」で 2 つとも SWING としてまとめて出すため |
| バンドル ID は `jp.ne.ama.swing`（LaunchAgent の Label と同じ文字列） | アプリ名が SWING なので。Label とバンドル ID は別の名前空間 |
| バンドルはスクリプト（`tray/macos/bundle.sh`）で組み、`cargo-bundle` などは入れない | 中身は Info.plist と 2 ファイルだけ。release と `macos-check` の両方から同じものを使える |
| `.icns` はリポジトリにコミットし、ビルド時には作らない | `.ico` と同じ扱い。Linux でも作れる（CairoSVG で書き出して Pillow で格納） |
| アイコンはロゴを 1024px の中央に 824px で置き、背景は付けない | macOS のアプリアイコンの余白の目安に合わせた。角丸の背景を付けるかは見え方を見てから |
| `codesign` があればバンドルに ad-hoc 署名する | Info.plist をコード署名に結び付けておく。ランナーの `codesign` で足り、`rcodesign` は要らない |
| `LSUIElement` も入れる | 今の activation policy の `Accessory` と同じく Dock に出さない。起動直後に一瞬 Dock に出るのも防ぐ |
| `macos-check` の System Events の操作は、プロセス名ではなく `pgrep -x swing-tray` の PID で探す | バンドルに入れるとプロセスの表示名が `CFBundleName`（`SWING`）になるため |

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace`・`cargo xwin clippy --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings` が通った。`cargo test` の 1 回目は `signer::tests::answers_from_a_signer_whose_clock_runs_behind_are_received`（todo にある、まれに落ちるテスト）が落ち、再実行で通った。
- `bundle.sh` を Linux で動かし、バンドルの構成と Info.plist のバージョンの置き換えを確かめた（`codesign` の無い環境なので署名は飛ぶ）。
- `SWING.icns` を Pillow で読み戻し、16〜1024px（Retina の 2x を含む）が入っていることを確かめた。

確かめていないこと（todo）: macOS での `dumpbtm` の表示、Finder のアイコン、バンドルにしたトレイの動作。`macos-check` を回して確かめる。
