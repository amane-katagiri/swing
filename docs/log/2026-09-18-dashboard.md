# 2026-09-18 ダッシュボード（`swing agent` 内蔵の Web UI）

## 目的

`swing agent` にブラウザ向けの管理画面を追加する。sites / webring / publish / mirror 操作 / 設定確認を CLI を叩かずに行えるようにする。

## 決めたこと

| 決定 | 理由 |
|---|---|
| 新しいサブコマンドにせず、`swing agent` 起動時に同じプロセス内で立ち上げる | オタクくんの要望どおり、agent を起動すれば自動でダッシュボードも使える形にする |
| HTTP サーバは axum 0.8 | `hyper`/`tower` がすでに依存に入っており、追加する依存が小さい |
| agent の `Mutex<State>` には触れず、CLI と同じく `state.json` をディスクから読む | `reconcile`・sweep・保存処理などの state ロックと競合させたくない。`Agent` を private のまま保てる。CLI と全く同じ見え方になる（`swing sites` とダッシュボードの Sites 画面で表示が食い違わない） |
| relay 接続は agent の `Arc<RelayClient>` を共有する | リクエストのたびに relay へ接続し直すのは重く、relay 側にも負荷をかける |
| 上の 2 点のため、`mirror`・`health`・`replicas`・`webring`・`publish`・`nostr` を「relay/Kubo とやり取りしてデータを返す関数」と「それを表示する CLI 側の薄い関数」に分割した | CLI の出力を 1 バイトも変えずに、同じロジックをダッシュボードからも呼べるようにするため |
| JSON は手書きの DTO（`dashboard::dto`）。`Config` に `Serialize` を実装しない | 秘密鍵の値が型のレベルで JSON に出せないようにする。表示するのは npub / hex のみ |
| フロントはビルド工程なし・外部依存なしの `index.html` / `style.css` / `app.js` / `graph.js` の 4 ファイル固定。`include_str!` でバイナリに埋め込む | 単一バイナリ配布と Docker イメージを単純に保つ。CDN からスクリプトを読み込まない |
| Webring のグラフは自前の force layout（SVG、ドラッグ・パン・ズーム） | 外部ライブラリを増やさずに済ませる |
| 表示スタイルの切替（Sites の list/cards/table、Webring の graph/list/ascii/source）は `localStorage` に保存 | サーバ側に状態を持たせず、ブラウザごとの好みとして扱う |
| CSS カスタマイズは CSS 変数（`--swing-*`）＋安定した class・`data-*` 属性＋サーバ設定の `custom_css`（`/custom.css`）＋ブラウザの `localStorage` によるユーザー CSS の 2 段構え | 自分のサイトの雰囲気に寄せたい人（サーバ管理者）と、自分のブラウザだけ変えたい人（閲覧者）の両方に対応する |
| セキュリティは Host 検証（DNS rebinding 対策）、書き込み系の `X-Swing-Dashboard` ヘッダ必須＋ Origin 検証（CSRF 対策）、CSP・`nosniff`・`no-store` のレスポンスヘッダ。認証トークンは今回見送り | 既定で `127.0.0.1` にしか出さない前提であれば、認証よりまず「意図しない別オリジンからの操作」を塞ぐ方を優先した。認証は todo に追記 |
| `publish` の `dir` はダッシュボードのブラウザ側ではなく agent が動いているホスト（Docker ならコンテナの中）のパス | ブラウザからファイルをアップロードする経路は作らず、既存の `swing publish` と同じ「ローカルディレクトリを Kubo に add する」動きのままにした |
| mirror add/remove が relay に受理されたら `tokio::sync::Notify` で agent ループに知らせ、poll を待たずに sweep → Follow Set 再取得 → レプリカ報告同期を実行する | ダッシュボードで mirror を変更した直後に、次の poll（既定 5 分）を待たずに反映されてほしい |
| 設定ファイルの書き換え・鍵生成はダッシュボードから行わない | 設定ファイルへの書き込みは影響範囲が大きく、今回のスコープでは扱わない。todo に追記 |
| （結合確認・コードレビュー後）JSON ボディの構文エラー・必須フィールド欠落・`Content-Type` 不正はすべて 400 に統一し、422 は publish の NIP-05 `require` 失敗専用にした（`AppJson` エクストラクタで axum 既定の 422 判定を読み替え） | axum の既定は `JsonRejection` を 422 にするが、このダッシュボードでは 422 を「NIP-05 検証が通らなかった」という意味専用の記号にしたかった。他の入力不正と衝突させたくない |
| `POST /api/mirror/add`・`/remove` の `keys`、`GET /api/webring` の `root`、`GET /api/replicas` の `key` を 1 リクエストあたり最大 100 件に制限した | 1 回のリクエストで際限なく `parse_pubkey_inputs` や relay フェッチを積ませないための最低限の歯止め。件数上限そのものの todo とは別に、まず入力サイズだけ絞った |
| `own_pubkey` を `AppState::new` で起動時に 1 回だけ求めて保持する（リクエストごとに `Keys::parse` しない） | 秘密鍵のパースは軽い処理だが、リクエストのたびに行う理由が無い。`RelayClient::connect` がすでに同じ鍵で成功しているので、ここで失敗することは実運用上ない |
| ダッシュボードのサーバタスクを `JoinHandle` として `select!` で監視し、シャットダウン前に終了/panic したら error ログを出すだけで agent 本体は止めない | ダッシュボードの不具合で agent 全体（relay 通知・poll・mirror 保存）まで巻き込みたくない |
| フロントの各非同期取得（4 画面の読み込み、webring のノード選択、storage check）に世代カウンタを入れて古い応答を捨てる | 画面をすばやく切り替えたり root/depth を連続で変えたりしたときに、後から返ってきた古いレスポンスが新しい画面の上に描画されるのを防ぐ |
| Webring のグラフにキーボード操作（`tabindex`/`role=button`/Enter・Space）と `prefers-reduced-motion` 対応を入れた | マウス操作前提のグラフだけにしないため。結合確認でアクセシビリティの指摘を受けて追加した |

## 作ったもの

- `src/dashboard/mod.rs`: `AppState`（共有する relay・config・IpfsClient・`Notify`・起動時刻・publish 用ロック）、`router()`、`serve()`（graceful shutdown）。
- `src/dashboard/guard.rs`: Host 検証・書き込み系のヘッダ/Origin 検証・セキュリティヘッダ付与の middleware。純粋関数部分（`extract_host`・`host_allowed`・`origin_matches_host`）は単体テスト。
- `src/dashboard/api.rs`: `/api/overview`・`/api/sites`・`/api/status`・`/api/mirror`（GET/add/remove）・`/api/webring`・`/api/replicas`・`/api/publish`・`/api/config` のハンドラ。
- `src/dashboard/dto.rs`: 上記のレスポンス用 DTO と、ドメイン型からの変換。
- `src/dashboard/assets.rs`: `web/` の 4 ファイルの `include_str!` 配信と `/custom.css`。
- `web/index.html`・`web/style.css`・`web/app.js`・`web/graph.js`: フロント本体（4 画面のハッシュルーティング、Webring の自前グラフ、CSS 変数によるテーマ／カスタマイズ）。
- `src/agent.rs`: `run()` にダッシュボードの bind・起動・シャットダウン、`Notify` による即時 refresh の分岐を追加。
- `src/config.rs`: `[dashboard]` セクション（`listen`・`allowed_hosts`・`gateway`・`custom_css`）と対応する環境変数、`DashboardListen`（`off` / `SocketAddr`）。
- `src/mirror.rs`・`src/health.rs`・`src/replicas.rs`・`src/webring.rs`・`src/publish.rs`・`src/nostr.rs`: 「データを返す `collect_*` 系の関数」を切り出すリファクタ（CLI の出力は変えていない）。
- `Dockerfile`（`COPY web ./web`）、`compose.yaml`（`mirror` の `SWING_DASHBOARD_LISTEN`（既定 `0.0.0.0:8082`、`.env` で `off` にできる）と `SWING_DASHBOARD_BIND` によるポート公開）、`.env.example`・`swing.example.toml` にダッシュボードの項目を追加。
- ドキュメント: `docs/architecture/dashboard.md`（新規）、`docs/architecture.md`・`docs/architecture/agent.md`・`docs/architecture/docker.md`・`README.md`・`docs/todo.md` の更新。`docs/protocol.md`・`docs/extensions.md` は新しい kind や `d` タグを追加していないため変更していない。

## 検証

- ユニットテスト: `cargo test` で 216 件成功・11 件 `#[ignore]`（`cargo fmt --check` と `cargo clippy --all-targets -- -D warnings` も警告なし。レビュー指摘を反映した後の `src/` に対する結果）。`dashboard` 関連のテスト（`guard.rs` の Host/Origin 判定を含む）で、Host 検証・書き込み系ヘッダ/Origin 検証・JSON ボディの構文エラー/フィールド欠落/`Content-Type` 不正がいずれも 400 になること・`keys`/`root`/`key` の 100 件超過が 400 になること・`webring` の depth 上限・`/api/config` が秘密鍵の値を含まないこと・relay 未接続時に 500（パニックしない）になることを確認した。
- モックサーバ（実装側で用意したスタブ API）を相手に、4 画面の一通りの表示・フォーム操作をブラウザで確認した。
- 到達不能な relay と Kubo を指定した状態で `swing agent` を起動し、ダッシュボードが立ち上がって `/`・`/api/config`・`/api/overview` が返ること、`/api/sites` は Follow Set 無しとして 200 を返すこと、`/api/webring?depth=5` が 400 になることを `curl` で確認した。Host 検証（不正な `Host` で 403）、書き込み系のヘッダ・Origin 検証（`X-Swing-Dashboard` 無し・Origin 不一致で 403）も `curl` で確認した。

### 結合確認

環境: ローカル Docker の Kubo v0.43.1（`IPFS_PROFILE=test`）+ scsibug/nostr-rs-relay、鍵 3 つ（A/B/C）、サイト 4 件、B→A・C / C→B の mirror 関係。公開 relay・公開 IPFS には接続していない。NIP-05 は off。

実ブラウザ（agent-browser）で確認して OK だったこと:

- コンソールエラー・CSP 違反なし。
- 空状態 → ダッシュボードから mirror add → relay ごとの成否表示 → `Notify` で agent が即座に refresh して数秒で保存が始まる、という一連の流れ。
- Sites の list/cards/table 切り替え、gateway リンクが 200 で開けること、Storage check、mirror remove、Unfollowed 表示。
- Webring の graph・list・ascii・source が、CLI の `swing webring`（text/dot/mermaid）と向き・mutual まで一致すること。ノード選択時の replicas 表示が `swing replicas` の出力と一致すること。
- Publish（結果表示、2 回目の publish で `pruned` に前回の版が入ること、存在しない `dir` と不正な `url` が 400、relay 全滅が 502 になること）。
- `/api/config` のレスポンス全文とページ本文を秘密鍵の nsec・hex で grep して 0 件。
- Host 偽装・書き込み系ヘッダ欠落・Origin 不一致がいずれも 403、`allowed_hosts` に追加したホストは通ること。
- `custom_css` の内容がブラウザに反映されること。
- `[dashboard].listen = off` で起動できること、使用中のポートを指定すると起動時エラーで exit 1 になること。
- SIGINT で即座に終了すること。

CLI 出力の非破壊確認: git worktree で HEAD をビルドし、同じ relay/Kubo/state に対して `sites` / `mirror list` / `replicas` / `webring`(text, dot, mermaid) / `status` を新旧で実行し、出力を diff した。ログのタイムスタンプ行以外はバイト一致。

`#[ignore]` の統合テスト 3 種（`kubo_integration`、`agent_stores_and_removes_through_real_kubo`、`nostr_relay_integration`）もローカル Kubo/relay で pass。

結合確認で見つけて直したもの:

- `web/app.js` に生の制御バイトが混入していたのを、正規表現のエスケープ表記（`stripControlChars`）に直した。
- `/custom.css` に `Cache-Control` が無く、ファイルを書き換えてもブラウザのリロードで反映されないことがあった → `no-store` を付けた。
- グラフのノードのラベル文字がクリックできなかった（今はノードの `<g>` の子要素なのでクリックできる。合わせてキーボード操作・`prefers-reduced-motion` 対応・`ResizeObserver` による再 fit も追加した）。

### コードレビュー

読み取り専用のレビューで Critical/High の指摘は無かった。反映したものは上の「決めたこと」に追記した各項目（JSON エラーの 400/422 の切り分け、`keys`/`root`/`key` の 100 件上限、`own_pubkey` の事前計算、ダッシュボードタスクの監視、フロントの世代カウンタ、グラフのアクセシビリティ）。見送って `docs/todo.md` に回したものは、relay を引く GET のキャッシュ・同時実行制限の欠如、認証トークン。SIGTERM 未処理は当時 todo に回したが、第 2 弾（下記）で対応した。

## 第 2 弾: ブラウザからのフォルダアップロード・自分のサイト一覧・signal 修正・見た目の一新

### 決めたこと

| 決定 | 理由 |
|---|---|
| Publish の入力元を「Upload folder」（ブラウザから直接フォルダをアップロード）既定にし、「Path on agent host」（従来の `dir` 方式）は選択肢として残す | Docker Compose で使う場合、`dir` 方式だとサイトのディレクトリを `mirror` コンテナにあらかじめ `volumes` でマウントしておく必要があり、ダッシュボードから完結しない。アップロードなら volume が要らず、ブラウザだけで publish が完結する。一方でホスト側に既にあるサイトを毎回アップロードし直すのは無駄なので、パス指定も残した |
| アップロードはサーバの `<state_dir>/upload/<ランダム名>/` にストリーミングで展開し、`POST /api/publish` と同じ `run_publish` に渡す。成功・失敗どちらでも展開先を削除し、起動時にも `<state_dir>/upload/` を丸ごと掃除する | multipart を受けるハンドラを publish のロジックと別に持つより、共有できる部分（NIP-05・add・署名・prune）は共有し、違いを「サイトのディレクトリの出所」だけに閉じ込めたかった。異常終了で残った展開先がディスクを圧迫し続けないようにする |
| 「My sites」は `localStorage` ではなく `GET /api/publish/sites`（relay から自分の pubkey のサイトイベントを取得）を情報源にした | ブラウザのローカル状態は端末・ブラウザをまたいで共有されず、他の場所から publish した履歴も追えない。relay 上の実際のサイトイベントを見れば、どの端末からダッシュボードを開いても同じ一覧になる |
| デザインを白黒中立基調＋不透明度グレー＋アクセント 1 色（インクブルー）の Scandinavian design に変更し、`system-ui` のみのフォント・ロゴの回転アニメーション廃止・テーブルの縞模様を導入した | 初期実装の配色（serif 見出し・複数の彩度の高い色・回転するロゴ）が管理画面として主張が強すぎるという指摘を受けた。CSS 変数名・class・data 属性は変えず、値だけを差し替えられることを確認した上で適用した |
| i18n（英語・日本語）を追加し、`data-i18n` 属性と `MESSAGES`/`t()` で切り替える。`Webring` の名称、ASCII/DOT/Mermaid の出力、API のエラー文字列、npub/hex/CID/パス、環境変数名・設定キー名、nip05/health のステータス値、nip05 の off/warn/require は翻訳しない | オタクくんの利用者が日本語話者中心なため。一方で、固有名詞・識別子・他システムと突き合わせる値まで訳すと、ログや相手への説明で不便になるので、値そのものは常に元の文字列のまま出す |
| SIGINT・SIGTERM を永続リスナーで受け、`reconcile()`/`poll_once()` の実行中もそれと競争させて即座に処理を打ち切る（`race_with_shutdown`）。加えて独立な watchdog を置き、シグナル受信から 10 秒たっても終了していなければ強制 exit する | 結合確認で見つけた「SIGINT が最大で相手 relay のタイムアウト分（15 秒以上）効かないことがある」という不具合の根本原因に対する修正。詳しくは下記「SIGINT が効かないことがあった件」を参照 |
| `main.rs` を `#[tokio::main]` から明示的な `Runtime` + `shutdown_timeout(10s)` に変更した | ランタイムの drop はブロッキング呼び出しで詰まったワーカースレッドを待ち続けることがあるため、それを打ち切ってプロセスの終了を保証する |

### 作ったもの

- `src/dashboard/upload.rs`（新規）: `validate_relative_path`、`POST /api/publish/upload` のハンドラ（multipart のストリーミング受信・展開・`run_publish` 呼び出し・後始末）、起動時の `cleanup_upload_dir`。
- `src/dashboard/api.rs`: `run_publish` を `POST /api/publish` と `upload.rs` の共通処理として切り出し、`GET /api/publish/sites` を追加。`ApiError` に `Internal`（500）を復活。
- `src/dashboard/dto.rs`: `PublishUploadResultDto`（`PublishResultDto` を `#[serde(flatten)]`）、`PublishSiteDto`/`PublishSitesDto`、`OverviewDto`/`ConfigDto` に `max_upload` を追加。
- `src/dashboard/mod.rs`: `/api/publish/upload` に `DefaultBodyLimit::max(max_upload)` を `layer`。
- `src/config.rs`: `[dashboard].max_upload`（`SWING_DASHBOARD_MAX_UPLOAD`、既定 2GB、0 はエラー）。
- `src/agent.rs`: SIGINT/SIGTERM の永続リスナーと watchdog、`race_with_shutdown`、`shutdown_dashboard`（5 秒タイムアウト）。
- `src/main.rs`: `#[tokio::main]` をやめて明示的な `Runtime` + `shutdown_timeout(10s)`。
- `web/index.html`・`web/app.js`・`web/style.css`・`web/graph.js`: Publish 画面の「My sites」・Source トグル（Upload folder / Path on agent host）・アップロードのプログレス表示、i18n（`MESSAGES`/`t()`/`data-i18n`）、Scandinavian design への配色変更、webring グラフのラベルのズーム逆補正と定数調整（`LABEL_MAX`/`LINK_DISTANCE`/`REPULSION`）、`.swing-identity` の折り返し修正。
- `compose.yaml`・`.env.example`・`swing.example.toml`: `max_upload`/`SWING_DASHBOARD_MAX_UPLOAD` を追加。
- ドキュメント: `docs/architecture/dashboard.md`・`docs/architecture/agent.md`・`docs/architecture.md`・`README.md`・`docs/architecture/docker.md`・`docs/todo.md` を更新（本ログを含む）。

### SIGINT が効かないことがあった件

結合確認の初回で、到達不能な relay を指定した状態で SIGINT を送っても、最大で relay の接続タイムアウト分（15 秒以上）反応しないことがあった。原因は、`tokio::select!` が「今実行中の分岐の本体」を他の分岐と同時にはポーリングしないことにある。`agent.reconcile()` や `poll_once()` を select 分岐の本体として素朴に await していたため、その中で relay/Kubo の I/O を待っている間はシグナル用の分岐がまったくポーリングされず、シグナルが届いても本体の await が終わるまで気づけなかった。

当初は WSL2 のシグナル配送、ブラウザ側の keep-alive 接続、`ctrl_c()` を毎回生成し直していたことを疑ったが、いずれも原因ではなかった（シグナルリスナーを毎回生成から永続リスナーに変えただけでは症状が変わらないことを確認済み）。`race_with_shutdown` でシグナルの受信を `reconcile()`/`poll_once()` と同じ `select!` に載せて競争させることで直った。

### 検証（第 2 弾）

- backend: `cargo test` で 225 passed・11 ignored（`cargo fmt --check`・`cargo clippy --all-targets -- -D warnings` も警告なし）。ローカル Kubo に対して `curl -F` で `POST /api/publish/upload` を叩き、MFS に格納されることを確認。ボディが `max_upload` を超えると 413、パス検証違反（絶対パス・`..`・重複パスなど）が 400 になることを確認。
- frontend: モックサーバに対して agent-browser で Upload folder / Path on agent host の切り替え、progress 表示、My sites の Use ボタン、テーマ・言語切り替えを確認。配色は `tints.js` など評価用スクリプトでコントラスト比を確認した。
- シグナル: 到達不能な relay を指定し、(a) 起動直後の `reconcile()` 中、(b) poll 実行中、(c) アイドル中、の 3 状況それぞれで SIGINT・SIGTERM を送り、いずれも約 100ms で終了することを確認（修正前は (a)(b) で 15 秒以上かかっていた）。ローカルの relay（到達可能）に対するアイドル中の終了は 52ms。watchdog（10 秒強制 exit）が誤発火しないことも確認した。

### UI 調整

実機（Windows のブラウザ、スマホ幅）で触ったフィードバックをもとに 8 回に分けて調整した。上の「作ったもの」に書いた Source トグル（Upload folder / Path on agent host）は、この過程で「パス指定」側を UI から撤去し、常時フォルダアップロードに一本化した（`POST /api/publish` 自体は API として残しており、`docs/architecture/dashboard.md` にその旨を明記した）。

主な決定:

- **画面単位の「Reload」ボタンを廃止**し、画面ごとに合う形に変えた。Sites は見出し横のアイコンボタン、Webring はクエリフォームの「Update」が兼ねる、Publish は画面自体には置かず「My sites」だけ見出し横にアイコンボタン、Settings は無し。
- **パス指定 publish の UI を撤去**。ブラウザからのフォルダアップロードだけを常用の入力手段にした（Docker で volume が要らない利点を活かし、選択肢を増やして操作を迷わせるより 1 本化を優先した）。
- **Sites の並び順の既定を「更新順」にした**。最初は API の並び（pubkey 順）のままだったが、実際に使うと「最近更新されたサイトを先に見たい」という要求の方が強かった。名前順・pubkey 順も選べるようにして `localStorage` に保持する。
- **非同期ボタンに busy 表示を統一導入**（`setBusy()`：`disabled` + `aria-busy` + 幅を変えないスピナー）。ボタンを連打した際の二重送信や、何も起きていないように見える待ち時間への対策。
- **webring のグラフの矢印マーカーを根本修正**。ズームや辺の太さで矢印の見た目が崩れていたのを、`markerUnits="userSpaceOnUse"` の固定長にし、線の終端をマーカー分あらかじめ短くする方式に直した。
- **`color-scheme` をテーマ（`data-theme`）に追従させた**。ダークテーマを選んでいるのに `color-scheme` が既定の `light dark` のままだと、環境によってはチェックボックスなどネイティブ部品の配色がテーマと噛み合わない（黒背景に黒いチェックボックスが乗るなど）ことがあり、それを解消した。
