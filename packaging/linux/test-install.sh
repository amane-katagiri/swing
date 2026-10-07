#!/bin/sh
set -eu

here=$(cd "$(dirname "$0")" && pwd -P)
SOURCE=$here/install.sh

case $(uname -m) in
  x86_64 | amd64)
    TARGET=x86_64-unknown-linux-musl
    KUBO_ARCH=amd64
    KUBO_PIN=KUBO_SHA512_AMD64
    ;;
  aarch64 | arm64)
    TARGET=aarch64-unknown-linux-musl
    KUBO_ARCH=arm64
    KUBO_PIN=KUBO_SHA512_ARM64
    ;;
  *)
    echo "unsupported architecture for this test" >&2
    exit 1
    ;;
esac
KUBO_VERSION=$(sed -n 's/^KUBO_VERSION=//p' "$SOURCE")

ROOT=$(mktemp -d)
trap 'rm -rf "$ROOT"' EXIT

FAILED=0
pass() { printf 'ok   %s\n' "$1"; }
fail() {
  printf 'FAIL %s\n' "$1"
  FAILED=$((FAILED + 1))
}
check() {
  desc=$1
  shift
  if "$@"; then pass "$desc"; else fail "$desc"; fi
}
absent() { [ ! -e "$1" ] && [ ! -L "$1" ]; }
unit_for() {
  mkdir -p "$(dirname "$1")"
  printf '[Service]\nExecStart="%s" up --config "%s/swing.toml"\n' "$2" "$H" >"$1"
}
contains() { grep -qF -- "$2" "$1"; }
no_dotfiles() {
  for f in "$1"/.*; do
    case $f in
      */. | */..) ;;
      *) return 1 ;;
    esac
  done
}
lacks() { ! grep -qF -- "$2" "$1"; }

sha256() { sha256sum "$1" | cut -d ' ' -f 1; }

# shellcheck disable=SC2016
make_script() {
  out=$1
  kubo=$2
  mkdir -p "$(dirname "$out")"
  sed \
    -e "s|^RELEASES_URL=.*|RELEASES_URL=file://$ROOT/releases|" \
    -e "s|^KUBO_BASE_URL=.*|KUBO_BASE_URL=file://$kubo|" \
    -e "s|--proto '=https'|--proto '=https,file'|" \
    -e 's|^SYSTEM_UNIT=.*|SYSTEM_UNIT=$HOME/system-swing.service|' \
    -e "s|^$KUBO_PIN=.*|$KUBO_PIN=$(sha512sum "$ROOT/kubo/kubo_v${KUBO_VERSION}_linux-$KUBO_ARCH.tar.gz" | cut -d ' ' -f 1)|" \
    "$SOURCE" >"$out"
  for line in "RELEASES_URL=file://" "KUBO_BASE_URL=file://" 'SYSTEM_UNIT=$HOME/' "$KUBO_PIN="; do
    grep -q "^$line" "$out" || {
      echo "could not rewrite $line in the copy of install.sh" >&2
      exit 1
    }
  done
  grep -qF -- "--proto '=https,file'" "$out" || {
    echo "could not let curl read file:// URLs in the copy of install.sh" >&2
    exit 1
  }
}

make_swing_release() {
  tag=$1
  dir=$ROOT/releases/download/$tag
  stage=$ROOT/stage-$tag/swing-$tag-$TARGET
  mkdir -p "$dir" "$stage"
  cat >"$stage/swing" <<EOF
#!/bin/sh
echo "swing \$*" >>"\$FAKE_LOG"
echo "$tag \$*" >>"\$FAKE_STATE/ran"
unit=\$HOME/.config/systemd/user/swing.service
case " \$* " in
  *" --system "*) unit=\$HOME/system-swing.service ;;
esac
dir=
for a; do dir=\$a; done
case "\$1" in
  --version) echo "swing ${tag#v}" ;;
  service)
    case "\$2" in
      stop) rm -f "\$FAKE_STATE/active" ;;
      start | install) touch "\$FAKE_STATE/active" ;;
      status)
        [ -f "\$unit" ] || exit 3
        grep -qF "ExecStart=\"\$dir/" "\$unit" || { echo "runs another swing"; exit 4; }
        ;;
      uninstall)
        case " \$* " in
          *" --only-from "*)
            if ! grep -qF "ExecStart=\"\$dir/" "\$unit"; then
              echo "left it as is"
              exit 0
            fi
            ;;
        esac
        rm -f "\$FAKE_STATE/active" "\$unit"
        ;;
    esac
    ;;
esac
EOF
  chmod +x "$stage/swing"
  echo "license" >"$stage/LICENSE"
  echo "fonts" >"$stage/LICENSE-PixelMplus.txt"
  echo "example" >"$stage/swing.example.toml"
  echo "readme $tag" >"$stage/README.md"
  tar -C "$ROOT/stage-$tag" -czf "$dir/swing-$tag-$TARGET.tar.gz" "swing-$tag-$TARGET"
  cp "$INSTALL" "$dir/install.sh"
  (cd "$dir" && sha256sum -- * >SHA256SUMS)
}

make_kubo_dist() {
  dir=$ROOT/kubo
  stage=$ROOT/kubo-stage/kubo
  mkdir -p "$dir" "$stage"
  cat >"$stage/ipfs" <<EOF
#!/bin/sh
case "\$*" in
  "version --number") echo "$KUBO_VERSION" ;;
esac
EOF
  chmod +x "$stage/ipfs"
  echo apache >"$stage/LICENSE-APACHE"
  echo mit >"$stage/LICENSE-MIT"
  name=kubo_v${KUBO_VERSION}_linux-$KUBO_ARCH.tar.gz
  tar -C "$ROOT/kubo-stage" -czf "$dir/$name" kubo
}

make_kubo_dist
INSTALL=$ROOT/script/install.sh
make_script "$INSTALL" "$ROOT/kubo"
make_swing_release v0.1.0
make_swing_release v0.2.0
mkdir -p "$ROOT/releases/latest"

FAKEBIN=$ROOT/fakebin
mkdir -p "$FAKEBIN"
cat >"$FAKEBIN/systemctl" <<'EOF'
#!/bin/sh
echo "systemctl $*" >>"$FAKE_LOG"
case "$*" in
  "--user is-active --quiet swing") [ -f "$FAKE_STATE/active" ] ;;
  *) exit 0 ;;
esac
EOF
chmod +x "$FAKEBIN/systemctl"

new_env() {
  H=$ROOT/home-$1
  rm -rf "$H"
  mkdir -p "$H"
  FAKE_LOG=$H/calls.log
  FAKE_STATE=$H/state
  mkdir -p "$FAKE_STATE"
  : >"$FAKE_LOG"
}

run() {
  ln -sfn "$ROOT/releases/download/${REL:-v0.1.0}" "$ROOT/releases/latest/download"
  env -u XDG_DATA_HOME -u XDG_CONFIG_HOME \
    HOME="$H" PATH="$FAKEBIN:$PATH" \
    FAKE_LOG="$FAKE_LOG" FAKE_STATE="$FAKE_STATE" \
    "$@"
}

echo "--- fresh install (piped, latest resolved from the checksums)"
new_env fresh
out=$(cat "$INSTALL" | run sh -s -- 2>&1) || {
  echo "$out"
  fail "fresh install exits 0"
}
lib=$H/.local/lib/swing
check "swing installed" [ -x "$lib/swing" ]
check "ipfs installed" [ -x "$lib/ipfs" ]
check "uninstaller installed" [ -x "$lib/swing-uninstall.sh" ]
check "kubo license installed" [ -f "$lib/LICENSE-kubo-MIT" ]
check "example config installed" [ -f "$lib/swing.example.toml" ]
check "symlink points at the real binary" [ "$(readlink "$H/.local/bin/swing")" = "$lib/swing" ]
check "manifest lists swing" contains "$lib/manifest" swing
check "manifest does not list itself" lacks "$lib/manifest" manifest
check "no leftovers in lib" no_dotfiles "$lib"
check "swing runs through the symlink" [ "$(run "$H/.local/bin/swing" --version)" = "swing 0.1.0" ]
check "warns about PATH" sh -c "printf '%s' \"\$1\" | grep -q 'not on your PATH'" _ "$out"
check "mentions the data directory" sh -c "printf '%s' \"\$1\" | grep -qF '$H/.local/share/swing'" _ "$out"

echo "--- upgrade restarts a running service"
touch "$FAKE_STATE/active"
unit_for "$H/.config/systemd/user/swing.service" "$lib/swing"
: >"$FAKE_LOG"
out=$(REL=v0.2.0 run sh "$INSTALL" 2>&1) || fail "upgrade exits 0"
check "binary replaced" [ "$(run "$lib/swing" --version)" = "swing 0.2.0" ]
check "service stopped then started" sh -c "grep -n 'service' '$FAKE_LOG' | grep -A1 'service stop' | grep -q 'service start'"
check "service is active again" [ -f "$FAKE_STATE/active" ]
check "ownership checked with the new binary" contains "$FAKE_STATE/ran" "v0.2.0 service status --points-into $lib"
check "kubo download skipped when the version matches" sh -c "! printf '%s' \"\$1\" | grep -q 'downloading Kubo'" _ "$out"

echo "--- upgrade leaves a stopped service stopped"
rm -f "$FAKE_STATE/active"
: >"$FAKE_LOG"
REL=v0.1.0 run sh "$INSTALL" >/dev/null 2>&1 || fail "second upgrade exits 0"
check "no service start" lacks "$FAKE_LOG" "service start"

echo "--- upgrade leaves a running service of another swing alone"
touch "$FAKE_STATE/active"
unit_for "$H/.config/systemd/user/swing.service" "$H/elsewhere/swing"
: >"$FAKE_LOG"
out=$(REL=v0.2.0 run sh "$INSTALL" 2>&1) || fail "upgrade with a foreign service exits 0"
check "ownership checked against lib" contains "$FAKE_LOG" "service status --points-into $lib"
check "foreign service not stopped" lacks "$FAKE_LOG" "service stop"
check "foreign service not started" lacks "$FAKE_LOG" "service start"
check "foreign service still active" [ -f "$FAKE_STATE/active" ]
check "foreign service reported" sh -c "printf '%s' \"\$1\" | grep -q 'leaving the swing service as is'" _ "$out"
check "binary still replaced" [ "$(run "$lib/swing" --version)" = "swing 0.2.0" ]
check "no check binary left behind" no_dotfiles "$lib"
rm -f "$FAKE_STATE/active" "$H/.config/systemd/user/swing.service"

echo "--- --version"
new_env pinned
REL=v0.2.0 run sh "$INSTALL" --version 0.2.0 >/dev/null 2>&1 || fail "--version install exits 0"
check "requested version installed" [ "$(run "$H/.local/bin/swing" --version)" = "swing 0.2.0" ]

echo "--- --service"
if [ "$(id -u)" != 0 ]; then
  new_env svc
  out=$(run sh "$INSTALL" --service 2>&1) || fail "--service exits 0"
  check "service install called" contains "$FAKE_LOG" "swing service install"
  check "dashboard hint printed" sh -c "printf '%s' \"\$1\" | grep -q 'swing dashboard open'" _ "$out"
else
  echo "skip --service (install.sh refuses it as root)"
fi

echo "--- checksum mismatch"
new_env bad
bad=$ROOT/releases/download/bad
rm -rf "$bad"
cp -r "$ROOT/releases/download/v0.1.0" "$bad"
echo tamper >>"$bad/swing-v0.1.0-$TARGET.tar.gz"
if REL=bad run sh "$INSTALL" >"$H/out" 2>&1; then
  fail "tampered archive is rejected"
else
  pass "tampered archive is rejected"
fi
check "mismatch reported" contains "$H/out" "checksum mismatch"
check "nothing installed after the mismatch" absent "$H/.local/lib/swing"

new_env badkubo
badk=$ROOT/kubo-bad
rm -rf "$badk"
cp -r "$ROOT/kubo" "$badk"
echo tamper >>"$badk/kubo_v${KUBO_VERSION}_linux-$KUBO_ARCH.tar.gz"
make_script "$ROOT/script-badkubo/install.sh" "$badk"
if run sh "$ROOT/script-badkubo/install.sh" >"$H/out" 2>&1; then
  fail "tampered kubo is rejected"
else
  pass "tampered kubo is rejected"
fi
check "kubo mismatch reported" contains "$H/out" "checksum mismatch for kubo_v"
check "nothing installed after the kubo mismatch" absent "$H/.local/lib/swing"

echo "--- existing swing in bin"
new_env clobber
mkdir -p "$H/.local/bin"
echo mine >"$H/.local/bin/swing"
if run sh "$INSTALL" >"$H/out" 2>&1; then
  fail "refuses to replace a foreign swing"
else
  pass "refuses to replace a foreign swing"
fi
check "foreign swing untouched" [ "$(cat "$H/.local/bin/swing")" = mine ]
run sh "$INSTALL" --force >/dev/null 2>&1 || fail "--force install exits 0"
check "--force replaces it" [ -L "$H/.local/bin/swing" ]

echo "--- unsupported platform"
mkdir -p "$ROOT/fakeuname"
cat >"$ROOT/fakeuname/uname" <<'EOF'
#!/bin/sh
case "$1" in -s) echo Darwin ;; *) echo arm64 ;; esac
EOF
chmod +x "$ROOT/fakeuname/uname"
new_env mac
if PATH="$ROOT/fakeuname:$PATH" run sh "$INSTALL" >"$H/out" 2>&1; then
  fail "macOS is rejected"
else
  pass "macOS is rejected"
fi
check "macOS points at Homebrew" contains "$H/out" "Homebrew"

echo "--- uninstall keeps data"
new_env un
run sh "$INSTALL" >/dev/null 2>&1
mkdir -p "$H/.local/share/swing/data"
echo secret >"$H/.local/share/swing/swing.toml"
unit_for "$H/.config/systemd/user/swing.service" "$H/.local/lib/swing/swing"
touch "$H/.local/bin/other"
out=$(run "$H/.local/lib/swing/swing-uninstall.sh" 2>&1) || fail "uninstall exits 0"
check "service uninstalled first" contains "$FAKE_LOG" "swing service uninstall --only-from $H/.local/lib/swing"
check "own unit removed" absent "$H/.config/systemd/user/swing.service"
check "lib dir removed" absent "$H/.local/lib/swing"
check "symlink removed" absent "$H/.local/bin/swing"
check "unrelated file kept" [ -f "$H/.local/bin/other" ]
check "config kept" [ -f "$H/.local/share/swing/swing.toml" ]
check "data location printed" sh -c "printf '%s' \"\$1\" | grep -qF '$H/.local/share/swing'" _ "$out"

echo "--- uninstall keeps a service of another swing"
new_env unforeign
run sh "$INSTALL" >/dev/null 2>&1
unit_for "$H/.config/systemd/user/swing.service" "$H/elsewhere/swing"
out=$(run sh "$INSTALL" --uninstall 2>&1) || fail "uninstall with a foreign service exits 0"
check "uninstall asked only for our registration" contains "$FAKE_LOG" "swing service uninstall --only-from $H/.local/lib/swing"
check "foreign unit kept" [ -f "$H/.config/systemd/user/swing.service" ]
check "files still removed" absent "$H/.local/lib/swing"
check "kept registration reported" sh -c "printf '%s' \"\$1\" | grep -q 'left it as is'" _ "$out"

echo "--- uninstall through install.sh --uninstall --purge --yes"
new_env purge
run sh "$INSTALL" >/dev/null 2>&1
mkdir -p "$H/.local/share/swing/data" "$H/.local/share/other"
echo secret >"$H/.local/share/swing/swing.toml"
run sh "$INSTALL" --uninstall --purge --yes >/dev/null 2>&1 || fail "purge exits 0"
check "lib dir removed" absent "$H/.local/lib/swing"
check "data removed" absent "$H/.local/share/swing"
check "neighbouring directory kept" [ -d "$H/.local/share/other" ]

echo "--- --purge without --yes and without a terminal"
new_env purge2
run sh "$INSTALL" >/dev/null 2>&1
mkdir -p "$H/.local/share/swing"
if run setsid sh "$INSTALL" --uninstall --purge </dev/null >"$H/out" 2>&1; then
  fail "purge without confirmation is refused"
else
  pass "purge without confirmation is refused"
fi
check "data kept" [ -d "$H/.local/share/swing" ]
check "install kept" [ -x "$H/.local/lib/swing/swing" ]

echo "--- uninstall refuses with a system service"
new_env sys
run sh "$INSTALL" >/dev/null 2>&1
unit_for "$H/system-swing.service" "$H/.local/lib/swing/swing"
if run sh "$INSTALL" --uninstall >"$H/out" 2>&1; then
  fail "uninstall is refused while a system service exists"
else
  pass "uninstall is refused while a system service exists"
fi
check "refusal names the system uninstall" contains "$H/out" "service uninstall --system"
check "install kept" [ -x "$H/.local/lib/swing/swing" ]
run sh "$INSTALL" --uninstall --force >/dev/null 2>&1 || fail "--force uninstall exits 0"
check "--force uninstalls" absent "$H/.local/lib/swing"

echo "--- a system service of another swing does not block the uninstall"
new_env sysforeign
run sh "$INSTALL" >/dev/null 2>&1
unit_for "$H/system-swing.service" /usr/bin/swing
run sh "$INSTALL" --uninstall >"$H/out" 2>&1 || fail "uninstall with a foreign system service exits 0"
check "uninstalled" absent "$H/.local/lib/swing"
check "foreign system unit kept" [ -f "$H/system-swing.service" ]

echo "--- XDG_DATA_HOME is honoured"
new_env xdg
run sh "$INSTALL" >/dev/null 2>&1
mkdir -p "$H/xdg/swing"
env XDG_DATA_HOME="$H/xdg" HOME="$H" PATH="$FAKEBIN:$PATH" FAKE_LOG="$FAKE_LOG" FAKE_STATE="$FAKE_STATE" \
  sh "$INSTALL" --uninstall --purge --yes >/dev/null 2>&1 || fail "xdg purge exits 0"
check "XDG data directory removed" absent "$H/xdg/swing"

echo "--- --prefix"
new_env prefix
PFX=$H/opt
run sh "$INSTALL" --prefix "$PFX" >/dev/null 2>&1 || fail "--prefix install exits 0"
check "binary under prefix lib" [ -x "$PFX/lib/swing/swing" ]
check "symlink under prefix bin" [ "$(readlink "$PFX/bin/swing")" = "$PFX/lib/swing/swing" ]
check "nothing under the default location" absent "$H/.local"
run "$PFX/lib/swing/swing-uninstall.sh" >/dev/null 2>&1 || fail "prefix uninstall exits 0"
check "prefix lib removed" absent "$PFX/lib/swing"
check "prefix symlink removed" absent "$PFX/bin/swing"
PFX=

echo "--- uninstall removes only plain file names from the manifest"
new_env manifest
run sh "$INSTALL" >/dev/null 2>&1
lib=$H/.local/lib/swing
echo victim >"$H/.local/lib/victim"
mkdir "$lib/keepdir"
printf '../victim\n.hidden\nkeepdir\n' >>"$lib/manifest"
echo hidden >"$lib/.hidden"
out=$(run sh "$INSTALL" --uninstall 2>&1) || {
  echo "$out"
  fail "uninstall with a tampered manifest exits 0"
}
check "file outside lib kept" [ -f "$H/.local/lib/victim" ]
check "dot file kept" [ -f "$lib/.hidden" ]
check "directory kept" [ -d "$lib/keepdir" ]
check "installed files removed" absent "$lib/swing"

echo "--- uninstall removes a symlink named in the manifest, not its target"
new_env manifestlink
run sh "$INSTALL" >/dev/null 2>&1
lib=$H/.local/lib/swing
echo outside >"$H/outside"
rm -f "$lib/README.md"
ln -s "$H/outside" "$lib/README.md"
grep -qxF README.md "$lib/manifest" || echo README.md >>"$lib/manifest"
run sh "$INSTALL" --uninstall >/dev/null 2>&1 || fail "uninstall with a symlinked manifest entry exits 0"
check "symlink target outside lib kept" [ -f "$H/outside" ]
check "lib removed with the symlink" absent "$lib"

echo "--- directories writable by their group or other users are refused"
new_env shared
mkdir "$H/shared"
chmod 777 "$H/shared"
if run sh "$INSTALL" --prefix "$H/shared/p" >"$H/out" 2>&1; then
  fail "a world-writable parent is refused"
else
  pass "a world-writable parent is refused"
fi
check "refusal names the directory" contains "$H/out" "$H/shared is writable by its group or other users"
check "nothing created under it" absent "$H/shared/p"
me=$(id -un)
shared_gid=
private_gid=
for gid in $(id -G); do
  entry=$(getent group "$gid") || continue
  if [ "${entry%%:*}" = "$me" ] && { [ -z "${entry##*:}" ] || [ "${entry##*:}" = "$me" ]; }; then
    private_gid=$gid
  elif [ -z "$shared_gid" ]; then
    shared_gid=$gid
  fi
done
if [ -n "$shared_gid" ]; then
  mkdir "$H/group"
  chgrp "$shared_gid" "$H/group"
  chmod 775 "$H/group"
  if run sh "$INSTALL" --prefix "$H/group/p" >"$H/out" 2>&1; then
    fail "a parent writable by a shared group is refused"
  else
    pass "a parent writable by a shared group is refused"
  fi
  check "group refusal names the directory" contains "$H/out" "$H/group is writable by its group or other users"
else
  echo "skip a parent writable by a shared group (the user is in no shared group)"
fi
if [ -n "$private_gid" ]; then
  mkdir "$H/own"
  chgrp "$private_gid" "$H/own"
  chmod 775 "$H/own"
  run sh "$INSTALL" --prefix "$H/own/p" >"$H/out" 2>&1 || fail "a parent writable by the user's own group is accepted"
  check "installed under a parent writable by the user's own group" [ -x "$H/own/p/lib/swing/swing" ]
else
  echo "skip a parent writable by the user's own group (the user has no private group)"
fi
delegated_gid=
if [ "$(id -u)" = 0 ]; then
  delegated_gid=$(getent group staff users nogroup | head -n 1 | cut -d : -f 3)
fi
if [ -n "$delegated_gid" ]; then
  mkdir "$H/delegated"
  chgrp "$delegated_gid" "$H/delegated"
  chmod 2775 "$H/delegated"
  if run sh "$INSTALL" --prefix "$H/delegated/p" >"$H/out" 2>&1; then
    fail "a root-owned parent writable by its group is refused"
  else
    pass "a root-owned parent writable by its group is refused"
  fi
  check "nothing installed under a root-owned parent writable by its group" [ ! -e "$H/delegated/p/lib/swing/swing" ]
else
  echo "skip a root-owned parent writable by its group (not running as root)"
fi
mkdir "$H/sticky"
chmod 1777 "$H/sticky"
run sh "$INSTALL" --prefix "$H/sticky/p" >"$H/out" 2>&1 || fail "a sticky world-writable parent is accepted"
check "installed under a sticky parent" [ -x "$H/sticky/p/lib/swing/swing" ]
chmod 1777 "$H/sticky/p/lib/swing"
if run sh "$INSTALL" --prefix "$H/sticky/p" >"$H/out" 2>&1; then
  fail "a world-writable lib/swing is refused"
else
  pass "a world-writable lib/swing is refused"
fi
chmod 755 "$H/sticky/p/lib/swing"

echo "--- an install in a directory that became writable by others"
new_env loose
mkdir "$H/loose"
run sh "$INSTALL" --prefix "$H/loose/p" >"$H/out" 2>&1 || fail "install before the parent is loosened exits 0"
chmod 777 "$H/loose"
if REL=v0.2.0 run sh "$INSTALL" --prefix "$H/loose/p" >"$H/out" 2>&1; then
  fail "an upgrade under a loosened parent is refused"
else
  pass "an upgrade under a loosened parent is refused"
fi
check "upgrade refusal suggests a private prefix" contains "$H/out" "--prefix /opt/swing"
check "upgrade refusal suggests uninstalling first" contains "$H/out" "install.sh --uninstall --prefix $H/loose/p"
check "binary not upgraded" [ "$(run "$H/loose/p/lib/swing/swing" --version)" = "swing 0.1.0" ]
lib=$H/loose/p/lib/swing
unit_for "$H/.config/systemd/user/swing.service" "$lib/swing"
printf '../victim\n.hidden\n' >>"$lib/manifest"
: >"$FAKE_LOG"
if run sh "$INSTALL" --uninstall --prefix "$H/loose/p" >"$H/out" 2>&1; then
  fail "uninstall under a loosened parent is refused"
else
  pass "uninstall under a loosened parent is refused"
fi
check "uninstall refusal names the directory" contains "$H/out" "$H/loose is writable by its group or other users"
check "uninstall refusal lists the files" contains "$H/out" "rm -f -- '$lib/swing'"
check "uninstall refusal lists the manifest" contains "$H/out" "rm -f -- '$lib/manifest'"
check "uninstall refusal lists the bin link" contains "$H/out" "rm -f -- '$H/loose/p/bin/swing'"
check "uninstall refusal lists lib/swing" contains "$H/out" "rmdir -- '$lib'"
check "uninstall refusal points at the user service" contains "$H/out" "first remove the service in $H/.config/systemd/user/swing.service"
check "uninstall refusal offers fixing the permissions" contains "$H/out" "make $H/loose writable only by its owner"
check "uninstall refusal skips names outside lib" lacks "$H/out" "victim"
check "uninstall refusal skips dot names" lacks "$H/out" ".hidden"
check "uninstall refusal does not purge" lacks "$H/out" "rm -rf"
check "swing not run from a loosened directory" lacks "$FAKE_LOG" "swing service"
check "nothing removed under a loosened parent" [ -x "$lib/swing" ]
check "bin link kept under a loosened parent" [ -L "$H/loose/p/bin/swing" ]
check "user unit kept under a loosened parent" [ -f "$H/.config/systemd/user/swing.service" ]
rm -f "$H/.config/systemd/user/swing.service"
mkdir -p "$H/.local/share/swing"
if run sh "$INSTALL" --uninstall --purge --yes --prefix "$H/loose/p" >"$H/out" 2>&1; then
  fail "purge under a loosened parent is refused"
else
  pass "purge under a loosened parent is refused"
fi
check "purge refusal lists the data directory" contains "$H/out" "rm -rf -- '$H/.local/share/swing'"
check "data kept under a loosened parent" [ -d "$H/.local/share/swing" ]
check "nothing removed by the refused purge" [ -x "$lib/swing" ]
grep '^  r' "$H/out" | grep -v 'rm -rf' >"$H/manual.sh"
sh "$H/manual.sh" || fail "the listed commands run"
check "the listed commands remove lib/swing" absent "$lib"
check "the listed commands remove the bin link" absent "$H/loose/p/bin/swing"
check "the listed commands keep other files" [ -d "$H/loose/p/lib" ]
run sh "$INSTALL" --prefix "$H/loose/p2" >"$H/out" 2>&1 && fail "install under a loosened parent is refused"
chmod 755 "$H/loose"
run sh "$INSTALL" --prefix "$H/loose/p2" >"$H/out" 2>&1 || fail "install after tightening the parent exits 0"
chmod 777 "$H/loose"
run sh "$INSTALL" --uninstall --prefix "$H/loose/p2" >"$H/out" 2>&1 && fail "uninstall under a loosened parent fails"
chmod 755 "$H/loose"
run sh "$INSTALL" --uninstall --prefix "$H/loose/p2" >"$H/out" 2>&1 || fail "uninstall after tightening the parent exits 0"
check "uninstall after tightening the parent removes lib/swing" absent "$H/loose/p2/lib/swing"

echo "--- uninstall through a symlinked prefix removes the bin link"
new_env linked
mkdir "$H/real"
ln -s "$H/real" "$H/linked"
run sh "$INSTALL" --prefix "$H/linked" >/dev/null 2>&1 || fail "install through a symlinked prefix exits 0"
check "bin link names the logical path" [ "$(readlink "$H/real/bin/swing")" = "$H/linked/lib/swing/swing" ]
run "$H/linked/lib/swing/swing-uninstall.sh" >/dev/null 2>&1 || fail "uninstall through a symlinked prefix exits 0"
check "lib removed through a symlinked prefix" absent "$H/real/lib/swing"
check "bin link removed through a symlinked prefix" absent "$H/real/bin/swing"

if [ -n "$delegated_gid" ]; then
  mkdir "$H/staff"
  run sh "$INSTALL" --prefix "$H/staff/p" >"$H/out" 2>&1 || fail "install before the parent is delegated exits 0"
  chgrp "$delegated_gid" "$H/staff"
  chmod 2775 "$H/staff"
  if run sh "$INSTALL" --uninstall --prefix "$H/staff/p" >"$H/out" 2>&1; then
    fail "uninstall under a root-owned parent writable by its group is refused"
  else
    pass "uninstall under a root-owned parent writable by its group is refused"
  fi
  check "group refusal lists the files" contains "$H/out" "rm -f -- '$H/staff/p/lib/swing/swing'"
  check "nothing removed under a root-owned parent writable by its group" [ -x "$H/staff/p/lib/swing/swing" ]
  mkdir "$H/leaf"
  run sh "$INSTALL" --prefix "$H/leaf/p" >"$H/out" 2>&1 || fail "install before lib/swing is delegated exits 0"
  chgrp "$delegated_gid" "$H/leaf/p/lib/swing"
  chmod 2775 "$H/leaf/p/lib/swing"
  if REL=v0.2.0 run sh "$INSTALL" --prefix "$H/leaf/p" >"$H/out" 2>&1; then
    fail "a root-owned lib/swing writable by its group is refused"
  else
    pass "a root-owned lib/swing writable by its group is refused"
  fi
  check "refusal names lib/swing" contains "$H/out" "$H/leaf/p/lib/swing is writable by its group or other users"
  mkdir "$H/foreign"
  run sh "$INSTALL" --prefix "$H/foreign/p" >"$H/out" 2>&1 || fail "install before lib/swing changes hands exits 0"
  chown nobody "$H/foreign/p/lib/swing"
  if run sh "$INSTALL" --uninstall --prefix "$H/foreign/p" >"$H/out" 2>&1; then
    fail "uninstall from a lib/swing owned by another user is refused"
  else
    pass "uninstall from a lib/swing owned by another user is refused"
  fi
  check "refusal names the owner problem" contains "$H/out" "owned by another user"
  check "nothing removed from a lib/swing owned by another user" [ -x "$H/foreign/p/lib/swing/swing" ]
  mkdir "$H/mixed"
  run sh "$INSTALL" --prefix "$H/mixed/p" >"$H/out" 2>&1 || fail "install before the owners change exits 0"
  chmod 777 "$H/mixed/p/lib/swing"
  chown nobody "$H/mixed"
  if run sh "$INSTALL" --uninstall --prefix "$H/mixed/p" >"$H/out" 2>&1; then
    fail "uninstall under a parent owned by another user is refused"
  else
    pass "uninstall under a parent owned by another user is refused"
  fi
  check "another owner above a loose lib/swing is reported" contains "$H/out" "$H/mixed is owned by another user"
  check "no commands listed under a parent owned by another user" lacks "$H/out" "rm -f --"
  check "nothing removed under a parent owned by another user" [ -x "$H/mixed/p/lib/swing/swing" ]
  mkdir "$H/swap"
  run sh "$INSTALL" --prefix "$H/swap/p" >"$H/out" 2>&1 || fail "install before lib/swing is swapped exits 0"
  run sh "$INSTALL" --prefix "$H/victim" >"$H/out" 2>&1 || fail "install of the other copy exits 0"
  chmod 777 "$H/swap/p/lib"
  mv "$H/swap/p/lib/swing" "$H/swap/p/lib/real"
  ln -s "$H/victim/lib/swing" "$H/swap/p/lib/swing"
  if run sh "$INSTALL" --uninstall --prefix "$H/swap/p" >"$H/out" 2>&1; then
    fail "uninstall through a lib/swing swapped to another install is refused"
  else
    pass "uninstall through a lib/swing swapped to another install is refused"
  fi
  check "the other install is kept" [ -x "$H/victim/lib/swing/swing" ]
  check "swing not run through a swapped lib/swing" lacks "$FAKE_LOG" "swing service"
  rm "$H/swap/p/lib/swing"
  mkdir "$H/evil"
  echo victim >"$H/evil/victim"
  printf 'victim\n' >"$H/evil/manifest"
  chown -R nobody "$H/evil"
  ln -s "$H/evil" "$H/swap/p/lib/swing"
  if run sh "$INSTALL" --uninstall --prefix "$H/swap/p" >"$H/out" 2>&1; then
    fail "uninstall through a lib/swing swapped to another user's directory is refused"
  else
    pass "uninstall through a lib/swing swapped to another user's directory is refused"
  fi
  check "file in the swapped-in directory kept" [ -f "$H/evil/victim" ]
  mkdir "$H/mf"
  run sh "$INSTALL" --prefix "$H/mf/p" >"$H/out" 2>&1 || fail "install before the manifest changes hands exits 0"
  chown nobody "$H/mf/p/lib/swing/manifest"
  if run sh "$INSTALL" --uninstall --prefix "$H/mf/p" >"$H/out" 2>&1; then
    fail "uninstall with a manifest owned by another user is refused"
  else
    pass "uninstall with a manifest owned by another user is refused"
  fi
  check "nothing removed when the manifest is owned by another user" [ -x "$H/mf/p/lib/swing/swing" ]
else
  echo "skip root-owned directories writable by their group (not running as root)"
fi

echo "--- bad usage"
new_env usage
check "unknown option fails" sh -c "! env HOME='$H' sh '$INSTALL' --nope >/dev/null 2>&1"
check "relative prefix fails" sh -c "! env HOME='$H' sh '$INSTALL' --prefix rel >/dev/null 2>&1"
check "purge without uninstall fails" sh -c "! env HOME='$H' sh '$INSTALL' --purge >/dev/null 2>&1"
check "a version with a slash fails" sh -c "! env HOME='$H' sh '$INSTALL' --version 1/../2 >/dev/null 2>&1"
check "a relative HOME fails" sh -c "! env HOME=rel sh '$INSTALL' --prefix '$H/p' >/dev/null 2>&1"

echo
if [ "$FAILED" -ne 0 ]; then
  echo "$FAILED check(s) failed"
  exit 1
fi
echo "all checks passed"
