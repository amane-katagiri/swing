# NIP-05 検証（nip05.rs）

[`architecture.md`](../architecture.md) の一部。

## 判定

`d` が次をすべて満たすときだけドメインとして扱い、満たさなければ `NotApplicable`。

- ラベルが 2 つ以上で、`/ : @ ? #` を含まない
- 各ラベルが `[a-z0-9-]`（大文字は小文字化）で 1〜63 バイト、先頭・末尾が `-` でない
- 全体 253 バイト以下
- WHATWG URL のホストとして IP アドレスではなくドメインになる（`127.0.0.1`、`0x7f.1`、`example.123` などは不可）

ドメインなら `https://{d}/.well-known/nostr.json?name=_` を取得する。リダイレクトは追わない。

| 条件 | 結果 |
|---|---|
| `names["_"]` がイベントの pubkey hex と一致（大文字小文字無視） | `Verified` |
| `_` が無い、または値が異なる | `Mismatch` |
| タイムアウト（10 秒）、非 2xx、64 KiB 超のボディ、非 UTF-8、JSON パース失敗 | `Error` |

検証は `Nip05Verify` トレイトで、テストはインメモリの fake を使う。

## クライアント

agent は `HttpNip05Verifier::public_only()`、publish は `HttpNip05Verifier::new()` を使う。`public_only()` は次の制限を加える。

- 名前解決の結果から公開アドレス以外を除き、残らなければ `Error`。除外するのは IPv4 の unspecified・loopback・private・link-local・broadcast・documentation・multicast・`0.0.0.0/8`・`240.0.0.0/4`・`100.64.0.0/10`・`198.18.0.0/15`・`192.0.0.0/24`、IPv6 の unspecified・loopback・multicast・`fc00::/7`・`fe80::/10`・`2001:db8::/32`、中身がこれらの IPv4 である IPv4-mapped アドレス。
- プロキシ環境変数を無視する。

## agent での適用

[保存の順序](agent.md#保存の順序) の事前判定の後、取得の前に行う。

| `[policy].nip05` | 動作 |
|---|---|
| `off` | 検証も記録もしない |
| `warn` | 検証して記録し、`Verified` 以外は warn ログを出して取得に進む |
| `require` | 検証して記録し、`Verified` のときだけ取得に進む |

`state.verifications` をキャッシュに使う。`checked_at` から `nip05_cache_ttl`（`error` はそれと 15 分の短い方）が経つまでは再検証せず、記録済みの `status` で判断する。`nip05_cache_ttl = 0` なら毎回検証する。記録するたびに、その pubkey の保存されていないサイトの記録を `checked_at` の新しい順に `max_sites_per_account` 件まで残して消す。
