# 2026-09-23 ダッシュボードからの設定編集とセットアップ

`docs/todo.md` にあった「ダッシュボードからの設定変更・鍵生成（鍵未設定での初期セットアップを含む）」を実装した回。architecture 側の結果は [`up.md`](../architecture/up.md)・[`dashboard.md`](../architecture/dashboard.md)・[`dashboard/http-api.md`](../architecture/dashboard/http-api.md)・[`dashboard/web.md`](../architecture/dashboard/web.md)・[`cli.md`](../architecture/cli.md)・[`service.md`](../architecture/service.md)・[`docker.md`](../architecture/docker.md)・[`../architecture.md`](../architecture.md)、README に反映済み（このログには経緯だけを書く）。

## 決めたこと

- 書き込める設定キーはホワイトリスト（`src/settings.rs::EDITABLE_KEYS`）に限定する。パス・待ち受けアドレス・ポート・`kubo.binary`・`dashboard.ui`・`allowed_hosts`・`kubo.managed`・`ipfs.*`・イベント kind・`gateway.*` は対象外にした。ダッシュボードに認証が無い（`docs/todo.md` の既知の課題）ことを前提に、書けると困るものを最初から狭く絞る方針にした。
- 秘密鍵はホワイトリストに入れず、`PUT /api/config` からは絶対に書けないようにした。書けるのは `POST /api/setup`（鍵が未設定のときだけ使える）に限定し、`secret_key: null` なら生成、文字列なら nsec/hex としてパースする。応答は `npub` だけで、鍵の値そのものはどのレスポンスにも出さない。
- 環境変数由来の値（`source: "env"`）は、たとえホワイトリストに載っていても編集不可にする。`PUT`/`POST /api/setup` は 1 つでも env 由来のキーが混ざっていれば要求全体を 400 で拒否し、部分適用はしない（`settings::check_not_env_sourced`）。
- 設定ファイルのパスを常に 1 つに決まるようにした（`config::resolve_config_path` が `--config`／`SWING_CONFIG`／`<カレントディレクトリ>/swing.toml` のいずれかを、存在確認せずに返す）。これにより「鍵も `swing.toml` も無い状態で `swing up` を起動する」を書き込み先が無いという理由でエラーにせずに済み、セットアップ画面がそこへ新規作成できる。`Config.config_exists: bool` と `Config.sources: BTreeMap<String, Source>`（`Source::{Env, File, Default}`、キーは `"<section>.<field>"`）を新設し、`Config::source_of` で引けるようにした。
- 鍵が無い状態の `swing up` は Kubo も agent も起動せず、ダッシュボードだけを動かす「セットアップモード」にした（`AppState::setup_mode()`、`own_pubkey: Option<PublicKey>`）。relay・Kubo を使う API は `agent is not configured`（503）を返す。`GET /api/overview` の `setup: true` を見て、フロントはどの URL でも Setup 画面に固定する。
- 設定の書き込みは `toml_edit::DocumentMut` を使い、コメントや無関係なキーを保持したまま該当キーだけを書き換える。書く前に必ず `config::build_config_from_str` で組み立て直して妥当性を確認し、失敗したらファイルには一切触れない。書き込みは tmp ファイル + `rename` の atomic write。既存ファイルは元の権限を維持し、新規作成は unix で `0600`。
- 保存直後にプロセスが今の設定のままでも「保存したらこうなる」を見せられるように、`AppState.display_config`（保存のたびに差し替える表示専用コピー）と `AppState.restart_required`（プロセス起動後 1 度でも保存があれば立ったままの `AtomicBool`）を新設した。実際に動いている relay・Kubo・agent の設定は実際に再起動するまで変わらない。
- `POST /api/setup` が鍵を書いた後の「再起動」は、プロセスを終了させてサービスマネージャの再起動ポリシーに委ねる（従来の exit code 3）のではなく、**同じプロセス内で `main.rs` のループが設定を読み直して `up::run` をもう一度呼ぶ**方式にした。`ExitRequest::restart()` → 最上位トークンを cancel → `up::run` が `Exit::Restart` を返す → `main.rs` の `Command::Up` ループが `continue` して `Config::load` からやり直す。`std::process::exit(3)` の経路は削除した。理由: OS のサービスマネージャごとに exit code の解釈が割れていて確実ではないこと、Docker Compose では `restart: unless-stopped` のせいで一瞬コンテナが再作成されうること、何より「保存→再起動」を毎回サービス層に横流しにする必要が無いこと。副作用として `swing stop --restart`／`POST /api/restart` も同じ経路になり、`docs/architecture/up.md`・`service.md` の「各サービスマネージャの反応」の説明を書き換えた。
- 後方互換のための処置（旧形式の `state.json` や `swing.toml` を読むための `#[serde(default)]`・フォールバック・移行コードなど）は今回入れていない。`secret_key` を `Option` にしたのは後方互換の処置ではなく、鍵が無い状態そのものを正規の状態として扱うための変更。

## やったこと

- `src/config.rs`: `resolve_config_path` を「常にパスを返す」形に変更（存在確認は呼び出し側の責務に）。`NostrConfig.secret_key: Option<NostrSecretKey>`。`Config.config_path: PathBuf`・`config_exists: bool`・`sources: BTreeMap<String, Source>`・`source_of`。`Config::require_secret_key`（従来のエラーメッセージのまま）を追加し、鍵を直接使う経路（publish・mirror list/sites・replicas・webring・agent）はこれを呼ぶように変更。`build_config_from_str`（バリデーション用に文字列から直接組み立てる）を追加。
- `src/settings.rs`（新規）: `EDITABLE_KEYS`・`Kind`・`find`・`is_editable`・`raw_value`（現在値を編集フォームに出せる形にする）・`update`（`PUT /api/config`）・`setup`（`POST /api/setup`）。`toml_edit` に依存を追加（`Cargo.toml`）。
- `src/dashboard/dto.rs`: `OverviewDto.setup`・`pubkey`/`npub` を `Option` に。`ConfigItemDto` に `source`/`editable`/`kind`/`raw`/`options` を追加。`ConfigDto` に `config_exists`/`writable`/`restart_required` を追加し、`is_config_writable`（既存ファイルは `OpenOptions::append` で開けるか、無ければ親ディレクトリが書けるかを見る、副作用のない判定）を実装。
- `src/dashboard/api.rs`: `ApiError::NotConfigured`（503 `agent is not configured`）と `not_ready()`（`setup_mode()` で `NotReady`/`NotConfigured` を切り替える）を追加。`PUT /api/config`（`update_config`）と `POST /api/setup` を追加。
- `src/dashboard/mod.rs`: `AppState::new` が `keys: Option<Keys>` を取るように変更。`display_config`・`restart_required`・`setup_mode()` を追加。ルータに `PUT /api/config`・`POST /api/setup`・`GET /setup.js` を追加。
- `src/up.rs`: `config.require_secret_key()` の成否で `keys: Option<Keys>` を決め、`None` ならダッシュボードだけ動かして `token.cancelled()` を待つ（Kubo・agent の起動ループには入らない）。
- `src/main.rs`: `Command::Up` を `loop` にし、`Exit::Restart` を `main` 側で消費して `Config::load` からやり直す（同一プロセス）。`std::process::exit(3)` の経路を削除。
- `Dockerfile`: `WORKDIR /data` を追加（`swing.toml` の既定書き込み先が volume の中になるように）。
- `web/settings.js`: セクションごとに編集可能な項目を入力欄にし、Save ボタン（1 セクション分の差分だけ送る）・env ロックの注記・`writable === false` のときの読み取り専用表示・`restart_required` の通知を追加。
- `web/setup.js`（新規）・`web/index.html`（`#view-setup` 追加）: 鍵の生成／貼り付け、relays、保存上限 3 つを入力するセットアップフォーム。送信後は `GET /api/overview` を 1 秒間隔でポーリングして `setup: false` を待つ。
- `web/app.js`: 最初のルーティング前に `loadOverview()` を待つようにし（初期表示のちらつき防止）、`overview.setup` の間はハッシュに関わらず `#/setup` に固定する `currentRoute()` を追加。
- `web/i18n.js`・`web/style.css`: Settings/Setup 用の文言（英語・日本語）と、警告色の `.swing-status[data-kind="warn"]`・設定編集フォームのスタイルを追加。

## 今回の作業で直した細部

- `dto::ConfigDto.writable` が `true` 固定になっていたのを実装した（上記 `is_config_writable`）。ユニットテストを 4 本追加（既存ファイルが書ける／書けない、親ディレクトリが無い、新規ファイル先の親が書けるケース。root 実行では読み取り専用ビットが効かないため、その場合はアサーションをスキップする）。
- `web/settings.js` の Save ボタンが `.swing-btn-small` で他の主要操作（Display パネルの Apply）と見た目が揃っていなかったのを `.swing-btn-accent` に統一した。
- `web/setup.js`／`web/index.html` の relays テキストエリアが `rows="4"` で既定の relay リスト（5 件）すら収まらなかったのを `rows="6"`（既定件数 + 1）に広げた。
- UI のヒント・プレースホルダーに `config::parse_size` が受け付けない単位（GiB/MiB など）を使っている箇所が無いことを確認した（該当なし、変更不要）。
- セットアップフォームの relays・保存上限 3 項目が、compose の `.env` などで env 由来になっている場合に、`PUT`/`POST /api/setup` の env 由来チェックで丸ごと 400 になってしまう問題を修正した。`GET /api/config` の `editable` を見て、env 由来の項目はフォーム側でも disabled にして現在値を表示し（Settings 画面と同じ「Locked: set by environment」の注記）、送信する `items` にも含めないようにした（`web/setup.js`）。

## 検証

- `cargo fmt` / `cargo clippy -j 3 --all-targets -- -D warnings` / `cargo test -j 3`（378 passed、15 ignored）を通した。
- `node --check` を変更した全 JS ファイル（`web/setup.js`・`web/settings.js`・`web/app.js`・`web/i18n.js`）に通した。
- ローカル（Docker を使わない直接実行、`/home/miki/lfs/cargo-target/release/swing`）:
  - `swing.toml` も鍵も無い状態で `swing up` → ログに `no Nostr secret key configured; running in setup mode` → `GET /api/overview` が `{"setup":true,"pubkey":null,"npub":null,...}` → `GET /api/sites` が 503 を返すことを確認。
  - 鍵未設定のまま `swing stop --config <path>` → `stopped` で exit code 0（プロセス終了を確認）。
  - 再度セットアップモードで起動し、`POST /api/setup`（`secret_key: null` + relays/保存上限）→ `{"ok":true,"npub":"npub1...","restart":true}` → `swing.toml` が新規作成され権限 `0600` → ログに `restarting: reloading configuration` の後 `dashboard listening` が同じプロセス（**同じ PID**、新しいプロセスは生成されない）で出る → 直後は Kubo バイナリが無い実行環境のため（この環境固有の制約で、機能側の不具合ではない）以降のエラーで終了したが、in-process restart 自体（設定の読み直し・同一プロセスでの `up::run` 再実行）は確認できた。
  - 続けて `[kubo] managed = false` と到達不能な `[ipfs] api` を手で足した状態で 2 回目の `swing up` を実行し、`GET /api/overview` が `{"setup":false,"pubkey":"...","npub":"npub1..."}` になる（＝セットアップモードではなく通常モードで待機している）ことを確認した。
  - `PUT /api/config` で `policy.keep_versions`（許可キー）を変更 → `restart_required: true`。`kubo.binary`（許可外）を送ると 400 `unknown or non-editable key`。`publish.keep_versions: "0"` を送ると 400 `edited configuration is invalid: publish keep_versions must be greater than 0`。`SWING_MAX_TOTAL_STORAGE` を設定したプロセスで `policy.max_total_storage` を書こうとすると 400 `... is set via an environment variable and cannot be edited here`。
- デモ環境（`docker/demo/demo.sh up`、外部ネットワーク非使用。ダッシュボード `http://127.0.0.1:18082/`）:
  - ブラウザでの確認（agent-browser）: Settings 画面のライト/ダーク・英語/日本語の 4 パターンで、env 由来の行（`secret_key`・`relays`・`mirror_set`・`ipfs.api`・`max_total_storage`・`state_dir`・`dashboard.listen`・`kubo.managed`・`kubo.repo`・`gateway.*` など、compose の `.env`/固定環境変数で設定される項目）が正しくロック表示（「Locked: set by environment」/「ロック中: 環境変数で設定されています」）になっていること、`policy.keep_versions` を編集して Save すると「Saved. Restart the agent to apply.」と上部に再起動要求バナーが出ること、`publish.keep_versions` に `0` を送ると 400 のエラーメッセージがその場に表示されファイルは変わらないこと、Save ボタンが Apply ボタンと同じ見た目（accent）になっていること、Setup 画面の Relays テキストエリアが既定 5 件を折り返しなしで見渡せる高さになっていることを確認した。
  - API での確認（curl）: `PUT /api/config` で `policy.keep_versions` を `9` に変更（`restart_required: true`）→ `POST /api/restart` → `docker ps` で **同じコンテナ**（再作成されていない）が起動したままであることを確認 → コンテナログに `restarting: reloading configuration` の後 `dashboard listening` が同じログストリームで出る → `GET /api/config` が `keep_versions: 9`、`source: "file"`、`restart_required: false` を返すことを確認（保存した値が再起動後に永続していること、かつ `restart_required` が実際の再起動でリセットされることの両方）。
  - 最後に `docker/demo/demo.sh down` でコンテナ・volume・network を削除した。`docker/demo/demo.env`・`docker/demo/.seeded` は既存の `.gitignore` 対象なのでリポジトリには残らない。

## 見つけた別件（このログの範囲外）

- `docs/architecture/docker.md` に「`SWING_DASHBOARD_LISTEN=off` でコンテナでもダッシュボードを無効化できる」という記述があったが、`config::parse_dashboard_listen` は `SocketAddr` としてしかパースせず `off` を受け付けない（実装のバグではなく、ドキュメントの誤り）。ドキュメント側を実態に合わせて修正した（コードは変更していない）。
- ダッシュボードの認証トークン（`docs/todo.md` の既存項目）は、設定が書けるようになったことで重要度が上がったため、優先度を中→高に上げ、理由を追記した。

## 追記: サイドナビの Setup 表示

上記の回でセットアップ画面（`#/setup`）自体は作ったが、サイドナビには対応する項目を出していなかった（`overview.setup === true` でも Desktop・Sites・Webring・Publish・Settings の 5 項目がそのまま表示され、クリックしても `currentRoute()` が強制的に Setup へ戻すだけで見た目上は何も起きない）。これを直した。

- `web/index.html`: Settings のナビ項目をコピーして `#/setup` へのリンクを追加（アイコンは Settings と同じ `#icon-settings`、既定で `hidden`）。
- `web/i18n.js`: `navSetup`（英語 `Setup` / 日本語 `セットアップ`）を追加。
- `web/app.js`: `showRoute()` で `cache.overview.setup` を見て、`true` の間は Setup 項目だけを表示（他 5 項目を `hidden`）、`false` なら逆に Setup 項目を隠すようにした。アクティブ表示（`aria-current="page"`）は既存のロジックがそのまま `data-route="setup"` にも効くため変更不要だった。
- `docs/architecture/dashboard/web.md` を実装に合わせて更新（サイドナビの並び順と Setup 項目の表示条件）。

検証: デモ環境（`docker/demo/demo.sh build mirror` → `mirror` コンテナだけ再作成）で確認しようとしたところ、`swing-demo_swing-data` ボリュームに以前のセッションで作られた `swing.toml`（鍵入り）が既に残っており、`GET /api/overview` は `setup: false` だった。ボリューム内のファイル削除やコンテナへの `exec` は破壊的操作として許可システムに拒否されたため、`.env`・ボリュームの状態には一切手を加えていない（`docker/demo/demo.env` に鍵は書き込まれていない）。そのため実際のセットアップモードでの確認はできず、代わりに agent-browser で実ページを開き、`import('/util.js')` で `app.js` と共有される同一の `cache` オブジェクトへ `setup: true` を直接セットしてから `location.hash = '#/setup'` を発火させ、実物の `showRoute()`/`currentRoute()` を通して検証した。結果、サイドナビは Setup 項目のみが表示され `aria-current="page"` が付き、他 5 項目は非表示、サイドナビ下部のフッター（mirror set / version）は変化しないことを確認した（実際の `overview.setup === false` の状態でも 5 項目 + Setup 非表示が正しく表示されることは、素の状態のデモで別途確認済み）。デモは検証後も `setup: false` のまま変更せず稼働させている。

## 追記: セットアップ完了後の遷移先とデモでの実確認

- セットアップ完了（`POST /api/setup` 後のポーリングで `setup: false` を確認）したときの遷移先を `#/sites` から `#/settings` に変えた。書き込んだ設定をそのまま確認できるようにするため。既に設定済みの状態で `#/setup` を開いたときの退避先は `#/sites` のまま。
- 上の追記で保留になっていた「実際のセットアップモードでのサイドナビ確認」は、デモボリュームの `swing.toml` を削除して `mirror` を再起動し（`GET /api/overview` が `setup: true`）、agent-browser で開いて `#/setup` に直行しナビが Setup 1 項目になることを確認した。

## 追記: env ロック注記の対象

- Settings 画面の `Locked: set by environment` 注記は、当初 env 由来の全項目に出していたが、もともと画面から変えられないホワイトリスト外の項目（`state_dir`・`dashboard.listen`・`kubo.managed` など）に「ロック」と出るのは誤解を招くため、ホワイトリスト入り（`kind` あり）かつ env 由来の項目だけに絞った。
