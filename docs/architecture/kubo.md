# Kubo と MFS（ipfs.rs, mfs.rs）

[`architecture.md`](../architecture.md) の一部。

## MFS の使い方

agent も publish も pin を使わず、MFS にサイトの CID を置いて GC から守る。同じ CID を複数の場所に置いても、すべての場所から消えるまで GC されない。`mfs_root` の外や手動の pin には触れない。

| パス | 持ち主 |
|---|---|
| `<mfs_root>/agent/<pubkey hex>/<site>/<created_at>` | agent。版ごとに 1 つ。この下は agent だけが使い、state が参照しない項目は sweep で消す |
| `<mfs_root>/publish/<pubkey hex>/<site>/<created_at>` | publish。自分のサイトの版ごとに 1 つ |

- `<site>` は `d` のパーセントエンコード（`A-Z a-z 0-9 - . _ ~` 以外を `%XX`）。`d` が `.` か `..` ならドットも `%2E` にする。
- `<created_at>` はサイトイベントの `created_at`（10 進）。

MFS は DAG が欠けていても置け、GC も `block/rm` も止めない。そのため置いた後の完全性は `dag/stat`（`offline=true`）で確かめる。

MFS から消したコンテンツや打ち切った取得のブロックは、Kubo の GC で消える。

## RPC

すべて `POST /api/v0/...`。CID とパスはクエリに入れる前にパーセントエンコードする（パスは要素ごと。`<site>` のエンコードと合わせて二重になる）。非 2xx はボディ付きのエラーにする。

| 操作 | リクエスト | タイムアウト |
|---|---|---|
| 取得 | `dag/export?arg={cid}&progress=false` | 全体 `SWING_FETCH_TIMEOUT`、無通信 `SWING_FETCH_IDLE_TIMEOUT` |
| 実サイズ・完全性 | `dag/stat?arg={cid}&progress=false&offline=true` → `TotalSize` | 300 秒 |
| ディレクトリ作成 | `files/mkdir?arg={path}&parents=true` | 60 秒 |
| 配置 | `files/cp?arg=/ipfs/{cid}&arg={path}&offline=true` | 60 秒 |
| 削除 | `files/rm?arg={path}&recursive=true&force=true` | 60 秒 |
| 一覧 | `files/ls?arg={path}&long=true` → `Entries`（`Type` 1 がディレクトリ） | 60 秒 |
| CID の確認 | `files/stat?arg={path}&hash=true` → `Hash` | 60 秒 |
| add（publish） | `add?recursive=true&cid-version=1&pin=false&quieter=true&wrap-with-directory=false&to-files={path}` | 300 秒 |

- `dag/export` は最初のブロックが取れるまでヘッダーを返さないので、無通信タイムアウトはヘッダー受信までにも適用する。
- 配置は親ディレクトリを作り、同名の項目を消してから行う（同名があると `files/cp` が失敗する）。`offline=true` なのでルートのブロックがローカルに無ければ即エラー。
- `files/rm` は失敗しても 200 でボディにメッセージを返すので、ボディが空でなければ失敗とする。存在しないパスは成功。
- `files/ls` と `files/stat` の `file does not exist` は、それぞれ空の一覧、「無い」として扱う。

`add` の multipart:

- 各ファイルは `name="file"` パート。`filename` はルートディレクトリ名を先頭に付けた相対パス（例: `public/css/style.css`、URL エンコード）。
- ファイルは `application/octet-stream` でストリーミング送信、ディレクトリは空ボディの `application/x-directory`。
- シンボリックリンクは辿る。循環はエラー。
- 最後の JSON 行の `Hash` がルート CID（`ipfs add -Qr --cid-version=1` と同じ）。
- `to-files` のパスにルートディレクトリそのものが置かれる。親ディレクトリは先に作る。

## Kubo のバージョン

compose の Kubo は検証済みの `v0.43.1` に固定している。次の挙動に依存しているので、上げると壊れうる。

- `file does not exist` の文面での判定（変わると、突き合わせが MFS から消えた版を取り直さず警告を出し続ける）
- `files/rm` が失敗時も 200 を返すこと
- 各 RPC の JSON の形（`TotalSize`、`Hash`、`Entries[].Type` など）と、`add` の multipart・`to-files`
- MFS の保護・GC・`offline=true` の挙動

上げるときは、新しいイメージで統合テスト（`kubo_integration` と `agent_stores_and_removes_through_real_kubo`）を通してから、`compose.yaml` と [テスト手順](../architecture.md#テスト) のタグを同時に上げる。既存の `ipfs-data` は `--migrate=true` で移行され、古いバージョンに戻せないことがある。
