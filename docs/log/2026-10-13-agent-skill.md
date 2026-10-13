# AI エージェント向けのスキルと swing skill

サイトを作った人が、Claude Code や Codex などの AI エージェントにサイトの確認を任せられるよう、Agent Skills 形式のスキル `swing-publish` を作り、`swing` に埋め込んで書き出すコマンド `swing skill show` / `install` を足した。前回の `swing publish --dry-run --json`（[`2026-10-13-publish-dry-run.md`](2026-10-13-publish-dry-run.md)）をエージェントから使う手順になる。

## 決めたこと

- スキルの正本はリポジトリの `skills/swing-publish/SKILL.md` に置き、`include_str!` で `swing` に埋め込む。リポジトリを clone しなくても、インストーラー・`install.sh`・Homebrew・Docker のどれで入れた `swing` からも手に入り、スキルの版と CLI の版が必ず揃う。エージェントの読むフィールドや終了コードは版で変わりうるので、別配布にして版がずれるのを避けた。
- 本文は英語。CLI の表示が英語で、エージェントが出力と照らすときに揃っているほうが読み違えにくいため。中身は Markdown の指示だけで、スクリプトは入れない。
- 最重要の縛りは「`--dry-run` 無しで `swing publish` を実行しない」。公開は取り消せず、利用者の鍵で署名されるため。確かめ終わったら、利用者が自分で打つコマンドを示して止まる。鍵や設定を作り替えない、確認のモードを緩めて通さない、直すのはサイトのソース側で利用者の了解を得てから、も書いた。
- 上限や既定値の数値はスキルに書き写さず、JSON の値（`checks.size.guideline` など）を見させる。数値の正本を増やさないため。JSON のフィールド名は `docs/architecture/cli/publish.md` のスキーマに合わせ、最初に `format` を確かめさせる。テストで `publish::dry_run::FORMAT` と本文の記述を照らす。
- 手引きへのリンクは GitHub の版付きの URL（`blob/v<版>/docs/...`）。正本では `{{version}}` にして、書き出すときに `CARGO_PKG_VERSION` で置き換える。`main` を指すと、古い `swing` が新しい仕様の手引きを読ませてしまう。同じ理由で `publish::links::SITE_GUIDE_URL`（CLI のリンクの確認・dry run の `guide_url`・ダッシュボードの公開画面の手引きのリンク）も `blob/main` から版のタグに変えた。web の静的ファイルには `blob/main/docs` を指す所が無く、ダッシュボードはこの URL を API から受け取るので、web は変えていない。
- 静的サイトジェネレーターの設定は、確信の持てる設定名だけを書き、残りは各ツールの公式ドキュメントで確かめるよう指示する形にした。版によって名前が変わるので、どれも確かめてから変えるよう書いた。
- Docker Compose では、JSON を読むために `docker compose run -T` を付けさせる。端末を割り当てると標準エラーが標準出力に混ざるため。
- `skill install` の既定の書き出し先は `~/.claude/skills/swing-publish/SKILL.md`。ほかのエージェントは `--dir <skills_dir>` で `<skills_dir>/swing-publish/SKILL.md` に書く。ホームは Windows では `USERPROFILE`（Claude Code の `~` と同じ）、ほかでは `HOME` で、既存の設定の場所の求め方に揃えて絶対パスでなければ使わない。
- 既にあるファイルは、中身が同じなら何もせず、違っていて frontmatter の `name` が `swing-publish` なら更新し、それ以外は `--force` 無しでは止める。利用者が同じ場所に置いた別のスキルを黙って消さないため。
- インストーラー・`install.sh`・Homebrew は利用者のホームのエージェントの設定に書き込まない。`install.sh` の最後の表示と Homebrew の caveats で `swing skill install` を案内するだけにした。Windows の Inno Setup のスクリプトは変えていない。
- `docs/architecture.md` が 20KB を超えそうになったので、テストの節を子ページ `docs/architecture/testing.md` に分けた。

## 作ったもの

- `skills/swing-publish/SKILL.md`: 縛り、swing の入り方の見分け、`--site`・`--url`・NIP-05 などの引数の決め方、dry run の実行と終了コード、JSON の読み方（`unknown` と `null` の違い、各項目、リンクの種別ごとの直し方）、dry run が見ない項目の確かめ方、静的サイトジェネレーターの設定の目安、公開のコマンドの渡し方（ダッシュボード・CI の `--yes` と終了コード・Docker）、版付きの手引きへのリンク。
- `src/skill.rs`: `render`・`install_into`・`install`。CLI の `swing skill show` / `install [--dir] [--force]`。
- Dockerfile の builder に `skills/` を COPY した（埋め込むため）。
- 案内: `install.sh` の最後の表示（`test-install.sh` でも確かめる）、Homebrew の caveats、`docs/guide/publish.md` の「AI エージェントで確かめる」、`docs/release/README.md`。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace`・`cargo xwin clippy --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings` が通った。
- 単体テスト: frontmatter が `name` と `description` だけで `name` が `swing-publish` であること、`{{` が残らないこと、リンクがすべてその版のタグを指しリポジトリにあるファイルであること、`format` の値が `publish::dry_run::FORMAT` と一致すること、install の 4 つの場合（新規・同じ・更新・別物で止まり `--force` で上書き）、frontmatter の見分け（CRLF・BOM・引用符・別の名前）、ホームの求め方。
- 実バイナリで `swing skill show` と、一時ディレクトリへの `swing skill install --dir` を、新規・同じ・古い版からの更新・別物（止まる・`--force`）の順に動かし、書き出したものが `show` の出力と同じことを確かめた。相対パスの `HOME` ではエラーになることも見た。
- `packaging/linux/test-install.sh` が通った。
- 鍵の無い設定と接続できないローカルの relay で `swing publish --dry-run --json` を動かし、`guide_url` が版付きになったこと、JSON のパスがサイトのディレクトリからの相対であること（スキルの記述どおり）を確かめた。
