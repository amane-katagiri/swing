# Windows のトレイ終了条件と service stop / uninstall の設定解決

Windows まわりの 2 件を直した。

1. `schtasks` が一時的に失敗しただけで、`swing-tray` が「サービス登録が消えた」と判断して閉じてしまう。
2. `swing service stop` が、設定ファイルが見つからないと `schtasks /End` にも進まずにエラー終了する。`uninstall` は同じ失敗を黙って無視していた。

## 登録状態を三値にする

`service::is_installed` の戻り値を `bool` から `Option<bool>` にした（`Some(true)` 登録済み、`Some(false)` 未登録、`None` 分からない）。新しい enum は作らず、`Option` で「分からない」を表した。値の意味が単純で、使う側（トレイ）も `== Some(false)` のような比較で済むため。

- Linux / macOS はファイルの有無で決まるので、常に `Some` を返す。
- Windows は `schtasks` の起動失敗・時間切れ・判定できない失敗を `None` にする。
- トレイの `worker.rs` は `spawn_blocking` の join 失敗も `None` にし、`None` も 10 秒キャッシュする（`schtasks` が固まっているときに毎回 10 秒待たないため）。
- `RegistrationWatch::removed` は `None` を無視して前の状態を保つ。一度 `Some(true)` を見た後に `Some(false)` が来たときだけ閉じる。
- メニューは、未登録と確認できたときだけ「停止中（サービス未登録）」と表示して「起動」を無効にする。分からないときは「停止中」で「起動」も押せる（登録されていなければ `schtasks /Run` の失敗が表示される）。起動時の自動起動は `Some(true)` のときだけにした。分からない状態で勝手に起動しようとして失敗を出すより、何もしないほうが自然なため。

### Windows で「タスクが無い」をどう判定するか

`schtasks /Query /TN swing` の終了コードでは判定できない。`schtasks` はタスクが無いときも、アクセス拒否や構文エラーなど他のどのエラーでも終了コード 1 を返す。エラーメッセージ（`ERROR: The system cannot find the file specified.` など）はロケールで変わり、日本語環境では OEM コードページの別の文になるので、文字列一致にも頼れない。

タスクスケジューラの COM API（`ITaskFolder::GetTask` が `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)` を返す）ならロケールに依存せず判定できるが、`windows-sys` には COM インターフェースの vtable 定義が無く、`windows` クレートを足すか vtable を手書きすることになる。依存と unsafe のコードが増える割に、下の方法で十分堅いので見送った。

採った方法:

1. `schtasks /Query /TN swing` が成功したら登録済み。
2. 失敗したら `schtasks /Query /FO CSV /NH` で全タスクを列挙する。この一覧取得が成功し、各行の先頭列（タスク名。`"\swing"` のようにフォルダ付きで出る）に `"\swing"` が無ければ未登録。あれば登録済み。
3. 一覧取得も失敗したら分からない。

タスク名はデータなのでロケールで変わらず、CSV の形も `/FO CSV` で固定される。一覧はそのユーザーが読めるタスクだけだが、`swing` のタスクは同じユーザーが `service install` で登録したものなので必ず含まれる。ふだん（登録済み）は 1 回目の `/Query /TN` で済み、全列挙は失敗したときだけ走る。先頭列の照合は大文字小文字を区別しない（タスク名は大文字小文字を区別しないため）。サブフォルダの `\Folder\swing` は一致させない。照合は純粋関数 `task_listed` にしてユニットテストした。

`swing service status`（Windows）も同じ判定を使うようにした。未登録なら従来どおり `not installed`、分からなければエラー終了する。以前は `/Query /V` のどんな失敗も `not installed` と表示していた。

### タイムアウト

`schtasks` の呼び出し（上の 2 回と `status` の `/Query /V`）に 10 秒のタイムアウトを付けた（`output_with_timeout`）。子プロセスを起動し、標準出力と標準エラーを別スレッドで読み切りながら `try_wait` で待ち、期限を過ぎたら kill してエラーを返す。パイプを読みながら待つのは、全タスクの一覧が大きくパイプのバッファを埋めて子が止まるのを避けるため。外部クレート（`wait-timeout` など）は足さなかった。`/Create`・`/Run`・`/End`・`/Delete` は CLI から人が実行するもので、ポーリングを止める心配が無いので従来のままにした。

トレイの状態確認は最悪 20 秒（2 回の `schtasks` がどちらも時間切れ）止まるが、キャッシュにより次の 10 秒は呼ばないので、ポーリングが止まり続けることはない。

## service stop / uninstall の設定解決

`windows::stop` は `install` 用の `resolve_service_paths` を流用していたため、設定ファイルが無いと `service install needs a config file (swing.toml): pass --config or set SWING_CONFIG` という（`stop` には `--config` が無いのに `--config` を勧める）メッセージで終わり、`schtasks /End` に進まなかった。実行ファイルのパスの `canonicalize()` も不要な失敗要因だった。

stop / uninstall 用に `load_stop_config` を分け、`resolve_config_path(None)` で設定ファイルだけを探すようにした。

- 設定ファイルが無い: `` Warning: could not find the config file (swing.toml) at <path> to stop swing through its dashboard; set SWING_CONFIG or run this from the directory containing swing.toml. Falling back to `schtasks /End`. ``
- 読めない（`Config::load` の失敗）: `` Warning: could not read the config file <path> to stop swing through its dashboard (<error>). Falling back to `schtasks /End`. ``
- どちらも `schtasks /End /TN swing` にフォールバックする。`stop::run` の失敗時の警告とフォールバックは従来どおり。

`uninstall` も同じ関数（`stop_gracefully`）を通すので、同じ警告が出る。以前は `stop::run` の失敗も黙って無視していたが、これも `stop` と同じ `Warning: graceful stop failed …` を出すようにした。その後の `/End`・`/Delete` の流れは変えていない。探すパスを警告に含めたのは、`SWING_CONFIG` が誤ったパスを指しているときにも原因が分かるようにするため。

## 検証

- `cargo fmt --all --check`
- `cargo clippy -j 2 --workspace --all-targets -- -D warnings`
- `cargo test -j 2 --workspace`（追加: `status::tests::an_unknown_registration_keeps_the_tray_open`、`registration_counts_as_removed_only_after_it_was_seen` の拡張、未登録と分からないときのメニュー、`service::tests::task_listing_matches_only_the_root_task_by_exact_name`、unix 上で `output_with_timeout` の出力回収と時間切れ）
- `cargo xwin clippy -j 2 --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings`

Windows 実機では未検証。`schtasks /Query /FO CSV /NH` の出力形式（先頭列が `"\<name>"`）と、日本語・英語環境での判定、設定ファイルが無いディレクトリからの `service stop` の動作は実機で確かめる（[`todo.md`](../todo.md)）。
