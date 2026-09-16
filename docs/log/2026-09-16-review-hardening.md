# 2026-09-16 レビュー指摘の修正（設定値・NIP-05 の宛先・通知ストリーム）

## 目的

初期実装のコードレビューで出た次の 5 件を直す。

1. `poll_interval = 0` で `tokio::time::interval` が panic する
2. agent の NIP-05 検証で、他人のイベントの `d` に IP アドレスや内部ホスト名を入れると内部ネットワークへ GET が飛ぶ（SSRF）
3. `parse_duration_secs` の乗算オーバーフロー、`parse_size` が `inf` / `NaN` / 指数表記を受け付ける（`NaN` は 0 バイトになる）
4. 通知ストリームが `None` を返した後、5 秒待って `None` を受け取るだけのループを永遠に続ける
5. `normalize_hostname` が理由なく `pub`

## 決めたこと

| 決定 | 理由 |
|---|---|
| `poll_interval` に加えて `SWING_PIN_TIMEOUT` も 0 を拒否 | 0 だと panic はしないが、すべての pin が即タイムアウトして永遠に成功しない |
| `parse_size` の数値部分を数字と `.` に限定し、u64 を超える値を拒否 | `f64` のパースは `inf`・`NaN`・`1e3`・`+5` を通す。`as u64` は飽和・0 化して黙って別の値になる |
| IP 形式の判定は `reqwest::Url`（WHATWG ホストパーサ）に任せる | `0x7f.1` や `0177.0.0.1` も IPv4 として解釈されるため、自前で `Ipv4Addr::from_str` を使うだけでは漏れる。`url` クレートは reqwest 経由で既に入っている |
| 内部ホスト名・DNS rebinding 対策として、名前解決後のアドレスを reqwest の `dns_resolver` で絞る | ホスト名の形だけでは `kubo.internal` のような名前を区別できない。接続に使うアドレスそのものを絞れば rebinding も防げる |
| フィルタは agent 用（`public_only()`）だけに入れ、publish 用（`new()`）には入れない | publish は自分のドメインを検証するだけで、スプリット DNS で自宅サーバーが private アドレスに解決される構成を壊したくない |
| `public_only()` は `no_proxy()` | プロキシ経由だと名前解決がプロキシ側で行われ、フィルタを素通りする |
| 通知ストリームの終了（`Shutdown` / `None`）は `run` をエラーで終える | nostr-sdk のストリームは Client の shutdown でしか終わらず、終わったら再開しない。プロセスを落として compose の restart に任せる |
| NAT64（`64:ff9b::/96`）や 6to4（`2002::/16`）に埋め込まれた IPv4 は見ない | 範囲を広げるほど誤判定の余地が増える。必要になったら追加する |

## 確認したライブラリの挙動（nostr-sdk 0.45.3）

- `RelayOptions::reconnect` は既定 `true`。再試行間隔は 10 秒から最大 60 秒、ジッター ±3 秒。
- 再接続後に `resubscribe()` が走り、購読は自動で戻る。
- `Client::notifications()` は broadcast channel（容量 2048）で、`Lagged` は `NotificationStream` が黙って読み飛ばす。取りこぼしは `poll_interval` ごとの過去分取得で回収される。

## 作ったもの

- `src/config.rs`: 上記の値検証とテスト
- `src/nip05.rs`: `normalize_hostname` の強化（ラベル長・先頭末尾ハイフン・IP 形式の拒否）、`PublicOnlyResolver`、`HttpNip05Verifier::public_only()`、テスト
- `src/agent.rs`: `public_only()` を使う、通知ストリーム終了時にエラーで終了
- `Cargo.toml`: tokio に `net` feature（`lookup_host` 用）

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`
- リゾルバのテストは `localhost`（`/etc/hosts` で解決）だけを使い、外部 DNS に問い合わせない
