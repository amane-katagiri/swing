# セットアップモードでのポートのずらし

## 背景

初回起動（セットアップモード）で、ダッシュボードの既定ポート（8082）がほかのプロセスに使われていると `swing up` が bind で落ち、セットアップ画面にたどり着けなかった。Kubo の gateway の既定ポート（8080）が使われている場合は、セットアップモードでは Kubo を起動しないので気づけず、セットアップを終えて通常モードに入ってから Kubo の起動失敗のバックオフを繰り返していた。

## 決めたこと

- ずらすのはセットアップモードの間だけ。通常モードではこれまでどおり、使われていればエラー。
- ダッシュボードも Kubo の gateway も、セットアップモードで起動したとき（ダッシュボードの bind のとき）に空いているポートを決めて、設定ファイルへまとめて書く。
  - ダッシュボードは、`swing dashboard open`・`swing-tray`・`swing stop` が設定ファイルの `listen` を見てつなぐので、セットアップの確定（`POST /api/setup`）まで待つと、その間のログインリンクが古いポートを向いてしまう。
  - Kubo の gateway は、最初は `POST /api/setup` のときに書く形で作った。しかしセットアップモードは `swing signer pair`（`remote-signer.json` だけを書く）でも抜けられ、その経路では書かれず、8080 が使われていると通常モードで Kubo の起動失敗のバックオフを繰り返すだけになる。`swing signer pair` 側でも書く案は、`--no-port-shift` を CLI 側にも付けることになり、付け忘れるとずれてしまうので採らなかった。起動時に書けば、どの経路で抜けても値が揃い、`--no-port-shift` も `swing up` だけで済む。書き込んでからセットアップを終えるまでの間にそのポートがほかのプロセスに使われると、通常モードで Kubo が起動できない（`[kubo].gateway_listen` を手で直すことになる）。起動からセットアップ完了までは短く、その間に起動し直せば探し直すので受け入れた。
- ずらす必要が無くても、ポートは設定ファイルに書く。後から既定値が変わったり、別の設定で上書きされたりしても、確定したポートが変わらないようにするため。
- 環境変数由来の値は絶対にずらさない（Docker のポート公開と食い違う）。設定ファイルに書いてある値はずらす（セットアップ前のファイルは `swing.example.toml` の写しであることが多い）。ダッシュボードはセットアップモードの間は毎回ずらしてよいので、セットアップを終えずに再起動して前回書いたポートが使われていても起動できる。
- ポートの調整を一切しない `swing up --no-port-shift` を用意した。ずらさず、書き込みもしない。環境変数 `SWING_NO_PORT_SHIFT` でも指定できる（clap の `env` 機能を有効にした）。
- Docker イメージ（`Dockerfile`・`docker/release.Dockerfile`）は `ENV SWING_NO_PORT_SHIFT=true` にした。コンテナの中ではほかのプロセスとポートがぶつかることがほぼ無く、ずらしても得がない。逆にずれると `-p` で公開したポートの先で誰も待ち受けず、その値が volume の `swing.toml` に残って直らない。`CMD ["up", "--no-port-shift"]` にしなかったのは、`docker run <image> up ...` で `CMD` を上書きされるとフラグが消えるため。compose は `SWING_DASHBOARD_LISTEN` を渡し、`SWING_KUBO_MANAGED=false` なので、もともとずれない。
- ずらし方は「同じ IP でポートを 1 ずつ上げて 20 個先まで、駄目ならポート 0」。近いポートの方が覚えやすく、ファイアウォールの設定にも合わせやすい。`AddrInUse` に加えて `PermissionDenied` でも次に進むのは、Windows の除外ポート範囲（Hyper-V などが予約する）で bind がこのエラーになるため。
- swarm ポートはずらさない。外からつながれるようにルータで転送していることが多く、黙って変えると到達できなくなるため。

## 作ったもの

- `src/ports.rs`: `bind_shifting`・`free_addr`・`may_shift`。bind は `tokio::net::TcpListener`（unix では `SO_REUSEADDR` 付き）で、これまでのダッシュボードの bind と、Kubo（Go）の listen の条件に揃えた。
- `settings::pin_addrs`: `editable: false` のアドレスのキー（`Kind::SocketAddr`）を書く。
- `up::bind_dashboard`: ダッシュボードの bind を `AppState` の作成より前に移し、セットアップモードではダッシュボードと Kubo の gateway のアドレスを決めて書き込み、書き込み後の設定で `AppState` を作る。

## 検証

- `ports` の単体テスト、`settings::pin_addrs` の単体テスト、`up::bind_dashboard` で両方のポートを塞いだ状態のテスト（ずらして書く／`--no-port-shift` 相当でずらさず書かない／環境変数由来の gateway はずらさない）。
- 手元でダッシュボードと Kubo の gateway のポートを別プロセスで塞いでセットアップモードの `swing up` を起動し、どちらも次のポートにずれて `swing.toml` に書き込まれること、ダッシュボードがそのポートで待ち受けること、`swing dashboard open` がそのポートのリンクを出すことを確かめた。`--no-port-shift` と `SWING_DASHBOARD_LISTEN` 指定ではこれまでどおり `Address already in use` で終了し、ファイルに書き込まないことも確かめた。
