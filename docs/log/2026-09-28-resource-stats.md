# リソース使用量の記録（`swing stats`）

`swing up` が動いている間の CPU・メモリ・通信量を後から見られるようにした。重いものは要らないという前提で、依存を増やさず、ファイルにも書かない形にした。

## 決めたこと

- **測るのは `swing up` 側で、間隔は 60 秒の固定。** 呼び出し側（ダッシュボード・CLI）は範囲を指定して読むだけにした。CPU 使用率も毎秒のバイト数も 2 回の測定の差からしか出せず、呼ばれたときに測ると値の意味が呼び出しの間隔で変わってしまうため。ダッシュボードを何枚開いても測る回数は変わらない。間隔は設定にしなかった（設定とその説明を増やすほどの理由が無い）。
- **保持はメモリ上の 24 時間分（1440 件）だけ。** 永続化は要らないと判断した。再起動で履歴は消える。
- **測る項目は `swing` と Kubo の CPU・メモリと、Kubo の IPFS の送受信。** 通信量は Kubo の `stats/bw` の累計の差から出す。Nostr relay との通信と内蔵 gateway の HTTP は数えない。ミラーの取得と配信で重くなるのはほぼ libp2p 側なので、gateway にカウンタを足すのは見送った。Kubo のリポジトリの容量は統計というよりストレージの話なので入れなかった。
- **プロセスの情報は `sysinfo` クレートを使わず OS ごとに自前で読む。** Linux は `/proc/<pid>/stat`、macOS は `proc_pidinfo`、Windows は `GetProcessTimes`・`GetProcessMemoryInfo`。どれも 1 回の呼び出しで済み、追加の依存は `windows-sys` の `Win32_System_ProcessStatus` だけ。
- **外部の Kubo では Kubo の CPU・メモリは測らない。** PID が分からないため。通信量は RPC で取れるので測る。
- CPU 使用率は 1 コアを 100% とする（`top` と同じ）。
- 画面の説明は測り方（誰が測り、どこに持つか）を書かず、「1 分ごとの記録を最大 24 時間さかのぼって見られる」ことだけにした。CPU の 1 コア基準の注釈も出さない。Kubo の CPU・メモリの注釈は、サンプルの有無からではなく `[kubo].managed` から出し（API の `kubo_managed`）、外部の Kubo のときだけ「SWING の管理外のため取得できない」と出す。Kubo の起動前に `null` になるサンプルで誤って出さないため。

## 作ったもの

- `src/stats.rs`（測定のループ・履歴・`swing stats` の表示）と `src/stats/process.rs`（OS ごとのプロセス情報）。
- `GET /api/stats?since=<epoch>`。agent の準備状態に関わらず、セットアップモードでも応答する。
- `swing stats [--last <duration>] [--json]`。`now`・`avg`・`max` の表を出す。
- ダッシュボードの Settings 画面に「リソース使用量」のパネル。画面を表示している間だけ 60 秒ごとに差分を取りにいく。
- 表示用に `format::format_bytes_approx`（ダッシュボードの `formatBytes` と同じ丸め）を足した。既存の `format_bytes` は設定値向けで、割り切れる単位を選ぶので計測値には向かない。

## 確かめたこと

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace`、Windows 向けの `cargo xwin clippy`。
- デモ環境で `swing stats` と Settings 画面のパネルを表示した。デモの Kubo は `--offline` で動いていて `stats/bw` が失敗するので、通信量は `-` のまま（外部の Kubo なので Kubo の CPU・メモリも `-`）。
- `--offline` を付けずにネットワークから切り離して起動した Kubo v0.43.1 で、`stats/bw` が `{"TotalIn":0,"TotalOut":0,"RateIn":0,"RateOut":0}` を返すことを確かめた。

## 残したこと

- macOS 向けのコードはこの環境ではコンパイルできていない（`ring` のビルドで止まる）。macos-check のワークフローで確かめる。
- 公開 IPFS につないだ Kubo の実測は `docs/todo.md` のまま。
