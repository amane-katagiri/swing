# AGENTS.md

SWING（Nostr + IPFS 個人サイト相互ミラー）のリポジトリで作業するエージェント向けの規則。

## ドキュメントの構成

`docs/` は次の役割に分ける。役割をまたいで書かない。

| パス | 役割 | 更新のしかた |
|---|---|---|
| `docs/plan.md` | 初期実装計画（元の計画書） | 変更しない。歴史的資料 |
| `docs/protocol.md` | 実装非依存のプロトコル定義。他クライアント実装者向け | 更新: イベント仕様を変えるときは必ずここを先に更新し、architecture は実装側の記述に留める |
| `docs/architecture.md` と `docs/architecture/` | 現状のリファレンス。`architecture.md` は構成・CLI の一覧・設定・テストと各ファイルへの索引、`architecture/` は CLI・agent・signer・NIP-05・nostr・Kubo・up・gateway・service・tray・Docker・ダッシュボード（`dashboard.md` と `dashboard/`）・release の詳細 | 実装を変えたら同じ変更で必ず更新する。常に「今のコード」を記述する |
| `docs/mascot-guide.md` | Desktop 画面のマスコットのパックを作る人向けの手引き（コマの用意・描き方・確かめ方） | パック形式や検証規則を変えたら `docs/architecture/dashboard/mascot.md` と同じ変更で更新する。値の正本は architecture 側に置き、ここでは目安と手順だけを書く |
| `docs/site-guide.md` | `swing publish` でサイトを公開する人向けの、IPFS で配りやすい静的サイトにするためのチェックリスト | publish の処理（追加するファイルの範囲・CID の作り方）やミラー側のポリシー（上限・既定値・判定の順序）を変えたら同じ変更で更新する。値の正本は README の設定一覧と architecture 側に置き、ここでは目安と理由だけを書く |
| `docs/extensions.md` | 将来のイベント拡張の命名規約と予約表 | 新しい kind や d タグを使う前にここへ追記する |
| `docs/todo.md` | 残タスク | 着手したら消す、見つけたら足す。完了済みは log へ |
| `docs/log/YYYY-MM-DD-<slug>.md` | 実装ログ。その回で何を決め、何を作り、何を検証したか。`<slug>` は英語の kebab-case（本文は日本語でよい） | 追記のみ。過去のログは書き換えない。1 回の作業単位で 1 ファイル |
| `docs/assets/` | README に貼る画像。ブランド素材（ロゴとロゴタイプを並べたロックアップの SVG、ライト用とダーク用）と、ダッシュボードの Desktop 画面のアニメーション（`dashboard-desktop.webp`）と静止画（`dashboard-desktop.png`）、その撮影手順（`capture.md`・`capture-desktop.sh`） | ダッシュボードのロゴマークを変えたら `web/index.html` のインライン SVG から作り直す。ロゴタイプはロックアップ側が元の形で、`web/index.html` のものはナビ用に整数 px の格子へ描き直したもの（[`docs/architecture/dashboard/web.md`](docs/architecture/dashboard/web.md)）なので、形を変えるときはロックアップを先に直してからナビ用を描き直す。Desktop 画面の素材は画面の見た目を変えたら [`docs/assets/capture.md`](docs/assets/capture.md) の手順で撮り直す |
| `docs/release/README.md` | リリースのアーカイブに `README.md` として入れる、ビルド済みバイナリを使う人向けの短い手引き。画像とリポジトリ内への相対リンクは使わない（アーカイブ内で切れる） | README の導入手順（バイナリでの起動・サービス登録・同梱物）や、アーカイブの中身を変えたら同じ変更で更新する |
| `docs/examples/` | 参考実装（`publish.sh` など） | サポート対象ではない。プロトコル説明のための例示 |

- 実装の「現状」を知りたいときは `docs/architecture.md` とそこから辿れるファイルだけを読めば足りる状態を保つ。
- 「なぜそうしたか」は log に書く。architecture には結果だけ書く。
- README はユーザー向けの導入と使い方。仕様の詳細は architecture へリンクして重複させない。
- ドキュメント（log を含む）に環境依存の情報（特定マシンの絶対パス・ローカルのツール導入状況・サンドボックスやセッション固有の事情・ユーザー名など）を書かない。誰の環境でも通じる書き方にする。

## デモ環境

画面や動作を見せるときは `docker/demo/demo.sh up --seed` で外部ネットワークに出ないデモ環境を、サンプルのサイトとフォロー関係（深さ 5 まで）入りで上げる（ダッシュボードは <http://127.0.0.1:18082/>）。実 relay・実鍵・リポジトリ直下の `.env` は使わない。構成と外に出る経路は [`docker/demo/README.md`](docker/demo/README.md)。

## コーディング規則

- コメントは原則書かない。書くなら「自然な実装を避けた理由」を 1 行だけ。
- `cargo fmt --all` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` を通す。
- Windows 向けのコード（`#[cfg(windows)]` など）に触れたら `cargo xwin clippy --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings` も通す。Windows の実行ファイルは `cargo xwin build --release --workspace --target x86_64-pc-windows-msvc` で作る。素の `cargo check --target x86_64-pc-windows-msvc` は `ring` の C コンパイルで止まるので使わない。
- テストで公開 relay や公開 IPFS に接続しない。統合テストはローカルの Kubo / relay に限定し `#[ignore]` にする。
- 環境変数は `SWING_` 接頭辞で統一する。
- ダッシュボードに要素を足すときは、既存のクラス（`swing-panel`・`swing-btn`・`swing-status` など）と余白トークン（`--swing-space-*`）だけで組み、隣接する要素との余白を必ず確認する。状態表示は既存のもの（例: `#publish-status`）と同じ置き方にする。`swing-status` は上の余白を持たないので、ボタン列などの直後に置くなら余白を足す。見た目の変更はデモ環境（`docker/demo/demo.sh up`）で実際に表示してから報告する。
- 後方互換性のための処置（古い形式の state.json や設定を読むための `#[serde(default)]`・フォールバック・移行コードなど）は、入れる前に要否を確認する。確認せずに入れた場合は、何のための処置かを結果報告ではっきり伝える。
