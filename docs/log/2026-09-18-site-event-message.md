# 2026-09-18 サイトイベントの `content` に更新メモを載せる

## 問題

- サイトイベントには版の CID・URL・サイズしか無く、何を更新したのかを読者に伝える手段が無かった。`alt` を付けても「どのサイトの版か」までしか分からない。

## 決めたこと

| 決定 | 理由 |
|---|---|
| メモは新しいタグではなく `content` に入れる | Nostr クライアントは `content` を本文として表示するので、非対応クライアントでもメモが見える。既存の受信側は `content` を読んでいないので、空文字でないイベントを出しても壊れない |
| protocol.md で `content` を「更新メモ（プレーンテキスト、空でもよい）」に変え、保存の判断に使うことを禁止する | メモは作者の自由記述で、検証できる情報ではない |
| 受信側は検証せず、表示時に無害化する | 他人の書いた文字列をそのまま端末に出すと ANSI エスケープなどを仕込める。制御文字を空白に置き換え、200 文字で切る |
| CLI は `swing publish -m/--message` | `git commit -m` と同じ感覚で書ける |
| 表示先は `swing sites` の各サイトの次の行 | 1 行の固定幅表示を崩さないため別の行にする |

## 作ったもの

- `build_site_event_builder` に `message` 引数、`SiteEvent` に `message` フィールドを足した。
- `publish.rs` / `main.rs` に `--message` を足し、`Message:` を表示する。
- `mirror.rs` の `format_message_line` で無害化して表示する。
- `docs/examples/publish.sh` は 4 番目の引数をメモとして `nak event -c` に渡す。
- protocol.md、architecture.md、README を更新した。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（184 passed, 11 ignored）。
- ローカルの `scsibug/nostr-rs-relay` に対して `nostr_relay_integration`（3 passed）。往復でメモが保たれることを確かめた。
