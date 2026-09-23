# 2026-09-23 ダッシュボード API を制御 API にし、CLI の一部をそのクライアントにする

[グレースフルな停止](2026-09-23-graceful-stop.md)で「`swing stop` をダッシュボード API の薄いクライアントにしたのは、他のサブコマンドも同じ方向に揃えるための布石」と書いた続き。今回は `status`・`mirror add`・`mirror remove` もダッシュボード API のクライアントにし、その前提として API 自体の寿命を `swing up` プロセス全体に広げた。architecture 側の結果は [`up.md`](../architecture/up.md)・[`agent.md`](../architecture/agent.md)・[`dashboard.md`](../architecture/dashboard.md)・[`dashboard/http-api.md`](../architecture/dashboard/http-api.md)・[`cli.md`](../architecture/cli.md)・[`service.md`](../architecture/service.md)・[`../architecture.md`](../architecture.md) に反映済み（このログには経緯だけを書く）。

## きっかけ

`swing stop` だけがダッシュボード API 経由になっていて、`status`・`mirror add`・`mirror remove` は agent とは独立に自分で relay に繋ぎ直し・`state.json` を読み直していた。動いている agent と同じ情報を二重に取りに行くだけでなく、`mirror add`/`remove` は relay への書き込み（Follow Set の再署名・publish）まで CLI プロセス自身が行っていたため、agent が同時に同じ Follow Set を操作する余地（レース）もあった。前回のログに「今の `sites`/`replicas`/`status`/`webring`/`mirror` は agent とは独立に relay に繋ぐ」と書いた課題そのものに手を付けた回。

## 決めたこと

- **`status`・`mirror add`・`mirror remove` を agent 必須にし、フォールバックを持たせない。** 以前の直接実行（relay に自分で繋ぐ）方式には戻さない。動いている `swing up` の API を叩くだけにすることで、二重の relay 接続と、CLI・agent が同時に Follow Set を書き換えるレースを両方無くせる。「agent が動いていないと使えない」という制約を明示的な仕様にし、[`cli.md`](../architecture/cli.md) の共通節にまとめて書いた。
- **`sites`・`replicas`・`webring`・`mirror list`・`publish` は API に寄せず、直叩きのまま残す。** 理由は 2 つに分かれる。`sites`/`replicas`/`webring`/`mirror list` は relay だけで完結する読み取りで、`swing up` が動いていなくても（agent を常駐させていない一時的な使い方でも）使えるべき機能だと判断した。`publish` はローカルディレクトリを直接 Kubo に流し込む処理で、ダッシュボードの `/api/publish/upload` はブラウザからの multipart アップロード用に作られており、ローカルパスを渡す経路が無い。API にローカルパスを渡す経路を新設するくらいなら、CLI から直接 Kubo を叩く今の形の方が単純と判断し、`docs/todo.md` に検討課題として残した。
- **`[dashboard].listen = "off"` を廃止し、設定エラーにする。** API を CLI の必須の入口にする以上、無効化できると `status`/`mirror add`/`mirror remove`/`stop` が原理的に使えなくなる構成を許してしまう。無効化したいのは大抵「ブラウザ向けの画面を配りたくない」であって「制御 API ごと止めたい」ではないと考え、両者を分離した新しい `[dashboard].ui`（既定 `true`）を設け、`listen` は常に有効なアドレスを要求するようにした（`config::parse_dashboard_listen`）。後方互換のフォールバック（`"off"` を読めたことにする、など）は入れていない。
- **ダッシュボード API の bind・serve を `agent::run_until` から `up::run` に移す。** 前回の実装では API は agent（`run_until`）の中で起動・終了しており、agent が落ちて再起動を待つ間は API ごと落ちていた。`status`/`mirror add`/`mirror remove`/`stop` を API 経由にするなら、agent の一時的な障害・再起動のたびに CLI まで使えなくなるのは望ましくない。API の寿命を `swing up` プロセスそのものに広げ、agent・Kubo が落ちて再起動している間も `/api/overview`・`/api/config`・`/api/shutdown`・`/api/restart` は応答し続けるようにした。relay・Kubo を使うエンドポイントだけ、agent が準備できていない間 503 `{"error": "agent is not ready"}` を返す（`AppState` の `relay`/`ipfs` を `RwLock<Option<...>>` にし、`set_ready`/`set_not_ready` で agent が出し入れする）。副作用として `shutdown::ExitRequest` のオーナーも `agent::run_until` から `up::run` 自身に移った（`agent::run_until` はもう `Exit` を返さず `Result<()>` だけを返す）。
- **`swing stop` のロックファイル試し取り（`instance_running` 相当のチェック）と unix 向け SIGTERM フォールバックを削除する。** API が agent の生死に関わらず `swing up` の寿命で必ず動くようになったので、「API に繋がらない」は「`swing up` が動いていない」と同じ意味になった。ロックファイルを試しに取って動作確認する経路や、繋がらないときの SIGTERM フォールバックは不要になり、`ApiClient::Unreachable` の一本にまとめられる。

## やったこと

引き継いだ未コミット差分（ダッシュボードの寿命移動・`AppState` の `RwLock` 化・`[dashboard].ui`・`api_client.rs`・`stop.rs`/`mirror.rs`/`health.rs` の API 化）に対して、今回追加で行ったこと:

- `src/dashboard/dto.rs`: `MirrorChangeDto` に `note: Option<String>` と `follow_set_found: bool` を追加し、`mirror_change_dto` で `mirror::MirrorChange` から詰めるようにした。`GarbageDto` に `list_failed_reason: Option<String>` を追加し、`status_dto` で `report.garbage.unlisted` のエラー文字列を詰めるようにした（DTO 化の際にどちらも黙って落ちていた）。
- `src/mirror.rs`: `print_mirror_change_dto` に `requires_follow_set: bool` を復元し、`change.follow_set_found`/`change.note` を見て、DTO 化前の `print_change_result` と同じ文言（`(no follow set found); no changes`、`(relays returned an older follow set; ...)` 等）を出すようにした。`add`/`remove` の呼び出し側もそれぞれ `false`/`true` を渡すよう修正。
- `src/health.rs`: `print_status_dto` が `GarbageDto::list_failed_reason` を使って `[list failed]: <理由>` を復元するよう修正（DTO 化後は `[list failed]` だけで理由が消えていた）。
- `web/`: `MirrorChangeDto`・`GarbageDto` を読む `sites.js`・`ui.js` はフィールド追加の影響を受けないことを確認した（使っているフィールドは変わっていない）。`settings.js` の設定表はサーバの `sections`/`items` をそのまま描画する汎用実装なので、`dto.rs` に既にあった `[dashboard].ui` の `ConfigItemDto` がそのまま表示に出ることを確認した。JS 自体の変更はしていない。
- docs: 上記「決めたこと」を反映して `architecture.md`・`architecture/cli.md`・`architecture/dashboard.md`・`architecture/dashboard/http-api.md`・`architecture/service.md`・`architecture/agent.md`・`architecture/up.md`・`README.md`・`.env.example` を更新した。`agent.md`/`up.md` は今回のタスクの指示には無かったが、ダッシュボードの bind・`ExitRequest` の所有元が `agent::run_until` から `up::run` に移ったことで内容が実装と食い違っていたため、AGENTS.md の「実装を変えたら同じ変更で必ず更新する」に従って合わせて直した。`docs/todo.md` から「CLI の読み取り系を API のクライアントにする」の行を消し、残った課題（`sites`/`replicas`/`webring`/`mirror list`/`publish` を API に寄せるかどうか）と、前回のログで見つかっていた「`agent::run_until` が relay 通知ストリーム終了で bail する経路では gateway のクリーンアップが漏れる」を 1 行ずつ足した。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -j 3 -- -D warnings` / `cargo test -j 3`（358 passed、15 ignored）を通した。
- `docker/demo/demo.sh up --seed` でデモ環境を再ビルド・起動（`.seeded` が既にあったため seed 自体はスキップ、既存のサンプルデータを再利用）。
  - `GET /api/overview`・`GET /api/config`（`dashboard.ui: true` を含む）・`GET /`（200）を確認した。
  - `docker/demo/demo.env` に `SWING_DASHBOARD_UI=false` を足してコンテナを再作成し、`/` が 404、`/api/overview` が 200 のままであることを確認した後、元に戻して 200 に戻ることを確認した。
  - コンテナ内で `swing status` を実行し、API 経由で版・実容量・garbage の一覧が出ることを確認した。
  - コンテナ内で使い捨ての鍵を作り、`swing mirror add <npub>` → `swing mirror remove <同じ npub>` を実行し、追加時に `Nostr` セクションと更新後のメンバー一覧が、削除時に `0 pubkey(s)` が正しく表示されることを確認した（API 経由での Follow Set 操作が動くことの確認。デモの Follow Set は下記の理由で元からこの 1 件しか無かったので、それ以上崩すものは無かった）。
  - `swing stop` を実行し `stopped` と表示されてプロセスが終了すること、compose の `restart: unless-stopped` により数秒後にコンテナが自動的に上がり直すことを確認した。
  - ホスト側で `SWING_DASHBOARD_LISTEN=127.0.0.1:1` を指定した `cargo run -- status`（コンテナを介さない代替）で `Error: swing up is not running (cannot connect to 127.0.0.1:1)` と非ゼロ終了を確認した。
  - 最後に `docker/demo/demo.sh down` でボリューム・ネットワークを削除した。

### 検証中に見つかったこと（今回のコード変更とは無関係）

`mirror add` の検証中、デモの `mirror` コンテナ起動直後のログに `no follow set found yet; will retry`（`swing::agent::follow`）と、それに続く 4 サイト分の `unfollowed`／`removed from MFS` が出ているのに気づいた。つまり自分（`my-garden`）の Follow Set（本来 alice/bob/carol を含むはず）が relay 上に既に存在せず、`remove_on_unfollow` の通常動作として全サイトが unfollow 済み・削除済みになっていた。この状態は私が何かコマンドを打つ前、コンテナ起動直後のログに既に出ていたので、今回のコード変更や検証操作が原因ではなく、デモ環境の relay データが（このセッション開始前の別の作業で）既に Follow Set を失っていたことによるもの。`mirror add`/`remove` の API 経由の動作自体は正しく機能していることを確認できたので、この状態のままデモ環境ごと `down` で破棄した。

## 見送ったこと

- `sites`/`replicas`/`webring`/`mirror list` を API のクライアントに寄せることと、`publish` にローカルパス入力の API を足すこと（`docs/todo.md` に残した）。
- ダッシュボードの認証トークン。API が常時有効になったことで重要度は上がったが、既定の bind 先（`127.0.0.1`）から出さない前提は変わらないため、今回も見送った（`docs/todo.md` に追記）。
- `agent::run_until` が relay 通知ストリーム終了で `Err` を返す経路で `shutdown_gateway`/`relay.client.shutdown()` を呼んでいない問題の修正。今回の作業範囲外と判断し、`docs/todo.md` に申し送りとして残した。

## 追記: gateway/relay の後始末漏れを直した

上で申し送った、relay 通知ストリーム終了（`Some(ClientNotification::Shutdown)` | `None`）の経路だけ `shutdown_gateway`/`relay.client.shutdown()` を呼ばずに抜けていた件を修正した。個別の分岐に同じ 2 行を重複させる代わりに、`'outer` ループの外に後始末を 1 か所へ集約し、`result` を確定させた後・`dashboard.set_not_ready()` の前に必ず `shutdown_gateway`→`relay.client.shutdown()` を呼ぶようにした（`src/agent/lifecycle.rs`）。全終了経路（正常な `shutdown.cancelled()`、poll 中の停止検知、relay 通知ストリーム終了）で挙動が揃う。

`docs/todo.md` に残していた本件の行は削除した。あわせて、読み取り系（`sites`/`replicas`/`webring`/`mirror list`）と `publish` を API に寄せるかどうかの検討行も、直叩きのままで確定したため削除した。
