# 2026-09-16 初期実装

## 目的

[`plan.md`](../plan.md) の MVP（必須 9 項目）を動く形にする。Docker Compose、Kubo コンテナ、mirror-agent、NIP-51 Follow Set 取得、サイトイベント購読、CID 取得、Kubo pin、保存容量制限、publish。

## 技術選定と理由

| 選択 | 理由 |
|---|---|
| Rust (edition 2024) | 作業環境に Go が無く Rust があった。隣接プロジェクトも Rust |
| `nostr-sdk` 0.45 | 署名検証、NIP-51、addressable event、npub/nprofile パースが揃っている |
| Kubo RPC を `reqwest` で直接呼ぶ | 必要なエンドポイントが 5 つだけで、専用クレートを入れるほどではない |
| サイトイベント kind 35980 | 計画ではプレースホルダ。addressable 範囲（30000〜39999）で既存 NIP と重ならない番号を選んだ。設定で変更可 |
| lib + bin 構成 | 統合テストから `swing::` でモジュールを直接使うため。`#[path]` で二重に含めると clippy の dead_code に引っかかった |
| runtime を `debian:trixie-slim` に固定 | `rust:1.97-slim` が trixie ベースで、bookworm だと glibc が古く動かない可能性がある。`stable-slim` は世代が動くので明示固定 |

## 計画からの変更点と理由

| 変更 | 理由 |
|---|---|
| コマンド名 `site-mirror` → `swing` | プロジェクト名に合わせた |
| Follow Set の `d` を `site-mirror` → `swing` | 将来 relay 横断で `kind 30000, #d=swing` を引いて参加者を集めるとき、汎用名だと他用途と混ざる |
| 環境変数を `SWING_` 接頭辞に統一（`IPFS_API` → `SWING_IPFS_API`、`NOSTR_MIRROR_SET` → `SWING_MIRROR_SET`、`MAX_STORAGE_GB` → `SWING_MAX_TOTAL_STORAGE`） | 計画の compose 例をそのまま写した 3 つだけ接頭辞と単位（GB 整数）が違っていた |
| publish PoC のシェルスクリプトを `docs/examples/` へ | Rust の `swing publish` を先に完成させたので、シェル版はプロトコルの参考実装に格下げ |
| 設定 TOML を `[nostr] [ipfs] [policy] [agent] [publish]` に分割 | 計画はフラットなキーだったが、項目が増えた |
| ポリシーに stale 拒否を追加 | `min_update_interval = 0` のとき古いイベントを受理してしまう穴があった |
| `max_total_storage` を evict 後の合計で判定 | evict 前の合計だと `keep_versions` で消える分まで数えて不要に拒否する |
| コンテナを非 root（uid 1000）で実行 | レビュー指摘 |
| デフォルト relay を 5 つに変更（damus.io, nos.lol, primal.net, yabu.me, relay-jp.nostr.wirednet.jp） | NIP-11 で応答を確認できたもの。`relay.nostr.band` は NIP-11 が返らずインデクサ寄りなので除外 |

## 追加した機能（計画に無いもの）

- NIP-05 検証。agent 側（`[policy].nip05`）と publish 側（`[publish].nip05` / `--nip05`）、いずれも `off` / `warn` / `require`、既定 `warn`
- `swing mirror list / add / remove`。既存 Follow Set の他タグと暗号化 `content` を保持して再署名する
- `swing sites`。フォロー先の最新サイトイベントと pin 状況、NIP-05 結果の一覧
- サイトイベントの `d` 検証（空・253 バイト超・制御文字で拒否）と `url` 検証（不正なら `url` だけ無視）
- live イベントの送信元を Follow Set で再確認するゲート
- `state.json`（バージョン一覧と NIP-05 結果）と、起動時の Kubo `pin/ls` との突き合わせ
- `docs/extensions.md`（拡張時の命名規約と kind 予約表）
- `AGENTS.md` / `CLAUDE.md`（ドキュメント構成とコーディング規則）

## レビューで見つけて直した不具合

独立レビュー（コード）:

| 不具合 | 修正 |
|---|---|
| 購読で届いたイベントの送信元を Follow Set で確認していなかった。relay がフィルタを無視すると任意の pubkey の CID を pin する。unfollow 直後の遅延イベントで再 pin も起きる | `targets` を渡して Follow Set 外は warn して無視 |
| 新版の pin 成功前に旧版を unpin していた。pin 失敗で保持ゼロになる | pin → 成功後に evict の順に変更 |
| `max_total_storage` を `keep_versions` / `keep_days` の evict 前に判定していた | 全 evict 後の合計で判定 |
| `min_update_interval = 0` で古いイベントを受理 | stale 拒否を独立したルールに |
| `publish` でシンボリックリンクを黙って無視 | リンク先を辿る。循環はエラー |
| `publish` でサイト全体をメモリに読んでから送信 | `tokio::fs::File` でストリーミング |
| コンテナが root | `USER swing`（uid 1000）、`/data` の所有権を設定 |
| `size` 無しイベントで `files/stat` 失敗時にサイズ 0 で記録 | unpin して記録せず、次回再試行 |
| Follow Set 再取得のたびに kind 30000 が site event パーサに流れて warn を出す | 購読 ID と kind で事前に絞る |
| `NostrConfig` の `Debug` で秘密鍵が平文 | newtype で `<redacted>` |

ドキュメント監査:

| 不具合 | 修正 |
|---|---|
| `swing.example.toml` の `mirror_set` が `"swing_mirror_set"` | `"swing"` に修正 |
| `size` 無しイベントが `max_per_site` / `max_total_storage` を size=0 で通過 | pin → stat → 実サイズで `decide` → reject なら unpin |
| `publish` で `files/stat` 失敗を無言で握りつぶす | `! size unknown` を表示（非致命のまま） |
| spec の記述ズレ 20 件超（multipart のファイル名規則、NIP-05 のエラー条件、compose の詳細など） | 現状に合わせて修正し、その後 `architecture.md` に再編 |

## 検証

- `cargo test`: 95 passed、3 ignored（統合テスト）
- `cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`: クリーン
- `docker build -t swing .`、`docker run --rm swing --help`: 成功。コンテナ内 `id` は uid 1000
- 統合テスト（ローカル Kubo、ローカル `nostr-rs-relay`）: `add_dir` の CID が `ipfs add -Qr --cid-version=1` と一致、pin/unpin/stat/ls の往復、サイトイベントの publish と取得
- E2E（Docker 上に 2 つの Kubo を bootstrap 無しで直結し、ローカル relay と組み合わせ。修正前後で 2 回実施）:

| シナリオ | 結果 |
|---|---|
| A. Follow Set を publish | PASS |
| B. `swing publish` で v1 を publish | PASS |
| C. agent が Follow Set → サイトイベント → 別ノードから取得して pin。`ipfs cat` で内容一致、state.json に記録 | PASS |
| D. v2, v3 を publish。`keep_versions=2` で最古版が unpin。pin → unpin の順序をログで確認 | PASS |
| E. Follow Set が無い状態で起動し、後から追加。次のポーリングで拾う | PASS |
| F. Follow Set から外す → 全版 unpin、state が空に | PASS |
| G. 不正 CID、サイズ超過のイベントで skip し、クラッシュしない | PASS |
| H. リポジトリの `compose.yaml` をそのまま起動。`mirror` が `http://ipfs:5001` に 403 無しで到達、非 root で `/data/state.json` 書き込み | PASS |
| 追加: Follow Set 外の鍵からのサイトイベントは pin されない。kind 30000 の warn ノイズ消失 | PASS |

- `swing mirror add / list / remove`: ローカル relay で往復し、`title` タグと `content` が保持されることを確認
- relay の生存: 候補 relay に NIP-11 を投げて応答を確認（damus.io, nos.lol, primal.net, yabu.me, relay-jp.nostr.wirednet.jp, r.kojira.io, nostr-relay.h3z.jp は応答、relay.nostr.band は失敗）

テストと E2E はすべてローカルで閉じており、公開 relay や公開 IPFS には送信していない。

## 未検証・既知の制限

- NIP-05 の実 HTTP 経路は fake と純ロジックのテストのみ。実 TLS エンドポイント相手の統合テストは未実施
- `docs/examples/publish.sh` は構文チェックと `nak` / `ipfs` のフラグ確認のみで、実行していない
- Follow Set の暗号化 private 部分は読まない
- git は `git init` のみでコミットしていない。GitHub リポジトリも未作成

## ドキュメント整備（同日追記）

- `AGENTS.md` と `CLAUDE.md`（`@AGENTS.md`）を追加し、docs の役割分担（plan / protocol / architecture / extensions / todo / log / examples）を固定した
- `docs/spec.md` を `docs/architecture.md`（実装のリファレンス）に置き換え、参照をすべて付け替えた
- `docs/protocol.md` を新設し、他クライアント実装者向けの実装非依存なプロトコル定義（kind 30000 `d=swing`、kind 35980、検証規則）を architecture から分離した
- `docs/extensions.md`（拡張の命名規約と予約表）、`docs/todo.md`（残タスク 11 件）を追加
- README を初めて使う人向けに書き直した。です・ます調、太字なし、参考実装の説明は削除、MVP 外と今後の予定を 1 セクションに統合。`swing publish` の Docker での実行例は `docker compose run --rm -v "$PWD/public:/site" mirror publish ... /site` に修正
- `swing key generate` を追加（設定不要で nsec / npub / hex を表示）。README に鍵の作り方（`swing key generate`、`nak`、クライアント）と、publish が「`d` = 自分がルートを管理するドメイン」を前提にしていること、サブパス運用時の `--site` / `--nip05 off` の逃げ道を明記。protocol.md にも SHOULD として追記
