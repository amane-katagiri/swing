# 内蔵 gateway（gateway.rs）

[`../architecture.md`](../architecture.md) の一部。ホスト名での振り分けと Kubo gateway へのプロキシを axum で agent プロセス内に持つ。bind・起動・終了の順序と待ち時間は `agent::run_until` の [全体の流れ](agent.md#全体の流れ) と [シグナルと終了](agent.md#シグナルと終了)。gateway のサーバタスクが先に終了（panic 等）しても、error ログを出すだけで agent は止めない。managed Kubo 側の設定は [`kubo.md`](kubo.md#適用する-kubo-設定kuboapply_config)、compose の外部 Kubo コンテナでの同等設定は [`docker.md`](docker.md#kubo-の設定)。

## 設定（`[gateway]`）

キー・環境変数・既定値は設定サンプル [`../../swing.example.toml`](../../swing.example.toml) を参照。

| キー | 意味 |
|---|---|
| `listen` | 待ち受けアドレス。`off`（既定）で無効。型は `Listen`（`off` または `SocketAddr`） |
| `hosts` | 転送を許可する `Host` の一覧。`listen` が `off` 以外なら空はエラー（[`../architecture.md`](../architecture.md#設定と環境変数)の検証）。ダッシュボードで開けるホスト名（`[dashboard].allowed_hosts` と、常に許可される `localhost`・`127.0.0.1`）と同じ名前はエラー。cookie はポートを区別しないので、peer のサイトの HTML がダッシュボードと同じサイトとして扱われないようにする |
| `upstream` | プロキシ先の Kubo gateway。既定は `managed` なら `http://<[kubo].gateway_listen>`、そうでなければ `http://127.0.0.1:8080` |

`hosts` は `[kubo].managed = true` のとき Kubo 自身の `Gateway.PublicGateways` にも同じ一覧が入る（[`kubo.md#適用する-kubo-設定kuboapply_config`](kubo.md#適用する-kubo-設定kuboapply_config)）。そのため managed では、gateway 側の許可ホストと Kubo 側の DNSLink 配信ホストが同じ集合になる。

## リクエストの扱い（`gateway::router` / `proxy`）

すべてのメソッド・パスを `fallback` ハンドラ 1 つで受ける。

1. `Host` ヘッダが無ければ空ボディの 404。
2. `Host` を `host::split_host_port`（ダッシュボードのガードと共有。`[...]` の後ろが `:<数字>` 以外ならヘッダ全体をホスト名として扱う）でホスト名とポートに分け、ホスト名が `hosts` のいずれとも一致しなければ（大文字小文字は区別しない）空ボディの 404。
3. 一致すれば `upstream` + 元のパス・クエリへ、元のメソッド・ボディのまま（ストリーミング）転送する。

### 転送するヘッダー・捨てるヘッダー

- hop-by-hop（`connection`、`keep-alive`、`proxy-authenticate`、`proxy-authorization`、`te`、`trailer`、`transfer-encoding`、`upgrade`）は往復とも捨てる。`Connection` ヘッダに列挙された追加のヘッダー名（例 `Connection: X-Foo`）もその都度捨てる。
- リクエスト側はさらに、クライアントが送ってきた `Forwarded` と `X-Forwarded-*` を丸ごと捨ててから、以下を付け直す:
  - `Host`: 一致した `hosts` の要素（設定の綴り）。受け取った `Host` にポートがあれば `:<ポート>` を付ける。クライアントの `Host` の値はそのまま転送しない。
  - `X-Forwarded-For`: 接続元 IP（`ConnectInfo<SocketAddr>` の IP 部分）。
  - `X-Forwarded-Proto`: `http` 固定。
  - `X-Forwarded-Host`: `Host` と同じ値。
- レスポンス側はステータス・ヘッダー（hop-by-hop を除く）・ボディをそのまま返す。

### エラー

- upstream に接続できない、またはリクエストの送信か応答ヘッダーの受信がエラーになった場合は `502 Bad Gateway`（空ボディ）。`warn!` にエラー内容と URL を出す。
- リクエストの送信を始めてから応答ヘッダーを受け取るまでが `UPSTREAM_HEADER_TIMEOUT`（60 秒）を超えたら `504 Gateway Timeout`（空ボディ）。`warn!` に待った秒数と URL を出す。ヘッダーを受け取った後のボディの中継には時間の上限を付けない（下記）。

### ストリーミング・TLS

リクエストボディ・レスポンスボディともストリーミングで中継し、メモリに丸ごと持たない（`reqwest::Body::wrap_stream` / `axum::body::Body::from_stream`）。TLS は実装しない。HTTPS で公開する場合は前段（Cloudflare Tunnel など）に任せる。

### HTTP クライアント（`gateway::client`）

`reqwest::Client`。リダイレクトは追わない（`Policy::none()`）、接続タイムアウト 10 秒、リクエスト全体のタイムアウトは無し。応答ヘッダーまでの待ちは `proxy` 側で `tokio::time::timeout(UPSTREAM_HEADER_TIMEOUT, …)` で区切る（上記）。

## Kubo 側との関係

内蔵 gateway は Kubo の gateway（`Addresses.Gateway`、既定 `127.0.0.1:8080`）を素通しでプロキシするだけで、許可ホストの判定以外の挙動（応答の中身）は Kubo 自身のものになる。Kubo の gateway の設定と公開の注意は [`kubo.md#kubo-の-gatewaynofetch`](kubo.md#kubo-の-gatewaynofetch) を参照。
