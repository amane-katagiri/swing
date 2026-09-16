# 2026-09-16 解放待ちリストと sweep、起動時の突き合わせの見直し

## 目的

- `pin/rm` に失敗した CID が、state から消えたまま二度と再試行されない問題を直す。
- その外し損ねた pin を、同じ CID の新しいサイトが「SWING の外の pin」と取り違え、以後二度と外さなくなる問題を直す。
- `keep_days` や `keep_versions` が、新しい版が来たときにしか効かない問題を直す。
- 起動時の「Kubo にあって state に無い pin」の警告をやめる。publish や手動の pin があるのが普通の状態で、ノイズにしかならない。

## 決めたこと

| 決定 | 理由 |
|---|---|
| state に `releasing`（CID → `preexisting_pin`、`since`）を足し、解放はここに入れて保存してから `pin/rm` する | 外すと決めたことを先に保存しておけば、失敗しても途中で落ちても後から再試行できる |
| 版のレコードに有効・無効のフラグを持たせる方式ではなく、別のリストにする | フラグ方式だと、容量・サイト数の集計、重複 CID の判定、`swing sites` の表示など `sites` を読む箇所すべてで無効な版を除く必要があり、除き忘れがバグになる。別リストなら `sites` は有効な版だけのまま |
| 手動 pin の判定で `releasing` の `preexisting_pin` も引き継ぐ | 外し損ねた pin は Kubo 上は pin されているので、Kubo に聞くと手動 pin と区別できない |
| `releasing` の `preexisting_pin` は論理和で更新し、`since` は最初の値を保つ | 真を一度でも記録した CID を、後から偽で上書きして外してしまわないため |
| sweep は poll の各 tick で、Follow Set の再取得の前に行う | 起動時だけだと、長く動かしている agent では再試行が再起動まで遅れる |
| sweep で全サイトに `policy::retention_evictions` を適用する。`decide` の evict 部分を関数に切り出して共有する | 更新が止まったサイトや、設定で上限を下げたサイトにも保持期間と世代数を効かせる。判定を二重に書かないため |
| `pin/rm` の `not pinned or pinned indirectly` エラーは成功扱いにする | 既に外れている CID が `releasing` に永遠に残らないようにする。Kubo 0.43.1 で、pin の無い CID に `pin/rm` するとこのメッセージの 500 が返ることを確認した |
| 起動時の突き合わせは、Kubo に pin の無い版を `sites` から消す。`releasing` の pin の無い CID も消す。Kubo にだけある pin は何もしない | 記録を消せば次の poll で取り直され、ミラーが自動で直る。以前は警告を出すだけだった |
| 突き合わせを agent の中（`KuboPins` 経由）に移す | state を書き換えるようになったので、テスト用の fake で検証できるようにする |
| `keep_days * 86_400` を `saturating_mul` にする | 切り出しのついでに、極端な設定値での乗算オーバーフローを避ける |

## publish の pin について調べたこと

`swing publish` は同じ Kubo に `pin=true` で add するが、その pin は state に記録されない。整理の方法として Kubo の pin 名を試した（Kubo 0.43.1）。

- `add` の `pin-name`、`pin/add` の `name` で名前を付けられ、`pin/ls` の `names=true` で見える。
- `pin/ls` の `name` は部分一致で絞り込める。
- pin は CID ごとに 1 つで、既に pin のある CID に別の名前で `pin/add` すると、名前が後から付けたもので上書きされる。

名前は publish の旧版を探すのには使えるが、同じ CID を複数の用途で pin したときに持ち主を複数記録することはできない。publish の整理は todo に積んだ。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（ユニットテスト 143 件）。
- 追加したテスト: 失敗した解放が `releasing` に残って保存され sweep で外れること、外し損ねた CID を新しいサイトが手動 pin と取り違えないこと、手動 pin の `releasing` を外さずに消すこと、更新の無いサイトへの保持期間の適用、仕事が無い sweep は state を書かないこと、突き合わせ、`retention_evictions`、`keep_days` のオーバーフロー、`releasing` の論理和。
- `preexisting_pin` の判定から `releasing` を外すと、取り違えのテストが失敗することを確かめた。
- ローカルの Kubo 0.43.1（`IPFS_PROFILE=test`）で統合テスト 5 件を実行した。pin の無い CID への `pin_rm` が成功扱いになることを追加で確認した。
