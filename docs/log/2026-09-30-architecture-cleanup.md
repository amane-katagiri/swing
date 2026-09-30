# architecture の整理

`docs/architecture.md` と `docs/architecture/` の肥大化・重複・理由の文・コードとの食い違いを整理した。

## 決めたこと

- 20KB を超えるファイルは、役割ごとに親子のページへ分けた。
  - `architecture.md` の「設定と環境変数」と `dashboard.md` の「設定の読み込みと編集」→ `config.md`
  - `dashboard.md` のガード・認証・リバースプロキシ・既知の弱点・秘密鍵を出さない仕組み → `dashboard/security.md`
  - `dashboard/http-api.md` は共通の規則とエンドポイント一覧だけにし、各エンドポイントは `dashboard/http-api/` の `status`・`nostr`・`publish`・`config`・`session` へ
  - `dashboard/web.md` の各画面 → `dashboard/views.md`、CSS カスタマイズ → `dashboard/css.md`
  - `dashboard/desktop.md` の更新の確認・おしらせの出し分け・ブラウザの通知 → `dashboard/notices.md`（どの画面でも動くので `dashboard.md` の子）
  - `dashboard/mascot.md` のパックの形式と配信 → `dashboard/mascot/pack.md`
  - `kubo.md` の MFS と RPC → `mfs.md`、`nostr.md` の取得と表示の上限 → `nostr/fetch.md`
  - `cli.md` の publish → `cli/publish.md`、表示系のサブコマンド → `cli/views.md`
  - `agent.md` のレプリカ報告 → `agent/replicas.md`、ポリシー判定 → `agent/policy.md`
  - `dashboard/desktop.md` のコントロール パネル → `dashboard/desktop/control-panel.md`
- 同じ規則を複数のファイルに書いていたところは正本を 1 か所に決め、ほかはリンクにした（イベントの規則は `protocol.md`、設定は `config.md`、認証は `dashboard/security.md`、停止の時間予算は `up.md`、おしらせは `dashboard/notices.md`、マスコットのパックは `dashboard/mascot/pack.md`）。
- 過去の log から張られている見出しは元のファイルに残し、中身は移動先を案内する 1 行にした。log 以外から張られていた見出しは消して、リンクを張り替えた。
- 理由の文は architecture から消した。log に無かったものは下の「architecture から移した理由」に書いた。
- `architecture.md` の索引は 1 行の説明とリンクだけにし、子ページは親ページが案内する。
- コードと食い違っていた記述は、コードに合わせて直した（同梱のマスコットのパックの数、`schedule_restart` の場所、publish の判定の順、`swing stop` が読む `instance` の出どころ、`Daemon::stop` の失敗の扱いなど）。
- コード側で見つけた食い違いは、別のコミットでコードを直した。設定カタログの `mascots_dir` の説明が同梱のパックを 2 つとしていたのを 3 つに（`swing.example.toml`・`.env.example` も生成し直した）、agent の取得失敗のログの "will retry on next poll" を、実際に取り直す時機に合わせて "will retry after min_update_interval" にした。

## 検証

- 整理したあと、何も知らない状態のサブエージェントにコードと照らし合わせて評価させ、指摘を直すことを、クリティカルな指摘が無くなるまで繰り返した。
- README・AGENTS.md・docs 以下（log を含む）の相対リンクとアンカーを機械的に照合し、切れが無いことを確かめた。
- 合計の大きさは約 459KB から約 392KB になり、20KB を超えるファイルは無くなった。

## architecture から移した理由

architecture を「今のコードの結果」だけにするため、log に書かれていなかった理由の文をここへ移した。

### relay からの取得と上限（`nostr/fetch.md`）

- サイトイベント・レプリカ報告・Follow Set は誰でも捨て鍵で出せるので、relay から読む経路はどれも `nostr::budget` の定数で件数を打ち切る。
- 1 回の REQ は relay ごとに `Relay::stream_events` で出して 1 本に合わせる。`Client::stream_events` はどの relay が EOSE まで答えたかを返さず、`fetch_events` は溜めた件数が上限（既定 10,000）を超えると全体をエラーにして捨てるので、どちらも使わない。
- `fetch_follow_sets`・`fetch_follow_set_authors_referencing`・`fetch_replica_reports_by` は `AUTHORS_PER_FILTER`（50）人ずつのフィルタに分け、`limit` もその組の人数から決める。1 人が大量のイベントを出しても、押し出せるのは同じ組の相手だけになる。
- `fetch_site_events`・`fetch_reports_about` は作者ごとに別のフィルタと `limit` を置く。1 人が `d` を変えて大量のサイトイベントを出しても、同じ REQ の他の作者の分は押し出せない。1 つの REQ のフィルタ数を relay の上限（NIP-11 の `max_filters`）に収めるため、組は `AUTHORS_PER_SPLIT_REQ`（10）人と小さくしてある。

### セットアップモードでのポートの調整（`up.md`）

- ずらしたダッシュボードのポートは、セットアップを終える前の起動の時点で設定ファイルに書く。`swing dashboard open`・`swing-tray`・`swing stop` は設定ファイルの `listen` を見てつなぐため。
- 管理下の Kubo の gateway のポートは、ダッシュボードの bind と同じ時点で bind してすぐ閉じて空きを確かめる。セットアップモードでは Kubo を起動しないので、この時点で確かめられる。

### 内蔵 gateway の `hosts`（`gateway.md`）

- `[gateway].hosts` にダッシュボードで開けるホスト名と同じ名前を入れるとエラーにする。cookie はポートを区別しないので、内蔵 gateway が配る peer のサイトの HTML がダッシュボードと同じホストに載らないようにするため。
