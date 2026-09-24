# 2026-09-25 ダッシュボード: 他人のイベント由来テキストの bidi/ゼロ幅文字対策

## 課題

ダッシュボードは他人の Nostr イベント（サイトの `title`・`d`・`message`、プロフィール由来の webring ラベルなど）を `textContent` 経由で挿入しているため XSS は元から無いが、Unicode の双方向制御文字（U+202A–U+202E の LRE/RLE/PDF/LRO/RLO、U+2066–U+2069 の LRI/RLI/FSI/PDI、U+200E/U+200F の LRM/RLM、U+061C の ALM）やゼロ幅・不可視文字（U+200B–U+200D、U+2060、U+FEFF）は素通りしていた。これらは表示上の文字順を反転させたり文字を隠したりできる（例: `abc` + RLO + `evil` + PDF は `abclive`\* のように見た目だけ入れ替わる）ため、なりすまし・誤読を狙った視覚的スプーフィングに使える。

\* 実際には "evil" が反転して "live" のように見え、"abc" と繋がって別の語に読める、という趣旨の例。

## 決めたこと

- **表示側だけで対応し、プロトコル・Rust 側のバリデーションは変えない**。`src/nostr.rs::validate_d_tag` / `valid_title` は Rust の `char::is_control()`（Unicode カテゴリ Cc）しか弾いておらず、これらの bidi/ゼロ幅文字（カテゴリ Cf）は意図的にせよ非意図的にせよ元から通る。プロトコルドキュメント（`docs/protocol.md`）・イベント検証を変える話ではなく、「他人の生テキストを画面に置くときにどう扱うか」という表示側だけの問題として切り分けた。
- 既存の `web/util.js::sanitizeMessage`（コントロール文字除去 + 長さ制限）を土台に、新しく `stripUnsafeUnicode`（対象の bidi/ゼロ幅文字だけを除去）と `sanitizeDisplayText`（`stripControlChars` + `stripUnsafeUnicode` を合成し、`null` を返さない版）を追加した。`sanitizeMessage` はこの `sanitizeDisplayText` の薄いラッパーに変更（空文字時は `null` を返す既存の挙動は維持）。
- 一箇所のヘルパーに集約し、他人のテキストを描画している全箇所（`ui.js`・`sites.js`・`publish.js`・`webring.js`・`graph.js`・`desktop.js`・`desktop-page.html`/`.css`）から呼ぶ形にした。オペレーター自身のフォーム入力（Publish フォームへの再入力に使う `site.d`/`title`/`url` など）には適用しない。
- 除去に加えて、対象要素には `dir="auto"` を付け、CSS 側に `unicode-bidi: isolate` を足す二段構え。ゼロ幅・双方向制御文字を除去するだけでも視覚的ななりすましは防げるが、それとは独立に「正当な RTL コンテンツ（アラビア語・ヘブライ語のタイトルなど）が前後の UI（ボタンラベルや隣接するバッジ）の並びまで巻き込んで反転させない」ことも保証しておきたかったため。`dir="auto"` だけでは周囲からの bidi 分離まではされないので、`unicode-bidi: isolate` を必ず対で使う。

## 実装したこと

- `web/util.js`: `stripUnsafeUnicode` / `sanitizeDisplayText` を追加。`sanitizeMessage` をその上に再実装。`maybeLink` もフォールバック表示（`text` 省略時に `url` 自体をテキスト表示するケース）を `sanitizeDisplayText` 経由にした。
- `web/ui.js`: `buildSiteNameRow` の `site.d`、`appendLinksAndMessage` の message 段落に `sanitizeDisplayText`/`dir="auto"` を適用。
- `web/sites.js`: テーブル表示の `site.d`（一覧・Storage check の `versions`/`sites` テーブル）、`detail`、`garbage` の `path` を `sanitizeDisplayText` 経由に。
- `web/desktop.js`: リンク一覧の行（`title` フォールバックの `site.d`、`(d)` 表示、message）とマーキー（最新更新のタイトル/`d`）を対応。
- `web/webring.js` / `web/graph.js`: webring のノードラベル（`label`）・names（`site_name_lists` 由来の `d` 一覧）を List/Graph/ASCII/Source（DOT・Mermaid）の各表示形式すべてでサニタイズ。グラフの SVG `<text>`・`aria-label`・`<title>` も同じ済みの文字列を使う。
- `web/style.css` / `web/desktop-page.css`: `.swing-site-name` / `.swing-hint` / `.swing-site-message` / `.swing-table td` / `.swing-webring-account-row` / `.swing-node-detail dd`,`h2` / `.swing-node text`、`.desk-link-title` / `.desk-link-d` / `.desk-link-message` / `#desk-marquee-text` に `unicode-bidi: isolate` を追加。`web/desktop-page.html` の `#desk-marquee-text` に `dir="auto"` を追加（JS 側の要素は `el()` 呼び出し時に `dir: 'auto'` を渡している）。
- `docs/architecture/dashboard/web.md` を更新（サニタイズ箇所とヘルパーの説明を追記）。

## 検証したこと

- `node --check` を編集した全 JS ファイルに実行、構文エラー無し。
- `cargo test -j 2`: 451 passed, 15 ignored（`every_imported_module_is_served` を含め全部緑。埋め込みアセットが最新の `web/` を含むことを確認）。Rust コードは触っていないので `cargo clippy` は対象外。
- `docker/demo/demo.sh up --seed` でデモ環境を起動し、`alice` の鍵で追加のサイトを 3 件 + d タグそのものに bidi 文字を含むサイトを 1 件、`swing publish`（`docker compose run --rm --no-deps -T -e SWING_NOSTR_SECRET_KEY=... seed publish ...`）で直接発行して検証した（`seed.sh` 自体は変更していない）。
  - `title` に RLO/PDF（`abc␞evil␜` 相当）→ Sites/Desktop で `abcevil` と表示され、反転や不可視化は起きない。
  - `message`/`title` に ZWSP（`zero​width`）→ `zerowidth` と表示。
  - `title`/`message` にアラビア語（`مرحبا` / `مرحبا بالعالم`）→ 崩れず正しく RTL 表示される。
  - `d` タグ自体に RLO/PDF（`evilsite␞reversed␜`）を仕込んだサイトを webring 経由（alice 配下）で発行 → Sites のテーブル/カード、Webring の Graph（ノードの `aria-label`/`<title>`/`<text>` を `eval` で確認）・List・ASCII・Source（DOT/Mermaid）・ノード detail パネル（見出し・`names`）、Desktop のリンク一覧・マーキー、すべてで `evilsitereversed` とだけ表示され、制御文字は残らないことを確認した。
  - ブラウザコンソールにエラー無し（`agent-browser console`）。
  - 確認後は `docker/demo/demo.sh down` で環境を破棄。
