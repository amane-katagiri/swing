# Windows 実機での確認（タスクトレイと service）

`docs/todo.md` にあった Windows 実機での確認 2 件（[タスクトレイ](2026-09-24-tray-icon.md)と、[Windows のトレイ終了条件と service stop](2026-09-26-windows-tray-and-service-stop.md)）を、`9da8e85` のリリースビルドで確かめた。コードは変えていない。

## 手順

`cargo xwin build --release --workspace --target x86_64-pc-windows-msvc` で作った `swing.exe`・`swing-tray.exe` を、Kubo v0.43.1 の `ipfs.exe`（公式の配布物。sha512 を照合）と同じフォルダに置き、日本語の表示言語の Windows で確かめた。

## 確かめたこと

- 準備: `ipfs.exe` を同じフォルダに置くだけで `swing up` が Kubo を見つける。初回起動で Windows Defender ファイアウォールの許可ダイアログ（パブリック / プライベート ネットワーク）が出る。
- ファイルアイコン: `swing.exe` と `swing-tray.exe` に別々のアイコンが付き、プロパティの詳細に名前とバージョンが出る。
- `service install`: コンソールウィンドウを開かずに成功し、直後にトレイが出る。タスク スケジューラに `swing` が登録され、タスクマネージャーのスタートアップ アプリに `swing-tray` が出る。
- トレイのメニュー: 状態の表示、ダッシュボードを開く、再起動・停止・起動（途中の表示と項目の無効化、停止中の灰色で半透明のアイコン）、停止の確認ダイアログ、終了のダイアログ（キャンセル / いいえ / はい）、停止中の終了（確認なし）、多重起動の防止がすべて期待どおり。`schtasks` を呼ぶ操作でコンソールウィンドウは一度も出なかった。
- 登録判定（日本語環境）: `service status` の出力が文字化けしない。登録済みで止めると「停止中」で「起動」が押せる。`uninstall` 後は `not installed` になり、トレイは約 15 秒以内に閉じる。未登録で手で起動したトレイは「停止中（サービス未登録）」で「起動」が無効になり、勝手には閉じない。
- 設定ファイルが無いときの `service stop` / `uninstall`: `swing.toml` の無いディレクトリ、存在しないパスを指す `SWING_CONFIG`、壊れた `swing.toml` のそれぞれで、対応する `Warning: …` を出して `schtasks /End` にフォールバックし、実際に止まる。`uninstall` も同じ警告を出してタスクと Run キーを消す。`swing.toml` のあるディレクトリでは警告なしでグレースフルに止まる。

## 残したこと

次は確かめていないので `docs/todo.md` に残した。

- サインインし直したときに、トレイが出て「動作中」になること
- 英語の表示言語で、トレイの文言が英語になることと、`schtasks` による登録判定が正しく働くこと
