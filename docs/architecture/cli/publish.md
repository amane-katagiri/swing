# publish（`src/publish.rs`, `src/publish/checks.rs`, `src/publish/new_files.rs`, `src/publish/staged.rs`）

[`../cli.md`](../cli.md) の子ページ。`swing publish` の引数・確認・処理順。ダッシュボードの公開画面からの publish は [`../dashboard/http-api/publish.md`](../dashboard/http-api/publish.md)。

`DIR` を Kubo に add して MFS に置き、サイトイベントに署名して全 relay に送り、同じサイトの古い版を MFS から消す。

- `--site` は必須で、そのまま `d` になる。[`d` の条件](../nostr.md#検証)を満たさなければ `invalid --site` でエラー終了する。
- `--url` は任意。指定すると `url` タグになり、[`url` の条件](../nostr.md#検証)を満たさなければ `invalid --url: ...` でエラー終了する。省略すると `url` タグを付けない。
- `--nip05` 省略時は `[publish].nip05`。
- `--check-dotfiles`・`--check-size`・`--check-unchanged` はサイトの確認のモード（`off`/`warn`/`require`）。省略時はそれぞれ `[publish].check_dotfiles`（既定 `require`）・`check_size`（既定 `warn`）・`check_unchanged`（既定 `require`）。`--nip05` を含めた 4 つのモードは表示や処理の前にまとめて解釈し、不正な値は `invalid --<フラグ名>` でエラー終了する。
- `--title` は任意。指定すると `title` タグになる。前後の空白を削り、空になれば付けない扱いにする。256 バイトを超える、または制御文字か見えない書式文字（[`title` の条件](../nostr.md#検証)）を含む場合は `invalid --title: ...` でエラー終了する。
- `--message` はサイトイベントの `content` になる（省略時は空文字）。受け取る側の上限（`MAX_CONTENT_BYTES`、4096 バイト。[`content` の扱い](../nostr.md#検証)）を超えたら何もせずに `invalid --message: must not exceed 4096 bytes (got N bytes)` でエラー終了する。
- `--yes`（`-y`）は、増えたファイルの確認（下の 5）を聞かずに通す。
- 最初に `Site: <d>`、`--url` があれば `URL:`、`--title` があれば `Title:`、`--message` があれば `Message:` を表示する。

引数とモードを解釈した後、表示の前に、次のどちらかなら確認のモードにかかわらず何もせずにエラー終了する（`publish::refuse_protected_paths`。存在しないパスは見ない）。

- `DIR` の実体（`canonicalize`）の中に、設定ファイル・`[agent].state_dir`・`[kubo].repo` のどれかの実体がある（`DIR` そのものである場合を含む）。
- `DIR` の実体が `[kubo].repo` か `[agent].state_dir` の実体の中にある（そのものである場合を含む）。ただし `<state_dir>/upload/` の下（`upload/` そのものは除く）は止めない。設定ファイルのあるディレクトリの中は止めない。

処理順:

1. `--nip05` が `off` でなければ、`d` と自分の pubkey で NIP-05 を検証し、`NIP-05` 見出しの下に結果を表示する（`✓ verified` / `! mismatch: ...` / `- not applicable (d is not a domain)` / `! error: ...`）。`require` で `Verified` 以外（`NotApplicable` を含む）なら add せず終了する。
2. `DIR` を 1 回だけ一覧する（`ipfs::SiteListing`。シンボリックリンクを辿り、ドットファイルも含める。リンク先が `DIR` の外ならこの時点でエラー。[`../mfs.md`](../mfs.md#rpc)）。以降の確認と 6 の add はこの一覧を使うので、一覧の後に `DIR` に増えたファイルは公開しない。`--check-dotfiles` と `--check-size` のどちらかが `off` でなければ、`Checks` 見出しの下に 1 行ずつ結果を表示する（`off` の項目は `- dotfiles: off` のように出す）。
   - ドットファイル: 各パスをサイトのルートから順にセグメントごとに見て、名前が `.` で始まり `[publish].dotfiles_allow` のどれとも一致しない最初のセグメントまでを 1 件とする（ディレクトリは 1 回だけ数え、その下は見ない）。一致するセグメントはそれ自身だけを見逃し、その下は続けて見る（`.well-known/.env` は `.well-known/.env` を 1 件とする）。無ければ `✓ dotfiles: none`、あれば `! dotfiles: N found (not in [publish].dotfiles_allow)` の後に先頭 `LISTED_DOTFILES`（10）件のパスを字下げして並べ、残りは `… and N more` にまとめる。
   - サイズ: ファイルの大きさの合計（ブロックの共有やディレクトリのノードは数えない）が `SIZE_GUIDELINE`（512 MiB、固定）を超えたら（ちょうどは超えない扱い）`! size: <合計> is over the 512 MiB guideline; each mirror decides by its own limits (max_update_size, default 2 GiB)`、超えなければ `✓ size: <合計> (guideline 512 MiB)`。
   - `require` の項目が引っかかったら、項目ごとの対処の案内を `; ` でつないだメッセージで、add せずにエラー終了する。
3. Kubo の RPC クライアントを `Config::ipfs_client` で作る（managed なら `<state_dir>/kubo-api.json` のポートと秘密を使い、その API の PeerID が `<repo>/config` のものと一致しなければエラー終了。[`kubo.md`](../kubo.md#rpc-クライアントの作り方configipfs_client)）。
4. relay に接続し（以降も同じ接続を使う）、前の版（自分の pubkey・この `d` のサイトイベントのうち `parse_site_event` を通り `created_at` が未来すぎない最新 1 件）を取る。結果は 5 と 8 で使う。以降でエラー終了するときは relay の接続を閉じてから終わる。
5. 増えたファイルを確かめる（`publish::PreviousFiles`）。`New files` 見出しの下に、比べた相手を 1 行出してから結果を出す。
   - 前の版があれば、その CID を Kubo でオフラインに（ローカルにあるブロックだけで）たどってファイルのパスを一覧する（`ipfs::list_files_local`。[`../mfs.md`](../mfs.md#rpc)）。1 行目は `compared with your latest version on the relays (<CID>)`。
   - 前の版が無ければ `no previous version on the relays; every file counts as new`。取れなかった・一覧できなかった（ブロックがローカルに無い、項目が `MAX_PREVIOUS_ENTRIES`（100 000）を超える、など）ときは `! previous version unavailable (<理由>); every file counts as new`（`<理由>` は `could not fetch it from the relays: ...` か `could not list <CID> in the local Kubo: ...`）。どちらも前の版を空として扱う。
   - 2 の一覧のファイル（ディレクトリは除く）のうち、前の版に同じパスが無いものが「増えたファイル」。中身が変わっただけのファイルは数えない。無ければ `✓ no new files`、あれば `! N new files` の後に、ルート直下のファイルを先に、続けてフォルダ（ファイルの親のパス全体。`a/b/` は `a/` の中に入れず 1 つのフォルダとする）ごとに `<フォルダ>/` の行とその下へさらに字下げしたファイル名を、フォルダもファイル名もバイト順で並べる（`new_files::group_by_folder`）。並べるファイルは先頭 `LISTED_NEW_FILES`（50）件までで、残りは `… and N more` にまとめる。
   - 増えたファイルがあり `--yes` が無ければ、標準入力が端末なら `Publish with N new files? [y/N]`（1 件なら `1 new file`。以下同じ）と聞き、`y` か `yes`（大小文字無視）以外なら `cancelled; nothing was added or published` でエラー終了する。端末でなければ聞かずに `confirmation needed for N new files: review them above and rerun with --yes` でエラー終了する。
6. `IPFS` 見出しを出す。現在時刻を `created_at` に決め、2 の一覧を CIDv1・pin なしで add し、`<mfs_root>/publish/<pubkey hex>/<site>/<created_at>` に置く（既存の項目は先に消す）。
7. `dag/stat`（`offline=true`）の `TotalSize` を `size` タグにする。失敗したら（ブロックが欠けていたら）6 で置いた版を MFS から消してエラーで終了する。`IPFS` 見出しの `Size:` はこの値。
8. `--check-unchanged` が `off` でなければ、4 で取った前の版と比べ、`Previous version` 見出しの下に結果を表示する。
   - CID が違えば `✓ changed from the latest version on the relays (<前の CID>)`、同じなら `! unchanged: the CID equals your latest version on the relays`、見つからなければ `- no previous version on the relays`、取得に失敗したら `! could not check: <理由>`。
   - 同じで `require` なら、6 で置いた版を MFS から消して `✓ removed <パス>` を出し、署名・送信・古い版の削除をせずに `Unchanged; not published.` で終わる（終了コード 0。消せなければエラー終了）。見つからない・取得に失敗したときは `require` でも続ける。
9. サイトイベント（`alt` は `SWING site announcement: <d>`）を 6 の `created_at` で作って署名し、全 relay に送る。署名アプリを使っているときは、署名の前（`Nostr` 見出しの直後）に `waiting for the signer app to sign the site event...` を表示し、署名アプリの返事を最大 90 秒待つ。relay ごとの成否（✓/✗）を表示する。署名できない、またはどこにも受理されなければ、6 で置いた版を MFS から消し、古い版は消さずにエラーで終了する（6 の版を消せなければその理由もエラーに続けて出す）。
10. `<mfs_root>/publish/<pubkey hex>/<site>/` の中で名前が整数の項目のうち、今回の版（6 の `created_at`）は必ず残し、それ以外を新しい順に `[publish].keep_versions - 1` 個残して消す（`Old versions (keeping N)` 見出し）。一覧に失敗したら警告を出して続ける。
11. `Published.` で終わる。
