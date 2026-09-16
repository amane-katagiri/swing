# 2026-09-17 ローカル Gateway と gateway プロファイル

## 目的

保存したサイトを手元のブラウザで見られるようにし、自分のサイトを決めたホスト名だけで外に配信できるようにする。TLS は Cloudflare Tunnel など compose の外で扱う。

## 決めたこと

| 決定 | 理由 |
|---|---|
| Kubo の 8080 を既定で `127.0.0.1` に公開する | 手元で見るだけならプロファイルを分けるほどの危険がない |
| `Gateway.NoFetch=true` を常に設定する | Gateway 経由でネットワークから取りに行かせない。agent の取得は RPC の `dag/export` なので影響しない |
| 外への配信は Caddy を挟む | boxo の `hostname.go` は `Host` と `X-Forwarded-Host` をそのまま信じ、`PublicGateways` に無いホストはパス Gateway にフォールバックする。Kubo 単体ではホストを絞れない |
| Caddy は HTTP のみ（`auto_https off`、`admin off`） | TLS は前段の Cloudflare Tunnel で終わる。cloudflared の設定は compose の外で持つ |
| 配信は DNSLink で、ホストは `SWING_GATEWAY_HOSTS`（カンマ区切り）で複数指定 | `Paths: []` にしてそのホストでは DNSLink の内容だけを返す。`NoDNSLink` は全体で切り、指定ホストだけ有効にする |
| Caddy の `host` マッチャには entrypoint で空白区切りに変換して渡す | Caddyfile の `{$VAR}` は展開後に字句解析されるので、空白区切りなら複数トークンになる。カンマはそのまま 1 トークンになる |
| ホスト名は小文字英数字・`-`・`.` に限る | 起動スクリプトで JSON を組み立てるため。不正なら Kubo を起動しない |
| Caddy は `2.11.4-alpine` に固定する | Kubo と同じく、確かめた版で動かす |

一覧ページ（ブラウザ上の Nostr クライアントで Follow Set とサイトイベントを集めてリンクを出す）も検討したが、今回は作っていない。relay に残るのは最新版だけで、手元にある古い版の CID をブラウザから知る手段が無い点が課題として残る。

## 検証

`IPFS_PROFILE=test` の Kubo 0.43.1 と Caddy 2.11.4 を別プロジェクト名で起動して確かめた。

- ローカル: `127.0.0.1` のパスで MFS に置いた CID が 200。`localhost` は `<cid>.ipfs.localhost` へ 301。ローカルに無い CID は 404（`block was not found locally (offline)`）。`only-if-cached` は有れば 200、無ければ 412。
- Caddy: 許可していないホスト、`Host: localhost` は 404。許可したホストでも `/ipfs/<cid>`、`/routing/v1/...`、`X-Forwarded-Host: 127.0.0.1` や `<cid>.ipfs.localhost` を付けたリクエストは 404。
- Caddy を通さず Kubo に `Host: example.test` と `X-Forwarded-Host: 127.0.0.1` を送ると、ローカルのサイトが 200 で返った。Caddy が必要な根拠。
- DNSLink: `docs.ipfs.tech` を許可すると `/ipns/docs.ipfs.tech/...` として解決され、ローカルに無いので取得せずエラーになった（bitswap の wantlist は空）。
- ブロックが無いときの 404 と 500 の違いを boxo 0.43.0 のソースと実機で確かめた。オフラインの fetcher（`fetcher/impl/blockservice/fetcher.go`）が `ipld.ErrNotFound` を `traversal.SkipMe` に置き換え、`gateway/errors.go` の `isErrNotFound` がそれを not found と見なさないので 500 になる。ルートだけの解決は fetcher を通らないので、パス Gateway の `/ipfs/<cid>/` だけが 404。サブドメインと DNSLink は `handleWebRequestErrors` から `_redirects` を探しにサブパスを辿るので、ルートでも 500。ファイルの途中のブロックが無いと 200 の後で本文が途切れる（curl exit 18）。`only-if-cached` は `IsCached` がパスの最後のブロックしか見ないので、DAG が欠けていても 200。
- `SWING_GATEWAY_HOSTS` が空だと Caddy が、不正な名前（大文字、`"`、`..`、先頭 `.`）だと Kubo が起動しない。カンマ区切りの 2 ホストはどちらも Kubo まで届き、3 つ目は Caddy で 404。
