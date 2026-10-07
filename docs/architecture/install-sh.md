# Linux のインストールスクリプト（`packaging/linux/`）

[`../architecture.md`](../architecture.md) の一部。リリースの作り方は [`release.md`](release.md)、登録するサービスは [`service.md`](service.md)、設定とデータの既定の場所は [`config.md`](config.md#設定ファイルの場所)。

`packaging/linux/install.sh` は POSIX sh（`set -eu`）の 1 ファイルで、Linux の x86_64・aarch64 に `swing` と Kubo を入れる。リリースに `install.sh` として添えられ（[`release.md`](release.md)）、次のように使う。

```
curl -fsSL https://github.com/amane-katagiri/swing/releases/latest/download/install.sh | sh
curl -fsSL .../install.sh | sh -s -- --version v0.1.0
```

## オプション

| オプション | 動作 |
|---|---|
| `--version <tag>` | そのタグのリリースを入れる（`0.1.0` は `v0.1.0` として扱う）。`v` の後は英数字と `.`・`+`・`-` だけ。無ければ最新 |
| `--prefix <dir>` | 絶対パス。`<dir>/lib/swing` に置き、リンクを `<dir>/bin` に作る。既定は `$HOME/.local` |
| `--service` | 入れた後に `swing service install` を実行する。root では使えない（ユーザーサービスのため） |
| `--force` | `bin/swing` が自分の入れたものでなくても置き換える。`--uninstall` では system unit があっても続ける |
| `--uninstall` | 入れたものを消す（下記） |
| `--purge` | `--uninstall` と一緒にだけ使え、既定の設定・データのディレクトリも消す |
| `--yes`・`-y` | `--purge` の確認を省く |

取得先などはスクリプト先頭の定数で決まり、環境変数では変えられない。

| 定数 | 値 |
|---|---|
| `RELEASES_URL` | `https://github.com/amane-katagiri/swing/releases`。`SHA256SUMS`・アーカイブ・`install.sh` は `<RELEASES_URL>/latest/download`（`--version` ありなら `<RELEASES_URL>/download/<tag>`）から取る |
| `KUBO_BASE_URL` | `https://github.com/ipfs/kubo/releases/download/v<KUBO_VERSION>` |
| `KUBO_SHA512_AMD64`・`KUBO_SHA512_ARM64` | Kubo の Linux 向けアーカイブの SHA-512 |
| `SYSTEM_UNIT` | `/etc/systemd/system/swing.service`（アンインストール時と更新時に system unit を探す場所） |

`HOME` は絶対パスでなければ失敗する。

`curl` か `wget`、`tar`、`sha256sum`（無ければ `shasum -a 256`）、`sha512sum`（無ければ `shasum -a 512`）が要る。macOS では Homebrew を案内して、その他の OS・アーキテクチャではエラーで、どちらも非ゼロで終わる。ダウンロードは HTTPS に限る（`curl --proto '=https' --tlsv1.2`、`wget --https-only`。リダイレクト先も含む）。作るファイルとディレクトリがグループに書き込めないように、最初に `umask 022` にする。

## 入れる手順

1. `SHA256SUMS` を取得する。`--version` が無ければ、その中から `swing-<tag>-<target>.tar.gz`（target は `x86_64-unknown-linux-musl` か `aarch64-unknown-linux-musl`。`/` を含む名前は使わない）を探して、最新のタグを決める。
2. アーカイブを取得して SHA-256 を照合し、展開する。
3. `swing-uninstall.sh` の元になる `install.sh` は、ファイルとして実行されていればそれ自身、パイプなら `SHA256SUMS` に載っていればリリースから取得して照合する。載っていなければ警告して、アンインストーラは入れない。
4. `lib/swing` と `bin` の置き場所を検査する（下記）。
5. Kubo（`KUBO_VERSION`）を `<KUBO_BASE_URL>/kubo_v<version>_linux-<amd64|arm64>.tar.gz` から取得して、`KUBO_SHA512_<AMD64|ARM64>` と照合する。配布元の `.sha512` は取得しない。`lib/swing/ipfs` の `ipfs version --number` が `KUBO_VERSION` と一致すれば取得しない。版を上げるときの手順は [`kubo.md#kubo-のバージョン`](kubo.md#kubo-のバージョン)。
6. `bin/swing` が既にあり、自分が作ったリンク（`lib/swing/swing` を指す）でなければ、`--force` が無いかぎり、何も変えずに失敗する。
7. `lib/swing` と `bin` を作り（`mkdir -p`）、置き場所をもう一度検査する。
8. 動いているサービスを止める（下記）。
9. ファイルを `lib/swing/` に置く。同じディレクトリに `.<名前>.<pid>` で書いてから `mv -f` する（1 ファイルごとにアトミック）。
10. 前回の `manifest` に載っていて今回は無いファイル（[一覧の読み方](#一覧の読み方)に合うものだけ）を消し、`manifest` を書き直す。
11. `bin/swing` を `lib/swing/swing` へのシンボリックリンクにする（一時名で作って `mv -f`）。`current_exe()` は実体のパスを返すので、`ipfs` は `lib/swing` で見つかり（[`kubo.md`](kubo.md#バイナリの検出kubolocate_binary)）、サービスの `ExecStart` も `lib/swing/swing` になる。
12. `lib/swing/swing --version` が動かなければ警告する。`bin` が PATH に無ければ警告する。

どれかの検証が失敗したら、`lib/swing/` と `bin/` には何も書かない（ダウンロードと照合は置き換えの前にすべて終える）。一時ディレクトリは `trap` で消す。

## 置き場所の検査

`lib/swing` と `bin` のそれぞれについて、実在するいちばん深い祖先（そのもの、または親をたどったもの）から `/` までのすべてのディレクトリを、書かれたパスのままのものと `pwd -P` で解決したものの両方で `ls -ldn` で調べ、どれかに当たれば何も変えずに失敗する（`refusing to use <dir>: ...`）。どちらも葉から `/` へ向かって調べ、別のユーザーの持ち物に当たればその時点でそれを理由にする。書き込めるディレクトリは最初に見つけたものを覚えて調べ続けるので、両方あるときは別のユーザーの持ち物のほうを理由にする。グループかその他のユーザーが書き込めることが理由なら、`--prefix /opt/swing` のように自分か root しか書き込めない prefix を案内し、その prefix に `lib/swing/manifest` があれば先に `install.sh --uninstall --prefix <prefix>` で消すよう続けて案内する。シンボリックリンクそのものは調べない（その親は調べる）。

- 持ち主が root でも実行しているユーザーでもない
- グループかその他のユーザーが書き込める。ただし祖先が sticky ビット付き（`/tmp` など）なら許す。`lib/swing`・`bin` そのものは sticky でも許さない
- グループの書き込みは、そのグループが持ち主の個人グループ（`getent` で引いた名前が持ち主のユーザー名と同じで、ほかのメンバーがいない）なら許す。`getent` が無ければ許さない。持ち主が root でも、ほかのグループの書き込みは許さない（Debian の `root:staff 2775` の `/usr/local` も断る。そのときは `--prefix /opt/swing` などを使う）

インストールと更新では、この検査を `lib/swing` の中のものを実行する前（Kubo の版の確認・持ち主の確認）に済ませる。アンインストールも `lib/swing` について同じ検査で失敗する（下記）。

インストールと更新では、検査の後も `lib/swing` の中のものをパス（`<lib>/ipfs`・`<lib>/swing`・`<lib>/.swing-check.<pid>`）で実行し、書き込みもパスで行う。`swing service stop`・`start`・`install` はカレントディレクトリの `swing.toml` を設定として探す（[`config.md`](config.md#設定ファイルの場所)）ので、カレントディレクトリは変えない。

## 表示

`info`・`warning:`・`error:` の行と、データの場所・アンインストーラの場所・`--purge` の確認の表示では、埋め込むパスや名前の制御文字（改行・ESC などの C0、DEL、UTF-8 の C1）と、UTF-8 の不可視の書式文字（U+200B〜U+200F・U+2028〜U+202E・U+2060〜U+2064・U+2066〜U+206F・U+FEFF）をそれぞれ `?` に置き換える。ほかのユーザーが名前を付けられるディレクトリのパスが、偽の行や端末のエスケープシーケンスを出せないようにするため。`swing service status` の出力はそのまま中継する（`swing` 側で置き換える）。

## 置く物

`lib/swing/` に次を置き、名前を 1 行ずつ `manifest` に書く（`manifest` 自身は書かない）。

| ファイル | 元 |
|---|---|
| `swing`・`LICENSE`・`LICENSE-PixelMplus.txt`・`swing.example.toml`・`README.md` | リリースのアーカイブ |
| `ipfs`・`LICENSE-kubo-APACHE`・`LICENSE-kubo-MIT` | Kubo のアーカイブ（`LICENSE-APACHE`・`LICENSE-MIT`） |
| `swing-uninstall.sh` | `install.sh` のコピー |

### 一覧の読み方

`manifest` の行のうち、英数字と `.`・`_`・`-` だけからなり `.` で始まらない名前（`manifest` を除く）で、`lib/swing` の中の通常のファイルかシンボリックリンクのものだけを消す対象にする。それ以外の行（`/` を含むもの、ディレクトリなど）は無視する。

## 更新（再実行）

同じコマンドを再実行すると、その場で更新する。動いているサービスは、[登録の持ち主の確認](#登録の持ち主の確認)で `lib/swing` の `swing` を起動するものだと分かったときだけ扱う。そうでなければ止めも起動し直しもせず、`leaving the swing service as is; ...` と `swing` の出力を表示して更新を続ける。

| サービス | 置き換えの前 | 置き換えの後 |
|---|---|---|
| ユーザー unit（`$XDG_CONFIG_HOME/systemd/user/swing.service`）が active | 今入っている `swing service stop`（Kubo も止まる。`lib/swing/swing` が無ければ `systemctl --user stop swing`）。失敗したら何も変えずに失敗する | `--service` なしなら新しい `swing service start`、ありなら `swing service install` が登録し直して起動する |
| ユーザー unit が active でなく system unit が active、root で実行 | `systemctl stop swing` | `systemctl start swing` |
| system unit が active、root 以外 | 止めない | `sudo systemctl restart swing` を促す警告 |

- 置き換えの途中で失敗してサービスを止めたままになったときは、`swing service start` を案内する。
- サービスを使わずに動かしている `swing up` には触れない（古い実行ファイルのまま動き続ける）。

### 登録の持ち主の確認

`swing service status [--system] --points-into <lib>`（[`service/ownership.md`](service/ownership.md)）の終了コードで決める。0 なら `lib/swing` のもの、3（未登録）と 4（別の場所）ならそうでない、それ以外なら何も変えずに失敗する。

実行するのは、取得した新しい版の `swing` を `lib/swing/.swing-check.<pid>` に写したもの。確認の後で消す（失敗して終わるときも後始末で消す）。

## アンインストール

`install.sh --uninstall` と `lib/swing/swing-uninstall.sh`（ファイル名が `swing-uninstall.sh` なら `--uninstall` 扱い。`--prefix` が無ければ、置かれている `<prefix>/lib/swing` から prefix を決める）は同じ処理。

1. `manifest` が無ければ「no installation found」で失敗する。`lib/swing` を[置き場所の検査](#置き場所の検査)にかけ、どれかに当たれば何も実行せず何も消さずに失敗する（`--purge` でも同じ）。
   - 理由がグループかその他のユーザーから書き込めるディレクトリなら、そのディレクトリを持ち主しか書き込めないようにしてから再実行するよう案内する
   - どの理由でも、手で消すためのコマンドは表示しない（表示したパスは、そのディレクトリに書き込める人が実行前にすり替えられるため）
2. `cd -P` で `lib/swing` に一度だけ入り、以後の実行と読み書きはその中から相対パスで行う（`swing` は `./swing` で実行する）。入った先（実体）が root か実行ユーザーの持ち物でなければ、`manifest` が root か実行ユーザーの持ち物の通常のファイル（シンボリックリンクでない）でなければ、何も消さずに失敗する。検査の後で `lib/swing` のパスがすり替えられても、実行と削除の先は最初に入ったディレクトリから動かない。
   - `bin` の検査（`check_dir`）はアンインストールでは行わない。`bin` からは何も実行せず、`cd -P` で入った `bin` の中で、シンボリックリンクの `swing` のうち、リンク先の親を `cd -P` で解決すると入った `lib/swing` になる `/.../swing` を指すものだけを消すため（prefix をシンボリックリンク経由で入れ、`swing-uninstall.sh` が実体のパスから prefix を決めたときも消える）
3. `--purge` なら、削除先を決めて確認する（`/dev/tty` から読む。端末が無くて `--yes` も無ければ何も消さずに失敗する）。
4. system unit（`/etc/systemd/system/swing.service`）があれば、`sudo <lib>/swing service uninstall --system` を先に実行するよう案内して失敗する。`--force` なら続ける。ただし `./swing service status --system --points-into <lib>` の終了コードが 4（別の場所の `swing` を起動する unit）なら、触れずに続ける（`swing` が無ければ確かめられないので失敗する）。
5. ユーザー unit があれば `./swing service uninstall --only-from <lib>` を実行する（失敗したら何も消さずに終わる。`swing` が無ければ、手で消すよう warn に出して続ける）。unit が別の場所の `swing` を起動するものなら、`swing` がその旨を出して unit を残す。
6. 入った `lib/swing` の中で `manifest` に書いたファイル（「一覧の読み方」に合うもの。シンボリックリンクならリンクだけ）と `manifest` を消し、自分のリンクだった `bin/swing` を消す。最後に `lib/swing` の実体の親へ移り、空になっていればその名前のディレクトリを消す。
7. 設定とデータは残し、場所を表示する。`--purge` なら、ユーザーごとの既定の場所（[`config.md`](config.md#設定ファイルの場所)。実行時の `HOME` と `XDG_DATA_HOME` で決まる）だけを消し、`--config` や `SWING_CONFIG` で指していた別の場所には触れない。その場所が `/swing` で終わらなければ、手順 3 で何も消さずに失敗する。

## テスト

`packaging/linux/test-install.sh` が、偽の `swing`（呼び出しを記録し、unit の `ExecStart` を見て `status --points-into` と `uninstall --only-from` に答える）・偽の Kubo のアーカイブ・`SHA256SUMS` を一時ディレクトリから `file://` で配り、一時 `HOME` と偽の `systemctl` で `install.sh` を動かす。`install.sh` の先頭の定数と `curl` の `--proto` を `sed` で書き換えたコピーを使い、公開ネットワークには出ない。新規インストール・更新・アンインストールの各オプション、チェックサムの不一致、既存の `bin/swing`、`manifest` の読み方、置き場所の検査（root で実行したときだけ、または当てはまるグループがあるときだけ行うものを含む）、表示の置き換え、不正な引数を確かめる。

`shellcheck -s sh packaging/linux/install.sh packaging/linux/test-install.sh` が警告なしで通ること。
