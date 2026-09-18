# Docker

[`architecture.md`](../architecture.md) の一部。

## Dockerfile

- builder `rust:1.97-slim-trixie`、runtime `debian:trixie-slim`（glibc を揃えるため同じコードネーム）。
- runtime には `/usr/local/bin/swing` だけを置き、ユーザー `swing`（uid/gid 1000）で実行する。`/data` はそのユーザー所有の `VOLUME`。
- `ENTRYPOINT ["swing"]`、`CMD ["agent"]`。

## compose.yaml

| サービス | 内容 |
|---|---|
| `ipfs` | `ipfs/kubo:v0.43.1`（[Kubo のバージョン](kubo.md#kubo-のバージョン)）。イメージ既定の `command` に `--enable-gc` を足す。volume `ipfs-data:/data/ipfs` と `./docker/kubo-init.d:/container-init.d:ro`。公開ポートは `4001/tcp`・`4001/udp` と、Gateway の `${SWING_KUBO_GATEWAY_BIND:-127.0.0.1:8080}:8080`。healthcheck は `ipfs id` |
| `mirror` | `build: .`、`env_file: .env`。`SWING_IPFS_API=http://ipfs:5001`、`SWING_STATE_DIR=/data`、`SWING_DASHBOARD_LISTEN=${SWING_DASHBOARD_LISTEN:-0.0.0.0:8082}`、`RUST_LOG=info`。volume `swing-data:/data`。公開ポート `${SWING_DASHBOARD_BIND:-127.0.0.1:8082}:8082`。`ipfs` が healthy になるのを待つ |
| `gateway` | profile `gateway` のときだけ起動する。`caddy:2.11.4-alpine`。`./docker/caddy:/etc/caddy:ro`、`${SWING_GATEWAY_BIND:-127.0.0.1:8081}:80`。`ipfs` が healthy になるのを待つ |

3 サービスとも `restart: unless-stopped`。`.env` は mirror の `env_file` と、compose の変数展開の両方に使われる。

## ダッシュボード（compose）

`mirror` サービスはコンテナ内で `SWING_DASHBOARD_LISTEN=${SWING_DASHBOARD_LISTEN:-0.0.0.0:8082}` を待ち受ける。`.env` に `SWING_DASHBOARD_LISTEN` を書けばそれが使われる（`off` にすればコンテナでもダッシュボードを無効化できる）。ホストにどう公開するかは別の変数 `SWING_DASHBOARD_BIND`（既定 `127.0.0.1:8082`）で決める。`SWING_KUBO_GATEWAY_BIND` と同じ流儀で、**`SWING_DASHBOARD_BIND` は compose 専用の変数展開にしか使われず、Rust 側（`swing` バイナリ）はこの名前を読まない**。

ダッシュボードの Publish 画面はブラウザから直接フォルダをアップロードする方式（`POST /api/publish/upload`。上限は `SWING_DASHBOARD_MAX_UPLOAD`、既定 2GB）だけを使うため、`mirror` コンテナに volume をマウントする必要はない。パスを指定する `POST /api/publish`（CLI の `swing publish` と同じ `dir` 方式）は API としては残っているが、UI 調整でダッシュボードの画面からは呼ばなくなった。`POST /api/publish` を直接使う場合（スクリプトなどから叩く場合）だけ、サイトのディレクトリを `mirror` コンテナから見える場所に置く必要がある。CLI の `docker compose run --rm -v "$PWD/public:/site" mirror publish ...` のような一時マウントは使えないため、`compose.yaml` の `mirror` サービスにあらかじめ `volumes` でサイトのディレクトリをマウントしておく。詳しくは [`dashboard.md`](dashboard.md#post-apipublish) と [`dashboard.md`](dashboard.md#post-apipublishupload) を参照。

## Kubo の設定

`ipfs` には次の環境変数を渡し、Kubo イメージが起動のたびに実行する `docker/kubo-init.d/001-swing-config.sh` で `ipfs config` に設定する。

| 環境変数 | 設定先 | 既定 |
|---|---|---|
| `SWING_KUBO_STORAGE_MAX` | `Datastore.StorageMax`（GC の基準） | `SWING_MAX_TOTAL_STORAGE`、それも無ければ `100GB` |
| `SWING_KUBO_PROVIDE_STRATEGY` | `Provide.Strategy` | `pinned+mfs` |
| `SWING_GATEWAY_HOSTS` | `Gateway.PublicGateways` | 空 |

`SWING_GATEWAY_HOSTS` 以外の値が空ならコンテナは起動しない。ほかに毎回 `Gateway.NoFetch=true` と `Gateway.NoDNSLink=true` を設定する。

`Provide.Strategy` に `mfs` か `all` を含めないと、MFS にしか無いサイトが DHT に告知されない。知らない値を与えると daemon が起動しない。

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

| 入口 | 受け付けるもの |
|---|---|
| `127.0.0.1:8080`（Kubo 直接） | すべて。`localhost` ではサブドメイン Gateway（`/ipfs/<cid>` は `<cid>.ipfs.localhost` へリダイレクト）、`127.0.0.1` ではパス Gateway |
| `127.0.0.1:8081`（`gateway` の Caddy） | `SWING_GATEWAY_HOSTS` の `Host` だけ。それ以外は Caddy が 404 を返す |

- `SWING_GATEWAY_HOSTS` はカンマ区切りのホスト名（小文字英数字・`-`・`.`）。起動スクリプトが各ホストを `{"Paths": [], "UseSubdomains": false, "NoDNSLink": false}` で `PublicGateways` に入れる。これらのホストでは DNSLink（`_dnslink.<host>`）の内容だけを返し、`/ipfs/`・`/ipns/`・`/routing/v1` は 404。不正な名前があると `ipfs` は起動しない。
- Caddy は HTTP だけで待ち受け、TLS は前段（Cloudflare Tunnel など）に任せる。`entrypoint.sh` が `SWING_GATEWAY_HOSTS` を空白区切りにして `host` マッチャに渡す。空なら起動しない。
- Kubo は `Host` と `X-Forwarded-Host` をそのまま信じるので、Kubo の 8080 を外に出してはいけない。Caddy は受け取った `X-Forwarded-*` を捨てて付け直す。
