# リソース使用量の記録（`src/stats.rs`, `src/stats/process.rs`）

[`../architecture.md`](../architecture.md) の一部。`swing up` が自分と Kubo の CPU・メモリと、Kubo の IPFS の通信量を一定の間隔で測り、メモリ上に持つ。読むのは [`GET /api/stats`](dashboard/http-api/status.md#get-apistats)・[`swing stats`](cli/views.md#stats)・ダッシュボードの Settings 画面（[`dashboard/views.md`](dashboard/views.md#リソース使用量)）。

## 測り方

- `up::run` がダッシュボードの `AppState` を作った直後に `stats::run` を起動し、`up::run` が終わるときに止める。セットアップモードでも動く。
- 測る間隔は `SAMPLE_INTERVAL`（60 秒）で固定。起動直後に 1 回測り、以後は 60 秒の固定周期で測る。1 回の測定が周期を超えたときだけ、次の測定を後ろへずらす。
- 1 回の測定（`Sample`）は、測った時刻（epoch 秒）と次の 3 つ。
  - `swing`: 自分のプロセスの CPU 使用率（`cpu_percent`）とメモリ（`rss_bytes`）。
  - `kubo`: Kubo のプロセスの同じ 2 つ。`[kubo].managed = true` で PID が分かるときだけ測る。
  - `traffic`: Kubo の RPC `stats/bw` の `TotalIn`・`TotalOut`（Kubo の起動からの累計バイト数、`total_in`・`total_out`）と、前回の測定との差から出した毎秒のバイト数（`in_per_sec`・`out_per_sec`）。Kubo が libp2p でほかの IPFS ノードとやりとりした量だけで、Nostr relay との通信と内蔵 gateway の HTTP は含まない。
- Kubo の対象は supervisor が `Recorder::set_kubo` で伝える。渡すのは `KuboTarget`（`pid` と RPC の `IpfsClient`）。管理下の Kubo なら起動してヘルスチェックが通った時点で PID と、その起動の RPC の秘密付きのクライアントを渡し、落ちたら外す。外部の Kubo なら RPC が応答した時点でクライアントだけを渡す（PID は `None`）。
- CPU 使用率は前回の測定からの CPU 時間（ユーザー + カーネル）の増分を経過時間で割った値で、1 コアを 100% とする（マルチコアでは 100% を超える）。
- 直近 `HISTORY_LEN`（1440 件、60 秒間隔で 24 時間分）だけを持ち、古いものから捨てる。ファイルには書かないので、プロセスを再起動すると（`POST /api/restart` でのプロセス内の再起動を含む）履歴は消える。

### `null` になる条件

| 値 | `null` になるとき |
|---|---|
| `swing` | その OS でプロセスの使用量が取れない（下記）とき |
| `kubo` | Kubo の対象が無い（準備前・落ちた後）、PID が分からない（外部の Kubo）、使用量が取れないとき |
| `traffic` | Kubo の対象が無い、または `stats/bw` が失敗したとき（Kubo を `--offline` で動かしている場合を含む） |
| `cpu_percent` | 最初の測定、前回にそのプロセスの値が無い、Kubo の PID が前回と違う、CPU 時間が前回より減ったとき |
| `in_per_sec`・`out_per_sec` | 最初の測定、前回に `traffic` が無い、累計が前回より減ったとき |

## OS ごとの取り方（`stats::process::usage`）

| OS | CPU 時間 | メモリ |
|---|---|---|
| Linux | `/proc/<pid>/stat` の `utime + stime`（`sysconf(_SC_CLK_TCK)` で秒に直す） | 同じファイルの `rss`（ページ数 × `sysconf(_SC_PAGESIZE)`） |
| macOS | `proc_pidinfo(PROC_PIDTASKINFO)` の `pti_total_user + pti_total_system`（`mach_timebase_info` でナノ秒に直す） | 同じ構造体の `pti_resident_size` |
| Windows | `GetProcessTimes` のカーネル時間 + ユーザー時間 | `GetProcessMemoryInfo` の `WorkingSetSize` |

それ以外の OS では常に `null`。Windows では `PROCESS_QUERY_LIMITED_INFORMATION` でプロセスを開く。
