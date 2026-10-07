# 状態の取得（`src/dashboard/api.rs`, `src/dashboard/dto.rs`）

[`../http-api.md`](../http-api.md) の子ページ。共通の形式・エラー・準備状態は親ページを参照。

## GET /api/overview

```json
{ "version": "0.1.0", "setup": false, "pubkey": "ab12…", "npub": "npub1…", "relays": ["wss://relay.damus.io"], "mirror_set": "swing", "gateway": "http://localhost:8080", "started_at": 1790000000, "instance": "cdeee5bc85519f44", "max_upload": 2147483648, "signer": { "remote": true, "relays": ["wss://relay.primal.net"], "last_failure": { "at": 1790000100, "message": "the signer app did not answer in time; check that it is running and approve the request" } } }
```

- `gateway`: `[dashboard].gateway` が空なら `null`。
- `started_at`・`instance`: `up::run` の回ごとに `AppState` を作った時刻（Unix 秒）と、そのたびに作るランダムな 16 桁の 16 進文字列。プロセス内の再起動（`POST /api/restart`）でも変わる。Web UI は `instance` で再起動の前後を見分け、`swing stop` は同じ値を [`POST /api/identity`](session.md#post-apiidentity) から読む。
- `max_upload`: `[dashboard].max_upload` のバイト数。
- `signer`: 署名の方法。`remote` は NIP-46 の署名アプリを使っているかどうか、`relays` は署名アプリとのやりとりに使っている relay（秘密鍵なら空）、`last_failure` は署名アプリへの最後のリクエストが失敗したときの時刻とメッセージ（成功していれば・秘密鍵なら `null`。[`../../signer.md`](../../signer.md)）。
- `setup`: 鍵も署名アプリも未設定（セットアップモード。[`../../up.md#セットアップモード鍵未設定`](../../up.md#セットアップモード鍵未設定)）かどうか。そのときは `pubkey`／`npub`／`signer` も `null`。

## GET /api/activity

```json
{ "latest_stored_at": 1790000000, "latest_published_at": 1790000100, "latest_replica_report_at": 1790000200 }
```

- `latest_stored_at`: `state.json` に記録された全版の `stored_at` の最大値。版が 1 つも無い（`state.json` が無い場合を含む）と `null`。
- `latest_published_at`: 自分が publish した版の `created_at` の最大値。agent がレプリカ報告の同期で `publish/<自分>/` を一覧したとき（CLI の `swing publish` の分もここで拾う）と、ダッシュボードの publish がどこかの relay に受理されたときに進む（[`../../agent/replicas.md`](../../agent/replicas.md)）。値は下がらない。一覧がすべて成功して publish した版が 1 つも無いと分かったら `0`、まだ分かっていなければ `null`。
- `latest_replica_report_at`: 自分以外の報告者が自分のサイトについて出したレプリカ報告の `created_at` の最大値。agent が poll ごとに relay から取得して記録する（[`../../agent/replicas.md`](../../agent/replicas.md)）。値は下がらない。取得に成功して数えられる報告が 1 つも無ければ `0`、まだ一度も取得に成功していなければ `null`。
- `latest_published_at` と `latest_replica_report_at` はメモリ上の値で `state.json` には書かない。再起動後は agent の最初の poll で元の値に戻り、セットアップモードでは両方 `null`。
- relay にも Kubo にも接続しない。画面での使い方は [`../notices.md#更新の確認`](../notices.md#更新の確認)。
- agent の準備状態に関わらず応答する。`state.json` の読み込み・解釈に失敗したら 500。

## GET /api/stats

```json
{ "interval": 60, "kubo_managed": true, "samples": [ { "at": 1790000000, "swing": { "cpu_percent": 0.4, "rss_bytes": 17432576 }, "kubo": { "cpu_percent": 2.1, "rss_bytes": 251658240 }, "traffic": { "in_per_sec": 5120, "out_per_sec": 2048, "total_in": 734003200, "total_out": 104857600 } } ] }
```

- `swing up` が測って持っているリソース使用量（[`../../stats.md`](../../stats.md)）を古い順に返す。呼んでも測り直さない。
- `since`（epoch 秒、省略時 0）より後の `at` のサンプルだけを返す。数値でなければ 400。
- `interval`: 測る間隔の秒数。
- `kubo_managed`: `[kubo].managed`。`false`（外部の Kubo）なら `kubo` は常に `null`。
- 各サンプルの `swing`・`kubo`・`traffic` と、その中の `cpu_percent`・`in_per_sec`・`out_per_sec` は、取れなかったときや前回との差が出せないときに `null`（条件は [`../../stats.md#null-になる条件`](../../stats.md#null-になる条件)）。`cpu_percent` は 1 コアを 100% とする小数、ほかはバイト数（`*_per_sec` は毎秒）の整数。
- agent の準備状態に関わらず応答する（セットアップモードでも）。

## GET /api/sites

`mirror::collect_sites` の結果。集計（Follow Set の選び方、`stored_size`、レプリカ数とその取得失敗時の扱い、unfollowed、件数の上限）は [`../../mirror.md`](../../mirror.md#sites-の集計mirrorcollect_sites)。

```json
{ "follow_set": { "found": true, "note": null },
  "accounts": [ { "pubkey": "…", "npub": "…", "sites": [ { "d": "example.com", "cid": "bafy…", "url": "…", "size": 12345, "stored_size": 12300, "stored_at": 1790000100, "created_at": 1790000000, "title": "…", "message": "…", "nip05": "verified", "replicas": 3, "unverified_replicas": 0, "stored": true, "gateway_url": "…", "previous": null } ] } ],
  "replicas_error": null,
  "unfollowed": { "remove_on_unfollow": true, "accounts": [ { "...": "同じ形。ただし url・title・message・replicas・unverified_replicas は常に null、stored は常に true、stored_size は size と同じ値、previous は常に null" } ] } }
```

- `follow_set.note`: [使う Follow Set](../../mirror.md#使う-follow-set) の注記から括弧を外した文字列。無ければ `null`。
- `nip05`・`title`・`message`・`size`・`stored_size`・`stored_at` は値が無ければ `null`。`title` は作者の自己申告で受信側は信頼しない（[`protocol.md` 第 4 節](../../../protocol.md#4-サイトイベント)）。`title` と `message`（生の `content`）のサニタイズ・切り詰めはフロントの責務。
- 版ごとの重複排除込みの実測合計は `/api/status` の `sites[].actual` にしかない。
- `replicas`・`unverified_replicas`: `ReplicaCounts` の `trusted` と `unverified`（[`../../nostr.md#レプリカ報告の信頼度replicastier`](../../nostr.md#レプリカ報告の信頼度replicastier)）。レプリカ報告の取得に失敗すると全サイトで両方 `null` になり、`replicas_error` に理由が入る。
- `gateway_url`: `stored` が true かつ gateway 設定がある版だけに付く。
- `previous`: イベントの版をまだ保存しておらず、同じサイトの別の版を保存しているとき（[`../../mirror.md`](../../mirror.md#sites-の集計mirrorcollect_sites) の `previous`）の `{ "cid", "created_at", "stored_at", "stored_size", "gateway_url" }`。`gateway_url` は gateway 設定があればその版の CID で付ける。それ以外は `null`。

## GET /api/status

`health::collect_status` の結果。判定・実容量・`garbage`・`problems` の数え方は [`../../health.md`](../../health.md#status-の集計healthcollect_status)。relay には接続しない。保存量に比例して重い。

```json
{ "versions": [
    { "pubkey": "…", "npub": "…", "d": "example.com", "path": "/swing/…", "cid": "bafy…", "size": 123, "created_at": 1, "health": "ok", "detail": null },
    { "pubkey": null, "npub": null, "d": null, "path": null, "cid": "bafy…", "size": null, "created_at": null, "health": "invalid_key", "detail": "<state.json の生のキー>" } ],
  "sites": [ { "pubkey": "…", "npub": "…", "d": "example.com", "path": "/swing/…", "actual": 123 } ],
  "actual_bytes": 123,
  "garbage": [ { "path": "/swing/…", "list_failed": false, "list_failed_reason": null } ], "problems": 0 }
```

- `sites[].actual`: サイトの実容量。測れなかったサイトは `null`。`actual_bytes` はその合計で、`null` のサイトが 1 つでもあれば `null`。
- `health`: `ok`/`missing`/`cid_mismatch`/`incomplete`/`check_failed`/`invalid_key`。`ok` 以外は `detail` に理由が入る。`invalid_key` はキーが `<pubkey hex>:<d>` として読めなかった版で、`pubkey`・`npub`・`d`・`path`・`size`・`created_at` は `null`、`cid` にはその版の CID、`detail` に元のキー文字列が入る。
- 問題があっても HTTP は常に 200（`problems` の件数で分かる）。
- `garbage[].list_failed_reason`: 一覧に失敗したディレクトリ（`list_failed: true`）だけ理由の文字列が入る。
