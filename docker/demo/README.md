# デモ環境

ダッシュボードなどの見た目・動作をローカルで確認するための、外部ネットワークに出ない compose 環境。本番用の `compose.yaml` に `docker/demo/compose.yaml` を重ねて使う。

## 使い方

```sh
docker/demo/demo.sh up          # ビルドして起動。初回だけ使い捨ての鍵を作る
docker/demo/demo.sh up --seed   # 同上。まだなら下記のサンプルデータも入れる
docker/demo/demo.sh seed        # 起動中の環境にサンプルデータを入れる
docker/demo/demo.sh down        # 停止してボリューム・鍵・サンプルの鍵を消す
docker/demo/demo.sh ps      # それ以外の引数はそのまま docker compose に渡す
docker/demo/demo.sh logs -f mirror
```

- ダッシュボード: <http://127.0.0.1:18082/>。ログインは `docker/demo/demo.sh exec mirror swing dashboard open --no-browser` で出た URL を開く（`docker/demo/compose.yaml` が `SWING_DASHBOARD_PUBLIC_URL=http://127.0.0.1:18082` を渡しているので、ホストのブラウザでそのまま開ける）
- Kubo Gateway: <http://localhost:18080/>（ダッシュボードの Gateway リンクもここを指す。`localhost` ではサイトごとに `<cid>.ipfs.localhost` へ移る）
- コードを変えたら `demo.sh up` をもう一度実行すれば mirror を作り直す。鍵とデータはそのまま残る。

NIP-05 検証を試したいときだけ、mirror を外部ネットワークにつなぐ:

```sh
SWING_DEMO_NIP05=1 docker/demo/demo.sh up   # mirror だけ外に出られる
docker/demo/demo.sh up                      # 付けずに実行し直せば隔離に戻る
```

## サンプルデータ（`seed.sh`）

架空の参加者 11 人（`alice`〜`mallory`）の鍵を作り、`swing publish` と `swing mirror add` をそれぞれの鍵で実行して、ローカル relay にサイトイベントと Follow Set を、Kubo にサイトの中身を入れる。最後に mirror を再起動して、agent にすぐ Follow Set を読ませる。

- 鍵は `docker/demo/personas.env`（git 管理外）に保存し、`seed` をやり直しても同じ参加者を使う。入れ直したいときは `down` から。
- 各コマンドは `SWING_STATE_DIR=/tmp` で実行するので、agent の `state.json` には触れない。
- `swing mirror add` は動いている `swing up` の API にしか話しかけないので、自分の分は `mirror` コンテナの中で実行し、他の参加者の分は `seed` コンテナの中で使い捨ての `swing up` を立ててから実行して `swing stop` で止める。使い捨ての `swing up` は `SWING_MFS_ROOT=/swing-seed`（mirror の MFS には触れない）・`SWING_MAX_UPDATE_SIZE=0`（何も保存しないのでレプリカ報告も出さない）で動かす。
- 自分（`self`、`my-garden`）から見た深さ（`swing webring --depth 5` の結果）:

| 深さ | 参加者（サイト） | Follow Set |
|---|---|---|
| 0 | self（`my-garden`） | alice, bob, carol |
| 1 | alice（`alice.example`、2 版）, bob（`bob-zine`）, carol（`carol.example`, `carol-photos`） | alice → self, dave／bob → carol, eve／carol → self, frank |
| 2 | dave（`dave-wiki`）, eve（`eve.example`）, frank（`frank-recipes`） | dave → grace／eve → bob, heidi／frank → ivan |
| 3 | grace（`grace.example`）, ivan（`ivan-lab`）, heidi（サイト無し） | grace → judy／ivan → frank／heidi → alice |
| 4 | judy（`judy.example`） | judy → grace, mallory |
| 5 | mallory（`mallory-archive`） | なし |

相互関係は self↔alice、self↔carol、bob↔eve、frank↔ivan、grace↔judy。自分がミラーするのは alice・bob・carol のサイト（4 件）で、agent が保存してレプリカ報告を出す。

- 各サイトの `created_at` は 3 時間前〜90 日前にばらしてある（`swing publish` は常に現在時刻で署名するので、`seed` サービスのイメージに入れた `faketime` で時計をずらして実行する）。時計は過去にずらすだけなので、publish の時計の確認（relay より進んでいないか）には当たらない。2 版ある `alice.example` は古い版から順に入れ、`seed` をやり直したときは古い版を入れ直さない（MFS にある新しい版より前の時刻で入れようとすると、未来の日付の版があるとして publish が止まるため）。Sites 画面の更新順は bob → carol（`carol.example` → `carol-photos`）→ alice になり、名前順・pubkey 順と見分けられる。
- `.example` のサイトには `url` を付けている（開いても何も無い）。NIP-05 は `demo.env` で `SWING_NIP05=off` にしてあり、NIP-05 モードでは `warn` になる。

## 構成

| サービス | ネットワーク | 内容 |
|---|---|---|
| `ipfs` | `isolated` | `--offline` で起動。4001・8080 はホストに公開しない |
| `relay` | `isolated` | `scsibug/nostr-rs-relay`。mirror の唯一の relay（`ws://relay:8080`）。DB は volume `relay-data` |
| `mirror` | `isolated`（NIP-05 モードでは `outside` も） | ホストにポートを公開しない |
| `seed` | `isolated` | `seed.sh` だけが使う（profile `seed`）。mirror のイメージに `faketime` を足した `swing-demo-seed`。ビルド時だけ apt でパッケージを取りに行く |
| `expose` | `isolated` + `outside` | `alpine/socat` で `127.0.0.1:18082`→`mirror:8082`、`127.0.0.1:18080`→`ipfs:8080` を中継するだけ |

- `isolated` は `internal: true` のネットワークで、ここにしか属さないコンテナは名前解決もインターネットへの接続もできない。コードに見落としがあっても外へは届かない。
- 設定は `docker/demo/demo.env`（git 管理外、`demo.sh up` が作る）だけから読む。`--env-file` で渡すので、リポジトリ直下の本物の `.env` は compose の変数展開にも mirror の環境変数にも使われない。
- 鍵はビルドしたイメージの `swing key generate` を `--network none` で実行して作る。

## 外に出る経路

mirror が外部と通信するのは次の 3 つだけ（`src/` と nostr-sdk 0.45 の既定動作を確認済み）。

| 経路 | 隔離時 | NIP-05 モード |
|---|---|---|
| Nostr relay（設定した relay だけに接続。nostr-sdk の gossip は既定で無効で、nprofile の relay ヒントなども辿らない） | ローカルの `relay` だけ | 同左（`SWING_NOSTR_RELAYS` を変えない限り外には出ない） |
| NIP-05（サイトイベントの `nip05` のドメインに `https://<domain>/.well-known/nostr.json` を取りに行く。私的・ループバックのアドレスには解決させない） | 遮断 | 外に出る。対象はローカル relay にあるイベントのドメインだけ＝自分でデモに入れたデータだけ。名前解決のために Docker ホストの DNS にもドメイン名が渡る |
| IPFS（Kubo の swarm） | `--offline` かつ隔離 | 同左（`ipfs` は NIP-05 モードでも隔離のまま） |

NIP-05 モードで気にすべきなのは、mirror がインターネットに出られる状態になること自体。上の 3 経路以外の通信はコード上に無いが、隔離時のように「ネットワークで保証する」のではなく「コードがそうなっている」ことに頼る形になる。
