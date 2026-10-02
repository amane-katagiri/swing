# Linux のインストールスクリプト（`packaging/linux/`）

[`../architecture.md`](../architecture.md) の一部。リリースの作り方は [`release.md`](release.md)、登録するサービスは [`service.md`](service.md)、設定ファイルの既定の場所は [`config.md`](config.md#設定ファイルの場所)。

`packaging/linux/install.sh` は POSIX sh（`set -eu`）の 1 ファイルで、Linux の x86_64・aarch64 に `swing` と Kubo を入れる。リリースに `install.sh` として添えられ（[`release.md`](release.md)）、次のように使う。

```
curl -fsSL https://github.com/amane-katagiri/swing/releases/latest/download/install.sh | sh
curl -fsSL .../install.sh | sh -s -- --version v0.1.0
```

## オプション

| オプション | 動作 |
|---|---|
| `--version <tag>` | そのタグのリリースを入れる（`0.1.0` は `v0.1.0` として扱う）。無ければ最新 |
| `--prefix <dir>` | 絶対パス。`<dir>/lib/swing` に置き、リンクを `<dir>/bin` に作る。既定は `$HOME/.local` |
| `--service` | 入れた後に `swing service install` を実行する。root では使えない（ユーザーサービスのため） |
| `--force` | `bin/swing` が自分の入れたものでなくても置き換える。`--uninstall` では system unit があっても続ける |
| `--uninstall` | 入れたものを消す（下記） |
| `--purge` | `--uninstall` と一緒にだけ使え、既定の設定・データのディレクトリも消す |
| `--yes` | `--purge` の確認を省く |

環境変数（テストのための差し替え）:

| 変数 | 既定 |
|---|---|
| `SWING_INSTALL_BASE_URL` | `https://github.com/amane-katagiri/swing/releases/latest/download`（`--version` ありなら `.../releases/download/<tag>`）。`SHA256SUMS`・アーカイブ・`install.sh` を置いたディレクトリの URL |
| `SWING_INSTALL_KUBO_BASE_URL` | `https://dist.ipfs.tech/kubo/v<KUBO_VERSION>` |
| `SWING_INSTALL_SYSTEM_UNIT` | `/etc/systemd/system/swing.service`（アンインストール時と更新時に system unit を探す場所） |

`curl` か `wget`、`tar`、`sha256sum`（無ければ `shasum -a 256`）、`sha512sum`（無ければ `shasum -a 512`）が要る。macOS では Homebrew を案内して、その他の OS・アーキテクチャではエラーで、どちらも非ゼロで終わる。

## 入れる手順

1. `SHA256SUMS` を取得する。`--version` が無ければ、その中から `swing-<tag>-<target>.tar.gz`（target は `x86_64-unknown-linux-musl` か `aarch64-unknown-linux-musl`）を探して、最新のタグを決める。
2. アーカイブを取得して SHA-256 を照合し、展開する。
3. Kubo（`KUBO_VERSION`）を `<KUBO_BASE_URL>/kubo_v<version>_linux-<amd64|arm64>.tar.gz` から取得して、同じ場所の `.sha512` と照合する。すでに `lib/swing/ipfs` があって `ipfs version --number` が `KUBO_VERSION` と一致すれば取得しない。
4. `swing-uninstall.sh` の元になる `install.sh` は、ファイルとして実行されていればそれ自身、パイプなら `SHA256SUMS` に載っていればリリースから取得して照合する。載っていなければ警告して、アンインストーラは入れない。
5. `bin/swing` が既にあり、自分が作ったリンク（`lib/swing/swing` を指す）でなければ、`--force` が無いかぎり、何も変えずに失敗する。
6. 動いているサービスを止める（下記）。
7. ファイルを `lib/swing/` に置く。同じディレクトリに `.<名前>.<pid>` で書いてから `mv -f` するので、置き換えは 1 ファイルごとにアトミックで、動いている実行ファイルにも書き込まない。
8. 前回の一覧に載っていて今回は無いファイルを消し、一覧（`manifest`）を書き直す。
9. `bin/swing` を `lib/swing/swing` へのシンボリックリンクにする（一時名で作って `mv -f`）。`swing` は自分の実体のパスで `ipfs` を探す（[`kubo.md`](kubo.md)）ので、リンク経由で動かしても隣の `ipfs` が見つかる。サービスの `ExecStart` も実体のパスになる。
10. `bin` が PATH に無ければ警告する。

どれかの検証が失敗したら、`lib/swing/` と `bin/` には何も書かない（ダウンロードと照合は置き換えの前にすべて終える）。一時ディレクトリは `trap` で消す。

## 置く物

`lib/swing/` に次を置き、名前を 1 行ずつ `manifest` に書く（`manifest` 自身は書かない）。

| ファイル | 元 |
|---|---|
| `swing`・`LICENSE`・`LICENSE-PixelMplus.txt`・`swing.example.toml`・`README.md` | リリースのアーカイブ |
| `ipfs`・`LICENSE-kubo-APACHE`・`LICENSE-kubo-MIT` | Kubo のアーカイブ（`LICENSE-APACHE`・`LICENSE-MIT`） |
| `swing-uninstall.sh` | `install.sh` のコピー |

## 更新（再実行）

同じコマンドを再実行すると、その場で更新する。サービスの扱い:

- ユーザー unit（`$XDG_CONFIG_HOME/systemd/user/swing.service`、既定 `~/.config/systemd/user/swing.service`）があり `systemctl --user is-active swing` が成功するなら、まず unit が `lib/swing` の `swing` を起動するものかを確かめる（下記「登録の持ち主の確認」）。そうでなければ止めも起動し直しもせず、`leaving the swing service as is; ...` と `swing` の出力を表示して更新を続ける。そうなら、置き換えの前に今入っている `swing service stop` で止め（Kubo も一緒に止まる）、止められなければ何も変えずに失敗する。置き換えたあと、`--service` が無ければ新しい `swing service start` で再び起動する。`--service` があれば `swing service install` が登録し直して起動する。
- 置き換えの途中で失敗してサービスを止めたままになったときは、`swing service start` で起動するよう案内する。
- system unit が動いていれば、同じく持ち主を確かめ（`--system` を付ける）、`lib/swing` のものでなければ触れない。`lib/swing` のもので root で実行しているなら、同じように `systemctl stop swing` と `systemctl start swing` で止めて起動する。root でなければ止めず、実行ファイルは置き換えるので、`sudo systemctl restart swing` で再起動するよう警告する。
- サービスを使わずに `swing up` を直接動かしているプロセスには触れない。動いているプロセスは古い実行ファイルのまま動き続けるので、止めて起動し直す。

### 登録の持ち主の確認

`swing service status [--system] --points-into <lib>`（[`service/ownership.md`](service/ownership.md)）の終了コードで決める。0 なら `lib/swing` のもの、3（未登録）と 4（別の場所）ならそうでない、それ以外なら何も変えずに失敗する。

実行するのは今入っている `swing` ではなく、取得した新しい版の `swing` を `lib/swing/.swing-check.<pid>` に写したもの。今入っている古い版はこのオプションを知らないことがあり、取得物を展開した一時ディレクトリ（`mktemp -d`）は `noexec` のこともあるため。確認の後で消す（失敗して終わるときも後始末で消す）。

## アンインストール

`install.sh --uninstall` と `lib/swing/swing-uninstall.sh`（ファイル名が `swing-uninstall.sh` なら `--uninstall` 扱い。`--prefix` が無ければ、置かれている `<prefix>/lib/swing` から prefix を決める）は同じ処理。

1. `manifest` が無ければ「no installation found」で失敗する。
2. `--purge` なら、削除先を決めて確認する（`/dev/tty` から読む。端末が無くて `--yes` も無ければ何も消さずに失敗する）。
3. system unit（`/etc/systemd/system/swing.service`）があれば、`sudo <lib>/swing service uninstall --system` を先に実行するよう案内して失敗する。`--force` なら続ける。ただし `<lib>/swing service status --system --points-into <lib>` の終了コードが 4（別の場所の `swing` を起動する unit）なら、触れずに続ける。
4. ユーザー unit があれば `<lib>/swing service uninstall --only-from <lib>` を実行する（失敗したら何も消さずに終わる）。unit が別の場所の `swing` を起動するものなら、`swing` がその旨を出して unit を残す。
5. `manifest` に書いたファイルと `manifest`、自分のリンクだった `bin/swing` を消し、空になった `lib/swing` を消す。
6. 設定とデータは残し、場所を表示する。`--purge` なら、既定のディレクトリ（Linux は `$XDG_DATA_HOME/swing`、無ければ `$HOME/.local/share/swing`。[`config.md`](config.md#設定ファイルの場所)）だけを消す。`--config` や `SWING_CONFIG` で別の場所を指していたものには触れない。`sudo` で `--prefix` を使う場合、この場所は実行時の `HOME` と `XDG_DATA_HOME` で決まる。

## テスト

`packaging/linux/test-install.sh` が、偽の `swing`（呼び出しを記録）・偽の Kubo のアーカイブと `.sha512`・`SHA256SUMS` をそろえた一時ディレクトリを `file://` で配り、一時 `HOME` で次を確かめる。公開ネットワークには出ない。`systemctl` は偽物に差し替える。

- 新規のインストール（パイプ実行・最新の解決・アンインストーラの取得・リンク・一覧・PATH の警告）、`--version`、`--service`、`--prefix`
- 更新（動いているサービスの停止と再起動、持ち主の確認を新しい版で行うこと、止まっていれば起動しない、別の場所の `swing` を起動するサービスには触れず確認用のファイルも残らないこと、同じ版の Kubo を取得しない）。偽の `swing` は unit の `ExecStart` を見て `status --points-into` と `uninstall --only-from` に答える
- `swing` と Kubo それぞれのチェックサム不一致で何も入らないこと
- 既存の `bin/swing` を `--force` なしでは壊さないこと、macOS の拒否
- アンインストール（データを残す、`--only-from` で自分の unit を消す、別の場所の `swing` の unit を残す、`--purge --yes`、確認できないときの拒否、`XDG_DATA_HOME`、system unit での拒否と `--force`、別の場所の `swing` の system unit では拒否しないこと）
- 不正な引数

`shellcheck -s sh packaging/linux/install.sh packaging/linux/test-install.sh` が警告なしで通ること。
