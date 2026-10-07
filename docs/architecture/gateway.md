# 内蔵 gateway（gateway.rs）

[`../architecture.md`](../architecture.md) の一部。bind・起動・終了の順序と待ち時間は `agent::run_until` の [全体の流れ](agent.md#全体の流れ) と [シグナルと終了](agent.md#シグナルと終了)、managed Kubo 側の設定は [`kubo.md`](kubo.md#適用する-kubo-設定kuboapply_config)、compose の外部 Kubo コンテナでの同等設定は [`docker.md`](docker.md#kubo-の設定)。

ホスト名での振り分けと Kubo gateway へのプロキシを axum で agent プロセス内に持つ。許可ホストの判定以外の挙動（応答の中身）は Kubo の gateway のもので、その設定と公開の注意は [`kubo.md#kubo-の-gatewaynofetch`](kubo.md#kubo-の-gatewaynofetch)。gateway のサーバタスクが先に終了（panic 等）しても、error ログを出すだけで agent は止めない。

## 設定（`[gateway]`）

キー・環境変数・既定値は設定サンプル [`../../swing.example.toml`](../../swing.example.toml) を参照。

| キー | 意味 |
|---|---|
| `listen` | 待ち受けアドレス。`off`（既定）で無効 |
| `hosts` | 転送を許可する `Host` の一覧（検証は下記） |
| `upstream` | プロキシ先の Kubo gateway。形式の検証は [`config.md#検証`](config.md#検証)。既定は `managed` なら `http://<[kubo].gateway_listen>`、そうでなければ `http://127.0.0.1:8080` |

### `hosts` の検証

- 各要素の前後の空白を取り除き、空の要素は捨てる。
- 各要素は `a-z`・`0-9`・`.`・`-` だけからなり、先頭・末尾が `.` でなく、`..` を含まないこと（大文字は不可）。満たさなければエラー。
- `listen` が `off` 以外なら、空はエラー。
- ダッシュボードで開けるホスト名（`[dashboard].allowed_hosts` と、常に許可される `localhost`・`127.0.0.1`・`::1`、`[dashboard].listen` が `0.0.0.0`・`::` 以外ならその IP）と同じ名前（大文字小文字は区別しない）はエラー。この検証は Kubo 自身の gateway（`[kubo].gateway_listen`）には及ばない（[`dashboard/security.md#既知の弱点`](dashboard/security.md#既知の弱点)）。

`[kubo].managed = true` のときは、同じ一覧が Kubo の `Gateway.PublicGateways` にも入る（[`kubo.md#適用する-kubo-設定kuboapply_config`](kubo.md#適用する-kubo-設定kuboapply_config)）。

## リクエストの扱い（`gateway::router` / `proxy`）

すべてのメソッド・パスを `fallback` ハンドラ 1 つで受ける。

1. `Host` ヘッダが無ければ空ボディの 404。
2. `Host` を `host::split_host_port`（ダッシュボードのガードと共有。`[...]` の後ろが `:<数字>` 以外ならヘッダ全体をホスト名として扱う）でホスト名とポートに分け、ホスト名が `hosts` のいずれとも一致しなければ（大文字小文字は区別しない）空ボディの 404。
3. 一致すれば `upstream` + 元のパス・クエリへ、元のメソッド・ボディのまま（ストリーミング）転送する。

### 転送するヘッダー・捨てるヘッダー

- hop-by-hop（`connection`、`keep-alive`、`proxy-authenticate`、`proxy-authorization`、`te`、`trailer`、`transfer-encoding`、`upgrade`）は往復とも捨てる。`Connection` ヘッダに列挙された追加のヘッダー名（例 `Connection: X-Foo`）もその都度捨てる。
- リクエスト側はさらに、クライアントが送ってきた `Cookie`・`Authorization`（Kubo のゲートウェイは使わず、`upstream` が遠くの HTTP なら平文で流れてしまうため）と `Forwarded`・`X-Forwarded-*` を丸ごと捨ててから、以下を付け直す:
  - `Host`: 一致した `hosts` の要素（設定の綴り）。受け取った `Host` にポートがあれば `:<ポート>` を付ける。クライアントの `Host` の値はそのまま転送しない。
  - `X-Forwarded-For`: 接続元 IP。
  - `X-Forwarded-Proto`: `http` 固定。
  - `X-Forwarded-Host`: `Host` と同じ値。
- レスポンス側はステータス・ヘッダー（hop-by-hop を除く）・ボディをそのまま返す。

### エラー

- upstream に接続できない、またはリクエストの送信か応答ヘッダーの受信がエラーになった場合は `502 Bad Gateway`（空ボディ）。`warn!` にエラー内容と URL を出す。
- リクエストの送信を始めてから応答ヘッダーを受け取るまでが `UPSTREAM_HEADER_TIMEOUT`（60 秒）を超えたら `504 Gateway Timeout`（空ボディ）。`warn!` に待った秒数と URL を出す。ヘッダーを受け取った後のボディの中継には時間の上限を付けない（下記）。

### ストリーミング・TLS

リクエストボディ・レスポンスボディともストリーミングで中継し、メモリに丸ごと持たない。TLS は実装しない。HTTPS で公開する場合は前段（Cloudflare Tunnel など）に任せる。

### HTTP クライアント（`gateway::client`）

`reqwest::Client`。リダイレクトは追わない、プロキシ環境変数を無視する、接続タイムアウト 10 秒、リクエスト全体のタイムアウトは無し（応答ヘッダーまでの待ちは上記の `UPSTREAM_HEADER_TIMEOUT`）。
