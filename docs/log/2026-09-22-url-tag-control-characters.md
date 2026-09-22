# url タグの制御文字

## きっかけ

自己申告のイベント内容を信用しすぎている箇所の監査で見つかった。`d` と `title` はパース時に制御文字を拒否し（`validate_d_tag` / `valid_title`）、`content` は CLI 表示の直前に `sanitize_display_text` を通すのに、`url` だけはどちらも無かった。

`valid_http_url` は長さと `reqwest::Url::parse` の成否とスキームしか見ない。`Url::parse` は `\x1b` や `\x07` を含む文字列でも成功する（自分の直列化ではパーセントエンコードするだけで、エラーにはしない）。そして `parse_site_event` は正規化後の URL ではなく元のタグ文字列を `SiteEvent::url` に入れるので、制御文字がそのまま残る。`mirror.rs` の `format_site_line` は `row.url` を無加工で出力するため、フォローしている作者が `https://x.example/\x1b]0;pwned\x07` のような `url` を署名して流すと、`swing mirror sites` を打った運用者の端末でウィンドウタイトルの書き換えや画面消去が起きる。

## やったこと

`valid_http_url` に制御文字の拒否を足した。`d` と `title` と同じく `char::is_control` で判定し、含んでいれば `url` タグだけを無視する（イベント全体は受理する）。判定を入口に置いたので表示側の変更は要らない。`url` は `state.json` には保存されず、CLI もダッシュボードも毎回 relay のイベントからパースし直すので、既存データの扱いを考える必要も無い。

`swing publish --url` も `valid_http_url` で検証しているので、自分が出すイベントに制御文字入りの URL を付けることもできなくなる。

`protocol.md` 第 4 節の `url` に「制御文字を含んではならない（MUST NOT）」、第 5 節に「制御文字を含む場合も `url` だけを無視する（SHOULD）」を足した。

## 検証

- `src/nostr.rs` に、`Url::parse` が成功する制御文字入りの URL がパース後に `None` になるテストを足した。
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` を通した。

## 見送ったこと

- `format_site_line` で `url` も `sanitize_display_text` に通す二重防御。入口で拒否している値を表示側でもう一度ふるうと、どちらが本当の判定か分かりにくくなるのでやらなかった。
- 空白や非 ASCII の扱い。`Url::parse` が通す範囲のものは端末に害が無いので、`d` / `title` と同じ「制御文字だけ」に揃えた。
