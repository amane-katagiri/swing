# 絵文字のフィンガープリント（`src/fingerprint.rs`, `web/emoji/`, `web/emoji.svg`, `packaging/twemoji/`）

[`web.md`](web.md) の子ページ。ダッシュボードで Nostr のアカウントを見分けやすくするため、pubkey から決まる 4 つの絵文字（Twemoji の画像）を npub や名前の前に出す。

見分けるための目印で、本人の証明ではない。4 つで 32 ビットしかなく、狙ったフィンガープリントの鍵は総当たりで作れる。アカウントを確かめるのは npub（と NIP-05）。CLI の出力には出さない。

## 求め方（`fingerprint::fingerprint`）

1. pubkey（x-only の 32 バイト）の SHA-256 を取る。
2. ダイジェストの先頭 4 バイト（`fingerprint::LENGTH`）を、それぞれ 0〜255 の番号として一覧（`web/emoji/list.txt`）の行に当てる。
3. その行のコードポイント（16 進の小文字、例 `1f40b`）を順に並べた配列を返す。

計算はサーバ側で行い、API の DTO に `fingerprint` として載せる（ダッシュボードは平文の http で LAN から開かれることがあり、そこでは `crypto.subtle` が使えない）。載せる場所は [`http-api/nostr.md`](http-api/nostr.md)（webring のノード・`referencing`・レプリカの報告者）と [`http-api/status.md`](http-api/status.md)（`/api/sites` のアカウントと `/api/status` の `versions`・`sites`）。

## 一覧（`web/emoji/list.txt`）

- ちょうど 256 行。各行は `<コードポイント> <Unicode の名前>`。行の順番が番号で、フィンガープリントの正本。Rust は `include_str!` で、スプライトの生成スクリプトも同じファイルを読む。
- 一度出した後は順番を変えない（変えると全員のフィンガープリントが変わる）。入れ替えるときも行の位置は保つ。
- 選び方:
  - Unicode 11 までの絵文字で、1 コードポイントだけのもの（ZWJ の組み合わせ・肌の色・国旗・キーキャップは入れない）。基本は `Emoji_Presentation=Yes` のものから選び、形で見分けられる記号（❤・❄・☀・♠ など）と一部の物（🐿・🛋 など）は既定がテキスト表示でも入れる。
  - 人の顔・手・人物は入れない。動物・植物・食べ物・道具・乗り物・天気・記号などの、形の違いがはっきりしたもの。
  - 状態・警告・セキュリティの合図に見えるもの（鍵・錠・ベル・信号機・工事・リンク・チェック・禁止・矢印・星・💤 など）や、法や裁きを連想させるもの（⚖）は入れない。npub のコピーや NIP-05 のバッジの隣で、状態の表示と取り違えられるため。
  - 同じものの変種（木・山・球技・車・靴・帽子・楽器・パン・丼・赤い果物・似た鳥・画面や箱・時計・紙・工具・光学機器など）は 1 つ、形がはっきり違うときだけ 2 つまで。色だけ違う記号（色違いの丸・四角・ハート）は 1 つも入れないか、1 つだけ（ハートは ❤ だけ）。16px で形が似る組（スポンジとチーズ、雲と煙など）も片方だけ。
  - 宗教・政治・星座にかかわるもの、お金、暴力・酒・薬・きわどい連想・不吉な連想のあるものは入れない。フィンガープリントは自分では選べないので、苦手な人の多い虫（🐜・🦗・🦂・🐛・🕷・🦟 など）や微生物も入れない。

## スプライト（`web/emoji.svg`）

- 一覧の各行に `<symbol id="e-<コードポイント>" viewBox="0 0 36 36">` を 1 つずつ持つ 1 枚の SVG。ID を持つ要素や `url(#…)`・`href` での参照を含まない（記号どうしで ID がぶつからない）。正方形でない元画像は 36×36 の中央に寄せる。
- 先頭の XML コメントに CC-BY 4.0 の表示（Twemoji graphics by Twitter, Inc and other contributors、ライセンスの URL、元のリポジトリとタグとコミット）を入れる。
- ほかの静的ファイルと同じく `assets.rs::STATIC_ASSETS` で `/emoji.svg`（`image/svg+xml`）として配る（[`../dashboard.md#静的ファイルの配信srcdashboardassetsrs`](../dashboard.md#静的ファイルの配信srcdashboardassetsrs)）。同じオリジンの `<use>` で参照するので CSP の `default-src 'self'` のままで読める。
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
  - Sites 画面: アカウントの見出し（カード・テーブル・Unfollowed）に 4 つ、Storage check の表のアカウント列に先頭の 1 つ。
  - Webring 画面: グラフのノードの円の中に先頭の 1 つ、`list` 表示の各行・詳細パネルの見出しと報告者一覧に 4 つ。
- Desktop 画面は独自のアカウント表示を持たない。

## テスト

`fingerprint.rs` の単体テストで、一覧が 256 の重複の無い 1 コードポイントであること、スプライトの記号の数が一覧と同じで全行に記号があること、既知の pubkey のフィンガープリント（固定値）を確かめる。
