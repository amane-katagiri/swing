# 全体レビューの指摘の修正

コード全体を 4 つの範囲（ダッシュボードと認証・リモートのデータとゲートウェイと Kubo・publish と署名と agent・プロセス管理と配布物）に分けてレビューし、出た 34 件の指摘を 1 件ずつコードで確かめてから直した。

## 判定

- 確かめた結果は、そのとおり起こるもの 6 件・一部だけ正しいもの（重大度・シナリオ・修正案のどれかが誇張か不正確）18 件・既知（todo やドキュメントに記載済み）9 件・起こらないもの 1 件だった。重大度「中」で残ったのは、外部の Kubo での内蔵ゲートウェイと、カレントディレクトリの設定ファイルの 2 件。
- 直さなかったもの:
  - 同じ `created_at` の置換可能イベントで先に保存した版を保つのは、`docs/protocol.md` で許している挙動。
  - 内蔵ゲートウェイが `X-Forwarded-Proto: http` を固定で付けても、公開ホストは `UseSubdomains: false` で、ディレクトリの末尾の `/` への転送は boxo がパスだけの `Location` で返すので、https から http へ落ちない。
  - 既存の Service Worker が残る件・Kubo の API ポートの `/debug/` と `/logs`・Windows インストーラーの停止・`brew uninstall` の LaunchAgent・配布物の署名・ダッシュボードの未認証 DoS は既知のまま。
  - `report_ttl` の判定（`report_ttl / 2 > poll_interval`、秒の整数）は、文言の「`poll_interval` の 2 倍より長い」と奇数秒の境目の 1 秒だけ食い違うが、出し直しの間隔も同じ `ttl / 2` なので判定はそのまま、文言も読みやすさを取って変えない。
  - systemd の `WorkingDirectory` を引用符で囲む案は、`WorkingDirectory` が引用符を外さないので採らなかった。

## 決めたこと

- カレントディレクトリの `swing.toml` を自動では読まない。他人のディレクトリで `swing up` すると、そこの `[kubo].binary` を実行し、`[agent].state_dir` の symlink でファイルを壊しうるため。既定の場所以外の設定ファイルは `--config` か `SWING_CONFIG` で指す。既定の場所を決められないときのカレントディレクトリへのフォールバックは Docker イメージが使うので残した。カレントディレクトリに `swing.toml` があっても警告は出さない。
- 内蔵ゲートウェイは、裏の Kubo の設定に頼らず、自分で `GET`・`HEAD` 以外を 405、`/ipfs`・`/ipns` で始まるパスを 404 にする。unmanaged の Kubo では SWING が `Gateway.NoFetch` も `PublicGateways` も設定しないので、これまでは任意の CID をネットワークから取って配れた。パスはデコードを繰り返し、`.`・`..`・`\` を畳んでから判定する。その結果、サイトの最上位の `ipfs`・`ipns` という名前は開けない。
- ゲートウェイの CSP に `script-src 'self' https: blob: data: 'unsafe-inline' 'unsafe-eval'` を足し、`http:` の他の origin のスクリプトを読み込ませない。ローカルのポートや LAN の JSONP などを `<script>` で読んで持ち出せたため。`object-src 'none'` は、`<object>`・`<embed>` での PDF の埋め込みが動かなくなるわりに、iframe で読み込めるものと変わらないので付けない。
- 並行する publish の版のパスは、`add` で CID だけを取り、既存のパスなら断る `files/cp` で置くことで確保する。`add --to-files` は既存のパスがあると何も置かずに成功するので、空のディレクトリで予約する案はやめた。断られたら 1 秒進めて置き直す。
- sweep は、MFS の一覧をロックの外で取り、消す前にロックを取り直して、そのときの state で残すべきものを外す。
- そのほか: relay のエラーをダッシュボードに出す前に CLI と同じく無害化して 500 文字で切る。ダッシュボードのトークンは 64 文字の小文字 hex でなければエラー、作り直しは専用のロックで直列にする。署名アプリの返事の購読は、ペアリング後は署名アプリの鍵に絞る。`remote-signer.json` の権限が広ければ警告する。ロックファイルは Unix で `O_NOFOLLOW` で開き、通常ファイルでなければ断る。`swing stop --timeout` は通信も含めた全体の上限にする。ポートの `PermissionDenied` を次の候補へ進めるのは Windows だけにする。サービス定義は一時ファイルから rename で置き換え、macOS は書き終えてから bootout する。Kubo が入れ替わったら通信量の前回値を捨てる。デモのスクリプトを空白を含むパスで動くようにする。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace`・`cargo xwin clippy --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings`。
- 外に出ない Kubo v0.43.1（`--offline`）で `tests/kubo_integration.rs` を流し、`files/cp` が既存のパスを断ること、`add` が MFS に何も置かないことを確かめた。
- デモ環境のゲートウェイで、新しい CSP のもとで、インラインのスクリプト・`eval`・同じ origin・`https:`・`data:`・`blob:` のスクリプトと YouTube の埋め込みが動き、`http://127.0.0.1:<port>` と別の CID の `http://<cid>.ipfs.localhost` のスクリプトが `script-src-elem` の違反で止まることをブラウザで確かめた。サンプルのサイトも違反なしで表示された。
