# CLI の出力から relay の接続ログを除く

## 決めたこと

`swing up` 以外のコマンドは、`RUST_LOG` が無いときの既定のフィルタを `info,nostr_sdk=warn,nostr_connect=warn` にする。`swing up` は今までどおり `info`。

nostr-sdk は relay への接続と切断のたびに INFO で `Connected to '<relay>'`・`Relay '<relay>' has been shutdown.` を出す。常駐する `swing up` のログには役に立つが、一度だけ動く CLI では結果の出力の間に挟まって読みにくい（`swing signer pair` では QR とメッセージの間に入る）。swing 自身の INFO は残したいので、nostr-sdk と nostr-connect だけを warn にした。`RUST_LOG` を指定すればどのコマンドでも今までどおり上書きできる。

## 検証

使い捨ての nostr-rs-relay をローカルに立てて `swing sites` を実行し、標準エラーを比べた。

- 既定: `(no follow set found)` だけ。
- `RUST_LOG=info`（以前の既定）: その前後に `nostr_sdk::relay::inner` の接続と切断の INFO が 2 行出る。
