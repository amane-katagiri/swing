# compose から swing up への移行手順

配布前に確かめることの 1 つ「既存 compose 利用者が `ipfs-data` から `swing up`（managed Kubo）へ移行する手順」を作り、デモ環境のデータで実際に移して確かめた。手順は [`../architecture/docker.md#compose-から-swing-up-への移行`](../architecture/docker.md#compose-から-swing-up-への移行)。

## 決めたこと

- 移行は「2 つの volume をホストにコピーするだけ」にした。移行用のサブコマンドやコードは作らない。`state_dir` の既定 `./data` と `[kubo].repo` の既定 `<state_dir>/kubo` にそのまま合う配置（`<dir>/swing.toml`・`<dir>/data/`・`<dir>/data/kubo/`）へ置けば、設定で場所を指定しなくて済み、`swing service install` の作業ディレクトリ（`swing.toml` の親）とも合う。
- コピーは `docker compose cp` を使う。止まったコンテナにも使え、コピーしたファイルは実行したユーザーの所有になるので `chown` が要らない。`docker run -v ... tar` のパイプは PowerShell 5 でバイナリが壊れるので、Windows でも同じ手順にできる `docker compose cp` にした。
- `.env` はサービス登録した `swing up` が読まないので、`swing.toml` へ移す。compose だけの変数（`*_BIND` と `compose.yaml` が固定で渡すもの）の扱いを表にした。
- Kubo の設定は `apply_config` が起動のたびに上書きするので、repo の `config` は手で直さなくてよい。`Addresses.Swarm` と PeerID はコピーした値のまま残る。
- 移行後に compose を再び起動しないよう書いた。同じ PeerID の Kubo と同じ鍵の agent が 2 つ動くことになる。

## 検証

デモ環境（`docker/demo/demo.sh up --seed`）で agent が alice・bob・carol の 4 サイトを保存した状態を作り、次を行った。

1. `demo.sh stop ipfs mirror` で止め、`demo.sh cp mirror:/data <scratch>/data`、`demo.sh cp ipfs:/data/ipfs <scratch>/data/kubo` でコピー。所有者はホストのユーザー（miki）になった。ディレクトリのモードは Kubo イメージのまま `2755`（setgid 付き）で、動作に影響は無かった。`swing.toml` はデモが `.env` 相当の `demo.env` だけで動いているので無く、`demo.env` の値から `swing.toml`（`secret_key`・`relays`・`mirror_set`・`max_total_storage`・`nip05`・`dashboard.listen`・`dashboard.gateway`）を書いた。
2. 外部ネットワークに出さないため、ホストではなく `swing-demo-mirror` イメージのコンテナを `--network swing-demo_isolated`・`--user 1000:1000` で起動し、コピーしたディレクトリをバインドマウントして `swing up --config /mig/swing.toml`（managed）を動かした。Kubo は `ipfs/kubo:v0.43.1` イメージから取り出した `ipfs`（0.43.1）を使った。
3. 結果:
   - Kubo は repo version 18 のまま起動し、PeerID は compose のときと同じ（`12D3KooWLo63...`）。
   - `apply_config` で `Addresses.API` が `/ip4/127.0.0.1/tcp/<動的>`、`Addresses.Gateway` が `/ip4/127.0.0.1/tcp/8080`、`Datastore.StorageMax` が `2147483648` になり、`Addresses.Swarm` は 4001 のまま。
   - `loaded state path=./data/state.json sites=4`。4 サイトとも `skip reason=duplicate_cid` で、取得し直さなかった。
   - `/swing/agent` の MFS はそのまま。`swing status` はコピーした `dashboard.token` で API に通り、4 版とも `[ok]`。`ipfs cat` でサイトの `index.html` を読めた。
   - `swing stop` でグレースフルに止まった。
4. 後片付けとして、試験用コンテナを消し、`demo.sh down` でデモ環境を落とし、コピーも消した。

移らないものは見つからなかった。Kubo の起動時に AutoConf の取得失敗（隔離ネットワークなので名前解決できない）が出たが、フォールバックの設定で起動した。コピーした repo の `Bootstrap` は `auto` のままなので、実際の移行ではネットワークに出れば普通に取得する。

## 確かめていないこと

- `remote-signer.json` とダッシュボードが書いた `swing.toml` を含む volume からの移行。どちらも `swing-data` の中のファイルで、手順どおりコピーすれば移るが、今回のデモには無かった。
- macOS・Windows のホストで、実際にホストの `swing up` を動かしての確認（今回はコンテナの中で動かした）。
- `swing service install` で登録した後の起動。
