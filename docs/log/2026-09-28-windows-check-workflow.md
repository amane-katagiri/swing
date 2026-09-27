# Windows の動作確認ワークフローと、トレイの起動でハンドルを継承させない修正

## 背景

`docs/todo.md` に、Windows の英語の表示言語での確認が 2 件残っていた（トレイのメニューとダイアログが英語になること、`schtasks` による登録判定が正しく働くこと）。手元の Windows は日本語なので、英語の GitHub の Windows ランナーで確かめることにした。

## 決めたこと

| 決定 | 理由 |
|---|---|
| `macos-check` と同じく、別のワークフロー `windows-check.yml` にして `workflow_dispatch` だけで動かす | 必要なときだけ起こす |
| 作り込みは作業ブランチで、一時的に `push` トリガーを付けて回した | `workflow_dispatch` はデフォルトブランチにあるワークフローしか起動できないため。main に入れる前に外した |
| スクリプトは PowerShell、トレイとダイアログの操作は UI Automation と Win32 API | ランナーのジョブは `runneradmin` の対話セッション（console）で動いていて、画面を撮ることも入力を送ることもできる |
| メニューは `MN_GETHMENU` で `HMENU` を取って読む | `muda` のポップアップメニューの項目は UI Automation に出てこなかった（`#32768` のウィンドウは見えるが、中身は空） |
| ステップごとに 3 分のタイムアウトを付け、ジョブは 30 分にした | 止まったときに 45 分待たされたため |
| `service install` でのトレイの起動を、`std::process::Command` から `CreateProcessW`（`bInheritHandles = FALSE`）に替えた | 下記 |

## 見つけた不具合: `swing service install` の出力をパイプで受けると返ってこない

1 回目と 2 回目は、`swing service install` の出力を PowerShell で受け取るステップが終わらず、タイムアウトまで止まった。`install` そのものは最後まで進んでいた（出力もダッシュボードの起動もあった）。`std::process::Command` は Windows では常に `bInheritHandles = TRUE` で子を作るので、`install` が起動した `swing-tray` が、呼び出し元から引き継いだ継承可能なハンドル（パイプの書き込み側）を開いたまま持ち続ける。そのため、読む側にはトレイが終わるまで EOF が届かない。`swing.exe` の標準出力を `cmd /c ... > file` でファイルに向けても、`cmd` が持っていたパイプのハンドルが同じように引き継がれて止まった。

手元で `swing service install | Tee-Object log.txt` のようにパイプで受けたときも、トレイを終了するまで返ってこなかったはず。トレイは標準入出力を使わないので、ハンドルを一切継承させない `CreateProcessW` で起動するようにした。コマンドラインは Run キーに書くのと同じ `tray_run_command` を使う。修正後は、パイプで受けても `install` がすぐに返った。

## 分かったこと

- ランナーは Windows Server 2025（10.0.26100）、表示言語・ロケールとも `en-US`（`Get-UICulture`・`Get-WinUserLanguageList`）、コードページは 65001。画面は 1024×768。
- 起動時に「System Properties」の、ページングファイルについてのダイアログが出ている。
- `swing-tray` のアイコンはタスクバーに出ていて、あふれには入っていなかった。UI Automation での名前は `SWING <ツールチップ>`。タスクバーのウィンドウのボタン（`swing-tray - 1 running window`）も `SWING` で始まる名前として拾えてしまうので、大文字小文字を区別して探す。
- 確認のダイアログのボタンは、UI Automation では `ControlType.Pane`、クラス名 `Button` として見える。

## 検証

作業ブランチで 6 回実行し、最後の回は全ステップが通った。英語の表示言語で確かめたこと:

- 登録判定: 未登録のとき `schtasks /Query /TN swing` は `ERROR: The system cannot find the file specified.` で失敗し、`swing service status` は `not installed` になる。登録後は `schtasks` の詳細が出て、`uninstall` 後は `not installed` に戻る。`uninstall` 後、トレイは 5〜16 秒で自分で閉じた。
- ツールチップ: `SWING: Waiting for setup`・`SWING: Stopped`。
- 動作中のメニュー: `SWING: Waiting for setup`（無効）、`Open dashboard`・`Restart`・`Stop`（有効）、`Start`（無効）、`Quit`（有効）。停止中は `SWING: Stopped` で、`Start` と `Quit` だけが有効。
- 「Stop」のダイアログ: タイトル `SWING`、`Stop SWING?` / `Mirrored sites will no longer be fetched or served.`、ボタンは Yes / No。No で止まらない。
- 「Quit」のダイアログ: `Stop SWING before closing the tray?` / `Choose No to close only the tray and keep SWING running.`、ボタンは Yes / No / Cancel。Cancel でトレイが残る。
- 停止中のメニューの「Start」で `swing up` が起動する。

確かめていないこと: ダイアログで Yes を選んだときの動き、英語の表示言語での「Open dashboard」。
