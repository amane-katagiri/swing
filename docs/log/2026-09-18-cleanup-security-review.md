# 2026-09-18 肥大化・重複の整理とセキュリティレビュー

## 問題

- `src/agent.rs`（2345 行、本体 962 行）にプロセスのライフサイクル、Follow Set の更新、レプリカ報告の差分計算、サイト保存の 4 つの責務が同居していた。`web/app.js`（1853 行）も i18n 辞書・共通処理・4 画面・ルーターが 1 ファイルだった。
- `config.rs` の「環境変数 → TOML → 既定値」の分岐、`nostr.rs` の `fetch_events` + タイムアウトの骨格、`mirror.rs` の add / remove、`publish.rs` と `dashboard/api.rs` の入力検証、`app.js` の世代カウンタ付きロードやコピー用ボタンが、それぞれ手書きで繰り返されていた。
- ダッシュボードの publish だけが NIP-05 検証に無フィルタの `HttpNip05Verifier::new()` を使っていた。`site` に自分で DNS を握ったドメインを入れると内部ネットワークへ HTTPS リクエストを飛ばせ、接続エラーの文字列が `nip05.detail` にそのまま返るので内部ポートの探索にも使えた。
- アップロードにファイル数・階層数の上限が無く、`max_upload` の範囲内でも極小ファイルを大量に送って inode を消費させられた。

## 決めたこと

| 決定 | 理由 |
|---|---|
| `src/agent.rs` を `src/agent/`（`mod` / `lifecycle` / `follow` / `store` / `replicas` / `test_support`）に分ける。外から見えるパス `swing::agent::run` は保つ | 責務ごとに読めるようにする。テストの共有フィクスチャは `#[cfg(test)]` の `test_support` に寄せる |
| `web/app.js` を ES modules（`storage` / `i18n` / `util` / `ui` / `sites` / `webring` / `publish` / `settings` / `app`）に分ける。ビルドステップは足さない | `include_str!` で埋め込む方式のまま、`assets.rs` に定数とハンドラ、`mod.rs` に route を足すだけで済む。CSP は `default-src 'self'` のままでよい |
| 言語切替は `settings.js` が `swing:langchange` を投げ、`app.js` が受けて全画面を再描画する | `settings.js` から `app.js` の `applyLanguage` を直接呼ぶと循環 import になる |
| 世代カウンタは `createLoadGuard()` でカウンタ管理だけを共通化し、読み込み全体を包むラッパーにはしない | 呼び出し元ごとにローディング表示とエラー処理のタイミングが違う |
| ダッシュボードの publish も `HttpNip05Verifier::public_only()` を使う | agent が他人の `d` を検証するときと同じ扱いにする |
| API が返す `nip05.detail` は `unreachable` / `timeout` / `invalid_response` の分類だけにし、生のメッセージは `tracing::warn` に出す。CLI の表示は変えない | 内部ネットワークの探索に使える情報を返さない。CLI は操作者本人の端末なので詳細を出してよい |
| アップロードの上限は固定の定数（10,000 ファイル、32 階層、パス長 4096）にし、設定項目にはしない | 個人サイトの規模で足りる。設定を増やすほどの必要が無い |
| リクエストのタイムアウトは通常 120 秒、`/api/publish/upload` だけ 30 分 | アップロードの長い転送を切らない |
| `submit()` の先頭で Follow Set の対象判定をする。`apply_site_event` 側の判定も残す | relay が購読フィルタを無視して対象外のイベントを大量に送っても、タスクを作らない |
| `POST /api/publish`（サーバ側の `dir` を受け取る口）を削除する。画面は `/api/publish/upload` しか使っておらず、無認証のまま agent が読める任意のディレクトリを公開させられる口を残す理由が無い | ダッシュボードは無認証で、`dir` はベースディレクトリの許可リストなどなく任意の絶対パスを受け付けるため、到達できる相手が任意ディレクトリを公開させられる |

## 作ったもの

- `src/agent/` への分割。`submit()` の早期判定と、対象外イベント 50 件でタスク 0 件になることのテスト。
- `config.rs` の `resolve` / `resolve_typed`、`nostr.rs` の `RelayClient::fetch`、`mirror.rs` の `MirrorOp` + `apply_change` / `print_change_result`、`publish.rs` の `validate_site_fields` / `resolve_nip05_mode`（`dashboard/api.rs` からも呼ぶ）。
- `State::save` は先に直列化して `String` だけを `spawn_blocking` に渡す。`State` 全体の clone をやめた。
- 使われていない `pub` の縮小（`ipfs::mfs_mkdir`、`mirror::print_account_header`、`mfs::site_name`、`key::describe`、`state::remove_site`）、`process_env` の除去、`reqwest` の `json` feature の削除。
- `web/` の分割、`copyButton` / `appendLinksAndMessage` / `storedBadge` / `wireButtonGroup` / `createLoadGuard` への共通化、未使用の `copyText` と死んでいた CSS 2 件の削除、ダークテーマ変数の二重定義の解消、見出しコメントの削除。
- 言語切替で `cache.status`（Storage check の結果）が再描画されなかった不具合の修正。
- `nip05.rs` の `ErrorCategory` と `coarse_detail()`、`upload.rs` の上限、`tower-http` の `TimeoutLayer`、`Referrer-Policy: no-referrer` と `X-Frame-Options: DENY`、非 loopback bind または `allowed_hosts` 指定時の起動時警告。
- `dto::format_bytes` の `n * 10` を `u128` で計算する（`u64::MAX` 付近の容量設定でのオーバーフロー）。
- architecture.md、architecture/agent.md、architecture/dashboard.md、architecture/nip05.md を更新した。
- `POST /api/publish`（`api::PublishRequest`・`api::publish`）を削除した。`api::run_publish` / `PublishFields` / `PublishOutcome` は `POST /api/publish/upload` と共有のまま残す。

## 見送ったもの

- ヘッダ読み取りのタイムアウト。`axum::serve` に設定が無く、`hyper_util` への切り替えが要る。todo に足した。
- `config.rs` は重複を 2 つのヘルパーに集約したが、呼び出しが複数行に展開されるので行数は 1142 → 1226 に増えた。
- 後方互換のための処置は足していない。

## 検証

- `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（235 passed, 11 ignored）。
- 修正後に、経緯を知らない別のレビューで品質とセキュリティを見直した。品質は Critical 0 / High 0 / Medium 1（`format_bytes`、修正済み）、セキュリティは Critical 0 / High 0 / Medium 0 / Low 3（todo に足した）。
- 実ブラウザでの確認。relay と Kubo に届かない専用設定で `swing agent` を起動し、静的アセット 11 本のステータス・Content-Type・ヘッダ、不正な Host・クロスオリジン POST・`X-Swing-Dashboard` 無しの 403、GET 系 API の JSON、全画面の描画、言語とテーマの切替、Webring の 4 スタイル、publish の失敗表示を確かめた。コンソールエラーと CSP 違反は無かった。
- Kubo と relay に届く正常系（publish の成功、mirror の反映）と、`#[ignore]` の統合テストは今回は動かしていない。
