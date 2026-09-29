# セキュリティレビューと肥大・重複の整理

リポジトリ全体をダッシュボード・Nostr/agent・IPFS/publish・service/設定・フロントエンドの 5 領域に分けてレビューし、見つかったセキュリティ上の問題を直したうえで、肥大したファイルの分割と重複・説明だけのコメントの削除をした。

## セキュリティ修正

### service

- `swing service install --system` の systemd unit に `User=` が無く、root で動いていた。root で動く daemon が、ユーザーの書けるホームにあるバイナリと設定を実行し、信頼できない Nostr/IPFS の入力を扱っていたことになる。`--run-as <user>`（無ければ `SUDO_UID`、それも無ければ root 以外の現在のユーザー）で実行ユーザーを決め、どれも決まらなければ unit を書かずに止める。system unit には `NoNewPrivileges=yes`・`PrivateTmp=yes`・`ProtectSystem=full`・`ReadWritePaths=<設定のディレクトリ>` を付ける。`ProtectHome` は state がホームにあるので付けない。
- 既に入っている system unit は書き直すまで root で動く。移行コードは入れず、`sudo swing service install --system` のやり直しで切り替える（root で作られた state のファイルは `chown` が要ることがある）。
- unit・plist・タスク XML に埋め込むパスとユーザー名に制御文字があれば拒否する（改行で unit のディレクティブを足せたため）。

### ダッシュボードと CLI・tray

- CLI と tray の `ApiClient` が `HTTP_PROXY` を使っていて、bearer トークンがプロキシへ平文で出ていた。`no_proxy()` にした。
- swing が止まっている間に同じポートを別のローカルユーザーが取ると、tray の 5 秒ごとのポーリングでトークンを集められた。認証なしの `POST /api/identity` を足し、クライアントは乱数 nonce に対する `HMAC-SHA256(token, "swing-identity:" || nonce)` を確かめてからトークンを送る。
- アップロードのパス検証が `:` を通していて、Windows では `C:foo/x` がアップロード用の一時ディレクトリの外に書けた（NTFS の代替データストリームも）。`:` を拒否し、結合後のパスが一時ディレクトリの下にあることを確かめ、`create_new` で作る。
- アップロードのテキストのパートを本文の上限（既定 2 GiB）まで丸ごと読んでいた。1 つ 64 KiB までにし、知らないパートはためずに読み捨てる。大文字小文字だけ違うパスとファイル・ディレクトリの衝突は 500 ではなく 400 にする。
- `GET /login?code=…` でログインコードを消費していたので、リンクのプレビューを作るボットが先に使い切れた。GET は `/#/login/code/<code>` へ 303 で送るだけにし、フロントが `POST /api/login` で使う。
- `write_private_file` の一時ファイル名が固定で、トークンのローテーションと設定の書き込みが同時に走ると競合した。乱数入りの名前で `create_new` する。
- セッション値の発行時刻は ASCII の数字だけを受け付ける（`+123` と `123` が両方通っていた）。
- CSP に `base-uri 'none'; form-action 'self'; object-src 'none'` を足した。`style-src 'unsafe-inline'` はユーザー CSS のために残す。

### 内蔵 gateway と設定

- gateway の Host の解析がダッシュボード側より緩く、`[example.com]junk` が許可リストを通ったうえで、生の Host のまま Kubo へ渡っていた。Kubo はそれを DNSLink 専用のホストと見なさず、パスゲートウェイとして任意の CID を配る可能性があった。解析を `src/host.rs` の 1 つにまとめ、Kubo へは設定にあるホスト名を渡す。
- `gateway.hosts` と `dashboard.allowed_hosts` が重なる設定、`gateway.hosts` に `localhost`・`127.0.0.1` を含む設定は読み込みで拒否する。ほかの人のサイトの HTML がダッシュボードと同じオリジン（Cookie はポートを見ない）で動かないようにするため。
- `dashboard.gateway` を http(s) のオリジンとして検証する。`public_url` と共通の規則で、userinfo（`@`）も拒否するようになった。

### Nostr と agent

- nostr-sdk の既定の上限（メッセージ 5 MB、イベントの大きさは無制限）のまま最大 20 000 件をためていたので、悪意のある relay がメモリを使い切れた。relay クライアントのメッセージを 128 KiB・イベントを 64 KiB・タグを 600 までにした（signer は NIP-44 の暗号文のためイベント 128 KiB）。4096 バイトを超える更新メモは捨て、超えるレプリカ報告は拒否する。`swing publish --message` とダッシュボードの `message` も同じ上限で先に拒否する。
- レプリカ報告を全報告者まとめて件数上限つきで取っていたので、使い捨ての鍵で大量に送れば信頼できる報告（作者と選ばれたミラー）を押し出せた。信頼できる報告者に絞った取得を別に行ってから混ぜる。
- 「新しいミラー」の時刻が、未来の `created_at` を持つ報告で先に進められて通知が止まった。自分が持っている CID を挙げる報告だけを数え、`created_at` が現在より先の報告は記録しない。CID を挙げない取り下げの報告は時刻を動かさなくなった。
- 大きすぎる・ポリシーで拒否した・ディレクトリでなかった更新を、ポーリングのたびに取り直していた（最大 2 GiB ずつ）。拒否した `(サイト, CID)` をメモリに覚え、CID が変わるまで取り直さない。state.json には書かないので、再起動で忘れる。
- 作者をまとめて 1 つの REQ で取っていたので、1 人の作者が件数上限を埋められた。50 人ずつに分けて取る。
- `d` と `title` で、双方向制御や幅ゼロなどの見えない書式文字も拒否する。
- NIP-05 の取得先の公開アドレス判定で、NAT64・6to4・IPv4 互換アドレスに埋め込まれた IPv4 も調べ、Teredo・site-local を拒否する。
- 取得した DAG の大きさの計測を state のロックの外に出した。ロックを離している間に GC で消されないよう、保存中のパスを `Agent.storing` に登録し、GC はそれを飛ばす。
- 同じ `created_at` の site event は NIP-01 の規則（id が小さい方）で選ぶ。
- 生成した鍵と signer のアプリ鍵の文字列を `Zeroizing<String>` にした。

### publish と Kubo・配布

- `swing publish` がサイトの外を指すシンボリックリンクを辿って、ホームのファイルなどを公開できてしまった（一度出すと取り消せない）。解決先がサイトのルートの外なら、確認のモードにかかわらずエラーにする。サイトの中を指すリンクは今まで通り辿る。確認の項目（`off`/`warn`/`require`）にしなかったのは、外のファイルを公開してよい場面が思いつかず、取り消せない事故を防ぐ方を優先したため。
- 前回の Kubo の残りを片付けるとき、記録された API ポートへ shutdown を送ってから pid の起動時刻を確かめていた。別の Kubo を止めうるので順番を逆にした。
- Kubo の応答本文を 16 MiB までにした（管理外の `[ipfs].api` を遠くへ向けた場合に備えて）。
- Dockerfile の `cargo build` に `--locked`、Kubo の初期化スクリプトに `set -f` を付けた。リリースのワークフローでは、`latest` や数字で始まるブランチ名でイメージのタグを上書きできないようにした。ブランチ名のイメージを出す運用は残す。

### フロントエンド

- `t()` の置換で、値の中の `$&` などが置換パターンとして展開されていた。置換関数を使う。
- relay や signer から来るエラー文も表示用のサニタイズを通す。サニタイズの範囲に C1 制御文字・U+2028/2029・U+00AD・U+180E・U+FFF9〜FFFB・タグ文字を足し、正規表現を `\u` のエスケープで書き直した（見えない文字がそのまま書かれていた）。
- Desktop ページの iframe に `sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"` を付けた。
- セットアップ成功後に秘密鍵の入力欄を空にし、`autocomplete="new-password"` にした。
- 壁紙の画像は一辺 16384 px または 8192×8192 px を超えると canvas を作る前に拒否し、最初の描画を 1920 px までにした。
- アップロードの 401 でもログイン画面に戻る。

## 肥大と重複の整理

- `src/config/build.rs` は、設定項目ごとに 9 引数の解決関数を呼び、環境変数名とエラー文を手書きしていた。エラー文が実際の値の出どころとずれている箇所もあった。設定の一覧から環境変数名と `[section].field` を引く `Resolver` にまとめ、エラーは実際に使った出どころ（環境変数か設定ファイルか）を名指すようにした。テストは `build/tests.rs` へ移した。
- `service.rs` を `service/`（templates・process・linux・macos・windows）に、`nostr.rs` を `nostr/`（client・site・follow・report）に、`mirror.rs`・`webring.rs`・`signer.rs` を同様に分けた。`dashboard/mascots.rs` からは nofollow と画像判定を、`dashboard/api.rs` からはセットアップと接続の処理（`setup.rs`）を分けた。`publish::run`・`apply_site_event`・`run_until`・`collect_sites` を段階ごとの関数に分けた。
- `SiteFieldError` からタイトル用の変種を外し、`unreachable!()` を無くした。
- 重複していた `/proc/<pid>/stat` の解析、アカウント単位の state の走査、`DEFAULT_MAX_UPDATE_SIZE`、サービスの識別子と停止の待ち時間、アップロード用ディレクトリの作成、フロントの `showStorageError`・`siteTitle`・`clamp`・`between` などを 1 か所にまとめた。フロントでほかから使われない `export` を外した。
- コードを言い換えただけのコメントを消した。
- `web/style.css` のダークテーマのトークンは 2 か所に同じものがあるが、`light-dark()` にまとめると custom.css で `--swing-*` を上書きしたときの効き方が変わり（ライトだけ → 両方）、古いブラウザで色が消えるため、そのままにした。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace`・Windows 向けの `cargo xwin clippy` が通る。
- フロントは `node --check` と、サニタイズと `t()` を node で確かめた。デモ環境で Desktop 画面と iframe 内のリンクを表示して確かめた。
- macOS 向けのコードはこの回ではコンパイルしていない（読んで確かめただけ）。

## 再評価で見つかったものの修正

別のレビューで重大なものは無かったが、次の中程度のものを直した。

- `swing publish` のサイトのディレクトリに設定ファイル・`agent.state_dir`・`kubo.repo` が入っていれば、確認のモードにかかわらず拒否する（既定の配置では `swing.toml` と `./data` が並ぶので、そのディレクトリを公開すると Nostr の秘密鍵・ダッシュボードのトークン・Kubo の鍵が出てしまう）。ダッシュボードからの公開も同じ確認を通す。
- nostr-sdk の `fetch_events` は既定で合計 10 000 件を超えるとエラーを返して全部捨てるので、使い捨ての鍵で報告を大量に送られると信頼できる報告の取得ごと失敗した。取得をストリームで読み、上限を超えたら古い方を捨てる。上限は 10 000 に下げ、信頼できる報告者の取得は別に並べて走らせ、片方が失敗しても残りを使う。座標は 250 件ずつに分けて取る。
- 「新しいミラー」の時刻と Desktop の通知は、自分の follow set で選んだミラー（と作者）の報告だけで動く。CID は公開されているので、誰でも自分の CID を挙げた報告を作れたため。
- CLI と tray は、トークンを付ける要求の前に毎回 `/api/identity` で本人確認をする。以前は確認の結果をクライアントごとに覚えていたので、`swing stop` の待ちのループで、止まった後にポートを取った相手へトークンが送られえた。`swing stop` の待ちはトークンを付けない `/api/identity` の `instance` で判定する。
- ブラウザは本人確認ができないので、再起動を待つ間は cookie を付けない `/api/identity` で新しい `instance` を確かめてから認証付きの呼び出しに戻り、定期的な呼び出しは接続できなかった後に止めて間隔を延ばす。`/api/identity` をまねる相手には cookie が渡るので、既知の弱点として architecture に書いた。
- ログインのリンクはコードを入れるだけで、送信はボタンを押したときにする（JavaScript を実行するプレビューでも消費されないように）。
- `add_dir` は walk で確かめた解決済みのパスを開く（確認と読み込みの間に差し替えられないように）。
- `[kubo].managed` のとき `dashboard.gateway` の既定を `kubo.gateway_listen` のポートから作る（setup でポートをずらしたとき、リンクが 8080 を取った別のプロセスへ向かっていた）。`ipfs.api` と `gateway.upstream` を http(s) のオリジンとして検証する。
- `d`・`title` で拒否する見えない文字を、フロントのサニタイズと同じ範囲に広げた。拒否した CID の記憶はアカウントごとに 50 件まで。follow set を出し直すときの `created_at` は前の版より必ず新しくする。
- 設定の書き込みは `swing.toml` がシンボリックリンクならリンク先へ書く。ダッシュボードの設定の保存とセットアップのファイル操作を `spawn_blocking` に移した。
- 取得して最新版を選ぶ処理の 4 か所の重複、`/proc` の解析、Kubo への要求の送り方の重複をまとめた。

並列の worktree で cargo の target ディレクトリを共有すると、ほかの worktree のテストバイナリで上書きされて古いテストが走ることがあった。最終の確認は専用の target ディレクトリで、テストの件数がソースの `#[test]` の数と合うことも確かめた。
