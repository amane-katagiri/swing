# ゲートウェイの CSP で Worker を止める

ローカルのゲートウェイに付けた `connect-src 'self' https: wss:; form-action 'self' https:` は、Service Worker を使うと迂回できるという指摘をレビューで受けたので塞いだ。

## 問題

- 応答の CSP は文書ごとに応答ヘッダーから決まる。ミラーしたサイトが Service Worker を登録し、次の読み込みで `new Response(html)` のように作った HTML を返すと、その文書には Kubo が付けた CSP が無く、`http://127.0.0.1:<別ポート>` や LAN へ `fetch` で送れてしまう。
- Service Worker のスクリプト自体はゲートウェイから CSP 付きで配られるので、Service Worker の中からの `fetch` は止まる。抜けるのは Service Worker が作った文書のほう。
- `<cid>.ipfs.localhost` は平文の HTTP でも secure context なので、登録は通る。

## 決めたこと

- CSP に `worker-src 'none'` を足す。CSP には Service Worker だけを止める指定が無いので、`Worker`・`SharedWorker` も止まる。Worker が無いと動かないミラーのサイトは動かなくなるが、ローカルのポートと LAN へ送らせないことを優先した。`docs/site-guide.md` に書いた。
- CSP を付ける前のゲートウェイで登録された Service Worker は残る。`Clear-Site-Data` で消すとサイトの保存データも読み込みのたびに消えるので使わない。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace`。
- デモ環境のゲートウェイの応答に `worker-src 'none'` 付きの CSP が付くことを確かめた。`navigator.serviceWorker.register("sw.js")` を呼ぶページを Kubo に入れて `<cid>.ipfs.localhost` で開き、登録が CSP 違反で失敗することをブラウザで確かめた。
