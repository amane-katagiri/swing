# README を最短の手順に絞り、使い方を docs/guide/ に分ける

## 決めたこと

- README は約 72KB まで膨らみ、初めての人が最初に何をすればいいかが埋もれていた。README の役割を「概要」と「初めての人が誰かのサイトをミラーできるまで」に絞る。
- README の「はじめかた」は、入れる → ダッシュボードでセットアップする → 誰かのサイトをミラーする → 保存できたか確かめる、の 4 段にした。入れ方は OS ごとに一番手軽なもの（Linux は `install.sh --service`、Windows はインストーラー、macOS は Homebrew、ほかに Docker Compose）だけを挙げ、設定はセットアップ画面に任せる。設定ファイルを先に書く手順やソースからのビルドは README から外した。
- それより先の使い方は `docs/guide/` に、読む人の目的ごとに分けた。
  - `install.md`: 必要なもの、設定ファイルとデータの置き場所、セットアップモード、OS ごとの入れ方、バイナリ
  - `docker.md`: Docker Compose、Docker Compose からバイナリへの移り方（`install.md` に入れると約 22KB になり、Docker Compose を使わない人には不要なので分けた）
  - `usage.md`: ミラー対象の管理・点検・ログ、ダッシュボード、マスコットの追加
  - `publish.md`: `swing publish`、レプリカ数、Webring、内蔵ゲートウェイ
  - `operation.md`: どれくらい保存されるか、動かし方の目安、設定一覧
  - `security.md`: 外から何が見えるか、ダッシュボードを外の端末から使う、署名アプリ（NIP-46）
- 既存の `docs/site-guide.md`・`docs/mascot-guide.md` は動かさず、README の「次に読むもの」から並べて辿れるようにした。
- 概要の「Web UI は提供していません」と、含まれていないものの「専用の Web UI」は、手元で動くダッシュボードと食い違って読めたので、「誰かが運営する Web サービス」を提供しないという意味に書き直した。
- それ以外の本文は旧 README から移しただけで、内容は変えていない。変えたのは見出しの階層、ファイルをまたぐ参照（「上記」「下記」やページ内アンカー）、相対リンクの起点。

## 作ったもの

- `README.md` を書き直し（約 11KB）。
- `docs/guide/` の 6 ファイル。
- 旧 README のアンカーを指していた `docs/architecture/docker.md`・`docs/architecture/signer.md`・`docs/architecture/dashboard/security.md`・`docs/site-guide.md`・`docs/todo.md` のリンクを `docs/guide/` へ付け替えた。
- `AGENTS.md` の文書の役割表に `docs/guide/` を足し、README の役割の説明を改めた。

## 確かめたこと

- README・`docs/guide/`・リンクを直したファイルについて、相対リンクの行き先のファイルと見出しアンカー（GitHub の生成規則に合わせたもの）がすべて存在することをスクリプトで確かめた。
- README で案内したセットアップ画面の項目名（鍵・リレー・ストレージ上限、「新しい鍵を生成する」）と Sites 画面の「ミラーに追加」は、ダッシュボードの日本語の表示文字列に合わせた。
