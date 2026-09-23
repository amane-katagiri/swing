# 内蔵 gateway（gateway.rs）

[`../architecture.md`](../architecture.md) の一部。Caddy 相当（ホスト名での振り分けと Kubo gateway へのプロキシ）を axum で agent プロセス内に持つ。起動・終了は `agent::run_until` の中でダッシュボードと同じ枠組み（[`agent.md`](agent.md#全体の流れ)）。managed Kubo 側の起動・設定は [`up.md`](up.md)、compose の外部 Kubo コンテナでの同等設定は [`docker.md`](docker.md#kubo-の設定)。

## 設定（`[gateway]`）

キー・環境変数・既定値は [`../architecture.md#設定と環境変数`](../architecture.md#設定と環境変数) の設定サンプルを参照。

| キー | 意味 |
|---|---|
| `listen` | 待ち受けアドレス。`off`（既定）で無効。`Listen`（`dashboard.listen` と共通の型、[`dashboard.md`](dashboard.md)） |
| `hosts` | 転送を許可する `Host` の一覧。`listen` が `off` 以外なら空はエラー（[`../architecture.md`](../architecture.md#設定と環境変数)の検証） |
| `upstream` | プロキシ先の Kubo gateway。既定は `managed` なら `http://<[kubo].gateway_listen>`、そうでなければ `http://127.0.0.1:8080` |

`hosts` は `[kubo].managed = true` のとき Kubo 自身の `Gateway.PublicGateways` にも同じ一覧が入る（[`up.md`](up.md#適用する-kubo-設定kuboapply_config)）。gateway 側の許可ホストと Kubo 側の DNSLink 配信ホストは常に同じ集合になる。

## bind と起動・終了

`agent::run_until`（[`agent.md`](agent.md#全体の流れ)）がダッシュボードと同じ手順で扱う: reconcile の前に `[gateway].listen` の `TcpListener` を bind し（失敗は起動エラー）、reconcile 後に `gateway::serve` を別タスクで `tokio::spawn` する。終了時は `gateway_token`（`shutdown` の子トークン）を cancel し、タスクを最大 5 秒（`GATEWAY_SHUTDOWN_TIMEOUT`）待つ（超えたら warn を出して待つのをやめる）。gateway のサーバタスクが先に終了（panic 等）した場合は error ログを出すだけで agent 本体は止めない。

## リクエストの扱い（`gateway::router` / `proxy`）

すべてのメソッド・パスを `fallback` ハンドラ 1 つで受ける。

1. `Host` ヘッダが無ければ空ボディの 404。
2. `extract_host`（ポートを取り除き小文字化。`[::1]:8081` のような IPv6 の `[...]` も考慮。`dashboard/guard.rs` の `extract_host` と同等のロジック）した値が `hosts` のいずれとも一致しなければ空ボディの 404。
3. 一致すれば `upstream` + 元のパス・クエリへ、元のメソッド・ボディのまま（ストリーミング）転送する。

### 転送するヘッダー・捨てるヘッダー

- hop-by-hop（`connection`、`keep-alive`、`proxy-authenticate`、`proxy-authorization`、`te`、`trailer`、`transfer-encoding`、`upgrade`）は往復とも捨てる。`Connection` ヘッダに列挙された追加のヘッダー名（例 `Connection: X-Foo`）もその都度捨てる。
- リクエスト側はさらに、クライアントが送ってきた `Forwarded` と `X-Forwarded-*` を丸ごと捨ててから、以下を付け直す（Caddy の `reverse_proxy` と同じ考え方）:
  - `Host`: 受け取った `Host` ヘッダの値をそのまま。
  - `X-Forwarded-For`: 接続元 IP（`ConnectInfo<SocketAddr>` の IP 部分）。
  - `X-Forwarded-Proto`: `http` 固定。
  - `X-Forwarded-Host`: 受け取った `Host` ヘッダの値。
- レスポンス側はステータス・ヘッダー（hop-by-hop を除く）・ボディをそのまま返す。

### エラー

upstream に接続できない・応答が返らない場合は `502 Bad Gateway`（空ボディ）。`warn!` にエラー内容と URL を出す。

### ストリーミング・TLS

リクエストボディ・レスポンスボディともストリーミングで中継し、メモリに丸ごと持たない（`reqwest::Body::wrap_stream` / `axum::body::Body::from_stream`）。TLS は実装しない。HTTPS で公開する場合は前段（Cloudflare Tunnel など）に任せる。

### HTTP クライアント（`gateway::client`）

`reqwest::Client`。リダイレクトは追わない（`Policy::none()`）、接続タイムアウト 10 秒、リクエスト全体のタイムアウトは無し（大きなファイルの配信を打ち切らないため）。

## Kubo 側との関係

内蔵 gateway は Kubo の gateway（`Addresses.Gateway`、既定 `127.0.0.1:8080`）を素通しでプロキシするだけで、許可ホストの判定以外の挙動（404/500/206 などの応答の中身）は Kubo 自身のものになる。Kubo の gateway 自体の応答の詳細（`NoFetch` の下でのブロック欠落時の挙動など）は [`docker.md#gateway`](docker.md#gateway) を参照（内蔵 gateway でも compose の Kubo コンテナでも同じ Kubo の挙動)。

Kubo は受け取った `Host` と `X-Forwarded-Host` をそのまま信じるため、Kubo の gateway ポートを直接外部に公開してはいけない。内蔵 gateway を経由させ、受け取った `X-Forwarded-*` を必ず捨てて付け直す構成にする。
