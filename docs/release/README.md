# SWING

SWING (Static-site Webring by IPFS and Nostr Generator) は、個人サイトの運営者同士が、互いのサイトを自発的に保存・配送し合うための相互ミラーツールです。「誰のサイトを保存するか」を Nostr で表明し、実際のデータの保存と配送は IPFS で行います。中央の保存サーバーや管理者はいません。

このファイルは、ビルド済みのアーカイブを使う人向けの短い手引きです。しくみ・Docker Compose での動かし方・設定の詳細は、リポジトリの README を参照してください。

<https://github.com/amane-katagiri/swing>

## アーカイブの中身

- `swing`（Windows は `swing.exe`）: 本体
- `swing-tray.exe`（Windows）・`SWING.app`（macOS）: タスクトレイ（macOS はメニューバー）のアイコン
- `swing.example.toml`: 設定ファイルの見本。すべての設定項目・対応する環境変数・既定値が載っています
- `LICENSE`・`LICENSE-PixelMplus.txt`: ライセンス

IPFS ノードの Kubo は同梱していません。Windows のインストーラー（`-setup.exe`）か Linux の `install.sh` で入れた場合は、Kubo の `ipfs`（`ipfs.exe`）とそのライセンスも同じ場所に入っていて、下記の Kubo の用意は済んでいます。Windows のインストーラーはサービスの登録も済ませています。

## 必要なもの

- Kubo v0.43.1 の `ipfs`（Windows は `ipfs.exe`）。<https://github.com/ipfs/kubo/releases/tag/v0.43.1> から取得し、`swing` と同じディレクトリに置くか PATH に通してください。`swing up` が自動で見つけて起動します
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

セットアップ画面で、鍵の生成（既存の鍵の貼り付けや署名アプリとの接続もできます）・relay・保存の上限を入力して送信すると `swing.toml` ができ、そのまま通常の動作に移ります。`swing.toml` は次のユーザーごとの既定の場所に作られ（`--config` を付けるか `SWING_CONFIG` を設定していればそのパス）、状態ファイルと Kubo のリポジトリもその下の `data` に置かれます。

- Linux: `~/.local/share/swing`（`XDG_DATA_HOME` を設定していれば `$XDG_DATA_HOME/swing`）
- macOS: `~/Library/Application Support/swing`
- Windows: `%LOCALAPPDATA%\swing`

### 設定ファイルを先に用意する

```bash
./swing key generate
cp swing.example.toml swing.toml
export SWING_CONFIG="$PWD/swing.toml"
```

この `swing.toml` は既定の場所に無いので、`SWING_CONFIG` でその場所を伝えます（Windows の PowerShell では `$env:SWING_CONFIG = "$PWD\swing.toml"`）。以下のコマンドはこのシェルで実行するか、各コマンドに `--config <path>` を付けてください。上記の既定の場所に置けば、どちらも要りません。

`swing.toml` を開き、少なくとも次を書き換えてから `./swing up` で起動してください。

- `[nostr] secret_key`: 生成した鍵の nsec または hex
- `[nostr] relays`: 使う Nostr relay
- `[policy] max_total_storage`: 保存する全サイト合計の容量上限

状態ファイルと Kubo のリポジトリは、設定ファイルのあるディレクトリの `data` の下に置かれます。

## ログイン時に自動で起動する

```bash
./swing service install
```

Linux では systemd のユーザーユニット、macOS では launchd の LaunchAgent、Windows ではタスクスケジューラに登録します。Windows と macOS では、`swing` と同じ場所にある `swing-tray.exe`・`SWING.app` も一緒に登録して起動します。トレイのアイコンから、ダッシュボードを開く・再起動・停止ができます。トレイが要らなければ `--no-tray` を付けてください。`SWING_CONFIG` を設定していればその `swing.toml` を登録します。設定していなくて設定ファイルがまだ無ければ、上記の既定の場所に空の `swing.toml` を作ってから登録するので、サービスはセットアップモードで起動します。

Linux でシステムユニットにするときは `sudo ./swing service install --system` とします。サービスは `sudo` を実行したユーザーの権限で動きます（別のユーザーにするなら `--run-as <user>`。root にするなら `--allow-root` も付けます）。`swing` の実行ファイル・設定ファイル・Kubo のバイナリとその親ディレクトリが、root かサービスのユーザーの持ち物で、ほかのユーザーやグループから書き込めないことを確かめ、そうでなければ登録しません。別のユーザーや root で動かすなら、`swing` と設定ファイルは root の持ち物でグループが書き込めないディレクトリ（`/usr/local/lib/swing` や `/etc/swing` など。Debian で `/usr/local` が `root:staff 2775` なら `/opt/swing` など）に置いてください。

状態の確認は `swing service status`、停止は `swing service stop`、登録の削除は `swing service uninstall` です。

## よく使うコマンド

`swing up` を動かしたまま、同じ設定ファイルを使うように実行します（既定の場所の設定ファイルならどのディレクトリからでもそのまま。それ以外なら `SWING_CONFIG` を設定するか `--config <path>` を付けます）。ほとんどの操作はダッシュボードからもできます。

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

ただし、ダッシュボードに同梱のマスコット「ゆれ子」（`yureko` のキャラクターとその画像）は MIT License の対象外で、Copyright (c) 2026 Amane Katagiri, all rights reserved とします。SWING とその改変版・再配布物（フォーク・パッケージ・コンテナイメージなど）の一部としてなら、ゆれ子のファイルを改変せずに複製・配布・表示できます。それ以外の利用（改変、別の作品やサービスでの使用など）には許諾が要ります。
