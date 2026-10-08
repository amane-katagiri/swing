# localhost でのゲートウェイの制限

ミラーした他人のサイトを `localhost` 系のホスト名で表示するとき、普通の `https://` のサイトより緩くなるところを洗い出し、Kubo のゲートウェイの設定で塞げるものを塞いだ。

## 確かめたこと

デモ環境の Kubo（サブドメイン形式の `<cid>.ipfs.localhost`）に確認用のページを入れ、Chrome（headless）で開いて比べた。

- `localhost`・`*.localhost`・`127.0.0.1` のページは平文の HTTP でも secure context になる。ただし Service Worker・通知・カメラなどは `https://` のサイトでも使えるものなので、`https://` と比べて増えるものではない。マイクや通知の権限は `prompt`／`default` で、許可の確認が出るのも同じ。
- `ipfs.localhost` は Public Suffix List に無いので、すべての CID が 1 つのサイトになり、あるサイトが置いた `Domain=ipfs.localhost` の cookie をほかの CID のサイトで読めた。`Domain=localhost` は置けなかった。`<cid>.ipfs.localhost` から `localhost:<別ポート>` へのリクエストは `Sec-Fetch-Site: cross-site` で、`SameSite` の cookie は送られなかった。
- パス形式（`127.0.0.1:<port>/ipfs/<cid>/`）のページから、ダッシュボード（`127.0.0.1:<別ポート>`）の `HttpOnly` のセッション cookie は上書きも `path=/api` での覆い隠しもできなかったが、cookie を 300 個置くと追い出されてログアウトした。サブドメイン形式からは追い出せなかった。
- ほかのローカルのポートや LAN へのリクエストは、ループバックのページからは止まらずに届いた（headless だったので、Local Network Access の確認が実際のブラウザで出るかは比べられていない）。
- Safe Browsing はループバックに対して確かめられない。

## 決めたこと

- `https://` と比べて増えるもののうち、塞ぐ価値があってゲートウェイの設定で塞げるのは「ほかのローカルのポート・LAN への送信」と「パス形式でダッシュボードと同じホストになること」の 2 つだと判断した。
- CSP の `sandbox`（`allow-same-origin` なし）も試した。origin が opaque になり、cookie・ストレージ・Service Worker・CacheStorage が SecurityError になり、通知も最初から `denied` になった。ただし sandbox が埋め込みの iframe に引き継がれ、YouTube のプレーヤー（`youtube-nocookie.com` も）が初期化中に localStorage と cookie で例外を出して真っ黒になった。`allow-same-origin` を足せば映るが、それでは親のページも元の origin に戻る。止めていたものの多くは `https://` のサイトでも使えるものなので、付けないことにした。
- `Permissions-Policy` も、localhost で許可なしに使えるようになるものではないので付けない。
- `Gateway.HTTPHeaders` で `Content-Security-Policy: connect-src 'self' https: wss:; form-action 'self' https:` を付ける。sandbox なしで YouTube の埋め込みと localStorage が動き、`fetch` での `localhost:<別ポート>`・LAN の IP・Kubo RPC のポートへの送信が CSP 違反として止まることを確かめた。`<img>` などの GET は止まらない。
- `Gateway.PublicGateways` に `127.0.0.1`・`::1`・`*.localhost` を `Paths: []` で足し、パス形式に 404 を返させる。`localhost` は Kubo の暗黙の既定のままサブドメイン形式へ転送する。`*.localhost` は boxo のワイルドカードが 1 ラベルにしか一致しないので `a.b.localhost` は通る。`*.*.localhost` を足すと `<cid>.ipfs.localhost` も完全一致の側で拾われてサブドメイン形式が 404 になったので足さない。LAN の IP やループバックに解決されるほかの名前は、Kubo の設定では塞げない。
- managed は `kubo::config::apply_config`、compose は `docker/kubo-init.d/001-swing-config.sh` で同じ値を入れ、値が揃っていることをテスト（`gateway_config_matches_shell_script`）で確かめる。`HTTPHeaders` はオブジェクトごと置き換える。CORS のヘッダーは Kubo が別に付けるので消えない（`Access-Control-Allow-Origin: *` が残ることを確かめた）。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace`。
- デモ環境で `ipfs` を再起動し、応答に CSP が付くこと、`127.0.0.1`・`[::1]`・`foo.localhost`・`ipfs.localhost` のパス形式が 404、`localhost` が 301、`<cid>.ipfs.localhost` が 200 になることを確かめた。サンプルのサイトの表示とリンク、YouTube の埋め込み、localStorage、同じサイトへの `fetch` が動くことをブラウザで確かめた。
