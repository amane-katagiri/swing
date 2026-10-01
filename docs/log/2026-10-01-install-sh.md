# Linux 向けインストールスクリプト

## 背景

[配布方式の設計](2026-09-21-distribution-design.md)で、Linux の導線を `curl -fsSL .../install.sh | sh` と決めていた。設定ファイルの既定の場所が決まった（[2026-10-01-default-config-location](2026-10-01-default-config-location.md)）ので、インストーラーが特定のディレクトリを意識せずに入れられる。

## 決めたこと

- 置き場所は `packaging/linux/install.sh`。リリースの `release` ジョブが `install.sh` として添え、`SHA256SUMS` にも含める。このために `release` ジョブへ checkout を足した。
- 「最新」のアーカイブ名にはタグが入る（`swing-<tag>-<target>.tar.gz`）ので、`latest/download/SHA256SUMS` から名前を引いてタグを決める。GitHub の API は使わない（レート制限と、非公開の間の認証を避けるため）。
- Kubo は `dist.ipfs.tech` の `.sha512` で照合する。スクリプトの `KUBO_VERSION` は `kubo::KUBO_VERSION` と手で揃える（[`kubo.md`](../architecture/kubo.md) の更新手順に足した）。すでに入っている `ipfs` が同じ版なら取得しない。
- 実行ファイルは `~/.local/lib/swing/` に置き、`~/.local/bin/swing` をシンボリックリンクにした。`locate_binary` は `current_exe()` の実体のパスの隣の `ipfs` を探すので、リンク経由でも見つかり、サービスの `ExecStart` も実体のパスになる。
- `bin/swing` に自分が作ったもの以外があれば `--force` が無いかぎり失敗する。依頼は「通常のファイル」を守ることだったが、別の場所を指すシンボリックリンク（別の手段で入れたもの）も同じ扱いにした。
- 更新は再実行で行い、動いているユーザーサービスは `swing service stop` で止めてから置き換えて、動いていたものだけ起動し直す。置き換えは同じディレクトリに書いて `mv` するので、動いている実行ファイルの書き込み（`ETXTBSY`）にならない。`swing up` を直接動かしているプロセスは止めない。
- アンインストールは `install.sh --uninstall` と `swing-uninstall.sh` が同じコード。`swing-uninstall.sh` の中身は `install.sh` そのもので、パイプ実行のときはリリースから取り直して `SHA256SUMS` で照合する（スクリプトの本文が手元に無いため）。ファイル名で `--uninstall` 扱いにし、`--prefix` は置かれている場所から決める。
- `--purge` が消すのは既定のディレクトリだけ。確認は `/dev/tty` から読み、端末が無ければ `--yes` が要る。
- テスト用に `SWING_INSTALL_BASE_URL`・`SWING_INSTALL_KUBO_BASE_URL`・`SWING_INSTALL_SYSTEM_UNIT` を足した。
- `--service` は root では拒否する（ユーザーサービスなので）。システム全体への導入（`sudo ... --prefix /usr/local`）の後は、各ユーザーが `swing service install` を実行する。

## 検証

- `packaging/linux/test-install.sh` が通ること（`sh` と `dash` の両方）。内容は [`install-sh.md`](../architecture/install-sh.md#テスト)。
- `shellcheck -s sh` が両方のスクリプトで警告なしで通ること。
- 実際の Kubo を `dist.ipfs.tech` から取得し、ローカルでビルドした `swing` をリリースの形に固めて `file://` で配り、一時 `HOME` で、パイプでのインストール・再実行による更新・`swing-uninstall.sh` を実行した。`ipfs version` が 0.43.1 になり、`.sha512` の照合を通ることを確かめた。
- 確かめていないこと: `wget` だけの環境、aarch64 実機、systemd の実機でのサービスの停止と再起動（テストは偽の `systemctl` と `swing`）、`sudo --prefix /usr/local`、GitHub 上のリリース（`latest/download` の URL）。
