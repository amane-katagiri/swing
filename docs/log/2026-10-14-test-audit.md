# テストの監査と整理

テストが 1000 本を超えたので、テストが本当に効いているか（壊れたら落ちるか）と、何をカバーしているかを見直し、整理した。

## 調べたこと

- `cargo llvm-cov` の行カバレッジは全体で約 87%。取りこぼしの多くは起動と I/O の配線（`up.rs`・`main.rs`・`service/linux.rs`・`mirror/print.rs`・`agent/lifecycle.rs` など）だった。
- `cargo mutants` を `policy.rs`・`auth.rs` で試した。1 つの変異に 2〜3 分かかり全体（約 5000 個）は現実的でないので、途中で止めた（試した分はすべて検出された）。
- 領域ごとにテストを読み、次の 3 種類を拾った。
  - 狙った検査が無くても通ってしまうテスト
  - テストの無い重要な経路
  - 重複・環境依存・ヘルパーの重複

## 直したこと

### 壊れていても通っていたテスト

入力に別の不正が混ざっていて、狙った検査を消しても他の検査で落ちて通っていたものがあった。

- サイトイベントの kind・d タグの拒否で、CID も不正だった。
- マスコットのパックの各スキップで、スプライトも不正だった。
- ダッシュボードのキー数の上限と Content-Type の検査で、キーも不正だった。

ほかに、次のようなものもあった。

- `iter().all` で確かめていて、結果が空でも通る。
- ヘルパーが Ok と Err の両方を受け入れていて、失敗したことを確かめていない。
- managed と既定値が同じで、分岐が無くても通る。
- Content-Type を同じ表の値と比べている（トートロジー）。
- 再起動が起きたかを確かめていない。

それぞれ入力を正しくして狙った検査だけが落とすようにし、エラーの中身も照らすようにした。主なものは、検査を一時的に壊してテストが落ちることを確かめた。

relay から受け取ったイベントを手元でもう一度絞る処理（`by_authors` など）は、nostr-sdk が購読に合わないイベントを先に落とすので、relay を通したテストでは確かめられない。念のための二重の防御として、そのまま残す。

### 環境と時計への依存

- unit テストでは `config::env_var` が常に `None` を返す。シェルに `SWING_*` があっても結果が変わらない。
- テスト用の設定は、設定ファイル・`state_dir`・Kubo の repo を一時ディレクトリの下の存在しないパスに向ける。今までは相対パスの `./data` を読んでいた。
- idle タイムアウトのテストは tokio の止めた時計で動かす（dev-dependencies に `tokio/test-util` を足した）。
- 時間の余裕が無かった待ちを延ばした。プロキシの子プロセスのテストは、相手が来なければ一定時間で落ちるようにした（今までは永久に待っていた）。
- Kubo のバイナリを使う `#[ignore]` のテストは、`SWING_TEST_KUBO_BIN` が無いと panic する。`--ignored` で明示的に回したのに何も試さず通る、ということを無くした。
  - CI では回さない。
  - `kubo_integration` の既知の fixture の CID は、Kubo v0.43.1 で求めた値を定数にした（`SWING_TEST_EXPECTED_CID` は廃止）。

### 足したテスト

- publish の通し（CLI とダッシュボードの両方）
  - 署名 → 送信 → 版の保持 → 古い版の削除
  - relay が全部拒否したとき
  - 署名が失敗したとき
  - 古い版の一覧・削除が失敗したとき
  - ダッシュボードでは、変化が無いとき・保護されたパス・時計の検査
- ダッシュボード API の成功経路（サイト・状態・ミラーの一覧と追加と削除・Webring・レプリカ・公開の一覧と前の版のファイル）
- `/api/overview` とトレイの取り決め（`instance`、署名アプリの項目）
- guard（Host ヘッダが無いとき、CORP、HEAD のマーカー）
- `mirror::apply_change`（変化が無ければ出さない、上限、`created_at` の繰り上げ）
- 編集できる全設定の書き戻しの往復と、型の合わない値
- Kubo のエラー文の読み替え（`files/rm`・`ls`・`stat`・`cp`、`fetch_dag` の上限とタイムアウト）
- NIP-05 の HTTP 経路（応答の読み取りを `read_response` に切り出した。挙動は同じ）
- 本物の `fetch_reports_about`
- clap の `debug_assert`

足したテストで見つかった不具合は無い。

### 整理

- Kubo の HTTP の偽物（8 か所）と、イベント・Follow Set・アンカー広告・`VersionRecord`・設定・ダッシュボードのリクエストの組み立てを、`test_support` に集めた。
- 同じ経路を入力だけ変えて確かめていたテストを、名前付きの行を持つ表にまとめた。他のテストと同じことを確かめていたものは消した。
- service のテンプレートは、`contains` を並べるのをやめ、全文と比べる。
- 大きいテストモジュールを `foo/tests.rs`・`foo/tests/<話題>.rs` に分けた。

テスト関数は 1103 本から 894 本になった。テスト以外の行のカバレッジは、整理の前後で同じ（未カバーの行が一致）。

## 決めたこと

- JS のテストは今回入れない（todo）。
- Kubo の CI ジョブは足さない。

## 検証

- `cargo fmt --all`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo xwin clippy --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings`
- `cargo test --workspace`（871 passed、23 ignored）

## 追記: mode の設定のエラー文

設定の編集で mode の設定（`policy.nip05` など）に不正な値を入れると、エラー文にキー名が出なかった（ほかの型は `<key>: invalid ...` の形で出す）。`<key>: invalid mode: ...` の形に揃えた。

## 追記: ダッシュボードの JS のテスト

todo に残していた JS のテストを入れた。

### 作ったもの

- Node.js の組み込みのテストランナー（`node --test 'web/tests/*.test.mjs'`）と `node:assert/strict` だけを使い、npm の依存・`package.json`・バンドラは入れない。ファイルを `.mjs` にしたので `"type": "module"` も要らない。ディレクトリを渡す書き方は Node.js 22 で動かないので、コマンドは glob を渡す形にした。
- テストは `web/tests/` に置く。ダッシュボードの配信物は `src/dashboard/assets/mod.rs` の明示的な一覧なので、テストは配信も埋め込みもされない。
- `web/` のモジュールは本番のまま読み込む。読み込み時に `localStorage`・`document`・`navigator` に触るものがあるので、各テストが最初に読む `web/tests/setup.mjs` に最小限の代わりを置いた（`TZ` も `UTC` に固定）。
- 本番の JS の変更は、テストから呼ぶための `export` の追加（`normalizeManifest`・`MESSAGES`・`collectNotices`・`diffReporters`・`computeRelativePath`・`findNewFiles`・`groupByFolder`）と、`desktop-scale.js` の倍率の計算を `snapScale` に切り出したことだけ。挙動は変えていない。
- 対象: 同梱マスコットのマニフェスト（`normalizeManifest` を通ること、コマ番号が PNG の `IHDR` から読んだシートに収まること、せりふの差し込み）と `normalizeManifest` の検証規則の境目、i18n（キー集合・`{var}`・`t()` のリテラルとキーの対応表）、`desktop-updates.js`・`util.js`・`desktop-mascot-behavior.js`（`random` と時刻を固定して状態の移り変わりを見る）・`publish.js` の相対パス・`snapScale`・`fingerprintIds`。
- CI: `release` ワークフローの `Lint and test` の後に、`actions/setup-node`（Node.js 24）と JS のテストの手順を足した（`check` の付いた 3 つのターゲット）。

### 不正な Unicode の判定を JS と Rust で照らす

`web/tests/unsafe-unicode.json` を共有のテストベクタにし、JS のテストと `format.rs` のテストの両方が読む。境目のコードポイントごとに「消す・空白にする・残す」を書き、片方の集合だけを変えるとどちらかが落ちる。

照らしてみると次が食い違っていた。どちらも表示を崩す・隠すものではないので本番の挙動は変えず、ベクタの該当項目に `js`・`rust`・`difference` を書いて、それぞれの今の挙動で照らす。揃えるかどうかは todo に回した。

- U+2028・U+2029: JS は制御文字として空白にし、Rust は `is_unsafe_char` で消す（`char::is_control` に含まれないため）。
- 制御文字の連なり: JS は空白 1 つにまとめ、Rust は 1 文字ずつ空白にする。
- trim の順: JS は制御文字を空白にした直後に trim し、その後で不正な文字を消し、切り詰める。Rust は全部の後で trim する。`"‮ a"` は JS で先頭に空白が残る。
- 切り詰めの長さ: JS は UTF-16 の単位で数え、Rust はコードポイントで数える。

最後のものは JS で絵文字などのサロゲートペアを割り、孤立したサロゲートを出す不具合になる。`util.test.mjs` に `skip: 'bug: ...'` のテストとして残した（共有のベクタは serde_json が孤立したサロゲートを読めないので入れない）。

### 確かめたこと

テストが本当に落ちることを、壊してから戻して確かめた: 同梱パックのコマ番号をシートの外にする・`fps` を 0 にする・`normalizeManifest` の `fps` の上限の判定を消す・せりふに差し込めない `{title}` を入れる、`ja` のキーを 1 つ消す・`ja` の `{var}` を変える・`t()` のリテラル（三項演算子の枝を含む）とキーの対応表の値を存在しないキーにする、`util.js` の正規表現から U+061C を外す・範囲を 1 つ縮める・ベクタの期待値を変える、`is_unsafe_char` から U+00AD を外す。どれも該当するテストだけが落ちた。

### 決めたこと

- テストに要る関数は本番のモジュールから `export` する。テスト専用の別ファイルに写すと本番とずれるため。
- DOM を組み立てるコード（`graph.js` の配置・ウィンドウ・ダイアログ）はテストしない。
- `desktop-fingerprint.js` はフィンガープリントを計算しない（ID はサーバから来る）ので、`docs/fingerprint-spec.md` のテストベクタとは照らさない。

### 検証

- `node --test 'web/tests/*.test.mjs'`（Node.js 22 と 26 で 63 本、1 本は不具合の skip）
- `cargo fmt --all`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`（872 passed、23 ignored）

## 追記: 表示用のサニタイズを JS と Rust で揃える

上で見つけた食い違いと、JS で絵文字を割る不具合を直した。JS（`sanitizeDisplayText`）と Rust（`sanitize_display_text`）を次の同じ手順にした。

1. 制御文字（C0・DEL・C1・U+2028・U+2029）の連なりを空白 1 つにする。
2. 見えない書式文字を取り除く。
3. 前後の空白を削る。
4. 文字（コードポイント）数で切り、切ったら `…` を足す。

今までとの違いは次のとおり。

- JS: trim を不正な文字を消した後にした。切り詰めをコードポイントで数えるようにした（サロゲートペアを割らない）。
- Rust: 制御文字の連なりを空白 1 つにまとめる。U+2028・U+2029 を消さずに空白にする。trim を切り詰めの前にした（先頭の空白が文字数を食わない）。
- 端末やログに出す `format::Sanitized` は変えていない（1 文字ずつ空白にし、U+2028・U+2029 は消す）。

共有のベクタから `js`・`rust`・`difference` の項目をなくし、すべて `want` で照らす。`skip` にしていたテストも有効にした。

## 追記: 切り詰めを書記素で数える

表示用のサニタイズの切り詰めを、コードポイントではなく書記素（見た目の 1 文字）で数えるようにした。JS は `Intl.Segmenter`、Rust は `unicode-segmentation` を使う（トレイが既に依存していたクレートで、本体の依存に足した）。これで、国旗・結合文字（`か` と濁点）・肌の色・キーキャップを途中で割らない。

書記素 1 つには結合文字をいくつでも付けられるので、書記素の数だけで切ると長さの上限が効かない。コードポイントでも書記素の上限の 4 倍までに抑え、超える書記素の手前で切る。

`nostr::with_rejection_hint` の予算も書記素で数える。共有のテストベクタに、国旗・結合文字・肌の色・キーキャップ・結合文字を大量に付けた書記素・上限ちょうどの場合を足した。書記素の区切りは Unicode の版で少し変わるので、ベクタには版によって区切りが変わらないものだけを入れる。

ZWJ は今も見えない書式文字として消している。後で絵文字の間の ZWJ を許せば、ZWJ でつないだ絵文字は書記素 1 つとして数えられる。
