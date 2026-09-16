# SWING -- Static-site Webring by IPFS and Nostr Generator / Nostr + IPFS 個人サイト相互ミラー MVP計画

## 概要

個人サイト運営者同士が、互いのサイトを自発的に保存・配送できる相互ミラーシステムを作る。

中央の保存サーバーや管理者に依存せず、

- 作者が「現在のサイト内容」を公開する
- 各参加者が「誰のサイトを保存するか」を選ぶ
- 選んだサイトを各自のIPFSノードへpinする

という構成にする。

Nostrは更新通知・参加者選択に使い、IPFSは実データの保存・配送に使う。

「分散」は、データが自動的に永久保存されることではなく、**複数の独立した主体が同じコンテンツを保持・配送しやすいこと**として扱う。

---

# 1. 基本アーキテクチャ

各参加者は以下を動かす。

```text
┌────────────────────────────┐
│ participant node           │
│                            │
│  mirror-agent              │
│      │                     │
│      ├── Nostr             │
│      │                     │
│      └── Kubo RPC          │
│             │              │
│            Kubo            │
│             │              │
└─────────────┼──────────────┘
              │
         Public IPFS
```

基本構成では、

- private swarmは作らない
- 独自IPFS Clusterも作らない
- HTTPS Gatewayも必須にしない

通常の公開IPFS Mainnetをそのまま利用する。

---

# 2. 役割分担

## Nostr

Nostrでは主に2種類の情報を扱う。

### サイト公開情報

作者が、

```text
このサイトの最新版は CID X
```

という署名付きイベントを公開する。

イベントには例えば以下を含める。

```text
site URL
root CID
公開日時
コンテンツサイズ
```

### ミラー対象リスト

各参加者が、

```text
私はAliceとBobとCarolのサイトを保存する
```

というリストをNostr上に持つ。

通常のフォローとは分け、NIP-51 Follow Setを利用する。

例えば、

```text
kind: 30000
d: site-mirror
```

というFollow Setを利用する。

---

## IPFS

サイトのHTML、CSS、画像など実際のファイルはIPFSで配布する。

作者が静的サイト全体をIPFSへ追加すると、

```text
bafy...
```

というroot CIDが得られる。

複数参加者が同じCIDをpinすることで、

```text
CID X

Alice   pin
Bob     pin
Carol   pin
```

という状態を作る。

作者のWebサーバーが消えても、他の参加者が保持していればIPFS経由で取得できる。

---

# 3. サイト公開フロー

作者は通常通り静的サイトを生成する。

```text
source
  ↓
static site generator
  ↓
./public
```

その後、

```bash
site-mirror publish ./public
```

を実行する。

内部では以下を行う。

```text
./public
   ↓
IPFSへadd
   ↓
root CID取得
   ↓
ローカルKuboでpin
   ↓
Nostrイベント生成
   ↓
作者の鍵で署名
   ↓
複数Relayへpublish
```

概念的には、

```bash
CID=$(ipfs add -Qr --cid-version=1 ./public)
```

でCIDを取得し、そのCIDをNostrへ公開する。

---

# 4. Nostrサイトイベント

サイトの「最新版」を表すイベントを一つ定義する。

addressable eventを使い、

```text
pubkey + kind + dタグ
```

でサイトを識別する。

例:

```json
{
  "kind": "<site event kind>",
  "tags": [
    ["d", "ama.ne.jp"],
    ["cid", "bafy..."],
    ["url", "https://ama.ne.jp/"],
    ["size", "12345678"]
  ],
  "content": ""
}
```

更新すると、

```text
v1 → CID A
v2 → CID B
v3 → CID C
```

となり、最新イベントが

```text
ama.ne.jp → CID C
```

を指す。

過去CIDを残すかどうかは各ミラー参加者の判断とする。

---

# 5. mirror-agent

MVPで主に自作する部分。

役割は以下。

```text
Nostr Relayへ接続
      ↓
自分のsite-mirror Follow Set取得
      ↓
対象pubkey一覧取得
      ↓
対象者のサイトイベントを購読
      ↓
署名検証
      ↓
CID取得
      ↓
保存ポリシー確認
      ↓
Kuboへpin要求
```

KuboとはRPC APIで通信する。

mirror-agentから見ると、

```text
POST /api/v0/pin/add?arg=<CID>
```

のような操作を行うだけでよい。

---

# 6. 保存ポリシー

Nostrは「誰を保存するか」だけを表す。

実際にどれだけ保存するかは各参加者のローカル設定とする。

例:

```toml
max_total_storage = "100GB"
max_per_site = "10GB"

keep_versions = 5
keep_days = 365
```

最低限、以下の制限を持つ。

```text
全体容量上限
サイト単位容量上限
1更新あたりサイズ上限
旧バージョン保持数
更新頻度制限
```

フォローした相手が無制限にディスクを消費できる設計にはしない。

---

# 7. 参加者管理

中央の参加者DBは作らない。

「参加者」という状態自体も必須にしない。

各ユーザーが単に、

```text
この人を保存する
```

というFollow Setを公開する。

つまり、

```text
Alice → Bob
Alice → Carol

Bob → Alice

Carol → Alice
Carol → Bob
```

という関係だけが存在する。

相互になっていれば、

```text
Alice ↔ Bob
```

としてWebring的に表示することもできる。

---

# 8. Docker Compose

参加障壁を下げるため、Docker Composeで配布する。

基本構成は2コンテナ。

```text
mirror-agent
     │
     │ RPC
     ↓
   Kubo
     │
Public IPFS
```

例:

```yaml
services:
  ipfs:
    image: ipfs/kubo
    restart: unless-stopped
    volumes:
      - ipfs-data:/data/ipfs

  mirror:
    image: example/site-mirror
    restart: unless-stopped
    environment:
      IPFS_API: http://ipfs:5001
      NOSTR_MIRROR_SET: site-mirror
      MAX_STORAGE_GB: 20

volumes:
  ipfs-data:
```

KuboのRPCポートはホストへ公開しない。

mirror-agentだけがDocker内部から、

```text
http://ipfs:5001
```

へアクセスする。

---

# 9. 参加方法

理想的には、

```bash
git clone https://example/site-mirror
cd site-mirror

cp .env.example .env

docker compose up -d
```

程度にする。

初期設定として必要なのは、

```text
Nostr identity
利用するRelay
最大保存容量
```

程度。

誰を保存するかは設定ファイルではなくNostr Follow Setから取得する。

---

# 10. publish機能

mirror-agentと同じプロジェクトに、

```bash
site-mirror publish
```

を持たせる。

例:

```bash
site-mirror publish \
  --site ama.ne.jp \
  --url https://ama.ne.jp/ \
  ./public
```

実行結果:

```text
Site: https://ama.ne.jp/

IPFS
  CID: bafy...
  ✓ added
  ✓ pinned

Nostr
  ✓ relay A
  ✓ relay B
  ✓ relay C

Published.
```

---

# 11. PoC段階のpublish

最初からpublish機能を実装する必要はない。

PoCでは、

```text
ipfs
+
nak
```

のシェルスクリプトでも十分。

つまり、

```text
publish.sh
   ↓
ipfs add
   ↓
CID取得
   ↓
nak event
   ↓
Nostrへpublish
```

でプロトコルを試す。

仕様が安定してから、

```bash
site-mirror publish
```

へ統合する。

---

# 12. Nostr鍵

MVPではサイト公開専用のNostr鍵を利用してもよい。

ただし将来的には、

```text
普段のNostr identity
      ↓
remote signer
      ↓
site-mirror
```

のようにNIP-46などのremote signer対応を検討する。

秘密鍵をサーバーやDocker Composeの`.env`へ直接置かなくても済む構成を目指す。

---

# 13. Gateway

MVPでは公開Gatewayを運営しない。

つまり参加者は、

```text
HTTP Server
HTTPS Certificate
Domain
```

を用意しなくていい。

IPFS peerとしてコンテンツを提供するだけでよい。

閲覧者は、

```text
作者の通常HTTPSサイト

自分のIPFSノード

第三者のpublic IPFS Gateway
```

などから取得できる。

Gateway運営は将来の任意オプションとする。

---

# 14. プライバシー

公開IPFS Mainnetを利用するため、匿名性は提供しない。

他のIPFS peerから、

```text
Peer ID
IPアドレス
提供しているCID
```

などの関連を観測される可能性がある。

このプロジェクトは、もともと公開Webサイトを保存することを前提とする。

一方、

```text
Kubo RPC
ローカルGateway
管理UI
```

などは外部へ公開しない。

---

# 15. MVPの実装範囲

最初に作るものは以下。

## 必須

```text
1. Docker Compose
2. Kuboコンテナ
3. mirror-agent
4. NIP-51 Follow Set取得
5. サイトイベント購読
6. CID取得
7. Kubo pin
8. 保存容量制限
9. publish PoC
```

## publish PoC

```text
ipfs add
+
nak event
```

で実装する。

---

# 16. MVPで作らないもの

以下は初期版では不要。

```text
中央管理サーバー
ユーザー登録
専用Web UI
IPFS Cluster
private swarm
独自DHT
public Gateway
レプリカ自動割当
高度なアクセス制御
決済
独自Nostr Relay
```

既存のNostrとIPFSに最大限乗る。

---

# 17. MVP後の拡張

## レプリカ数可視化

各参加者が、

```text
CID Xを現在pinしている
```

というイベントをNostrへ公開する。

集計して、

```text
ama.ne.jp
CID bafy...

Alice   ✓
Bob     ✓
Carol   ✓

3 replicas
```

と表示できる。

---

## Webring表示

Follow Setを集計して、

```text
Alice ↔ Bob
  ↓      ↑
Carol → Dave
```

のような保存関係グラフを生成する。

---

## Gateway

希望者だけ、

```text
Caddy
+
Kubo Gateway
```

を追加する。

---

## private mode

IPアドレス公開を避けたい参加者向けに、

```text
WireGuard
Tailscale
private IPFS network
```

などを使う別モードを検討する。

---

# 18. 最小の全体像

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

作者側:

```text
static site
    ↓
site-mirror publish
    ↓
┌─────────────┬─────────────┐
│             │             │
IPFS add    local pin     Nostr
│                           │
└──── CID ──────────────────┘
```

ミラー側:

```text
Nostr Follow Set
       ↓
対象者
       ↓
サイトイベント
       ↓
CID
       ↓
保存ポリシー
       ↓
ipfs pin
```

---

# 19. このプロジェクトの原則

このシステムでは、

> ネットワークに投稿したから永久保存される

ことは保証しない。

代わりに、

> 残したいと思う人が、自分の意思でコピーを持ち続けられる

ことを容易にする。

中央管理者が存在しなくても、

```text
誰を保存するか
どれだけ保存するか
いつまで保存するか
```

を各参加者自身が決められる。

リンクだけを交換する従来のWebringを、**保存と配送まで含む相互扶助ネットワークへ拡張する**ことが、このプロジェクトの中心的なアイデアである。
