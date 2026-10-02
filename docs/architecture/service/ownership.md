# 登録の持ち主の判定（service/ownership.rs）

[`../service.md`](../service.md) の一部。インストーラーからの使い方は [`../installer-windows.md`](../installer-windows.md) と [`../install-sh.md`](../install-sh.md)。

`uninstall --only-from <dir>` と `status --points-into <dir>`（`--help` に出さない隠しオプション。[`cli.md`](../cli.md#service-install--uninstall--start--stop--status)）は、今の登録を読み、それぞれが起動する実行ファイルが `<dir>` の下にあるかを判定する（`src/service/ownership.rs`）。インストーラーが、自分が入れたフォルダー以外から登録された `swing` を消したり登録し直したりしないために使う。

| OS | 登録（`Part`） | 実行ファイルを読む場所 |
|---|---|---|
| Linux | unit（本体） | unit ファイルの最初の `ExecStart=` の先頭の語。引用符の中の `\\`・`\"` を戻し、`$$`・`%%` を `$`・`%` に戻す（`systemd_unit` が書くエスケープの逆）。先頭の `-`・`@`・`:`・`+`・`!` は飛ばす。`--system` なら system unit を読む |
| macOS | `jp.ne.ama.swing.plist`（本体）・`jp.ne.ama.swing-tray.plist`（トレイ） | `Program` があればその値、無ければ `ProgramArguments` の最初の要素（XML の実体参照を戻す）。バイナリ形式の plist は読み取れないものとして扱う |
| Windows | タスク `swing`（本体）・Run キーの値 `swing-tray`（トレイ） | タスクは `schtasks /Query /TN swing /XML`（10 秒のタイムアウト。UTF-16 の BOM があれば UTF-16 として読む）の最初の `Exec`。`Command` が `conhost.exe` なら `Arguments` の `-` で始まらない最初の引数、そうでなければ `Command` そのもの。Run キーの値は先頭の引数（`RegGetValueW`） |

- 比べ方: どちらも絶対パスのときだけ比べ、`.` と `..` を畳んだうえで、パスの要素の並びとして `<dir>` が真の前置きになっていれば「下にある」（`/opt/swing-old/swing` は `/opt/swing` の下ではない）。Windows では `\\?\`（`\\?\UNC\` は `\\`）を外し、`/` を `\` とみなし、大文字小文字を区別しない。`<dir>` は `std::path::absolute` で絶対パスにする。登録のパスと `<dir>` は、そのままのものに加え、実在すれば `canonicalize` したものどうしも比べ、どれかの組で下にあれば下にあるとする（シンボリックリンク経由の登録や `<dir>` に対応する）。
- 実行ファイルを読み取れない登録（書式が違う、手で書き換えた）は「`<dir>` の下でない」として扱い、消さない。
- `uninstall --only-from`: 何も登録されていなければ `swing is not registered; nothing to uninstall.` と出して終わる。`<dir>` の下でない登録ごとに `<登録> runs <exe>, which is not under <dir>; left it as is.`（読み取れなければ `<登録> could not be read to tell which swing it runs, so it is treated as not under <dir>; left it as is.`）を出す。残りを、`--only-from` なしの `uninstall` と同じ手順で消す。本体とトレイは別々に扱い、本体だけが `<dir>` の下ならトレイを残して本体だけを（停止を含めて）消し、トレイだけが `<dir>` の下なら本体には触れず（止めもせず）トレイの登録だけを消す。終了コードは 0（登録の読み取りに失敗したときだけ 1）。
- `status --points-into`: 登録ごとに `<登録> runs <exe>, which is under <dir>.` か、上と同じ「下でない」文を出す（何も無ければ `not installed`）。終了コードは、登録が 1 つも無ければ 3、すべてが下にあれば 0、1 つでも下でない（読み取れないものを含む）なら 4。本体とトレイの両方を見るので、片方だけが別の場所を指す混ざった状態も 4 になり、インストーラーの上書きは何も登録し直さない。
