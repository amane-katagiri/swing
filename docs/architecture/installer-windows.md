# Windows のインストーラー（`packaging/windows/`）

[`../architecture.md`](../architecture.md) の一部。登録するサービスとトレイは [`service.md`](service.md) と [`tray.md`](tray.md)、インストーラーを作る `windows-installer` ジョブは [`release.md`](release.md)。

Inno Setup で作るインストーラー 1 つを、GUI のインストーラーと winget の `InstallerType: inno` の両方に使う。コード署名はしていない。

| ファイル | 役割 |
|---|---|
| `swing.iss` | Inno Setup のスクリプト。英語と日本語、`UTF-8`（BOM 付き） |
| `build.ps1` | release の zip と Kubo を集めて `swing.iss` をコンパイルする |
| `kubo.sha512` | 同梱する Kubo の zip の SHA-512。`<hash>  <ファイル名>` の 1 行 |
| `check-installer.ps1` | `windows-installer-check` ワークフローの確認手順（[下記](#動作確認の-cigithubworkflowswindows-installer-checkyml)） |

## 組み立て（`build.ps1`）

```
build.ps1 -Archive <swing-<ref>-x86_64-pc-windows-msvc.zip> -OutDir <dir> -RefName <ref> [-WorkDir <dir>] [-Iscc <ISCC.exe>]
```

1. `Cargo.toml` の最初の `version = "..."` を版、`src/kubo.rs` の `KUBO_VERSION` を Kubo の版として読む。
2. Kubo の zip を取得して検証し（下記）、`ipfs.exe` と Kubo のライセンス（`LICENSE`・`LICENSE-APACHE`・`LICENSE-MIT` を `LICENSE-Kubo.txt`・`LICENSE-Kubo-APACHE.txt`・`LICENSE-Kubo-MIT.txt` に改名）を作業ディレクトリ（既定は一時ディレクトリの `swing-installer`。毎回作り直す）の `stage` に置く。
3. `-Archive` の zip の中のただ 1 つのディレクトリの中身を `stage` に足す。
4. `-Iscc` が無ければ、Inno Setup 6.7.3 のインストーラーを `jrsoftware/issrc` のリリースから取得し、固定した SHA-256 と照らしてから作業ディレクトリの `inno` に portable モード（`/PORTABLE=1 /CURRENTUSER`）で入れる。
5. `ISCC.exe /DAppVersion=<版> /DRefName=<ref> /DStageDir=<stage> /O<OutDir> swing.iss` で `swing-<ref>-x86_64-pc-windows-msvc-setup.exe` を作る。

`swing.iss` は `AppVersion`・`StageDir` が無いとコンパイルエラー。`RefName` の既定は `v<AppVersion>`。`VersionInfoVersion` は `AppVersion` の `-` より前（`0.2.0-rc.1` なら `0.2.0`）。

### Kubo の取得と検証

Kubo の GitHub のリリース（`https://github.com/ipfs/kubo/releases/download/v<版>/`）から `kubo_v<版>_windows-amd64.zip` と隣の `.sha512` を取得し（Inno Setup の取得と同じく、つながらなければ 10 秒おきに 5 回までやり直す）、次のどれかなら失敗する。

- `kubo.sha512` のファイル名が `kubo_v<KUBO_VERSION>_windows-amd64.zip` でない
- 配布元の `.sha512` の中身が `kubo.sha512` と違う
- zip の SHA-512 が `kubo.sha512` と違う

Kubo の版を上げるときの手順は [`kubo.md#kubo-のバージョン`](kubo.md#kubo-のバージョン)。インストール時には何も取得しない。

## インストーラーの設定

| 項目 | 値 |
|---|---|
| `AppId` | `{E8A9B45D-3A72-492B-905A-51911FD534C2}`（変えると上書きにならず別のアプリになる） |
| 名前・発行元 | `SWING`・`Amane Katagiri` |
| 権限 | `PrivilegesRequired=lowest`。常にユーザー単位で、UAC を出さない |
| インストール先（`{app}`） | `{autopf}\SWING`＝`%LOCALAPPDATA%\Programs\SWING`。上書きでは前回の場所。`;` か `%` を含むフォルダーは、選ぶ画面の「次へ」とサイレントの `/DIR=` の準備段階で `InvalidAppDir` を出して拒む |
| アーキテクチャ | `x64compatible` |
| 対応 OS | Windows 10 1903（`10.0.18362`）以降（タスクが使う `conhost.exe --headless` の要件） |
| 言語 | 英語（`Default.isl`）と日本語（`Japanese.isl`）。OS の表示言語で選ばれ、合わないときだけ選ぶ画面が出る |
| アイコン | `assets/swing.ico`。「アプリと機能」は `{app}\swing.exe` |
| 使用中のファイル | `CloseApplications=no`（止め方は下記） |
| 環境変数 | `ChangesEnvironment=yes` |

## 入るもの

`{app}` に release の zip の中身と Kubo を置く。

- `swing.exe`・`swing-tray.exe`・`ipfs.exe`（`ipfs.exe` は `swing.exe` の隣で見つかる。[`kubo.md`](kubo.md#バイナリの検出kubolocate_binary)）
- `swing.example.toml`・`README.md`（`docs/release/README.md`）
- `LICENSE`・`LICENSE-PixelMplus.txt`・`LICENSE-Kubo.txt`・`LICENSE-Kubo-APACHE.txt`・`LICENSE-Kubo-MIT.txt`
- `unins000.exe`・`unins000.dat`

ほかに作るもの:

- スタートメニュー（ユーザーの `Programs`）の `SWING`: `{app}\swing-tray.exe` を作業ディレクトリ `{app}` で起動する（トレイは登録済みで止まっている `swing up` を起動する。[`tray.md`](tray.md#起動したときの自動起動)）
- ユーザーの `Path`（`HKCU\Environment`、`REG_EXPAND_SZ`）の末尾に `{app}`。既にあれば（大文字小文字と末尾の `\` を無視）足さない
- 「アプリと機能」の項目（`HKCU\...\Uninstall\{E8A9B45D-...}_is1`）

## swing.exe の呼び出し方

インストーラーが実行する `swing.exe` は、作業ディレクトリ `{app}`・ウィンドウなし・終了待ちで、`dashboard open` 以外は出力をインストーラーのログに書く。`--config` は付けない（設定ファイルは通常どおり [`config.md`](config.md#設定ファイルの場所) の順で決まり、普通は `%LOCALAPPDATA%\swing\swing.toml`）。

`{app}` の `swing.exe`・`swing-tray.exe`・`ipfs.exe` のプロセスは WMI（`Win32_Process` の `ExecutablePath` の一致）で探し、別の場所の同名の実行ファイルは数えない。

### プロセスの止め方

上書きとアンインストールで共通。

1. `{app}` の `swing.exe` か `ipfs.exe` が動いていれば、`swing service stop`（[`service.md`](service.md#windowsタスクスケジューラ)。グレースフルに止まらなければ `schtasks /End`）を実行する。登録が `{app}` のものでないとき（下記）は、ダッシュボード経由でだけ止める `swing stop`（[`cli.md#stop`](cli.md#stop)）にする。
2. それらが無くなるまで最大 60 秒待ち、残っていれば強制終了して（ログに `Terminating a process of <path>`）さらに最大 10 秒待つ。`up` 以外の `swing.exe`（`swing publish` など）もここで強制終了になる。
3. `{app}` の `swing-tray.exe` は、指定の秒数だけ自分で閉じるのを待ってから強制終了する。

## 新規インストール

`{app}\swing.exe` が無ければ新規。

1. ファイルを置き、`Path` に `{app}` を足す。
2. `swing service install --no-start` を実行する。既定の場所に空の `swing.toml` ができ（[`service.md`](service.md#共通)）、タスク `swing` とトレイの Run キーの値が登録される。何も起動しない。失敗したら終了コードを添えたメッセージを出し（`/SUPPRESSMSGBOXES` なら出さない）、インストールは成功で終える。
3. 次のサインインで、セットアップモード（[`up.md`](up.md#セットアップモード鍵未設定)）で起動する。

サイレント（`/VERYSILENT /SUPPRESSMSGBOXES /NORESTART`）でも同じで、ダイアログは出ない。

### 完了画面の「SWING を起動してダッシュボードを開く」

GUI で、`service install` が成功したときだけ出す（既定はオン）。オンで完了すると:

1. `swing service start`。
2. トレイを起動する。Run キーの値 `swing-tray` の先頭の引用符付きパスが `{app}\swing-tray.exe` なら、後ろの引数（`--config "<config>"`）も付ける。そうでなければ引数なし。トレイが既に動いていれば新しいほうはすぐ終わる（[`tray.md`](tray.md#多重起動の防止)）。
3. `swing dashboard open --no-browser` を 1 秒おきに最大 60 回試し、出力の 1 行目が `http://` か `https://` で始まればそれを既定のブラウザで開く。ログインコードを含むので、この出力はログに書かない。

## 上書き（アップグレード）

`{app}\swing.exe` があれば上書き。ファイルを置く前（`PrepareToInstall`）に:

1. 前の状態を調べてログに出す（`Upgrading: task registered=…, tray registered=…, swing up running=…, tray running=…, registrations point here=…`）。
   - タスク `swing` が登録済みか（`schtasks /Query /TN swing`）、Run キーの値 `swing-tray` があるか、`{app}` のトレイが動いているか。
   - タスクが登録済みのときだけ: `{app}` の `swing.exe up` が動いているか（コマンドラインが ` up ` を含むか ` up` で終わる）と、登録が `{app}` のものか。後者は新しい `swing.exe` を `{tmp}` に取り出して `swing service status --points-into "{app}"` の終了コードが 0 かで決める（[`service/ownership.md`](service/ownership.md)。本体とトレイのどちらかが別の場所なら 0 にならない）。
2. [プロセスの止め方](#プロセスの止め方)で止める（トレイは待たずに強制終了）。

ファイルを置き `Path` を確かめた後、前の状態で分ける。

| 前の状態 | すること |
|---|---|
| タスクが未登録 | 何も登録しない |
| 登録が `{app}` のものでない | 何も登録せず、起動し直さない |
| 登録済み、Run キーの値あり | `swing service install --no-start` |
| 登録済み、Run キーの値なし | `swing service install --no-start --no-tray` |

上の 2 行では完了画面のチェックボックスも出ない。登録し直して、前に `swing up` が動いていたなら `swing service start` で起動し、トレイも動いていたなら完了画面と同じ方法で起動する（サイレントでも行う）。

## アンインストール

ファイルを消す前（`usUninstall`）に次を行う。サイレント（`unins000.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART`）でも同じ。

1. `swing service status --points-into "{app}"`（出力はログへ）の後、`swing service uninstall --only-from "{app}"` で `{app}` を指す登録だけを消す（[`service/ownership.md`](service/ownership.md)）。
2. [プロセスの止め方](#プロセスの止め方)で止める。トレイは、タスクの削除を見て自分で閉じる（[`tray.md`](tray.md#サービスの登録が消えたら終了する)）のを最大 30 秒待つ。1 で別の場所を指す登録を残したときは、`swing stop` を使い、トレイは待たずに強制終了する。
3. `Path` から `{app}` の項目を（大文字小文字と末尾の `\` を無視して）すべて除く。ほかは順序を保ち、空の項目は落とし、残りが空なら値ごと消す。

その後 Inno Setup がファイル・スタートメニュー・「アプリと機能」の項目を消す。GUI では、1 で残した登録があれば、その行（`status` の出力のうち `{app}` の下でないもの）とそのコピーの `swing service uninstall` で消せることをメッセージ（`KeptRegistrations`）で伝える。`%LOCALAPPDATA%\swing`（設定・鍵・状態・Kubo のリポジトリ・`swing.log`）は消さず、完了のメッセージ（`UninstalledAll`・`UninstalledMost`）で場所を伝える。

## 動作確認の CI（`.github/workflows/windows-installer-check.yml`）

手動実行（`workflow_dispatch`）だけ。`windows-latest` でブランチを release ビルドし、release と同じ形の zip（ref は `check`）から `build.ps1` でインストーラーを作り、`check-installer.ps1` を 4 段階で実行する。判定の正本は `check-installer.ps1`。

| 段階 | 確かめること |
|---|---|
| `install` | サイレントインストールが 0 で終わり、ファイル・`Path`（1 つだけ）・タスク・Run キーの値・空の `swing.toml`・スタートメニュー・「アプリと機能」がそろい、5 秒後も `{app}` のプロセスが無い |
| `upgrade` | `swing up` とトレイが動いている状態でのサイレントの上書きが 0 で終わり、ログの前の状態が正しく、`swing.exe` を強制終了せず、`swing up`（ダッシュボードの応答）とトレイが動き直し、登録と `Path` が残る |
| `uninstall` | （前が失敗しても行う）サイレントアンインストールが 0 で終わり、アンインストーラー（`_iu*.tmp`）が終わり、`swing.exe` を強制終了せず、登録・`Path` の項目・ファイル・スタートメニュー・プロセスが消え、`swing.toml` が残る。トレイの強制終了は記録だけ |
| `foreign` | （前が失敗しても行う）新規インストール後に `{app}` の外のコピー（`%LOCALAPPDATA%\swing-check-other-copy`）から `swing service install --no-start` する。上書きが `registrations point here=0` と判定して登録を変えず、アンインストールが残した登録をログに出してファイルだけ消す。最後にコピーの `swing service uninstall` で片付ける |

インストーラーのログ（`/LOG`）・プロセス・Run キー・`Path`・タスクの様子・`swing.log`・作ったインストーラーは artifact `windows-installer-check` に残る。入力 `ssh` は [`windows-check`](release.md#windows-の動作確認githubworkflowswindows-checkyml) と同じ。

確かめていないこと: 鍵の無いセットアップモードで動かすので、`ipfs.exe` が使用中のときの上書き。GUI（完了画面・ブラウザ・日本語）と、サインインし直したときの自動起動。
