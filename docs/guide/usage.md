# ミラーの管理とダッシュボード

ミラー対象の追加・確認、保存状況の点検、ダッシュボードの使い方をまとめます。

## ミラー対象を管理する・状態を見る

ここから先のコマンド例はバイナリで直接動かしている場合の書き方です。Docker Compose の場合は `swing ...` を `docker compose exec mirror swing ...` に読み替えてください（`up` はコンテナ起動時に自動で実行されるので、`swing up` 自体を読み替える必要はありません）。

保存したい相手を追加するには `swing mirror add` を実行します。相手の npub（または hex、nprofile）を指定してください。

```bash
swing mirror add npub1alice... npub1bob...
```

これは Nostr 上の NIP-51 Follow Set（`kind 30000`, `d = swing`）を更新し、指定した相手を保存対象として宣言します。設定を確認するには次のようにします。

```bash
swing mirror list
```

保存対象のサイトの状態を見るには `swing sites` を使います。

```bash
swing sites
```

各サイトについて `d`（サイト識別子）、`cid`、`url`、`size`、`created_at`、NIP-05 の検証結果、最新版のレプリカ数、保存状況（`stored` / `not stored`）が 1 行ずつ表示されます。作者が更新メモを付けていれば、次の行に `message:` として表示されます。mirror-agent は最後に確認したミラー対象リストを状態ファイルに保存しています。relay が古いリストを返したり、リストを失ったりしても、保存済みの新しいリストを使い、relay に送り直します。そのため、relay の不調でミラー対象から外れたと誤認してサイトを消すことはありません。ミラーをやめたい相手は `swing mirror remove` で外してください。

ミラー対象から外したのにまだ保存しているサイトは、最後に `[unfollowed]` として表示されます。これを消すには `remove_on_unfollow` を `true` にして mirror-agent を再起動してください。次の Follow Set の確認で消えます。

保存したサイトが Kubo 上で壊れていないかは `swing status` で確認できます。

```bash
swing status
```

状態ファイルに記録した版が Kubo の MFS に揃っているかと、状態ファイルに無い余分なパスを表示します。問題があれば 0 以外で終了するので、cron などからの監視にも使えます。詳しくは [`docs/architecture/cli/views.md`](../architecture/cli/views.md#status) を参照してください。

動いている `swing up` の CPU・メモリと IPFS の通信量は `swing stats` で見られます。`swing up` が 1 分ごとに測って直近 24 時間分をメモリに持っていて、`--last 6h` のように期間を指定すると、その間の平均と最大を表示します。ダッシュボードの Settings 画面にも同じ内容が出ます。詳しくは [`docs/architecture/stats.md`](../architecture/stats.md) を参照してください。

保存したサイトはローカルのゲートウェイで閲覧できます。`http://localhost:8080/ipfs/<cid>/` を開くと `http://<cid>.ipfs.localhost:8080/` に移り、サイトごとに別のオリジンで表示されます。ゲートウェイはローカルにあるデータだけを返し、ネットワークから取りに行きません。

動作状況は、直接 `swing up` を実行していればそのまま端末（または `--log-file` で指定したファイル）に出ます。サービスとして登録した場合は、Linux なら `journalctl --user -u swing -f`、macOS なら `~/Library/Logs/swing.log`、Windows ならサービス登録時のログファイルで確認できます。Docker Compose の場合はコンテナのログで確認します。

```bash
docker compose logs -f mirror
```

## ダッシュボード

SWING は mirror-agent（バイナリでは `swing up`、Docker Compose では `mirror` コンテナ）がブラウザ向けの管理画面も兼ねています。起動したら、次のコマンドでログインしてダッシュボード（`http://127.0.0.1:8082/`）を開いてください。使い捨てのログインリンクがブラウザで開き、ログイン状態は 30 日続きます。

```bash
# バイナリ
swing dashboard open
# Docker Compose（コンテナ内ではブラウザを開けないので、表示された URL を開く）
docker compose exec mirror swing dashboard open --no-browser
```

`SWING_DASHBOARD_BIND` でポートを変えた場合など、ブラウザから開く URL が `http://127.0.0.1:8082` と違うときは、`.env` に `SWING_DASHBOARD_PUBLIC_URL=http://127.0.0.1:18082` のように書くと、表示される URL がそれに合わせて変わります（表示されたコードをログイン画面に貼っても構いません）。

全ブラウザのログインを取り消したいときは `swing dashboard rotate-token` を実行します。

Windows と macOS では、`swing-tray` を起動するとタスクトレイ（macOS はメニューバー）にアイコンが出ます。そこからダッシュボードを開く（ログイン済みで開きます）・再起動・停止ができます。サービスとして登録してあれば、トレイを起動したときに `swing up` が止まっていれば起動し、メニューから起動することもできます。トレイを終了するときは、SWING も止めるかどうかを選べます。`swing service install` で登録すると、ログイン時に自動で起動します。手で起動するときは、`swing up` と同じ設定ファイルを読むように `swing-tray --config <swing.toml のパス>`（macOS は `SWING.app/Contents/MacOS/swing-tray --config <swing.toml のパス>`）と指定してください（詳しくは [`docs/architecture/tray.md`](../architecture/tray.md)）。

- **Desktop**: 保存中のサイトを、懐かしい Windows 風デスクトップ上のブラウザウィンドウに表示される「リンク集」ページ風に眺められます。ウィンドウのツールバーの「ミラー」（星のアイコン）から、Sites 画面と同じようにミラーするアカウントを追加できます。
- **Sites**: `swing sites` と同じ内容を一覧表示し、そのまま「mirror に追加」「mirror から外す」を操作できます。ボタンひとつで `swing status` 相当のストレージチェックも実行できます。
- **Webring**: `swing webring` のグラフを、ドラッグ・パン・ズームできる図として表示します。ノードを選ぶとレプリカ数の詳細が見られ、そこから mirror への追加もできます。
- **Publish**: これまでに公開したサイトの一覧（「My sites」）から選び直したり、新しく publish したりできます。ブラウザから直接フォルダを選んでアップロードする方式なので、**Docker Compose でも volume のマウントは不要**です（既定の上限は 2GiB、`SWING_DASHBOARD_MAX_UPLOAD` で変更可）。
- **Settings**: 現在の設定を表示します（秘密鍵の値は一切表示されません）。環境変数で設定した項目を除き、その場で編集して保存できます（保存後、エージェントを再起動すると反映されます）。ブラウザ側のテーマ・表示言語（日本語/English）・カスタム CSS もここで設定します。
- **Setup**: 鍵も署名アプリも未設定のとき（セットアップモード）だけ表示される導入画面です。「[セットアップモード](install.md#セットアップモード)」を参照してください。

ダッシュボードは既定で `127.0.0.1` だけで待ち受け、ログインが必要です。`swing status`・`swing mirror add`・`swing mirror remove`・`swing stop` はこのダッシュボードの API を経由し、データディレクトリの `dashboard.token` を使って認証します（`swing up` と同じ設定・同じユーザーで実行してください）。ホストでの公開先を変えたい場合や、Web の管理画面だけを外して API だけ残したい場合は `.env` に次のように設定してください。

```bash
# ホストでの公開先を変える（既定は 127.0.0.1:8082）
SWING_DASHBOARD_BIND=0.0.0.0:8082
# Web の管理画面だけを配信しない（Docker Compose でも直接バイナリを動かす場合でも共通。/api/* は残る）
SWING_DASHBOARD_UI=false
```

見た目は `--swing-*` の CSS 変数と `SWING_DASHBOARD_CUSTOM_CSS`（`/custom.css` として配信される追加スタイルシート）でカスタマイズできます（使える変数・class・data 属性は [`docs/architecture/dashboard/css.md`](../architecture/dashboard/css.md)）。Desktop 画面のリンク集ページは、`SWING_DASHBOARD_DESKTOP_PAGE`（ページ本体の HTML）・`SWING_DASHBOARD_DESKTOP_PAGE_CSS`（そのページ専用の CSS）・`SWING_DASHBOARD_DESKTOP_BANNER`（88×31 バナー画像）で丸ごと自分のものに差し替えられます（いずれも起動時に読み込みます）。このページは同一オリジンの iframe に入っているので、ダッシュボードのスタイルは一切当たらず、こちらのスタイルも外に漏れません。ページに `desk-link-list` などの決まった `id` を置いておくと、そこにリンク一覧が描画されます（詳しくは [`docs/architecture/dashboard/desktop.md#リンク集ページiframe`](../architecture/dashboard/desktop.md#リンク集ページiframe)）。API の詳しい仕様は [`docs/architecture/dashboard/http-api.md`](../architecture/dashboard/http-api.md)、ガード（Host 検証、CSRF 対策など）は [`docs/architecture/dashboard/security.md`](../architecture/dashboard/security.md) を参照してください。

## 自分のマスコットを追加する

Desktop 画面を歩き回るマスコットは、同梱の 3 体（`yureko`・`mochi`・`neko`）に加えて自分で追加できます。`SWING_DASHBOARD_MASCOTS_DIR` にディレクトリを指定し、その直下に `manifest.json` とスプライト画像を入れたサブディレクトリ（1 つがそのまま 1 パック、ディレクトリ名がパックの id）を置いて `swing up` を再起動してください。作り方は [`docs/mascot-guide.md`](../mascot-guide.md)、マニフェストの書き方・検証規則の詳細は [`docs/architecture/dashboard/mascot/pack.md#パック形式-1`](../architecture/dashboard/mascot/pack.md#パック形式-1) を参照してください。

どのマスコットを出すか（既定は `yureko` だけ）と動きは、Desktop 画面の「コントロール パネル」の「マスコット」タブで選べます。更新の確認の間隔と、おしらせする内容（フォロー中のサイトを新しくミラーしたとき・サイトを公開したとき・自分のサイトが新しくミラーされたとき）は「通知」タブで、デスクトップ（マスコット）とブラウザの通知で別々に選べます。ブラウザの通知をオンにすると、Desktop 画面を見ていないときやタブが裏にあるときもブラウザの通知でおしらせします（`https://` か `localhost`・`127.0.0.1` で開いたときだけ使えます）。確認の間隔とブラウザの通知の設定は Settings 画面にもあります。

