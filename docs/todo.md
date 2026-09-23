# 残タスク

優先度は 高 / 中 / 低。出所は「plan §17」（初期計画の拡張項目）か「レビュー」（初期実装のレビュー・監査で出たもの）。

| 優先度 | タスク | 出所 |
|---|---|---|
| 高 | NIP-46 remote signer 対応。秘密鍵を `.env` に置かずに済む構成にする | plan §12 |
| 中 | NIP-05 の実 HTTP 経路の統合テスト（ローカル TLS エンドポイント相手、`#[ignore]`） | レビュー |
| 中 | 取得に失敗した CID を覚えて指数バックオフで再試行する。今は poll ごとに同じ CID の取得を試み、そのたびに最大 `SWING_FETCH_IDLE_TIMEOUT` の間、並行枠を 1 つ使う | レビュー（DoS） |
| 低 | レプリカ報告 1 件が持てる `cid` タグの数に上限が無い。誰でも 1 件の報告に大量の `cid` タグを詰め込める（サイトイベント・レプリカ報告・Follow Set の取得件数、1 作者が持つ `d` の数、Follow Set 1 件の `p` タグ数、`webring::crawl` のノード総数には [取得と表示の上限](log/2026-09-23-fetch-and-display-budgets.md) で上限を入れた） | レビュー（DoS） |
| 低 | `/api/sites`・`/api/replicas`・`/api/webring` の GET はサーバ側でキャッシュせず、同時実行数の制限もレート制限も無い。`keys`/`root`/`key` を 100 件に制限したのはリクエスト 1 回あたりの入力サイズを抑えるだけで、relay を引く GET 自体は何度リクエストしても毎回 relay に取得しに行く | レビュー（DoS） |
| 高 | ダッシュボードの認証トークン。今は Host 検証・書き込み系の CSRF 対策（`X-Swing-Dashboard` ヘッダ・Origin 検証）だけで、閲覧そのものへの認証は無い。API は `swing up` の寿命でずっと有効で、CLI の一部（`status`/`mirror add`/`mirror remove`/`stop`）も依存するが、既定の bind 先（`127.0.0.1`）から出さない前提で見送っている。`PUT /api/config`／`POST /api/setup`（[ダッシュボードからの設定編集とセットアップ](log/2026-09-23-dashboard-settings-and-setup.md)）でダッシュボードから設定ファイルの書き換え・秘密鍵の生成までできるようになったため、`127.0.0.1` から出た構成（compose の `SWING_DASHBOARD_LISTEN=0.0.0.0:8082` など）では影響が広がった。優先度を中から高へ上げる | レビュー |
| 低 | `/api/status` が重い。サイト単位で DAG をたどるため保存量に比例して時間がかかるが、進捗表示もタイムアウトも無い（フロントはボタンを押したときだけ呼ぶ運用でしのいでいる） | レビュー |
| 低 | 容量の上限判定を実容量（版どうしの共有を数えない値）で行う。今は版ごとの `dag/stat` の和で判定していて、差分更新のサイトを実際より大きく見積もる。evict の途中経過ごとに測り直す必要があるので、`policy::decide` に suffix union の表を渡すなど、純粋関数のまま保てる形にする | [実容量の表示](log/2026-09-22-actual-storage-size.md) |
| 低 | `SWING_DASHBOARD_GATEWAY` を環境変数で空文字にできない（他の環境変数と同じく空文字は「未設定」として扱われ、既定値に戻る）。TOML の `gateway = ""` でなら無効にできる | レビュー |
| 低 | レプリカ報告の裏付け。報告に Peer ID を載せ、`routing/findprovs` でその Peer が CID を提供しているかを確かめる | レプリカ報告の実装 |
| 低 | NIP-05 のアドレスフィルタで NAT64（`64:ff9b::/96`）や 6to4（`2002::/16`）、Teredo（`2001::/32`）に埋め込まれた IPv4 を判定する | レビュー |
| 低 | ダッシュボードのヘッダ読み取りタイムアウト。`TimeoutLayer` はリクエストを受け取ってからしか効かず、ヘッダを少しずつ送る接続は切れない。`axum::serve` に設定が無いので `hyper_util` のサーバへ切り替える必要がある | レビュー（DoS） |
| 低 | `state.json` とアップロードの一時ディレクトリのパーミッションを明示する。今は umask 任せ（`state.json` に秘密鍵は入らない） | レビュー |
| 低 | MFS パスは `mfs::site_name` で 1 回、`ipfs::query_path` で Kubo API のクエリとしてもう 1 回 percent-encode される。Kubo 側のデコードは 1 回なので正しく往復するが、片方だけを直接使う変更で壊れやすい。実 Kubo で空白や `!` を含む `d` の往復を確かめる統合テストを足す | 結合確認 |
| 低 | 「全履歴保持」オプション（`keep_versions` / `keep_days` を無制限にする明示的な設定） | plan §4 |
| 低 | private mode: WireGuard / Tailscale / private IPFS network を使う別モード | plan §17 |
| 低 | サブパス公開サイト向けに NIP-05 の代替検証（例: `<url>/.well-known/swing.json`）を検討 | レビュー |
| 低 | `compose.yaml` の `mirror.env_file: .env` が必須指定なので、`.env` が無いと `docker compose config` も失敗する（`--env-file` では代わりにならない） | 結合確認 |
| 低 | gateway の TLS（`rustls-acme`）。前段（Cloudflare Tunnel など）に任せる運用で当面は不要だが、直接インターネットに晒す構成では要る | [配布方式の設計](log/2026-09-21-distribution-design.md)、[配布方式の実装](log/2026-09-23-distribution-implementation.md) |
| 低 | インストーラとパッケージ。Homebrew tap（kubo は `depends_on "kubo"` で解決）、`install.sh`、winget（Inno の installer 型、`ipfs.exe` 同梱、ユーザー権限でのインストール、`InstallerType: inno`、インストール時にサービスを起動しない） | [配布方式の設計](log/2026-09-21-distribution-design.md) |
| 低 | リリースワークフローに macOS の ad-hoc 署名（`rcodesign`）と GitHub の artifact attestation を入れる | [配布方式の設計](log/2026-09-21-distribution-design.md) |
| 中 | macOS でのコンパイル確認（CI か実機で `cargo check`）。Windows は `cargo xwin clippy --target x86_64-pc-windows-msvc` で通ることを確認済み（[多重起動防止と孤児 Kubo 回収](log/2026-09-23-instance-lock-and-orphan-kubo.md) の検証を参照）。macOS は SDK を自動取得できないため WSL からは未確認（`cargo-zigbuild` で通る可能性はある） | [配布方式の実装](log/2026-09-23-distribution-implementation.md) |
| 中 | 配布前に確かめること: Windows のタスクスケジューラ（S4U ログオン、非対話セッション）で Kubo の 4001 inbound に対するファイアウォールのダイアログが出ず黙って遮られる可能性（`netsh advfirewall` でのルール追加は管理者権限が要る）、upstream の kubo darwin-arm64 バイナリが署名されているか、既存 compose 利用者が `ipfs-data` から `swing up`（managed Kubo）へ移行する手順 | [配布方式の設計](log/2026-09-21-distribution-design.md)、[配布方式の実装](log/2026-09-23-distribution-implementation.md) |
| 低 | Windows の Job Object（`kubo::windows_job`）は実機・実 CI で未検証。`cargo xwin` で `swing` 本体の Windows 向けビルドは通るが、実際に swing を `TerminateProcess` したときに Kubo が道連れで落ちるかは未確認 | [多重起動防止と孤児 Kubo 回収](log/2026-09-23-instance-lock-and-orphan-kubo.md) |
| 低 | `content` の長さ上限。サーバ側では切らず `/api/sites` に全文を返している（表示はクライアントで 200 文字に切る） | 監査（自己申告の信用） |
| 低 | Desktop 画面のアイコンが NIP-05 の「対象外」と「検証済み」を区別しない（Sites 画面には N/A バッジがある） | 監査（自己申告の信用） |
| 低 | 特定のレプリカ報告者・Follow Set 由来のアカウントを個別にブロックする仕組み。今回の tier 分け（Author/Chosen/Other）はブロックではなく信頼度の提示だけ | [信頼度による tier 分け](log/2026-09-23-trust-tiers.md) |
| 中 | 取得バイト数の累積予算（`[agent] max_fetch_per_day` / `max_fetch_per_month`）。今の上限は 1 回・1 サイトあたりだけで、1 tick の総取得量に上限が無い。`src/ipfs.rs` の取得中のバイト数を state に積み、`policy::decide` の事前判定で `budget_exhausted` として skip する（`dag/export` を張る前に止める） | [間欠運用と通信量](log/2026-09-23-intermittent-operation-and-traffic-limits.md) |
| 低 | ミラー取得の一時停止トグル（ダッシュボードのスイッチ・設定・環境変数）と、取得の時間帯ウィンドウ（`active_hours`）。メータード回線の自動判定（NetworkManager の `Metered`、WinRT の `NetworkCostType`、`NWPathMonitor.isExpensive`）は OS ごとに分かれるので後回し | [間欠運用と通信量](log/2026-09-23-intermittent-operation-and-traffic-limits.md) |
| 低 | 上り（bitswap）の抑制。SWING からは観測も制御もできない。`swing up`（[配布方式の実装](log/2026-09-23-distribution-implementation.md)）で Kubo を子プロセスにする構成なら一時停止が上りにも効くが、compose では agent を止めても外部の `ipfs` コンテナは配り続ける（`SWING_KUBO_MANAGED=false` のため）。間接的な緩和として `Swarm.ConnMgr.HighWater` と `Reprovider.Interval` を `001-swing-config.sh` / `kubo::apply_config` 相当に足す余地がある。前提として Kubo v0.43.1 にバイトレートの帯域制限が無いことの確認が要る | [間欠運用と通信量](log/2026-09-23-intermittent-operation-and-traffic-limits.md) |
| 低 | README に「常時起動しない場合に何が起きるか」（レプリカ報告が `report_ttl` で expire する、ミラーが 0 人だと自分のサイトが読めなくなる）を書く。README と architecture のどちらに置くかは未決 | [間欠運用と通信量](log/2026-09-23-intermittent-operation-and-traffic-limits.md) |
| 低 | Windows: SCM サービスとしての登録（`service install --system` 相当、UAC 1 回、`windows-service` クレート）で OS シャットダウン時の graceful stop を得る | [グレースフルな停止](log/2026-09-23-graceful-stop.md) |
