#!/bin/sh
set -eu

here=$(cd "$(dirname "$0")" && pwd -P)
INSTALL=$here/install.sh

case $(uname -m) in
  x86_64 | amd64)
    TARGET=x86_64-unknown-linux-musl
    KUBO_ARCH=amd64
    ;;
  aarch64 | arm64)
    TARGET=aarch64-unknown-linux-musl
    KUBO_ARCH=arm64
    ;;
  *)
    echo "unsupported architecture for this test" >&2
    exit 1
    ;;
esac
KUBO_VERSION=$(sed -n 's/^KUBO_VERSION=//p' "$INSTALL")

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

make_swing_release() {
  tag=$1
  dir=$ROOT/release-$tag
  stage=$ROOT/stage-$tag/swing-$tag-$TARGET
  mkdir -p "$dir" "$stage"
  cat >"$stage/swing" <<EOF
#!/bin/sh
echo "swing \$*" >>"\$FAKE_LOG"
echo "$tag \$*" >>"\$FAKE_STATE/ran"
unit=\$HOME/.config/systemd/user/swing.service
case " \$* " in
  *" --system "*) unit=\${SWING_INSTALL_SYSTEM_UNIT:-/nonexistent} ;;
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
  (cd "$dir" && sha512sum "$name" >"$name.sha512")
}

make_swing_release v0.1.0
make_swing_release v0.2.0
make_kubo_dist

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
  env -u XDG_DATA_HOME -u XDG_CONFIG_HOME \
    HOME="$H" PATH="$FAKEBIN:$PATH" \
    FAKE_LOG="$FAKE_LOG" FAKE_STATE="$FAKE_STATE" \
    SWING_INSTALL_BASE_URL="file://$ROOT/release-${REL:-v0.1.0}" \
    SWING_INSTALL_KUBO_BASE_URL="file://$ROOT/kubo" \
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
new_env svc
out=$(run sh "$INSTALL" --service 2>&1) || fail "--service exits 0"
check "service install called" contains "$FAKE_LOG" "swing service install"
check "dashboard hint printed" sh -c "printf '%s' \"\$1\" | grep -q 'swing dashboard open'" _ "$out"

echo "--- checksum mismatch"
new_env bad
bad=$ROOT/release-bad
rm -rf "$bad"
cp -r "$ROOT/release-v0.1.0" "$bad"
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
if run env SWING_INSTALL_KUBO_BASE_URL="file://$badk" sh "$INSTALL" >"$H/out" 2>&1; then
  fail "tampered kubo is rejected"
else
  pass "tampered kubo is rejected"
fi
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
if run env SWING_INSTALL_SYSTEM_UNIT="$H/system-swing.service" sh "$INSTALL" --uninstall >"$H/out" 2>&1; then
  fail "uninstall is refused while a system service exists"
else
  pass "uninstall is refused while a system service exists"
fi
check "refusal names the system uninstall" contains "$H/out" "service uninstall --system"
check "install kept" [ -x "$H/.local/lib/swing/swing" ]
run env SWING_INSTALL_SYSTEM_UNIT="$H/system-swing.service" sh "$INSTALL" --uninstall --force >/dev/null 2>&1 || fail "--force uninstall exits 0"
check "--force uninstalls" absent "$H/.local/lib/swing"

echo "--- a system service of another swing does not block the uninstall"
new_env sysforeign
run sh "$INSTALL" >/dev/null 2>&1
unit_for "$H/system-swing.service" /usr/bin/swing
run env SWING_INSTALL_SYSTEM_UNIT="$H/system-swing.service" sh "$INSTALL" --uninstall >"$H/out" 2>&1 || fail "uninstall with a foreign system service exits 0"
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

echo "--- bad usage"
new_env usage
check "unknown option fails" sh -c "! env HOME='$H' sh '$INSTALL' --nope >/dev/null 2>&1"
check "relative prefix fails" sh -c "! env HOME='$H' sh '$INSTALL' --prefix rel >/dev/null 2>&1"
check "purge without uninstall fails" sh -c "! env HOME='$H' sh '$INSTALL' --purge >/dev/null 2>&1"

echo
if [ "$FAILED" -ne 0 ]; then
  echo "$FAILED check(s) failed"
  exit 1
fi
echo "all checks passed"
