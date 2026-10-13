# 絵文字のフィンガープリント（`src/fingerprint.rs`, `web/emoji/`, `web/emoji.svg`, `web/desktop-fingerprint.js`, `packaging/twemoji/`）

[`web.md`](web.md) の子ページ。ダッシュボードで Nostr のアカウントを見分けやすくするため、pubkey から決まる 4 つの絵文字（Twemoji の画像）を npub や名前の前に出す。

見分けるための目印で、本人の証明ではない。4 つで 32 ビットしかなく、狙ったフィンガープリントの鍵は総当たりで作れる。アカウントを確かめるのは npub（と NIP-05）。CLI の出力には出さない。

## 求め方（`fingerprint::fingerprint`）

[`../../fingerprint-spec.md`](../../fingerprint-spec.md)（第 1 版）の求め方のとおり。pubkey の SHA-256 の先頭 4 バイトを一覧の行番号に当て、コードポイント（16 進の小文字、例 `1f40b`）の配列を返す。絵文字の文字として表すとき（自分の情報のコピーボタン・`aria-label`）は、仕様のとおり各コードポイントに U+FE0F を付ける（`ui.js::emojiChar`）。

計算はサーバ側で行い、API の DTO に `fingerprint` として載せる（ダッシュボードは平文の http で LAN から開かれることがあり、そこでは `crypto.subtle` が使えない）。載せる場所は [`http-api/nostr.md`](http-api/nostr.md)（webring のノード・`referencing`・レプリカの報告者・`/api/mirror/preview`）と [`http-api/status.md`](http-api/status.md)（`/api/overview` の自分・`/api/sites` のアカウントと `/api/status` の `versions`・`sites`）。

## 一覧（`web/emoji/list.txt`）

- ちょうど 256 行。各行は `<コードポイント> <Unicode の名前>`。行の順番が番号。Rust は `include_str!` で、スプライトの生成スクリプトも同じファイルを読む。
- 仕様の絵文字の表の写しで、仕様と違ってはならない（テストで照らす）。第 1 版の表は変えない。表の選び方は仕様の参考の章にある。

## スプライト（`web/emoji.svg`）

- 一覧の各行に `<symbol id="e-<コードポイント>" viewBox="0 0 36 36">` を 1 つずつ持つ 1 枚の SVG。ID を持つ要素や `url(#…)`・`href` での参照を含まない（記号どうしで ID がぶつからない）。正方形でない元画像は 36×36 の中央に寄せる。
- 先頭の XML コメントに CC-BY 4.0 の表示（Twemoji graphics by Twitter, Inc and other contributors、ライセンスの URL、元のリポジトリとタグとコミット）を入れる。
- ほかの静的ファイルと同じく `assets/mod.rs::STATIC_ASSETS` で `/emoji.svg`（`image/svg+xml`）として配る（[`../dashboard.md#静的ファイルの配信srcdashboardassets`](../dashboard.md#静的ファイルの配信srcdashboardassets)）。同じオリジンの `<use>` で参照するので CSP の `default-src 'self'` のままで読める。
- 大きさは約 440KB（gzip で約 160KB）。

### 作り直し方（`packaging/twemoji/build-sprite.py`）

生成したスプライトはリポジトリに入れてあり、`cargo build` では作らない。一覧を変えたときや Twemoji の版を上げるときだけ実行する。

```sh
python3 -I packaging/twemoji/build-sprite.py [--archive <ダウンロード済みのアーカイブ>]
```

- jdecked/twemoji の固定したコミット（タグ `v17.0.3`）の tar.gz を GitHub から取り、固定した SHA-256 と照らす。合わなければ止まる。`--archive` を渡すとダウンロードせずにそのファイルを使う（同じく照らす）。
- `list.txt` が 256 の重複の無いコードポイントでなければ、アーカイブに無いもの・想定外のルート要素や `viewBox`・ID や参照を含むものがあれば止まる。
- 版を上げるときはスクリプトのタグ・コミット・SHA-256 を書き換え、スプライトの先頭のコメントもそれに合わせて出し直される。

## ライセンス（`web/emoji/LICENSE-twemoji.txt`）

Twemoji の画像は CC-BY 4.0。表示（作者・ライセンス・元のリポジトリ、スプライトにまとめたこと）とライセンスの全文を `LICENSE-twemoji.txt` に置き、`LICENSE-PixelMplus.txt` と同じくリリースのアーカイブ・Linux の `install.sh`・Windows のインストーラーに入れる（[`../release.md`](../release.md)・[`../install-sh.md`](../install-sh.md)・[`../installer-windows.md`](../installer-windows.md)）。ダッシュボードの画面には表示を出さない。

## 表示（`ui.js::fingerprintBadge`, `graph.js`）

- 部品は [`web.md#共通の-ui-部品`](web.md#共通の-ui-部品)、class は [`css.md`](css.md)（`swing-fingerprint`・`swing-node-emoji`）。
- 出す場所（[`views.md`](views.md)）:
  - Sites 画面: アカウントの見出し（カード・テーブル・Unfollowed）に 4 つ、Storage check の表のアカウント列に先頭の 1 つ、「ミラーに追加」の入力中に鍵ごとに 4 つ。
  - Publish 画面: 自分の情報の「フィンガープリント」の行に 4 つと、絵文字の文字としてコピーするボタン。
  - Webring 画面: グラフのノードの円の中に先頭の 1 つ、`list` 表示の各行・詳細パネルの見出しと報告者一覧に 4 つ。
- Desktop 画面: 「ミラーに追加」ダイアログの入力中に鍵ごとに 4 つ。ここだけは画面の見た目に合わせて 16×16 のドット絵にして出す（`desktop-fingerprint.js::deskFingerprint`。[`desktop/mirror-add.md`](desktop/mirror-add.md#フィンガープリントのドット絵)）。

## テスト

`fingerprint.rs` の単体テストで、一覧が 256 の重複の無い 1 コードポイントであること、一覧が仕様の表と同じであること、スプライトの記号の数が一覧と同じで全行に記号があること、仕様のテストベクタのフィンガープリントを確かめる。
