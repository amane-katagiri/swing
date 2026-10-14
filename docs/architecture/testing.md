# テスト

[`../architecture.md`](../architecture.md) の子ページ。ユニットテストの置き方と、統合テストの動かし方。ビルドとリリース、macOS・Homebrew・Windows の動作確認は [`release.md`](release.md)。

- ユニットテスト: `cargo test`。テストモジュールがおよそ 250 行を超えたら、`foo.rs`（または `foo/mod.rs`）に `#[cfg(test)] mod tests;` を置いて `foo/tests.rs` に分ける（`state/tests.rs`・`policy/tests.rs`・`config/build/tests.rs`・`nostr/site/tests.rs`・`agent/tests.rs`・`dashboard/upload/tests.rs` など）。それでも大きいものや話題がいくつもあるものは、`foo/tests.rs` に共有の補助と全体にかかわるテストを残し、話題ごとのテストを `foo/tests/<話題>.rs`（`tests.rs` の `mod <話題>;`）に分ける: `kubo/tests/`（`access`・`binary`・`config`・`daemon`・`orphan`、サブモジュールごと）、`publish/links/tests/`（`html`・`css`・`js`。参照の解決と報告の整形は `links/tests.rs`）、`publish/tests/run.rs`（`swing publish` の一連）、`dashboard/api/tests/ready.rs`（relay と Kubo につながった API）。入力と期待値だけが違うテストは、名前付きの行を並べた表にまとめ、失敗メッセージに行の名前を入れる（`assert_eq!(got, want, "{case}")`）。前提の組み立てや確かめる性質が違うものは分けたままにする。サービス定義の雛形（`service/templates/tests.rs`）は、出力の全文をテストに書いた期待値と比べる。agent と health のテストは MFS をメモリ上で真似る `FakeKubo`（`src/test_support.rs`）を使い、agent はこれに `FakeNip05` を組み合わせる。publish の一連（`swing publish` とダッシュボードのアップロード）と、relay と Kubo につながったダッシュボードの API のテストは、Kubo の HTTP API（MFS・add・dag/stat・ls）をメモリ上で真似る `KuboApi` と nostr-sdk の `LocalRelay`（書き込みを断る `read_only_relay` を含む）を使う（どちらも `src/test_support.rs`）。ダッシュボードのテストは axum の `Router` に `oneshot` でリクエストを投げ、TCP で listen しない。署名アプリとのやり取りのテストは [`signer.md#テスト`](signer.md#テスト)。`ipfs::tests::kubo_client_probe` は `#[ignore]` だが、通常のテスト `kubo_client_ignores_proxy_environment` が `SWING_TEST_KUBO_PROBE_API` とプロキシの環境変数を付けた子プロセスとして起動する（Kubo のクライアントがプロキシを通らないことの確認）。
- テスト用の部品は 3 か所にまとめ、各テストで作り直さない。`src/test_support.rs` はクレート全体で使うもの: Kubo の偽物（`FakeKubo`・`KuboApi`、決まった応答だけを返す `kubo_replying`、生の HTTP 応答を返して届いたリクエストを記録する `serve_raw`）、relay（`relay_holding`・`serve_relay`・`seed_relay`・`refusing_relay`・`read_only_relay`・偽のイベントを返す `forging_relay`・`events_on_relay`）、イベントの組み立て（`make_site_event`・`site_event_fixture`・`site_event_with`・`follow_set`・`full_follow_set`・`anchor_ad_event`・`replica_report_event`・`replica_report`・`deletion_event`・タグを直接並べる `raw_event`）、`version`（`VersionRecord`）、設定（`MINIMAL_TOML`・`config_at`）、署名アプリ（`remote_signer_file`・`unanswered_signer`・`serve_test_signer`）、共通の定数（`CID_A`・`CID_B`・`PEER_*`）と小物（`serve_router`・`unused_local_addr`・`unused_dir`・`sorted_pubkeys`・unix の `mode`）。`src/agent/test_support.rs` は agent の `Fixture`・`FakeRelay`・`FakeNip05`、`src/dashboard/test_support.rs` はダッシュボードの状態と設定、リクエストの組み立て（`request`・`get`・`post_json`・`multipart_request`）と送受信（`call`・`send_json`・`json_body`）。
- ユニットテストは実行する環境に左右されない。`config::env_var` は `cfg(test)` では常に `None` を返すので、シェルに `SWING_*` が設定されていても `Config::load` と設定の書き換えは環境変数を読まない（環境変数の扱いを確かめるテストは `build_config` などに引数で渡す）。ダッシュボードと agent のテスト用の設定（`dashboard/test_support.rs`・`agent/test_support.rs` の `test_config`）は、設定ファイルと `state_dir` を一時ディレクトリの下の存在しないパスに向けるので、作業ディレクトリの `./swing.toml` や `./data` を読み書きしない。`tests/` の統合テストはライブラリを通常どおりにビルドするので、環境変数を読む。
- ダッシュボードの JS のテスト: `node --test 'web/tests/*.test.mjs'`（Node.js の組み込みのテストランナーと `node:assert/strict`。npm の依存・`package.json`・バンドラは使わない。ディレクトリを渡す書き方は Node.js 22 では動かないので glob を渡す）。`web/tests/` はダッシュボードの配信物（`src/dashboard/assets/mod.rs` の明示的な一覧）に入らず、`every_imported_module_is_served` も `web/` の直下しか見ない。各テストファイルは最初に `web/tests/setup.mjs` を読み込む。これは `TZ=UTC`、メモリ上の `localStorage`（`swing:lang` は `en`）、何も返さない `document`（`EventTarget` なので `dispatchEvent` は動く）、`navigator` を置き、`web/` のモジュールを本番のまま読み込めるようにする。DOM を組み立てるコード（`graph.js` の配置など）はテストしない。テストに要る関数は本番のモジュールから `export` する（`normalizeManifest`・`MESSAGES`・`collectNotices`・`diffReporters`・`computeRelativePath`・`findNewFiles`・`groupByFolder`・`snapScale`）。
  - `mascot-pack.test.mjs`: 同梱パック（`web/mascots/*/`、サーバが埋め込むものと一致すること）がすべて `normalizeManifest` を通り、使うコマ番号が PNG の `IHDR` から読んだシートの大きさに収まり、シートが上限以内で、せりふが種類ごとに差し込める `{title}`・`{d}`・`{count}` だけを使うこと。`normalizeManifest` の既定値と、[`dashboard/mascot/pack.md`](dashboard/mascot/pack.md) の検証規則の境目（通るものと落ちるもの）。
  - `i18n.test.mjs`: `en` と `ja` のキー集合が同じで、キーごとに `{var}` の集合が一致し、`web/*.js` の `t()` の第 1 引数に書いた文字列リテラル（三項演算子の枝を含み、比較の相手は除く）と、キーの対応表（`NIP05_ERROR_KEYS`・`LINK_KIND_KEYS` など。表の名前はテストに列挙する）の値がすべて `en` にあること。`index.html` の `data-i18n*` は Rust の `every_static_i18n_key_exists_in_both_languages` が見る。
  - `unsafe-unicode.test.mjs` と `format::tests::sanitizing_matches_the_vectors_shared_with_the_dashboard`: 共有のテストベクタ `web/tests/unsafe-unicode.json` を JS（`util.js` の `sanitizeDisplayText`）と Rust（`format.rs` の `sanitize_display_text`）の両方で照らす。`chars` は境目を含むコードポイントごとに `a<c>b` を消す（`removed`）・空白にする（`space`）・残す（`kept`）のどれか、`texts` は文字列と切り詰めの長さ。両者が食い違う項目は `want` の代わりに `js`・`rust`・`difference` を持つ（[テストの監査の追記](../log/2026-10-14-test-audit.md#追記-ダッシュボードの-js-のテスト)）。どちらかの集合を変えたら、このファイルを直さない限り片方のテストが落ちる。
  - `util.test.mjs`（`formatBytes`・`backoffDelay`・`sanitizeDisplayText` などの純粋な関数と `apiResponseError`）・`desktop-updates.test.mjs`（`collectNotices`・`diffReporters`・`isNoticeEvent`）・`mascot-behavior.test.mjs`（`createBehavior` に固定の `random` と時刻を渡し、待機・歩く・寝る・話す・驚く・つまむ・落ちるの移り変わりを外から見える値で確かめる）・`publish.test.mjs`（フォルダのアップロードの相対パス・新しいファイル・フォルダごとのまとめ）・`desktop-scale.test.mjs`（`snapScale`）・`fingerprint.test.mjs`（`fingerprintIds`・`emojiChar`）。
  - 見つかった不具合は直すまで `{ skip: 'bug: ...' }` で残す。
- 統合テスト（`#[ignore]`、ローカルの Kubo / relay が必要。公開ネットワークには接続しない）:

```bash
# test プロファイルは bootstrap とローカル探索を無効にする
docker run -d --rm -e IPFS_PROFILE=test -p 127.0.0.1:15001:5001 ipfs/kubo:v0.43.1
# SWING_TEST_IPFS_API（既定 http://127.0.0.1:15001）。add_dir_matches_ipfs_cli_cid_for_known_fixture は固定のフィクスチャの CID を `ipfs add -r --cid-version=1` の結果と照らす
cargo test --test kubo_integration -- --ignored --test-threads=1
# agent の保存・sweep・突き合わせ・unfollow を実物の Kubo で通す（health.rs の検査を含む）
cargo test --lib agent_stores_and_removes_through_real_kubo -- --ignored

docker run -d --rm -p 127.0.0.1:18080:8080 scsibug/nostr-rs-relay
# SWING_TEST_RELAY（既定 ws://127.0.0.1:18080）。サイトイベント・レプリカ報告・Follow Set の送受信と #p での Follow Set の取得
cargo test --test nostr_relay_integration -- --ignored --test-threads=1
```

`kubo::tests` の `#[ignore]` テスト（`daemon` の Kubo の一生・`orphan` の孤児の回収）と `mfs_kubo_integration`（空白・記号・`.`/`..`・日本語・長い名前を含む `d` が、agent と publish の MFS パスで往復し、`find_garbage` が誤検知せず、削除できること）は Kubo のバイナリを子プロセスとして起動する（repo は一時ディレクトリ）。`SWING_TEST_KUBO_BIN` が無いと、黙って通らずに失敗する:

```bash
SWING_TEST_KUBO_BIN=/path/to/ipfs cargo test --lib kubo::tests -- --ignored
SWING_TEST_KUBO_BIN=/path/to/ipfs cargo test --test mfs_kubo_integration -- --ignored
```
