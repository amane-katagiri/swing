# publish（`src/dashboard/upload.rs`, `src/dashboard/api.rs`, `src/dashboard/dto.rs`）

[`../http-api.md`](../http-api.md) の子ページ。共通の形式・エラー・準備状態は親ページを参照。

## POST /api/publish/upload

`multipart/form-data`。

パート:

- `site`（必須）・`url`・`title`・`message`・`nip05`・`check_dotfiles`・`check_size`・`check_unchanged`（省略可）。モードの 4 つは `off`/`warn`/`require` で、省略時は `[publish]` の同名の設定。不正な値は 400 `invalid <パート名>: ...`。
- テキストのパートは 1 つあたり `MAX_TEXT_FIELD_BYTES`（64 KiB）までで、超えるか UTF-8 でなければ 400。知らない名前のパートは読み捨てる。
- `site`/`url`/`title` は CLI と同じ規則で検証し、違反は 400。`message` は `MAX_CONTENT_BYTES`（4096 バイト）を超えたら、パートを読んだ時点で 400 `invalid message: ...`。`title` が空白のみなら未指定として扱う。
- `file`（1 個以上）: 各パートの `filename` がサイトルートからの相対パス（`/` 区切り）。`filename` の無い `file` パートは 400。

パスの検証（パートを受け取りながら順に行い、違反は 400）:

- 非空、`/` で始まらない、`\`・`:`・制御文字を含まない
- 長さ `MAX_PATH_LEN`（4096 バイト）以下、セグメント数 `MAX_PATH_SEGMENTS`（32）以下、各セグメントは非空かつ `.`/`..` でない
- 各セグメントは `.` や半角スペースで終わらない、Windows の予約デバイス名（`CON`・`PRN`・`AUX`・`NUL`・`COM1`〜`9`・`LPT1`〜`9`、大小文字無視、拡張子付き `nul.txt` も含む）でない（プラットフォームを問わず拒否）
- 同じパスの重複（大文字小文字を区別しない）
- 同じパスをファイルとディレクトリの両方に使う組み合わせ（`a` と `a/b`）は、後から来たほうを書こうとした時点で 400
- 展開先の下にあることを確かめてから、既存のファイルを上書きしない `create_new` で書く
- `file` パートの総数は `MAX_UPLOAD_FILES`（10,000）まで

本体を受け取り終えてから、`site` が無ければ 400 `missing site`、`file` が 0 個なら 400 を返す。

パート・パスの上限は固定の定数（`MAX_CONTENT_BYTES` は `nostr::budget`、ほかは `src/dashboard/upload.rs`）。ボディ全体の上限は設定の `[dashboard].max_upload`（下記）。

判定の順:

1. パートの受信とパスの検証、`site` と `file` の有無（400・413）
2. `site`/`url`/`title` とモードの 4 つの検証（400）
3. 展開先が設定ファイル・`[agent].state_dir`・`[kubo].repo` と重ならないか（CLI と同じ。400）
4. 多重実行（409 `a publish is already running`）。本体を最後まで受け取ってから判定するので、409 はアップロードの後になる。排他するのはダッシュボード内で同時に来た publish どうしだけで、同じホスト上の CLI `swing publish` とは排他しない
5. セットアップモード（503 `agent is not configured`）
6. NIP-05（422）
7. ドットファイル・サイズ（422）
8. Kubo の準備（503 `agent is not ready`）→ add（失敗は 502）
9. relay の準備（503 `agent is not ready`）→ 同じ内容かの確認 → 署名と送信 → `[publish].keep_versions` を超えた古い版の削除

処理:

- `<state_dir>/upload/` の下に一時ディレクトリを作り、各 `file` パートをストリーミングで書き込む。unix ではディレクトリを `0o700`、ファイルを `0o600` で作る。
- 展開先をサイトのディレクトリとして、CLI の `swing publish`（[`../../cli/publish.md`](../../cli/publish.md)）と同じ処理と判定を上記の順で行う。ドットファイル・サイズは受け取ったファイルそのもので判定する（空のディレクトリは届かない）。relay は agent の接続を使う。受け取ったファイルの一覧に失敗したら 500。
- 展開先ディレクトリは、成功・失敗のときは応答の前に削除する。タイムアウト（[`../../dashboard.md#タイムアウトsrcdashboardmodrs`](../../dashboard.md#タイムアウトsrcdashboardmodrs)）とクライアントの切断では処理を打ち切ったときに削除を始めるので、削除は応答の後になりうる。取りこぼした分は `up::run` の起動時に `<state_dir>/upload/` ごと掃除する（[`../../up.md`](../../up.md)）。
- ボディが `[dashboard].max_upload` を超えたら 413（ストリーミング中に超えても打ち切る）。それ以外の multipart の受信エラー（ボディの読み取り自体の失敗を含む）は 400。

```json
{ "published": true, "site": "example.com", "url": "…", "title": "…", "message": "note", "nip05": { "status": "verified", "detail": null },
  "checks": {
    "dotfiles": { "status": "ok", "mode": "require", "count": 0, "paths": [] },
    "size": { "status": "ok", "mode": "warn", "bytes": 12000, "threshold": 536870912 },
    "unchanged": { "status": "changed", "mode": "require", "previous_cid": "bafy…", "previous_created_at": 1780000000, "detail": null } },
  "cid": "bafy…", "size": 12345, "created_at": 1790000000, "mfs_path": "/swing/publish/<hex>/example.com/1790000000",
  "relays": [ { "relay": "wss://…", "ok": true, "error": null } ], "pruned": ["1780000000"], "prune_error": null,
  "gateway_url": "…", "files": 3 }
```

- `nip05.status` は `off`/`verified`/`mismatch`/`not_applicable`/`error`。`require` で検証が通らなければ、add する前に 422 を返す: `{ "error": "...", "nip05": { "status": "...", "detail": "..." } }`。
- `checks`: 各項目の `mode` はその回に使ったモード。
  - `dotfiles`: `status` は `off`（`count` は 0、`paths` は空）/`ok`/`found`。`count` は見つかった件数（ディレクトリは 1 件）、`paths` はその先頭 `LISTED_DOTFILES`（10）件。
  - `size`: `status` は `off`（`bytes` は `null`）/`ok`/`over`。`bytes` はファイルの大きさの合計、`threshold` は `SIZE_GUIDELINE`（512 MiB）。上の `size`（`dag/stat` の値）とは別物。
  - `unchanged`: `status` は `off`/`changed`/`unchanged`/`no_previous`（relay に前の版が無い）/`unknown`（relay から取れなかった。理由が `detail`）。`previous_cid`・`previous_created_at` は前の版が見つかったときだけ入る。
- ドットファイル・サイズの `require` が引っかかったら、add する前に 422 を返す: `{ "error": "...", "nip05": {...}, "checks": { "dotfiles": {...}, "size": {...}, "unchanged": null } }`。NIP-05 の 422 には `checks` が付かない。
- `unchanged` が `unchanged` で `check_unchanged` が `require` なら、add した版を MFS から消し、署名も送信も古い版の削除もせずに 200 を返す。このとき `published` は `false`、`created_at` と `mfs_path` は `null`、`relays` と `pruned` は空、`prune_error` は `null`。`cid`・`size`・`gateway_url` は通常どおり入る。[`/api/activity`](status.md#get-apiactivity) の `latest_published_at` は進まない。版を消せなければ 502。
- どの relay にも受理されなければ 502（Kubo に add した内容と古い版はそのまま残す）。
- 古い版の削除に失敗したときは `prune_error` に理由が入るだけで、応答は成功のまま。
- `files` は受け取ったファイル数。

## GET /api/publish/sites

自分（agent の鍵）が過去に公開したサイトの一覧。relay から自分の pubkey のサイトイベントを取得し、`d` ごとの最新版を `d` の昇順で先頭 `MAX_SITES_PER_AUTHOR_LISTED` 件まで返す（[取得と表示の上限](../../nostr/fetch.md)）。

```json
{ "sites": [ { "d": "example.com", "url": "https://example.com/", "cid": "bafy…", "size": 123, "created_at": 1790000000, "title": null, "message": null, "gateway_url": "http://localhost:8080/ipfs/bafy…/" } ] }
```

- `gateway_url` は gateway 設定があれば付ける（`stored` 判定はしない）。
- `state.json` は見ないので、`size` は常にイベントの自己申告の値。
- relay の取得に失敗したら 502。

## GET /api/publish/previous-files?site=<d>

公開画面が、アップロードする前に増えたファイルを出すために使う。CLI の[増えたファイルの確認](../../cli/publish.md)と同じく、relay から自分の pubkey・この `d` の最新のサイトイベントを取り、その CID を Kubo でオフラインに一覧する（`publish::PreviousFiles::load`）。比べるのはブラウザ側で、サーバは一覧を返すだけ。`/api/publish/upload` はこの確認を経たかどうかを見ない。

- `site` が無ければ 400 `missing site`、[`d` の条件](../../nostr.md#検証)を満たさなければ 400 `invalid site: ...`。
- relay と Kubo を使う（準備状態と同時実行の上限は親ページ）。

```json
{ "status": "listed", "previous_cid": "bafy…", "previous_created_at": 1790000000, "detail": null, "files": ["index.html", "css/style.css"] }
```

- `status` は `listed`（一覧できた）/`no_previous`（relay に前の版が無い）/`unknown`（relay から取れなかった、または Kubo で一覧できなかった。理由が `detail`）。`listed` 以外では `previous_cid`・`previous_created_at` は `null`、`files` は空。
- `files` はファイルのパス（ディレクトリは含まない）の名前順。前の版の項目が `MAX_PREVIOUS_ENTRIES`（100 000）を超えたら `unknown`。
- relay や Kubo が失敗しても 200 の `unknown` で返す。
