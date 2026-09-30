# MFS と Kubo RPC（mfs.rs, ipfs.rs）

[`../architecture.md`](../architecture.md) の一部。managed／unmanaged どちらの Kubo にも共通する。Kubo プロセスの管理は [`kubo.md`](kubo.md)。

## MFS の使い方

agent も publish も pin を使わず、MFS にサイトの CID を置いて GC から守る。同じ CID を複数の場所に置いても、すべての場所から消えるまで GC されない。`mfs_root` の外や手動の pin には触れない。

| パス | 持ち主 |
|---|---|
| `<mfs_root>/agent/<pubkey hex>/<site>/<created_at>` | agent。版ごとに 1 つ。この下は agent だけが使い、state が参照しない項目は sweep で消す |
| `<mfs_root>/publish/<pubkey hex>/<site>/<created_at>` | publish。自分のサイトの版ごとに 1 つ |

- `<site>` は `d` のパーセントエンコード（`A-Z a-z 0-9 - . _ ~` 以外を `%XX`）。`d` が `.` か `..` ならドットも `%2E` にする。
- MFS のパスを RPC のクエリ（`arg`・`to-files`）に載せるときは、`/` で区切った各段をもう 1 回パーセントエンコードする（`<site>` の `%` は `%25` になる）。Kubo はクエリを 1 回だけデコードするので、MFS 上の名前は `<site>` のまま残る。実 Kubo での往復は `mfs_kubo_integration` で確かめる。
- `<created_at>` はサイトイベントの `created_at`（10 進）。

MFS は DAG が欠けていても置け、GC も `block/rm` も止めない。置いた後の完全性は `dag/stat`（`offline=true`）で確かめる。MFS から消したコンテンツや打ち切った取得のブロックは、Kubo の GC で消える。

## RPC

- すべて `POST /api/v0/...`。CID とパスはクエリに入れる前にパーセントエンコードする（パスは要素ごと。[上記](#mfs-の使い方)）。
- 非 2xx はボディ付きのエラーにする。
- 応答のボディは 16 MiB（`ipfs::MAX_RESPONSE_BYTES`）までしか読まず、超えたらエラーにする。`dag/export` のエラーのボディも同じ上限で読む（超えたらボディ無しのエラー）。`dag/export` の成功時のボディは読み捨てながら `max_bytes` で打ち切る。
- HTTP クライアントはすべて `ipfs::kubo_http_client(secret)` で作り、プロキシの環境変数（`HTTP_PROXY` など）やシステムのプロキシ設定を使わない。managed の Kubo への秘密の付け方は [`kubo.md#rpc-の認証managed-のみ`](kubo.md#rpc-の認証managed-のみ)。

| 操作 | リクエスト | タイムアウト |
|---|---|---|
| 取得 | `dag/export?arg={cid}&progress=false` | 全体 `[agent].fetch_timeout`、無通信 `[agent].fetch_idle_timeout` |
| 実サイズ・完全性 | `dag/stat?arg={cid}[&arg={cid}...]&progress=false&offline=true` → `TotalSize` | 300 秒 |
| ディレクトリ作成 | `files/mkdir?arg={path}&parents=true` | 60 秒 |
| 配置 | `files/cp?arg=/ipfs/{cid}&arg={path}&offline=true` | 60 秒 |
| 削除 | `files/rm?arg={path}&recursive=true&force=true` | 60 秒 |
| 一覧 | `files/ls?arg={path}&long=true` → `Entries`（`Type` 1 がディレクトリ） | 60 秒 |
| CID の中のファイル一覧 | `ls?arg={cid}&resolve-type=true&size=false&offline=true` → `Objects[].Links`（`Type` 1 と 5 がディレクトリ）。ディレクトリごとに呼ぶ | 60 秒（1 回あたり） |
| CID の確認 | `files/stat?arg={path}&hash=true` → `Hash` | 60 秒 |
| ディレクトリ判定 | `files/stat?arg=/ipfs/{cid}` → `Type`（`directory` か `file`） | 60 秒 |
| PeerID | `id` → `ID` | 10 秒 |
| 通信量 | `stats/bw` → `TotalIn`・`TotalOut`（[`stats.md`](stats.md)） | 10 秒 |
| 停止 | `shutdown` | 呼び出し元が指定（[`kubo.md#停止daemonstopgrace`](kubo.md#停止daemonstopgrace) は 5 秒、孤児回収は 3 秒） |
| add（publish） | `add?recursive=true&cid-version=1&pin=false&quieter=true&wrap-with-directory=false&to-files={path}` | 300 秒 |

- `dag/stat` に CID を複数渡すと、`TotalSize` はそれらをまとめた重複排除後のサイズ（同じブロックを 1 回だけ数えた合計）になる。1 つでもブロックが欠けていれば呼び出し全体が失敗する。CID を 1 つも渡さないときは呼ばずに 0 を返す。
- `dag/export` の無通信タイムアウトはヘッダー受信までにも適用する。
- 配置は親ディレクトリを作り、同名の項目を消してから行う。`offline=true` なのでルートのブロックがローカルに無ければ即エラー。
- CID の中のファイル一覧（`ipfs::list_files_local`。publish の[増えたファイルの確認](cli/publish.md)が前の版に使う）は、ディレクトリを 1 つずつ `ls` でたどり、ファイルのパス（ルートからの相対、`/` 区切り）を名前順で返す。`offline=true` なのでブロックがローカルに無ければその時点でエラーになり、たどった項目（ディレクトリを含む）が呼び出し元の指定した数を超えてもエラーにする。
- `files/rm` は失敗しても 200 でボディにメッセージを返すので、ボディが空でなければ失敗とする。存在しないパスは成功。
- `files/ls` と `files/stat` の `file does not exist` は、それぞれ空の一覧、「無い」として扱う。
- ディレクトリ判定は MFS のパスではなく `/ipfs/{cid}` を `files/stat` に渡す。agent は取得の直後に呼ぶ（[`agent.md` の「保存の順序」](agent.md#保存の順序)）。

`add` の multipart:

- 各ファイルは `name="file"` パート。`filename` はルートディレクトリ名を先頭に付けた相対パス（例: `public/css/style.css`、URL エンコード）。
- ファイルは `application/octet-stream` でストリーミング送信、ディレクトリは空ボディの `application/x-directory`。
- シンボリックリンクは辿る。ただし、リンク先を `canonicalize` した実パスが、`canonicalize` したルートディレクトリの下に無ければエラーにして何も追加しない。循環もエラー。
- 最後の JSON 行の `Hash` がルート CID（`ipfs add -Qr --cid-version=1` と同じ）。
- `to-files` のパスにルートディレクトリそのものが置かれる。add の前に親ディレクトリを作り、同じパスの既存の項目を `files/rm` で消す。
