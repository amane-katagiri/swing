# architecture の整理

`docs/architecture/` は機能を足すたびに追記してきたため、重複・コードとのずれ・CLI と API が共有する処理の置き場所の偏りがたまっていた。全ページをコードと照らして整理した。整理と評価を交互に繰り返し、評価は毎回前の作業を知らない新しい担当が行った。クリティカルな指摘が無くなるまで続け、5 回目で無くなった。

## 決めたこと

- CLI とダッシュボード API の両方が使う処理は、表示に依存しない共通ページを正本にする。CLI のページには出力の書式と終了コードだけ、API のページには判定の順・ステータスコード・JSON だけを残し、共通ページの節へアンカーでリンクする。手順の番号で参照し合うと、番号を振り直したときにリンク元が黙って壊れるので使わない。
  - `publish.md`: publish の段階（保護パス、時計の確認、`created_at`、版の配置、後始末、古い版の削除）
  - `mirror.md`: Follow Set の選び方・書き換え、`mirror list` と `sites` の集計
  - `webring.md`: たどり方とグラフ
  - `health.md`: 版と MFS の突き合わせ、ゴミの検出、`status` の集計。agent と `status` で共通
  - レプリカ報告の一覧の集計（`replicas::collect`）は、信頼度の規則と同じ `nostr.md` に置く
- 20KB を超えていた `kubo.md` は、デーモンの起動・停止・孤児の回収を `kubo/daemon.md` に分けた。
- 利用者向けの `guide/publish.md` にあった時計の確認の数値（15 分・14 分・1 分・NIP-11 の `Date`）は、architecture の正本へのリンクにした。

## 直したコードとのずれ

- `mfs.md`: CID の中の一覧（`ls`）でディレクトリとみなす `Type` は 1 だけ。以前は「1 と 5」と書いていた。Kubo v0.43.1 は HAMT シャードのディレクトリも 1 で返す。これは実物で確かめた（8000 ファイルのディレクトリを add して `ls` する）。Kubo のコードでも、`coreapi` が `THAMTShard` を `TDirectory` にまとめている。
- `signer.md`: 存在しない `publish::sign_and_send` を `publish::sign_site_event` に直した。
- `config.md`: `api::blocking` を `dashboard::error::blocking` に直した。
- 次の挙動の説明や範囲がコードと違っていたので、合わせて直した。
  - agent のログの水準
  - `installer-windows.md` で `swing stop` を使う条件
  - `dashboard/desktop.md` の「ミラーに追加」を開いたときの動き
  - `http-api/nostr.md` の Follow Set の `created_at` の理由
- 抜けていた挙動を足した。対象は `install-sh.md`・`cli.md`・`cli/publish.md`・`up.md` など。

## 検証したこと

- docs の相対リンクと見出しのアンカーを、スクリプトで全部確かめて切れが 0 件。
- 最後の評価では、architecture 内のバッククォートで囲んだ識別子と定数をコードで照合した。あわせて、差分で消えた記述が移した先に残っていることを確かめた。
- コードは変えていない。
