# Docker

[`architecture.md`](../architecture.md) の一部。

## Dockerfile

- builder `rust:1.97-slim-trixie`、runtime `debian:trixie-slim`（glibc を揃えるため同じコードネーム）。
- runtime には `/usr/local/bin/swing` だけを置き、ユーザー `swing`（uid/gid 1000）で実行する。`/data` はそのユーザー所有の `VOLUME`。
- `ENTRYPOINT ["swing"]`、`CMD ["agent"]`。

## compose.yaml

| サービス | 内容 |
|---|---|
| `ipfs` | `ipfs/kubo:v0.43.1`（[Kubo のバージョン](kubo.md#kubo-のバージョン)）。イメージ既定の `command` に `--enable-gc` を足す。volume `ipfs-data:/data/ipfs` と `./docker/kubo-init.d:/container-init.d:ro`。公開ポートは `4001/tcp`・`4001/udp` のみ。healthcheck は `ipfs id` |
| `mirror` | `build: .`、`env_file: .env`。`SWING_IPFS_API=http://ipfs:5001`、`SWING_STATE_DIR=/data`、`RUST_LOG=info`。volume `swing-data:/data`。`ipfs` が healthy になるのを待つ |

両サービスとも `restart: unless-stopped`。`.env` は mirror の `env_file` と、compose の変数展開の両方に使われる。

## Kubo の設定

`ipfs` には次の環境変数を渡し、Kubo イメージが起動のたびに実行する `docker/kubo-init.d/001-swing-config.sh` で `ipfs config` に設定する。値が空ならコンテナは起動しない。

| 環境変数 | 設定先 | 既定 |
|---|---|---|
| `SWING_KUBO_STORAGE_MAX` | `Datastore.StorageMax`（GC の基準） | `SWING_MAX_TOTAL_STORAGE`、それも無ければ `100GB` |
| `SWING_KUBO_PROVIDE_STRATEGY` | `Provide.Strategy` | `pinned+mfs` |

`Provide.Strategy` に `mfs` か `all` を含めないと、MFS にしか無いサイトが DHT に告知されない。知らない値を与えると daemon が起動しない。
