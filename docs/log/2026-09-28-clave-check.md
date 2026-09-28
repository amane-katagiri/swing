# iPhone の Clave で NIP-46 の接続と署名を確かめる

## やったこと

使い捨ての Nostr アカウントを入れた iPhone の Clave と、`swing signer pair` でペアリングし、署名を頼んだ。署名だけを頼んで relay には送らない使い捨てのプログラム（`Signer::require` → `Signer::sign` で kind 35981 の空のイベント）を使い、公開 relay にイベントを出さずに確かめた。署名アプリとのやりとりの relay を `wss://relay.primal.net`（既定）と `wss://relay.powr.build` で 1 回ずつペアリングした。

## 結果

- ペアリング: どちらの relay でも、Clave の QR 読み取りで `nostrconnect://` を読めた。接続の画面に、リクエストした kind の一覧が出た。確認の署名（probe）は通った。
- Clave の署名アプリの公開鍵は、ユーザーの公開鍵と同じだった。

| relay | Clave | 画面 | 結果 |
|---|---|---|---|
| primal | 開いている | オン | 4.7 秒で署名が届いた |
| primal | 閉じている | オフ | 90 秒のタイムアウト |
| primal | 閉じている | オン | 90 秒のタイムアウト |
| powr | 閉じている | オン | 3.1 秒で署名が届いた |
| powr | 閉じている | オフ | 3.7 秒で署名が届いた |

primal で失敗したときも、iPhone にはプッシュ通知が出ていて、Clave の履歴にはその依頼がチェック付き（署名済み）で残っていた。Clave は依頼を受け取って署名していたが、答えが primal 経由では SWING に届かなかった。閉じているときにプッシュで起きた Clave が答えを返せるのは、`relay.powr.build` でつないだときだけ、と判断した。画面の点灯は結果に関係しなかった。

NIP-46 の kind 24133 は relay が保存しなくてよいので、primal に答えが送られなかったのか、送られたが SWING の購読に届かなかったのかは、relay の保存内容からは区別できなかった。

閉じているときの Clave が答えを別の relay に送っている可能性を見るため、primal でつないだ状態で閉じたまま署名を頼み、その間 `wss://relay.powr.build` と `wss://relay.damus.io` でもアプリ鍵宛ての kind 24133 を購読した。どちらにも答えは来ず、Clave の履歴にはこのときも署名済みの依頼が増えた。原因（答えを Clave のサーバー経由で送っていて、そこが powr でつないだ接続にしか出さないのか、primal が受け付けないのか）は SWING の側からは分からなかった。

## 変更

- README の署名アプリの一覧: Clave は確認済みとし、閉じていても答えさせるには relay を `wss://relay.powr.build` にすることを書いた。以前の「コピーしたリンクを Connect に貼り付けてつなぐ」は QR 読み取りで足りるので外した。
- セットアップ画面の Clave の注記を同じ内容にした（`setupSignerAppsClaveNote`）。
- 既定の relay（`wss://relay.primal.net`）は変えていない。Amber は primal で確かめていて、Clave だけのために既定を変える理由がないため。
