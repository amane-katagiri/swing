# テスト

[`../architecture.md`](../architecture.md) の子ページ。ユニットテストの置き方と、統合テストの動かし方。ビルドとリリース、macOS・Homebrew・Windows の動作確認は [`release.md`](release.md)。

- ユニットテスト: `cargo test`。大きいテストモジュールは `foo.rs` に `#[cfg(test)] mod tests;` を置いて `foo/tests.rs` に分ける（`config/build/tests.rs`・`nostr/client/tests.rs`・`nostr/client/resend/tests.rs`・`nostr/site/tests.rs`・`agent/replicas/tests.rs` など）。agent と health のテストは MFS をメモリ上で真似る `FakeKubo`（`src/test_support.rs`）を使い、agent はこれに `FakeNip05` を組み合わせる。publish の一連（`swing publish` とダッシュボードのアップロード）と、relay と Kubo につながったダッシュボードの API のテストは、Kubo の HTTP API（MFS・add・dag/stat・ls）をメモリ上で真似る `KuboApi` と nostr-sdk の `LocalRelay`（書き込みを断る `read_only_relay` を含む）を使う（どちらも `src/test_support.rs`）。ダッシュボードのテストは axum の `Router` に `oneshot` でリクエストを投げ、TCP で listen しない。署名アプリとのやり取りのテストは [`signer.md#テスト`](signer.md#テスト)。`ipfs::tests::kubo_client_probe` は `#[ignore]` だが、通常のテスト `kubo_client_ignores_proxy_environment` が `SWING_TEST_KUBO_PROBE_API` とプロキシの環境変数を付けた子プロセスとして起動する（Kubo のクライアントがプロキシを通らないことの確認）。
- ユニットテストは実行する環境に左右されない。`config::env_var` は `cfg(test)` では常に `None` を返すので、シェルに `SWING_*` が設定されていても `Config::load` と設定の書き換えは環境変数を読まない（環境変数の扱いを確かめるテストは `build_config` などに引数で渡す）。ダッシュボードと agent のテスト用の設定（`dashboard/test_support.rs`・`agent/test_support.rs` の `test_config`）は、設定ファイルと `state_dir` を一時ディレクトリの下の存在しないパスに向けるので、作業ディレクトリの `./swing.toml` や `./data` を読み書きしない。`tests/` の統合テストはライブラリを通常どおりにビルドするので、環境変数を読む。
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

`kubo::tests` の `#[ignore]` テスト（Kubo の一生・孤児の回収）と `mfs_kubo_integration`（空白・記号・`.`/`..`・日本語・長い名前を含む `d` が、agent と publish の MFS パスで往復し、`find_garbage` が誤検知せず、削除できること）は Kubo のバイナリを子プロセスとして起動する（repo は一時ディレクトリ）。`SWING_TEST_KUBO_BIN` が無いと、黙って通らずに失敗する:

```bash
SWING_TEST_KUBO_BIN=/path/to/ipfs cargo test --lib kubo::tests -- --ignored
SWING_TEST_KUBO_BIN=/path/to/ipfs cargo test --test mfs_kubo_integration -- --ignored
```
