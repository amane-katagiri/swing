# SWING

SWING (Static-site Webring by IPFS and Nostr Generator) は、個人サイトの運営者同士が、互いのサイトを自発的に保存・配送し合うための相互ミラーツールです。「誰のサイトを保存するか」を Nostr で表明し、実際のデータの保存と配送は IPFS で行います。中央の保存サーバーや管理者はいません。

このファイルは、ビルド済みのアーカイブを使う人向けの短い手引きです。しくみ・Docker Compose での動かし方・設定の詳細は、リポジトリの README を参照してください。

<https://github.com/amane-katagiri/swing>

## アーカイブの中身

- `swing`（Windows は `swing.exe`）: 本体
- `swing-tray.exe`（Windows）・`SWING.app`（macOS）: タスクトレイ（macOS はメニューバー）のアイコン
- `swing.example.toml`: 設定ファイルの見本。すべての設定項目・対応する環境変数・既定値が載っています
- `LICENSE`・`LICENSE-PixelMplus.txt`: ライセンス

IPFS ノードの Kubo は同梱していません。

## 必要なもの

- Kubo v0.43.1 の `ipfs`（Windows は `ipfs.exe`）。<https://dist.ipfs.tech/kubo/v0.43.1/> から取得し、`swing` と同じディレクトリに置くか PATH に通してください。`swing up` が自動で見つけて起動します
- Nostr の秘密鍵。SWING 専用の鍵を新しく作ることをおすすめします（下記の手順で作れます）。秘密鍵をこのコンピュータに置きたくなければ、代わりにスマホの署名アプリ（NIP-46）を使えます
- 保存に回すディスク容量（既定の上限は 100GiB）

以下のコマンド例は Linux と macOS の書き方です。Windows では `./swing` を `.\swing.exe` に読み替えてください。

## はじめかた

### ダッシュボードから設定する

設定ファイルを用意せずに起動すると、ダッシュボードだけが動くセットアップモードになります。

```bash
./swing up
```

別の端末で次を実行すると、ログイン済みのダッシュボード（`http://127.0.0.1:8082/`）がブラウザで開きます。

```bash
./swing dashboard open
```

セットアップ画面で、鍵の生成（既存の鍵の貼り付けや署名アプリとの接続もできます）・relay・保存の上限を入力して送信すると、カレントディレクトリに `swing.toml` ができ、そのまま通常の動作に移ります。

### 設定ファイルを先に用意する

```bash
./swing key generate
cp swing.example.toml swing.toml
```

`swing.toml` を開き、少なくとも次を書き換えてから `./swing up` で起動してください。

- `[nostr] secret_key`: 生成した鍵の nsec または hex
- `[nostr] relays`: 使う Nostr relay
- `[policy] max_total_storage`: 保存する全サイト合計の容量上限

状態ファイルと Kubo のリポジトリは、設定ファイルのあるディレクトリの `data` の下に置かれます。

## ログイン時に自動で起動する

```bash
./swing service install
```

Linux では systemd のユーザーユニット、macOS では launchd の LaunchAgent、Windows ではタスクスケジューラに登録します。Windows と macOS では、`swing` と同じ場所にある `swing-tray.exe`・`SWING.app` も一緒に登録して起動します。トレイのアイコンから、ダッシュボードを開く・再起動・停止ができます。トレイが要らなければ `--no-tray` を付けてください。

状態の確認は `swing service status`、停止は `swing service stop`、登録の削除は `swing service uninstall` です。

## よく使うコマンド

`swing up` を動かしたまま、同じ設定ファイルのディレクトリで実行します。ほとんどの操作はダッシュボードからもできます。

```bash
./swing mirror add npub1...     # 保存する相手を追加する（remove で外す、list で一覧）
./swing sites                   # 保存しているサイトの状態
./swing status                  # 保存したサイトが Kubo 上で揃っているかの確認
./swing publish --site example.jp --url https://example.jp/ ./public   # 自分のサイトを公開する
./swing stop                    # 止める（Kubo も止まる）
```

保存したサイトは `http://localhost:8080/ipfs/<cid>/` で閲覧できます。

## 注意

- SWING は公開の IPFS ネットワークをそのまま使うので、匿名性はありません。他の参加者から、あなたの IP アドレスと保存しているサイトの関係が見えます。
- 外から受ける必要があるポートは IPFS 用の `4001/tcp`・`4001/udp` だけです。ダッシュボード（8082）と Kubo のゲートウェイ（8080）は `127.0.0.1` だけで待ち受けます。
- 秘密鍵は `swing.toml` に平文で保存されます。

## ライセンス

MIT License（`LICENSE`）。ダッシュボードの Desktop 画面は同梱フォント PixelMplus12（M+ FONT LICENSE、`LICENSE-PixelMplus.txt`、Copyright (C) 2002-2013 M+ FONTS PROJECT）を使用しています。
