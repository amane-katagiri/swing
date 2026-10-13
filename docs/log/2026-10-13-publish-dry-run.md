# publish の dry run と JSON の出力

AI エージェント向けのスキルからサイトを確かめるとき、本番の publish を叩かずに済むよう、何も公開しない `swing publish --dry-run` と、機械が読める `--json` を足した。人間が直しながら何度も確かめるのにも使える。

## 決めたこと

- 段階の関数は複製しない。`publish.rs` の段階から表示を切り離し（`prepare`・`read_and_check`・`date_version`・`print_section` など）、本番の `run` と `dry_run::evaluate` が同じ関数を呼ぶ。本番の表示と挙動は変えていない。dry run は結果を `Evaluation` に集めてから、文字の表示か JSON に変える。
- dry run は本番が止まる条件に当たっても途中で止めず、全部の段階を通してから止める理由をまとめて返す。エージェントが 1 回の実行で直すべきものを全部知れるようにするため。
- CID は Kubo の `add` に `only-hash=true` を足して求める。ほかのオプションは本番の add と同じ文字列（`ADD_QUERY`）を共有し、同じ CID になることを統合テストで確かめた。ブロックを保存しないので `dag/stat` はできず、サイトイベントの `size` タグの値は出さない。代わりにファイルの合計（`checks.size.bytes`）を常に出す。
- 鍵・Kubo・relay が無くても、分かることは必ず出す。公開鍵は `Signer::load` で読むだけで、署名アプリには接続しない（`Channel` は署名を頼んだときに初めてつなぐので、作るだけなら通信しない）。鍵が無い・読めないときは NIP-05・前の版・MFS の版を「不明」にし、relay の時計は鍵が無くても取る。Kubo が無ければ CID・前の版のファイル一覧・同じ内容かを「不明」にする。「不明」は止める理由にしない。
- 設定ファイルが無いときは `Config::load` の既定の動き（既定の場所に無ければ既定値で続け、`--config` や `SWING_CONFIG` で指したファイルが無ければエラー）をそのまま使う。既定値では管理下の Kubo を使うので、`swing up` が動いていなければ CID は「不明」になり、ローカルの確認だけが出る。
- 引数の誤り・設定の読み込み・保護パス・ディレクトリの一覧の失敗は、評価を始める前のエラーとして本番と同じメッセージで止め、`--json` でも JSON を出さない。JSON は常に評価を通した結果だけを表すようにして、フィールドが「評価していない」のか「不明」なのかを区別しなくて済むようにした。保護パスの中を一覧したり Kubo に送ったりもしない。
- 終了コードは本番のエラーに揃えて 1。止める理由は `the publish would stop: <本番のメッセージを ; でつないだもの>` を標準エラーに出す。同じ内容で `check_unchanged` が `require` なら、本番が正常終了するので 0。
- `check_unchanged` が `off` でも、CID と前の版を比べた結果は出す（`unchanged.status`）。止めるかどうか（`skips_publish`）にはモードを使う。
- JSON の値が分からない項目は `status: "unknown"` と `detail` で表し、`null` は値が無いこと（`url` を付けていない、前の版が無い）にだけ使う。スキーマの版は `format: 1`。
- `--json` のときは tracing のログを標準エラーに出す。既定の出力先は標準出力で、relay の接続の失敗のログが JSON に混ざるため。ほかのコマンドの出力先は変えていない。
- `--yes` は dry run でも受け付けて使わない。エージェントが本番と同じ引数に `--dry-run` を足すだけで動くように。
- 公開鍵が設定と `remote-signer.json` の両方にあるなど、本番では `Signer::require` で止まる設定の誤りも、dry run では公開鍵の「不明」の理由として出し、止める理由には数えない。
- `docs/architecture/publish.md` が 20KB に近づいたので、dry run の段階ごとの分岐は子ページ `publish/dry-run.md` に置いた。

## 作ったもの

- `IpfsClient::hash_site`（`add_site` と同じ要求に `only-hash=true`）。
- `publish::dry_run`（`evaluate`・表示・JSON の `Report`）。`LocalChecks::abort_reasons`（確認ごとの止める理由）、`PreviousFiles::load_with`（Kubo が無いときの「不明」）、`RelayState::clocks_only`。
- CLI の `--dry-run` と `--json`（`requires = "dry_run"`）。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通った。
- 単体テスト: 偽の Kubo（呼ばれた RPC を記録する）とローカルの relay で dry run を通し、Kubo には `only-hash=true` の `add`・`files/ls`・`ls` しか来ないこと、relay のイベントが増えないことを確かめた。ほかに、同じ CID で `require` なら `unchanged` で止める理由が無いこと、NIP-05・ドットファイル・リンク・時計の止める理由が全部集まって `run` が失敗すること、鍵も Kubo も無いときに JSON の各項目が `unknown` になりローカルの確認は出ること、引数の誤りと保護パスでは評価を始めないこと、`--json` だけでは clap が止めること。
- 統合テスト（`#[ignore]`、ローカルの Kubo）: `hash_site` の CID が `add_dir` と同じで、`hash_site` の後にブロックがローカルに無く pin も無いこと。オフラインで動かした Kubo 0.43.1 で `kubo_integration` を全部通した。
- 手で、オフラインの Kubo と接続できない relay を設定して実行し、表示・JSON（標準出力は JSON だけで、ログとエラーは標準エラー）・終了コード（止める理由があれば 1、無ければ 0）を見た。実行の後に MFS が空のままで、CID のブロックが Kubo に無いことも確かめた。設定ファイルも鍵も無い環境では、ローカルの確認だけが出て、CID などが理由付きで「不明」になり、終了コード 0 で終わることを見た。
