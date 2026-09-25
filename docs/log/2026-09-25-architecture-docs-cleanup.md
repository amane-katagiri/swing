# architecture ドキュメントの整理

`docs/architecture.md` と `docs/architecture/` が増築を重ねて肥大化し、同じ内容が複数ファイルに重複したり、実装の現状とは別に変更の経緯が書き込まれたりしていた。今回、正本を 1 箇所に決め直し、大きすぎるファイルを分割し、経緯は log に寄せて architecture 側は結果だけを書く状態に整理した。

## 決めたこと

### 正本の割り当て

トピックごとに正本を 1 ファイルに決め、他のファイルからはそこへリンクするだけにした。

| トピック | 正本 |
|---|---|
| セットアップモードの起動挙動 | `up.md` |
| セットアップ中の API 503・409 | `dashboard/http-api.md`「共通」 |
| プロセス内再起動・シグナル・起動順 | `up.md` |
| API 経由の CLI 一覧・`swing stop` | `cli.md` |
| `swing service` の実体 | `service.md` |
| 設定ファイルの探索順 | `architecture.md` |
| 設定の書き込み手順と権限・編集可能キー | `dashboard.md` |
| 未来ずれの許容・取得と表示の上限・レプリカ報告の信頼度 | `nostr.md` |
| Kubo プロセスの管理・適用する設定・`NoFetch` | `kubo.md` |
| NIP-05 検証と SSRF 対策 | `nip05.md` |
| ビルドとリリース | `release.md` |

### 分割

- `nostr.md` を新設し、`architecture.md` にあった Nostr イベントの検証・未来ずれの許容・取得と表示の上限・レプリカ報告の信頼度をまとめて移した。
- `release.md` を新設し、`architecture.md` にあったビルドとリリースの節を移した。
- `dashboard/desktop.md` を新設し、`dashboard/web.md` が抱えていた Desktop 画面（壁紙・アイコン・コントロール パネルなど）の節を移した。
- `up.md` にあった `kubo.rs` 関連の記述（バイナリの検出、動的な API ポートなど）は `kubo.md` に移した。`up.md` は supervisor のループとシグナル・終了要求だけを扱う。
- `service.md` にあった `swing stop` の実装詳細は `cli.md#stop` に一本化した（`service.md` は OS ごとのサービス登録の実体だけを扱う）。

### 経緯・理由の扱い

「なぜそうしたか」は architecture には残さず、該当する log（無ければ本エントリ）に寄せた。architecture 側は現状の結果だけを記述する。

## 直した誤り

現状の実装と食い違っていた記述を、この整理と合わせて修正した。

- `PUT /api/config` が書き込むファイルの権限は常に `0600`（既存ファイルの権限を引き継ぐという誤記があった）。
- Windows の Job Object を使った子プロセス管理は実機で確認済み（未検証という記述が残っていた）。
- gateway の起動主体は `agent::run_until` で、ダッシュボードの起動主体は `up::run`（逆の記述があった）。
- `[gateway].listen` の型は `Listen`、`[dashboard].listen` の型は `SocketAddr`（両者を同じ型として書いていた）。
- 静的ファイルの一覧表から `pairing.js` が漏れていた。個別列挙をやめ、漏れが起きない書き方に変えた。
- レプリカ報告の `cid` も `canonical_cid` を通す（素の `cid` をそのまま比較しているような記述があった）。
- セットアップモードの条件は `[nostr].secret_key` と `<state_dir>/remote-signer.json` の両方が無いとき（どちらか一方の記述になっていた）。
- `swing stop --restart` は（プロセスが終了するのではなく）`instance` の値が変わるのを待って完了とする。
- ダッシュボードの `web` 側: `data-status` に `na` という値は無い、`desk-icon-shortcut` はコントロール パネルにも付く、`data-view` には `setup`・`login` もある（画面の状態一覧から漏れていた）。

## architecture から削った理由のうち他の log に無かったもの

architecture 側から経緯の記述を削るにあたって、既存の log に書かれていなかった理由をここに残す。

1. NIP-46 ペアリング用の `nostrconnect://` URI に `metadata={"name":"SWING"}` を付けているのは、`name` を読まない古い署名アプリ（rust-nostr のパーサを含む）でもアプリ名を表示させるため。
2. `kubo.pid` の読み込みに互換読み込みは入れていない。壊れている、または形式が違うファイルは無条件に「検証不能」として扱う。
3. 孤児 Kubo の本人確認が一致しないときはバックオフして黙って再試行することはしない。repo lock を握ったままの孤児がいる状態で新しい Kubo を起動しても意味が無いため。
4. `swing mirror remove` は Follow Set が無いとエラーになるが、`add` には同じ制限を付けていない。何も無い状態からの新規作成を許すため。
5. `[dashboard].listen` を未指定アドレスで bind している構成で、CLI（`ApiClient`）が接続先アドレスと `Host` ヘッダをループバックの同じポートへ正規化するのは、`0.0.0.0:8082` のような値をそのまま `Host` に送るとガードの Host 検証に落ちるため。
6. 内蔵 gateway の HTTP クライアントにリクエスト全体のタイムアウトを設定していないのは、大きなファイルの配信を打ち切らないため（接続タイムアウトは 10 秒で別に設定している）。
7. レプリカ報告の送信を 1 件のエラーで打ち切り、残りは次の poll に回すのは、オフラインの署名アプリ相手に報告の件数ぶん `SIGN_TIMEOUT` を待たないため。
8. `kubo::locate_binary` が実行ファイルの隣や `PATH` から Kubo を探すので、複数のユーザーが使うホストでは `PATH` 上のすべてのディレクトリを信用することになる。そうしたホストでは `[kubo].binary` を明示するのが望ましい。
9. `swing-tray` の `rfd` を `default-features = false` にしているのは、既定の機能が Linux 用（GTK など）のものだけで、Linux の musl ビルドや `cargo clippy --workspace` に GTK を要求させないため。
10. `Daemon::stop` がまず Kubo の RPC でシャットダウンを要求するのは、シグナルで止める前に Kubo 自身に後始末の機会を与えるため。
11. `desktop-frame.css` を `desktop.js` が iframe の中に差し込むのは、親文書の CSS では iframe のスクロールバーの見た目を変えられないため。

## 検証したこと

- README・AGENTS.md・`docs/`（`docs/log/` と `docs/plan.md` を除く）の相対リンクとアンカーを検査するスクリプトを書いて実行し、見つかった壊れたリンク（旧ファイルの見出しを指したままのもの、移動先を指し直す必要があったもの）をすべて修正した。修正後、同じスクリプトで 0 件になることを確認した。

## 過去ログのリンク

見出しの移動で壊れた過去ログのリンク（graceful-stop、app-icons、release-workflow-and-kubo-signature、ghcr-image、release-actions-rationale、desktop-wallpaper-settings）は、ユーザーの了承を得て移動先（`kubo.md`・`up.md`・`release.md`・`dashboard/desktop.md`）へ向け直した。リンク先だけを変え、本文は書き換えていない。

## 追加の見直しで直したこと

- 誤り: `swing up` の起動順にシグナルハンドラの設定を含めていた（実際は `main.rs` が `up::run` の外で 1 回だけ設定する）。読み取り専用の例外から `dashboard rotate-token`（`swing up` が動いていなければトークンファイルを直接書く）が漏れていた。API を経由する `service stop` は Windows だけで、Linux は `systemctl stop`、macOS は `launchctl kill SIGTERM`。`/api/replicas` の `reporters` の並びは (tier, latest, hex)。アップロードの残骸の掃除は agent ではなく `up::run` の起動時。`recover_orphan` を呼ぶのは `swing.lock` を取った直後ではなく `run_managed` の冒頭。`kubo.pid` が書けなくても起動は続ける。
- 重複: レプリカ報告の集計・並べ替え・切り詰め、`MAX_FOLLOW_SET_ENTRIES`、セットアップモードの条件、設定例の再生成手順、API を使う CLI の一覧、`ApiClient` のループバックへの正規化、サイズ表示、ダッシュボードの停止待ち、Kubo gateway を公開しないこと、compose 用の Kubo 設定を、それぞれ 1 箇所（`nostr.md`・`up.md`/`signer.md`・`cli.md`・`http-api.md`・`kubo.md`）に寄せた。
- Kubo の版: コードのピン（`compose.yaml` と `KUBO_VERSION`）と文書中の版表記（README・`architecture.md` のテスト手順・`docker.md`・`kubo.md` の Gateway 節）を `kubo.md` で分けて列挙した。
- `dashboard/desktop.md` の見た目の細部（フォントの寸法、フォーカスの色、レイアウト）と `desk-` クラスの列挙を削り、見出しを「内部クラス（非安定）」にした。
- リンクの表示テキスト（ファイル名・アンカー）を href と一致させた。

### 検証したこと

- リンクとアンカーの検査スクリプトを `docs/log/` を含めて実行し、0 件であることを確認した。表示テキストと href の食い違いを検出するスクリプトでも 0 件になった。

## 4 回目の見直し（コア側）

- 誤り: `SETTINGS` に既定値の欄は無い（既定値は `config/build.rs`）。`collect_*` に分かれているのは `mirror.rs`・`health.rs`・`replicas.rs`・`webring.rs` で、`publish.rs` は段階別の関数を共有し、`nostr.rs` には無い。`[gateway].hosts` の空要素はエラーではなく捨てる。`[kubo].provide_strategy` は空白だけでもエラー。0 でエラーになる設定に `[dashboard].max_upload` が漏れていた。Windows の `service stop` は設定の解決や読み込みに失敗すると `/End` せずにエラー終了する。`uninstall` は先にトレイの登録を消す。NIP-46 の応答の購読は `since` ではなく `limit(0)`。`talking to the signer app failed` はペアリング経路だけの文言。nostr-sdk の通知チャネルの容量の記述（2048）は誤りだったので数値ごと消した。`001-swing-config.sh` は `Addresses.*` を設定しない。`authors` の最大 100 件はダッシュボード API だけ。トレイが消えるまでは最大約 15 秒。gateway の `extract_host` は `dashboard/guard.rs` のものと同じではない（`]` の後ろを検査しない）。
- 移動: 設定ファイルの探索順を `architecture.md` の「CLI」節から「設定と環境変数」節へ移した。Windows 向けのクロスビルドの手順を `architecture.md` のテスト節から `release.md` の「ローカルでのクロスビルド」へ移し、ローカル環境寄りの導入手順を削った。compose から `swing up` への移行手順を `docker.md` から README の Docker Compose の節へ移して圧縮した。
- `kubo.md` の Kubo の版を上げるときの文書側のチェックリストは「`v0.43.1` で検索して直す」1 文にした。
- `kubo.md` の Gateway 節は Kubo 自体の挙動の記録だったので、下記の調査結果としてここへ移し、architecture には `NoFetch` でローカルのブロックだけを返すことと、Kubo の Gateway ポートを直接公開しないことだけを残した。

### Kubo の Gateway（`NoFetch`）の挙動の調査結果

Kubo 0.43.1 / boxo 0.43.0 で確かめた。`Gateway.NoFetch=true` の下でブロックが無いときの応答:

| 状況 | 応答 |
|---|---|
| パス Gateway で `/ipfs/<cid>/` のルートブロックが無い | 404 |
| 上記以外（サブパス付き、サブドメイン・DNSLink のルート）で、途中までのブロックが無い | 500（`skip: ...`） |
| ファイルの途中のブロックが無い | 200 を返した後で本文が途切れる |
| サイトにあるブロックで辿れて、パスが無い | 404 |
| `Cache-Control: only-if-cached` | パスの最後のブロックがあれば 200、無ければ 412。その下の DAG が揃っているかは見ない |

オフラインでは boxo の fetcher が「ブロックが無い」を `traversal.SkipMe` に置き換え、Gateway がそれを not found と判定しないので 500 になる。サブドメインと DNSLink はルートでも `_redirects` を探すためにサブパスを辿るので 500 になる。

`127.0.0.1:8080`（Kubo 直接）はすべてのパスを受け付ける（`localhost` ではサブドメイン Gateway（`/ipfs/<cid>` は `<cid>.ipfs.localhost` へリダイレクト）、`127.0.0.1` ではパス Gateway）。`Gateway.PublicGateways` に入れたホストでは DNSLink（`_dnslink.<host>`）の内容だけを返し、`/ipfs/`・`/ipns/`・`/routing/v1` は 404。不正な名前があると Kubo は起動しない。Kubo は `Host` と `X-Forwarded-Host` をそのまま信じる。

## 5 回目の見直し

各指摘は `src/`・`web/`・`compose.yaml` で裏を取ってから直した。

- 誤り: `[dashboard].listen` に `off` は無い（`config::parse_dashboard_listen` は `SocketAddr` だけ。UI を止めるのは `ui = false`）。Desktop 画面のリンク集ページ（iframe）にも `desktop.js` が `desktop-frame.css` を差し込むので「ダッシュボードの CSS は一切当たらない」は誤りで、Desktop 画面の CSS の一覧にも加えた。`select_latest` は同じ `created_at` の版を `id` で決着させない（先に見た方が残る）。`kubo.pid` を書くのは `start_kubo`、`MAX_FOLLOW_SET_ENTRIES` の切り詰めの warn を出すのは `resubscribe_and_backfill`。`desktop-wallpaper-image.js` は純粋関数だけではない（ファイルの読み込み・canvas への描画を含む）。最小サイズ 320×240 はウィンドウだけ。「マイ コンピュータ」「ごみ箱」は `aria-hidden` でも選択でき、ステータスバーの文言は読み込み状態で変わる。`sanitizeMessage` は既定 200 文字で切り詰める。未ログインの判定はセットアップモードより優先する。「Loading webring…」は表示中の内容が無いときに毎回出る。ペアリングの状態に `idle` がある。署名アプリでのセットアップの成功表示は `remote-signer.json` の案内。760px 以下の `#page-footer` は Desktop 画面では出さない。`/api/config` の例の `value` と `raw` が食い違っていた。
- 追加: `service install`/`uninstall` が扱う `swing-tray` の自動起動（Windows の Run キー、macOS の tray 用 plist）、鍵が無くても動くコマンドに `dashboard open`/`rotate-token`、`webring` の `<key>` が複数取れること、`status` の `invalid_key`、ポリシー判定の reason 名（`max_update_size`・`max_per_site_exceeded_alone`・`max_total_storage`）と `keep_days = 0`、`kubo::tests` の `#[ignore]` テスト 3 本（`SWING_TEST_KUBO_BIN`）を `architecture.md` のテスト節と Kubo の版を上げる手順に、シグナル経由の停止が 10 秒（`FORCE_EXIT_GRACE_PERIOD`）で打ち切られ Kubo の 30 秒の猶予を待ち切らないことを `kubo.md` に、`main.rs` の `RUNTIME_SHUTDOWN_TIMEOUT`（10 秒）を `up.md` に、`web/desktop-icons.svg`・`setup.js` から `publish.js` への依存を `web.md` に。
- 重複: compose の `mirror` の環境変数・ポート・volume の書き写しをやめて `compose.yaml` を正本にし、ホスト公開用の `SWING_*_BIND` と `SWING_GATEWAY_LISTEN` の説明は `architecture.md` の「設定と環境変数」に 1 か所にした。ダッシュボードの寿命と `set_ready`/`set_not_ready` は `up.md` を正本にし、`agent.md` には後始末の順序だけを残した。署名方法の作り直しは `signer.md`、`public_url` 未設定時の URL は `dashboard.md` の設定節、設定の書き込み手順と env 由来キーの拒否は `dashboard.md`、突き合わせが保存量に比例して時間がかかることは `agent.md` の起動時の突き合わせ、Desktop 画面のリンク集ページのルート・Content-Type・差し替えは `dashboard.md` の静的ファイルの節を正本にした。`up.md` の「Dockerfile / compose との関係」節、`release.md` のクロスビルド手順の説明（コマンドだけ残した）、`release.md` の `release.Dockerfile` の説明、`service.md` の Run キーの箇条書き、`agent.md` の対象外イベントと未来ずれの二重説明、`web.md` の `pollUntil` の間隔・回数の繰り返し、`cli.md` の `swing stop` の既定値の繰り返しを削った。
- 整理: `web.md` の Settings 画面の説明（設定編集・テーマと言語とカスタム CSS・プロセス操作）を 1 つの節にまとめた。`desktop.md` のモジュール表を基盤のモジュールが先に来る順（`web.md` と同じ規約）に並べ替え、`desktop-focus.js` の役割を実態（フレームの登録とアクティブ表示の同期・モーダル判定・Tab の折り返し）に合わせた。Desktop 画面で `.swing-main` の幅制限と余白を外すことは CSS の値の書き写しをやめて `desktop.md` のレイアウト節に書いた。Windows API 名の逐語（`up.md` の親プロセス監視、`kubo.md` の Job Object）と warn 文字列の逐語（`agent.md` の unfollow）を圧縮し、理由の記述（`docker.md`・`service.md`・`tray.md`・`kubo.md`）を削った。削った理由のうち既存の log に無かったものは上の「architecture から削った理由」に足した。`nostr.md` の kind 35980・35981 は設定で変えられる既定値だと明記した。
- 直さなかったもの: `cli.md` の共通節にある `ApiClient` が未指定アドレスをループバックに直す記述は、`public_url` の決め方ではなく API への接続先の話なので残した。

### 検証したこと

- リンクとアンカーの検査スクリプトを `docs/log/` を含めて実行し、0 件であることを確認した。

## 6 回目の見直し

各指摘は `src/`・`web/`・`compose.yaml` で裏を取ってから直した。

- 誤り: `Config::ipfs_api_url()` を呼ぶのは CLI の `swing publish`・agent（`agent/lifecycle.rs`）・unmanaged の `swing up` で、`swing status` はダッシュボード API を叩くだけ。`/api/*` が 503 `agent is not ready` を返すのは relay 接続と Kubo の URL が決まるまでではなく、agent が起動時の突き合わせを終えて `set_ready` を呼ぶまで（正本は `http-api.md` の「共通」にし、`dashboard.md`・`cli.md` は参照だけにした）。セットアップモードを抜ける経路は `POST /api/setup` のほかに、`swing signer pair` で `remote-signer.json` を書いて再起動する経路もある。macOS の `service stop` は常に exit 0 で止まったままになるとは限らず、シグナルでの停止が 10 秒（`FORCE_EXIT_GRACE_PERIOD`）で打ち切られると exit 1 になり launchd が再起動しうる（managed の停止は agent 最大 15 秒と Kubo の猶予 30 秒）。systemd は `systemctl stop` による終了なら終了コードに関わらず再起動しない。`/api/mirror/add` で Follow Set が `MAX_FOLLOW_SET_ENTRIES` を超えるときのエラーは `api::upstream` を通るので 502。`webring` の `beyond` は深さの上限の外にいるものだけでなく、`MAX_CRAWL_NODES` で弾いたものも数えるので `over_budget` と重なりうる。`swing signer pair` の別アカウントの拒否は、確認の署名のリクエストより前に止まるとは限らない（状態をポーリングして `Checking` か `Ready` を見た時点）。`up::run` は `Signer::load` を最初には呼ばない。`lock.rs` の公開 API には `InstanceLock::path` もある。publish 側のディレクトリ名を `d` に戻すのは `mfs::site_from_name`。レプリカ報告の座標の kind は `[nostr].site_event_kind`。
- 移動: compose の `SWING_*_BIND` とポートの対応・`SWING_GATEWAY_LISTEN` の既定・内蔵 gateway の有効化を `architecture.md` から `docker.md` の compose.yaml 節へ移した。`docker.md` のダッシュボードのログイン手順・`SWING_DASHBOARD_PUBLIC_URL` の設定手順・コンテナでの `swing publish` の手順は README の該当節へのリンクにし、実装の事実（URL はコンテナ内の待ち受けから組み立てる、compose は `SWING_DASHBOARD_BIND` から `public_url` を作らない、アップロード方式なので volume は要らない）だけを残した。`dashboard.md` のリバースプロキシの設定手順（プロキシの種類・`Host` を書き換えないこと・`X-Forwarded-Proto`・`public_url`・Cloudflare Access・プロキシ側のヘッダのタイムアウト）を README の「ダッシュボードを外の端末から使う」へ移し、`dashboard.md` には Host 検証・Origin と `Host` の一致・`Secure` の条件だけを残した。
- 圧縮: `service.md` の systemd unit の全文・plist の全キー・タスク XML の設定値一覧を、動作に効く値の表にし、全文は `src/service.rs` の `systemd_unit`・`launchd_plist`・`schtasks_xml` を正本とした。`desktop.md` のフォーカスの移動先の表・Tab トラップの細則・位置保持ルールを短い段落にし、「拡張点」（タブやダイアログの足し方）を削った。`cli.md` の共通節のファイルを書くコマンドの説明を要約にし、トレイの自動起動の登録は `service.md` へのリンクだけにした。`kubo.md` の Kubo の版を上げる節を揃える場所の一覧とテストの案内に、`Daemon::stop` のログレベルの説明を 1 文にした。`cli.md` の `config example` の再生成コマンドを 1 文にした。`web.md` のサニタイズの Unicode 範囲と `http-api.md` の `display` の整形規則を、正本の関数名を示す形に縮めた。
- 重複: 取得と表示の上限の値（50・200・500・1000）を `http-api.md`・`cli.md`・`agent.md` で書き直すのをやめて定数名と `nostr.md` へのリンクにした。Follow Set とサイトイベントの新しさの比べ方（`select_latest` は同時刻を `id` で決着させない）は `nostr.md` の未来ずれの節に寄せ、`cli.md`・`agent.md` はリンクにした。`sites` の unfollow の見出しの説明は `agent.md#unfollow` へ寄せた。Kubo の Gateway を直接公開しないことは `kubo.md` だけに書き、`docker.md`・`gateway.md` はリンクにした。署名アプリがオフラインのときの扱いは `signer.md` の節にまとめ、`http-api.md` の待ち時間の記述はリンクにした。秘密鍵に戻す手順は README へのリンクにした。`nostr.md` の `Chosen` の作り方と `collect_sites` の説明、`dashboard.md` の `secret_key` を書ける API と既知の弱点・タイムアウト節のヘッダのタイムアウトの重複をまとめた。
- 語: 経緯を匂わせる「他は同期のまま」「流用」「現在編集可能な」「今は…のみ」を言い換えた。`desktop.md` の位置付けを、`architecture.md` の索引・`dashboard.md` の子ページ一覧・`desktop.md` の冒頭で「`dashboard.md` の子ページで `web.md` と並列」に揃えた。

### 検証したこと

- リンクとアンカーの検査スクリプトを `docs/log/` を含めて実行し、README に足した見出しへのリンクも含めて 0 件であることを確認した。

## 7 回目の見直し

各指摘は `src/`・`web/`・`tray/`・`.github/`・`docker/` で裏を取ってから直した。今回は指摘の箇所に加えて、編集した段落を 1 文ずつコードと突き合わせ直し、各ファイルを通読して同じ事実の言い換えの食い違いと重複を探した。

- 誤り: `nostr.md` の `title` の項が「検証せず」と矛盾していた。Follow Set の受信後の作者の照合は、自分の Follow Set を 1 件取る `fetch_follow_set` だけで、複数の作者をまとめて取る `fetch_follow_sets` はイベント自身の `pubkey` ごとに最新を選ぶだけ。`cid` の正規形を MFS のパスに使うとしていたのは誤り（版のパスは `created_at`）。release ワークフローはブランチでの手動実行でもイメージを push し、タグの ref での手動実行ならドラフトのリリースも作る。Windows の `schtasks /End` は `conhost.exe` を終わらせるだけで、`swing` は `--exit-with-parent` でグレースフルに止まる（強制終了ではない）。`swing publish` の不正な `--site`/`--url`/`--title` は何もせず終わるのではなくエラー終了。Settings 画面のホワイトリストの目印は `item.raw != null` で、`item.editable` は「ホワイトリストにあり env 由来でない」。`public_url` はログインリンクの頭（CLI とトレイ）とセッション cookie の `Secure` の判定の 2 か所で使う。`swing status` と `mirror add`/`remove` はダッシュボード API 経由。内蔵 gateway の 502 は接続か送信・応答ヘッダの受信のエラーで、応答待ちのタイムアウトは無い。kubo-init は managed と同じ値ではなく同じキーを設定する（`StorageMax` は文字列のまま Kubo が解釈）。`Daemon::stop` は 1・2 段目が共通で 3 段目だけ OS 依存。Windows の `service uninstall` でトレイが閉じるのは Run 値ではなくタスク `swing` の削除を見るためで、`install --no-tray` では閉じない。`up::run` は `locate_binary`・`version`・`recover_orphan` の失敗、unmanaged の Kubo API の URL の解決失敗、停止時の `Daemon::stop` の失敗ではエラーで終わる。`locate_binary`・`version`・`recover_orphan` は起動時に 1 回ではなく `run_managed` のたび。バックオフのリセットに使う Kubo の稼働時間は healthy になってから。agent は購読してから過去分を取得する。「保存の順序」の最初に Follow Set の対象判定がある（番号がずれたので参照を直した）。NIP-05 の warn ログは実際に検証したときだけでキャッシュで判断したときは出ない。トレイのメッセージの切り詰めは `…` を含めて 80 文字。`/api/status` の `invalid_key` でも `cid` は入る。`kubo.swarm_port` は常に文字列。`/api/publish/upload` の 409 は本体を最後まで受け取った後。セットアップモードの停止は agent のループを経ない。Setup 画面は `writable` を見ない。デスクトップの壁紙の保存値はページ読み込み時に 1 回読むだけで、保存に失敗したらダイアログは閉じない。ウィンドウの状態は画面を移っても保ち、既定の中央寄せは横方向だけ。明滅するのはタイトルバー。`swing sites`・`mirror list` は取得エラーのとき relay を閉じずに終わる。`ApiClient` はリクエストごとではなく作るたびにトークンを読む。`up.md` の「コマンドラインからは常に起動でき」を「鍵が無くても起動でき」にした。
- 重複: 10 秒の強制終了は `up.md#shutdown` だけに書き、`kubo.md`・`service.md` はリンクにした。ログインコード・`POST /api/login`・トークンの作り直しは仕組みを `dashboard.md`、入出力を `http-api.md` に分けた。ガードの説明はエンドポイントごとに繰り返さず `http-api.md` の「共通」に 1 回だけ書いた。Desktop の資産の読み込み時機、`custom_css` のフォールバック、Slowloris、リバースプロキシの 4 項目、トレイの「終了以外を無効」、`--system` の非対応、`boot.js`・来訪者カウンタ・`--desk-*` の独立性、ペアリングの画面の挙動をそれぞれ 1 か所に寄せた。`<redacted>` は `architecture.md` の設定の節だけに残した。
- 移動・整理: `http-api.md` の「共通」をステータスの表・認証とガード・準備状態の表・件数と負荷の小見出しに分けた。`service.md` のモジュール全体の説明を冒頭の「共通」節にまとめた。`tray.md` の登録の監視を独立した節にし、ロケールを冒頭の箇条書きへ移した。`web.md` の小見出しの無い段落の連続に見出しを付けた。`desktop.md` に同梱の CSS・リンク集ページ・バナー・アイコン・フォントの表と、差し替えページ向けの契約（差し込む要素のクラスと `data-status`・`data-kind`）を足し、「非安定」をそれ以外の `desk-` クラスに限った。`.gitattributes` の記述を `release.md` から `architecture.md` の設定の節へ移した。
- 削除: `cli.md` の `dashboard open` の利用者向けの手順、`docker.md` の `.env` の手順と言い換え、`kubo.md` の Kubo の移行の注意（README の移行手順にある）、`agent.md` の運用助言（`mirror remove` を使うこと、state を消したときの帰結）、`release.md` の macOS の確かめ方と rc の実装の細部、`gateway.md` のリンクだけの節、`dashboard.md` の 1 行だけの「終了」節とマクロの説明、`signer.md` のエラー文言の逐語の列挙と画面の文言（`web.md` へ）、`service.md`・`tray.md` の実装の細部。`docker.md` の「compose から `swing up` への移行」節は README へのリンクだけの節だが、過去の log がこのアンカーを参照しているので残した。
- README: 「今のところ配布物は用意していない」を、release ワークフローが作るビルド済みアーカイブがあればそれを使う書き方にした（指示の範囲で 48 行目だけ。「バイナリで動かす」節の同じ趣旨の文は変えていない）。

### 検証したこと

- リンクとアンカーの検査スクリプトを `docs/log/` を含めて実行し、0 件であることを確認した。
