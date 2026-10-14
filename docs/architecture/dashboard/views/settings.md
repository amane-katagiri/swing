# Settings 画面（`web/settings.js`, `web/settings-notify.js`, `web/stats.js`, `web/network.js`）

[`../views.md`](../views.md) の子ページ。上から設定表・「表示」・「通知」・「リソース使用量」・「ネットワーク」・「プロセス」のパネルが並ぶ。

## 設定編集

[`/api/config`](../http-api/config.md#get-apiconfig) をセクションごとの表（キー・値・env）で出す（`renderConfig`）。

- `item.editable === true`（書き込み範囲に入っていて env 由来でない。[`../../config.md#編集できるキー`](../../config.md#編集できるキー)）の項目は値のセルが入力欄になる。`kind` が `bool`/`mode` ならセレクト、`list` ならテキストエリア、それ以外はテキスト入力で、初期値は `item.raw`。ほかの項目はテキスト表示。
- 書き込み範囲の項目（`item.raw != null`）には、値のセルの下に `item.description[lang]`（無ければ `.en`）を `swing-hint` で添える。env 由来で編集できないものには、env で固定されている旨も添える。
- セクションごとに 1 つの Save ボタン。そのセクション内で初期値から変わったフィールドだけを [`PUT /api/config`](../http-api/config.md#put-apiconfig) に送る（変更が無ければ何もしない）。成功すると応答の `ConfigDto` で描き直し、状態行に再起動が必要な旨を出す。失敗したらそのセクションだけ編集できる状態に戻してエラーを出す。
- `config.writable === false` なら、`editable` な項目も読み取り専用で表示し、書き込めない旨の注記を出す。
- `config.restart_required === true` なら、画面上部に再起動待ちの注記を出す。

## 表示の設定

テーマ・表示言語・カスタム CSS をブラウザの `localStorage` に保存する（キーは [`../web.md#localstorage-キー一覧`](../web.md#localstorage-キー一覧)、効き方は [`../css.md`](../css.md) と [`../web.md#表示言語i18n`](../web.md#表示言語i18n)）。

## 通知

「通知」パネル（`settings-notify.js`）は、Desktop 画面の「コントロール パネル」→「通知」タブの「更新の確認」と「ブラウザの通知」と同じ設定を読み書きする。値・既定・許可の規則・反映の時機は [`../notices.md#設定`](../notices.md#設定)。ブラウザにだけ保存される旨の説明を 1 行添える。

- 上から「確認の間隔」のセレクト、「ブラウザの通知を使う」チェックボックス、「通知する内容」の 3 つの種類のチェックボックス（使わないときは無効表示）。間隔を「確認しない」にするとブラウザの通知もマスコットのおしらせも来ない旨を `swing-hint` で添える。
- 変えたその場で保存する（保存ボタンは無い）。種類と「使う」はブラウザ側（`browser`）だけを書き換える。
- 許可の理由と保存の失敗は `#settings-notify-status`（`swing-status`、`data-kind="error"`）に出し、言語を切り替えると出し直す。間隔や種類を変えたときは保存の失敗の表示だけを消す。
- 画面を開くたびに `localStorage` から読み直す。

## リソース使用量

`stats.js` が受け持つ。

- 画面を開くたびに [`GET /api/stats`](../http-api/status.md#get-apistats) を呼び、以後は Settings 画面を表示している間だけ `interval` 秒ごとに呼び直す（画面を離れると次の呼び出しで止まる）。`since` には手元の最後のサンプルの `at`（無ければ 0）を渡して新しいサンプルだけを取り、最新のサンプルから 24 時間より古いものを捨てる。
- 表（`.swing-table`）は `swing` の CPU・メモリ、Kubo の CPU・メモリ、IPFS の受信・送信の 6 行。列は「現在」（最新のサンプル）・「1 時間平均」・「1 時間最大」・「24 時間最大」（最新のサンプルから数えた期間）。値が無ければ `–`。バイト数は `formatBytes`、通信量はそれに `/s` を付ける。
- パネルの説明は、1 分ごとの記録を最大 24 時間さかのぼって見られることと、IPFS の通信量が何を数えたものか。
- 表の下に最終記録の時刻、最新のサンプルに通信量があれば Kubo 起動からの累計、`kubo_managed` が `false` なら Kubo が SWING の管理外で CPU・メモリを取得できない旨を出す。サンプルが 1 つも無ければ表の代わりに「まだ記録がありません」を出す。
- 読み込みの失敗は `#stats-status` に出し、次の呼び出しは続ける。接続できなかった後の扱いは [`../web.md#止まっている間の呼び出し`](../web.md#止まっている間の呼び出し)。

## ネットワーク

`network.js` が受け持つ。

- 画面を開くたびと、見出しの右の再読み込みのアイコンボタン（`#network-reload`。Publish 画面の「自分のサイト」と同じ形。見出しの下線を全幅に保つため、`h2` 自体に `swing-heading-row` を付けて中に置く。呼び出し中は無効）で [`GET /api/network`](../http-api/status.md#get-apinetwork) を呼ぶ。自動では呼び直さない。
- 判定を `swing-status` の 1 行で出す。`public` は「このノードは外から直接アクセスできます。」（`data-kind="ok"`）、`private` は「このノードは外から直接アクセスできません。」（`warn`）、`offline` は「このノードはオフラインです。」（`error`）、`unknown` とほかの値は「外からこのノードにアクセスできるか判定中です。」（`info`）。その下に判定ごとの説明を 1 段落出す（届かないときは、ホールパンチでしか取得されないことと swarm のポートの TCP と UDP を外から受けると届きやすくなること、判定中は起動から数分かかること）。`reachability` が `public` でなく `nat` があれば、ルーターの外側の NAT についての段落を足す。
- `public` では、判定の説明の後に、アンカーとしての広告の状態の段落を足す。`anchor.advertising` なら「このノードをアンカーとして広告中です。…」、`anchor.advertise` が `false` なら「アンカーとしての広告は設定でオフです（[anchor] advertise）。」、ほかは「外から直接アクセスできる状態がしばらく続けば、このノードをアンカーとして広告します。」。`public` でなければ広告の状態は出さない（判定の説明で足りる）。
- `offline` では説明（Kubo がネットワークにつながずに動いていて、ほかの参加者とやりとりしないこと）だけを出し、リンクもアドレスの表もアンカーの部分も出さない。
- それ以外で `public` でなければ続けて `guide` へのリンク（「修正ガイド」。動作性の確認のリンクと同じ名前）を出す。最後に、アドレスごとの結果の表（`.swing-table`。列は「アドレス」と「外からの接続」。後者は `swing-badge` の `data-reach` で、`reachable` は「届く」（`--swing-ok`）、`unreachable` は「届かない」（`--swing-warn`）、`unknown` は「未確認」（既定の灰色。AutoNAT がまだ試していないアドレス））を出す。アドレスが 1 つも無ければ表の代わりにその旨を `swing-hint` で出す。
- `offline` 以外では、最後に `anchor`（[アンカー](../../agent/anchors.md)）を「アンカー」の `h3`（`#network-content h3` で上に `--swing-space-5`）の下に出す。
  - `anchors` があれば説明の `swing-hint` と表（`.swing-table`。列は「アカウント」（フィンガープリントの先頭 1 つと npub の短縮。`title` に npub 全体）・「PeerID」（等幅の短縮。`title` に全体）・「選んだ理由」（`mutual` は「相互ミラー」、`coverage` は「ミラー対象を N サイト保存」）・「接続」（`connected` が `true` なら `swing-badge` の `data-reach="reachable"` で「接続中」、`false` なら `data-reach="unknown"` で「未接続」、`null` なら「–」））。無ければ `swing-hint` で、`max_peers` が 0 なら設定でオフであること、ほかは接続しているアンカーが無いことを出す。
- 読み込み中と失敗は `#network-status` に出し、失敗したら前の結果を消す。言語を切り替えると手元の結果で描き直す。

## プロセス操作

ページ末尾の「プロセス」パネル（停止・再起動の 2 ボタン）。

- どちらも確認のダイアログの後で [`POST /api/shutdown`](../http-api/session.md#post-apishutdown-post-apirestart)／`POST /api/restart` を呼び（`settings.js::runProcessAction`）、呼び出し中はボタンを無効にし、結果かエラーを状態行に出す。
- リクエストが受理された時点で完了とし、プロセスが実際に止まる/再起動するまでは待たない。
