# インストーラー一式の見直し（品質とセキュリティ）

## 背景

既定の設定ファイルの場所から自分の登録だけを消すところまで（`install.sh`・Windows のインストーラー・Homebrew の formula・それぞれのワークフロー、`service/ownership.rs`・`config` の既定の場所・アップロードのパスの衝突の判定）を、重複・不要なコード・攻撃者の目で見直した。

## 直したもの

| 重さ | 場所 | 内容 | 対応 |
|---|---|---|---|
| 高 | `service/windows.rs` の Run キーの読み取り | `RegGetValueW` に `RRF_RT_REG_SZ \| RRF_RT_REG_EXPAND_SZ` を `RRF_NOEXPAND` なしで渡していた。この組み合わせは値の型にかかわらず `ERROR_INVALID_PARAMETER`（87）になるので、トレイの値がある（既定のインストールでは常にある）と `status --points-into` と `uninstall --only-from` が必ず失敗していた。上書きは「自分の登録でない」とみなして登録し直さず、アンインストールはタスクと Run キーの値を消したファイルを指したまま残す | `RRF_RT_REG_SZ` だけにした（`REG_EXPAND_SZ` は展開した値で読める）。文字列でない型（`ERROR_UNSUPPORTED_TYPE`）は読み取れない登録として扱う。Wine 上で 32 ビットの小さなプログラムを動かし、元の組み合わせが `REG_SZ`・`REG_EXPAND_SZ`・`REG_DWORD` のどれでも 87 を返し、`RRF_RT_REG_SZ` だけなら前の 2 つは成功・`REG_DWORD` は 1630 になることを確かめた |
| 中 | `install.sh` の置き場所 | `lib/swing` やその親が別のユーザーの持ち物か、その他のユーザーが書き込めると、root で動かしたときに、予測できる一時名（`.<名前>.<pid>`）へ仕込んだシンボリックリンク経由で任意のファイルを上書きされる、`manifest` を書き換えられて任意の名前を消される、`lib/swing` の `ipfs`・`.swing-check.<pid>`・`swing` を差し替えられて root で実行される | `lib/swing` と `bin` のそれぞれについて、書かれたパスと解決したパスの両方で、`/` までのディレクトリの持ち主が root か実行しているユーザーで、その他のユーザーが書き込めないことを確かめる。祖先の sticky ビット付き（`/tmp` など）は許し、`lib/swing`・`bin` そのものは許さない。`lib/swing` の中のものを実行する前と `mkdir -p` の後に行う |
| 中 | `install.sh` の `manifest` | 中身をそのまま信じて `rm -f "$LIB/<行>"` していた。`/` を含む行は飛ばしていたが、`.`・`..`・ディレクトリの名前で `rm` が失敗し、`set -e` でアンインストールが途中で止まった | 英数字と `.`・`_`・`-` だけで `.` で始まらない名前の、通常のファイルかシンボリックリンクだけを消す。読み方を 1 つの関数にまとめ、更新とアンインストールで使う |
| 中 | `install.sh` の Kubo の照合 | dist.ipfs.tech の `.sha512` を同じ配布元から取って照らしていたので、配布元が差し替えられると防げない（Windows のインストーラーは固定していた） | Linux の amd64・arm64 の SHA-512 をスクリプトに固定し、`.sha512` は取得しない。`kubo::tests::install_sh_pins_the_same_kubo_version` で `KUBO_VERSION` と固定値の形を確かめる |
| 低 | `install.sh` の環境変数 | テスト用の `SWING_INSTALL_BASE_URL`・`SWING_INSTALL_KUBO_BASE_URL`・`SWING_INSTALL_SYSTEM_UNIT` を本番のスクリプトが読んでいたので、`sudo -E` などで root の実行に持ち込まれると取得先や system unit の場所を変えられた | スクリプト先頭の定数にした。テストは定数を `sed` で書き換えたコピーを動かし、`latest/download` をシンボリックリンクで切り替える |
| 低 | `install.sh` の入力 | `--version` の値と、`SHA256SUMS` から選ぶアーカイブ名をそのままパスに使っていた | `--version` は `v` の後を英数字と `.`・`+`・`-` に限り、アーカイブ名は `/` を含めば使わない |
| 低 | `install.sh` の `HOME` | 相対パスの `HOME` だと、`--purge` の削除先が作業ディレクトリからの相対パスになった | 絶対パスでなければ失敗する |
| 低 | `service/ownership.rs` のパスの比べ方 | `..` がドライブや UNC のルートを越えて上がれたので、`\\server\share\..\..\C:\...` のような登録が `C:\...` の下に見えた | `..` はルートより上に上がらない。UNC のサーバー名の直後の `..` は比べられないものとする |
| 低 | `swing.iss` のインストール先 | `;` を含むフォルダーだと `Path` に足した値が 2 つに割れ、後ろ半分が相対パスの項目になる（作業ディレクトリのファイルが実行されうる）。`%` を含むと `REG_EXPAND_SZ` で展開されて別のパスになる | どちらかを含むフォルダーは、フォルダーを選ぶ画面と準備の段階（サイレントの `/DIR=` も）で `InvalidAppDir` を出して受け付けない |
| 低 | `packaging/homebrew/render.sh` | タグと URL の基点を formula の Ruby の二重引用符の文字列に埋めるので、`#{...}` や `"` を含むタグで Ruby のコードになった（タグを打てるのはリポジトリに書ける人だけ） | タグは英数字と `.`・`_`・`/`・`+`・`-` に限り、URL の基点は `"`・`\`・`#`・空白を含めば失敗する |
| 低 | アップロードのパス | Windows の予約デバイス名のうち `COM¹`〜`³`・`LPT¹`〜`³`・`CONIN$`・`CONOUT$` と、拡張子の前に空白のある `nul .txt` を通していた。大文字小文字の比べ方では別だがファイルシステムの上では同じになる、ファイルとディレクトリの組み合わせが Windows では 500 になりえた | 予約名を足し、拡張子の前の空白を無視して比べる。ファイルの作成が既存のディレクトリに当たったら 400 の衝突にする |

## 品質

- `swing.iss`: `WaitForSwingExit` と `WaitForTrayExit` が同じ形だったので `WaitForExit(Tray, Seconds)` にまとめた。`[CustomMessages]` を言語の組ごとに並べ直した。
- `service/windows.rs`: Run キーの読み取りは、同じ `RegGetValueW` の呼び出しと結果の分岐を 2 回書いていたのを 1 つのループにした。
- `install.sh`: `manifest` の読み方の重複を 1 つにし、Kubo の `.sha512` の取得をやめた。
- 足したコードの説明だけのコメント・経緯のコメントは無かった。残っているコメントは「自然な書き方を避けた理由」のものだけ。
- `build.ps1` の配布元の `.sha512` との照合は、固定値との照合があれば安全性には足さないが、固定値が古いときに分かりやすく失敗するので残した。

## 直さなかったもの

| 場所 | 内容 | 理由 |
|---|---|---|
| `install.sh` の `SHA256SUMS` | アーカイブと `install.sh` の照合に使う `SHA256SUMS` は同じ GitHub のリリースから取るので、転送中の破損や切り詰めは防げるが、リリースやアカウントそのものの差し替えは防げない | 署名か attestation が要り、鍵の管理を決める必要がある。残タスクの attestation の項目に足した |
| `install.sh` の `curl \| sh` の途中切れ | 途中で切れたスクリプトが一部だけ実行される | すでに処理を関数にまとめ、最後の `main "$@"` で呼んでいる |
| `install.sh` の途中の失敗 | `set -eu` で途中で止まったときの状態 | ファイルは 1 つずつ一時名から `mv -f` で置き換え、`manifest` は最後に書き直す。サービスを止めたまま終わったら起動の方法を出す。再実行で直る |
| `install.sh` の展開 | 照合済みのアーカイブを `tar` で展開する | 照合の拠り所がリリースなので、アーカイブの中身を疑っても守れるものが増えない |
| `install.sh` の `/dev/tty` | `--purge` の確認を `/dev/tty` から読む | パイプ実行でも利用者に聞けるのはここだけで、読めなければ `--yes` を求めて何も消さない |
| `install.sh` のグループの書き込み権 | 置き場所の検査はグループの書き込み権を見ない | umask が `002` の環境では自分で作ったディレクトリもグループに書けるので、見ると普通のインストールが通らない |
| `swing.iss` の WMI | `ExecutablePath` で `{app}` のプロセスを探す問い合わせ | `LIKE` ではなく `=` で比べ、`\` と `'` をエスケープしているので、`%`・`_`・`[` を含むパスでも別のものに当たらない。別のユーザーのプロセスは `ExecutablePath` が見えず、終了もできない |
| `swing.iss` の `Path` | ユーザーの `Path` の読み書き | `RegQueryStringValue` は `REG_EXPAND_SZ` を展開せずに返し、`REG_EXPAND_SZ` で書き戻すので `%VAR%` を含む項目は保たれる。ほかの項目は残し、大文字小文字と末尾の `\` を無視して完全一致で比べる。もとが `REG_SZ` なら `REG_EXPAND_SZ` に変わり、アンインストールで空の項目が詰められるが、どちらも害は無い |
| `swing.iss` の引数 | `{app}` を引数に入れる `"…"` の引用 | Windows のパスは `"` を含めず、`AllowRootDirectory` が既定の `no` なので `{app}` が `\` で終わって閉じ引用符を打ち消すことも無い |
| `swing.iss` のダッシュボードの URL | `ShellExec` で開く URL | 自分の `swing.exe` が出したもので、`http://` か `https://` で始まるものだけを開く。`ShellExec` はシェルを通さない |
| `swing.iss` の `{tmp}` と DLL の探索 | 新しい `swing.exe` を `{tmp}` に取り出して実行する | `{tmp}` は Inno Setup が実行ごとに作るランダムな名前のディレクトリで、中には Inno Setup 自身のファイルしかない。セットアップ自身の DLL の読み込みは Inno Setup が抑えている |
| `build.ps1` | 取得物の照合 | Inno Setup のインストーラーは SHA-256、Kubo の zip は SHA-512 を固定して照合してから使う |
| `config` の既定の場所 | 環境変数（`XDG_DATA_HOME`・`HOME`・`LOCALAPPDATA`）を信じる | 相対パスは無視する。作るディレクトリは 0700、空の `swing.toml` は `create_new` の 0600 で、既にあれば（シンボリックリンクでも）触れない。どれも本人の環境 |
| `service/ownership.rs` のシンボリックリンク | `canonicalize` したものどうしも比べるので、シンボリックリンク経由の登録も「下にある」になる | 登録は本人のファイルで、書き換えられるのは本人だけ。同じ実行ファイルを指すので意図どおり |
| `service/ownership.rs` の大文字小文字 | Windows では Rust の `to_lowercase` で比べ、NTFS の大文字化の表とは一部（ケルビン記号など）違う | 登録は本人のもので、ずれても判定が保守的か、同じ人のものどうしの取り違えにしかならない |
| アップロードの大文字小文字・正規化 | Rust の小文字化とファイルシステムの比べ方（`ß`・`ſ`・トルコ語の `i`、macOS の NFC と NFD）がずれる | `create_new` なので上書きは起きず、ずれは 400 か、重複としての拒否（安全側）になる |
| ワークフロー | 権限・入力・アクション・成果物 | `run:` に入る `github.ref_name` は環境変数経由、`${{ }}` で埋めるのは固定の `matrix` の値だけ。権限は全体が `contents: read` で、`release` ジョブだけ `contents: write`、`image` ジョブだけ `packages: write`。アクションはコミット SHA と版のコメントで固定。成果物は同じ実行の中でだけ受け渡し、リリースを作るのはタグの実行だけ。確認用の 2 つは手動実行だけで、tmate は実行した人だけが入れ、トークンは読み取りのみ |
| release ワークフローの zig | `pip3 install --user ziglang==0.16.0` をハッシュなしで入れる | この見直しの範囲より前からある。PyPI は同じファイル名の差し替えを許さないので当面は版の指定で足りるとし、残タスクにした |

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo xwin clippy --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings`・`cargo test --workspace` が通ることを確かめた。足したテスト: `..` がルートを越えない比べ方、予約デバイス名、`install.sh` の Kubo の固定値。
- `sh packaging/linux/test-install.sh` がすべて通ることを確かめた。足した確認: `manifest` の `/` を含む名前・`.` で始まる名前・ディレクトリを消さないこと、その他のユーザーが書き込める親と `lib/swing` の拒否、sticky ビット付きの親の許可、Kubo の不一致の表示、`/` を含む `--version` と相対パスの `HOME` の拒否。`shellcheck` は `install.sh`・`test-install.sh`・`render.sh` で警告なし。
- `render.sh` に `#{...}` を含むタグと `"` を含む URL の基点を渡して失敗すること、`&`・`|` を含む URL の基点は正しく置き換わることを確かめた。
- `actionlint` が 3 つのワークフローで指摘なし。
- `swing.iss` を Wine 上の Inno Setup 6 の ISCC で、中身の無いファイルを置いた `stage` でコンパイルし、`AppVersion` が `0.2.0` と `0.2.0-rc.1` のどちらでも警告もエラーも無く通ることを確かめた。`InvalidAppDir` の表示は実機で確かめていない（残タスクの「インストーラの実機での確認」に含まれる）。
- 固定した Kubo の SHA-512（Linux の 2 つと Windows の 1 つ）が dist.ipfs.tech の `.sha512` と一致することを確かめた。

## 追記: ワークフローでの確認

- `windows-installer-check` で、Windows のランナーから dist.ipfs.tech への接続が 2 回続けてタイムアウトした（やり直しを入れても同じ）。`build.ps1` の Kubo の取得元を、同じファイルを置いている Kubo の GitHub のリリースに変えた。中身はリポジトリに固定した SHA-512 で照らすので、取得元を変えても検証の強さは変わらない。`install.sh` は利用者の環境で動くので dist.ipfs.tech のまま。
- `homebrew-check` で、`brew upgrade` の後に古い keg が残った。ランナーで自動の後片付けが動かなかったので、確認の手順に `brew cleanup` を足した。
- `windows-check` の「Start from the tray」で、メニューの「Start」が見つからずに 1 回失敗した。直前の段階ではメニューに有効な「Start」が出ており、やり直しでは通ったので、メニューが開ききる前に探したものとみなして手を入れていない。
