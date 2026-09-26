#!/bin/sh
set -eu

if [ $# -ne 2 ]; then
  echo "usage: $0 <swing-tray binary> <output directory>" >&2
  exit 2
fi

here=$(cd "$(dirname "$0")" && pwd)
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$here/../Cargo.toml" | head -n 1)
app="$2/SWING.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
sed "s/@VERSION@/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
cp "$1" "$app/Contents/MacOS/swing-tray"
chmod 0755 "$app/Contents/MacOS/swing-tray"
cp "$here/../assets/SWING.icns" "$app/Contents/Resources/SWING.icns"

if command -v codesign > /dev/null 2>&1; then
  codesign --force --sign - "$app"
fi
echo "$app"
