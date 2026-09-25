# 残タスク

優先度は 高 / 中 / 低。出所は「plan §17」（初期計画の拡張項目）か「レビュー」（初期実装のレビュー・監査で出たもの）。

| 優先度 | タスク | 出所 |
|---|---|---|
| 中 | NIP-46: iPhone の Clave で、リンクの貼り付け・確認の署名・閉じているときの応答（プッシュで起きるか。`nostrconnect://` で指定した relay でも起きるのか、`wss://relay.powr.build` でないと起きないのか）を確かめ、画面と README の案内を合わせる | [NIP-46 対応](log/2026-09-24-nip46-remote-signer.md) |
| 低 | NIP-46: セットアップ後に、同じアカウントのまま秘密鍵と署名アプリを切り替える操作をダッシュボードに用意する（設定画面に「署名の方法」を置く案）。秘密鍵に切り替えるときは今の公開鍵と同じ鍵だけを受け付け、秘密鍵を消す前に確認する。秘密鍵が環境変数（`SWING_NOSTR_SECRET_KEY`）由来なら、ダッシュボードからは消せないので案内だけにする。アカウント自体を変える操作は作らない（Follow Set・公開したサイト・レプリカ報告が前のアカウントに残るため。手作業で `secret_key` か `remote-signer.json` を書き換える）。今は `swing up` を止めて `remote-signer.json`（か `secret_key`）を消し、セットアップからやり直す（署名アプリどうしのつなぎ直しは公開画面からできる） | [NIP-46 対応](log/2026-09-24-nip46-remote-signer.md) |
| 低 | CLI の出力に nostr-sdk の INFO ログ（`Connected to '<relay>'`・`Relay '<relay>' has been shutdown.`）が標準エラーに混ざる。`swing signer pair` では QR とメッセージの間に挟まって読みにくい。`swing up` 以外のコマンドでは既定のログレベルを下げるか、`nostr_sdk` を warn にする | [CLI からのペアリング](log/2026-09-25-cli-signer-pair.md) |
| 低 | NIP-46: 署名アプリが作った `bunker://` URI を貼って接続する方法（署名アプリ起点）。今は SWING が出す `nostrconnect://` の QR コードだけ | [NIP-46 対応](log/2026-09-24-nip46-remote-signer.md) |
| 低 | `signer::tests::answers_from_a_signer_whose_clock_runs_behind_are_received` がまれに落ちる（`src/signer.rs` の `remote.sign(...).await.unwrap()`）。`RemoteSigner` の応答待ちが 5 秒なので、マシンが重いと間に合わない可能性がある。`cargo test --workspace` を 1 回だけ回したときに 1 度落ち、直後の単独実行と全体の再実行では通った | [タスクトレイ](log/2026-09-24-tray-icon.md)の検証中に見つけた |
| 低 | Desktop 画面の表示倍率: WebKit では `zoom` の内側の文字サイズが整数 px に丸められ（175% で 12px が 12.25 デバイスピクセルになる）、150% では文字の位置が半ピクセルずれて、親のデスクトップもリンク集ページもぶれる。Linux の Playwright WebKit で確かめただけなので、Mac の Safari 実機（Retina で 110%〜175% にズームしたとき）でも起きるか確かめ、起きるなら対策を考える | [表示倍率を整数倍に揃える](log/2026-09-26-desktop-integer-scale.md) |
| 中 | NIP-05 の実 HTTP 経路の統合テスト（ローカル TLS エンドポイント相手、`#[ignore]`） | レビュー |
| 中 | 取得に失敗した CID を覚えて指数バックオフで再試行する。今は poll ごとに同じ CID の取得を試み、そのたびに最大 `SWING_FETCH_IDLE_TIMEOUT` の間、並行枠を 1 つ使う | レビュー（DoS） |
| 低 | レプリカ報告 1 件が持てる `cid` タグの数に上限が無い。誰でも 1 件の報告に大量の `cid` タグを詰め込める（サイトイベント・レプリカ報告・Follow Set の取得件数、1 作者が持つ `d` の数、Follow Set 1 件の `p` タグ数、`webring::crawl` のノード総数には [取得と表示の上限](log/2026-09-23-fetch-and-display-budgets.md) で上限を入れた） | レビュー（DoS） |
| 低 | `/api/sites`・`/api/replicas`・`/api/webring` の GET はサーバ側でキャッシュせず、同時実行数の制限もレート制限も無い。`keys`/`root`/`key` を 100 件に制限したのはリクエスト 1 回あたりの入力サイズを抑えるだけで、relay を引く GET 自体は何度リクエストしても毎回 relay に取得しに行く | レビュー（DoS） |
| 低 | `/api/status` が重い。サイト単位で DAG をたどるため保存量に比例して時間がかかるが、進捗表示もタイムアウトも無い（フロントはボタンを押したときだけ呼ぶ運用でしのいでいる） | レビュー |
| 低 | 容量の上限判定を実容量（版どうしの共有を数えない値）で行う。今は版ごとの `dag/stat` の和で判定していて、差分更新のサイトを実際より大きく見積もる。evict の途中経過ごとに測り直す必要があるので、`policy::decide` に suffix union の表を渡すなど、純粋関数のまま保てる形にする | [実容量の表示](log/2026-09-22-actual-storage-size.md) |
| 低 | `SWING_DASHBOARD_GATEWAY` を環境変数で空文字にできない（他の環境変数と同じく空文字は「未設定」として扱われ、既定値に戻る）。TOML の `gateway = ""` でなら無効にできる | レビュー |
| 低 | レプリカ報告の裏付け。報告に Peer ID を載せ、`routing/findprovs` でその Peer が CID を提供しているかを確かめる | レプリカ報告の実装 |
| 低 | NIP-05 のアドレスフィルタで NAT64（`64:ff9b::/96`）や 6to4（`2002::/16`）、Teredo（`2001::/32`）に埋め込まれた IPv4 を判定する | レビュー |
| 低 | ダッシュボードのヘッダ読み取りタイムアウト。`TimeoutLayer` はリクエストを受け取ってからしか効かず、ヘッダを少しずつ送る接続は切れない。`axum::serve` に設定が無いので `hyper_util` のサーバへ切り替える必要がある。HTTP のリバースプロキシの裏に置く構成ならプロキシ側で止まるので、平文のまま LAN に直接出す構成でだけ効いてくる（[リバースプロキシ経由での公開](architecture/dashboard.md#リバースプロキシ経由での公開)） | レビュー（DoS） |
| 低 | ダッシュボードのセッションを 1 つだけ取り消す手段。今は `swing dashboard rotate-token` で全セッションをまとめて無効にするしかない。サーバ側に何も持たない設計（HMAC の検証だけ）を崩すことになるので、発行時刻より前のセッションを拒否する「最小発行時刻」をトークンの横に持つ、などの軽い形から検討する | [ダッシュボードの認証](log/2026-09-23-dashboard-auth.md) |
| 低 | `state.json` とアップロードの一時ディレクトリのパーミッションを明示する。今は umask 任せ（`state.json` に秘密鍵は入らない） | レビュー |
| 低 | MFS パスは `mfs::site_name` で 1 回、`ipfs::query_path` で Kubo API のクエリとしてもう 1 回 percent-encode される。Kubo 側のデコードは 1 回なので正しく往復するが、片方だけを直接使う変更で壊れやすい。実 Kubo で空白や `!` を含む `d` の往復を確かめる統合テストを足す | 結合確認 |
| 低 | 「全履歴保持」オプション（`keep_versions` / `keep_days` を無制限にする明示的な設定） | plan §4 |
| 低 | private mode: WireGuard / Tailscale / private IPFS network を使う別モード | plan §17 |
| 低 | サブパス公開サイト向けに NIP-05 の代替検証（例: `<url>/.well-known/swing.json`）を検討 | レビュー |
| 低 | `compose.yaml` の `mirror.env_file: .env` が必須指定なので、`.env` が無いと `docker compose config` も失敗する（`--env-file` では代わりにならない） | 結合確認 |
| 低 | 自分専用のゲートウェイ（外出先からミラー済みサイトを見る）。ダッシュボードの認証と同じ `auth::sign_session` を用途ラベル `gateway-session` で使い、`[gateway]` に認証必須の `private_hosts` を分けて置く。ダッシュボードとは別オリジンにする（ミラーしたサイトの JS が cookie 付きで `/api/*` を叩けないように）。cookie は `Secure` 付き・期限 7 日程度。パス形式（`/ipfs/<cid>/`）だとサイトどうしが同じオリジンになり、subdomain 形式（`<cid>.ipfs.<host>`）だと 2 階層のワイルドカード証明書が要る。どちらにするかは未決 | [ダッシュボードの認証](log/2026-09-23-dashboard-auth.md) |
| 低 | gateway の TLS（`rustls-acme`）。前段（Cloudflare Tunnel など）に任せる運用で当面は不要だが、直接インターネットに晒す構成では要る | [配布方式の設計](log/2026-09-21-distribution-design.md)、[配布方式の実装](log/2026-09-23-distribution-implementation.md) |
| 中 | タスクトレイ: Windows と macOS の実機で確かめる（メニューの各操作、状態の表示の切り替わり、`schtasks` を呼ぶときにコンソールウィンドウが出ないこと、macOS で Dock に出ないこと、`service install` の後にログインし直してトレイが出ること、Windows のタスクマネージャーのスタートアップ アプリに出ること、macOS のメニューバーのアイコンがライトとダークの両方で見えて停止中は薄くなること、エクスプローラーで `swing.exe` と `swing-tray.exe` にアイコンが付くこと） | [タスクトレイ](log/2026-09-24-tray-icon.md) |
| 中 | Windows 実機で、タスクの登録判定（`schtasks /Query /TN swing` の失敗時に `schtasks /Query /FO CSV /NH` の一覧から未登録を判定する）が日本語・英語の両環境で正しく働くこと、`service stop` / `uninstall` が設定ファイルの無いディレクトリから実行したときに警告を出して `schtasks /End` にフォールバックすることを確かめる | [Windows のトレイ終了条件と service stop](log/2026-09-26-windows-tray-and-service-stop.md) |
| 低 | タスクトレイ: Linux 対応（`ksni` なら Rust だけで書けて musl でもビルドできる。GNOME は拡張を入れないと表示されない） | [タスクトレイ](log/2026-09-24-tray-icon.md) |
| 低 | macOS: `swing-tray` を `.app` バンドル（`SWING.app`。`Info.plist` と `.icns`）にして、Finder と「ログイン項目」にアイコンと名前を出す。LaunchAgent の `ProgramArguments`、`swing` と同じディレクトリにあるかの判定（`service install`）、release ワークフローの同梱の仕方が変わる | [アプリのアイコン](log/2026-09-24-app-icons.md) |
| 低 | インストーラとパッケージ。Homebrew tap（kubo は `depends_on "kubo"` で解決）、`install.sh`、winget（Inno の installer 型、`ipfs.exe` 同梱、ユーザー権限でのインストール、`InstallerType: inno`、インストール時にサービスを起動しない） | [配布方式の設計](log/2026-09-21-distribution-design.md) |
| 低 | ghcr.io の arm64 のイメージを arm64 の環境で実際に起動して確かめる（中身が aarch64 のバイナリであることまでは確認済み）。あわせて、最初の `v*` タグで `latest` とバージョンのタグが付くことを確かめる | [ghcr へのイメージ push](log/2026-09-25-ghcr-image.md) |
| 低 | release ワークフローに macOS の ad-hoc 署名（`rcodesign`）と GitHub の artifact attestation を入れる。attestation は private リポジトリだと GitHub Enterprise Cloud が要るので、public にしてから | [配布方式の設計](log/2026-09-21-distribution-design.md)、[release ワークフロー](log/2026-09-24-release-workflow-and-kubo-signature.md) |
| 低 | `content` の長さ上限。サーバ側では切らず `/api/sites` に全文を返している（表示はクライアントで 200 文字に切る） | 監査（自己申告の信用） |
| 低 | Desktop 画面のアイコンが NIP-05 の「対象外」と「検証済み」を区別しない（Sites 画面には N/A バッジがある） | 監査（自己申告の信用） |
| 低 | 「コントロール パネル」ダイアログに「背景」以外のタブ（例: 画面保護（スクリーンセーバー）風の何か、配色）を足す。タブ帯の構造（`data-tab`/`aria-controls`）は複数タブに対応済み | [Desktop 画面に壁紙設定を追加](log/2026-09-25-desktop-wallpaper-settings.md) |
| 低 | 特定のレプリカ報告者・Follow Set 由来のアカウントを個別にブロックする仕組み。今回の tier 分け（Author/Chosen/Other）はブロックではなく信頼度の提示だけ | [信頼度による tier 分け](log/2026-09-23-trust-tiers.md) |
| 中 | 取得バイト数の累積予算（`[agent] max_fetch_per_day` / `max_fetch_per_month`）。今の上限は 1 回・1 サイトあたりだけで、1 tick の総取得量に上限が無い。`src/ipfs.rs` の取得中のバイト数を state に積み、`policy::decide` の事前判定で `budget_exhausted` として skip する（`dag/export` を張る前に止める） | [間欠運用と通信量](log/2026-09-23-intermittent-operation-and-traffic-limits.md) |
| 低 | ミラー取得の一時停止トグル（ダッシュボードのスイッチ・設定・環境変数）と、取得の時間帯ウィンドウ（`active_hours`）。メータード回線の自動判定（NetworkManager の `Metered`、WinRT の `NetworkCostType`、`NWPathMonitor.isExpensive`）は OS ごとに分かれるので後回し | [間欠運用と通信量](log/2026-09-23-intermittent-operation-and-traffic-limits.md) |
| 低 | 上り（bitswap）の抑制。SWING からは観測も制御もできない。`swing up`（[配布方式の実装](log/2026-09-23-distribution-implementation.md)）で Kubo を子プロセスにする構成なら一時停止が上りにも効くが、compose では agent を止めても外部の `ipfs` コンテナは配り続ける（`SWING_KUBO_MANAGED=false` のため）。間接的な緩和として `Swarm.ConnMgr.HighWater` と `Reprovider.Interval` を `001-swing-config.sh` / `kubo::apply_config` 相当に足す余地がある。前提として Kubo v0.43.1 にバイトレートの帯域制限が無いことの確認が要る | [間欠運用と通信量](log/2026-09-23-intermittent-operation-and-traffic-limits.md) |
| 低 | README に「常時起動しない場合に何が起きるか」（レプリカ報告が `report_ttl` で expire する、ミラーが 0 人だと自分のサイトが読めなくなる）を書く。README と architecture のどちらに置くかは未決 | [間欠運用と通信量](log/2026-09-23-intermittent-operation-and-traffic-limits.md) |
| 低 | Windows: SCM サービスとしての登録（`service install --system` 相当、UAC 1 回、`windows-service` クレート）で OS シャットダウン時の graceful stop を得る | [グレースフルな停止](log/2026-09-23-graceful-stop.md) |
| 低 | ダッシュボード API が内部エラーの詳細をそのまま認証済みクライアントに返している（`src/dashboard/upload.rs`・`src/dashboard/api.rs` の `ApiError::Internal(format!("{e:#}"))`。ペアリング・remote-signer と設定ファイルの保存（`api::internal` は `error!` でログに残すが、同じ詳細をそのまま返す）・QR 生成・アップロードが該当）。絶対パスや OS のエラーメッセージが混じることがある。修正案: `tracing::error!` で詳細を残しつつ、`Internal` はクライアントには汎用メッセージだけ返す | レビュー（セキュリティ） |
