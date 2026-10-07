# v0.1.0 のリリースの準備と Homebrew の確認

v0.1.0 のタグを main の先頭に打ち直し、ドラフトのリリースを作り直した。Homebrew の formula を初めて CI で確かめ、tap のリポジトリを用意した。

## 決めたこと

- v0.1.0 のタグは、まだリリースを公開していないので、新しく版を上げずに打ち直した。打ち直したタグの push で `release` ワークフローが通り、ドラフトのリリースとイメージ（`0.1.0`・`latest`）ができた。
- リリースの本文は `--generate-notes` に任せない。main に直接積んでいて PR が無いので、生成される本文はコミット一覧へのリンクだけになる。最初のリリースは、紹介・できること・入手方法・はじめかた・注意（互換性を保証しないこと、匿名性が無いこと、署名をしていないこと、Homebrew の tap と winget がまだ無いこと）を手で書いた。
- Homebrew は自前の tap で配る。homebrew-core はソースからビルドする formula と知名度を求めるので、今は対象にしない。
- tap のリポジトリは private で作っておき、本体のリポジトリを public にしてリリースを公開した後に `amane-katagiri/homebrew-swing` として public にする。formula のアーカイブはリリースの添付ファイルなので、本体が private のうちやドラフトのうちは tap を公開しても `brew install` が失敗する。

## 検証したこと

- `homebrew-check` を v0.1.0 のコミットで手動実行し、すべての段階（ローカルの tap からのインストール、strip 後の署名、`swing service install`、`brew upgrade` 後のパス、アンインストール後に LaunchAgent が残らないこと）が通った。

## 残したもの

- Intel の Mac と、tap のリポジトリからの実際のダウンロードは確かめていない。tap を public にしたら実機で `brew install amane-katagiri/swing/swing` を試す。
- 次の版からは、リリースのたびに `swing.rb` を tap に置き換える手作業が要る。
