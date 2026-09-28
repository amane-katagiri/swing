# macOS の確認のダイアログと日本語の表示

## 背景

[macOS の動作確認ワークフロー](2026-09-27-macos-check-workflow.md) では、`swing-tray` の日本語の表示、確認のダイアログ（「Stop」と動作中の「Quit」）、「Open dashboard」でブラウザが開くことが確かめられていなかった。`macos-check` にこれらの段階を足して、GitHub の macOS ランナーで確かめた。

## 決めたこと

| 決定 | 理由 |
|---|---|
| 英語の確認を終えてから `AppleLanguages` を `ja-JP` にし、トレイを `launchctl kickstart -k` で起動し直す | トレイは `sys_locale::get_locale()`（macOS では `CFLocaleCopyPreferredLanguages`）で言語を決め、起動時に 1 回だけ読む。既存の英語の段階をそのまま残せる |
| ダイアログの操作は日本語で一通り行い、英語では「Stop」→「No」と「Quit」→「Cancel」だけにする | 分岐（キャンセル・いいえ・はい）を確かめるのは 1 言語で足りる。英語は文言とボタンを保存するだけ |
| メニューの項目は名前で探し、そこまでの有効な項目の数だけ ↓ を押して Return で選ぶ | 座標を決め打ちしない。AX で読める項目名と有効・無効から数えられる |
| ダイアログのボタンは `UserNotificationCenter` のウィンドウの AX の `click button` で押す | ダイアログはトレイのプロセスのウィンドウではない（下記） |
| 「Open dashboard」の URL は Safari の `History.db` から読む | Apple Events で Safari に URL を聞くと、1 回目は 2 分、`with timeout` を付けても時間切れで返らなかった。ランナーの `/bin/bash` はフルディスクアクセスを持つので履歴を直接読める |
| macOS ではダイアログのボタンの文字をトレイの言語で渡す（`OkCancelCustom`・`YesNoCancelCustom`） | 下記の「ボタンが英語のまま」を直した |
| macOS でダイアログを開いている間にトレイが止まるのは直さず、`tray.md` に書く | システムのモーダルなアラートとしては普通のふるまいで、ダイアログに答えれば元に戻る |

## 分かったこと

- rfd 0.17 は、親ウィンドウの無い `MessageDialog` を macOS では NSAlert ではなく `CFUserNotificationDisplayAlert` で出す。ダイアログは `UserNotificationCenter` のプロセスの `window 1`（subrole `AXSystemDialog`）として見え、トレイのプロセスにはウィンドウが無い。1 回目の実行ではトレイのプロセスの `window 1` を探して見つけられなかった。
- rfd はこのアラートを `run_on_main` でメインスレッドから出し、答えるまで返らない。そのため、開いている間はトレイのイベントループが止まり、System Events からもトレイの `menu bar 1` が見えなくなる（`Invalid index`）。アイコンをクリックしてもメニューは開かず、ダイアログに答えた後でそのクリックが処理されてメニューが開いた。以前の `tray.md` の「ダイアログを開いている間もイベントループは止まらない」は Windows だけに当てはまる。
- rfd は `MessageButtons::YesNo`・`YesNoCancel` のボタンの文字を macOS では "Yes"・"No"・"Cancel" に決め打ちしている。日本語の表示でも、本文が「「いいえ」を選ぶと…」なのにボタンが「No」になっていた。以前の `tray.md` の「ボタンの文字は OS 標準の「はい」「いいえ」「キャンセル」」は Windows だけに当てはまる。
- そこで macOS では、はい / いいえのダイアログを `OkCancelCustom(はい, いいえ)`、はい / いいえ / キャンセルのダイアログを `YesNoCancelCustom` にし、トレイの言語のラベルを渡すようにした。rfd はカスタムのボタンを押されると `MessageDialogResult::Custom(<ボタンの文字>)` を返すので、同じ言語のラベルと比べて答えに戻す（`status::Answer::from_label`。どれにも当たらなければキャンセル扱いにして、止める操作に倒れないようにした）。Windows は `MessageBox` が表示言語でボタンを出すので、`YesNo`・`YesNoCancel` のまま。
- 「Open dashboard」は既定のブラウザ（Safari）を開き、履歴は `/login?code=…` → `/`（リダイレクト）→ `/#/setup` と進んだ。画面はサイドナビ付きのセットアップ画面で、ログイン画面ではないので、セッションが付いている。アドレスバーは `127.0.0.1` とだけ出る。
- `/usr/bin/osascript` と `/bin/bash` の Apple Events の許可は System Events には効くが、Safari への問い合わせは時間切れになった（自動化の確認が画面に出ずに待っているとみられる）。

## 検証

作業ブランチ `macos-check-dialogs` で、一時的な `push` トリガーを付けて 4 回実行した。最後の実行の後はトリガーを外しただけで、ワークフローの中身は変えていない。

| 実行 | 内容 | 結果 |
|---|---|---|
| 36418390351 | 最初の版 | ダイアログをトレイのプロセスで探していて見つからなかった。Safari への Apple Events が 2 回とも 2 分の時間切れ |
| 36419845809 | ダイアログを `UserNotificationCenter` で探す。開いている間のトレイを調べる | 日本語の段階はすべて通った。英語の「Quit」の前に、ダイアログの間に送ったクリックの後始末をしておらず失敗した |
| 36420626308 | 履歴から URL を読む。クリックの後始末 | すべて通った。ボタンは英語のまま |
| 36424064346 | ボタンの文字をトレイの言語で渡す修正。日本語のダイアログのボタンを日本語の名前で押す | すべて通った。以下の確認はこの実行による |

確かめたこと（画像と保存したテキストを見て確認した）:

- 日本語の動作中のメニューは `SWING: セットアップ待ち`・`ダッシュボードを開く`・`再起動`・`停止`・`起動`・`終了` で、「起動」だけが無効。停止中は `SWING: 停止中` で、「起動」と「終了」だけが有効。
- 「停止」のダイアログは、タイトル `SWING`、本文「SWING を停止しますか？」「ミラーしているサイトの取得と配信が止まります。」、ボタンは「いいえ」「はい」。「いいえ」では止まらず、「はい」でダッシュボードが応答しなくなり、アイコンが薄くなる。その後メニューの「起動」で起動し直せる。
- 「終了」のダイアログは、本文「SWING を停止してからトレイを閉じますか？」「「いいえ」を選ぶと、SWING は動かしたままトレイだけを閉じます。」、ボタンは「はい」「いいえ」「キャンセル」。「キャンセル」ではトレイも `swing up` も残る。「いいえ」ではトレイだけが閉じ、`swing up` は動き続ける。トレイを起動し直して「はい」を選ぶと、`swing up` が止まってからトレイも閉じる。
- ダイアログを開いている間はアイコンをクリックしてもメニューが開かず、答えた後で開いた（36419845809・36420626308）。
- 英語の「Stop」のダイアログは「Stop SWING?」「Mirrored sites will no longer be fetched or served.」（ボタンは No・Yes）で、「No」では止まらない。「Quit」のダイアログは「Stop SWING before closing the tray?」「Choose No to close only the tray and keep SWING running.」（ボタンは Yes・No・Cancel）で、「Cancel」ではトレイが残る。
- 「ダッシュボードを開く」で Safari が起動し、ウィンドウ名は `SWING Dashboard`、ログイン済みのセットアップ画面が出る。履歴は `/login?code=…` → `/` → `/#/setup`。

確かめていないこと: ログインし直したときのトレイの自動起動（ランナーではログインし直せない）。
