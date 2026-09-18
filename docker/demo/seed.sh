#!/bin/sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
personas="$here/personas.env"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

compose() { "$here/demo.sh" "$@"; }

generate_key() {
  docker run --rm --network none --entrypoint swing swing-demo-mirror key generate
}

if [ ! -f "$personas" ]; then
  : > "$personas"
  chmod 600 "$personas"
  for name in alice bob carol dave eve frank grace heidi ivan judy mallory; do
    out=$(generate_key)
    printf '%s_secret=%s\n%s_public=%s\n' \
      "$name" "$(printf '%s\n' "$out" | sed -n 's/^hex (secret): //p')" \
      "$name" "$(printf '%s\n' "$out" | sed -n 's/^hex (public): //p')" >> "$personas"
  done
fi
. "$personas"
self_secret=$(sed -n 's/^SWING_NOSTR_SECRET_KEY=//p' "$here/demo.env")

secret_of() { if [ "$1" = self ]; then echo "$self_secret"; else eval "echo \$${1}_secret"; fi; }
public_of() { eval "echo \$${1}_public"; }

run_as() {
  who=$1
  shift
  if ! out=$(compose run --rm --no-deps -T \
    -e SWING_NOSTR_SECRET_KEY="$(secret_of "$who")" \
    -e SWING_STATE_DIR=/tmp \
    -v "$work:/seed:ro" \
    mirror "$@" 2>&1); then
    printf '%s\n' "$out" >&2
    exit 1
  fi
}

make_site() {
  dir="$work/$1"
  mkdir -p "$dir"
  cat > "$dir/index.html" <<HTML
<!doctype html>
<html lang="ja">
<head><meta charset="utf-8"><title>$2</title><link rel="stylesheet" href="style.css"></head>
<body>
<h1>$2</h1>
<p>$3</p>
<p><a href="about.html">about</a></p>
</body>
</html>
HTML
  cat > "$dir/about.html" <<HTML
<!doctype html>
<html lang="ja"><head><meta charset="utf-8"><title>about</title><link rel="stylesheet" href="style.css"></head>
<body><h1>about</h1><p>SWING デモ用のサンプルサイト（$1）。</p><p><a href="index.html">top</a></p></body></html>
HTML
  printf 'body { font-family: sans-serif; max-width: 40em; margin: 2em auto; color: %s; }\n' "$4" > "$dir/style.css"
}

publish() {
  who=$1 site=$2 message=$3
  shift 3
  run_as "$who" publish --nip05 off --site "$site" -m "$message" "$@" "/seed/$site"
  echo "  $who: $site"
}

follow() {
  who=$1
  shift
  keys=""
  for name in "$@"; do
    if [ "$name" = self ]; then
      keys="$keys $(sed -n 's/^SWING_DEMO_SELF_PUBLIC=//p' "$here/demo.env")"
    else
      keys="$keys $(public_of "$name")"
    fi
  done
  # shellcheck disable=SC2086
  run_as "$who" mirror add $keys
  echo "  $who -> $*"
}

echo "publishing sample sites"
make_site my-garden "わたしの庭" "デモ環境の自分のサイト。" "#234"
publish self my-garden "サイトを開設"
make_site alice.example "Alice's Notes" "日々のメモ。" "#633"
publish alice alice.example "初版" --url https://alice.example/
printf '<p>追記: 2 本目の記事。</p>\n' >> "$work/alice.example/index.html"
sleep 1
publish alice alice.example "記事を 1 本追加" --url https://alice.example/
make_site bob-zine "bob zine" "手作りのジン。" "#363"
publish bob bob-zine "第 1 号"
make_site carol.example "Carol" "ポートフォリオ。" "#336"
publish carol carol.example "トップを更新" --url https://carol.example/
make_site carol-photos "Carol's photos" "写真置き場。" "#555"
publish carol carol-photos ""
make_site dave-wiki "dave wiki" "個人 wiki。" "#446"
publish dave dave-wiki "ページを整理"
make_site eve.example "eve" "日記。" "#644"
publish eve eve.example "引っ越しました" --url https://eve.example/
make_site frank-recipes "Frank's recipes" "レシピ集。" "#553"
publish frank frank-recipes "カレーを追加"
make_site grace.example "Grace" "研究ノート。" "#265"
publish grace grace.example "" --url https://grace.example/
make_site ivan-lab "ivan lab" "実験場。" "#522"
publish ivan ivan-lab "試作"
make_site judy.example "Judy" "旅行記。" "#256"
publish judy judy.example "北海道編" --url https://judy.example/
make_site mallory-archive "mallory archive" "古いサイトの保管庫。" "#444"
publish mallory mallory-archive "アーカイブを公開"

echo "publishing follow sets"
follow self alice bob carol
follow alice self dave
follow bob carol eve
follow carol self frank
follow dave grace
follow eve bob heidi
follow frank ivan
follow grace judy
follow heidi alice
follow ivan frank
follow judy grace mallory

compose restart mirror > /dev/null 2>&1
