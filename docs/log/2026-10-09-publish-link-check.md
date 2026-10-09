# swing publish でゲートウェイや IPFS で崩れるリンクを確かめる

## 背景

[サイトの手引き](../site-guide.md)の「リンクとパス」「外部への依存」の多くは、ファイルの中身を読めば機械的に分かる。特に、`/` で始まる参照（パス形式のゲートウェイで崩れる）、サイト最上位の `ipfs`・`ipns`（内蔵ゲートウェイで必ず 404）、`http://` のスクリプト（内蔵ゲートウェイの CSP で止まる）は、publish してミラーされてから気づいても直すのが遅い。[前回の確認](2026-09-29-publish-site-checks.md)と同じ形で、publish の前に確かめる 4 つ目の確認 `check_links` を足す。

## 決めたこと

| 決定 | 理由 |
|---|---|
| モードは `check_links`（`off`/`warn`/`require`、既定 `warn`）を 1 つだけ持ち、種別を「確実に崩れる（A）」と「崩れるかもしれない（B）」に分けて、`require` で止めるのは A だけにする | B（外部の資源・POST のフォーム・Worker・`http://` への通信・元のサイトへのリンク）は意図して使っていることが多く、止めると確認ごと `off` にされる。A は直すまでミラー先で確実に壊れる |
| 既定は `warn` | 既存のサイトの多くは `/` で始まる参照を持っていて、`require` を既定にすると今まで通っていた publish が止まる。まず知らせるだけにする |
| HTML・CSS・JavaScript の走査は依存を足さず自前で書く（`publish/links/` の `html.rs`・`css.rs`・`js.rs`） | 要るのはタグと属性・生テキストの要素・コメント・最低限の文字参照だけで、木を作る必要が無い。`html5ever` などは依存の木が大きく、それでも CSS と JavaScript の走査は別に要る。小さな走査器なら、何を読んで何を読まないかをテストで固定できる |
| 判定する属性と要素を固定の表にする（`a`・`area`・`link`・`script`・`img` など。`meta` は見ない） | 何を見つけ何を見逃すかを、手引きと architecture にそのまま書けるようにする |
| `link` は `rel` で資源（`stylesheet`・`icon` など）とそれ以外に分け、`canonical` は対象外、それ以外（`alternate` など）は `root_relative`・`broken` だけを見る | `canonical` は元の URL を示すのが目的で、絶対 URL や `/` で始まっていても表示は崩れない。フィードへの `alternate` が `/feed.xml` なら崩れるので見る。他のホストの `alternate`・`preconnect` は読み込みではないので `external` にしない |
| 元のサイトと同じホストを指す資源は `own_site` だけにし、`external` を重ねない | どちらも「他のホストからの読み込み」だが、同じ参照が 2 行並ぶと読みにくい。`own_site` のほうが直し方（相対パスにする）がはっきりしている |
| サイト最上位に `_redirects` があれば、`broken` を B の扱い（表示だけ）にする | Kubo は `_redirects` をサブドメイン形式と DNSLink（内蔵ゲートウェイはこれ）で効かせるので、無いパスへの参照が意図した書き換え（シングルページアプリ・移動したページ）でありうる。規則を評価して確かめるのは重く、正しく真似できる保証も無い。`root_relative` はパス形式のゲートウェイが `_redirects` を無視するので変えない |
| `/` で始まる参照には `root_relative` と、ルートから解決して無ければ `broken` の両方を付ける | 「パス形式で崩れる」と「どこで開いても崩れる」は直し方の重さが違う。両方出れば、相対パスにするだけで済むかどうかが分かる |
| パスの比べ方は、`?` と `#` 以降を除き、セグメントごとにパーセントデコードし、大文字小文字を区別する。`..` でルートより上に出たら `broken` | ゲートウェイ（UnixFS のディレクトリ）は大文字小文字を区別し、`%2F` を区切りにしない。パス形式のゲートウェイでルートより上に出ると `/ipfs/` の外を指してしまう |
| `srcset` は HTML の規則どおり空白までを 1 つの URL とする（`a.png,b.png` は 1 つ） | ブラウザと同じに読まないと、`data:` の URL のコンマで誤って分ける |
| JavaScript の `insecure_request` は文字列リテラル（テンプレートリテラルを含む）の中だけを見て、`http://` の直後にホストの文字が無いもの（`"http://"` だけ）と、`http://www.w3.org/` で始まる名前空間は数えない。`<script type="application/ld+json">` など JavaScript でない `script` は読まない | ライブラリには `startsWith("http://")` のような比較や SVG の名前空間が多く、数えると警告が埋まる。JSON-LD の `http://schema.org` も通信ではない |
| `//` で始まるスクリプトは `insecure_script` にしない（`external` だけ） | https で開けば止まらない。内蔵ゲートウェイを http で開いたときだけ止まるので、確実に崩れるとは言えない |
| 1 ファイルあたり 4 MiB（`LINK_SCAN_MAX_FILE`）を超えるものは読まずに飛ばし、飛ばした数を出す | 大きなバンドルや生成物で時間とメモリを使わないため。飛ばしたことは表示して、見落としがありうることを知らせる |
| ファイルは add と同じ `SiteListing` から、一覧したときと同じファイルであることを確かめて開く（`SiteListing::files` → `SiteFile::open`）。開けなければ publish をエラーで止める | 確かめた中身と追加する中身を一致させる。入れ替わったファイルは add でも同じ理由で止まるので、先に止めても失うものが無い |
| 1 件以上見つかったら、site-guide のページそのもの（アンカー無し）への URL を 1 つだけ出す（CLI は一覧の最後に `see <URL>` を 1 行、ダッシュボードはリンクの行の最後に「直し方」のリンクを 1 本）。URL のもとは `links::SITE_GUIDE_URL` の 1 か所で、ダッシュボードには応答の `checks.links.guide` で渡す | 警告だけでは直し方が分からない。URL を Rust 側だけに置けば、Web 側に同じ定数を持たずに済む |
| 種別ごとに site-guide の節へのアンカー付きリンクにはしない（一度そう作ってから、相談して 1 本に改めた）。最上位の `ipfs`・`ipns` については、手引きの「ファイル名」に短い段落を足した | site-guide は頭から読むチェックリストで、見出しとコードの結びつきを持ちたくない。アンカーにすると、手引きの見出しを変えるたびにコードも直す必要が出る（同じ見出しが 2 つあると GitHub が付ける `-1` にも頼ることになる） |
| CLI の `Publish` の 5 つのモードのフラグを `PublishModeArgs` にまとめ、`Box` で持つ | 1 つ足したことで clippy の `large_enum_variant` に当たったため |

後方互換のための処置は入れていない。新しい設定 `[publish].check_links` は無ければ既定の `warn` になる（ほかの設定と同じ）。

## 作ったもの

- `src/publish/links.rs`: `LinkKind`（種別・A/B・表示の説明）、`SITE_GUIDE_URL`、`LinkReport`（種別ごとの結果・`blocking`・表示の行・止める理由）、参照の解決と判定、`scan`・`scan_async`。
- `src/publish/links/html.rs`・`css.rs`・`js.rs`: 走査器。`src/publish/links/tests.rs`: 単体テスト。
- `src/ipfs/site.rs`: `SiteListing::files` と `SiteFile`（パス・大きさ・一覧と同じか確かめて開く）。
- `src/publish/checks.rs`: `LocalChecks` に `links_mode`・`links`。`lines`・`abort_message`・`all_off` に含める。
- `src/publish.rs`: `Modes`・`ModeOverrides` の `check_links`、`scan_links`、CLI の流れ。
- `src/main.rs`: `--check-links`（`PublishModeArgs`）。
- 設定: `PublishConfig::check_links`、`SWING_PUBLISH_CHECK_LINKS`、カタログ（編集可、en/ja）、`swing.example.toml`・`.env.example` を生成し直した。
- ダッシュボード: パート `check_links`、応答の `checks.links`、422 の `checks.links`、公開画面のセレクトと結果の行（英語・日本語）、`swing:publish:last` に `check_links`。
- ドキュメント: `docs/architecture/publish/links.md`（新規）、`publish.md`・`cli/publish.md`・`architecture.md`・`config.md`・ダッシュボードの `http-api/publish.md`・`views/publish.md`・`web.md`、`docs/guide/publish.md`、`docs/site-guide.md`、AGENTS.md。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通る。Windows 向けのコード（`#[cfg(windows)]` など）には触れていない。
- 単体テスト（`src/publish/links/tests.rs`）: 各種別、参照元からの相対の解決、`..` でルートより上に出るもの、パーセントエンコード（`%2F`・`%2e%2e` を含む）、大文字小文字、`srcset`、CSS の `url()`（引用符あり・なし・エスケープ）と `@import`、`style` 属性と `<style>`、コメントの中・`script`・`textarea`・`title` の中の参照を読まないこと、属性値の文字参照、`<base href>`、`http://www.w3.org/` と JSON-LD の除外、`canonical` と `meta` の除外、`_redirects` があるときの扱い、種別ごとの表示の上限、手引きのリンクが 1 行だけであること、4 MiB を超えるファイルを飛ばすこと、一覧の後に入れ替わったファイルを読まないこと。`LocalChecks`・`resolve_modes`・設定の優先順位・CLI のフラグ・ダッシュボードのパート（422 の `checks.links`・`warn` なら先へ進むこと・不正な値の 400）のテストも足した。
- デモ環境で、問題を入れた小さなサイト（最上位の `ipfs/`、`/` で始まる参照、無い画像、`http://` のスクリプト、`POST` のフォーム、Service Worker、`fetch("http://...")`、外部のフォント、元のサイトへのリンク、コメントの中の参照、`canonical`）を `swing publish --check-links require` すると、15 件（うち止めるもの 8 件）が種別ごとに並び、手引きの URL 1 行の後にエラーで止まった。コメントの中の参照と `canonical` は出なかった。
- 同じサイトをダッシュボードの公開画面から `warn` で publish すると公開され、結果のパネルのリンクの行に種別ごとの行と、最後に「直し方」のリンク（`target="_blank" rel="noopener noreferrer"`）が 1 本だけ出た。リンクは独立した行なので、`dd` の `word-break: break-all` で語の途中で折り返されることは無く、CSS は変えていない。`require` では `#publish-status` に止めた理由が出て、結果のパネルにリンクの行が出た。フォームの「リンクの確認」のセレクトは前後のセレクトと同じ余白で並び、`swing:publish:last` から選択が戻ることも確かめた。

## 外部レビューの指摘と対応

実装の後に外部のレビューを受け、指摘を 1 件ずつコードで再現条件を確かめてから直した。方針は「`require` で止める判定（A）は保守的にする」で、走査器が確信を持てない参照は、仕様どおりに読み切るより A を付けない側に倒した。どの指摘も入力例を回帰テスト（`src/publish/links/tests.rs`）にした。

| 指摘 | 対応 |
|---|---|
| 他のホストを指す `<base href>` のある文書で、`/` で始まる参照にローカルの `root_relative`・`broken` が付く | 直した。外部の base はローカルの解決より先に見て、相対の参照はそのホストを指すものとして `external`・`own_site` だけを見る |
| 属性値の文字参照の読み違い（`a&amp=b.html` を戻してしまう、`&AMP;` を戻さない、`&#128;` を U+0080 にする） | 直した。属性の中で `;` の無い参照は後ろが英数字か `=` なら戻さない、`AMP`・`LT`・`GT`・`QUOT` も戻す、数値参照の 0x80〜0x9F を Windows-1252 の文字に置き換える。名前付き参照の全表は入れず、表に無い名前の参照を含む属性は「確かでない」として A を付けない。`a&b.html` のように表に無い `;` 無しの名前も確かでない扱いになるが、誤って止めないほうを優先した |
| URL の中の TAB・LF・CR を取り除かずに `broken` にする（`im&#10;g.png`、`java&#9;script:`） | 直した。判定と解決の前に取り除き、表示はもとの綴りのまま |
| `<script>` の escaped・double-escaped の状態を見ず、`<!-- <script></script> -->` の中の終了タグで script を閉じる | 直した。状態は 3 つで済み、終了の判定に閉じるので、逃がすより正しく追うほうが単純だった |
| 終了タグの属性値にある `>` で終了タグの読み飛ばしを終える | 直した。英字で始まる終了タグは開始タグと同じ読み方で属性を消費して捨てる |
| CSS の CRLF の行の継続で文字列が途中で切れる | 直した。先に CRLF・CR・FF を LF にそろえる |
| `%2F` を含むセグメントが、復号後に `/` でつなぐと `a/b.html` に一致してしまう | 直した。`/` を含むセグメントは一覧に無い（ファイル名に `/` は使えない）ので `broken` |
| コメントを挟んだ `@import` を見逃す | 直した。`@import` の後の空白とコメントを繰り返し飛ばす |
| エスケープした `url`・`@import`（`u\72l(`、`@\69mport`）を見逃す | 直した。識別子をエスケープを戻しながら丸ごと読んでから名前を比べる。`url(` の前が識別子の文字でないかの判定もこれで要らなくなった |
| 正しくない記述子の `srcset` 候補（`-1x` など）まで A を付ける | 直した。記述子を読み、正しくない候補は捨てる。捨てる側に倒れるので保守的な向きでもある |
| `--!>` で閉じたコメントの後を読まない | 直した |
| 属性名の重複を線形に探していて、異なる属性を大量に並べると二乗の時間になる | 直した。集合で調べる。4 MiB 近い入力が普通のテスト時間で終わることをテストにした |
| 文字列リテラル中の URL を探すとき、各 `http://` から末尾まで探し直していて二乗の時間になる | 直した。見つけた URL の後ろから次を探す。表示用に切る処理も先頭から必要な分だけ数える。`http://a/` を 4 MiB 近くつないだ入力のテストを足した |
| JavaScript の文字列で CRLF の行の継続を 1 つとして読まず、後半をコードとして `worker` にする | 直した。`\` + CRLF を 1 つの継続として読み、エスケープされていない CR でも文字列を終える |
| 表示用に切ってから重複を除くので、先頭が同じ長い参照が 1 件にまとまる | 直した。重複は切る前の綴りで比べ、表示するときだけ切る |

`cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通る。画面は変えていない。

## 画面の名前

ダッシュボードの表示名を、機能から見た名前に変えた。`check_links` は「動作性の確認」（Compatibility check）、`check_unchanged` は「前回のデプロイとの差分の確認」（Diff from last deploy）、手引きへのリンクは「修正ガイド」（Fix guide）。リンクだけでなく予約名・フォーム・Worker なども見るので「リンクの確認」では中身と合わず、「同じ内容の確認」も何と比べるのかが読み取れなかったため。設定のキー・CLI の出力（`links:`）・architecture の呼び方は変えていない。

## 2 回目のレビューの指摘と対応

別のレビュアーに走査器を見てもらった（強い指摘・パニック・二乗の時間は無し）。前回と同じく、`require` で止める判定（A）は誤って止めない側に倒し、入力例を回帰テストにした。

| 指摘 | 対応 |
|---|---|
| `\` を `/` として読まず、`images\a.png` や `\\host\x` が `broken` になる | 直した。判定の前に `\` を `/` にする。http(s) の URL でのブラウザの読み方と同じで、内蔵ゲートウェイのコンテンツパスの判定も `\` を区切りにしている。`\\host\x` は `//host/x` として他のホストを指す |
| `<template>` の中の `{{src}}`・`${u}` が `broken` になる | 直した。中（入れ子を含む）の参照は確かでない扱いにして A を付けない。中の要素はスクリプトで埋めてから使う雛形であることが多い。生テキストにはせず、B は見る |
| `images//a.png` の途中の空のセグメントで `broken` | 直した。途中の空のセグメントは飛ばす（ゲートウェイの解決と同じ） |
| CSS の bad-string・bad-url を捨てずに拾う | 直した。改行で切れた文字列と、引用符なしの `url(` の中に途中の空白・引用符・`(`・制御文字・改行の前の `\` があるものは捨てる |
| `<style type="text/x-template">` を CSS として読む | 直した。`type` が無い・空・`text/css` のときだけ読む |
| ファイルの終わりで切れたタグを読む、`<![CDATA[` を最初の `>` で閉じる、`<plaintext>` を生テキストにしない | 直した。切れたタグは捨て、CDATA は `]]>` まで飛ばし（中を読まない側に倒した）、`plaintext` はファイルの終わりまで生テキストにする |
| script の中の `<!-->`・`<!--->` で escaped から戻らない | 直した |
| `.xhtml` を HTML の規則で読み、`<script src="a.js"/>` の後ろを飲み込む | `.xhtml` を対象から外した。XHTML は XML として読まれ、自己終了のほか文字参照・CDATA・名前空間の規則も HTML と違う。自己終了だけ認めても XML の規則を部分的に真似ることになり、誤って止める余地が残る。静的サイトで `.xhtml` を使うことは少ない |
| `links.md` の「インライン SVG の href は見つけない」が実装と食い違う | 文書を実装に合わせた（SVG の中でも `<a href>` は見て、`use`・`image` などは見ない） |
| `srcset` の記述子で `+1x`・`2X`・`100W` を受け付ける | 直した。単位は小文字だけ、先頭の `+` は不可 |
| B がうるさい（`http://json-schema.org/`、`new URL(p, "http://n")`、CSS の `@namespace`、同じ外部の URL が全ページで並ぶ） | 直した。`http://json-schema.org/` を名前空間に足し、ホストにドットの無い URL は置き場所とみなして数えない（`localhost` と `[...]` は通信先になりうるので残す）。`@namespace` の URL は拾わない。`external`・`own_site` は参照ごとに 1 件にまとめ、参照元の数を CLI（`<ファイル> and N other files`）・ダッシュボード（「ほか N ファイル」）・応答（`items[].files`）に出す |
| 表示する参照に改行や ESC などの制御文字が残る | 直した。表示するときだけ `\n`・`\u{1b}` のように書く（判定と重複の比較はもとの綴り） |
| `new self.Worker(`・`new window.`・`new globalThis.` を見逃す | 直した |

参考として挙がった `@font-face` の代わりの形式（`.woff2` だけを置いて `.woff` への参照が残る）は、ブラウザは読まない候補でも参照としては残っているので、`broken` で止まるのは仕様どおりとした。site-guide の「公開前のリンクの確認」に、使わない形式の参照は消すよう 1 行足した。

`cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通る。
