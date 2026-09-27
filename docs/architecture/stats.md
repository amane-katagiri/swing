# リソース使用量の記録（`src/stats.rs`, `src/stats/process.rs`）

`swing up` が自分と Kubo の CPU・メモリと、Kubo の IPFS の通信量を一定の間隔で測り、メモリ上に持つ。読むのは [`GET /api/stats`](dashboard/http-api.md#get-apistats)・[`swing stats`](cli.md#stats)・ダッシュボードの Settings 画面（[`dashboard/web.md`](dashboard/web.md#リソース使用量)）。

## 測り方

- `up::run` がダッシュボードの `AppState` を作った直後に `stats::run` を起動し、`up::run` が終わるときに止める。セットアップモードでも動く。測る間隔は `SAMPLE_INTERVAL`（60 秒）で固定。起動直後に 1 回測り、以後は前回の測定が終わってから間隔を数える（呼ぶ側の数や頻度で測る回数は変わらない）。
- 1 回の測定（`Sample`）は、測った時刻（epoch 秒）と次の 3 つ。取れなかった値は `null`。
  - `swing`: 自分のプロセスの CPU 使用率とメモリ。
  - `kubo`: Kubo のプロセスの CPU 使用率とメモリ。`swing up` が Kubo を子プロセスとして起動しているとき（`[kubo].managed = true`）だけ測る。外部の Kubo では PID が分からないので常に `null`。
  - `traffic`: Kubo の RPC `stats/bw` の `TotalIn`・`TotalOut`（Kubo の起動からの累計バイト数）と、前回の測定との差から出した毎秒のバイト数（`in_per_sec`・`out_per_sec`）。Kubo が libp2p でほかの IPFS ノードとやりとりした量だけで、Nostr relay との通信と内蔵 gateway の HTTP は含まない。`stats/bw` が失敗すれば `null`（Kubo を `--offline` で動かしている場合も失敗する）。
- Kubo の対象は supervisor が `Recorder::set_kubo` で伝える。管理下の Kubo なら起動してヘルスチェックが通った時点で PID と RPC の URL を渡し、落ちたら外す。外部の Kubo なら RPC が応答した時点で URL だけを渡す。Kubo の準備ができる前の測定では `kubo` と `traffic` が `null` になる。
- CPU 使用率は前回の測定からの CPU 時間（ユーザー + カーネル）の増分を経過時間で割った値で、1 コアを 100% とする（マルチコアでは 100% を超える）。毎秒のバイト数も前回との差から出す。どちらも最初の測定と、次の場合は `null` にする: 前回に値が無い、Kubo の PID が前回と違う（再起動した）、累計が前回より減った。
- 直近 `HISTORY_LEN`（1440 件、60 秒間隔で 24 時間分）だけを持ち、古いものから捨てる。ファイルには書かないので、プロセスを再起動すると（`POST /api/restart` でのプロセス内の再起動を含む）履歴は消える。

## OS ごとの取り方（`stats::process::usage`）

| OS | CPU 時間 | メモリ |
|---|---|---|
| Linux | `/proc/<pid>/stat` の `utime + stime`（`sysconf(_SC_CLK_TCK)` で秒に直す） | 同じファイルの `rss`（ページ数 × `sysconf(_SC_PAGESIZE)`） |
| macOS | `proc_pidinfo(PROC_PIDTASKINFO)` の `pti_total_user + pti_total_system`（`mach_timebase_info` でナノ秒に直す） | 同じ構造体の `pti_resident_size` |
| Windows | `GetProcessTimes` のカーネル時間 + ユーザー時間 | `GetProcessMemoryInfo` の `WorkingSetSize` |

それ以外の OS では常に `null`。Windows では `PROCESS_QUERY_LIMITED_INFORMATION` でプロセスを開く。
