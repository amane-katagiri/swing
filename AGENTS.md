# AGENTS.md

SWING（Nostr + IPFS 個人サイト相互ミラー）のリポジトリで作業するエージェント向けの規則。

## ドキュメントの構成

`docs/` は次の役割に分ける。役割をまたいで書かない。

| パス | 役割 | 更新のしかた |
|---|---|---|
| `docs/plan.md` | 初期実装計画（元の計画書） | 変更しない。歴史的資料 |
| `docs/protocol.md` | 実装非依存のプロトコル定義。他クライアント実装者向け | 更新: イベント仕様を変えるときは必ずここを先に更新し、architecture は実装側の記述に留める |
| `docs/architecture.md` と `docs/architecture/` | 現状のリファレンス。`architecture.md` は構成・CLI の一覧・設定・テストと各ファイルへの索引、`architecture/` は CLI・agent・NIP-05・Kubo・Docker・ダッシュボード（`dashboard.md` と `dashboard/`）の詳細 | 実装を変えたら同じ変更で必ず更新する。常に「今のコード」を記述する |
| `docs/extensions.md` | 将来のイベント拡張の命名規約と予約表 | 新しい kind や d タグを使う前にここへ追記する |
| `docs/todo.md` | 残タスク | 着手したら消す、見つけたら足す。完了済みは log へ |
| `docs/log/YYYY-MM-DD-<題名>.md` | 実装ログ。その回で何を決め、何を作り、何を検証したか | 追記のみ。過去のログは書き換えない。1 回の作業単位で 1 ファイル |
| `docs/examples/` | 参考実装（`publish.sh` など） | サポート対象ではない。プロトコル説明のための例示 |

- 実装の「現状」を知りたいときは `docs/architecture.md` とそこから辿れるファイルだけを読めば足りる状態を保つ。
- 「なぜそうしたか」は log に書く。architecture には結果だけ書く。
- README はユーザー向けの導入と使い方。仕様の詳細は architecture へリンクして重複させない。

## デモ環境

画面や動作を見せるときは `docker/demo/demo.sh up --seed` で外部ネットワークに出ないデモ環境を、サンプルのサイトとフォロー関係（深さ 5 まで）入りで上げる（ダッシュボードは http://127.0.0.1:18082/）。実 relay・実鍵・リポジトリ直下の `.env` は使わない。構成と外に出る経路は [`docker/demo/README.md`](docker/demo/README.md)。

## コーディング規則

- コメントは原則書かない。書くなら「自然な実装を避けた理由」を 1 行だけ。
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` を通す。
- テストで公開 relay や公開 IPFS に接続しない。統合テストはローカルの Kubo / relay に限定し `#[ignore]` にする。
- 環境変数は `SWING_` 接頭辞で統一する。
- 後方互換性のための処置（古い形式の state.json や設定を読むための `#[serde(default)]`・フォールバック・移行コードなど）は、入れる前に要否を確認する。確認せずに入れた場合は、何のための処置かを結果報告ではっきり伝える。
