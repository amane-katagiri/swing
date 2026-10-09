# macOS でサービスを入れ直すときに bootout の完了を待つ

リリース前の macos-check で、動いているサービスの上から `swing service install` すると `launchctl bootstrap` が `Bootstrap failed: 5: Input/output error` で失敗した。サービスまわりのコードは前のリリースから変わっておらず、前回は通っていたので、たまにしか起きない競合。

## 原因

`install` は古いジョブを `launchctl bootout` してすぐ `bootstrap` していた。`bootout` は launchd がジョブのプロセスに SIGTERM を送ったところで返ることがあり、`swing` が Kubo を止め終えるまでジョブは残る。その間に同じラベルを `bootstrap` すると I/O エラーになる。利用者の更新手順（`brew upgrade` の後の `swing service install`）がまさにこの経路を通る。

## 決めたこと

- `bootout` が成功したら、`launchctl print` でジョブが消えたことを確かめるまで待つ。待つ上限は plist の `ExitTimeOut`（`STOP_TIMEOUT`、90 秒）に 10 秒を足したもの。launchd はその時間で SIGKILL するので、それを過ぎても残っているなら待っても変わらない。
- `bootstrap` の失敗を数回やり直す方法は取らなかった。ジョブが消えたかは `print` で直接分かるので、時間を当て推量で決めるより確か。
- トレイ（`register_tray`・`unregister_tray`）と `uninstall` も同じ `bootout` を通るので、同じく待つ。

## 検証

- Linux で `cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace` が通る。macOS のコードは手元でクロスビルドできない（`ring` の C コンパイル）ので、macos-check で確かめる。
