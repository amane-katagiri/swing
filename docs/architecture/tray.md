# タスクトレイ（`swing-tray`）

[`../architecture.md`](../architecture.md) の一部。操作する相手は [`up.md`](up.md) の `swing up`、使う API は [`dashboard/http-api.md`](dashboard/http-api.md)。配布は [`release.md`](release.md)、ログイン時の自動起動とサービス登録は [`service.md#タスクトレイの自動起動windows-と-macos`](service.md#タスクトレイの自動起動windows-と-macos) を参照。

```
swing-tray [--config <path>]
```

`tray/` にある別クレート（workspace のメンバー、バイナリ名 `swing-tray`）。Windows の通知領域と macOS のメニューバーにアイコンを出し、動いている `swing up` をメニューから操作する。`swing up` とは別のプロセスで、CLI の `swing status` や `swing stop` と同じく、ダッシュボード API のクライアントとして動く（`swing` クレートの `api_client::ApiClient`・`config::Config`・`login`・`service`・`stop` を使う）。

- **対応 OS**: Windows と macOS だけ。GUI の依存（`tray-icon`・`tao`・`png`・`sys-locale`・`rfd`）は `cfg(any(windows, target_os = "macos"))` の target 依存にしてあり、他の OS では「swing-tray supports only Windows and macOS」を出して終了コード 1 で終わるだけのバイナリになる。
- **Windows**: `windows_subsystem = "windows"` の GUI アプリなので、起動してもコンソールウィンドウが開かない。標準出力・標準エラーはどこにも出ない。
- **macOS**: `SWING.app` バンドル（下記）の中に入れて配る。activation policy を `Accessory` にし、`Info.plist` にも `LSUIElement` を入れて、Dock にアイコンを出さない。
- **言語**: メニューとダイアログの文言は OS のロケール（`sys_locale::get_locale()`）が `ja` で始まれば日本語、それ以外は英語。

## 設定ファイル

`--config <path>` を渡すか、渡さなければ他のサブコマンドと同じく `config::resolve_config_path`（`SWING_CONFIG` → カレントディレクトリの `swing.toml`）で決める。ポーリングと操作のたびに `Config::load` で読み直すので、ダッシュボードの設定画面で `[dashboard].listen` を変えて再起動しても、そのまま追いかけられる。

## 多重起動の防止

起動時に設定を読めたら、`<state_dir>/swing-tray.lock` を `File::try_lock` でロックする。ほかの `swing-tray` が同じ `state_dir` でロックしていれば、何も出さずに終了コード 0 で終わる。設定が読めないときや `state_dir` がまだ無いときはロックせずに起動する（`state_dir` は作らない）。ロックはプロセスが終わるまで持ち続ける。

## メニュー

| 項目 | 押したとき | 使える条件 |
|---|---|---|
| 状態の表示 | —（常に無効） | — |
| ダッシュボードを開く | `login::request_link`（`POST /api/login-code`、`swing dashboard open` と同じ URL の組み立て）→ `login::open_browser` | 動作中で、`[dashboard].ui = true` |
| 再起動 | `POST /api/restart`（プロセス内の再起動） | 動作中 |
| 停止 | 確認のダイアログ（はい / いいえ）を出し、「はい」なら `POST /api/shutdown` | 動作中 |
| 起動 | `service::start(false)`。`swing service start`（[`service.md`](service.md)）と同じ関数を呼ぶ。サービスとして登録していない `swing up` は、トレイからは起動できない | 停止中で、サービスとして未登録と確認できていない（登録済み、または登録状態が分からない） |
| 終了 | トレイを閉じる。動作中なら、SWING も止めるかをダイアログ（はい / いいえ / キャンセル）で聞く（下記） | 「終了」で SWING が止まるのを待っている間でなければ常に |

「再起動」「停止」「起動」「終了（はい）」を押してから状態が変わりきるまでの間の表示と無効にする項目は、下記「[操作の途中の表示](#操作の途中の表示)」。

### 終了

- 停止中やエラーのときは、確認せずにすぐ閉じる（閉じても止まるものが無いため）。
- 動作中なら「SWING を停止してからトレイを閉じますか？」と聞き、「いいえ」ならトレイだけを閉じることを添える。ボタンの文字は OS 標準の「はい」「いいえ」「キャンセル」のまま。
  - **はい**: `stop::run(&config, false, 90 秒)`（`swing stop` と同じ。`POST /api/shutdown` を送り、API に接続できなくなるまで待つ）を呼び、止まったらトレイを閉じる。止められなかったら、エラーを表示してトレイは残す。
  - **いいえ**: トレイだけを閉じる。`swing up` は動き続ける。
  - **キャンセル**: 何もしない。

停止した `swing up` は、サービスの仕組みによって自動では立ち上がらない（[`service.md`](service.md)）。

### 確認のダイアログ

`rfd::MessageDialog` を別スレッドから出すので、ダイアログを開いている間もイベントループは止まらない。ダイアログを開いている間に、もう一度「停止」や「終了」を押しても、2 つ目は出さない。

## 起動したときの自動起動

起動して最初の状態の確認で、`swing up` が止まっていて、しかもサービスとして登録済みと確認できたなら（登録状態が分からないときは起動しない）、`service::start(false)` で起動する。表示は下記の「起動しています…」になる。最初の 1 回だけ行い、その後に止まっても起動しない。ログイン時は、サービス（Windows のログオン時トリガー、launchd の `RunAtLoad`）とトレイがほぼ同時に起動するので、両方から起動することがある。その場合も、Windows はタスクの `MultipleInstancesPolicy = IgnoreNew`、macOS は `kickstart`（`-k` なし）が 2 つ目を起こさない。万一 2 つ目が立ち上がっても `swing.lock`（[`up.md`](up.md#多重起動の防止lockrs)）ですぐ終わる。

## 状態の表示

別スレッドの tokio ランタイム（`worker::run`）が `GET /api/overview` を叩き（5 秒で返らなければエラー扱い）、結果をイベントループへ送る。間隔はふだん 5 秒で、操作をしてから（自動起動を含む）90 秒間は 1 秒にする。

| 状態 | 条件 | 表示 | アイコン |
|---|---|---|---|
| 動作中 | `/api/overview` が返った | `SWING: 動作中`。`setup: true` なら `セットアップ待ち`、`signer.last_failure`（NIP-46 の署名アプリへの最後のリクエストが、時間切れ・拒否・接続できないなどで失敗した。次に成功すると消える。[`signer.md`](signer.md)）があれば `動作中（前回の署名に失敗しました）` | ロゴ |
| 停止中 | API に接続できない（`ApiClientError::Unreachable`） | `SWING: 停止中`。サービスとして未登録と確認できたときだけ `停止中（サービス未登録）` | 灰色で半透明のロゴ（macOS は薄いロゴ。下記「アイコン」） |
| エラー | 設定を読めない、トークンが合わない（401）、応答が無いなど | `SWING: エラー: <メッセージの 1 行目>` | 灰色で半透明のロゴ（macOS は薄いロゴ。下記「アイコン」） |

サービスとして登録済みかどうか（`service::is_installed`）は、状態の確認のたびに、設定ファイルを読む前に調べる（設定ファイルが読めなくても下記の判定が狂わないように）。結果は「登録済み／未登録／分からない」の三値で、分からないときも含めて 10 秒覚えておく（`schtasks` や `launchctl` を起動するため、1 秒ごとには呼ばない）。Windows の判定と、`schtasks` の 10 秒のタイムアウトは [`service.md`](service.md) の「Windows」。`spawn_blocking` の join に失敗したときも分からないとする。操作に失敗したら、状態の行を `操作に失敗しました: <メッセージ>` に 15 秒間差し替える。次の操作が成功したら元に戻す。ツールチップにも同じ文字列を出す。メッセージは 1 行目だけにし、80 文字を超えるときは先頭 79 文字に `…` を付けた 80 文字にする。

### 操作の途中の表示

「起動」「再起動」「停止」「終了（はい）」を押したら、API の応答や次の状態の確認を待たずに、すぐ表示を切り替える（`status::Pending`）。

| 操作 | 表示 | 表示を戻す条件 |
|---|---|---|
| 起動（自動起動を含む） | `起動しています…` | 動作中を確認した |
| 再起動 | `再起動しています…` | 押す前と違う `instance`（`/api/overview`）で動作中を確認した。再起動の途中で API に一時的につながらなくなっても、停止中とは表示しない |
| 停止 | `停止しています…` | 停止中を確認した |
| 終了（はい） | `停止しています…` | 止まってトレイが閉じるか、止めるのに失敗した |

- 途中の間は「終了」以外の項目をすべて無効にする（「終了（はい）」の途中は「終了」も無効）。逆向きの操作（止めている途中の「起動」など）は、状態が変わりきったのを確認してから有効にする。
- 操作が失敗したら（API が 4xx/5xx を返した、`schtasks` が失敗したなど）、途中の表示をやめて失敗を表示する。
- 120 秒たっても状態が変わらなければ、途中の表示をやめて、確認できた状態に戻す（「終了（はい）」は `stop::run` の 90 秒のタイムアウトで失敗になる）。

状態から表示・使える項目・アイコンを決める部分と、途中の表示を戻す条件は、`status.rs` の純粋関数（`Status::from_overview`・`menu_state`・`Pending::settled_by`）にしてある。OS に依存しないユニットテストがある。

## サービスの登録が消えたら終了する

`status::RegistrationWatch` は `swing` 本体のサービス登録（`service::is_installed`。Windows はタスク `swing`、macOS は `jp.ne.ama.swing.plist`）を見る。一度でも登録済み（`Some(true)`）と確認した後で、未登録（`Some(false)`）と確認したら、`swing service uninstall` されたとみなしてトレイを閉じる。分からない（`None`。Windows で `schtasks` が起動できない・失敗した・時間切れになった）ときは無視して前の状態を保つので、`schtasks` の一時的な失敗ではトレイは閉じない。最初から登録が無いまま手で起動したトレイは、この判定では閉じない。

- Windows の `uninstall` はトレイのプロセスには触らない。タスクの削除から最大で約 15 秒（登録確認のキャッシュ 10 秒 + ポーリング間隔 5 秒）でトレイが閉じる。`install --no-tray` でトレイの自動起動の登録（Run キーの値）だけを消しても、タスクは残るので動いているトレイは閉じない。
- macOS の `uninstall` はトレイの LaunchAgent を `bootout` するので、その時点でトレイも止まる。

## アイコン

OS ごとに 1 枚の PNG をバイナリに埋め込む。停止中とエラーのときに使う版は、起動時にこの画像から作る（輝度に変換し、アルファを半分にする）。

- **Windows**: `tray/assets/icon-64.png`（`web/favicon.svg` を 64×64 に書き出したもの）。停止中は灰色で半透明になる。
- **macOS**: `tray/assets/icon-template-64.png`（`web/favicon.svg` の図形をすべて黒で塗り、64×64 に書き出したもの）を、テンプレート画像として出す。メニューバーの色に合わせて macOS が白か黒で描く。停止中はアルファが半分になり、薄く表示される。

`swing-tray.exe` のファイルアイコンは `tray/assets/swing-tray.ico`（元図は `tray/assets/swing-tray.svg`。[`release.md`](release.md)）。macOS の `SWING.app` のアイコンは `tray/assets/SWING.icns`（`web/favicon.svg` を 1024px の正方形の中央に 824px で置き、16〜1024px を格納したもの）。

## macOS のアプリバンドル（`SWING.app`）

`tray/macos/bundle.sh <swing-tray> <出力先ディレクトリ>` が `<出力先>/SWING.app` を作る（あれば作り直す）。release と `macos-check` のワークフローがこれを使う。`cargo build` だけでは作られないので、手元でビルドした `swing-tray` を `service install` に登録させるときも、このスクリプトで `swing` の隣に `SWING.app` を置く。

```
SWING.app/Contents/
  Info.plist            tray/macos/Info.plist の @VERSION@ を tray/Cargo.toml の version に置き換えたもの
  MacOS/swing-tray
  Resources/SWING.icns
```

| `Info.plist` のキー | 値 |
|---|---|
| `CFBundleIdentifier` | `jp.ne.ama.swing`（LaunchAgent の `AssociatedBundleIdentifiers` と同じ。[`service.md`](service.md)） |
| `CFBundleName`・`CFBundleDisplayName` | `SWING` |
| `CFBundleExecutable` | `swing-tray` |
| `CFBundleIconFile` | `SWING` |
| `LSUIElement` | `true` |
| `LSMinimumSystemVersion` | `11.0` |

`codesign` があれば（macOS なら）バンドル全体に ad-hoc 署名（`codesign --force --sign -`）をする。開発者 ID の署名と公証はしていない。

手で起動するときは `SWING.app/Contents/MacOS/swing-tray --config <path>` を実行する（`open SWING.app` では `--config` を渡せず、カレントディレクトリも `/` になる）。
