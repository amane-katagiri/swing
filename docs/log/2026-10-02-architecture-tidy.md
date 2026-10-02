# architecture の整理（インストーラーと既定の設定の場所まわり）

インストーラー（`install.sh`・Windows・Homebrew）と既定の設定ファイルの場所を入れた一連の変更で、複数の手が並行して architecture を書いたため、重複・理由の混入・コードの言い換えが増えていた。その整理の回。

## 決めたこと

- **正本の置き場所**
  - 既定の設定ファイルとデータの場所: architecture は [`config.md#設定ファイルの場所`](../architecture/config.md#設定ファイルの場所)、README は新しい節「設定ファイルとデータの置き場所」。インストーラーの各ページと README のインストール手順はそこへリンクし、OS ごとの場所を繰り返さない。`docs/release/README.md` はアーカイブの中で読まれリポジトリへのリンクが使えないので、一覧をそのまま持つ。
  - Kubo の版を上げるときの手順: [`kubo.md#kubo-のバージョン`](../architecture/kubo.md#kubo-のバージョン)。`install-sh.md`・`installer-windows.md` は照合の仕方だけを書き、手順と更新漏れを検出するテストはそこへ寄せた。
  - 登録の持ち主の判定（`--only-from`・`--points-into` の規則・出力・終了コード）: [`service/ownership.md`](../architecture/service/ownership.md)。`service.md` の同名の節と `cli.md` の終了コードの説明は削り、リンクだけにした。
  - インストーラーと formula を作るジョブ: `release.md` はジョブの存在と artifact 名だけ、中身は `installer-windows.md`・`homebrew.md`。
- **分割はしない**: `installer-windows.md`（18.4K）と `service.md`（19.3K）は、言い換えと重複を削れば 20KB から十分離れるので、子ページに分けずに縮めた。`kubo.md`・`up.md` はこの一連の変更では数行しか触れておらず 18KB 前後で、今回は分けていない。次に大きく足すときに分ける。
- **理由の扱い**: architecture に混ざっていた理由（`--system` で空の設定ファイルを作らない理由、実行ファイルを `canonicalize` しない理由、`LOCALAPPDATA` を選んだ理由、新しい版の `swing` で持ち主を確かめる理由、Windows で `swing stop` に切り替える理由、`Path` の書き換えを `usUninstall` で行う理由、cask を選ばなかった理由など）は、どれも既にその回のログ（`2026-10-01-default-config-location.md`・`2026-10-01-homebrew-formula.md`・`2026-10-01-windows-installer.md`・`2026-10-02-installer-review.md`・`2026-10-02-uninstall-own-registration-only.md`）にあったので、移さずに削った。

## 直した古い記述

- Kubo の版を上げる手順に、`install.sh` の `KUBO_SHA512_AMD64`・`KUBO_SHA512_ARM64` が無かった。
- `service/ownership.md` の終了コード: `uninstall --only-from` は削除の失敗でも 1、`status --points-into` は判定できなければ 1。
- `SWING.app` を作るワークフローに `homebrew-check` が抜けていた。
- `docs/release/README.md` が、Kubo を同梱するのは Windows のインストーラーだけのように読めた（`install.sh` も `ipfs` を隣に置く）。
- README の「アンインストールしても設定とデータは消えない」に、`install.sh --purge` の例外を足した。

## 検証

`docs/`・`README.md`・`AGENTS.md` のすべての相対リンクと、GitHub と同じ規則で作った見出しのアンカーを解決するスクリプトで、壊れたリンクが無いことを確かめた（過去のログを含めても 0 件）。
