# Kubo デーモン（kubo/daemon.rs, kubo/orphan.rs）

[`../kubo.md`](../kubo.md) の一部。managed の Kubo の起動・ヘルス待ち・停止と、前の `swing` が残した Kubo の回収。呼ぶ順序と時間の上限は [`../up.md#managed`](../up.md#managed)。

## デーモンの起動（`kubo::Daemon::spawn`）

```
<bin> daemon --migrate=true --enable-gc --agent-version-suffix=swing
```

`Daemon::spawn(bin, repo, api)` は起動する Kubo の `ApiAccess`（[`../kubo.md#rpc-の認証managed-のみ`](../kubo.md#rpc-の認証managed-のみ)）を受け取り、その URL と秘密を付けるクライアントを `Daemon` に持たせる。ヘルス待ちと停止がこれを使い、`Daemon::ipfs()` で外にも渡す。

- 起動の前に `<repo>/api` があれば消す。
- `IPFS_PATH=<repo>`。stdin は `/dev/null` 相当、stdout/stderr は pipe。
- swing が強制終了されたときに Kubo を残さないための仕組みは OS ごとに違う。
  - すべての OS: `kill_on_drop(true)`。
  - Linux: `pre_exec` で `PR_SET_PDEATHSIG(SIGTERM)` を設定する。設定した直後に親の pid が spawn 前の swing の pid と違えば（設定の前に swing が消えていた）、exec せずに起動を失敗させる。このシグナルは spawn したスレッドの終了で届くので、`Daemon::spawn` は tokio のランタイムのワーカー上で呼ぶ（`spawn_blocking` の中では呼ばない）。
  - Windows: `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` の Job Object に子プロセスを割り当てる。ハンドルは `Daemon` が持つので、swing が終わると Kubo も一緒に落ちる。
  - macOS: 上の 2 つに当たるものは無く、次回起動時の[孤児回収](#kubopid-と孤児-kubo-の回収managed-のみ)に任せる。
- 標準出力・標準エラーは 1 行ずつ `target: "kubo"` のログ（`stream` フィールド付き）に流す。標準エラーに `repo.lock` か `someone else has the lock` を含む行が出たら覚えておく（`Daemon::saw_repo_lock_error()`。下記「[repo lock のヒント](#repo-lock-のヒント)」）。

## ヘルス待ち（`kubo::wait_healthy` / `Daemon::wait_healthy`）

どちらも `IpfsClient::peer_id()`（`POST <api_url>/api/v0/id`、リクエストタイムアウト 10 秒）を 1 秒間隔で呼び、指定した `timeout` を超えたらエラー。

- `kubo::wait_healthy(ipfs, timeout)`（unmanaged）: `id` が成功すれば成功。時間切れのエラーには最後の失敗の理由を付ける。
- `Daemon::wait_healthy(repo, timeout)`（managed）: 始めに `<repo>/config` の `Identity.PeerID` を読み、読めなければ何も送らずにエラーにする。毎回、まず子プロセスが終わっていれば `Kubo exited (<status>) before becoming healthy` でエラーにする。次に `<repo>/api` が自分の URL を指していなければ、その回は何も送らない（秘密を別のプロセスに送らないため）。`id` の `ID` が PeerID と一致したときだけ成功とし、別の ID を返す相手には成功しない（時間切れのエラーにその ID と期待した PeerID を入れる）。

## 停止（`Daemon::stop(grace)`）

1. `IpfsClient::shutdown`（`POST <api_url>/api/v0/shutdown`）をリクエストタイムアウト 5 秒（`SHUTDOWN_RPC_TIMEOUT`）で送る。成功・失敗・接続エラーのどれでも次に進む（リトライしない）。
2. 子プロセスの終了を `grace` まで待つ。終了すればここで成功。
3. まだ生きていれば、unix は SIGTERM を送って 10 秒（`SIGTERM_GRACE`）待ち、それでも終わらなければ SIGKILL。Windows は待たずに直接 kill する。kill 自体の失敗はエラーとして返す。

`kubo::daemon_stop_budget(grace)` はこの最悪時間（5 秒 + `grace` + unix は 10 秒。SIGKILL 後の終了待ちは数えない）を返す。`swing up` が渡す `grace` と停止全体の時間予算は [`../up.md#停止の時間予算`](../up.md#停止の時間予算)。

## `kubo.pid` と孤児 Kubo の回収（managed のみ）

`swing up` は `Daemon::spawn` の直後に `kubo::write_pid_file` で `<state_dir>/kubo.pid`（JSON: `pid`・`api_port`・`started_at`、Linux ではさらに `boot_id`）を `auth::write_private_file` で書く。書けなければ warn を出して続行する（その回は孤児回収の対象にならない）。

- `started_at` はその `pid` の開始時刻を OS ごとの方法（`proc::process_start_marker`。Linux は `/proc/<pid>/stat`、macOS は `LC_ALL=C` で `/bin/ps -o lstart=`、Windows は `GetProcessTimes`）で取った比較専用の文字列。
- Linux の開始時刻は起動からの経過クロック数で、再起動をまたぐと別のプロセスと一致しうるので、`/proc/sys/kernel/random/boot_id`（`proc::boot_id`）も記録する。macOS と Windows の開始時刻は絶対時刻なので `boot_id` は持たない。

`kubo::recover_orphan` は `swing.lock` を持っている間に 1 回呼ぶ（呼ぶ時点は [`../up.md#managed`](../up.md#managed)）。

1. `kubo.pid` が無ければ何もしない。読めない（空・JSON として壊れている・Linux で `boot_id` が無い・読み取りエラー）ときは、Kubo が repo lock で起動できなければこの repo を使っている残りの Kubo を止めるよう添えた warn を出し、何も kill せずに続ける（ファイルは次の `write_pid_file` で上書きされる）。
2. Linux で、記録の `boot_id` が今の boot id と違う（読めない場合を含む）なら、ファイルを消して終わる。
3. その `pid` の今の開始時刻を取り直し、プロセスがもう無いか、記録と一致しない（PID の再利用）なら、ファイルを消して終わる。
4. 一致したら記録の `api_port` に `shutdown` を送り（タイムアウト 3 秒、`ORPHAN_SHUTDOWN_RPC_TIMEOUT`）、2xx が返ればその `pid` の終了を最大 30 秒（`ORPHAN_SHUTDOWN_GRACE`）待つ。秘密は `kubo-api.json` の `port` が記録の `api_port` と同じときだけ付ける（読めなければ warn を出して秘密なしで送る）。
5. 2xx が返らないか、待っても終わらなければ強制終了する。unix は SIGTERM → 最大 30 秒（`ORPHAN_SIGTERM_GRACE`）→ SIGKILL → 最大 10 秒（`ORPHAN_KILL_WAIT`）、Windows は `taskkill /T /F` → 最大 10 秒。それでも終わらなければエラーを返し、`swing up` は Kubo を起動せずに終了する。

1〜3 で終わるときは API にも何も送らず kill もしない。`kubo.pid` と `kubo-api.json` をいつ消すかは [`../up.md#swing-up-のループuprs`](../up.md#swing-up-のループuprs)。

### repo lock のヒント

`recover_orphan` が拾えるのは自分が書いた `kubo.pid` だけで、`swing up` の管理下に無い Kubo が同じ repo を使っていると起動が失敗し続ける。`Daemon::wait_healthy` が失敗した時点で daemon が exit していて、標準エラーに repo lock の行が出ていたら、`another ipfs daemon seems to hold the Kubo repo lock; ...` を warn で出してから通常のバックオフに入る。
