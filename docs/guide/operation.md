# 保存量と運用の目安

ミラーする側がどれだけ保存するか、常時起動しない場合に何が起きるか、必要なスペックと通信量、設定の一覧をまとめます。

## どれくらい保存されるか

あなたのサイトを保存してくれる各参加者は、`[policy]`（`max_total_storage`・`max_per_site`・`max_per_account`・`max_sites_per_account`・`max_update_size`・`keep_versions`・`keep_days`・`min_update_interval`・`remove_on_unfollow`・`nip05`・`nip05_cache_ttl` など）に沿って保存量・保存期間を制限しています。値は参加者ごとのローカル設定です。キーごとの既定値と説明は [設定一覧](#設定一覧) を参照してください。

サイズはイベントの `size` タグではなく、実際に取得したデータ量で判定します。取得中に上限（`max_update_size`・`max_per_site`・`max_per_account` の最小値）を超えた時点で取得を打ち切ります。

打ち切った取得や削除した版のデータは、Kubo の GC が走るまでディスクに残ります。Kubo は `--enable-gc` で起動し、GC の基準になる `Datastore.StorageMax` を起動のたびに設定します。バイナリで `swing up` が管理する Kubo では `[kubo].storage_max`（`SWING_KUBO_STORAGE_MAX`。未設定なら `[policy].max_total_storage` と同じ値）を、Docker Compose の `ipfs` コンテナでは `.env` の `SWING_KUBO_STORAGE_MAX`（未設定なら `SWING_MAX_TOTAL_STORAGE`）を使います。GC はこの値の 90% を超えたときに走るので、少し余裕を足した値にしておくことをおすすめします。容量は `100GiB` のように `GiB` 系の単位で書いてください。swing は `GB` も `GiB` と同じ 1024 基数で読みますが、Docker Compose の Kubo は `GB` を 10 進（1GB = 10^9 バイト）で読むため、`GiB` 系で書いたときだけ両者が同じ値になります。

SWING は Kubo の pin を使わず、MFS の `/swing`（`SWING_MFS_ROOT` で変更可）の下だけを使います。手動で付けた pin や、MFS の他の場所に置いたものには触れません。一方で、`/swing/agent` の下は SWING が管理する場所なので、手で置いたものは消されます。

MFS に置いたサイトを他のノードから見つけてもらうには、Kubo の `Provide.Strategy` に `mfs` か `all` が含まれている必要があります。バイナリで `swing up` が管理する Kubo では `[kubo].provide_strategy`（`SWING_KUBO_PROVIDE_STRATEGY`、既定 `pinned+mfs`）を、Docker Compose の `ipfs` コンテナでは `.env` の `SWING_KUBO_PROVIDE_STRATEGY`（既定同じ）を、起動のたびに設定します。外部の Kubo を使う場合（`[kubo].managed = false`）は自分で設定してください。

判定の詳しい順序は [`docs/architecture/agent.md`](../architecture/agent.md) を参照してください。

## 動かし方の目安

### 想定している使い方

SWING は、常時起動のサーバでも、普段使いの PC でも動かせます。

- **常時起動のサーバ（自宅サーバ・VPS など）**: 自分のサイトもミラーしているサイトも、いつでも自分のノードから配れます。そのため、あなたがミラーしているサイトは、作者やほかのミラー参加者が全員止まっている時間帯でも読めます。
- **普段使いの PC**: 使っている間だけ起動すれば十分です。ずっと起動しておく必要はありません。ノート PC を閉じてスリープさせても、止めている間に来た更新は次に起動したときにまとめて取り込みます。`swing service install` でログイン時に起動するようにしておくと手間がかかりません。

どちらでも同じ設定ファイル・同じ手順で動き、途中で移ることもできます（Docker Compose からバイナリへの移り方は「[Docker Compose からバイナリの `swing up` に移る](docker.md#docker-compose-からバイナリの-swing-up-に移る)」）。

### 常時起動しない場合に何が起きるか

止めている間も壊れないもの:

- 公開したサイトイベントは relay に残ります。自分のノードが止まっていても「このサイトの最新版はこの CID」という情報は届き続け、ミラーしている参加者はそこから取得できます（relay がどれだけ保持するかは relay しだいです）。
- 止めている間に来たミラー対象の更新は、起動後の最初の確認でまとめて取り込みます。起動時には保存済みの版が揃っているかを確かめ、欠けていれば取り直します。
- 止めている間に別の端末から Follow Set（ミラー対象リスト）を変えていれば、起動後の確認で反映します。外した相手のサイトも、`remove_on_unfollow = true` なら消えます。

止めている間に起きること:

- **自分のノードからは配れません。** 自分のサイトは、ほかにミラーしている参加者が起動していれば、そこから読めます。ミラーしている人が 0 人のうちは、自分が止まると誰からも読めなくなります。始めたばかりのころがいちばん弱いので、まずは相互にミラーしてくれる相手を見つけるか、常時起動の参加者にミラーしてもらうのがおすすめです。
- **止めている期間が `report_ttl`（既定 `3d`）を超えると、レプリカ報告の期限が切れます。** 他の参加者の `swing replicas` やダッシュボードで、あなたがそのサイトの保存者として数えられなくなります。起動すれば次の確認で報告を出し直すので、数え直されます。報告は `report_ttl` の半分（既定 1.5 日）ごとに出し直すので、**1〜2 日に 1 回、しばらく起動する**くらいなら報告は切れません。受信側は 7 日より古い報告を数えないので、`report_ttl` を延ばせるのは `7d` までです。
- 署名アプリ（NIP-46）を使っている場合は、署名アプリが応答できないと起動していても報告を出し直せません（「[署名アプリ（NIP-46）で署名する](security.md#署名アプリnip-46で署名する)」）。

常時起動が必要なのは「自分」ではなく「相互ミラーの網のうちの誰か」です。

### 必要なスペック

- **OS**: Linux・macOS・Windows。Docker のイメージは `linux/amd64` と `linux/arm64` があります
- **メモリ**: `swing` 本体はデモ環境の待機中で約 10MB です。大半は Kubo が使い、公開 IPFS につないでいる間は接続している peer の数に応じて増えます
- **CPU**: 待機中はほとんど使いません。サイトの取得・publish・Kubo の GC のときに一時的に上がります
- **ディスク**: `[policy] max_total_storage`（既定 `100GiB`）に余裕を足した容量。これを「差し出してよい容量」に合わせて決めてください（「[どれくらい保存されるか](#どれくらい保存されるか)」）
- **ネットワーク**: `4001/tcp`・`4001/udp` を外から受けられると、他の参加者に配りやすくなります。受けられない環境でも、取得と保存はできます

### 起動中のリソースと通信量の見積もり

通信量は次の 4 つに分かれます。

| 通信 | 量の目安 | 何で決まるか |
|---|---|---|
| Nostr relay との通信 | 小さい。ミラー対象 3 人・4 サイトのデモ環境で 1 日約 2MB | `[agent] poll_interval`（既定 `5m`）ごとの確認とミラー対象の数 |
| ミラー対象のサイトの取得（下り） | 相手が更新したぶんだけ。手元に無いブロックだけを取りに行くので、差分の小さい更新なら小さく済みます。ミラー対象に加えた直後は最新版をまるごと取得します | 相手の更新頻度とサイズ。1 回の更新は `max_update_size`（既定 `2GiB`）などの上限まで。同じサイトの取り込みは `min_update_interval`（既定 `1h`）より頻繁にはしません |
| 他の参加者への配送（上り） | 自分が持っているサイトがどれだけ読まれるかしだい | SWING からは制限できません（Kubo が配っています） |
| IPFS ネットワークの維持（DHT など） | 何もしていなくても常に流れます | Kubo の設定（接続数・ルーティングの方式）と、保存しているブロックの数 |

実際の量は、Kubo の `ipfs stats bw` で確かめられます（バイナリなら `IPFS_PATH=<[kubo] repo のパス> ipfs --api-auth="bearer:$(jq -r .secret <state_dir>/kubo-api.json)" stats bw`（repo の既定は `<state_dir>/kubo`。`swing up` が管理する Kubo の RPC は起動のたびに作り直す秘密を要求し、`kubo-api.json` にはその回のポートと秘密が入っています）、Docker Compose なら `docker compose exec ipfs ipfs stats bw`）。

月あたりの通信量に上限がある回線（モバイル回線・テザリングなど）では、今のところ SWING 側で通信量の合計を抑える設定はありません。つながっている間は SWING ごと止めてください。バイナリの `swing up` なら `swing stop`（またはトレイの「Stop」）で Kubo も止まりますが、Docker Compose の構成では `mirror` を止めても `ipfs` コンテナは配り続けるので、`docker compose stop` で両方止めます。IPFS の維持の通信を減らしたい場合は、Kubo の設定の `Swarm.ConnMgr`（接続数）や `Routing.Type`（`autoclient` にすると、ほかの peer の DHT の問い合わせに答えなくなります）を直接変えてください。SWING はこれらの設定に触れないので、変えた値はそのまま残ります。

## 設定一覧

TOML の設定ファイル（`swing.toml`）を使う場合と、環境変数だけで動かす場合のどちらにも対応しています。優先順位は環境変数 > TOML > 既定値。キーごとの環境変数名・既定値・説明は [`swing.example.toml`](../../swing.example.toml) にすべて載っています（`swing config example` で生成、Docker Compose 用の `.env` は [`.env.example`](../../.env.example)、`swing config env-example` で生成）。設定ファイルの探し方は「[設定ファイルとデータの置き場所](install.md#設定ファイルとデータの置き場所)」のとおりです。探索順の詳細や、容量・時間の書式（`"100GiB"` や `"10m"` のような文字列）は [`docs/architecture/config.md`](../architecture/config.md) を参照してください。ダッシュボードから編集できるのはそのうちの一部（ホワイトリスト、[`docs/architecture/config.md`](../architecture/config.md#編集できるキー)）で、環境変数で設定した項目は編集できません。
