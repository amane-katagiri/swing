# Docker

[`../architecture.md`](../architecture.md) の一部。

## Dockerfile

- builder に `Cargo.toml`・`Cargo.lock`・`build.rs`・`assets/`・`src/`・`tray/`・`web/` を COPY し、`cargo build --release --locked` で `swing` だけをビルドする（`tray/` は workspace のメンバーなので入れるがビルドしない）。
- builder `rust:1.97-slim-trixie`、runtime `debian:trixie-slim`。
- runtime には `/usr/local/bin/swing` だけを置き、ユーザー `swing`（uid/gid 1000）で実行する。`/data` はそのユーザー所有の `VOLUME`。
- `WORKDIR /data`。`--config`／`SWING_CONFIG` のどちらも無いときに `resolve_config_path` が返す `<cwd>/swing.toml`（[`config.md`](config.md)）がこの `/data`（volume の中）になる。
- `ENTRYPOINT ["swing"]`、`CMD ["up"]`（[`up.md`](up.md)）。
- `ENV SWING_NO_PORT_SHIFT=true`。セットアップモードでもダッシュボードと Kubo の gateway のポートをずらさない（[`up.md#セットアップモードでのポートの調整`](up.md#セットアップモードでのポートの調整)）。`CMD` を上書きしても効く。
- `docker/release.Dockerfile` は release ワークフローが ghcr.io に push するイメージ用。1 ステージで、ビルド済みの musl バイナリを `<TARGETARCH>/swing` から入れる。それ以外はこの Dockerfile の runtime と同じ（[`release.md`](release.md)）。`compose.yaml` はこれを使わずルートの `Dockerfile` からビルドする。

## compose.yaml

| サービス | 内容 |
|---|---|
| `ipfs` | `ipfs/kubo:v0.43.1`（[Kubo のバージョン](kubo.md#kubo-のバージョン)）を `daemon --migrate=true --agent-version-suffix=docker --enable-gc` で動かす（`swing up` が管理する Kubo とは agent version で見分けられる。[`kubo.md#デーモンの起動kubodaemonspawn`](kubo.md#デーモンの起動kubodaemonspawn)）。RPC（5001）はホストに公開せず compose の内部ネットワーク（`mirror` からは `http://ipfs:5001`）だけで待ち受け、認証は設定しないので、同じネットワークのコンテナはすべて RPC を使える。healthcheck は `ipfs id` |
| `mirror` | `swing up` の unmanaged 経路で動く（`SWING_KUBO_MANAGED=false` を固定で渡す）。Kubo は子プロセスにせず、外部の `ipfs` サービスの API と Gateway を使う。状態は volume `swing-data`（`/data`）。ダッシュボードと内蔵 gateway のホスト側の公開アドレスは `SWING_DASHBOARD_BIND`・`SWING_GATEWAY_BIND` で決める（下記）。`build: .` の直後に、release ワークフローが push するイメージ（`ghcr.io/amane-katagiri/swing`）の `image:` がコメントアウトしてある（`build` と入れ替えて使う）。`ipfs` が healthy になるのを待つ |

環境変数・ポート・volume の値は [`../../compose.yaml`](../../compose.yaml) が正本。2 サービスとも `restart: unless-stopped`。内蔵 gateway（[`gateway.md`](gateway.md)）は `mirror` コンテナの中で動き、`SWING_GATEWAY_UPSTREAM=http://ipfs:8080` で `ipfs` の Kubo の Gateway にプロキシする。

コンテナ内の待ち受け（`SWING_DASHBOARD_LISTEN`・`SWING_GATEWAY_LISTEN`）とホスト側の公開アドレス（`SWING_DASHBOARD_BIND`・`SWING_GATEWAY_BIND`・`SWING_KUBO_GATEWAY_BIND`。それぞれ `mirror` の 8082・`mirror` の 8081・`ipfs` の 8080 をホストに出す）は別々に決める。`*_BIND` は compose の変数展開だけに使い、`swing` は読まない。ホスト側のポートは `[gateway].listen` に関わらず常にマッピングされる。compose での `SWING_GATEWAY_LISTEN` の既定は `off`（有効にする手順は README の「[自分のサイトをゲートウェイで配信する](../../README.md#自分のサイトをゲートウェイで配信する)」）。

外部ネットワークに出ないデモ用の重ね合わせ（`docker/demo/`）は [`../../docker/demo/README.md`](../../docker/demo/README.md) を参照。`.env` は mirror の `env_file` と、compose の変数展開の両方に使われる。`.env`（と compose が固定で渡す環境変数）で設定したキーはすべて `Source::Env` になるので、ダッシュボードの Settings／Setup 画面ではロック表示（編集不可）になる（[`config.md`](config.md)）。

## ダッシュボード（compose）

`SWING_DASHBOARD_LISTEN` に `off` は無い（値の形式は [`dashboard.md`](dashboard.md#設定dashboard)。UI を止めるなら `ui = false`）。

コンテナ内で `swing dashboard open` が出す URL は、`public_url` が無ければコンテナ内の待ち受け（既定 8082）から組み立てる。compose は `SWING_DASHBOARD_BIND` から `SWING_DASHBOARD_PUBLIC_URL` を作らない。トークンは `swing-data` volume の `/data/dashboard.token` に置かれる。ログインの手順は README の「[ダッシュボード](../../README.md#ダッシュボード)」。

ダッシュボードの Publish 画面はブラウザからフォルダをアップロードする（`POST /api/publish/upload`。上限は `SWING_DASHBOARD_MAX_UPLOAD`、既定 2 GiB）ので、`mirror` コンテナにサイトの volume は要らない（[`dashboard/http-api/publish.md`](dashboard/http-api/publish.md#post-apipublishupload)）。CLI の `swing publish` をコンテナで使う手順は README の「[自分のサイトを公開する](../../README.md#自分のサイトを公開する)」。

## Kubo の設定

`ipfs` には次の環境変数を渡し、Kubo イメージが起動のたびに実行する `docker/kubo-init.d/001-swing-config.sh` で `ipfs config` に設定する。

| 環境変数 | 設定先 | 既定 |
|---|---|---|
| `SWING_KUBO_STORAGE_MAX` | `Datastore.StorageMax`（GC の基準）。`100GiB` のような文字列のまま渡し、Kubo が解釈する（下記） | `SWING_MAX_TOTAL_STORAGE`、それも無ければ `100GiB` |
| `SWING_KUBO_PROVIDE_STRATEGY` | `Provide.Strategy` | `pinned+mfs` |
| `SWING_GATEWAY_HOSTS` | `Gateway.PublicGateways` | 空 |

`SWING_GATEWAY_HOSTS` 以外の値が空の場合と、`SWING_GATEWAY_HOSTS`（`,` 区切り、空の要素は捨てる）に [`[gateway].hosts` の検証](gateway.md#hosts-の検証)の文字の規則を満たさないホスト名がある場合はコンテナは起動しない。同じ検証のうちダッシュボードのホスト名との重なりと、空のときのエラーは確かめない。ほかに毎回 `Gateway.NoFetch=true` と `Gateway.NoDNSLink=true` を設定する。`Addresses.*` は設定しない（Kubo イメージの既定のまま）。

`SWING_KUBO_STORAGE_MAX` は、managed の `swing up` では swing の容量パーサが 1024 基数のバイト数にしてから渡すのに対し、compose では Kubo が文字列のまま解釈する。Kubo は `GiB` 系を 1024 基数、`GB` 系を 10 進で読むので、`GiB` 系で書けば両者は同じ値になる。`GB` 系で書くと compose だけ 10 進になり、managed より約 7% 小さくなる（`100GB` なら 10^11 バイトと 100×2^30 バイト）。

キーの意味は managed の `swing up` が適用する設定と同じで、正本は [`kubo.md#適用する-kubo-設定kuboapply_config`](kubo.md#適用する-kubo-設定kuboapply_config)。

## compose から `swing up` への移行

volume をホストにコピーしてバイナリの `swing up` に移る手順は README の「[Docker Compose からバイナリの `swing up` に移る](../../README.md#docker-compose-からバイナリの-swing-up-に移る)」を参照。
