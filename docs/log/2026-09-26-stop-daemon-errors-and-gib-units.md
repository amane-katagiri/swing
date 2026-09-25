# Kubo 停止失敗の扱いの統一と容量の GiB 表記

## Kubo の停止失敗（`stop_daemon`）

managed の `run_managed` で Kubo を止める経路のうち、親トークンの cancel だけが `Daemon::stop` の失敗を `?` で伝播し、`up::run` → `main` で exit 1 になっていた。ほかの経路（ヘルス待ち中の cancel、unhealthy、agent の Ok、バックオフ中の cancel）は `let _ =` で黙って捨てていた。

親トークンの cancel は SIGTERM だけでなく、`swing stop`・トレイ（`/api/shutdown`）、`/api/restart`、setup 後の再起動も通る。ここで exit 1 になると、`swing stop` したのに systemd（`Restart=on-failure`）や launchd（`KeepAlive`）が再起動したり、プロセス内再起動がプロセス終了になったりする。停止に失敗しても swing 側にできることは無く、終わり方を変える理由が無いので、全経路で「warn を出して経路どおりに続ける」に揃えた。

- `stop_daemon` は `Result` を返さないようにし、失敗は中で warn にする。`let _ =` の経路も黙って捨てずに同じ warn が出る。
- `stop_daemon` は `Daemon::stop` が失敗しても `kubo.pid` を消していた。Kubo が残っていると次の `recover_orphan` が手がかりを失うので、成功したときだけ消し、失敗したら残して次回の孤児回収に任せる。プロセス内再起動でも `run_managed` の冒頭で `recover_orphan` が走るので、同じプロセスのうちに回収される。
- pid ファイルの扱いは `settle_stop(result, state_dir)` に切り出し、実 Kubo なしでテストした（失敗なら残る、成功なら消える）。

## 容量の単位（`KiB`/`MiB`/`GiB`/`TiB`）

`parse_size` は `GB` などを 1024 基数で読み、`GiB` 系は受け付けなかった。compose の `ipfs` コンテナは `SWING_KUBO_STORAGE_MAX` の文字列をそのまま `Datastore.StorageMax` に渡し、Kubo（go-humanize）は `GB` を 10 進、`GiB` を 1024 基数で読むため、同じ `100GB` でも managed と compose で約 7% ずれていた。一方で `.env` に `100GiB` と書くと swing が起動しなかった。

- `parse_size` に `KiB`/`MiB`/`GiB`/`TiB` を足した（大文字小文字は区別しない。既存の単位と同じ）。`KB`/`MB`/`GB`/`TB` は従来どおり 1024 基数のまま受け付ける。これは既存の意味の維持で、互換のための処置ではない。
- 設定カタログ（`src/settings/mod.rs`）の例と compose.yaml・デモの既定値を `GiB` 表記にし、`swing.example.toml`・`.env.example` を再生成した。`kubo.storage_max` の説明に「GiB 系で書くと compose の Kubo でも同じ値になる」ことを足した。
- docs（architecture.md の容量の書式、docker.md の Kubo の設定、http-api.md の例）と README を `GiB` 表記にし、`GB` で書いたときは compose だけ 10 進になることを書いた。

### 表示（`format_bytes`・`formatBytes`）も GiB 表記にした

`crate::format::format_bytes` と `web/util.js` の `formatBytes` は 1024 基数なのに `GB` と表示していた。入力の表記を `GiB` に揃える以上、表示だけ `GB` のままだと「ダッシュボードに 100 GB と出ている値を compose の `.env` に `100GB` と写すと 10 進になる」というずれの入口が残る。影響を確認したところ、

- `format_bytes` はダッシュボードの設定一覧の `display` と、編集フォームの初期値になる `raw` に使われている。`raw` は `parse_size` に戻るが、`"100 GiB"` は新しいパーサでそのまま読めるので往復は壊れない。
- 変わるテストは `format.rs` の単体テストと `dashboard/api.rs` の `raw` の比較 1 件だけ。i18n の文言に単位は含まれていない。

ので、どちらも `KiB`/`MiB`/`GiB`/`TiB` 表記にした（計算は変えていない）。

## 検証

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`（生成物と checked-in の `swing.example.toml`・`.env.example` の一致を含む）
