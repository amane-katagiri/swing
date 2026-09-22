# 残タスク

優先度は 高 / 中 / 低。出所は「plan §17」（初期計画の拡張項目）か「レビュー」（初期実装のレビュー・監査で出たもの）。

| 優先度 | タスク | 出所 |
|---|---|---|
| 高 | NIP-46 remote signer 対応。秘密鍵を `.env` に置かずに済む構成にする | plan §12 |
| 中 | NIP-05 の実 HTTP 経路の統合テスト（ローカル TLS エンドポイント相手、`#[ignore]`） | レビュー |
| 中 | 取得に失敗した CID を覚えて指数バックオフで再試行する。今は poll ごとに同じ CID の取得を試み、そのたびに最大 `SWING_FETCH_IDLE_TIMEOUT` の間、並行枠を 1 つ使う | レビュー（DoS） |
| 低 | relay から取得するサイトイベント・レプリカ報告・Follow Set の件数上限。`fetch_events` は件数無制限で、30 秒のタイムアウトだけで抑えている。レプリカ報告や、`#p` で見つかる Follow Set は誰でも出せるので、`swing replicas` / `sites` / `webring` で特に効く。`webring` はたどるアカウント数の上限も要る。 1 作者が持つ `d` の数（`replicas::collect` / `webring::collect` は作者の全サイトを引く）、Follow Set 1 件の `p` タグ数、`webring::crawl` の 1 レベルあたりの pubkey 数と総ノード数、レプリカ報告 1 件の `cid` タグ数にも上限が無い。ダッシュボードの `/api/replicas`・`/api/webring`・`/api/sites` も同じ `collect_*` 関数を呼ぶため、認証の無いブラウザからも同じ負荷をかけられる。ダッシュボード側は `keys`/`root`/`key` を 100 件に制限したが、これはリクエスト 1 回あたりの入力サイズを抑えるだけで、relay を引く GET 自体にはサーバ側のキャッシュも同時実行数の制限も無く、何度リクエストしても毎回 relay に取得しに行く | レビュー（DoS） |
| 中 | ダッシュボードの認証トークン。今は Host 検証・書き込み系の CSRF 対策（`X-Swing-Dashboard` ヘッダ・Origin 検証）だけで、閲覧そのものへの認証は無い。既定の bind 先（`127.0.0.1`）から出さない前提で見送った | レビュー |
| 低 | `/api/status` が重い。サイト単位で DAG をたどるため保存量に比例して時間がかかるが、進捗表示もタイムアウトも無い（フロントはボタンを押したときだけ呼ぶ運用でしのいでいる） | レビュー |
| 低 | 容量の上限判定を実容量（版どうしの共有を数えない値）で行う。今は版ごとの `dag/stat` の和で判定していて、差分更新のサイトを実際より大きく見積もる。evict の途中経過ごとに測り直す必要があるので、`policy::decide` に suffix union の表を渡すなど、純粋関数のまま保てる形にする | [実容量の表示](log/2026-09-22-actual-storage-size.md) |
| 低 | `SWING_DASHBOARD_GATEWAY` を環境変数で空文字にできない（他の環境変数と同じく空文字は「未設定」として扱われ、既定値に戻る）。TOML の `gateway = ""` でなら無効にできる | レビュー |
| 低 | ダッシュボードからの設定変更・鍵生成（鍵未設定での初期セットアップを含む）。設定ファイルの書き換えを伴うため今回は見送った | レビュー |
| 低 | レプリカ報告の裏付け。報告に Peer ID を載せ、`routing/findprovs` でその Peer が CID を提供しているかを確かめる | レプリカ報告の実装 |
| 低 | NIP-05 のアドレスフィルタで NAT64（`64:ff9b::/96`）や 6to4（`2002::/16`）、Teredo（`2001::/32`）に埋め込まれた IPv4 を判定する | レビュー |
| 低 | ダッシュボードのヘッダ読み取りタイムアウト。`TimeoutLayer` はリクエストを受け取ってからしか効かず、ヘッダを少しずつ送る接続は切れない。`axum::serve` に設定が無いので `hyper_util` のサーバへ切り替える必要がある | レビュー（DoS） |
| 低 | `state.json` とアップロードの一時ディレクトリのパーミッションを明示する。今は umask 任せ（`state.json` に秘密鍵は入らない） | レビュー |
| 低 | MFS パスは `mfs::site_name` で 1 回、`ipfs::query_path` で Kubo API のクエリとしてもう 1 回 percent-encode される。Kubo 側のデコードは 1 回なので正しく往復するが、片方だけを直接使う変更で壊れやすい。実 Kubo で空白や `!` を含む `d` の往復を確かめる統合テストを足す | 結合確認 |
| 低 | 「全履歴保持」オプション（`keep_versions` / `keep_days` を無制限にする明示的な設定） | plan §4 |
| 低 | private mode: WireGuard / Tailscale / private IPFS network を使う別モード | plan §17 |
| 低 | サブパス公開サイト向けに NIP-05 の代替検証（例: `<url>/.well-known/swing.json`）を検討 | レビュー |
| 低 | `compose.yaml` の `mirror.env_file: .env` が必須指定なので、`.env` が無いと `docker compose config` も失敗する（`--env-file` では代わりにならない） | 結合確認 |
| 中 | `swing up`: Kubo を子プロセスとして起動・管理する supervisor。init、config 適用、`/api/v0/id` でのヘルス待ち、バックオフ再起動、終了時の子プロセス回収。compose の `depends_on: service_healthy` / `healthcheck` / `restart: unless-stopped` に相当する | [配布方式の設計](log/2026-09-21-distribution-design.md) |
| 中 | `docker/kubo-init.d/001-swing-config.sh` 相当を Rust に移植する（`Datastore.StorageMax`、`Provide.Strategy`、`Gateway.NoFetch`、`Gateway.PublicGateways`） | [配布方式の設計](log/2026-09-21-distribution-design.md) |
| 中 | gateway プロファイルの Caddy 相当（ホスト名での振り分けと Kubo gateway へのプロキシ）を axum に実装する。TLS が要るなら `rustls-acme` | [配布方式の設計](log/2026-09-21-distribution-design.md) |
| 中 | `swing service install / uninstall / status`。systemd user unit（`loginctl enable-linger`、`--system` も）、launchd の LaunchAgent、Windows はタスクスケジューラ（`sc.exe` は UAC が出るので使わない） | [配布方式の設計](log/2026-09-21-distribution-design.md) |
| 低 | インストーラとパッケージ。Homebrew tap（kubo は `depends_on "kubo"` で解決）、`install.sh`、winget（Inno の installer 型、`ipfs.exe` 同梱、ユーザー権限でのインストール、`InstallerType: inno`、インストール時にサービスを起動しない） | [配布方式の設計](log/2026-09-21-distribution-design.md) |
| 低 | リリースワークフローに macOS の ad-hoc 署名（`rcodesign`）と GitHub の artifact attestation を入れる | [配布方式の設計](log/2026-09-21-distribution-design.md) |
| 低 | compose 専用の環境変数（`SWING_KUBO_GATEWAY_BIND`、`SWING_DASHBOARD_BIND` など）を `swing.toml` へ寄せる。compose 側は外部の Kubo を使う設定にする | [配布方式の設計](log/2026-09-21-distribution-design.md) |
| 低 | Kubo RPC を Unix socket か loopback の動的ポートに閉じる。単一プロセスで動かすなら 5001 を固定する必要が無い | [配布方式の設計](log/2026-09-21-distribution-design.md) |
| 低 | 配布前に確かめること: upstream の kubo darwin-arm64 バイナリが署名されているか、Windows のファイアウォール（4001）の初回ダイアログの扱い、既存 compose 利用者が `ipfs-data` から移行する手順 | [配布方式の設計](log/2026-09-21-distribution-design.md) |
| 中 | レプリカ報告者の数に上限を付ける。`replicas::collect` は報告者を全部集めて報告者ごとに Follow Set も引き、`web/webring.js` は報告者数ぶん `<li>` を作る。報告者は捨て鍵で量産できる | 監査（自己申告の信用） |
| 中 | サイト一覧（`web/sites.js` のカードと表）の容量は自己申告の `size` タグをそのまま表示している。実測値は Storage check にしか出ない。申告値であることを示すか、実測値を並べる | 監査（自己申告の信用） |
| 低 | `duplicate_cid` の比較を CID の正規形で行う。今は文字列比較なので base32 と base58btc で書いた同じ CID が別の版になる | 監査（自己申告の信用） |
| 低 | `content` の長さ上限。サーバ側では切らず `/api/sites` に全文を返している（表示はクライアントで 200 文字に切る） | 監査（自己申告の信用） |
| 低 | `cid` がディレクトリの root であることの確認。今は構文検証だけで、ファイル単体や raw block も保存される | 監査（自己申告の信用） |
| 低 | Desktop 画面のアイコンが NIP-05 の「対象外」と「検証済み」を区別しない（Sites 画面には N/A バッジがある） | 監査（自己申告の信用） |
