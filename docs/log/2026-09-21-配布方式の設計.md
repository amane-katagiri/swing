# Docker compose に依らない配布方式の設計

実装は行っていない。方針の決定と、その根拠として調べた事実だけを残す。

## 出発点

compose の 3 サービスを、Docker の無い環境へどう届けるか。

| サービス | 実体 | 単一実行ファイル化 |
|---|---|---|
| `mirror` | swing 本体（Rust） | 既に単一バイナリ |
| `gateway` | Caddy。許可ホストなら `ipfs:8080` へプロキシ、それ以外 404 | axum に吸収できる |
| `ipfs` | Kubo（Go） | 別プロセスにするしかない |

## 決めたこと

- Kubo を実行時に自己展開する方式は採らない。インストーラ（またはパッケージマネージャ）が配置する。
- compose の `depends_on: service_healthy` / `healthcheck` / `restart: unless-stopped` 相当は `swing up` の supervisor として Rust 側に実装する。kubo は swing の子プロセスとして面倒を見る。OS に登録するサービスは swing 1 個だけにする。
  - OS のサービス機構に kubo と swing を別々に登録する案は採らない。「healthy を待ってから起動」が systemd では素直に書けず、Windows のタスクスケジューラには依存関係の表現が無いため。
- Caddy 相当（ホスト名での振り分けと Kubo gateway へのプロキシ）は axum に実装する。TLS が要るなら `rustls-acme`。
- 導線は OS ごとのワンコマンドにする。GUI インストーラは作らない。

  | OS | 導線 | kubo |
  |---|---|---|
  | macOS | Homebrew tap | formula 依存（`depends_on "kubo"`）で解決。同梱しない |
  | Linux | `curl -fsSL .../install.sh \| sh`、AUR、nixpkgs | 依存解決またはインストーラが取得 |
  | Windows | winget（Inno Setup の installer 型） | `ipfs.exe` を同梱 |

- 常駐の登録は `swing service install / uninstall / status` として swing 自身に持たせ、ユニット定義は swing が生成する。

  | OS | 実体 | 注意 |
  |---|---|---|
  | Linux | `~/.config/systemd/user/swing.service` | `loginctl enable-linger` が要る。サーバ向けに `--system` も用意 |
  | macOS | `~/Library/LaunchAgents/jp.ne.ama.swing.plist` | `KeepAlive` |
  | Windows | タスクスケジューラ（ログオン時＋失敗時再実行） | `sc.exe` のサービス登録は管理者権限が要り UAC が出るので使わない |

- コード署名は当面行わない。macOS の ad-hoc 署名（`rcodesign` なら Linux の CI からでも打てる）と GitHub の artifact attestation だけ付ける。
- compose は残す。`swing up` を foreground の supervisor にしておけば、サービスから起動されても compose の `mirror` コンテナとして起動されても同じコードパスになる。compose 側は外部の kubo を使う設定（例: `[ipfs] external = true`）にする。

## 調べたこと

### Kubo の repo 移行はオフラインで完結する

`ipfs daemon --migrate=true` が外部の `fs-repo-migrations` をダウンロードするのは、古い repo version からの移行に限る。v0.43.1 では:

- `repo/fsrepo/migrations/migrations.go` の `RunHybridMigrations` に `embeddedMigrationsMinVersion = 16`。現在と目標の双方が 16 以上なら `RunEmbeddedMigrations` に入り、ネットワークを使わない。
- `repo/fsrepo/migrations/embedded.go` に `fs-repo-16-to-17` と `fs-repo-17-to-18` が埋め込まれている。
- 旧 `RunMigration`（ダウンロードする方）は Deprecated。

新規に init した repo は最初から最新なので、この経路には入らない。ダウングレードは `allowDowngrade` が要るうえ revert 可能な移行に限られる。swing のバージョンと同梱・要求する kubo のバージョンは 1 対 1 で固定する。

### パッケージマネージャ側の kubo の有無

- Homebrew に `kubo` formula がある。バージョンは `compose.yaml` が固定しているのと同じ 0.43.1。`service` ブロック（`ipfs daemon`）を持つ。
- winget には kubo が無い（`IPFS/IPFS-Desktop` のみ）。Windows だけ `ipfs.exe` の同梱が要る。

### OS の警告は署名の有無ではなく「ネット由来の印」で決まる

- macOS の Gatekeeper（notarization 検査）は `com.apple.quarantine` 拡張属性の付いたファイルに対して働く。Windows の SmartScreen は Mark-of-the-Web に対して働く。どちらもブラウザ等が付けるもので、`curl` やパッケージマネージャ経由では付かない。
- したがってワンコマンド導線なら、notarization（Apple Developer Program、年 99 USD）も Authenticode も要らない。
- Apple Silicon は署名の無い実行ファイルの実行を拒否するが、ad-hoc 署名で足りる。これは無料。
- 逆に GUI インストーラ（.pkg / .dmg / ブラウザから落とす .exe）はブラウザ DL が前提なので、この印が必ず付く。GUI を選ぶと署名・notarization のコストが戻る。

### winget-pkgs の審査にコード署名の要件は無い

`doc/Validation.md` の全 10 ステップ、`doc/Policies.md`、`doc/FAQ.md`、`doc/ValidationFailureGuide.md` を確認した。

- 署名への言及は 2 か所だけ。Microsoft の CLA（同意書）と、MSIX/APPX の `SignatureSha256`。MSIX はそもそも署名しないとインストールできない形式なので、MSIX を選ばなければ未署名でよい。
- 実際の関門はスキャン。`doc/Policies.md` は「いずれかのセキュリティスキャンでフラグされた場合、アプリケーションの正当性や意図に関係なく受け入れられない」としている。
- `doc/FAQ.md` によれば、サンドボックスでインストールしたうえで「怪しいサービスが追加されていないか」を見て、さらに「インストール後にアプリを実行して怪しいプロセスが起動しないか」を確認する。モデレーターの手動承認もある。
- `doc/Validation.md` はサイレントインストールを要求する。UAC を含むダイアログで進行が止まると失敗する。
- portable 型の「インストール」はファイル配置・レジストリ・PATH 追加のみで post-install の実行が無い（`doc/FAQ.md`）。サービス登録ができないので installer 型が要る。
- 失敗した場合は誤検知として Microsoft に検体を提出し、解決後にモデレーターが `@wingetbot run` で再検証する経路がある。

この結果、Windows の配布側に次の制約を置く。

- インストール時にサービスを起動しない。登録だけ行い、起動は次回ログオンか明示的な操作に任せる。インストール中に kubo が 4001 で listen するとファイアウォールのダイアログが出て、サイレントインストールの要件に抵触する。
- ユーザー権限でインストールし（`%LOCALAPPDATA%\Programs\swing\`）UAC を出さない。
- `InstallerType` は `inno` を正しく宣言する。generic な `exe` だと winget が渡すサイレントスイッチが合わない。
- `ipfs.exe` は同梱し、インストール時に外部から取得しない。

## 見送ったこと

- **自己展開**（kubo を圧縮して swing に埋め込み、起動時に展開して実行）。展開先の安全確保（`/tmp` を避ける、アトミックな rename、flock、親ディレクトリの所有者確認、TOCTOU）が必要になるうえ、挙動がドロッパー型マルウェアと同じなので AV の誤検知を構造的に抱える。インストーラが配置すればいずれも不要になる。
- **GUI インストーラ**。OS ごとに別物を作ることになり「1 個」にならない。加えて上記のとおり署名コストが戻る。ターミナルを使わない層に配る段階まで保留する。
- **Rust 製 IPFS 実装の組み込み**（rust-ipfs / iroh）。Bitswap・DHT の provide・MFS・Gateway 互換を自前で持つことになる。
- **compose.yaml を埋め込んで `docker compose` を呼ぶランチャー**。Docker 依存が残るため目的を満たさない。
- **SignPath Foundation の無料 Authenticode 証明書**。winget 経路では不要と分かったので当面見送る。`Validation-Defender-Error` を踏んだ時点で取りに行く。証明書があると署名者単位で評判が蓄積するため、リリースごとに評判がリセットされない利点はある。

## 未確認

- upstream の kubo の darwin-arm64 バイナリが署名されているか。Homebrew 経由なら問題にならないが、インストーラが直接取得する経路を作る場合は確認が要る。
- Windows のファイアウォール（4001）の扱い。初回に許可ダイアログが出る。ユーザーが拒否すると暗黙のブロックルールが作られ、以後 P2P が繋がらなくなる。
- 既存の compose 利用者が `ipfs-data` ボリュームから移行する手順。

## 次にやること

- `swing up` の supervisor。kubo の init、config 適用、子プロセスの起動と終了時の回収、`/api/v0/id` でのヘルス待ち、バックオフ再起動。
- `docker/kubo-init.d/001-swing-config.sh` 相当を Rust に移植する（`Datastore.StorageMax`、`Provide.Strategy`、`Gateway.NoFetch`、`Gateway.PublicGateways`）。
- Caddy 相当を axum に実装する。
- `swing service install / uninstall / status`。
- compose 専用の環境変数（`SWING_KUBO_GATEWAY_BIND`、`SWING_DASHBOARD_BIND` など）を `swing.toml` 側へ寄せる。
- Kubo RPC を Unix socket か loopback の動的ポートに閉じる（単一プロセスなら 5001 を固定する必要が無い）。
