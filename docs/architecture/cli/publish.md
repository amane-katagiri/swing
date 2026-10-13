# publish（`src/publish.rs`）

[`../cli.md`](../cli.md) の子ページ。`swing publish` の引数・表示・確認のプロンプト・終了のしかた。ダッシュボードと共有する判定と処理（保護パス・ローカルの確認・時計・版・後始末・古い版の削除）の正本は [`../publish.md`](../publish.md)、ダッシュボードの公開画面からの publish は [`../dashboard/http-api/publish.md`](../dashboard/http-api/publish.md)。

`DIR` を Kubo に add して MFS に置き、サイトイベントに署名して全 relay に送り、同じサイトの古い版を MFS から消す。

## 引数

- `--site` は必須で、そのまま `d` になる。条件を満たさなければ `invalid --site` でエラー終了する。
- `--url` は任意。指定すると `url` タグになり、条件を満たさなければ `invalid --url: ...` でエラー終了する。省略すると `url` タグを付けない。
- `--title` は任意。指定すると `title` タグになる。空白のみなら付けない扱いにし、条件を満たさなければ `invalid --title: ...` でエラー終了する。
- `--message` はサイトイベントの `content` になる（省略時は空文字）。上限を超えたら何もせずに `invalid --message: must not exceed 4096 bytes (got N bytes)` でエラー終了する。
- `--note` を付けると、サイトイベントの後に[通常の投稿](../publish.md#通常の投稿)もする。`--url` が要り、無ければ clap が引数の誤りとして止める。
- `--nip05`・`--check-dotfiles`・`--check-size`・`--check-links`・`--check-unchanged` は確認のモード（`off`/`warn`/`require`）。省略時は `[publish]` の同名の設定（既定は `check_dotfiles` が `require`・`check_size` が `warn`・`check_links` が `warn`・`check_unchanged` が `require`）。5 つは表示や処理の前にまとめて解釈し、不正な値は `invalid --<フラグ名>` でエラー終了する。
- `--yes`（`-y`）は、[増えたファイルの確認](#増えたファイルの確認)を聞かずに通す。
- `--dry-run` は、何も追加・署名・送信せずに確かめるだけにする（[確かめるだけの実行](#確かめるだけの実行--dry-run)）。`--yes` は受け付けるが使わない。
- `--json` は結果を JSON で出す。`--dry-run` が要り、無ければ clap が引数の誤りとして止める（終了コード 2）。

各値の条件は[共通処理の段階の順](../publish.md#段階の順)の 1。

## 処理と表示

[共通処理の段階の順](../publish.md#段階の順)に沿って進み、各段階で見出しと結果を表示する。

1. 引数とモードを解釈し、[保護パスの拒否](../publish.md#保護パスの拒否)に当たれば何も表示せずにエラー終了する。
2. `Site: <d>`、`--url` があれば `URL:`、`--title` があれば `Title:`、`--message` があれば `Message:` を表示する。秘密鍵も署名アプリの接続情報も無ければここでエラー終了する（`Signer::require`）。
3. `--nip05` が `off` でなければ `NIP-05` 見出しの下に結果を出す（`✓ verified` / `! mismatch: ...` / `- not applicable (d is not a domain)` / `! error: ...`）。
4. `DIR` を一覧し（`--check-links` が `off` でなければ続けて[リンクの確認](../publish/links.md)のためにファイルを読み）、`--check-dotfiles`・`--check-size`・`--check-links` のどれかが `off` でなければ `Checks` 見出しの下に[ローカルの確認](../publish.md#サイトの一覧とローカルの確認)の結果を 1 行ずつ出す。
   - `off` の項目は `- dotfiles: off` / `- size: off` / `- links: off`。
   - ドットファイルは `✓ dotfiles: none` か、`! dotfiles: N found (not in [publish].dotfiles_allow)` の後に先頭 10 件のパスを字下げして並べ、残りは `… and N more` にまとめる。
   - サイズは `✓ size: <合計> (guideline 512 MiB)` か `! size: <合計> is over the 512 MiB guideline; each mirror decides based on its own limit (max_update_size, default 2 GiB)`。
   - リンクは、何も無ければ `✓ links: no problems found`（4 MiB を超えて読まなかったファイルがあれば `; N files not read (over 4 MiB)` を続ける）。あれば、止める件数があれば `! links: N found (M break on gateways)`、無ければ `! links: N found (may break on some gateways)` の後に、種別の順（[種別](../publish/links.md#種別)の表の順）に `    <参照元ファイル>: <参照> (<種別の短い説明>)`（`reserved` は `    <名前> (<説明>)`）を種別ごとに先頭 5 件まで並べ、残りは `    … and N more (<種別>)` にまとめる。続けて読まなかったファイルがあれば `    N files not read (over 4 MiB)`、最後に手引きのページの URL（[結果](../publish/links.md#結果)）を `    see <URL>` の 1 行で出す。
   - `require` の項目が引っかかったら、各項目の対処の案内を `; ` でつないでエラー終了する。リンクの案内は `links that break on gateways found: rename top-level ipfs/ipns entries, make links inside the site relative and point them at files in it, load scripts over https, or set --check-links / [publish].check_links to warn or off`。
5. Kubo の RPC クライアントを `Config::ipfs_client` で作る（[`kubo.md`](../kubo.md#rpc-クライアントの作り方configipfs_client)）。relay に接続し、以降も同じ接続を使う。以降でエラー終了するときは relay の接続を閉じてから終わる。
6. [前の版と relay の時計](../publish.md#前の版と-relay-の時計)を取り、[時計の確認](../publish.md#時計の確認)を一度先に行う。当たれば増えたファイルの確認の前に、何も add せずにエラー終了する。
7. [増えたファイルの確認](#増えたファイルの確認)。
8. `IPFS` 見出しを出し、[add と版の配置](../publish.md#add-と版の配置)を行って（ここで時計をもう一度確かめる）、`CID:`・`✓ added to <パス>`・`Size: N bytes` を出す。
9. `--check-unchanged` が `off` でなければ `Previous version` 見出しの下に[同じ内容かの確認](../publish.md#同じ内容かの確認)の結果を出す（`✓ changed from the latest version on the relays (<前の CID>)` / `! unchanged: the CID equals your latest version on the relays` / `- no previous version on the relays` / `! could not check: <理由>`）。同じで `require` なら、版を消して `✓ removed <パス>` を出し、`Unchanged; not published.` で終わる（終了コード 0）。
10. `Nostr` 見出しを出して[署名と送信](../publish.md#署名と送信)を行う。署名アプリを使っているときは、署名の前に `waiting for the signer app to sign the site event...` を表示し、返事を最大 90 秒（`signer::SIGN_TIMEOUT`）待つ。
    - relay ごとの成否（✓/✗）を表示する。断った relay が理由を返していれば `✗ <relay>: <理由>` とし、理由は `format::sanitize_display_text` で 500 文字までにする。
    - どこにも受理されなければ `no relay accepted the site event; old versions were kept` でエラー終了する。
11. `--note` があれば `Note` 見出しを出して[通常の投稿](../publish.md#通常の投稿)をする。署名アプリを使っているときは、署名の前に `waiting for the signer app to sign the note...` を表示する。relay ごとの成否をサイトイベントと同じ形で出し、どこにも受理されなければ `! no relay accepted the note`、署名や送信に失敗したら `! could not post the note: <理由>` を出す（どれも終了コードは変えない）。
12. `Old versions (keeping N)` 見出しを出して[古い版の削除](../publish.md#古い版の削除)を行い、消した版ごとに `✓ removed <パス>`、失敗は `! could not remove <パス>: <理由>`、一覧の失敗は `! could not list old versions: <理由>` を出す（どれも終了コードは変えない）。
13. `Published.` で終わる。

## 増えたファイルの確認

`New files` 見出しの下に、比べた相手を 1 行出してから結果を出す。比べ方は[前の版のファイル一覧](../publish.md#前の版のファイル一覧)。

- 1 行目は、一覧できたら `compared with your latest version on the relays (<CID>)`、前の版が無ければ `no previous version on the relays; every file counts as new`、不明なら `! previous version unavailable (<理由>); every file counts as new`（`<理由>` は `could not fetch it from the relays: ...` か `could not list <CID> in the local Kubo: ...`）。
- 増えたファイルが無ければ `✓ no new files`、あれば `! N new files` の後に、ルート直下のファイルを先に、続けてフォルダ（ファイルの親のパス全体。`a/b/` は `a/` の中に入れず 1 つのフォルダとする）ごとに `<フォルダ>/` の行とその下へさらに字下げしたファイル名を、フォルダもファイル名もバイト順で並べる（`new_files::group_by_folder`）。並べるのは先頭 `LISTED_NEW_FILES`（50）件までで、残りは `… and N more` にまとめる。
- 増えたファイルがあり `--yes` が無ければ、標準入力が端末なら `Publish with N new files? [y/N]`（1 件なら `1 new file`。以下同じ）と聞き、`y` か `yes`（大小文字無視）以外なら `cancelled; nothing was added or published` でエラー終了する。端末でなければ聞かずに `confirmation needed for N new files: review them above and rerun with --yes` でエラー終了する。

## 確かめるだけの実行（`--dry-run`）

本番と同じ引数・確認のモード・設定で、[`../publish/dry-run.md`](../publish/dry-run.md) の段階を通す。ブロックの保存・pin・MFS への書き込み・署名・relay への送信・`--note` の投稿・古い版の削除はせず、増えたファイルも聞かない。

### 表示

本番と同じ見出しの並びで、全部の段階の結果を出してから終わる（本番が止まる条件に当たっても途中で止めない）。

1. `Site:` などの見出しの行。公開鍵が分からなければ続けて `Key: unknown (<理由>)`。
2. `--nip05` が `off` でなければ `NIP-05`。公開鍵が不明なら `! could not check: your public key is unknown`。
3. `Checks`（本番と同じ）。
4. `Clock`: `✓ the new version would be dated <created_at>` か `! <時計のエラー>`。MFS の版を読めなければ `! could not check the versions in MFS: <理由>`、時計を答えた relay が無ければ `- no relay answered the clock check` を足す。
5. `New files`（本番と同じ行。聞かない）。
6. `IPFS (CID only; nothing was added)`: `CID: <CID>` か `! could not compute the CID: <理由>`。
7. `--check-unchanged` が `off` でなければ `Previous version`（本番と同じ行。CID が不明なら `! could not check: could not compute the CID: <理由>`）。
8. 判定を `Would be published.` / `Unchanged; would not be published.` / `Would stop before publishing; see the error below.` のどれかで出し、`Dry run; nothing was published.` で終わる。

### 終了コード

- 0: 本番が止まる条件に当たらない。同じ内容で `--check-unchanged` が `require` のとき（本番は `Unchanged; not published.` で正常終了する）も 0。「不明」は止める理由にしない。
- 1: 本番が止まる条件に当たる（`require` の NIP-05・ドットファイル・サイズ・リンクと、時計）。全部を集めてから、`the publish would stop: <理由>`（理由は本番のメッセージを `; ` でつないだもの）を標準エラーに出す。
- 1: 評価を始める前のエラー（`--site` などの引数の誤り・設定の読み込み・[保護パス](../publish.md#保護パスの拒否)・ディレクトリの一覧の失敗）。本番と同じメッセージを標準エラーに出し、`--json` でも JSON は出さない。
- 2: clap の引数の誤り（`--json` だけを付けた、など）。

### JSON（`--json`）

標準出力に JSON のオブジェクトを 1 つだけ出す（字下げ付き、末尾に改行）。それ以外の表示（エラーとログ）は標準エラーに出す。値が分からない項目は、`status` を `"unknown"` にして `detail` に理由を入れる。`null` は「その値が無い」（`url` を付けていない、前の版が無いなど）を表し、不明とは区別する。版が変わらない限り、フィールドの追加はあっても意味や型は変えない。

- `format`: スキーマの版（整数）。今は `1`。
- `dry_run`: 常に `true`。
- `site`・`url`・`title`・`message`: 引数の値（`title` は前後の空白を削ったもの）。無ければ `null`。`note`: `--note` を付けたか。
- `pubkey`: `{"status": "known", "hex", "npub"}` か `{"status": "unknown", "detail"}`。
- `nip05`: `mode`（`off`/`warn`/`require`）・`status`（`off`・`verified`・`mismatch`・`not_applicable`・`error`・`unknown`）・`detail`（`error`・`unknown` の理由。ほかは `null`）・`stops`（本番が止まるか）。
- `checks.dotfiles`: `mode`・`status`（`off`・`ok`・`found`）・`count`・`paths`（[ドットファイル](../publish.md#サイトの一覧とローカルの確認)の 1 件ごとのパス。全件）・`stops`。
- `checks.size`: `mode`・`status`（`off`・`ok`・`over`）・`files`（ファイルの数）・`bytes`（ファイルの大きさの合計。`off` でも出す）・`guideline`（536870912）・`stops`。
- `checks.links`: `mode`・`status`（`off`・`ok`・`found`）・`count`（参照の件数）・`breaking`（そのうち確実に崩れるもの）・`redirects`（`_redirects` があるか）・`skipped`（4 MiB を超えて読まなかったファイル）・`findings`・`stops`。`findings` の各要素は `kind`（[種別](../publish/links.md#種別)の名前。`root_relative` など）・`breaks`（確実に崩れるなら `true`、崩れるかもしれないだけなら `false`）・`file`（参照元のファイル。`reserved` は該当する名前）・`reference`（参照。`reserved` は空文字）・`files`（同じ参照があるファイルの数。外部の資源と元のサイトへの絶対 URL だけ 2 以上になる）。`off` なら `count`・`breaking` は 0、配列は空。
- `clock`: `status`（`ok`・`stops`）・`created_at`（`ok` のとき本番が使う日時。`stops` なら `null`）・`detail`（`stops` の理由）・`mfs_versions`（`{"status": "listed", "newest"}`。`newest` は MFS の最大の版で、無ければ `null`。読めなければ `{"status": "unknown", "detail"}`）・`relays_answered`（時計を答えた relay の数）。
- `ipfs`: `{"status": "computed", "cid"}` か `{"status": "unknown", "detail"}`。
- `previous`: relay 上の自分の最新版。`{"status": "found", "cid", "created_at"}`・`{"status": "none"}`・`{"status": "unknown", "detail"}`。
- `new_files`: `status`（`listed`・`no_previous`・`unknown`）・`compared_with`（`listed` のとき比べた CID）・`detail`（`unknown` の理由）・`count`・`paths`（全件）。`no_previous`・`unknown` では本番と同じく全ファイルを数える。
- `unchanged`: `mode`・`status`（`changed`・`unchanged`・`no_previous`・`unknown`。`mode` が `off` でも比べた結果を出す）・`previous_cid`・`detail`・`skips_publish`（同じ内容なので本番が公開せずに正常終了するなら `true`）。
- `outcome`: `publish`（公開される）・`unchanged`（公開されずに正常終了する）・`blocked`（止まる。終了コード 1）。
- `blockers`: 止める理由の配列。各要素は `check`（`nip05`・`dotfiles`・`size`・`links`・`clock`）と `message`（本番のエラーのメッセージ）。
- `guide_url`: 直し方の手引き（[`docs/site-guide.md`](../../site-guide.md)）の URL。

`--json` のときはログも標準エラーに出す（ほかのコマンドと `--json` の無い publish では標準出力）。
