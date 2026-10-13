# 確かめるだけの publish（`src/publish/dry_run.rs`）

[`../publish.md`](../publish.md) の子ページ。CLI と共有する段階の関数で、何も書かずに確かめる処理。表示・JSON・終了コードは [`../cli/publish.md`](../cli/publish.md#確かめるだけの実行--dry-run)。

`publish::dry_run::run`（評価は `evaluate`）。CLI の `swing publish --dry-run` だけが使う。[共通処理](../publish.md#段階の順)の段階の関数（`prepare`・`check_nip05`・`read_and_check`・`RelayState`・`list_newest_version`・`date_version`・`PreviousFiles`・`UnchangedOutcome`）をそのまま呼び、本番で止まる条件に当たっても途中で止めずに全部の結果を集める。ブロックの保存・pin・MFS への書き込み・署名・relay への送信・通常の投稿・古い版の削除はしない。署名アプリ（NIP-46）には接続せず、公開鍵は設定と `remote-signer.json` から読むだけ。

段階ごとの分岐:

1. 引数の検証と[保護パスの拒否](../publish.md#保護パスの拒否)は本番と同じ（`prepare`）。当たれば評価を始めずにエラーにする。
2. 公開鍵: `Signer::load`。秘密鍵も署名アプリの接続情報も無い、または読めないときは「不明」として続ける（本番は `Signer::require` でここで止まる）。
3. NIP-05: モードが `off` でなく公開鍵が分かれば本番と同じに確かめる。公開鍵が不明なら「不明」。
4. [サイトの一覧とローカルの確認](../publish.md#サイトの一覧とローカルの確認)は本番と同じ。一覧の失敗（ディレクトリが無い、リンク先が外にあるなど）はエラーにする。
5. Kubo: `Config::ipfs_client`。作れなければ（管理下の Kubo が動いていないなど）理由を持って「不明」として続ける。relay: 公開鍵が分かれば接続して[前の版と relay の時計](../publish.md#前の版と-relay-の時計)を取る。公開鍵が不明か接続できなければ、前の版は「不明」とし、relay の時計だけを取る（`RelayState::clocks_only`）。
6. [時計の確認](../publish.md#時計の確認): MFS の版を一覧する（`files/ls` で読むだけ）。Kubo か公開鍵が不明、または一覧に失敗したら MFS の版を「不明」として、前の版と relay の時計だけで確かめる。当たれば止める理由に数え、当たらなければ本番が使う `created_at` を出す（本番は add の直前にもう一度決めるので、同じ値になるとは限らない）。
7. [前の版のファイル一覧](../publish.md#前の版のファイル一覧)と増えたファイル: 本番と同じ。Kubo が不明なら「不明」（`PreviousFiles::load_with`）。増えたファイルの確認は聞かない。
8. CID: サイトの一覧を本番の add と同じオプションに `only-hash=true` を足して Kubo に送る（`IpfsClient::hash_site`）。Kubo はブロックを保存せず、pin もせず、MFS にも置かずに CID だけを返す。Kubo の設定とサイトの中身が同じなら、本番の add と同じ CID になる。`dag/stat` はしないので、サイトイベントの `size` タグの値は出さない。
9. [同じ内容かの確認](../publish.md#同じ内容かの確認): 8 の CID を前の版の CID と比べる。モードが `off` でも比べた結果は出す（止める・止めないの判定には使わない）。CID か前の版が不明なら「不明」。

止める理由は NIP-05（`require` で `Verified` 以外）・ドットファイル・サイズ・リンク（`require` で引っかかる）・時計の順に集める。止める理由が無く、同じ内容で `check_unchanged` が `require` なら「公開されない（unchanged）」、それ以外は「公開される」。「不明」は止める理由にしない。
