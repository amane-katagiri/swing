# リンクの確認（`src/publish/links.rs`, `src/publish/links/`）

[`../publish.md`](../publish.md#サイトの一覧とローカルの確認) の子ページ。`check_links` が `off` でないときに、サイトの一覧とそこに含まれるファイルの中身から、ゲートウェイ（特に内蔵ゲートウェイ。[`../gateway.md`](../gateway.md)）や IPFS で崩れる参照を探す判定。CLI の表示は [`../cli/publish.md`](../cli/publish.md#処理と表示)、ダッシュボードの応答は [`../dashboard/http-api/publish.md`](../dashboard/http-api/publish.md#post-apipublishupload)。

## 読むファイル

- 一覧（`ipfs::SiteListing`）にあるファイルのうち、拡張子が `.html`・`.htm`（HTML）、`.css`（CSS）、`.js`・`.mjs`（JavaScript）のもの（大小文字は区別しない）。
- 一覧したときの大きさが `LINK_SCAN_MAX_FILE`（4 MiB）を超えるものは読まずに飛ばし、飛ばした数を結果に含める。読んでいる間に大きくなって上限を超えたものも同じく飛ばす。
- 開くのは `SiteListing::files` の `SiteFile::open` で、add と同じく、一覧したときと同じファイルであることを確かめてから、ルートからシンボリックリンクを辿らずに開く（[`../mfs.md`](../mfs.md#rpc)）。開けない・入れ替わっていたときは publish をエラーで止める（add でも同じ理由で止まるため）。
- 中身は UTF-8 として読み、不正なバイトは置き換える。
- 判定は `links::evaluate` で、全ファイルのパスと大きさの一覧（`SiteEntry`）と、読むファイルのパスから中身を返す関数を受け取る（`None` なら読まなかったとして数える）。`links::scan` は `SiteListing` から一覧を作り、上の確かめ方で開いて読む関数を渡す。ダッシュボードの [`POST /api/publish/check`](../dashboard/http-api/publish.md#post-apipublishcheck) は、ブラウザから受け取った一覧と中身を渡す。読むかどうかの判定（拡張子と大きさ）は `links::scanned`。
- `swing publish` とダッシュボードの upload では、publish する前に 1 回だけ、`spawn_blocking` の中で読む（`links::scan_async`）。

## 種別

`LinkKind`。A は「確実に崩れる」もので、`require` なら 1 件でもあれば add せずに止める。B は「崩れるかもしれない」もので、`require` でも止めない。

| 種別（`name`） | 分類 | 内容 |
|---|---|---|
| `reserved` | A | サイト最上位のファイル・ディレクトリの名前が `ipfs` か `ipns`（大小文字は区別しない）。内蔵ゲートウェイでは必ず 404 になる（[`../gateway.md`](../gateway.md#コンテンツパスの判定is_content_path)） |
| `root_relative` | A | HTML・CSS 中の対象の参照が `/` で始まる（`//` は除く）。`<base href="/...">` も含める。パス形式のゲートウェイで崩れる |
| `broken` | A | HTML・CSS 中の対象の参照を解決して、一覧に無いもの（下記） |
| `insecure_script` | A | `<script src="http://...">`。CSP の `script-src` で止まる |
| `post_form` | B | `<form method>` か `<button>`・`<input>` の `formmethod` が空・`get`・`dialog` 以外。内蔵ゲートウェイは 405 を返す |
| `worker` | B | インラインスクリプトと JavaScript ファイルのコード中の `serviceWorker.register`（`serviceWorker?.register` を含む）・`new Worker(`・`new SharedWorker(`（`new self.`・`new window.`・`new globalThis.` を挟んだものを含む）。CSP の `worker-src 'none'` で止まる |
| `insecure_request` | B | インラインスクリプトと JavaScript ファイルの文字列リテラル（テンプレートリテラルを含む）中の `http://`・`ws://` の URL と、`<form action>`・`formaction` の `http://`。CSP の `connect-src`・`form-action` で止まるかもしれない |
| `external` | B | HTML・CSS の資源の読み込みが他のホスト（`http(s)://` か `//`）を指す。元のサーバーが止まると一緒に壊れる外部依存 |
| `own_site` | B | publish に `url` が渡されていて、`a`・`area` の `href` か資源の読み込みが、その URL と同じホストを絶対 URL で指す |

CSP の値は [`../kubo.md`](../kubo.md#応答に付けるヘッダー)。

### 対象の参照

- `a`・`area` の `href`（移動）。
- `link` の `href`。`rel` に `canonical` があれば対象外。`stylesheet`・`icon`・`apple-touch-icon`・`apple-touch-icon-precomposed`・`mask-icon`・`manifest`・`preload`・`modulepreload`・`prefetch`・`prerender` のどれかがあれば資源の読み込み、それ以外（`alternate` など）は `root_relative`・`broken` だけを見る。
- `script` の `src`（資源）。
- `img`・`iframe`・`frame`・`video`・`audio`・`source`・`track`・`embed` の `src`、`type` が `image` の `input` の `src`、`img`・`source` の `srcset` の各候補、`video` の `poster`、`object` の `data`（どれも資源）。
- すべての要素の `style` 属性と `<style>` の中身、CSS ファイルの中の `url(...)` と `@import`（資源）。
- `meta` は見ない（`og:url` などの絶対 URL は対象外）。
- 前後の ASCII 空白を除き、さらに中の TAB・LF・CR を取り除き、`\` を `/` にしてから判定する（http(s) の URL でのブラウザの読み方と同じ。表示はもとの綴りで、制御文字は `\n`・`\u{1b}` のように書く）。空・`#` で始まる・スキーム付き（`http(s):` 以外。`mailto:`・`data:`・`javascript:`・`tel:`・`blob:` など）のものは対象外。`http(s)://` と `//` は `external`・`own_site`・`insecure_script` だけを見る。
- `<base href>` が他のホストを指す文書では、相対の参照（`/` で始まるものを含む）はそのホストを指すものとして `external`・`own_site` だけを見て、`root_relative`・`broken` は付けない。
- `own_site` に当たる資源は `external` にしない（同じホストなら `own_site` だけ）。ホストは小文字にし、ユーザー情報・ポート・末尾の `.` を除いて比べる。

### 解決と `broken`

- 参照から `?` と `#` 以降を除き、`/` で区切った各セグメントをパーセントデコードしてから比べる（`%2F` は区切りにならず、`/` を含むセグメントのファイルは無いので `broken`。`%2e%2e` は `..` として扱う）。大文字小文字は区別する。
- 相対の参照は参照元ファイルのあるディレクトリ（HTML で `<base href>` があればその指すディレクトリ）から、`/` で始まる参照はサイトのルートから解決する。`/` で始まる参照には `root_relative` と `broken` の両方が付くことがある。
- `.` と途中の空のセグメント（`a//b` の間）は飛ばし、`..` は 1 つ上がる。ルートより上に出たら `broken`。
- 末尾が `/`・`.`・`..` のものはディレクトリとして一覧にあればよい。それ以外はファイルかディレクトリとして一覧にあればよい。パスが空（ルート）や、`?` だけの参照は常にある扱い。
- `<base href>` は文書の最初のものだけを使う。他のホストやスキーム付きを指すとき、確かでない（下記）とき、解決できないときは、その文書の相対の参照は解決しない（`root_relative`・`broken` を付けない）。
- 確かでない参照には A（`root_relative`・`broken`・`insecure_script`）を付けない。確かでないのは、属性値に、戻せない名前付き文字参照（下の表に無い名前の `&name;` と、`;` の無い `&name` のうち `amp`・`lt`・`gt`・`quot`（大文字も）で始まらず、後ろが `=` でないもの）を含むとき。HTML の名前付き文字参照の全表は持たないので、読み違えて止めないようにする。`style` 属性から拾った CSS の参照も、属性が確かでなければ同じ扱い。
- `<template>` の中（入れ子を含む）の要素の参照・`style` 属性・`<style>` も確かでない扱いにする。テンプレートは `{{src}}` や `${u}` のようにスクリプトで埋める前の形であることが多いため。中の `<base>` は使わない。生テキストにはしないので、B の種別は見る。
- サイト最上位に `_redirects` というファイルがあれば、`broken` は B と同じ扱い（表示はするが `require` でも止めない）にする。`root_relative` などほかの種別は変えない。

## スキャナ

依存を足さず、自前の小さな走査器で読む。

- HTML（`links/html.rs`）: コメントは `<!--` から `-->` か `--!>` まで（`<!-->`・`<!--->` はそこで閉じる）。`<!...>`・`<?...>`・`</` の後が英字でないものは次の `>` まで飛ばす。開始タグと終了タグの属性は引用符あり・なしの両方を読み（終了タグの属性は捨てる）、名前は小文字にし、同じ名前の 2 つ目以降は無視する（名前の重複は集合で調べる）。`script`・`style`・`textarea`・`title`・`xmp`・`iframe`・`noembed`・`noframes` の中身は対応する終了タグまで、`plaintext` の中身はファイルの終わりまで生テキストとして扱い、タグとして読まない。`script` は escaped・double-escaped の状態を追い、`<!--` の後の `<script>` から `</script>` までの間では終了しない（`<!-->`・`<!--->` は escaped に入ってすぐ戻る）。ファイルの終わりで切れたタグ・属性値（`<a href="/x` で終わるもの）は、ブラウザと同じく捨てる。`<![CDATA[` は `]]>` まで飛ばす（最初の `>` で閉じない）。`script` の中身は `type` が無い・空・`module`・`javascript`/`ecmascript` を含むときだけ JavaScript として読み、`style` の中身は `type` が無い・空・`text/css`（大小文字無視）のときだけ CSS として読む。
  - 属性値の文字参照: 数値参照（10 進・16 進、`;` は省略可。0x80〜0x9F は Windows-1252 の文字に置き換え、0 と範囲外は U+FFFD）と、`;` 付きの `amp`・`lt`・`gt`・`quot`（この 4 つは大文字も）・`apos`・`nbsp`・`colon`・`sol`・`period`・`num`・`quest`・`equals`・`percnt`・`commat`・`lpar`・`rpar`・`Tab`・`NewLine` を戻す。`;` の無い `amp`・`lt`・`gt`・`quot`（大文字も）は、後ろが英数字か `=` でなければ戻す。後ろが `=` の名前はそのまま残す。それ以外の名前付き参照はそのまま残し、その属性を確かでないとする。
- CSS（`links/css.rs`）: 先に CRLF・CR・FF を LF にそろえる。コメント `/* */` と文字列を飛ばし、識別子をエスケープを戻しながら丸ごと読んで、名前が `url`（大小文字無視）で直後が `(` なら `url(...)`（引用符あり・なし）を、`@` の後の名前が `import` なら空白とコメントを飛ばした直後の文字列を拾う（`u\72l(`・`@\69mport` も同じ）。改行で切れた文字列（bad-string）と、引用符なしの `url(` の中に途中の空白・引用符・`(`・制御文字・改行の前の `\` があるもの（bad-url）は、ブラウザと同じく捨てる。`@namespace` の URL は拾わない。CSS のエスケープ（`\` + 16 進 1〜6 桁と後ろの空白 1 つ、`\` + 文字、`\` + 改行は行の継続）を戻す。
- JavaScript（`links/js.rs`）: 行コメント・ブロックコメント・文字列・テンプレートリテラル（`${ }` の入れ子を含む）・正規表現リテラル（直前の記号やキーワードで割り算と見分ける）を区別する。`worker` はコードの部分だけ、`insecure_request` は文字列リテラルの中だけを見る。文字列はエスケープされていない LF・CR で終わり、`\` + LF・CRLF・CR は行の継続として文字列のまま読む。文字列中の `\/` は `/` として読む。URL は区切り（空白・引用符・括弧など）までを 1 つとし、次はその後ろから探す。`http://`・`ws://` の直前が英数字なら URL とみなさず、直後が英数字か `[` でなければ（`"http://"` だけなど）数えない。`http://www.w3.org/`・`http://json-schema.org/` で始まるもの（名前空間）と、ホストにドットが無いもの（`new URL(p, "http://n")` のような置き場所。`localhost` と `[...]` は除かない）は数えない。`on*` 属性と `javascript:` の URL は読まない。
- `srcset` は HTML の規則どおり、空白までを URL とし、末尾の `,` を除く（`a.png,b.png` は 1 つの URL）。記述子が正しくない候補（負の密度、先頭の `+`、同じ種類の重複、`w` と `x` の併用、`0w`、`w` の無い `h`、大文字や知らない単位）は捨てる。

## 結果

`LinkReport`。

- 種別ごとに、参照元ファイル（`reserved` では名前）と参照（空白を除いたもとの綴り。160 文字を超えたら切って `…` を付ける）を持つ。並びは増えたファイルの一覧（[`../cli/publish.md`](../cli/publish.md#増えたファイルの確認)）と同じで、参照元を（フォルダのパス, ファイル名）で比べ、最上位のファイルが先、フォルダどうしはフォルダのパスの順、同じフォルダの中はファイル名の順にする。比べ方は Rust の文字列の比較（UTF-8 のバイト順）。同じファイルの中は出てきた順のまま。CLI・ダッシュボード（公開前の確認と結果）は、この順の先頭から切り出して並べる。同じファイルの同じ参照（切る前の綴りで比べる）は種別ごとに 1 回だけ数える。`external`・`own_site` は参照ごとに 1 件にまとめ、参照元のうち上の順で最初のファイルと参照元の数（`files`）を持つ（同じ外部の URL は全ページの雛形に入っていることが多いため）。CLI は 2 つ以上なら `<ファイル> and N other files: <参照>` と出す。
- `blocking` は `require` で止める件数（A の件数。`_redirects` があれば `broken` を除く）。`abort_reason` は 1 件以上あれば止める理由を返す。
- 表示や応答に並べるのは種別ごとに先頭 `LISTED_LINKS`（5）件。
- 1 件以上あれば、直し方の手引きとして [`../../site-guide.md`](../../site-guide.md) のページへの URL を 1 つだけ付ける。URL は `links::SITE_GUIDE_URL`（公開リポジトリの `main` の `docs/site-guide.md`）だけにあり、見出しのアンカーは付けない。

## 見つけないもの

- JavaScript が実行時に組み立てる URL、`fetch` などで読む先のファイルがあるかどうか、`on*` 属性の中身。
- SVG の `use`・`image` などの `href`・`xlink:href`（インラインの SVG の中でも `<a href>` は `a` として見る）、`meta http-equiv="refresh"`、`<form action>` の行き先のファイル。
- `//` で始まるスクリプトが http で開いたときに `script-src` で止まること（`external` としてだけ出す）。
- HTML・CSS・JavaScript 以外のファイル（`.svg`・`.json`・`.webmanifest` など）の中の参照。`.xhtml` も読まない（XML として読まれ、`<script src="a.js"/>` のような自己終了や文字参照の規則が HTML と違うため）。
- 4 MiB を超えるファイルの中身。
