<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/swing-lockup-dark.svg">
    <img src="docs/assets/swing-lockup.svg" alt="SWING" width="460">
  </picture>
</p>

<p align="center">
  <picture>
    <source media="(prefers-reduced-motion: reduce)" srcset="docs/assets/dashboard-desktop.png">
    <img src="docs/assets/dashboard-desktop.webp" alt="ダッシュボードの Desktop 画面。Windows 風デスクトップの上のブラウザウィンドウに、保存中のサイトのリンク集が並び、最新の更新情報がマーキーで流れている。コントロール パネルのアイコンから、デスクトップの背景を設定するダイアログを開いて右下へドラッグする。タスクバーの上ではマスコットが歩き回っていて、つまんで持ち上げると宙づりになり、離すと落ちる" width="900">
  </picture>
  <br>
  <sub>Desktop 画面（<a href="docker/demo/README.md">デモ環境</a>のサンプルデータ）</sub>
</p>

# SWING

SWING (Static-site Webring by IPFS and Nostr Generator) は、個人サイトの運営者同士が、互いのサイトを自発的に保存・配送し合うための相互ミラーツールです。

「誰のサイトを保存するか」を Nostr で表明し、実際のデータの保存と配送は IPFS で行います。中央の保存サーバーや管理者は存在しません。あなたのサイトを保存するかどうかは、それぞれの参加者が自分の意思で決めます。逆に言うと、SWING はデータの永久保存を保証するものではありません。保存してくれる参加者がいる間だけ、あなたのサイトのコピーが IPFS 上に残ります。

SWING が提供するのは、この「誰を保存するか」の表明と、実際に保存・配送するための最小限の仕組みだけです。今のところ、誰かが運営する Web サービスやアカウント登録、決済、専用の Relay や IPFS ネットワークは提供していません。管理用のダッシュボードも、各参加者が自分の手元で動かすものです。

## しくみ

```text
                   Nostr
       +-------------+-------------+
       |             |             |
     Alice          Bob          Carol
       |             |             |
 mirror-agent  mirror-agent  mirror-agent
       |             |             |
     Kubo          Kubo          Kubo
       |             |             |
       +-------------+-------------+
                Public IPFS
```

Nostr は「更新の通知」と「誰のサイトを保存するか」を伝えるために使います。IPFS はサイトの実データを保存・配送するために使います。

各参加者は `mirror-agent`（このツールの `swing up` が動かす agent）と、IPFS ノードである Kubo を動かします。mirror-agent は自分が保存すると決めた相手のサイト更新を Nostr 経由で受け取り、ポリシーに沿って CID を Kubo の MFS（Kubo 内のファイルシステム）に置いて保存します。

## はじめかた

ここでは、SWING を入れて、誰かのサイトをミラーできるようになるまでを説明します。OS ごとの細かい違いや、設定ファイルを先に用意する方法は [`docs/guide/install.md`](docs/guide/install.md) を参照してください。

### 1. 入れる

いちばん手軽な方法を OS ごとに挙げます。どれも IPFS ノードの Kubo を一緒に入れるので、別に用意する必要はありません。

- **Linux**（x86_64・aarch64）: インストールスクリプトで入れて、ログイン時に起動するよう登録します。

  ```bash
  curl -fsSL https://github.com/amane-katagiri/swing/releases/latest/download/install.sh | sh -s -- --service
  ```

- **Windows**: [GitHub のリリース](https://github.com/amane-katagiri/swing/releases/latest)にある `swing-<版>-x86_64-pc-windows-msvc-setup.exe` を実行します。署名をしていないので SmartScreen の警告が出たら「詳細情報」→「実行」で進めてください。最後に「SWING を起動してダッシュボードを開く」を選ぶと、そのまま手順 2 のセットアップ画面が開きます。
- **macOS**: Homebrew で入れて、ログイン時に起動するよう登録します。

  ```bash
  brew install amane-katagiri/swing/swing
  swing service install
  ```

- **Docker Compose**: リポジトリを取得して起動します。ここから先のコマンドは `swing ...` を `docker compose exec mirror swing ...` に読み替えてください。

  ```bash
  git clone https://github.com/amane-katagiri/swing.git
  cd swing
  cp .env.example .env
  docker compose up -d
  ```

ビルド済みのバイナリを自分で置く方法、ソースからのビルド、更新・アンインストールは [`docs/guide/install.md`](docs/guide/install.md)、Docker Compose の設定は [`docs/guide/docker.md`](docs/guide/docker.md) にあります。

### 2. ダッシュボードでセットアップする

次のコマンドで、ログイン済みのダッシュボード（`http://127.0.0.1:8082/`）がブラウザで開きます。Windows のタスクトレイ・macOS のメニューバーのアイコンの「ダッシュボードを開く」でも開けます。

```bash
swing dashboard open
# Docker Compose ではコンテナ内でブラウザを開けないので、表示された URL を開く
docker compose exec mirror swing dashboard open --no-browser
```

初めて起動したときは設定ファイルが無いので、セットアップ画面が出ます。次の 3 つを入力して送信すると設定ファイルができ、そのままミラーを始めます。

- **鍵**: 「新しい鍵を生成する」で SWING 専用の鍵を作るのがおすすめです。既存の鍵を貼り付けることも、秘密鍵をこのコンピュータに置かずにスマホの署名アプリ（NIP-46）を使うこともできます（[`docs/guide/security.md`](docs/guide/security.md#署名アプリnip-46で署名する)）
- **リレー**: 更新の通知とミラー対象リストのやりとりに使う Nostr relay。既定のままでも動きます
- **ストレージ上限**: ほかの人のサイトの保存に差し出してよいディスク容量（合計の既定 `100GiB`）。目安は [`docs/guide/operation.md`](docs/guide/operation.md) を参照してください

秘密鍵は設定ファイル（Docker Compose では `.env`）に平文で保存されます。ほかの用途の鍵とは分けてください。

### 3. 誰かのサイトをミラーする

ミラーしたい相手の Nostr の公開鍵（`npub1...`）を、ダッシュボードの **Sites** 画面の「ミラーに追加」に貼り付けて追加します。コマンドラインなら次のとおりです。

```bash
swing mirror add npub1alice...
```

これで「このアカウントのサイトを保存する」という表明が Nostr に出て、相手が公開しているサイトの最新版を取得し始めます。**Webring** 画面では、ミラー対象がさらにミラーしている相手をたどれるので、そこから気になる相手を追加することもできます。

### 4. 保存できたか確かめる

取得が終わると、Sites 画面や `swing sites` でそのサイトが `stored` になります。相手の更新は数分おきに確かめて取り込みます。

```bash
swing sites
```

保存したサイトは、ダッシュボードの **Desktop** 画面のリンク集や、ローカルのゲートウェイ（`http://localhost:8080/ipfs/<cid>/`）で読めます。

SWING は普段使いの PC で、使っている間だけ動かしても構いません。止めていた間の更新は次に起動したときに取り込みます（[`docs/guide/operation.md`](docs/guide/operation.md#常時起動しない場合に何が起きるか)）。

## 次に読むもの

- [`docs/guide/install.md`](docs/guide/install.md): OS ごとの入れ方の詳細、設定ファイルとデータの置き場所、サービス登録
- [`docs/guide/docker.md`](docs/guide/docker.md): Docker Compose での動かし方と、バイナリの `swing up` への移り方
- [`docs/guide/usage.md`](docs/guide/usage.md): ミラー対象の管理、保存状況の点検（`swing status`・`swing stats`）、ログ、ダッシュボードの各画面とカスタマイズ、マスコットの追加
- [`docs/guide/publish.md`](docs/guide/publish.md): 自分のサイトを `swing publish` で公開する、何人が保存しているかを見る、Webring を見る、内蔵ゲートウェイで配信する
- [`docs/site-guide.md`](docs/site-guide.md): IPFS で配りやすい静的サイトにするためのチェックリスト
- [`docs/guide/operation.md`](docs/guide/operation.md): どれくらい保存されるか、常時起動しない場合に何が起きるか、スペックと通信量、設定一覧
- [`docs/guide/security.md`](docs/guide/security.md): 外から何が見えるか、ダッシュボードを外の端末から使う、署名アプリ（NIP-46）で署名する

開発者・他の実装を作る方向け:

- [`docs/protocol.md`](docs/protocol.md): 実装非依存のプロトコル定義
- [`docs/architecture.md`](docs/architecture.md): 現在の実装の詳細なリファレンス
- [`docs/plan.md`](docs/plan.md): 初期実装計画。設計の背景や原則
- [`docs/extensions.md`](docs/extensions.md): 新しい kind や `d` タグの命名規約と予約表
- [`docs/todo.md`](docs/todo.md): 残タスクの一覧
- [`docs/log/`](docs/log/): 実装ログ
- [`docs/examples/publish.sh`](docs/examples/publish.sh): `ipfs` CLI と `nak` だけでプロトコルを再現する参考実装（サポート対象外）

## 含まれていないもの・今後の予定

以下は現時点では扱いません。既存の Nostr と IPFS にできるだけそのまま乗ることを優先しています。

- 中央管理サーバー / ユーザー登録 / 誰かが運営する Web サービス
- IPFS Cluster / private swarm / 独自 DHT
- 任意の CID を配信する公開 Gateway（内蔵ゲートウェイは設定したホストの DNSLink だけを配信します）
- レプリカの自動割当
- 高度なアクセス制御
- 決済
- 独自の Nostr Relay
- winget・AUR・nixpkgs での配布と、Homebrew の tap（`amane-katagiri/homebrew-swing`）の公開（Windows と macOS は確かめていない動作も残っています。範囲は [`docs/todo.md`](docs/todo.md)）

今後の拡張として、private mode（IP アドレスを隠したい参加者向けの別モード）などを検討しています。

## ライセンス

[MIT License](LICENSE)

ただし、同梱のマスコット「ゆれ子」（[`web/mascots/yureko/`](web/mascots/yureko/) のキャラクターとその画像）は MIT License の対象外で、Copyright (c) 2026 Amane Katagiri, all rights reserved とします。SWING とその改変版・再配布物（フォーク・パッケージ・コンテナイメージなど）の一部としてなら、ゆれ子のファイルを改変せずに複製・配布・表示できます。それ以外の利用（改変、別の作品やサービスでの使用など）には許諾が要ります。

ダッシュボードの Desktop 画面は同梱フォント PixelMplus12（[M+ FONT LICENSE](web/fonts/LICENSE-PixelMplus.txt)、Copyright (C) 2002-2013 M+ FONTS PROJECT）を使用しています。
