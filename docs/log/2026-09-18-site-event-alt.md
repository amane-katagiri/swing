# 2026-09-18 サイトイベント（35980）に NIP-31 `alt` タグを付ける

## 問題

- レプリカ報告（35981）には `alt` を付けていたが、サイトイベント（35980）には無かった。SWING に対応しないクライアントや njump のようなビューアでは、未知の kind として中身の分からないイベントに見えていた。

## 決めたこと

| 決定 | 理由 |
|---|---|
| 値は `SWING site announcement: <d>` | extensions.md の例と、レプリカ報告の `SWING replica report: <d>` に揃える |
| CID や URL は `alt` に入れない | `alt` は非対応クライアント向けの「何のイベントか」の説明で、中身は `cid` / `url` タグにある。重複させない |
| protocol.md では任意のまま | 受信側は `alt` を読まない。無くても検証に影響しない |

## 作ったもの

- `nostr::build_site_event_builder` が最後に `alt` タグを足す。
- `parses_valid_site_event` テストで `alt` の値を確かめる。
- protocol.md のサイトイベントのタグ一覧と JSON 例、extensions.md の予約表と `alt` の節、architecture.md の publish 手順、`docs/examples/publish.sh` に反映した。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（182 passed, 11 ignored）。
