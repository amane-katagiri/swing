<!-- packaging/release/notes.sh で下書きのリリースに入れる本文のひな形。この行と <...> を書き換えて使う -->
<このリリースの要点を 1〜2 文で。利用者が対応しなければならない変更があれば、ここで見出しへのリンク付きで知らせる>

## 変更点

### <利用者から見た変更を 1 つずつ見出しにする>

<なぜ変えたか、どう使うか。利用者が対応しなければならないこと（設定・手順・互換性）は必ず書く>

### そのほかの修正

- <小さな修正を 1 行ずつ>

## 入手方法

入手方法は v0.1.0 と同じです。ファイル名とバージョンを `v<version>` に読み替えてください。Kubo は引き続き v0.43.1 です。

ダウンロードしたファイルは `SHA256SUMS` で破損がないか確かめられます。

```bash
sha256sum --check --ignore-missing SHA256SUMS
```

### Linux（x86_64・aarch64）

`curl -fsSL https://github.com/amane-katagiri/swing/releases/latest/download/install.sh | sh -s -- --service`

既に入れている場合も、同じコマンドでその場で更新します。

### Windows（x86_64）

`swing-v<version>-x86_64-pc-windows-msvc-setup.exe` を実行します。署名していないので SmartScreen の警告が出たら「詳細情報」→「実行」を押してください。

### macOS（Apple シリコン・Intel）

```bash
brew update
brew upgrade amane-katagiri/swing/swing
swing service install
```

`brew upgrade` だけでは、動いている `swing` は古いバージョンのままです。続けて `swing service install` を実行すると、新しいバージョンで起動し直します（`swing stop --restart` では入れ替わりません）。既定の場所以外の `swing.toml` を使っている場合は、`--config` を付けて実行してください。

### Docker Compose

`git pull` で更新してから、次を実行します。Kubo のゲートウェイの設定は `ipfs` コンテナの起動時に入れ直すので、`ipfs` も再起動します。

```bash
docker compose up -d --build
docker compose restart ipfs
```
