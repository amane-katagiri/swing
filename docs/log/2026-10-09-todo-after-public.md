# public にした後の残タスクの整理

本体のリポジトリと tap（`amane-katagiri/homebrew-swing`）を public にし、v0.1.3 の formula を tap に置いたので、`docs/todo.md` のうち済んだものと前提が変わったものを直した。

## 済んだもの

- tap の公開と、リリースごとの `swing.rb` の置き換え。置き換えは `packaging/release/update-tap.sh` で行う（[`architecture/homebrew.md`](../architecture/homebrew.md#tap-への公開)）。「配布の残り」には winget・AUR・nixpkgs だけを残した。
- ghcr.io のイメージのタグ。`v0.1.0` から `v0.1.3` までのタグでそれぞれバージョンのタグが付き、`latest` は最新の `0.1.3` に付いている。`0.1.3` のマニフェストリストに `linux/amd64` と `linux/arm64` があることも確かめた。arm64 の環境での起動は残っている。

## 前提が変わったもの

- artifact attestation は private リポジトリだと GitHub Enterprise Cloud が要るので後回しにしていたが、public になったので条件を外した。
