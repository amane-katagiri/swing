# Docker Compose で動かす

Docker Compose で SWING を動かす手順と、あとからバイナリの `swing up` に移る方法をまとめます。鍵の用意や設定ファイルの考え方は [`install.md`](install.md) と共通です。

## 起動する

`.env` ファイルを作り、必要な項目を設定します。

```bash
cp .env.example .env
```

`SWING_NOSTR_SECRET_KEY` を埋めるには、次のように鍵を生成するのが手軽です。

```bash
docker compose run --rm mirror key generate
```

`.env.example` は `SWING_NOSTR_SECRET_KEY` 以外すべてコメントアウトされていて、コメントを外さない限りコードの既定値がそのまま使われます（relay や保存上限も含め、既定値のまま起動できます）。行ごとの意味は [`.env.example`](../../.env.example) 自体のコメントと、[設定一覧](operation.md#設定一覧) を参照してください。`SWING_NOSTR_RELAYS` は実際に自分が使う relay に、`SWING_MAX_TOTAL_STORAGE` は保存したい容量に、必要に応じてコメントを外して書き換えてください。最低限、`SWING_NOSTR_SECRET_KEY` は必ず自分の値に書き換えてください（空のままでも起動でき、その場合はダッシュボードのセットアップ画面から鍵を生成・保存できます。「[セットアップモード](install.md#セットアップモード)」を参照）。`.env` に書いた値はダッシュボードから編集できなくなる（[設定一覧](operation.md#設定一覧)）ので、コメントを外すのは実際に固定したい項目だけにしてください。

`.env` を用意できたら、コンテナを起動します。

```bash
docker compose up -d
```

`ipfs`（Kubo）と `mirror`（このツール本体、`swing up` を実行します。Kubo は `ipfs` コンテナ側を使うため `SWING_KUBO_MANAGED=false` を固定で渡しています）の 2 つのコンテナが立ち上がります。外部に公開されるのは IPFS の swarm 用ポート（`4001/tcp`・`4001/udp`）だけです。Kubo の RPC（5001）はホストにも公開されず、ゲートウェイ（8080）はホストの `127.0.0.1:8080` だけに公開されます。

`mirror` は既定で手元のソースからイメージをビルドします。リリースごとに公開しているイメージを使う場合は、`compose.yaml` の `mirror` の `build: .` をコメントアウトし、その下の `image: ghcr.io/amane-katagiri/swing` のコメントを外してください（最新のリリースが使われます。バージョンを固定するなら `ghcr.io/amane-katagiri/swing:0.1.0` のように書きます）。

## Docker Compose からバイナリの `swing up` に移る

compose の 2 つの volume をホストにコピーすれば、同じ Kubo（PeerID・ブロック・MFS）と同じ agent の状態のまま `swing up`（`[kubo] managed = true`）に移れます。`swing-data`（`state.json`・`dashboard.token`・`remote-signer.json`・ダッシュボードで書いた `swing.toml`）を `<dir>/data` に、`ipfs-data`（Kubo の repo）を `<dir>/data/kubo` に置き、`swing.toml` だけ `<dir>` に出します。`<dir>` は `swing.toml` を置くディレクトリで、`[agent] state_dir` と `[kubo] repo` の既定値にそのまま合います。

```sh
docker compose stop                       # volume は消さない
dest=~/swing                              # <dir>。data はまだ作らない（あると data/data に入る）
mkdir -p "$dest"
docker compose cp mirror:/data "$dest/data"
docker compose cp ipfs:/data/ipfs "$dest/data/kubo"
[ -f "$dest/data/swing.toml" ] && mv "$dest/data/swing.toml" "$dest/"
```

Windows（PowerShell）でも `docker compose cp` はそのまま使えます（`$dest` を `"$HOME\swing"` などに、最後の行を `Move-Item` に読み替えてください）。コピーしたファイルはコマンドを実行したユーザーの所有になります。

`<dir>` が[ユーザーごとの既定の場所](install.md#設定ファイルとデータの置き場所)でなければ、この後の `swing up`・`swing status`・`swing service install` などは `export SWING_CONFIG="$dest/swing.toml"`（PowerShell では `$env:SWING_CONFIG = "$dest\swing.toml"`）を設定したシェルで実行するか、`--config "$dest/swing.toml"` を付けて実行します。

- `.env` の設定は `<dir>/swing.toml` に移します（サービスとして登録した `swing up` は `.env` を読みません）。キー名の対応は [`swing.example.toml`](../../swing.example.toml) の各行のコメントにあります。`SWING_IPFS_API`・`SWING_STATE_DIR`・`SWING_KUBO_MANAGED`・`SWING_GATEWAY_UPSTREAM`・`SWING_DASHBOARD_LISTEN` は書きません。`SWING_DASHBOARD_BIND`・`SWING_KUBO_GATEWAY_BIND`・`SWING_GATEWAY_BIND` を変えていたら、その値をそれぞれ `[dashboard] listen`・`[kubo] gateway_listen`・`[gateway] listen` に書きます（gateway は `SWING_GATEWAY_LISTEN` を `off` 以外にしていた場合だけ）。`SWING_DASHBOARD_PUBLIC_URL` はホスト側のポートに合わせていただけなら要りません。
- ホストの `ipfs` は compose のイメージと同じ v0.43.1 にします。古い Kubo は新しい repo を開けず、新しい Kubo は repo を移行するので、その後は compose に戻せません。
- Kubo の設定は `swing up` が起動のたびに上書きします（API とゲートウェイは `127.0.0.1` で待ち受け直します）。PeerID や `Bootstrap` などはコピーした値のままです。
- ポートは compose と同じなので、compose を止めてから `swing up` を起動します。`swing status` で保存済みの版が `[ok]` になることを確かめたら、`docker compose down -v` で volume を消してください。同じ PeerID と鍵で 2 つ動かすことになるので、移行後に compose をまた起動しないでください。

