# NIP-05 検証（nip05.rs）

[`../architecture.md`](../architecture.md) の一部。

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

`Error` はメッセージ文字列に加えて粗い分類（`ErrorCategory`）を持つ。reqwest のエラーはタイムアウトなら `Timeout`、接続エラーなら `Unreachable`、それ以外は `InvalidResponse` で、HTTP ステータス異常・ボディサイズ超過・非 UTF-8・JSON パース失敗は `InvalidResponse`。CLI の `swing publish` と agent（`state.verifications` の `detail`）は生のメッセージ（`VerificationResult::detail()`）を使い、ダッシュボードの公開（`POST /api/publish/upload`）の応答は分類名（`coarse_detail()`: `unreachable` / `timeout` / `invalid_response`）だけを返して生のメッセージは warn ログにだけ出す（内部ネットワークへの到達性オラクル防止）。

検証は `Nip05Verify` トレイトで、テストはインメモリの fake を使う。

## クライアント

agent と dashboard は `HttpNip05Verifier::public_only()`、CLI の `swing publish`（`--site` で指定するドメイン、オペレーター自身の入力）は `HttpNip05Verifier::new()` を使う。`public_only()` は次の制限を加える。

- 名前解決の結果から公開アドレス以外を除き、残らなければ `Error`。除外するのは IPv4 の unspecified・loopback・private・link-local・broadcast・documentation・multicast・`0.0.0.0/8`・`240.0.0.0/4`・`100.64.0.0/10`・`198.18.0.0/15`・`192.0.0.0/24`、IPv6 の unspecified・loopback・multicast・`fc00::/7`・`fe80::/10`・`fec0::/10`（site-local）・`2001:db8::/32`・`2001::/32`（Teredo）・`64:ff9b:1::/48`（ローカル用 NAT64）。IPv4 を埋め込んだ IPv6（IPv4-mapped `::ffff:0:0/96`、NAT64 `64:ff9b::/96`、6to4 `2002::/16`、IPv4-compatible `::a.b.c.d`）は埋め込まれた IPv4 を取り出して IPv4 の規則で判定する。
- プロキシ環境変数を無視する。

dashboard の `POST /api/publish/upload` はネットワーク越しに渡ってくる `site`（`d` タグ）をこの verifier で検証する（SSRF 防止）。

## agent での適用

[保存の順序](agent.md#保存の順序) の事前判定の後、取得の前に行う。

| `[policy].nip05` | 動作 |
|---|---|
| `off` | 検証も記録もしない |
| `warn` | 検証して記録し、結果に関わらず取得に進む |
| `require` | 検証して記録し、`Verified` のときだけ取得に進む |

`state.verifications` をキャッシュに使う。`checked_at` から `nip05_cache_ttl`（`error` はそれと 15 分の短い方）が経つまでは再検証せず、記録済みの `status` で判断する。`nip05_cache_ttl = 0` なら毎回検証する。`warn`・`require` のどちらでも、実際に検証して `Verified` 以外なら warn ログを出す（キャッシュで判断したときは出さない）。記録するたびに、その pubkey の保存されていないサイトの記録を `checked_at` の新しい順に `max_sites_per_account` 件まで残して消す。
