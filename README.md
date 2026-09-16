# SWING

SWING (Static-site Webring by IPFS and Nostr Generator) は、個人サイトの運営者同士が、互いのサイトを自発的に保存・配送し合うための相互ミラーツールです。

「誰のサイトを保存するか」を Nostr で表明し、実際のデータの保存と配送は IPFS で行います。中央の保存サーバーや管理者は存在しません。あなたのサイトを保存するかどうかは、それぞれの参加者が自分の意思で決めます。逆に言うと、SWING はデータの永久保存を保証するものではありません。保存してくれる参加者がいる間だけ、あなたのサイトのコピーが IPFS 上に残ります。

SWING が提供するのは、この「誰を保存するか」の表明と、実際に保存・配送するための最小限の仕組みだけです。今のところ、Web UI やアカウント登録、決済、専用の Relay や IPFS ネットワークは提供していません。

詳しい設計思想を知りたい方は [`docs/plan.md`](docs/plan.md) を、現在の実装の詳細な仕様を知りたい方は [`docs/architecture.md`](docs/architecture.md) をご覧ください。

## しくみ

```text
                 Nostr
        ┌─────────┼─────────┐
        │         │         │
      Alice      Bob      Carol
        │         │         │
   mirror-agent mirror-agent mirror-agent
        │         │         │
      Kubo      Kubo      Kubo
        │         │         │
        └─────────┼─────────┘
             Public IPFS
```

Nostr は「更新の通知」と「誰のサイトを保存するか」を伝えるために使います。IPFS はサイトの実データを保存・配送するために使います。

各参加者は `mirror-agent`（このツールの `swing agent`）と、IPFS ノードである Kubo を動かします。mirror-agent は自分が保存すると決めた相手のサイト更新を Nostr 経由で受け取り、ポリシーに沿って CID を Kubo に pin します。

## 必要なもの

- Docker と Docker Compose（`docker compose` コマンドが使えること）
- Nostr の秘密鍵（nsec または hex 形式）。サイト保存・ミラー参加専用の鍵を新しく作ることをおすすめします。作り方はいくつかあります。
  - `docker compose run --rm mirror key generate`（このツールだけで完結。`.env` が無くても動きます）
  - `nak key generate`（[nak](https://github.com/fiatjaf/nak) の CLI。出力は hex の秘密鍵なので、そのまま `SWING_NOSTR_SECRET_KEY` に入れられます。`nak key public <hex>` で公開鍵を得られます）
  - Nostr クライアントで新規アカウントを作り、設定画面から nsec を書き出す（例: Damus、Amethyst、noStrudel、Nostur）
- 自分のサイトも公開したい場合は、そのビルド済み静的サイトのディレクトリ（例: `./public`）。あわせて、ルートを自分で管理しているドメインがあると NIP-05 で本人確認ができます（必須ではありません）

## はじめかた（ミラー参加者として）

まずリポジトリを取得します。

```bash
git clone <このリポジトリ>
cd swing
```

`.env` ファイルを作り、必要な項目を設定します。

```bash
cp .env.example .env
```

`SWING_NOSTR_SECRET_KEY` を埋めるには、次のように鍵を生成するのが手軽です。

```bash
docker compose run --rm mirror key generate
```

`.env.example` に含まれる項目は次のとおりです。

| 変数 | 意味 |
| --- | --- |
| `SWING_NOSTR_SECRET_KEY` | 署名用の秘密鍵（nsec または hex）。空のままでは起動できません |
| `SWING_NOSTR_RELAYS` | 接続する Nostr Relay（カンマ区切り）。実際に自分が使っている relay に置き換えることをおすすめします |
| `SWING_MIRROR_SET` | ミラー対象リスト（Follow Set）の `d` タグ。通常は既定値 `swing` のままで構いません |
| `SWING_MAX_TOTAL_STORAGE` | 保存する全サイト合計の容量上限（例: `20GB`） |

最低限、`SWING_NOSTR_SECRET_KEY` は必ず自分の値に書き換えてください。他の環境変数の全体像は「設定一覧」を参照してください。

`.env` を用意できたら、コンテナを起動します。

```bash
docker compose up -d
```

`ipfs`（Kubo）と `mirror`（このツール本体、既定で `swing agent` を実行）の 2 つのコンテナが立ち上がります。Kubo の RPC（5001）やゲートウェイ（8080）はホストに公開されず、公開されるのは IPFS の swarm 用ポート（`4001/tcp`・`4001/udp`）だけです。

保存したい相手を追加するには、`mirror` コンテナの中で `swing mirror add` を実行します。相手の npub（または hex、nprofile）を指定してください。

```bash
docker compose exec mirror swing mirror add npub1alice... npub1bob...
```

これは Nostr 上の NIP-51 Follow Set（`kind 30000`, `d = swing`）を更新し、指定した相手を保存対象として宣言します。設定を確認するには次のようにします。

```bash
docker compose exec mirror swing mirror list
```

保存対象のサイトの状態を見るには `swing sites` を使います。

```bash
docker compose exec mirror swing sites
```

各サイトについて `d`（サイト識別子）、`cid`、`url`、`size`、`created_at`、NIP-05 の検証結果、pin 状況が 1 行ずつ表示されます。

コンテナのログで動作状況を確認することもできます。

```bash
docker compose logs -f mirror
```

## 自分のサイトを公開する

自分の静的サイトを SWING に乗せて公開するには、`swing publish` を使います。

SWING の publish は、サイト識別子 `d` に自分のドメイン名を使い、そのドメインのルート（`https://<ドメイン>/`）を自分で管理していることを前提にしています。NIP-05 の検証はそのドメインの `/.well-known/nostr.json` を見に行くためです。サイトをサブパス以下で配信している場合や、共有ホスティングでドメインのルートを管理していない場合は、次のいずれかで対応してください。

1. `--site` にドメイン以外の識別子（例: `example-com-myname`）を指定する。この場合 NIP-05 は「対象外」となり、`warn` モードならそのまま publish できます
2. `--nip05 off` を指定して NIP-05 検証自体を行わない

`d` の値は、ミラーする側や今後の webring 一覧がそのサイトの名前として表示するものになるので、一度決めたら変えずに使い続けることをおすすめします。

Docker Compose で動かしている場合は、サイトのディレクトリをコンテナにマウントして実行します。

```bash
docker compose run --rm -v "$PWD/public:/site" mirror publish --url https://example.jp/ /site
```

`cargo build --release` でビルドした `swing` バイナリをホストで直接使う場合は、Kubo の RPC に届く設定（`SWING_IPFS_API`）を用意した上で次のように実行します。

```bash
swing publish --url https://example.jp/ ./public
```

`--site` を省略すると、`--url` のホスト名（この例では `example.jp`）がサイト識別子（`d` タグ）になります。実行すると、次のような出力になります。

```text
Site: https://example.jp/

NIP-05
  ✓ verified

IPFS
  CID: bafy...
  ✓ added
  ✓ pinned

Nostr
  ✓ wss://relay.damus.io
  ✓ wss://nos.lol

Published.
```

処理内容は、ディレクトリを Kubo に追加して pin し、その root CID を含むサイトイベント（`kind 35980`）に自分の鍵で署名し、設定した全 relay に publish する、というものです。

NIP-05 は、`d` タグがドメイン名の形をしている場合に、そのドメインの所有者が自分の pubkey を掲載しているかどうかを確認する任意の検証です。確認するには、公開するドメインの `https://{ドメイン}/.well-known/nostr.json` に `{"names": {"_": "<自分の pubkey の hex>"}}` を置きます。検証モードは `--nip05 off|warn|require`（省略時は `.env` の `SWING_PUBLISH_NIP05`、既定 `warn`）で切り替えられ、`warn` は結果を表示するだけで publish を続行し、`require` は検証に成功しない限り publish を中止します。

## どれくらい保存されるか

あなたのサイトを保存してくれる各参加者は、以下のようなポリシーに沿って保存量・保存期間を制限しています。値は参加者ごとのローカル設定であり、SWING の既定値は次のとおりです。

| キー | 既定値 | 意味 |
| --- | --- | --- |
| `max_total_storage` | `100GB` | 保存する全サイト合計の容量上限 |
| `max_per_site` | `10GB` | 1 サイトあたりの容量上限。超えた分は古い版から削除される |
| `max_update_size` | `2GB` | 1 回の更新（1 バージョン）あたりのサイズ上限。超えると保存されない |
| `keep_versions` | `5` | サイトごとに保持する旧バージョンの数。超えた分は古い順に削除される |
| `keep_days` | `365` | バージョンを保持する日数。最新版を除き、これより古い版は削除される |
| `min_update_interval` | `10m` | 同じサイトの更新を受け付ける最短間隔。これより短い間隔で来た更新は保存されない |
| `unpin_on_unfollow` | `true` | 相手をミラー対象から外したときに、自動でそのサイトの保存をやめるかどうか |
| `nip05` | `warn` | 保存前に行う NIP-05 検証のモード（`off` / `warn` / `require`） |

判定の詳しい順序は [`docs/architecture.md`](docs/architecture.md) を参照してください。

## 設定一覧

TOML の設定ファイル（`swing.toml`）を使う場合と、環境変数だけで動かす場合のどちらにも対応しています。環境変数は常に TOML の値を上書きします。設定ファイルの探索順や、容量・時間の書式（`"100GB"` や `"10m"` のような文字列）は [`docs/architecture.md`](docs/architecture.md) を参照してください。設定例は [`swing.example.toml`](swing.example.toml) にあります。

| 環境変数 | 対応する設定 | 説明 |
| --- | --- | --- |
| `SWING_CONFIG` | (CLI `--config`) | 設定ファイルのパス。省略時は `./swing.toml` |
| `SWING_NOSTR_SECRET_KEY` | `nostr.secret_key` | 署名用秘密鍵（nsec または hex） |
| `SWING_NOSTR_RELAYS` | `nostr.relays` | 接続する relay（カンマ区切り） |
| `SWING_MIRROR_SET` | `nostr.mirror_set` | Follow Set の `d` タグ（既定 `swing`） |
| `SWING_SITE_EVENT_KIND` | `nostr.site_event_kind` | サイトイベントの kind（既定 `35980`） |
| `SWING_IPFS_API` | `ipfs.api` | Kubo RPC のエンドポイント |
| `SWING_MAX_TOTAL_STORAGE` | `policy.max_total_storage` | 全体容量上限 |
| `SWING_MAX_PER_SITE` | `policy.max_per_site` | サイト単位の容量上限 |
| `SWING_MAX_UPDATE_SIZE` | `policy.max_update_size` | 1 更新あたりのサイズ上限 |
| `SWING_KEEP_VERSIONS` | `policy.keep_versions` | 保持する旧バージョン数 |
| `SWING_KEEP_DAYS` | `policy.keep_days` | バージョン保持日数 |
| `SWING_MIN_UPDATE_INTERVAL` | `policy.min_update_interval` | 更新受理の最短間隔 |
| `SWING_UNPIN_ON_UNFOLLOW` | `policy.unpin_on_unfollow` | unfollow 時に自動 unpin するか |
| `SWING_NIP05` | `policy.nip05` | mirror-agent の NIP-05 検証モード（既定 `warn`） |
| `SWING_STATE_DIR` | `agent.state_dir` | 状態ファイルを置くディレクトリ |
| `SWING_POLL_INTERVAL` | `agent.poll_interval` | Follow Set の再取得間隔 |
| `SWING_PIN_TIMEOUT` | (Kubo RPC の pin タイムアウト) | `pin/add` のタイムアウト（既定 1 時間） |
| `SWING_PUBLISH_NIP05` | `publish.nip05` | `swing publish` の NIP-05 検証モード（既定 `warn`。CLI の `--nip05` が優先） |

## プライバシーと注意点

SWING は公開の IPFS Mainnet をそのまま使うため、匿名性は提供しません。他の IPFS peer から、あなたの Peer ID・IP アドレス・提供している CID などの関連を観測される可能性があります。もともと公開 Web サイトを保存することが前提のツールなので、この点は許容した上でご利用ください。

一方で、Kubo の RPC やローカルのゲートウェイ、管理 UI は外部に公開しません。Docker Compose の構成では、公開されるのは IPFS swarm 用のポート（`4001`）だけです。

Nostr の秘密鍵は `.env` に平文で保存されます。サイト公開・ミラー参加専用の鍵を新しく作り、他の用途の鍵とは分けて扱うことをおすすめします。`.env` を Git にコミットしないよう注意してください。

## 含まれていないもの・今後の予定

以下は現時点では扱いません。既存の Nostr と IPFS にできるだけそのまま乗ることを優先しています。

- 中央管理サーバー / ユーザー登録
- 専用の Web UI
- IPFS Cluster / private swarm / 独自 DHT
- 公開 Gateway
- レプリカの自動割当
- 高度なアクセス制御
- 決済
- 独自の Nostr Relay

今後の拡張として、レプリカ数の可視化、Follow Set を集計した Webring 表示、任意の Gateway、private mode（IP アドレスを隠したい参加者向けの別モード）などを検討しています。NIP-46 remote signer への対応も予定にあります。残タスクの一覧は [`docs/todo.md`](docs/todo.md)、新しい kind や `d` タグの命名規約は [`docs/extensions.md`](docs/extensions.md) を参照してください。

## ドキュメント

- [`docs/plan.md`](docs/plan.md): 初期実装計画。設計の背景や原則を説明しています
- [`docs/protocol.md`](docs/protocol.md): 実装非依存のプロトコル定義。他のクライアントやエージェントを実装する方向けです
- [`docs/architecture.md`](docs/architecture.md): 現在の実装の詳細なリファレンス
- [`docs/extensions.md`](docs/extensions.md): 新しい kind や `d` タグを追加する際の命名規約と予約表
- [`docs/todo.md`](docs/todo.md): 残タスクの一覧
- [`docs/log/`](docs/log/): 実装ログ（何を決め、何を作り、何を検証したかの記録）
- [`docs/examples/publish.sh`](docs/examples/publish.sh): `ipfs` CLI と `nak` だけでプロトコルを再現する参考実装（サポート対象外）

## ライセンス

[MIT License](LICENSE)
