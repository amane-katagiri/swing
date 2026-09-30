# publish（`src/publish.rs`, `src/publish/checks.rs`）

[`../cli.md`](../cli.md) の子ページ。`swing publish` の引数・確認・処理順。ダッシュボードの公開画面からの publish は [`../dashboard/http-api/publish.md`](../dashboard/http-api/publish.md)。

`DIR` を Kubo に add して MFS に置き、サイトイベントに署名して全 relay に送り、同じサイトの古い版を MFS から消す。

- `--site` は必須で、そのまま `d` になる。[`d` の条件](../nostr.md#検証)を満たさなければ `invalid --site` でエラー終了する。
- `--url` は任意。指定すると `url` タグになり、[`url` の条件](../nostr.md#検証)を満たさなければ `invalid --url: ...` でエラー終了する。省略すると `url` タグを付けない。
- `--nip05` 省略時は `[publish].nip05`。
- `--check-dotfiles`・`--check-size`・`--check-unchanged` はサイトの確認のモード（`off`/`warn`/`require`）。省略時はそれぞれ `[publish].check_dotfiles`（既定 `require`）・`check_size`（既定 `warn`）・`check_unchanged`（既定 `require`）。`--nip05` を含めた 4 つのモードは表示や処理の前にまとめて解釈し、不正な値は `invalid --<フラグ名>` でエラー終了する。
- `--title` は任意。指定すると `title` タグになる。前後の空白を削り、空になれば付けない扱いにする。256 バイトを超える、または制御文字か見えない書式文字（[`title` の条件](../nostr.md#検証)）を含む場合は `invalid --title: ...` でエラー終了する。
- `--message` はサイトイベントの `content` になる（省略時は空文字）。受け取る側の上限（`MAX_CONTENT_BYTES`、4096 バイト。[`content` の扱い](../nostr.md#検証)）を超えたら何もせずに `invalid --message: must not exceed 4096 bytes (got N bytes)` でエラー終了する。
- 最初に `Site: <d>`、`--url` があれば `URL:`、`--title` があれば `Title:`、`--message` があれば `Message:` を表示する。

引数とモードを解釈した後、表示の前に、次のどちらかなら確認のモードにかかわらず何もせずにエラー終了する（`publish::refuse_protected_paths`。存在しないパスは見ない）。

- `DIR` の実体（`canonicalize`）の中に、設定ファイル・`[agent].state_dir`・`[kubo].repo` のどれかの実体がある（`DIR` そのものである場合を含む）。
- `DIR` の実体が `[kubo].repo` か `[agent].state_dir` の実体の中にある（そのものである場合を含む）。ただし `<state_dir>/upload/` の下（`upload/` そのものは除く）は止めない。設定ファイルのあるディレクトリの中は止めない。

処理順:

1. `--nip05` が `off` でなければ、`d` と自分の pubkey で NIP-05 を検証し、`NIP-05` 見出しの下に結果を表示する（`✓ verified` / `! mismatch: ...` / `- not applicable (d is not a domain)` / `! error: ...`）。`require` で `Verified` 以外（`NotApplicable` を含む）なら add せず終了する。
2. `--check-dotfiles` と `--check-size` のどちらかが `off` でなければ、`DIR` を add と同じ辿り方（`ipfs::list_site`。シンボリックリンクを辿り、ドットファイルも含める。リンク先が `DIR` の外ならこの時点でエラー）で一覧し、`Checks` 見出しの下に 1 行ずつ結果を表示する（`off` の項目は `- dotfiles: off` のように出す）。
   - ドットファイル: 各パスをサイトのルートから順にセグメントごとに見て、名前が `.` で始まり `[publish].dotfiles_allow` のどれとも一致しない最初のセグメントまでを 1 件とする（ディレクトリは 1 回だけ数え、その下は見ない）。一致するセグメントはそれ自身だけを見逃し、その下は続けて見る（`.well-known/.env` は `.well-known/.env` を 1 件とする）。無ければ `✓ dotfiles: none`、あれば `! dotfiles: N found (not in [publish].dotfiles_allow)` の後に先頭 `LISTED_DOTFILES`（10）件のパスを字下げして並べ、残りは `… and N more` にまとめる。
   - サイズ: ファイルの大きさの合計（ブロックの共有やディレクトリのノードは数えない）が `SIZE_GUIDELINE`（512 MiB、固定）を超えたら（ちょうどは超えない扱い）`! size: <合計> is over the 512 MiB guideline; each mirror decides by its own limits (max_update_size, default 2 GiB)`、超えなければ `✓ size: <合計> (guideline 512 MiB)`。
   - `require` の項目が引っかかったら、項目ごとの対処の案内を `; ` でつないだメッセージで、add せずにエラー終了する。
3. Kubo の RPC クライアントを `Config::ipfs_client` で作る（managed なら `<state_dir>/kubo-api.json` のポートと秘密を使い、その API の PeerID が `<repo>/config` のものと一致しなければエラー終了。[`kubo.md`](../kubo.md#rpc-クライアントの作り方configipfs_client)）。現在時刻を `created_at` に決め、`DIR` を CIDv1・pin なしで add し、`<mfs_root>/publish/<pubkey hex>/<site>/<created_at>` に置く（既存の項目は先に消す）。
4. `dag/stat`（`offline=true`）の `TotalSize` を `size` タグにする。失敗したら（ブロックが欠けていたら）エラーで終了する。`IPFS` 見出しの `Size:` はこの値。
5. relay に接続する（6 と 7 で同じ接続を使う）。
6. `--check-unchanged` が `off` でなければ、relay から自分の pubkey・この `d` のサイトイベントのうち `parse_site_event` を通り `created_at` が未来すぎない最新 1 件を取り、`Previous version` 見出しの下に結果を表示する。
   - CID が違えば `✓ changed from the latest version on the relays (<前の CID>)`、同じなら `! unchanged: the CID equals your latest version on the relays`、見つからなければ `- no previous version on the relays`、取得に失敗したら `! could not check: <理由>`。
   - 同じで `require` なら、3 で置いた版を MFS から消して `✓ removed <パス>` を出し、署名・送信・古い版の削除をせずに `Unchanged; not published.` で終わる（終了コード 0。消せなければエラー終了）。見つからない・取得に失敗したときは `require` でも続ける。
7. サイトイベント（`alt` は `SWING site announcement: <d>`）を 3 の `created_at` で作って署名し、全 relay に送る。署名アプリを使っているときは、署名の前（`Nostr` 見出しの直後）に `waiting for the signer app to sign the site event...` を表示し、署名アプリの返事を最大 90 秒待つ。relay ごとの成否（✓/✗）を表示する。署名できない、またはどこにも受理されなければ、古い版を消さずにエラーで終了する。
8. `<mfs_root>/publish/<pubkey hex>/<site>/` の中で名前が整数の項目のうち、今回の版（3 の `created_at`）は必ず残し、それ以外を新しい順に `[publish].keep_versions - 1` 個残して消す（`Old versions (keeping N)` 見出し）。一覧に失敗したら警告を出して続ける。
9. `Published.` で終わる。
