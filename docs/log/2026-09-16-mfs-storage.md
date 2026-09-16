# 2026-09-16 保存先を pin から MFS に移す

## 目的

Kubo の pin は、ノード全体で CID ごとに 1 つ（名前も 1 つ）しか持てない。そのため、SWING・publish・運用者の手動 pin が同じ CID を持つと持ち主を区別できず、次の問題を個別の仕組み（`preexisting_pin`、`releasing`、共有 CID の参照確認）で抑えていた。

- 手動の pin を SWING が外してしまう。
- SWING が pin した後に運用者が付けた pin を記録できず、SWING が外すときに一緒に消える。
- direct pin のある CID を SWING が recursive で pin すると、direct pin が recursive に置き換わる。
- publish が付けた pin は管理されず、古い版が溜まり続ける。

MFS（Kubo 内のファイルシステム）に CID を置けば、場所ごとに独立して GC から守られるので、これらをまとめて解消できる。

## 確認したこと（Kubo 0.43.1、`IPFS_PROFILE=test`）

- `files/cp` で置いた CID は元の CID のまま（CIDv1 のディレクトリも CIDv0 のファイルも）。元の CID でそのまま取り出せる。MFS のディレクトリ側には別の CID が付く。
- 同じ CID を 2 か所に置くと、片方を消して GC しても残り、両方消して GC すると消える。
- MFS は、子ブロックが欠けた DAG も `files/cp` で置ける。GC は欠けていてもエラーにならない。MFS にしか無いブロックは `block/rm` で直接消せる（pin 済みなら拒否される）。
- `files/cp` に `offline=true` を付けると、ルートのブロックが無い CID は即エラーになる。
- `files/rm` は失敗しても 200 を返し、ボディにメッセージの文字列が入る。`force=true` なら存在しないパスの削除は空のボディで成功する。ディレクトリには `recursive=true` が要る。
- `files/ls` は `long=true` でないと `Type` が常に 0。空のディレクトリは `Entries: null`。存在しないパスは `file does not exist`。
- `files/cp` は、同じ名前の項目があると `directory already has entry by that name` で失敗する。
- クエリの `arg` は Kubo がデコードしてから使う。パス中の `%2F` を生で送ると `/` として扱われる。760 バイトの名前も置けた。
- `add` の `to-files` で、ディレクトリのルートを指定したパスに置ける。親ディレクトリは先に作っておく必要がある。
- `Provide.Strategy` は `all`・`pinned`・`roots`・`mfs`・`pinned+mfs` を受け付け、知らない値では daemon が起動しない。未設定の既定では変更していない。
- pin の名前（`pin-name` / `name`）は CID ごとに 1 つで、別の名前で `pin/add` すると上書きされる。`pin/ls` の `name` は部分一致。名前では複数の持ち主を表せないので採用しなかった。

## 決めたこと

| 決定 | 理由 |
|---|---|
| agent は `<mfs_root>/agent/<pubkey hex>/<site>/<created_at>`、publish は `<mfs_root>/publish/<pubkey hex>/<site>/<created_at>` に置く。`mfs_root` は設定（既定 `/swing`） | 用途ごとに独立した場所なので、同じ CID でも互いに影響しない。運用者の pin や MFS の他の場所には触れない |
| `<site>` は `d` のパーセントエンコード。`.` と `..` はドットもエンコードする | `d` は `/` を含みうる。`.` と `..` はそのままだとパスの意味を持つ |
| 版の名前は `created_at` だけにし、CID は入れない | publish は add の前に名前を決める必要があり、CID はまだ分からない。agent 側では同じサイトの記録済みの版と `created_at` が重なることはない（stale 判定）ので、置く前に同名の項目を消してよい |
| `<mfs_root>/agent` の下は agent だけが使い、sweep で state が参照しない項目をすべて消す | 失敗した削除や、置いた後に state を保存する前の中断で残った項目を、別のリストを持たずに片付けられる。これで `releasing` が不要になった |
| `preexisting_pin`、`releasing`、`is_pinned`、共有 CID の参照確認、pin 関連の RPC を削除する | MFS では持ち主ごとに場所が分かれるので要らない |
| 取得後は、MFS に置いてから `dag/stat` で完全性を確かめる | 取得したブロックは置くまで GC から守られない。置いた後に確かめれば、確かめた結果が GC で崩れない。`files/cp` はルートしか見ないので、欠けていても止まらない |
| 起動時の突き合わせは、各版のパスの CID と `dag/stat` を確かめ、合わない版を state から消す | MFS の保護が緩い（欠けを許す、`block/rm` を止めない）ので、壊れた版を取り直せるようにする。全 DAG をたどるので起動が遅くなることは受け入れる |
| unfollow は state を消して保存してから、`<mfs_root>/agent/<pubkey hex>` をまとめて消す | 版ごとに消す必要が無い。失敗しても sweep が消す |
| publish は pin なしで add し、`to-files` で置く。その後 `dag/stat` の `TotalSize` を `size` タグにし、失敗したらエラーで終了する | pin ありで add してから外すと、運用者が同じ CID を pin していた場合に外してしまう。pin なしの add は GC ロックを取らないので、add 中の GC による欠けを `dag/stat` で検出する。`files/stat` の `CumulativeSize` ではなく agent と同じ尺度にし、以前の「サイズが取れなくても `size` 無しで publish する」挙動はやめた |
| publish は、どこかの relay に受理されてから古い版を消す。残す数は `[publish].keep_versions`（既定 5） | 送信に失敗したのに古い版を消すと、他のノードが知っている最新版が自分のノードから消える |
| イベントの `created_at` は add の前に決めた時刻にし、MFS の名前と一致させる | publish の古い版を名前だけで並べられる |
| 設定の `unpin_on_unfollow` を `remove_on_unfollow`、`SWING_PIN_TIMEOUT` を `SWING_FETCH_TIMEOUT` に改名する | pin を使わなくなった。運用中の設定は無いので、古い名前の読み替えは入れない |
| `VersionRecord.pinned_at` を `stored_at`、`swing sites` の表示を `stored` / `not stored`、`Decision` のフィールドを `store` / `evict` に改名する | 同上 |
| compose の Kubo の起動スクリプトで `Provide.Strategy` を `SWING_KUBO_PROVIDE_STRATEGY`（既定 `pinned+mfs`）に設定する。スクリプト名を `001-swing-config.sh` に変える | `pinned` だけにされると MFS にしか無いサイトが DHT に告知されず、ミラーとして役に立たない |

## 移行について

運用中の state.json や Kubo の pin は無いので、移行処理は入れていない。この変更より前の state.json（`preexisting_pin`、`releasing`、`pinned_at` を含むもの）は読めない。以前の版が付けた pin は SWING からは外されない。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（ユニットテスト 145 件、ほかに `#[ignore]` 1 件）。agent のテストは MFS をメモリ上で真似る fake に書き直した（保存、欠け・reject 時の削除、共有 CID の独立性、unfollow のディレクトリ削除、失敗した削除の sweep での回収、state に無い項目の削除、MFS の外に触れないこと、保持期間、突き合わせ）。
- sweep の「state が参照するか」の判定を常に真にすると、sweep のテスト 2 件が失敗することを確かめた。
- ローカルの Kubo 0.43.1 で統合テスト 7 件（add と MFS の往復、特殊文字を含む `d`、上書き、GC からの保護、取得の打ち切り、存在しない CID で `files/cp` が即失敗すること、欠けた DAG の `dag/stat`）と、実物の Kubo で agent の保存・突き合わせ・sweep・unfollow を通す `#[ignore]` テストを実行した。
- ローカルの Kubo と nostr-rs-relay で `swing publish` を 3 回実行し（`keep_versions = 2`）、最古の版が MFS から消えることを確認した。続けて自分の pubkey をミラー対象にして `swing agent` を動かし、最新版が `<mfs_root>/agent/...` に置かれて state に記録されること、`swing sites` が `[stored]` を表示することを確認した。
- 起動スクリプトで `Datastore.StorageMax` と `Provide.Strategy` が設定されることをコンテナで確認した。
