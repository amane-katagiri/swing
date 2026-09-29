# swing publish でサイトのチェックリストを機械的に確かめる

## 背景

[IPFS で配りやすいサイトのチェックリスト](2026-09-29-site-guide.md)のうち、ドットファイル・1 版のサイズ・前回と同じ内容かどうかの 3 つは、publish する側が機械的に判定できる。手引きを読まずに publish しても、取り返しのつかない漏れ（`.git`・`.env`）や、意味の無い更新（同じ CID のイベント）を防げるようにしたい。ダッシュボードの公開画面にも同じ結果を出す。

## 決めたこと

| 決定 | 理由 |
|---|---|
| モードは NIP-05 と同じ `off`/`warn`/`require` を項目ごとに持つ（`check_dotfiles`・`check_size`・`check_unchanged`） | 項目で重みが違う。ドットファイルの漏れは公開したら取り消せない。サイズは目安にすぎず、上限はミラーする各参加者のローカル設定で決まる。同じ CID は止めても何も失わない。1 つのモードにまとめると、どれかに合わせて他が強すぎるか弱すぎる |
| 既定は `check_dotfiles = require`・`check_size = warn`・`check_unchanged = require` | 上の重みのとおり。漏れと無意味な更新は止め、サイズは知らせるだけにする |
| 設定の優先順位と置き場所は `--nip05` と同じ（CLI のフラグ・ダッシュボードのパート > 環境変数 > 設定ファイル > 既定）。4 つのモードは `publish::resolve_modes` でまとめて解釈する | 使い方を NIP-05 と揃える。CLI とダッシュボードで同じ関数を通し、不正な値の扱いを 1 か所にする |
| `Nip05Mode` を `CheckMode` に、設定の種類 `Kind::Nip05`（`"nip05"`）を `Kind::Mode`（`"mode"`）に改名した | 同じ 3 値を NIP-05 以外にも使うため。`GET /api/config` の `kind` の値も変わるが、読むのは同梱の `settings.js` だけ |
| 見逃す名前の一覧 `dotfiles_allow`（既定 `.well-known`・`.nojekyll`・`.gitkeep`・`.keep`・`.domains`）を設定に持つ。一致したセグメントの下は確かめない | `.nojekyll` のように意図して置くドットファイルで止まると、確認そのものを `off` にされてしまう。名前の一覧なら、足したいものだけ足せる。指定すると既定を置き換える（足すだけの書き方は作らない。他の一覧の設定と揃える） |
| `dotfiles_allow` の要素は `.` で始まる 1 つの名前だけ（`/` を含まない、`.`・`..` でない） | パスで書けるようにすると、どこから数えるかの規則が要る。名前なら、どの深さにあっても同じに扱える |
| ドットファイルとサイズは add と同じ辿り方（`ipfs::list_site`。`walk_root` を共有）で一覧する | 確かめた集合と追加する集合を必ず一致させる。シンボリックリンクを辿ることも、ドットファイルを含むことも add と同じになる |
| 見つけたディレクトリは 1 件と数え、その下は数えない。表示は先頭 10 件まで | `.git` の中身を並べても意味が無く、出力が埋まる |
| サイズの目安は 512 MiB の固定の値にし、設定にしない | 手引きの「1 版は数百 MB 程度まで」を数にしたもの。ミラーする側の判定とは関係しない目安なので、設定にしても合わせる相手が無い |
| サイズはファイルの大きさの合計で見る（`dag/stat` の値ではない） | add の前に判定するため。`require` で止めるなら、追加する前でないと意味が無い |
| 同じ内容かどうかは add の後、署名の前に、relay から自分のこの `d` の最新のサイトイベントを 1 件取って CID を比べる | CID は add しないと分からない（`ipfs add` を別に呼んで計算すると、ドットファイルやシンボリックリンクの扱いで publish と CID がずれうる。前回の手引きの回で確かめた）。比べる相手は手元の MFS ではなく relay にする。別のマシンから publish した版もあるため |
| 同じなら `require` は add した版を MFS から消し、署名・送信・古い版の削除をしない。CLI は `Unchanged; not published.` で終了コード 0 | CI で毎回 publish を実行しても、変わっていなければ何も起きずに成功で終わるようにする。失敗扱いにすると、CI で「変わっていないとき」を別に扱う必要が出る |
| relay から取れない・前の版が無いときは、`require` でも止めない | 確かめられないのは同じと分かったのではない。relay の不調で publish できなくなるほうが困る |
| CLI は relay に 1 回だけ接続し、確認と署名・送信に同じ接続を使う。署名アプリを待つ表示は `Nostr` 見出しの後（確認の後）に出す | 接続を 2 回張らないため。確認の結果より先に「署名アプリを待っています」が出ると、止まったときに紛らわしい |
| ダッシュボードでは、ドットファイル・サイズの `require` は NIP-05 と同じ 422（本文に `checks`）。判定の順は NIP-05 の直後 | NIP-05 と同じく add の前に止める確認で、画面側の扱いも揃えられる |
| ダッシュボードで同じ内容のため止めたときは 200 で `published: false` を返し、`created_at`・`mfs_path` は `null`、`relays`・`pruned` は空にする。`latest_published_at` は進めず、`swing:published` イベントも出さない | CLI の終了コード 0 と揃え、エラーではなく結果として見せる。公開していないので、Desktop 画面のおしらせも出さない |
| 新しい 4 つの設定はダッシュボードから編集できるようにした | `publish.nip05` と同じ扱い。どれも publish の挙動を変えるだけで、ファイルパスや待ち受けアドレスのような安全境界に関わらない |

既定の `check_dotfiles = require` と `check_unchanged = require` は、今までの挙動を変える。ドットファイルを含むディレクトリの publish は止まるようになり、同じ内容の publish はイベントを出さなくなる。互換のための処置（前の挙動に戻す既定など）は入れていない。前の挙動が要るなら `off` を設定する。

## 作ったもの

- `src/config`: `CheckMode`（`Nip05Mode` から改名）、`PublishConfig` の `check_dotfiles`・`check_size`・`check_unchanged`・`dotfiles_allow`、`DEFAULT_DOTFILES_ALLOW`、`validate_dotfile_name`。
- `src/settings`: 4 つの設定のカタログ項目（編集可）と `raw_value`。`Kind::Mode`。`swing.example.toml`・`.env.example` を生成し直した。
- `src/ipfs.rs`: `list_site`（`walk_root` の結果を相対パスと大きさにしたもの）。
- `src/publish/checks.rs`: `find_dotfiles`・`LocalChecks`（表示の行と中止のメッセージ）・`UnchangedOutcome::decide`（relay の結果と新しい CID から判定する純粋関数）。
- `src/publish.rs`: `ModeOverrides`・`resolve_modes`・`check_unchanged`、CLI の `Checks` と `Previous version` の見出し。
- `src/nostr.rs`: `RelayClient::fetch_own_latest_site`。
- `src/main.rs`: `--check-dotfiles`・`--check-size`・`--check-unchanged`。
- ダッシュボード: `POST /api/publish/upload` のパート `check_dotfiles`・`check_size`・`check_unchanged`、レスポンスの `published` と `checks`、422 の `checks`。公開画面に 3 つのセレクトと結果の行（英語・日本語）。
- ドキュメント: `architecture/cli.md`・`architecture/dashboard/http-api.md`・`architecture/dashboard/web.md`・`architecture/dashboard.md`・`architecture.md`・README・`site-guide.md`。`todo.md` から項目を消した。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通る。足したテスト: ドットファイルの判定（見逃す名前とその下、入れ子、見つけたディレクトリの下を数えない、シンボリックリンクのディレクトリを辿る、空の一覧、表示の上限）、サイズのしきい値（ちょうどは超えない）、`off` で何も見ないこと、同じ内容の判定（`require` と `warn`、前の版が無い・取れない、`off`）、設定の既定値・優先順位・不正な値・`dotfiles_allow` の置き換えと検証、`resolve_modes`、ダッシュボードの不正なパート（400）・ドットファイルの 422 と本文・`warn` なら先に進むこと。
- デモ環境（`docker/demo/demo.sh up --seed`）で確かめた。サンプルの投入（CLI の publish 13 回）はそのまま通った。
  - CLI: `.git` と `.env` と `.well-known/nostr.json` を含むディレクトリは `.env`・`.git` の 2 件を出して終了コード 1。消してからの初回は `no previous version` で公開、同じ内容の 2 回目は追加した版を消して `Unchanged; not published.`（終了コード 0）、`--check-unchanged warn` なら同じ内容でも公開。不正なモードは `invalid --check-size`。
  - ダッシュボード: `.env` を含むフォルダは 422 で、エラーの下に NIP-05・ドットファイル（`1 件: .env`）・サイズの行が出る。`.env` の無いフォルダは公開でき、同じフォルダをもう一度送ると「変わっていないので公開しませんでした」と、relay・MFS パスの行の無い結果が出る。ドットファイルを `warn` にすると「公開はしています」を添えて公開する。日本語表示でも確かめた。追加したセレクトの余白は NIP-05 のセレクトと同じ。Settings 画面の publish の欄に 4 つの設定が編集できる形で出る。
- Desktop 画面には公開のための画面が無いので、手を入れていない。
