# install.sh --service がほかの swing のサービスを登録し直さないようにする

architecture の整理（[`2026-10-14-architecture-cleanup.md`](2026-10-14-architecture-cleanup.md)）で見つけた、コードの食い違い 2 つを直した。

## install.sh --service

`--service` を付けて更新すると、ユーザー unit がほかの場所の `swing` を起動するものでも、最後の `swing service install` で今入れた `lib/swing` の `swing` を指すように登録し直していた。`--service` なしの更新は、持ち主の確認でほかの swing だと分かれば止めも起動し直しもしないので、`--service` の有無で扱いが逆になっていた。

- ユーザー unit があって `lib/swing` の `swing` を起動するものでなければ、`--service` でも `swing service install` を実行せずに警告し、`--service` なしと同じ次の手順（`swing service install` を含む）を表示する。切り替えたい利用者は自分でそれを実行する。
- 持ち主の確認は、unit が active のときは停止の前に済ませた結果を使い、active でなく `--service` のときだけ置き換えの前に確かめる。確かめられなければ（`status --points-into` の終了コードが 0・3・4 以外）、今までどおり何も変えずに止まる。
- `test-install.sh` に、active でないほかの swing の unit と、active なほかの swing の unit の 2 つの場合を足した。どちらも `service install` が呼ばれないこと、unit が残ること、警告が出ることを確かめる。修正前の `install.sh` ではこの検査が落ちることを確かめた。

## `swing up --log-file` の help

help は "instead of stderr" だったが、ログの既定の出力先は標準出力（`publish --json` のときだけ標準エラーで、`publish` は `--log-file` を取らない）なので "instead of stdout" に直した。

## 検証

- `sh packaging/linux/test-install.sh`、`shellcheck -s sh packaging/linux/install.sh packaging/linux/test-install.sh`
- `cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`
