# Docker

[`architecture.md`](../architecture.md) の一部。

## Dockerfile

- builder に `Cargo.toml`・`Cargo.lock`・`build.rs`・`assets/`・`src/`・`tray/`・`web/` を COPY し、`cargo build --release` で `swing` だけをビルドする。`tray/` はビルドしないが、workspace のメンバーの `Cargo.toml` が無いと cargo がワークスペースを読めないので入れる。`build.rs` は Linux では何もしない。
- builder `rust:1.97-slim-trixie`、runtime `debian:trixie-slim`（glibc を揃えるため同じコードネーム）。
- runtime には `/usr/local/bin/swing` だけを置き、ユーザー `swing`（uid/gid 1000）で実行する。`/data` はそのユーザー所有の `VOLUME`。
- `WORKDIR /data`。`--config`／`SWING_CONFIG` のどちらも無いときに `resolve_config_path` が返す `<cwd>/swing.toml`（[`../architecture.md#設定と環境変数`](../architecture.md#設定と環境変数)）がこの `/data` の下（volume の中）になるようにするため。これが無いと `swing.toml` はコンテナのルート直下に作られ、コンテナを作り直すたびに消える。
- `ENTRYPOINT ["swing"]`、`CMD ["up"]`（[`up.md`](up.md)）。`mirror` サービスは `SWING_KUBO_MANAGED=false` を固定で渡すので、コンテナの中では Kubo を子プロセスにせず外部の `ipfs` サービスに対して動く（`swing up` の unmanaged 経路）。
- `docker/release.Dockerfile` は release ワークフローが ghcr.io に push するイメージ用。runtime ステージだけで、ビルド済みの musl バイナリを `<TARGETARCH>/swing` から入れる。それ以外はこの Dockerfile の runtime と同じ（[`../architecture.md#ビルドとリリース`](../architecture.md#ビルドとリリース)）。`compose.yaml` はこれを使わずルートの `Dockerfile` からビルドする。

## compose.yaml

| サービス | 内容 |
|---|---|
| `ipfs` | `ipfs/kubo:v0.43.1`（[Kubo のバージョン](kubo.md#kubo-のバージョン)）。イメージ既定の `command` に `--enable-gc` を足す。volume `ipfs-data:/data/ipfs` と `./docker/kubo-init.d:/container-init.d:ro`。公開ポートは `4001/tcp`・`4001/udp` と、Gateway の `${SWING_KUBO_GATEWAY_BIND:-127.0.0.1:8080}:8080`。healthcheck は `ipfs id` |
| `mirror` | `build: .`、`env_file: .env`。`build: .` の直後に、release ワークフローが push するイメージ（`ghcr.io/amane-katagiri/swing`。タグを書かないので `latest`）の `image:` をコメントアウトして置いてある（`build` と入れ替えて使う）。`SWING_IPFS_API=http://ipfs:5001`、`SWING_STATE_DIR=/data`、`SWING_DASHBOARD_LISTEN=${SWING_DASHBOARD_LISTEN:-0.0.0.0:8082}`、`SWING_KUBO_MANAGED=false`、`SWING_GATEWAY_LISTEN=${SWING_GATEWAY_LISTEN:-off}`、`SWING_GATEWAY_UPSTREAM=http://ipfs:8080`、`RUST_LOG=info`。volume `swing-data:/data`。公開ポート `${SWING_DASHBOARD_BIND:-127.0.0.1:8082}:8082` と `${SWING_GATEWAY_BIND:-127.0.0.1:8081}:8081`。`ipfs` が healthy になるのを待つ |

2 サービスとも `restart: unless-stopped`。Caddy による専用の `gateway` サービス（旧 `gateway` プロファイル、`docker/caddy/`）は廃止した。内蔵 gateway（[`gateway.md`](gateway.md)）が `mirror` コンテナの中で同じ役割を果たす。`SWING_GATEWAY_LISTEN` の既定は `off` なので、有効にする場合は `.env` で `SWING_GATEWAY_LISTEN=0.0.0.0:8081`（コンテナ内バインド）と `SWING_GATEWAY_HOSTS` を設定する。ホスト側の 8081 ポート自体は `[gateway].listen` の設定に関わらず常に compose がマッピングする（`SWING_GATEWAY_BIND` で変更・変えなければ `127.0.0.1:8081` に固定で公開される。gateway を使わない構成でもポートだけは空いている、という compose 側のトレードオフ）。

外部ネットワークに出ないデモ用の重ね合わせ（`docker/demo/`）は [`docker/demo/README.md`](../../docker/demo/README.md) を参照。`.env` は mirror の `env_file` と、compose の変数展開の両方に使われる。`.env`（と compose が固定で渡す環境変数）で設定したキーはすべて `Source::Env` になるので、ダッシュボードの Settings／Setup 画面ではロック表示（編集不可）になる（[`dashboard.md`](dashboard.md)）。compose 構成では `nostr.relays` や保存上限などを `.env` に書くほど、ダッシュボードから変更できる項目が減っていく。

## ダッシュボード（compose）

`mirror` サービスはコンテナ内で `SWING_DASHBOARD_LISTEN=${SWING_DASHBOARD_LISTEN:-0.0.0.0:8082}` を待ち受ける。`.env` に `SWING_DASHBOARD_LISTEN` を書けばそれが使われる。`config::parse_dashboard_listen` は `SocketAddr` としてパースするだけで `off` は受け付けない（`[gateway].listen` など他の listen 系キーとは違い、ダッシュボード自体を無効にする設定は無い）。Web UI の配信だけを止めたいなら `SWING_DASHBOARD_UI=false`（`/api/*` は残る）。ホストにどう公開するかは別の変数 `SWING_DASHBOARD_BIND`（既定 `127.0.0.1:8082`）で決める。`SWING_KUBO_GATEWAY_BIND` と同じ流儀で、**`SWING_DASHBOARD_BIND` は compose 専用の変数展開にしか使われず、Rust 側（`swing` バイナリ）はこの名前を読まない**。

ログインは `docker compose exec mirror swing dashboard open --no-browser` で出た URL を開くか、コードをログイン画面に貼る。URL はコンテナ内の待ち受けポート（既定 8082）で作るので、`SWING_DASHBOARD_BIND` でホスト側のポートやアドレスを変えた場合は `.env` に `SWING_DASHBOARD_PUBLIC_URL`（例 `http://127.0.0.1:18082`）を書いて合わせる。compose は `SWING_DASHBOARD_BIND` から自動では作らない（`0.0.0.0:8082` のような bind 用のアドレスはブラウザから開く URL にならないため）。トークンは `swing-data` volume の `/data/dashboard.token` に置かれる。

ダッシュボードの Publish 画面はブラウザから直接フォルダをアップロードする方式（`POST /api/publish/upload`。上限は `SWING_DASHBOARD_MAX_UPLOAD`、既定 2GB）だけを使うため、`mirror` コンテナに volume をマウントする必要はない。詳しくは [`dashboard.md`](dashboard/http-api.md#post-apipublishupload) を参照。CLI の `swing publish` をコンテナで使う場合は `docker compose run --rm -v "$PWD/public:/site" mirror publish ...` のような一時マウントでよい。

## Kubo の設定

`ipfs` には次の環境変数を渡し、Kubo イメージが起動のたびに実行する `docker/kubo-init.d/001-swing-config.sh` で `ipfs config` に設定する。

| 環境変数 | 設定先 | 既定 |
|---|---|---|
| `SWING_KUBO_STORAGE_MAX` | `Datastore.StorageMax`（GC の基準） | `SWING_MAX_TOTAL_STORAGE`、それも無ければ `100GB` |
| `SWING_KUBO_PROVIDE_STRATEGY` | `Provide.Strategy` | `pinned+mfs` |
| `SWING_GATEWAY_HOSTS` | `Gateway.PublicGateways` | 空 |

`SWING_GATEWAY_HOSTS` 以外の値が空ならコンテナは起動しない。ほかに毎回 `Gateway.NoFetch=true` と `Gateway.NoDNSLink=true` を設定する。

`Provide.Strategy` に `mfs` か `all` を含めないと、MFS にしか無いサイトが DHT に告知されない。知らない値を与えると daemon が起動しない。

この設定内容（`Datastore.StorageMax`・`Provide.Strategy`・`Gateway.NoFetch`・`Gateway.NoDNSLink`・`Gateway.PublicGateways`）は、`swing up` が `[kubo].managed = true` で Kubo を子プロセスとして起動する際にも `kubo::apply_config` が同じ値を適用する（シェルスクリプトと Rust の実装を両方持つのは、compose の外部 Kubo コンテナと `swing up` の管理下 Kubo という 2 つの起動経路があるため）。設定するキーの一覧と意味は [`up.md#適用する-kubo-設定kuboapply_config`](up.md#適用する-kubo-設定kuboapply_config) を参照（そちらが正本）。

## Gateway

Kubo の Gateway は `NoFetch` なので、ローカルにあるブロックだけを返す。ブロックが無いときの応答（Kubo 0.43.1 / boxo 0.43.0）:

| 状況 | 応答 |
|---|---|
| パス Gateway で `/ipfs/<cid>/` のルートブロックが無い | 404 |
| 上記以外（サブパス付き、サブドメイン・DNSLink のルート）で、途中までのブロックが無い | 500（`skip: ...`） |
| ファイルの途中のブロックが無い | 200 を返した後で本文が途切れる |
| サイトにあるブロックで辿れて、パスが無い | 404 |
| `Cache-Control: only-if-cached` | パスの最後のブロックがあれば 200、無ければ 412。その下の DAG が揃っているかは見ない |

オフラインでは boxo の fetcher が「ブロックが無い」を `traversal.SkipMe` に置き換え、Gateway がそれを not found と判定しないので 500 になる。サブドメインと DNSLink はルートでも `_redirects` を探すためにサブパスを辿るので 500 になる。

`127.0.0.1:8080`（Kubo 直接）はすべてのパスを受け付ける（`localhost` ではサブドメイン Gateway（`/ipfs/<cid>` は `<cid>.ipfs.localhost` へリダイレクト）、`127.0.0.1` ではパス Gateway）。`SWING_GATEWAY_HOSTS` はカンマ区切りのホスト名（小文字英数字・`-`・`.`）。起動スクリプトが各ホストを `{"Paths": [], "UseSubdomains": false, "NoDNSLink": false}` で `Gateway.PublicGateways` に入れる。これらのホストでは DNSLink（`_dnslink.<host>`）の内容だけを返し、`/ipfs/`・`/ipns/`・`/routing/v1` は 404。不正な名前があると `ipfs` は起動しない。

Kubo は `Host` と `X-Forwarded-Host` をそのまま信じるので、Kubo の 8080 を外に出してはいけない。compose では内蔵 gateway（[`gateway.md`](gateway.md)。`mirror` コンテナの中で `SWING_GATEWAY_UPSTREAM=http://ipfs:8080` としてこの Kubo にプロキシする）を前段に置き、ホスト名での振り分けと `X-Forwarded-*` の付け直しをそちらに任せる。内蔵 gateway 自体の Host 判定・転送するヘッダー・404/502 の挙動は [`gateway.md`](gateway.md) を参照。

## compose から `swing up` への移行

compose の 2 つの volume をホストにコピーすれば、同じ Kubo（PeerID・ブロック・MFS）と同じ agent の状態のまま `swing up`（`[kubo].managed = true`）に移れる。

| volume | 中身 | 移し先 |
|---|---|---|
| `swing-data`（`mirror:/data`） | `state.json`・`dashboard.token`・`remote-signer.json`（署名アプリを使っている場合）・`swing.toml`（ダッシュボードのセットアップや Settings で書いた場合） | `<dir>/data`。`swing.toml` だけ `<dir>/swing.toml` へ |
| `ipfs-data`（`ipfs:/data/ipfs`） | Kubo の repo | `<dir>/data/kubo` |

`<dir>` は `swing.toml` を置くディレクトリ。`state_dir` の既定 `./data` と `[kubo].repo` の既定 `<state_dir>/kubo`（[`../architecture.md#設定と環境変数`](../architecture.md#設定と環境変数)）にそのまま合う配置で、`swing service install` も作業ディレクトリを `swing.toml` の親にする（[`service.md`](service.md)）。

```sh
cd <このリポジトリ>                       # compose.yaml のあるディレクトリ
docker compose stop                       # volume は消さない
dest=~/swing                              # <dir>。data はまだ作らない
mkdir -p "$dest"
docker compose cp mirror:/data "$dest/data"
docker compose cp ipfs:/data/ipfs "$dest/data/kubo"
[ -f "$dest/data/swing.toml" ] && mv "$dest/data/swing.toml" "$dest/"
```

- `docker compose cp` は止まっているコンテナにも使え、コピーしたファイルはコマンドを実行したユーザーの所有になる（コンテナ内の uid 1000 は引き継がない）。Windows（PowerShell）でも `docker compose cp` はそのまま使える（`$dest` を `"$HOME\swing"` などに、最後の行を `Move-Item` に読み替える）。コピー先の `data` が既にあると `data/data` の下に入るので、先に作らない。
- `.env` の設定を `<dir>/swing.toml` に移す。`swing service install` で登録したサービスは `.env` を読まない。キー名の対応は [`swing.example.toml`](../../swing.example.toml) の各行のコメント（`SWING_...` がその行の環境変数）。compose だけの変数は次のように扱う。

| `.env`・`compose.yaml` の変数 | `swing up` での扱い |
|---|---|
| `SWING_IPFS_API`・`SWING_STATE_DIR`・`SWING_KUBO_MANAGED`・`SWING_GATEWAY_UPSTREAM`（`compose.yaml` が固定で渡す） | 書かない（既定値で managed の構成になる） |
| `SWING_DASHBOARD_LISTEN`（コンテナ内の待ち受け） | 書かない。代わりに `SWING_DASHBOARD_BIND` を変えていたらその値を `[dashboard].listen` に |
| `SWING_KUBO_GATEWAY_BIND` | 変えていたら `[kubo].gateway_listen` に |
| `SWING_GATEWAY_LISTEN`（`off` 以外にしていた場合）と `SWING_GATEWAY_BIND` | `SWING_GATEWAY_BIND` の値を `[gateway].listen` に |
| `SWING_DASHBOARD_PUBLIC_URL` | ホスト側のポートに合わせて書いていただけなら書かない |

- ホストの `ipfs` は compose のイメージと同じ 0.43.1 にする（[`kubo.md#kubo-のバージョン`](kubo.md#kubo-のバージョン)）。古い Kubo は新しい repo を開けない。新しい Kubo は `--migrate=true` で repo を移行するので、その後は compose のイメージに戻せない。
- Kubo の設定は、`swing up` が起動のたびに `apply_config` で上書きする（[`up.md#適用する-kubo-設定kuboapply_config`](up.md#適用する-kubo-設定kuboapply_config)）。compose の `Addresses.API`（`/ip4/0.0.0.0/tcp/5001`）と `Addresses.Gateway`（`/ip4/0.0.0.0/tcp/8080`）は `127.0.0.1` に戻る。`Addresses.Swarm` は `[kubo].swarm_port` を設定しない限りコピーした値（4001）のまま。PeerID・keystore・`Bootstrap` など、`apply_config` が触らないキーはコピーした値のまま。
- 4001・8080・8082 は compose と同じポートなので、compose を止めてから `swing up` を起動する。
- `swing up` が動いて `swing status` で保存済みの版が `[ok]` になることを確かめたら、`docker compose down -v` で volume を消す。同じ PeerID と同じ鍵で 2 つ動かすことになるので、移行後に compose をまた起動しない。
