# 停止・再起動とログイン（`src/dashboard/api.rs`, `src/dashboard/session.rs`）

[`../http-api.md`](../http-api.md) の子ページ。共通の形式・エラーは親ページを、ログインコード・セッション・トークン・本人確認の仕組みは [`../security.md`](../security.md) を参照。

## POST /api/shutdown, POST /api/restart

`swing up` を止める／プロセス内で再起動する。ボディは不要（送っても無視する）。

`shutdown::ExitRequest` の `stop()`／`restart()` を呼んで、すぐに `202 Accepted` を返す。止まるまでの流れと時間は [`../../up.md#停止の時間予算`](../../up.md#停止の時間予算)、プロセス内再起動は [`../../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit`](../../up.md#終了要求と-exit-codeshutdownexitrequest-shutdownexit)。

```json
{ "ok": true, "action": "stop" }
```
```json
{ "ok": true, "action": "restart" }
```

## POST /api/login-code

使い捨てのログインコードを発行する（`swing dashboard open` と `swing-tray` が使う）。`Authorization: Bearer` で認証したときだけ受け付け、セッション cookie での呼び出しは 403（[`../security.md#ガード`](../security.md#ガード)）。ボディは不要。`expires_in` は有効期限の秒数。

```json
{ "code": "cc2455ac565b74586b0628e1d7bda4c3", "expires_in": 300 }
```

## POST /api/identity

認証なしで受け付ける。ボディは `{"nonce": "<64 文字の hex>"}`（32 バイト）。長さか文字が違えば 400。返すのは HMAC-SHA256（鍵はトークン、メッセージは `swing-identity:` と小文字にした nonce）の hex の `proof` と、[`GET /api/overview`](status.md#get-apioverview) と同じ `instance`。

```json
{ "proof": "5f0c…（64 文字）", "instance": "3f9a0c1d2b4e5f60" }
```

## POST /api/login

認証なしで受け付ける。ボディは `{"code": "<ログインコード>"}`（前後の空白は無視、大文字小文字は区別しない）。コードが有効なら消費して `200 {"ok": true}` とセッション cookie（`Set-Cookie`）を返す。無効・期限切れ・使用済みなら 401 `{"error": "invalid or expired login code"}`。

## POST /api/token/rotate

トークンを作り直す。`POST /api/login-code` と同じく Bearer のときだけ受け付ける（cookie なら 403）。成功で `200 {"ok": true}`、ファイルが書けなければ 500。
