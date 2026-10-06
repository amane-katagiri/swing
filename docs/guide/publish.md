# 自分のサイトを公開する

自分の静的サイトを SWING に乗せて公開するには、`swing publish` を使います。

IPFS で配りやすいサイトにするための注意（容量・外部リソースへの依存・相対パス・ビルドの再現性・更新の頻度など）は、チェックリストの形で [`docs/site-guide.md`](../site-guide.md) にまとめています。publish の前に一度目を通してください。

publish したサイトは、そのサイトをミラーする参加者の手元に保存され、IPFS やゲートウェイを通じてほかの人にも配られます。publish することで、SWING の参加者がそのサイトの各版を保存・複製し、IPFS やゲートウェイで配布・表示することを許したものとして扱います。自分が権利を持たないもの（他人の文章・画像・フォントなど）を含めるときは、こうした配布を許せるかを確かめてください。一度配られた版は取り消せません（[`docs/site-guide.md`](../site-guide.md#公開したものは取り消せない)）。

SWING の publish は、サイト識別子 `d` に自分のドメイン名を使い、そのドメインのルート（`https://<ドメイン>/`）を自分で管理していることを前提にしています。NIP-05 の検証はそのドメインの `/.well-known/nostr.json` を見に行くためです。サイトをサブパス以下で配信している場合や、共有ホスティングでドメインのルートを管理していない場合は、次のいずれかで対応してください。

1. `--site` にドメイン以外の識別子（例: `example-com-myname`）を指定する。この場合 NIP-05 は「対象外」となり、`warn` モードならそのまま publish できます
2. `--nip05 off` を指定して NIP-05 検証自体を行わない

`d` の値は、ミラーする側や今後の webring 一覧がそのサイトの名前として表示するものになるので、一度決めたら変えずに使い続けることをおすすめします。

Docker Compose で動かしている場合は、サイトのディレクトリをコンテナにマウントして実行します。

```bash
docker compose run --rm -v "$PWD/public:/site" mirror publish --site example.jp --url https://example.jp/ /site
```

バイナリで動かしている場合は、`swing up` を動かしたまま同じ設定ファイルで次のように実行します（`swing up` が管理する Kubo を自動で見つけます）。

```bash
swing publish --site example.jp --url https://example.jp/ ./public
```

`--site` はサイト識別子（`d` タグ）で必須です。`--url` はサイトを HTTP で配信している場合の URL で、省略できます。省略すると、IPFS だけで公開するサイトとして publish します（例: `swing publish --site my-notes ./public`）。`--title` でサイトの表示用タイトルを付けられます。作者の自己申告であり、受信側はこれを検証や保存判断には使いません。`-m`（`--message`）で「ブログに記事を追加」のような更新メモを付けられます。メモはサイトイベントの本文になり、ミラーする側の `swing sites` や、SWING に対応していない Nostr クライアントにも表示されます。メモは 4096 バイトまでで、超えると publish を始める前にエラーになります。実行すると、次のような出力になります。

```text
Site: example.jp
URL: https://example.jp/

NIP-05
  ✓ verified

Checks
  ✓ dotfiles: none
  ✓ size: 12.1 KiB (guideline 512 MiB)

New files
  compared with your latest version on the relays (bafy...)
  ! 1 new file
      posts/
        hello.html
  Publish with 1 new file? [y/N] y

IPFS
  CID: bafy...
  ✓ added to /swing/publish/<pubkey>/example.jp/1700000000
  Size: 12345 bytes

Previous version
  ✓ changed from the latest version on the relays (bafy...)

Nostr
  ✓ wss://relay.damus.io
  ✓ wss://nos.lol

Old versions (keeping 5)
  ✓ removed /swing/publish/<pubkey>/example.jp/1690000000

Published.
```

処理内容は、ディレクトリを Kubo に追加して MFS の `/swing/publish/` の下に置き、その root CID を含むサイトイベント（`kind 35980`）に自分の鍵で署名し、設定した全 relay に publish する、というものです。どれかの relay に受理されたら、今回の版を必ず残し、同じサイトの版を今回の版を含めて `keep_versions`（既定 5）個になるよう新しい順に残して、ほかを MFS から消します。

NIP-05 は、`d` タグがドメイン名の形をしている場合に、そのドメインの所有者が自分の pubkey を掲載しているかどうかを確認する任意の検証です。確認するには、公開するドメインの `https://{ドメイン}/.well-known/nostr.json` に `{"names": {"_": "<自分の pubkey の hex>"}}` を置きます。検証モードは `--nip05 off|warn|require`（省略時は `.env` の `SWING_PUBLISH_NIP05`、既定 `warn`）で切り替えられ、`warn` は結果を表示するだけで publish を続行し、`require` は検証に成功しない限り publish を中止します。

publish はあわせて、[チェックリスト](../site-guide.md)のうち機械的に確かめられる 3 つを確かめます。どれも NIP-05 と同じ `off`（確かめない）・`warn`（表示して続ける）・`require`（引っかかったら止める）で切り替えられ、省略時は `[publish]` の設定（環境変数は `SWING_PUBLISH_` で始まる名前）に従います。

| 確かめること | フラグ | 設定 | 既定 |
|---|---|---|---|
| 名前が `.` で始まるファイル・ディレクトリ（`.git`・`.env` など）が入っていないか。`dotfiles_allow`（既定 `.well-known`・`.nojekyll`・`.gitkeep`・`.keep`・`.domains`）に載っている名前そのものは見逃す。ただし、そのディレクトリの中にある `.` で始まるもの（`.well-known/.env` など）は見逃さない | `--check-dotfiles` | `check_dotfiles`・`dotfiles_allow` | `require` |
| ファイルの合計が 512 MiB を超えていないか（目安。保存するかどうかはミラーする側の設定で決まる） | `--check-size` | `check_size` | `warn` |
| 追加した CID が relay 上の自分の最新版と同じではないか。`require` なら追加した版を消して、署名も送信もせずに `Unchanged; not published.` で正常終了する（終了コード 0） | `--check-unchanged` | `check_unchanged` | `require` |

IPFS に追加する前に、relay 上の自分の最新版に無かったファイル（増えたファイル）の一覧を出し、1 件でもあれば公開してよいか `y/N` で聞きます。`--yes`（`-y`）を付けると聞かずに続けます。端末から実行していない（CI など）ときは、増えたファイルがあれば `--yes` が無い限り止まります。ダッシュボードの公開画面でも、アップロードする前に同じ一覧を出して確かめます。何を確かめるか・何を見逃すかは、チェックリストの「[増えたファイルを確かめる](../site-guide.md#増えたファイルを確かめる)」を参照してください。

ドットファイルとサイズは IPFS に追加する前、同じ内容かどうかは追加した後に確かめます。これとは別に、リンク先がディレクトリの外にあるシンボリックリンクがあるときと、ディレクトリの中に SWING の設定ファイル・状態ディレクトリ（`data/`）・Kubo のリポジトリがあるときは、設定にかかわらず何も追加せずに止まります。relay から前の版を取れなかったときは、`require` でも止めずに publish します。ダッシュボードの公開画面でも同じものを確かめて結果を出します。

## ほかの人にミラーしてもらう

サイトをミラーしてもらうには、自分の公開鍵（npub）を相手に伝えます。[SWING Connect](https://swing-connect.pages.dev/) を使うと、公開鍵とミラーの手順をまとめたページへのリンクを作れるので、自分のサイトに貼っておけます。

- `https://swing-connect.pages.dev/?key=npub1...` を開くと、その公開鍵と、コマンド・ダッシュボードそれぞれでのミラーの手順が表示されます。
- `nip05=example.jp` を付けると、`https://example.jp/.well-known/nostr.json` の `_` で公開鍵を確かめた結果を添えます（上の NIP-05 と同じファイルです）。ブラウザから確かめるので、そのファイルを CORS（`Access-Control-Allow-Origin`）付きで配信している必要があります。
- `light`・`dark` に色（`rrggbb`）を渡すと、ライトモード・ダークモードのページの色を自分のサイトに合わせられます。
- パラメータを付けずに開くと、これらを入力してリンクを作る画面になります。

見出しや説明文・見た目を変えた自分用のページを置きたいときは、[swing-connect](https://github.com/amane-katagiri/swing-connect) をフォークして設定を変えてください。

## 何人が保存しているかを見る

mirror-agent は、保存している版の CID を「レプリカ報告」（`kind 35981`）として Nostr に出し続けます。自分で publish したサイトも、同じ Kubo（同じ `SWING_MFS_ROOT`）で mirror-agent を動かしていれば、作者本人の分として報告されます。`swing replicas` で、自分のサイトを誰が保存しているかを確認できます。

```bash
docker compose exec mirror swing replicas
```

```text
npub1me... (<pubkey>)
  d=example.jp cid=bafy... replicas=2 (reports=3)
    npub1alice...  [latest]  [chosen]
    npub1me...     [latest]  [author]
    npub1bob...    [older version]  [unverified]
```

`replicas` は、最新版を持っていると報告した参加者のうち、作者本人（`[author]`）か、作者またはあなたのミラー対象リストに載っている人（`[chosen]`）の数です。それ以外の報告者（`[unverified]`。誰でも自称できるため信頼度が低い扱い）が最新版を持っていれば、`replicas=2 (+3 unverified)` のように別枠で添えます（0 件なら省略）。`[older version]` は古い版だけを持っている参加者です。報告は自己申告なので、実際に配送できるかまでは保証しません。npub などを渡すと、他の人のサイトについても表示します。

## 相互ミラーの関係（Webring）を見る

`swing webring` は、自分を起点にミラー対象リストをたどり、誰が誰を保存しているかをグラフとして表示します。たどるのは自分が実際に保存対象へ入れている相手（`p` タグ）だけで、自分をミラー対象に入れているだけの相手（`#p` で見つかる、フォローし返されていない相手）はグラフには加えず、「Referencing the root」に自称にすぎない一覧として別枠で出します。

```bash
docker compose exec mirror swing webring
```

```text
Webring of mirror set "swing" (depth 2): 4 accounts, 1 mutual, 2 one-way

Accounts
  example.jp             npub1me...     depth=0  [root]
  alice.example          npub1alice...  depth=1
  npub1bob12…xyz789      npub1bob...    depth=1
  carol.example          npub1carol...  depth=2

Mutual
  example.jp ↔ alice.example

One-way (A → B: A mirrors B)
  alice.example → carol.example
  npub1bob12…xyz789 → example.jp

Referencing the root (unverified)
  npub1dave...
```

各アカウントは公開しているサイトの `d` で表示し、サイトが無ければ npub を縮めて表示します。`--depth <N>`（既定 2）でたどる距離を、npub などを渡すと起点を変えられます。`--format dot` で Graphviz、`--format mermaid` で Mermaid の図として出力します。

```bash
docker compose exec mirror swing webring --format dot | dot -Tsvg > webring.svg
```

## 自分のサイトをゲートウェイで配信する

決めたホスト名だけを DNSLink で配信する HTTP サーバーは、SWING 自身に内蔵しています。TLS は扱わないので、Cloudflare Tunnel などを前段に置いて、そこから転送してください。

バイナリで動かす場合は `swing.toml` に次を書いて `swing up`（サービス登録している場合は再起動）します。

```toml
[gateway]
listen = "127.0.0.1:8081"
hosts = ["example.com", "blog.example.net"]
```

Docker Compose で動かす場合は `.env` に次を書いて `docker compose up -d` します。

```bash
SWING_GATEWAY_LISTEN=0.0.0.0:8081
SWING_GATEWAY_HOSTS=example.com,blog.example.net
```

コンテナ内では `0.0.0.0:8081` で listen させ、ホストへの公開先は別途 `SWING_GATEWAY_BIND`（既定 `127.0.0.1:8081`）で決めます。

`hosts` にはダッシュボードを開くホスト名（`localhost`・`127.0.0.1` と `SWING_DASHBOARD_ALLOWED_HOSTS`）を入れられません。同じ名前にすると、配信するサイトとダッシュボードの cookie が混ざるためで、設定の読み込みがエラーになります。

各ホストの DNS に `_dnslink.<ホスト名>` の TXT レコード（`dnslink=/ipfs/<cid>`）を置きます。ゲートウェイは Kubo のゲートウェイにそのまま中継するだけで、ローカルにあるデータしか返しません。CID は `swing publish` でこのノードに置いたものにしてください。publish のたびに TXT レコードも更新します。

設定したホスト名以外、および `/ipfs/<cid>` のようなパスでのアクセスには 404 を返します。詳しくは [`docs/architecture/gateway.md`](../architecture/gateway.md) を参照してください。
