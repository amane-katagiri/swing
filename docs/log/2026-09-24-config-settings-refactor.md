# 2026-09-24 config.rs/settings.rs のセキュリティ修正と分割

## 問題

- `src/settings.rs::write_atomic` が `std::fs::write` で tmp ファイルを作ってから `chmod` していた。umask 既定（多くは 0644）でファイルが作られる一瞬が生じるうえ、既存の `swing.toml` があればその権限をそのまま引き継いでいた。0644 のまま運用されている `swing.toml`（`[nostr].secret_key` を含む）は、`PUT /api/config` / `POST /api/setup` で 1 回書き換えるだけで世界読み取り可能なまま放置され続ける。`src/auth.rs::write_private_file`（`dashboard.token`・`remote-signer.json` が使う実装）は `OpenOptions::create_new` + `mode(0o600)` で最初から 0600 で作るので、同じ保証が無かった。
- `src/config.rs` の `build_config` が 1 関数 769 行で、8 セクション分の env/TOML/既定値解決が全部同じスコープに並んでいた。
- `config.rs` に「optional PathBuf を env-or-file から取る」ブロックが 5 回、`resolve()` で書けるのに手で if/else を書いているブロックが 4 回、カンマ区切りリストの分岐が 3 回、ほぼ同じ形の「env 値 X を弾く」テストが約 14 本あった。NIP-05 のモード名（`off`/`warn`/`require`）が `NIP05_MODE_NAMES`・`parse_nip05_mode`・`settings::nip05_mode_name` の 3 箇所に別々に書かれていた。

## 決めたこと

| 決定 | 理由 |
|---|---|
| `settings::edit::write_atomic` は `auth::write_private_file` をそのまま呼ぶ（tmp ファイル作成・0600・`sync_all`・rename は auth.rs 側の実装のまま）。既存ファイルの権限を引き継ぐ分岐は削除し、常に 0600 にする | 同じ「秘密鍵を含みうる設定ファイルを安全に書く」処理を 2 箇所に持つ理由が無い。挙動を変える（既存 0644 のファイルを次の書き込みで 0600 に締める）が、これは意図した修正 |
| `config.rs` を `src/config/{mod.rs, build.rs}` に分け、`build_config` をセクションごとの `resolve_nostr` / `resolve_ipfs` / `resolve_policy` / `resolve_agent` / `resolve_publish` / `resolve_dashboard` / `resolve_kubo` / `resolve_gateway` に分割する。公開パス（`crate::config::Config` など）は変えない | 1 関数 769 行は追わせるのに大きすぎる。セクションの依存関係（kubo が state_dir と max_total_storage に、ipfs と gateway が kubo.managed に依存）を見えるようにするため、呼び出し順を nostr → policy → agent → kubo → ipfs → publish → dashboard → gateway に変えた（`BTreeMap` へのキー挿入順は結果に影響しないため安全） |
| `settings.rs` を `src/settings/{mod.rs, example.rs, edit.rs}` に分ける（カタログ / `swing.example.toml`・`.env.example` 生成 / `update`・`setup` の書き込み）。公開パス（`settings::update` など）は `pub use` で保つ | 責務ごとに読める大きさにする |
| 5 個の optional-PathBuf ブロックを `resolve_opt_path`、4 個の手書き resolve を `resolve()` 呼び出しに、3 個のカンマ区切りリストを `resolve_csv_list`（`clean_file_items` と `default_when_empty` で各呼び出しの違いを吸収）に、約 14 本の「env 値を弾く」テストを `assert_env_rejects` に、`config_path`/`config_exists` を上書きするテストの前処理を `config_at` にそれぞれ集約した | 同じ分岐・同じ検証コードを何箇所にも書く理由が無い |
| NIP-05 モード名は `Nip05Mode::name()`（`const fn`）1 箇所にし、`NIP05_MODE_NAMES`（`dashboard/dto.rs` が参照するため公開パスは維持）と `parse_nip05_mode` はそこから作る。`settings::nip05_mode_name` は削除し呼び出し側で `.name()` を直接呼ぶ | 3 箇所に同じ対応表を書く理由が無い |
| `resolve`/`resolve_typed` の `#[allow(clippy::too_many_arguments)]` は残す | 呼び出し側 30 箇所超をスペック構造体経由に書き換えるコストが、9 引数を許容するコストを上回る |

## 作ったもの

- `src/config/mod.rs`（639 行）: 型定義・パーサ・`Config::load`・`resolve_config_path`・純粋なパーサのテスト。
- `src/config/build.rs`（1641 行）: セクションごとの `resolve_*` とその共通ヘルパー（`resolve`/`resolve_typed`/`resolve_opt_path`/`resolve_csv_list`）、`build_config`、`build_config` に依存するテスト一式（`assert_env_rejects` を使うよう書き換えたものを含む）。
- `src/settings/mod.rs`（790 行）: `Kind`/`Setting`/`SETTINGS`/`find`/`env_of`/`is_editable`/`InputValue`/`RawValue`/`raw_value`、カタログ自体のテスト 2 本。
- `src/settings/example.rs`（233 行）: `render_toml_example`/`render_env_example`/`ENV_NO_EFFECT_IN_COMPOSE`、生成物のドリフトテスト。
- `src/settings/edit.rs`（365 行）: `set_item`/`apply_items`/`check_not_env_sourced`/`load_document`/`write_atomic`（`auth::write_private_file` を呼ぶだけに縮小）/`update`/`setup`/`setup_keys`、`existing_file_permissions_are_tightened_to_0600`（0644 の既存ファイルが次の書き込みで 0600 になることを確認する新規テスト）を含む書き込み系のテスト。
- `.github/workflows/release.yml`: `dtolnay/rust-toolchain@master` → `@02cb101e...` (`# v1`)、`Swatinem/rust-cache@v2` → `@49a0bdc7...` (`# v2`)、`taiki-e/install-action@v2` → `@9983c65e...` (`# v2`) にコミット SHA 固定。`actions/*` は対象外のまま。

## 検証したこと

- `cargo fmt --check` / `cargo clippy --workspace --all-targets -- -D warnings`（`-j 2`） / `cargo test --workspace`（`-j 2`）が通ること（446 件成功、15 件 `#[ignore]`。分割前と同じテスト内容を包含し、パーミッションの新規テスト 1 本を追加）。
- 分割の前後で `build_config`/`build_config_from_str` の入出力（既定値・env 上書き・エラーメッセージ）が変わっていないこと（既存テストをそのまま、または `assert_env_rejects`/`config_at` 経由に書き換えて全て通した）。

## 見送ったこと

- `resolve`/`resolve_typed` の引数をスペック構造体にまとめて `#[allow(clippy::too_many_arguments)]` を外す案。呼び出し側が 30 箇所を超え、書き換えの見返りが薄いので見送った。
