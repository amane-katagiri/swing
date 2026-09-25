# ダッシュボードの設定保存の不具合 2 件

ダッシュボードから設定を保存する経路（`PUT /api/config`・`POST /api/setup`）に 2 つの不具合があったので直した。

## 2 回目の保存で 1 回目の内容が消える

`settings::update`／`settings::setup` は `AppState.config`（起動時に作られて以後変わらない）を受け取り、`load_document` はその `config_exists` が `false` ならファイルを読まずに空文書から組み立てていた。起動時に `swing.toml` が無いと、1 回目の保存でファイルができても `config_exists` は `false` のままなので、2 回目の保存は 1 回目の項目を含まない文書で上書きしていた。`PUT /api/config` の後の `POST /api/setup` も同じ。

- `load_document` は書き込み先パスを受け取り、毎回 `read_to_string` する。`NotFound` のときだけ空文書から始める。`exists()` で確かめてから読むのではなく、読んだ結果で分けた（確認と読み込みの間にファイルが変わる隙間を作らないため）。`config_exists` は `GET /api/config` の表示（`config_dto`）と書き込み可否の判定には引き続き使う。
- 設定ファイルを書く API（`PUT /api/config`・`POST /api/setup`・`POST /api/signer/reconnect`）を `AppState.config_writes: tokio::sync::Mutex<ConfigWrites>` で直列化した。読んでから書くまでの間に別の保存が挟まると後勝ちで項目が消えるうえ、`auth::write_private_file` の一時ファイル名が `<path>.tmp` 固定なので、並行した書き込みが互いの一時ファイルを消したり `rename` を失敗させたりしうる。同じプロセス内の書き手はこの 3 つだけなので Mutex で足り、一時ファイル名は変えていない。プロセス外からの同時書き込みは対象外。`reconnect` は `swing.toml` ではなく `remote-signer.json` を書くが、`POST /api/setup` の `remote_signer: true` も同じファイルを書くので同じ Mutex に入れた。
- `POST /api/setup` は再起動を約 300ms 後にスケジュールするので、その間にもう 1 回通ると 2 回目が新しい鍵で上書きし、1 回目の応答で返した `npub` が無効になっていた。`ConfigWrites.setup_done` を Mutex の中で見て、立っていれば 409 `setup is already done; swing is restarting` を返す。立てるのは設定ファイル（と `remote-signer.json`）の保存がすべて成功した後なので、途中で失敗したときは再試行できる。再起動で `AppState` ごと作り直されるので戻す処理は要らない。

## 設定ファイルの IO エラーが 400 になる

`update_config`・`setup` は `settings::update`／`setup` の `anyhow::Error` をすべて 400 にしていた。ディスクの書き込み失敗などは要求側の問題ではないので 500 にしてログに残すべき。

- `settings::EditError { Invalid, Io }` を足し、`update`／`setup` の戻り値をこの型にした。anyhow のチェーンから `std::io::Error` を downcast する方法も考えたが、検証の途中で IO が起きうる経路（`build_config_from_str` の中身は別の場所で変わりうる）と IO でないが 500 に寄せたい失敗（下記の読み直し）の両方を、呼び出し箇所ごとに明示して分けられる型の方を選んだ。`publish::SiteFieldError`・`api_client::ApiClientError` と同じ、`anyhow::Error` を包む enum の形。
- `Invalid`（400）: 編集できない・env 由来のキー、値の形式違い、`build_config_from_str` による検証失敗、既存ファイルの TOML 解析失敗、鍵がある状態での `setup`。既存ファイルの解析失敗は要求の中身のせいではないが、同じく既存ファイルの中身が原因になりうる組み立て後の検証失敗と揃え、「ファイルを直してほしい」というメッセージが 400 で返る形にした。
- `Io`（500）: 設定ファイルの読み込み（`NotFound` 以外）、`create_private_dir_all`、`write_private_file`（一時ファイルの作成・書き込み・`rename`）、書き込み後の `Config::load`。`Config::load` は直前に同じ文字列を同じ環境変数（`config::env_var`）で `build_config_from_str` にかけて通っているので、ここで失敗するのはファイルの読み戻しの失敗か、プロセス外から書き込み直後に差し替えられた場合しかない。どちらも要求側では直せないので 500 に入れた。
- `api::settings_error` が `EditError` を `ApiError` に変換し、`Io` のときは `api::internal` が `error!` でログに出す。`remote-signer.json` の保存失敗（もとから 500）も同じ `internal` を通してログに出るようにした。`setup_keys`（鍵の文字列が読めない）は 400 のまま。

## テスト

- `put_config_twice_without_a_config_file_keeps_both_items`: ファイルが無い状態から `PUT` を 2 回して両方の項目が残る。
- `setup_after_put_config_keeps_the_put_item`: `PUT` の後の `POST /api/setup` で `PUT` の項目が残る。
- `a_second_setup_before_the_restart_is_conflict_and_keeps_the_first_key`: 2 回目の `setup` が 409 で、ファイルの鍵が 1 回目の `npub` と一致する。
- `an_invalid_config_value_is_bad_request`: 検証エラーは 400 でファイルを作らない。
- `an_unwritable_config_directory_is_internal_error`・`setup_into_an_unwritable_config_directory_is_internal_error`（unix）: 書き込めないディレクトリ（`0500`）で `PUT`／`setup` が 500、鍵の文字列が不正な `setup` は 400。root など権限が効かない環境では書き込みを試して通ったら飛ばす。

## 検証

- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- 変更したファイルの `rustfmt --check`
