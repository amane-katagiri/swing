# 2026-09-23 グレースフルな停止（RPC shutdown・ダッシュボード API・swing stop）

`docs/todo.md` にはまだ書いていなかったが、[配布方式の実装](2026-09-23-distribution-implementation.md)・[多重起動防止と孤児 Kubo 回収](2026-09-23-instance-lock-and-orphan-kubo.md)の続きで、「swing を安全に止める・再起動する手段」を Windows でも動く形で用意した回。architecture 側の結果は [`up.md`](../architecture/up.md)・[`service.md`](../architecture/service.md)・[`cli.md`](../architecture/cli.md)・[`dashboard/http-api.md`](../architecture/dashboard/http-api.md)・[`dashboard/web.md`](../architecture/dashboard/web.md)・[`../architecture.md`](../architecture.md) に反映済み（このログには経緯だけを書く）。

## きっかけ

Windows はタスクスケジューラ登録（UAC を避けるための選択、[配布方式の設計](2026-09-21-distribution-design.md)）で常駐させている。タスクスケジューラのプロセスには SCM サービスの `SERVICE_CONTROL_STOP`/`SERVICE_CONTROL_SHUTDOWN` のような「行儀よく止めてくれ」という通知を送る標準的な仕組みが無く、`taskkill`（既定は `TerminateProcess`）で終わらせるしかない。Kubo は Job Object の `KILL_ON_JOB_CLOSE` で道連れに落とせるので孤児化はしない（[多重起動防止と孤児 Kubo 回収](2026-09-23-instance-lock-and-orphan-kubo.md)）が、Kubo 自身が RPC シャットダウン（`ipfs shutdown`）を経ずに `TerminateProcess` されるのは避けたかった。unix には SIGTERM があるが、Windows の `swing` プロセスには「グレースフルに止めて」を伝える着信経路そのものが無い。

## 決めたこと

- Kubo 側はまず RPC（`POST /api/v0/shutdown`、`ipfs shutdown` と同じ）を叩き、それでも `grace` 秒以内に終わらなければ unix は SIGTERM → 10 秒 → SIGKILL、Windows はそのまま `kill()` という段階構成にした（[`up.md`](../architecture/up.md#停止daemonstopgrace)）。RPC はレスポンスの成否を見ずに「要求した」ものとして進む: Kubo は終了処理を始めてから接続を切ることがあり、そこでリトライしても意味が無いため。
- swing プロセス自身への「グレースフルに止まれ」という着信経路として、OS 固有の仕組み（Windows の名前付きイベントなど）を新しく作るのではなく、**既にある HTTP のダッシュボード API をクロスプラットフォームの制御チャンネルとして使う**ことにした。`POST /api/shutdown`／`POST /api/restart`（[`dashboard/http-api.md`](../architecture/dashboard/http-api.md)）を追加し、`swing stop` CLI（`stop.rs`）はこれを叩くだけの薄いクライアントにした。ダッシュボードが無効（`[dashboard].listen = off`）なときだけ、unix は `swing.lock` から読んだ PID に SIGTERM、Windows はエラーにする（グレースフルな停止手段が無い、という制約をそのまま受け入れた）。
  - この設計は、CLI を「agent 用の別の relay クライアント」から「動いている agent の API を叩くクライアント」へ寄せていく方向の最初の一歩でもある、と意識して選んだ。今の `sites`/`replicas`/`status`/`webring`/`mirror` の各サブコマンドは agent とは独立に自分で relay に繋いで `state.json` を読んでおり、agent が動いていても二重に relay を引く。`swing stop` を（OS のシグナルではなく）ダッシュボード API 経由にしたのは、他のサブコマンドも同じ方向に揃えられるかを試す布石で、`docs/todo.md` に一段深い検討課題として書いた。トレードオフは、`[dashboard].listen = off` にしている Windows 環境ではこの経路そのものが使えず、グレースフルな停止手段が無くなること（`--config` はあっても、宛先が無い）。
- 「止める」と「再起動する」を区別する必要があるので、`swing up`／`swing agent` の終了を表す `shutdown::Exit`（`Stop` | `Restart`）と、それを立てる `shutdown::ExitRequest`（`stop()`/`restart()`、内部は同じ `CancellationToken` の cancel + 再起動用の `AtomicBool`）を導入した。`agent::run_until` は自分のトークンから作った `ExitRequest` をダッシュボードの `AppState` に渡し、`POST /api/restart` はこれの `restart()` を呼ぶだけ（`stop()` との違いはフラグを立ててから cancel することだけ）。プロセスの exit code は `Exit::Stop` → 0、`Exit::Restart` → 3 とし、`main.rs` がランタイムを畳んでから `std::process::exit` する。exit code で分けたのは、OS のサービスマネージャごとに「動いていたプロセスが自分から終了コード 0 で終わった」ことの解釈が割れているため（systemd の `Restart=on-failure` は 0 を失敗と見なさず再起動しない一方、launchd の `KeepAlive=true` はコードに関わらず再起動する。詳細は [`up.md`](../architecture/up.md#各サービスマネージャの反応)）。3 を「失敗」として使うことで、`Restart=on-failure`／Windows の `RestartOnFailure` の両方を「再起動して」の合図として使い回せる。
- Windows の OS シャットダウン／ログオフでの強制終了（タスクスケジューラにその通知経路が無いこと自体）は今回のグレースフルストップの対象外として受け入れた。理由は `state.json` が tmp + rename の原子的な書き込みで、途中終了で壊れたファイルが残ることがないため（[`state.rs`](../../src/state.rs)）、また Kubo のデータストアも中断からの復旧を前提にしているため。実際に SCM サービスとして登録すれば `SERVICE_CONTROL_SHUTDOWN` を受け取れるが、UAC を避けるという既存の設計判断と両立しないため別 todo にした。

## やったこと

- `src/kubo.rs`: `Daemon::spawn` に `api_url: String` を追加（`Daemon::stop` が使う）。`Daemon::stop`: RPC shutdown → `grace` 待ち → （unix のみ）SIGTERM → 10 秒待ち → `kill()`。各段の成否を `tracing::debug!` に出す。
- `src/shutdown.rs`: `Exit`（`Stop`/`Restart`）と `ExitRequest`（`new`/`stop`/`restart`/`restart_requested`/`exit`）を追加。`cancel_on_signal` は変更なし。
- `src/agent/lifecycle.rs`: `run`/`run_until` が `Result<Exit>` を返すよう変更。`run_until` の冒頭で自分のトークンから `ExitRequest` を作り、ダッシュボード有効時は `dashboard::AppState` にクローンを渡す。ループを抜けるすべての箇所で `exit.exit()` を返す。
- `src/dashboard/mod.rs`・`src/dashboard/api.rs`: `AppState` に `exit: Option<ExitRequest>` を追加（`AppState::new` の新引数）。`POST /api/shutdown`／`POST /api/restart` ハンドラを追加（202 を返してから `exit.stop()`/`exit.restart()` を呼ぶ。他の書き込み系と同じガードを通る）。
- `src/up.rs`: `run`/`run_managed`/`run_unmanaged` が `Result<Exit>` を返すよう変更。`Daemon::spawn` の呼び出しに `api_url` を渡す。agent が返す `Exit` をそのまま `up::run` の戻り値として持ち上げる。
- `src/stop.rs`（新規）: `stop::run(config, restart, timeout)`。まずロックが取れるか（＝動いていないか）を確認、動いていればダッシュボード API を叩き、無ければ（unix のみ）`swing.lock` の PID に SIGTERM、ロックが取れるようになるまでポーリング。
- `src/service.rs`: `stop`（新規、Linux は `systemctl stop`、macOS は `launchctl kill SIGTERM`、Windows は `stop::run` → 失敗時 `schtasks /End` にフォールバック）。`uninstall`（Windows のみ）は削除前に `stop::run` を試みるよう変更。`uninstall`/`stop` は非同期処理（Windows 経路が `stop::run` を awaitする）を要するため `async fn` にした（他の関数は同期のまま。ネストした `tokio::runtime::Runtime` は既存のランタイム内から呼ぶとパニックするため避けた）。
- `src/main.rs`: `swing stop` サブコマンド、`swing service stop` サブコマンドを追加。`run()` は `Agent`/`Up` だけ直接 `Result<Exit>` を返し、それ以外は新設の `run_other()`（`Result<()>` のまま）に委譲して `Exit::Stop` にくるむ。`main()` は `Exit::Restart` でランタイム shutdown 後に `std::process::exit(3)`。
- `web/index.html`・`web/settings.js`・`web/i18n.js`・`web/style.css`: Settings 画面に「プロセス」パネル（停止・再起動ボタン、`confirm()` → `POST /api/shutdown`/`restart` → 状態行）を追加。Desktop 画面は変更していない。

## 検証

- `cargo fmt` / `cargo clippy -j 3 --all-targets -- -D warnings` / `cargo test -j 3`（350 passed、15 ignored）を通した。
- `SWING_TEST_KUBO_BIN` で実 Kubo 0.43.1 に対する `#[ignore]` テスト 2 本（`full_lifecycle_against_real_kubo`・`recover_orphan_kills_a_leftover_daemon`）を通した。
- `cargo xwin clippy -j 3 --target x86_64-pc-windows-msvc --all-targets -- -D warnings` が警告 0 で通ることを確認した（実行・リンクの確認ではなく、型として正しいことの確認。[多重起動防止と孤児 Kubo 回収](2026-09-23-instance-lock-and-orphan-kubo.md) と同様の制約）。
- スモークテスト（実 Kubo 0.43.1 + ローカル nostr-rs-relay、公開ネットワーク不使用。ポート 19080-19083）:
  - (a) `swing up` → `swing stop --config ...`: ログに `kubo RPC shutdown requested` → Kubo 側 `Received interrupt signal, shutting down...` → `kubo exited after RPC shutdown`（SIGTERM/kill には進んでいない）、`swing stop` は `stopped` と出して exit code 0、`ps` に `ipfs daemon` の残留なし。
  - (b) `swing up` → `swing stop --restart`: `swing up` プロセス自身の exit code が 3 であることを確認（バックグラウンドプロセスの `$?` をファイルに書かせて確認）。
  - (c) `curl -X POST -H 'X-Swing-Dashboard: 1' http://127.0.0.1:19083/api/shutdown`: `202` と `{"action":"stop","ok":true}`、`swing up` は exit code 0 で終了。
  - (d) `[dashboard].listen = off` にして `swing stop`: ログに `shutdown requested signal="SIGTERM"`（`shutdown::cancel_on_signal` の通常の SIGTERM 経路に入った）が出て、Kubo は変わらず RPC shutdown 経由で終了、exit code 0。
  - 最後に relay コンテナ（`swing-smoke-relay`）を削除し、作業ディレクトリ（scratchpad 配下）を削除した。リポジトリ直下には何も残していない。

## 見送ったこと

- Windows の SCM サービス登録（`windows-service` クレート、UAC が 1 回要る）は見送った。OS シャットダウン時の通知を受け取れるようになるが、既存の「UAC を避けてタスクスケジューラで常駐させる」という配布方式の設計判断と衝突するため、別 todo として残した（[`../todo.md`](../todo.md)）。
- `swing stop`/ダッシュボード API を経由しない、OS ネイティブな停止シグナル（Windows の名前付きイベントや Job Object 経由の通知など）は見送った。HTTP の方が実装・テストが軽く、`[dashboard].listen` が既にある設定の再利用で済むため。
- CLI の読み取り系サブコマンドをダッシュボード API のクライアントに寄せる話全体は、`swing stop` 1 本だけで検証しきれる範囲を超えるため、今回は手を付けず `docs/todo.md` に方向性だけ書いた。
- `swing stop` のポーリング中に「再起動後の新しいプロセスが古いロックを再取得した」ケースの判別は見送った。ロックが一度でも空くのを見た時点で `stopped` として成功にする（[`stop.rs`](../../src/stop.rs)）。サービスマネージャの再起動間隔（systemd は 5 秒、launchd はほぼ即座、タスクスケジューラは 1 分）次第では極短時間の空き窓を見逃してタイムアウトする可能性が理論上はあるが、実害は小さいと判断した。

## 追記: launchd の KeepAlive

当初は `KeepAlive = true` にしていて、macOS だけ「`swing stop` しても即座に戻ってくる」非対称を受け入れていた。exit code を 0（停止）/ 3（再起動）に決めた以上、launchd もそれに従うべきなので `KeepAlive = { SuccessfulExit = false }` に変えた。これで `swing stop` は次のログインまで止まったまま、`--restart` とクラッシュは戻ってくる、と systemd・タスクスケジューラと同じ意味になる。再開は `launchctl kickstart -k gui/<uid>/jp.ne.ama.swing`、ログインをまたいで止め続けるなら `launchctl disable` → `bootout`。`service.rs` の `launchd_plist` とそのテスト、`service stop` の案内文、`service.md`・`up.md` の表を更新した。macOS 実機での確認は引き続き未実施。
