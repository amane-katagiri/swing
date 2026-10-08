# publish（`src/publish.rs`）

[`../cli.md`](../cli.md) の子ページ。`swing publish` の引数・表示・確認のプロンプト・終了のしかた。ダッシュボードと共有する判定と処理（保護パス・ローカルの確認・時計・版・後始末・古い版の削除）の正本は [`../publish.md`](../publish.md)、ダッシュボードの公開画面からの publish は [`../dashboard/http-api/publish.md`](../dashboard/http-api/publish.md)。

`DIR` を Kubo に add して MFS に置き、サイトイベントに署名して全 relay に送り、同じサイトの古い版を MFS から消す。

## 引数

- `--site` は必須で、そのまま `d` になる。条件を満たさなければ `invalid --site` でエラー終了する。
- `--url` は任意。指定すると `url` タグになり、条件を満たさなければ `invalid --url: ...` でエラー終了する。省略すると `url` タグを付けない。
- `--title` は任意。指定すると `title` タグになる。空白のみなら付けない扱いにし、条件を満たさなければ `invalid --title: ...` でエラー終了する。
- `--message` はサイトイベントの `content` になる（省略時は空文字）。上限を超えたら何もせずに `invalid --message: must not exceed 4096 bytes (got N bytes)` でエラー終了する。
- `--note` を付けると、サイトイベントの後に[通常の投稿](../publish.md#通常の投稿)もする。`--url` が要り、無ければ clap が引数の誤りとして止める。
- `--nip05`・`--check-dotfiles`・`--check-size`・`--check-unchanged` は確認のモード（`off`/`warn`/`require`）。省略時は `[publish]` の同名の設定（既定は `check_dotfiles` が `require`・`check_size` が `warn`・`check_unchanged` が `require`）。4 つは表示や処理の前にまとめて解釈し、不正な値は `invalid --<フラグ名>` でエラー終了する。
- `--yes`（`-y`）は、[増えたファイルの確認](#増えたファイルの確認)を聞かずに通す。

各値の条件は[共通処理の段階の順](../publish.md#段階の順)の 1。

## 処理と表示

[共通処理の段階の順](../publish.md#段階の順)に沿って進み、各段階で見出しと結果を表示する。

1. 引数とモードを解釈し、[保護パスの拒否](../publish.md#保護パスの拒否)に当たれば何も表示せずにエラー終了する。
2. `Site: <d>`、`--url` があれば `URL:`、`--title` があれば `Title:`、`--message` があれば `Message:` を表示する。秘密鍵も署名アプリの接続情報も無ければここでエラー終了する（`Signer::require`）。
3. `--nip05` が `off` でなければ `NIP-05` 見出しの下に結果を出す（`✓ verified` / `! mismatch: ...` / `- not applicable (d is not a domain)` / `! error: ...`）。
4. `DIR` を一覧し、`--check-dotfiles` と `--check-size` のどちらかが `off` でなければ `Checks` 見出しの下に[ローカルの確認](../publish.md#サイトの一覧とローカルの確認)の結果を 1 行ずつ出す。
   - `off` の項目は `- dotfiles: off` / `- size: off`。
   - ドットファイルは `✓ dotfiles: none` か、`! dotfiles: N found (not in [publish].dotfiles_allow)` の後に先頭 10 件のパスを字下げして並べ、残りは `… and N more` にまとめる。
   - サイズは `✓ size: <合計> (guideline 512 MiB)` か `! size: <合計> is over the 512 MiB guideline; each mirror decides by its own limits (max_update_size, default 2 GiB)`。
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
