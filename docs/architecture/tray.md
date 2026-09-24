# タスクトレイ（`swing-tray`）

[`../architecture.md`](../architecture.md) の一部。操作する相手は [`up.md`](up.md) の `swing up`、使う API は [`dashboard/http-api.md`](dashboard/http-api.md)。

```
swing-tray [--config <path>]
```

`tray/` にある別クレート（workspace のメンバー、バイナリ名 `swing-tray`）。Windows の通知領域と macOS のメニューバーにアイコンを出し、動いている `swing up` をメニューから操作する。`swing up` とは別のプロセスで、CLI の `swing status` や `swing stop` と同じく、ダッシュボード API のクライアントとして動く（`swing` クレートの `api_client::ApiClient`・`config::Config`・`login`・`service` を使う）。

- **対応 OS**: Windows と macOS だけ。GUI の依存（`tray-icon`・`tao`・`png`・`sys-locale`・`rfd`）は `cfg(any(windows, target_os = "macos"))` の target 依存にしてあり（`rfd` は `default-features = false`。既定の機能は Linux 用のものだけ）、他の OS では「swing-tray supports only Windows and macOS」を出して終了コード 1 で終わるだけのバイナリになる。Linux の musl ビルドや `cargo clippy --workspace` には GTK などが要らない。
- **Windows**: `windows_subsystem = "windows" の GUI アプリなので、起動してもコンソールウィンドウが開かない。標準出力・標準エラーはどこにも出ない。
- **macOS**: activation policy を `Accessory` にして、Dock にアイコンを出さない。

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
| 起動 | `service::start(false)`（下記） | 停止中で、サービスとして登録済み |
| 終了 | トレイを閉じる。動作中なら、SWING も止めるかをダイアログ（はい / いいえ / キャンセル）で聞く（下記） | 「終了」で SWING が止まるのを待っている間でなければ常に |

「再起動」「停止」「起動」「終了」は、どれもすぐには終わらない。「使える条件」に加えて、押してから状態が変わりきるまでの間（下記「操作の途中の表示」）は、「終了」以外の項目をすべて無効にする。

### 終了

- 停止中やエラーのときは、確認せずにすぐ閉じる（閉じても止まるものが無いため）。
- 動作中なら「SWING を停止してからトレイを閉じますか？」と聞き、「いいえ」ならトレイだけを閉じることを添える。ボタンの文字は OS 標準の「はい」「いいえ」「キャンセル」のまま（Windows でボタンの文字を変えるには、`TaskDialogIndirect` を使うためのマニフェストを exe に埋め込む必要があるため）。
  - **はい**: `stop::run(&config, false, 90 秒)`（`swing stop` と同じ。`POST /api/shutdown` を送り、API に接続できなくなるまで待つ）を呼び、止まったらトレイを閉じる。待っている間は「停止」と同じく「停止しています…」と表示し、「終了」も含めてすべての項目を無効にする。止められなかったら、エラーを表示してトレイは残す。
  - **いいえ**: トレイだけを閉じる。`swing up` は動き続ける。
  - **キャンセル**: 何もしない。

停止した `swing up` は、サービスの仕組みによって自動では立ち上がらない（systemd の `Restart=on-failure`、launchd の `KeepAlive = { SuccessfulExit = false }`、Windows のログオン時トリガー。[`service.md`](service.md)）。

### 確認のダイアログ

`rfd::MessageDialog`（Windows は `MessageBoxW`、macOS は `NSAlert`）で出す。別スレッドから出して、答えを `EventLoopProxy` で送り返すので、ダイアログを開いている間もイベントループは止まらない。ダイアログを開いている間に、もう一度「停止」や「終了」を押しても、2 つ目は出さない。

メニューとダイアログの文言は OS のロケール（`sys_locale::get_locale()`）が `ja` で始まれば日本語、それ以外は英語。

## 起動したときの自動起動

起動して最初の状態の確認で、`swing up` が止まっていて、しかもサービスとして登録済みなら、`service::start(false)` で起動する。表示は下記の「起動しています…」になる。最初の 1 回だけ行い、その後に止まっても起動しない。ログイン時は、サービス（Windows のログオン時トリガー、launchd の `RunAtLoad`）とトレイがほぼ同時に起動するので、両方から起動することがある。その場合も、Windows はタスクの `MultipleInstancesPolicy = IgnoreNew`、macOS は `kickstart`（`-k` なし）が 2 つ目を起こさない。万一 2 つ目が立ち上がっても `swing.lock`（[`up.md`](up.md#多重起動の防止lockrs)）ですぐ終わる。

## 状態の表示

別スレッドの tokio ランタイム（`worker::run`）が `GET /api/overview` を叩き（5 秒で返らなければエラー扱い）、結果をイベントループへ送る。間隔はふだん 5 秒で、操作をしてから（自動起動を含む）90 秒間は 1 秒にする。

| 状態 | 条件 | 表示 | アイコン |
|---|---|---|---|
| 動作中 | `/api/overview` が返った | `SWING: 動作中`。`setup: true` なら `セットアップ待ち`、`signer.last_failure`（NIP-46 の署名アプリへの最後のリクエストが、時間切れ・拒否・接続できないなどで失敗した。次に成功すると消える。[`signer.md`](signer.md)）があれば `動作中（前回の署名に失敗しました）` | ロゴ |
| 停止中 | API に接続できない（`ApiClientError::Unreachable`） | `SWING: 停止中`。サービスとして登録されていなければ `停止中（サービス未登録）` | 灰色で半透明のロゴ |
| エラー | 設定を読めない、トークンが合わない（401）、応答が無いなど | `SWING: エラー: <メッセージの 1 行目>` | 灰色で半透明のロゴ |

サービスとして登録済みかどうか（`service::is_installed`）は、状態の確認のたびに、設定ファイルを読む前に調べる（設定ファイルが読めなくても下記の判定が狂わないように）。結果は 10 秒覚えておく（`schtasks` や `launchctl` を起動するため、1 秒ごとには呼ばない）。操作に失敗したら、状態の行を `操作に失敗しました: <メッセージ>` に 15 秒間差し替える。次の操作が成功したら元に戻す。ツールチップにも同じ文字列を出す。メッセージは 1 行目だけにして、80 文字を超える分は `…` で切る。

### 操作の途中の表示

「起動」「再起動」「停止」「終了（はい）」を押したら、API の応答や次の状態の確認を待たずに、すぐ表示を切り替える（`status::Pending`）。

| 操作 | 表示 | 表示を戻す条件 |
|---|---|---|
| 起動（自動起動を含む） | `起動しています…` | 動作中を確認した |
| 再起動 | `再起動しています…` | 押す前と違う `instance`（`/api/overview`）で動作中を確認した。再起動の途中で API に一時的につながらなくなっても、停止中とは表示しない |
| 停止 | `停止しています…` | 停止中を確認した |
| 終了（はい） | `停止しています…` | 止まってトレイが閉じるか、止めるのに失敗した |

- 途中の間は「終了」以外の項目をすべて無効にする。止めている途中に「起動」を押すと、サービス側は動いているプロセスがあるとして何もしない（タスクの `IgnoreNew` など）。その後で止まりきると、結局止まったままになる。そのため、逆向きの操作は、状態が変わりきったのを確認してから有効にする。
- 操作が失敗したら（API が 4xx/5xx を返した、`schtasks` が失敗したなど）、途中の表示をやめて失敗を表示する。
- 120 秒たっても状態が変わらなければ、途中の表示をやめて、確認できた状態に戻す（「終了（はい）」は `stop::run` の 90 秒のタイムアウトで失敗になる）。

状態から表示・使える項目・アイコンを決める部分と、途中の表示を戻す条件は、`status.rs` の純粋関数（`Status::from_overview`・`menu_state`・`Pending::settled_by`）にしてある。OS に依存しないユニットテストがある。

### サービスの登録が消えたら終了する

一度でも登録済みと確認した後で、登録が無いと確認したら、`swing service uninstall` されたとみなしてトレイを閉じる（`status::RegistrationWatch`）。Windows の `uninstall` は Run キーの値を消すだけで、動いているトレイのプロセスには触らないため。登録を消した後、10 秒ほどでトレイが消える。外から `TerminateProcess` で止めると、通知領域のアイコンがマウスを乗せるまで残るので、トレイ自身に閉じさせている。最初から登録が無いまま手で起動したトレイは、この判定では閉じない。macOS の `uninstall` はトレイの LaunchAgent を `bootout` するので、その時点でトレイも止まる。

## 起動（`service::start`）

`swing service start`（[`service.md`](service.md)）と同じ関数を呼ぶ。サービスとして登録していない `swing up` は、トレイからは起動できない。

## アイコン

`tray/assets/icon-64.png`（`web/favicon.svg` を 64×64 に書き出したもの）をバイナリに埋め込む。停止中とエラーのときに使う灰色の版は、起動時にこの画像から作る（輝度に変換し、アルファを半分にする）。macOS のテンプレート画像（メニューバーの色に合わせて白黒が反転するもの）にはしていない。

## 配布

release ワークフロー（[`../architecture.md#ビルドとリリース`](../architecture.md#ビルドとリリース)）は、Windows と macOS のアーカイブに `swing-tray`（`swing-tray.exe`）を同梱する。Linux のアーカイブには入れない。

## ログイン時の自動起動

`swing service install` が、`swing` と同じディレクトリにある `swing-tray` をログイン時に起動するよう登録し、その場で起動もする（`--no-tray` で外す）。Windows は Run キー、macOS は 2 つ目の LaunchAgent。詳細は [`service.md#タスクトレイの自動起動windows-と-macos`](service.md#タスクトレイの自動起動windows-と-macos)。
