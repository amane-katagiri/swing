# Windows のインストーラー（Inno Setup）

現状の動作は [architecture/installer-windows.md](../architecture/installer-windows.md)。ここには決めたことと理由、確かめたことを残す。

## Windows だけ GUI のインストーラーを作る

[2026-09-21 の配布方式の設計](2026-09-21-distribution-design.md)では「GUI インストーラーは作らない」とした。理由は、OS ごとに別物になることと、ブラウザで落とすと Mark-of-the-Web が付いて署名のコストが戻ることだった。Windows についてはこれを覆す。

- winget の `InstallerType: inno` のインストーラーは、そのままブラウザで落として実行する GUI のインストーラーでもある。成果物は 1 つで、winget のためにどのみち作る。
- 署名は引き続きしない。ブラウザで落とした人には SmartScreen の「詳細情報 → 実行」が出るが、それは受け入れる（README で案内する）。winget 経由では Mark-of-the-Web が付かないので出ない。
- macOS の GUI インストーラー（.pkg / .dmg）は引き続き作らない。notarization のコストが戻る事情は変わっていない。

## 決めたこと

- **ユーザー単位・UAC なし**: `PrivilegesRequired=lowest` で、`{autopf}` がユーザー単位の `%LOCALAPPDATA%\Programs` になるので `DefaultDirName={autopf}\SWING`。`PrivilegesRequiredOverridesAllowed` は付けず、管理者としてのインストールは選べない（サービスもトレイもユーザー単位の登録なので、Program Files に入れる意味が無い）。
- **インストールの最後は登録だけ**: `swing service install --no-start`。[既定の設定ファイルの場所](2026-10-01-default-config-location.md)の回で、既定の場所に空の `swing.toml` を作るようにしてあるので、何も用意していないマシンでも登録でき、次のサインインでセットアップモードになる。winget の検証で新しいプロセスや 4001 のファイアウォールのダイアログが出ないよう、サイレントでは何も起動しない。
- **「SWING を起動」はコードを変えずに組んだ**: `swing dashboard open` は `swing up` が応答しなければすぐ失敗し、待たない。待つオプションを足す案もあったが、インストーラーから `dashboard open --no-browser` を 1 秒おきに最大 60 回試せば足りるので、本体には手を入れていない。`/api/login-code` はセットアップモードでも使えるので、初回もそのままセットアップ画面に入れる。ブラウザは `swing` に開かせず、出力のリンクをインストーラーが `ShellExec` で開く（前面に出る権利を持っているのはインストーラーのプロセスのため）。
- **使用中のファイルは自分で止める**: Restart Manager（`CloseApplications`）は使わない。サイレントでは確認なしにアプリを閉じて再起動しようとするが、`conhost --headless` の下のコンソールプロセスには閉じる要求を送る手段が無く、閉じられなければ強制終了になる。`swing` は Kubo を Job Object に入れているので、強制終了すると Kubo もグレースフルな停止を経ずに止まる。代わりに既存の `swing service stop`（ダッシュボード経由で止め、だめなら `schtasks /End` で conhost を終わらせて `--exit-with-parent` で止める）を使い、プロセスが消えるのを待つ。60 秒待っても残れば強制終了する。
- **プロセスは実行ファイルのパスで探す**: `taskkill /IM` は名前だけで選ぶので、開発中に別の場所で動かしている `swing.exe` まで止めてしまう。Inno Setup の Pascal Script から WMI（`Win32_Process`）を使い、`ExecutablePath` が `{app}` の中のものだけを数え、止める。
- **トレイは強制終了する**: トレイはダッシュボード API のクライアントで、持っている状態はロックファイルだけ（プロセスが終われば外れる）。アンインストールではタスクの削除を見て自分で閉じるのを 30 秒待つが、上書きでは待たずに止める。
- **上書きでは前の状態を保つ**: タスクが未登録なら（利用者が `swing service uninstall` した）登録し直さない。Run キーの値が無ければ（`--no-tray` で登録した）`--no-tray` を付けて登録し直す。動いていた `swing up` とトレイは、更新後に起動し直す。次のサインインまで止めたままにする案もあったが、winget の `upgrade` で常駐が黙って止まるのは不便なので起動し直すことにした。Windows のファイアウォールの許可はプログラムのパスごとで、上書きしてもパスは変わらない。
- **アンインストールでデータを残す**: `%LOCALAPPDATA%\swing` には鍵と Kubo のリポジトリがあり、消すと戻せない。完了のメッセージでその場所を伝える。メッセージには `%LOCALAPPDATA%` と書かず、「ユーザーフォルダーの `AppData\Local\swing`」とした（Inno Setup のメッセージでは `%` が書式の記号になるため）。
- **`Path` の削除は `usUninstall` で行う**: Inno Setup のソースを見ると、アンインストール時の環境変数の変更の通知（`ChangesEnvironment`）はファイルを消す処理（`PerformUninstall`）の中にあり、`usPostUninstall` より前に済む。`usPostUninstall` で `Path` を書き換えると、通知の後になって Explorer に伝わらない。
- **Kubo のチェックサムをリポジトリに固定する**: dist.ipfs.tech の `.sha512` と照らすだけだと、同じ配布元が差し替えられたときに防げない。`packaging/windows/kubo.sha512` に固定し、配布元の `.sha512` と取得した zip の両方と照らす。ファイル名が `KUBO_VERSION` と合っていることは `cargo test` で確かめるので、Kubo を上げたときの更新漏れは CI より前に分かる。
- **Inno Setup は 6.7.3 に固定する**: 7.x も出ているが、7.0 は 2026 年 5 月に出たばかりのメジャーで、手元で検証に使えた ISCC も 6.7 系だった。CI の windows-latest に Inno Setup が入っているかどうか・その版はイメージの更新で変わりうるので、入っているものには頼らず、GitHub のリリースから取得して SHA-256（リリースの `digest` と一致することを確かめた値）で照らし、portable モードで一時ディレクトリに入れる。
- **ファイル名は既存の zip に揃える**: `swing-<ref>-x86_64-pc-windows-msvc-setup.exe`。release ジョブの `SHA256SUMS` にも入る。
- **確認のワークフローは分けた**: 既存の `windows-check.yml`（トレイの UI 操作）とは確かめることが重ならないので、`windows-installer-check.yml` を新しく作った。
- **スタートメニューは「SWING」（トレイ）だけ**: 「ダッシュボードを開く」の項目も考えたが、`swing.exe` はコンソールアプリなので、ショートカットから実行するとコンソールが一瞬開き、`swing up` が止まっているときはエラーが読めないまま閉じる。トレイを起動すれば、止まっていれば起動し、メニューからダッシュボードを開ける。

## 検証

- Linux 上で Wine で動かした Inno Setup 6.7 の ISCC で `swing.iss` をコンパイルし、警告もエラーも無く通ることを確かめた（`AppVersion` が `0.1.0` と `0.2.0-rc.1` の両方）。`swing.exe`・`swing-tray.exe` は中身の無いファイルで代用し、`ipfs.exe` は実物を使った（出力は約 37MB）。
- `build.ps1` を Linux の PowerShell で、ISCC の代わりに引数と `stage` の中身を出すだけのスクリプトを渡して動かし、Kubo の取得・2 つのチェックサムとの照合・改名・release の zip の展開が意図どおりになることを確かめた。
- `build.ps1` と `check-installer.ps1` が PowerShell の構文解析を通ること、actionlint が全ワークフローで通ることを確かめた。
- `cargo fmt`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通ることを確かめた。

確かめていないこと:

- Windows の実機・ランナーでの動作はどれも確かめていない（`windows-installer-check` はまだ実行していない）。とくに Pascal Script からの WMI の呼び出し（`ItemIndex`・`Terminate`）、`ExecAndCaptureOutput`・`ExecAndLogOutput` の挙動、完了画面のチェックボックスとブラウザが前面に開くこと、日本語の表示。
- 上書きで `ipfs.exe` が使用中のときの扱い。確認のワークフローはセットアップモードで動かすので Kubo が起動しない。
- winget の検証（サンドボックスでのサイレントインストール・スキャン）。マニフェストはまだ作っていない。
