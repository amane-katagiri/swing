# Windows のインストーラー（`packaging/windows/`）

[`../architecture.md`](../architecture.md) の一部。登録するサービスとトレイは [`service.md`](service.md) と [`tray.md`](tray.md)、リリースのワークフロー全体は [`release.md`](release.md)。

Inno Setup で作るインストーラー 1 つを、ブラウザで落として使う GUI のインストーラーと、winget の `InstallerType: inno` のインストーラーの両方に使う。コード署名はしていない。

| ファイル | 役割 |
|---|---|
| `swing.iss` | Inno Setup のスクリプト。英語と日本語、`UTF-8`（BOM 付き） |
| `build.ps1` | release の zip と Kubo を集めて `swing.iss` をコンパイルする |
| `kubo.sha512` | 同梱する Kubo の zip の SHA-512。dist.ipfs.tech の `.sha512` と同じ `<hash>  <ファイル名>` の 1 行 |
| `check-installer.ps1` | `windows-installer-check` ワークフローの確認手順（下記） |

## 組み立て（`build.ps1`）

```
build.ps1 -Archive <swing-<ref>-x86_64-pc-windows-msvc.zip> -OutDir <dir> -RefName <ref> [-WorkDir <dir>] [-Iscc <ISCC.exe>]
```

1. `Cargo.toml` の最初の `version = "..."` を版、`src/kubo.rs` の `KUBO_VERSION` を Kubo の版として読む。
2. Kubo の zip を取得して検証する（下記）。`ipfs.exe` と、`LICENSE`・`LICENSE-APACHE`・`LICENSE-MIT` を `LICENSE-Kubo.txt`・`LICENSE-Kubo-APACHE.txt`・`LICENSE-Kubo-MIT.txt` に改名して作業ディレクトリ（既定は一時ディレクトリの `swing-installer`、毎回作り直す）の `stage` に置く。
3. `-Archive` の zip を展開し、中のただ 1 つのディレクトリの中身を `stage` に足す。
4. `-Iscc` が無ければ Inno Setup 6.7.3 のインストーラーを GitHub の `jrsoftware/issrc` のリリースから取得し、スクリプトに固定した SHA-256 と照らしてから、作業ディレクトリの `inno` に portable モード（`/PORTABLE=1 /CURRENTUSER`）で入れる。レジストリやスタートメニューには何も残さない。
5. `ISCC.exe /DAppVersion=<版> /DRefName=<ref> /DStageDir=<stage> /O<OutDir> swing.iss` でコンパイルする。出力は `swing-<ref>-x86_64-pc-windows-msvc-setup.exe`。

`swing.iss` は `AppVersion`・`StageDir` が無いとコンパイルエラーにする。`RefName` は省略すると `v<AppVersion>`。ファイルのバージョン情報（`VersionInfoVersion`）には `AppVersion` の `-` より前（`0.2.0-rc.1` なら `0.2.0`）を使う。

### Kubo の取得と検証

`https://dist.ipfs.tech/kubo/v<版>/kubo_v<版>_windows-amd64.zip` と、隣の `.sha512` を取得し、次のどれかに当たれば失敗にする。

- `kubo.sha512` のファイル名が `kubo_v<KUBO_VERSION>_windows-amd64.zip` でない（`kubo::tests` の `windows_installer_pins_the_same_kubo_version` も同じことを確かめる）
- 配布元の `.sha512` の中身が `kubo.sha512` と違う
- 取得した zip の SHA-512 が `kubo.sha512` と違う

インストール時に外から何かを取得することは無い。

## インストーラーの設定

| 項目 | 値 |
|---|---|
| `AppId` | `{E8A9B45D-3A72-492B-905A-51911FD534C2}`（変えると別のアプリとして扱われ、上書きにならない） |
| 名前・発行元 | `SWING`・`Amane Katagiri` |
| 権限 | `PrivilegesRequired=lowest`。UAC を出さず、常にユーザー単位のインストールになる |
| インストール先 | `{autopf}\SWING`。ユーザー単位なので `%LOCALAPPDATA%\Programs\SWING`。上書きのときは前回の場所 |
| アーキテクチャ | `x64compatible`（64 ビットモード） |
| 対応 OS | Windows 10 1903（`10.0.18362`）以降。タスクが使う `conhost.exe --headless` がそれより前に無いため |
| 言語 | 英語（`Default.isl`）と日本語（`Japanese.isl`）。OS の表示言語で選ばれ、選ぶ画面は合わないときだけ出る |
| アイコン | `assets/swing.ico`。「アプリと機能」のアイコンは `{app}\swing.exe` |
| 使用中のファイル | `CloseApplications=no`（Restart Manager は使わない。止め方は下記） |
| 環境変数 | `ChangesEnvironment=yes`。インストールとアンインストールの最後に環境変数の変更を通知する |

## 入るもの

`{app}`（インストール先）に次を置く。release の zip の中身に `ipfs.exe` と Kubo のライセンスを足したもの。

- `swing.exe`・`swing-tray.exe`・`ipfs.exe`（`kubo::locate_binary` が `swing.exe` の隣に見つける。[`kubo.md`](kubo.md)）
- `swing.example.toml`・`README.md`（`docs/release/README.md`）
- `LICENSE`・`LICENSE-PixelMplus.txt`・`LICENSE-Kubo.txt`・`LICENSE-Kubo-APACHE.txt`・`LICENSE-Kubo-MIT.txt`
- `unins000.exe`・`unins000.dat`（Inno Setup のアンインストーラー）

ほかに次を作る。

- スタートメニュー（ユーザーの `Programs`）の `SWING`: `{app}\swing-tray.exe` を作業ディレクトリ `{app}` で起動する。トレイは起動時に、登録済みで止まっている `swing up` を起動する（[`tray.md`](tray.md#起動したときの自動起動)）
- ユーザーの環境変数 `Path`（`HKCU\Environment`）の末尾に `{app}`。既にあれば（大文字小文字と末尾の `\` を無視して比べる）足さない。値は `REG_EXPAND_SZ` で書く
- 「アプリと機能」の項目（`HKCU\...\Uninstall\{E8A9B45D-...}_is1`）

## swing.exe の呼び出し方

インストーラーが実行する `swing.exe` は、どれも作業ディレクトリを `{app}` にし、ウィンドウを出さず、終わるまで待つ（`dashboard open` 以外は出力をインストーラーのログに書く）。`--config` は付けないので、設定ファイルは `SWING_CONFIG` → `{app}\swing.toml`（あれば）→ ユーザーごとの既定の場所（`%LOCALAPPDATA%\swing\swing.toml`）の順で決まる（[`config.md`](config.md#設定ファイルの場所)）。

`{app}` の `swing.exe`・`swing-tray.exe`・`ipfs.exe` のプロセスは WMI（`Win32_Process` の `ExecutablePath` が一致するもの）で探す。同じ名前でも別の場所の実行ファイルは数えず、止めない。

## 新規インストール

`{app}\swing.exe` が無ければ新規として扱う。

1. ファイルを置く。
2. `Path` に `{app}` を足す。
3. `swing service install --no-start` を実行する。設定ファイルが既定の場所に決まり、まだ無ければ空の `swing.toml` を作り（[`service.md`](service.md#共通)）、タスク `swing` とトレイの Run キーの値を登録する。何も起動しないので、Kubo が 4001 で待ち受けてファイアウォールのダイアログが出ることもない。失敗したら「サインイン時に起動する登録に失敗した」旨と終了コードを出し（`/SUPPRESSMSGBOXES` なら出さない）、インストール自体は成功で終える。
4. 次のサインインでタスクとトレイが起動し、空の設定ファイルなのでセットアップモード（[`up.md`](up.md#セットアップモード鍵未設定)）で動く。

サイレントインストール（`/VERYSILENT /SUPPRESSMSGBOXES /NORESTART`）でも同じで、ダイアログは出ない。

## 完了画面の「SWING を起動してダッシュボードを開く」

GUI のときだけ完了画面にチェックボックスを出す（既定はオン。サイレントでは出さず、何もしない）。上の 3 が成功したときだけ出す（上書きで登録しなかったときは出ない）。オンで完了すると次を行う。

1. `swing service start`（タスクの即時起動）。
2. トレイを起動する。Run キーの値 `swing-tray` の先頭の引用符で囲んだパスが `{app}\swing-tray.exe` と一致すれば、その後ろの引数（`--config "<config>"`）も付ける。一致しない・値が無いときは引数なしで起動する。同じ `state_dir` のトレイが既に動いていれば、新しいほうはすぐ終わる（[`tray.md`](tray.md#多重起動の防止)）。
3. `swing dashboard open --no-browser` を 1 秒おきに最大 60 回試し、成功したら出力の 1 行目（ログインリンク。`http://` か `https://` で始まるときだけ）を既定のブラウザで開く。リンクにはログインコードが入るので、この出力はログに書かない。60 回とも失敗したら何もしない。

## 上書き（アップグレード）

`{app}\swing.exe` があれば上書きとして扱い、ファイルを置く前（Inno Setup の `PrepareToInstall`）に次を行う。

1. 前の状態を覚える: タスク `swing` が登録済みか（`schtasks /Query /TN swing` の終了コード）、Run キーに値 `swing-tray` があるか、`{app}\swing.exe` の `up` が動いているか（タスクが登録済みのときだけ数える。コマンドラインが ` up ` を含むか ` up` で終わる）、`{app}\swing-tray.exe` が動いているか。タスクが登録済みなら、登録が `{app}` のものか（下記）。ログに `Upgrading: task registered=…, tray registered=…, swing up running=…, tray running=…, registrations point here=…` と出す。
   - 登録が `{app}` のものかは、これから入れる新しい `swing.exe` を `ExtractTemporaryFile` で一時ディレクトリ（`{tmp}`）に取り出し、`{tmp}\swing.exe service status --points-into "{app}"` の終了コードが 0 かどうかで決める（[`service/ownership.md`](service/ownership.md)。タスクとトレイの登録のどちらか一方でも別の場所を指していれば 0 にならない）。`{app}` にある古い `swing.exe` はこのオプションを知らないことがあるので使わない。出力はログに書く。
2. `{app}` の `swing.exe` か `ipfs.exe` が動いていれば、`swing service stop`（登録が `{app}` のものでないときは `swing stop`。下記）（ダッシュボード経由のグレースフルな停止。失敗したら `schtasks /End` で conhost を終わらせ、`--exit-with-parent` でグレースフルに止まる。[`service.md`](service.md#windowsタスクスケジューラ)）を実行し、`{app}` の `swing.exe`・`ipfs.exe` が無くなるまで最大 60 秒待つ。残っていれば強制終了し（ログに `Terminating a process of <path>`）、さらに最大 10 秒待つ。`swing publish` など `up` 以外の `swing.exe` も、この時点で残っていれば強制終了になる。登録が別の場所のものなら、`schtasks /End` に落ちて別のコピーのタスクを止めてしまわないよう、ダッシュボード経由でだけ止める `swing stop`（[`cli.md#stop`](cli.md#stop)）を使う。止めるのはどちらも `{app}` のプロセスが動いているときだけ。
3. `{app}` の `swing-tray.exe` を強制終了する（API のクライアントでしかないので、止め方による害は無い）。

ファイルを置いた後は、新規と同じく `Path` を確かめてから、前の状態で分ける。

| 前の状態 | すること |
|---|---|
| タスクが未登録 | `service install` を実行しない（利用者が `swing service uninstall` した状態を保つ）。完了画面のチェックボックスも出ない |
| タスクが登録済みだが、登録が `{app}` のものでない | 何もしない（別の場所から登録した `swing` を `{app}` に付け替えない）。完了画面のチェックボックスも出ず、起動し直しもしない |
| タスクが登録済み、Run キーの値あり | `swing service install --no-start` |
| タスクが登録済み、Run キーの値なし | `swing service install --no-start --no-tray`（`--no-tray` で登録した状態を保つ） |

登録できて、しかも 1 で `swing up` が動いていたなら、`swing service start` で起動し直し、トレイも動いていたならトレイも上記と同じ方法で起動し直す（サイレントでも行う）。動いていなかったものは起動しない。

## アンインストール

ファイルを消す前（`usUninstall`）に次を行う。サイレントアンインストール（`unins000.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART`）でも同じで、確認は出ない。

1. `swing service status --points-into "{app}"` で登録を確かめ（出力はログに書く）、続けて `swing service uninstall --only-from "{app}"` を実行する。`{app}` の下を指す登録だけ（タスクと Run キーの値を別々に判定する）を消し、タスクを消すときは `swing up` をグレースフルに止めてから消す。別の場所を指す登録は残す（[`service/ownership.md`](service/ownership.md)）。
2. `{app}` の `swing.exe`・`ipfs.exe` が残っていれば、上書きの 2 と同じく止める（1 で残した登録があれば `swing stop`）。
3. `{app}` の `swing-tray.exe` は、タスクの削除を見て自分で閉じる（[`tray.md`](tray.md#サービスの登録が消えたら終了する)）のを最大 30 秒待ち、残っていれば強制終了する。1 で残した登録があればタスクが消えないこともあるので、待たずに強制終了する。
4. `Path` から `{app}` の項目を（大文字小文字と末尾の `\` を無視して）すべて取り除く。ほかの項目は順序を保ち、空の項目は落とす。残りが空なら値ごと消す。Inno Setup の環境変数の通知はファイルを消す処理の中で行われるので、`Path` の書き換えはその前のこの段階で行う。

その後、Inno Setup がインストールしたファイル・スタートメニューの項目・「アプリと機能」の項目を消す。1 で残した登録があれば、GUI のときだけ（`UninstallSilent` でないとき）完了のメッセージの前に、残した登録の行（`swing service status` の出力のうち `{app}` の下でないもの）と、そのコピーの `swing service uninstall` で消せることを伝えるメッセージ（`KeptRegistrations`）を出す。`%LOCALAPPDATA%\swing`（設定ファイル・鍵・状態ファイル・Kubo のリポジトリ・`swing.log`）は消さず、完了のメッセージ（`UninstalledAll`・`UninstalledMost`）でその場所（ユーザーフォルダーの `AppData\Local\swing`）を伝える。

## 動作確認の CI（`.github/workflows/windows-installer-check.yml`）

手動実行（`workflow_dispatch`）でだけ動く。`windows-latest` のランナーでブランチを release ビルドし、release と同じ形の zip（ref は `check`）を作り、`build.ps1` でインストーラーを作って、`check-installer.ps1` を 4 段階で実行する。手順と判定の正本は `check-installer.ps1`。

| 段階 | 確かめること |
|---|---|
| `install` | 事前に何も無いこと。サイレントインストールの終了コードが 0、上記のファイルがすべてあること、`Path` に `{app}` が 1 つだけあること、タスクの登録、Run キーの値が `{app}\swing-tray.exe` を指すこと、既定の場所に空の `swing.toml` ができること、スタートメニューの項目と「アプリと機能」の項目、5 秒待っても `{app}` のプロセスが何も動いていないこと |
| `upgrade` | `swing service start` とトレイを起動して動いている状態で、同じインストーラーをもう一度サイレントで実行する。終了コードが 0、ログで前の状態（登録が `{app}` のものであることを含む）を正しく判定したこと、`swing.exe` を強制終了していないこと、`swing up` が別のプロセスとして動き直してダッシュボードが応答すること、トレイが動き直すこと、タスク・Run キーの値・`Path`（1 つだけ）が残ること |
| `uninstall` | 前の段階が失敗しても行う。サイレントアンインストールの終了コードが 0、アンインストーラーの本体（一時ディレクトリの `_iu*.tmp`）が終わり「アプリと機能」の項目が消えること、`swing.exe` を強制終了していないこと、タスク・Run キーの値・`Path` の項目・ファイル・スタートメニューの項目が消えること、`{app}` のプロセスが残っていないこと、`swing.toml` が残ること。トレイを強制終了したかどうかは記録するだけで判定しない |
| `foreign` | 前の段階が失敗しても行う。新規にインストールしてから、`swing.exe` と `swing-tray.exe` を `{app}` の外（`%LOCALAPPDATA%\swing-check-other-copy`）に写し、そこから `swing service install --no-start` してタスクと Run キーの値をそちらに向ける。同じインストーラーでの上書きが登録を `{app}` のものでないと判定して（ログの `registrations point here=0`）タスクと Run キーの値を変えないこと、サイレントアンインストールが残した登録をログに出し、タスクと Run キーの値がそのコピーを指したまま残り、ファイルは消えること。最後にそのコピーの `swing service uninstall` で登録を消してコピーを削除し、消えたことを確かめる |

各段階のインストーラーのログ（`/LOG`）、プロセス・Run キー・`Path`・タスクの様子、`swing.log` と、作ったインストーラーは artifact `windows-installer-check` に残る。入力 `ssh` は `windows-check` と同じ。

鍵の無いセットアップモードで動かすので Kubo は起動せず、上書きで `ipfs.exe` が使用中のときの扱いはこの CI では確かめていない。GUI（完了画面の起動とブラウザでダッシュボードが開くこと、日本語の表示）とサインインし直したときの自動起動も、ランナーでは確かめない。
