# 2026-09-24 セキュリティレビュー対応

## 問題

外部のセキュリティ/品質レビューで指摘された 8 件。

1. `src/dashboard/upload.rs` の `<state_dir>/upload/` 配下（アップロード用の一時ディレクトリ・展開先ディレクトリ・書き込むファイル）が umask 既定のまま作られていた。サイトのソースがアップロード中は他ユーザーから読める窓ができる。
2. `src/settings/mod.rs::raw_value()` と `src/dashboard/config_dto.rs::config_value()` が編集可能キーを手で別々に列挙しており、`raw_value()` にキーを足し忘れると黙って `None` を返す。加えて `settings`（設定カタログ）が `dashboard::dto`（HTTP 層）の関数を呼ぶ逆向きの依存があった。
3. `src/dashboard/guard.rs::split_host_port` が `[::1]xyz` のようなブラケット付きホストの `]` 以降のゴミを無視し、`::1` だけを取り出してしまっていた（ループバック判定を誤魔化せる余地）。
4. `src/dashboard/upload.rs::validate_relative_path` が Windows の予約デバイス名（`CON`/`NUL`/`COM1`など）や末尾がドット・スペースのセグメントを拒否していなかった。Linux で受け付けたサイトを Windows 側でチェックアウトすると壊れる。
5. `src/service.rs` の Windows タスク登録が `%TEMP%\swing-task-<pid>.xml` という予測可能な名前に `fs::write`（上書き可）で書いていた。同じ temp ディレクトリを使う別ユーザーがシンボリックリンクや先回りの空ファイルを仕込める。
6. `.github/workflows/release.yml` が `pip3 install --user ziglang` をバージョン指定なしで実行し、`actions/checkout`・`actions/upload-artifact`・`actions/download-artifact` がタグ参照（`@v7`/`@v8`）のままだった。
7. `src/nostr.rs` の `fetch_follow_set` と `ReportRelay::fetch_own_reports` に他の `fetch_*` と違って `.limit()` が無く、相手が返すイベント数に上限が無かった。
8. `src/dashboard/config_dto.rs`・`src/dashboard/mod.rs` に「何をしているか」を自然言語に書き下しただけのコメントが 2 箇所あった。

## 決めたこと

| 決定 | 理由 |
|---|---|
| ディレクトリ作成は「swing が新規作成した分だけ `0o700`、既存ディレクトリはそのまま」を徹底する。sync 側（`auth.rs`・`lock.rs`・`signer.rs` が使う `state_dir`）は `auth::create_private_dir_all`（`std::fs::DirBuilder::new().recursive(true).mode(0o700)`）に共通化。async 側（`dashboard/upload.rs` のアップロード用ディレクトリ）は tokio 版の同等ヘルパーを別途 upload.rs 内に置く | `DirBuilder::recursive(true)` は「既に存在するディレクトリは触らず、新規作成分だけ指定モードを付ける」という要件をそのまま満たす。sync/async で API が別なので共有はロジックのみにした |
| アップロードで書くファイルは `tokio::fs::OpenOptions` に `#[cfg(unix)] .mode(0o600)` を付けて作る（`tokio::fs::File::create` から置き換え） | ファイル自体も umask 任せにしない |
| `raw_value()` に「editable な `SETTINGS` キーは必ず `Some` を返す」ドリフト検知テストを追加する（既存のキー集合を洗い出したところ実際には漏れは無かった。将来キー追加時の回帰を防ぐのが目的） | 手で 2 箇所に同じキー集合を書く構造自体は残るが、テストで乖離を機械的に検出できれば実害は防げる。関数を 1 本化する再構成は `Config` の型ごとの参照が settings 側と dto 側で違う（dto は生の数値も返す）ため、この場で小さくは収まらないと判断した |
| `format_bytes`/`format_duration_secs` を `src/dashboard/config_dto.rs` から新設の `src/format.rs` に移し、`settings/mod.rs` は `crate::dashboard::dto::format_bytes` ではなく `crate::format::format_bytes` を直接呼ぶ | 「設定カタログが HTTP 層の関数を呼ぶ」という逆依存を消すのに十分安く済んだ。`dashboard/dto.rs` の re-export（`format_bytes`/`format_duration_secs`）はこの 1 箇所のためだけだったので削除した |
| `split_host_port` はブラケット `]` の後ろが空か `:<数字1桁以上>` のどちらでもなければ、その部分だけを host として使わず元のヘッダ全体を返す（既存の「閉じ括弧が無い」ケースと同じフォールバック） | 元のヘッダ全体はどんな正当なホスト名にも一致しないので、安全側に倒せる。非ブラケット形式（`host:port` でポートが数字以外）は元々同じフォールバックで弾けていたので変更不要、テストだけ追加した |
| `validate_relative_path` の各セグメントで、末尾ドット/スペースと Windows 予約デバイス名（大文字小文字無視、`.` より前の部分だけで判定するので `nul.txt` も含む）を拒否する。全プラットフォームで一律に拒否する | サイトは Linux で受け取って Windows でも展開されうるので、片方でしか壊れない名前を最初から通さない |
| Windows タスク登録の一時 XML は `crate::auth::random_hex(8)` を名前に使い、`OpenOptions::create_new(true)` で書く（成功後に削除する既存の流れは変えない） | ランダム名 + `create_new` で「他ユーザーが先回りしたパスを掴まされる」レースを閉じる。Windows 専用コードなのでこの環境ではビルド確認できていない（後述） |
| `ziglang` を pip でインストールする際は `==0.16.0`（2026-09-24 時点の PyPI 最新）を明示的に指定する。`cargo-zigbuild` は `pyproject.toml` で `ziglang>=0.9.0` としか宣言しておらず zig の上限は無いので、最新安定版で問題ないと判断した | バージョン固定なしだと zig の破壊的変更がリリースビルドを突然壊せる |
| `actions/checkout@v7`・`actions/upload-artifact@v7`・`actions/download-artifact@v8` をコミット SHA 固定に変える（`git ls-remote --tags` で該当タグが軽量タグだと確認済み。`^{}` 展開が無いのでタグ SHA がそのままコミット SHA） | 既存の `dtolnay/rust-toolchain`・`Swatinem/rust-cache`・`taiki-e/install-action` と同じ扱いに揃える |
| `fetch_follow_set` に `capped_limit(1, 2)`（1 作者・1 mirror_set の単一の置き換え可能イベント、relay が古い版を返す場合の 2 倍だけ余裕を見る）、`fetch_own_reports` に `capped_limit(budget::MAX_SITES_PER_AUTHOR_LISTED, 2)` を追加する | `fetch_own_reports` はレプリカ報告（サイトごとに 1 件のアドレス可能イベント）を全件フィルタなしで取っていた。1 作者が一覧できるサイト数は既存の `MAX_SITES_PER_AUTHOR_LISTED`（50）が上限として使われているので、それを流用し古い版への 2 倍を掛けた（`fetch_follow_sets` と同じ考え方） |
| 2 つの「何をしているか」コメントは、`dashboard/mod.rs::display_config` の方は削除（自明な getter の説明で「避けた実装」の理由になっていない）、`config_dto.rs` のテスト内コメントは「なぜこの分岐で return するか」（root 実行や 0o400 を無視する fs での早期リターン）に書き換えて残した | ルールは「自然な実装を避けた理由」だけを許すので、それに当たらない方は消し、当たる方は理由をはっきりさせた |

## 検証したこと

- `cargo fmt`・`cargo clippy -j 2 --workspace --all-targets -- -D warnings`・`cargo test -j 2 --workspace` すべて成功（460 passed, 15 ignored）。
- 新規/追加したユニットテスト: `auth::create_private_dir_all_*`（新規ディレクトリは 0700・既存は不変）、`lock::acquire_creates_state_dir_as_0700`、`signer::remote_signer_file_save_creates_state_dir_as_0700`、`upload::create_private_dir_all_makes_new_dirs_0700`・`create_private_file_makes_new_files_0600`、`upload::validate_relative_path_rejects_windows_reserved_device_names`・`_rejects_trailing_dot_or_space_segments`、`guard::extract_host_rejects_junk_after_the_bracketed_host`・`host_allowed_rejects_bracketed_host_with_trailing_junk`・`extract_host_rejects_non_digit_ports`、`settings::raw_value_covers_every_editable_key`、`format::format_bytes_prefers_the_largest_exact_unit`・`format_duration_secs_prefers_the_largest_exact_unit`（`config_dto.rs` から移設）。
- `src/service.rs` の Windows 分岐（項目5）は Linux 上ではビルドできない。`cargo check --target x86_64-pc-windows-msvc` を試したが、依存クレート `ring` のビルドスクリプトが MSVC のネイティブコンパイラを要求しこの環境には無いため失敗し、型チェックまで到達できなかった。コードは目視で確認済み（`super::*` 経由で `Context`/`Result` が既にスコープにあり、`crate::auth::random_hex` は `pub(crate)` で同一クレートから参照可能）だが、実機（Windows ランナー）でのビルドはこのセッションでは確認できていない。CI の `windows-latest` ジョブ（`check: true`）で次回ビルド時に確認されるはず。
- `docs/architecture.md`（取得と表示の上限の表）・`docs/architecture/dashboard.md`（state_dir の権限）・`docs/architecture/dashboard/http-api.md`（アップロードのパス検証規則）を実装に合わせて更新した。

## 保留・スキップしたこと

- `raw_value()`/`config_value()` の統合自体はしていない（ドリフト検知テストのみ追加）。理由は上表のとおり、`ConfigValue`（数値も返す）と `RawValue`（常に文字列/リスト）で戻り値の形が違い、両者を 1 関数に統合するには呼び出し側の変更が広がりすぎると判断したため。
- 後方互換性のための処置は今回追加していない（`raw_value()` のキー集合は変更前後で同じ）。
