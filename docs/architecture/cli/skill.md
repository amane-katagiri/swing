# skill（`src/skill.rs`）

[`../cli.md`](../cli.md) の子ページ。AI エージェント（Agent Skills 形式の `SKILL.md` を読むもの）向けのスキル `swing-publish` を書き出す `swing skill show` と `swing skill install`。スキルは、サイトを `swing publish --dry-run --json` と手引きのチェックリストで確かめ、公開のコマンドを利用者に渡して止まる手順で、エージェントには本番の publish をさせない。

設定ファイル・Kubo・relay・鍵を使わない（`--config` を取らない）。

## スキルの正本と版の埋め込み

- 正本はリポジトリの `skills/swing-publish/SKILL.md`（英語）。`include_str!` で `swing` に埋め込む。
- 正本の中の `{{version}}` を、書き出すときにすべて `CARGO_PKG_VERSION` に置き換える（`skill::render`）。手引きへのリンクは `https://github.com/amane-katagiri/swing/blob/v{{version}}/docs/...` と書き、その版のタグのドキュメントを指す。
- frontmatter は `name: swing-publish` と `description` だけ。
- テストは、frontmatter の形、`{{` が残らないこと、リンクがすべてその版のタグを指しリポジトリにあるファイルであること、dry run の JSON の `format` の値（`publish::dry_run::FORMAT`）が本文の記述と一致することを確かめる。

## skill show

版を埋めた `SKILL.md` を標準出力に出す。Docker Compose では `docker compose run --rm --no-deps -T mirror skill show > SKILL.md` で取り出す。

## skill install

`[--dir <skills_dir>] [--force]`。書き出し先は `<skills_dir>/swing-publish/SKILL.md` で、途中のディレクトリは作る。`--dir` が無ければ `<ホーム>/.claude/skills`。ホームは Windows では `USERPROFILE`、ほかの OS では `HOME` で、絶対パスでなければ `cannot determine the home directory (<変数> is not an absolute path); pass --dir` でエラー終了する。

書き出し先にファイルがあるときの扱い:

| 書き出し先 | 動作 | 表示（標準出力） |
|---|---|---|
| 無い | 書く | `installed the swing-publish skill for swing <版> at <パス>` |
| 中身がバイト単位で同じ | 何もしない | `<パス> is up to date (swing <版>)` |
| 違い、frontmatter の `name` が `swing-publish` | 上書きする | `updated <パス> to swing <版>` |
| それ以外（別物のファイル） | `--force` が無ければ何もせずに `<パス> exists and is not the swing-publish skill; rerun with --force to overwrite it` でエラー終了。`--force` なら上書きする | `replaced <パス> with the swing-publish skill for swing <版>` |

frontmatter は、1 行目が `---`（先頭の BOM は無視）で、次の `---` の行までに `name:` の値（前後の空白と引用符を除く）が `swing-publish` の行があるものを指す。改行は LF でも CRLF でもよい。

インストーラー・`install.sh`・Homebrew は利用者のホームのエージェントの設定に書き込まない。`install.sh` の最後の表示と Homebrew の caveats で、このコマンドを案内するだけ。
