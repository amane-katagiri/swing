# macOS の動作確認ワークフロー

## 背景

Mac の実機が無く、`swing-tray` とサービス登録の macOS での動作（`docs/todo.md` の「タスクトレイ: macOS の実機で確かめる」）が未確認のままだった。GitHub の macOS ランナーには GUI のログインセッションがあり、`screencapture` と `osascript` が使えるので、そこで動かして画面を撮ることにした。

## 決めたこと

| 決定 | 理由 |
|---|---|
| release とは別のワークフロー `macos-check.yml` にし、`workflow_dispatch` だけで動かす | 必要なときだけ起こす。private リポジトリでは macOS ランナーの分数が 10 倍で数えられるので、push や PR では動かさない |
| 作り込みは作業ブランチで、一時的に `push` トリガーを付けて回した | `workflow_dispatch` はデフォルトブランチにあるワークフローしか起動できないため。main に入れる前に外した |
| 鍵の無いセットアップモードで動かし、Kubo も relay も使わない | トレイの「動作中」「停止中」とサービス登録を見るのにはダッシュボードが応答すれば足りる。外部に出ない |
| メニューは AX の `click` ではなく、アイコンの中央に `CGEventPost` でマウスのクリックを送って開く | `tray-icon` はマウスのイベントでメニューを出すので、AX の `click` は成功を返すだけでメニューが開かなかった |
| 起動はメニューを開いて ↓ → Return で選ぶ | 停止中に最初に選べる項目が「Start」なので、座標を決め打ちしなくてよい |
| ログイン項目は `sfltool dumpbtm` だけを残す | システム設定の画面は一覧がアルファベット順で、swing の行が 1024×768 の画面に収まらず、Page Down でもスクロールしなかった |
| Safari の表示倍率の確認は入れない | ランナーの画面は等倍（Retina ではない）で、todo の条件を再現できない |

## 分かったこと

- ランナーは macOS 26、画面は 1024×768、ロケールは `en_US`。`/bin/bash` と `osascript` にはアクセシビリティと Apple Events の許可が付いている。`system_profiler SPDisplaysDataType` は何も返さない。
- `swing-tray` のアイコンは、System Events では `swing-tray` のプロセスの `menu bar 1` の `menu bar item 1` に見える（`menu bar 2` ではない）。
- 1 回目は時計の左に出ていた SWING のアイコンをシステムのアイコンと見誤り、「アイコンが出ていない」と判断していた。切り抜いて並べて確かめた。
- `swing stop --config <path>` を別のディレクトリから実行すると、`state_dir = "./data"` がカレントディレクトリから解決されてトークンが合わず、`missing or invalid dashboard token or session` で失敗した。ワークフローでは設定ファイルのディレクトリで実行している。

## 検証

作業ブランチで 5 回実行し、最後の 2 回は全ステップが通った。確かめたこと:

- `service install` で 2 つの LaunchAgent（`jp.ne.ama.swing`・`jp.ne.ama.swing-tray`）が `running` になり、トレイの `ApplicationType` が `UIElement` で、Dock に出ない
- アイコンがライトとダークの両方で見え、停止中は薄くなる
- 動作中（セットアップ待ち）のメニューは「Open dashboard」「Restart」「Stop」「Quit」が有効で「Start」が無効、停止中は「Start」と「Quit」だけが有効。状態の行は `SWING: Waiting for setup` と `SWING: Stopped`
- 停止中にメニューの「Start」を選ぶと `swing up` が起動し、ツールチップが `SWING: Waiting for setup` に戻る
- `sfltool dumpbtm` では、2 つとも名前が実行ファイル名（`swing`・`swing-tray`）の legacy agent として登録される
- `service uninstall` の後は LaunchAgent もプロセスも残らない

確かめていないこと: ログインし直したときの自動起動、日本語のロケール、確認のダイアログ、「Open dashboard」でブラウザが開くこと。
