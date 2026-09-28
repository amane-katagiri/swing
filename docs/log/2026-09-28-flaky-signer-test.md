# 時計の遅れた署名アプリのテストがまれに落ちる件

## 原因

`signer::tests::answers_from_a_signer_whose_clock_runs_behind_are_received` は、テスト内で署名アプリ役のクライアントを立てて relay を購読してから、`RemoteSigner` に署名を頼む。署名アプリ役は `connect()` を待たずに `subscribe` していたので、relay につながる前に `RemoteSigner` の依頼が relay に届くと、署名アプリ役がそれを受け取れず、`RemoteSigner` の応答待ち（テストでは 5 秒）が切れて落ちていた。

`RemoteSigner` の本体の問題ではない。本体は `send_event` が relay の OK を待つので、購読の REQ が同じ接続で先に送られる。

## 変更

署名アプリ役のクライアントを `connect().and_wait(5 秒)` にして、relay につながってから購読する。

## 検証

テストのバイナリを 24 並列で 120 回動かして比べた。

| | 失敗 |
|---|---|
| 変更前 | 120 回中 20 回 |
| 変更後 | 120 回中 0 回 |
