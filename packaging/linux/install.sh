#!/bin/sh
set -eu

KUBO_VERSION=0.43.1
KUBO_SHA512_AMD64=ff53b2428794fc8cca39505d28c15b4cceaef4b90a09284f291f611fcdc8c06399690575fc89cbd4d85ef71f3652581296c6f2e710386f887c8edf36b6e89d71
KUBO_SHA512_ARM64=70f082584651ef78fb5b07448be53bb0adf141aadcb7eea15ffa59bd9ca78fed46781c2cf5915b3528c591052f97486067fedb0a7a1415abd5bfd44942921501

REPO_URL=https://github.com/amane-katagiri/swing
RELEASES_URL=$REPO_URL/releases
KUBO_BASE_URL=https://dist.ipfs.tech/kubo/v$KUBO_VERSION
SYSTEM_UNIT=/etc/systemd/system/swing.service

TMP=
STOPPED=
LIB=
BIN=

SAFE_SED=$(printf 's/[[:cntrl:]]/?/g;s/\302[\200-\237]/?/g;s/\342\200[\213-\217\250-\256]/?/g;s/\342\201[\240-\244\246-\257]/?/g;s/\357\273\277/?/g')

safe() {
  printf '%s' "$*" | tr '\n' '?' | LC_ALL=C sed "$SAFE_SED"
}

info() {
  printf '==> %s\n' "$(safe "$*")"
}

warn() {
  printf 'warning: %s\n' "$(safe "$*")" >&2
}

die() {
  printf 'error: %s\n' "$(safe "$*")" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: install.sh [options]

Install or upgrade swing and the Kubo (ipfs) binary it needs.

Options:
  --version TAG   Install the release TAG (for example v0.1.0) instead of the latest
  --prefix DIR    Install under DIR (lib/swing and bin); default: ~/.local
  --service       Run `swing service install` afterwards (user service)
  --force         Replace a swing in bin that this script did not install;
                  with --uninstall, continue although a system service exists
  --uninstall     Remove what this script installed (data is kept)
  --purge         With --uninstall, also delete the default config and data directory
  --yes           Do not ask for confirmation of --purge
  -h, --help      Show this help
EOF
}

cleanup() {
  status=$?
  trap - EXIT
  if [ -n "$TMP" ]; then
    rm -rf "$TMP"
  fi
  if [ -n "$LIB" ] && [ -z "$UNINSTALL" ]; then
    rm -f "$LIB"/.*."$$" "$BIN"/.*."$$"
  fi
  if [ "$status" -ne 0 ] && [ -n "$STOPPED" ]; then
    warn "the swing service was stopped and not restarted; start it with: $LIB/swing service start"
  fi
  exit "$status"
}

have() {
  command -v "$1" >/dev/null 2>&1
}

fetch() {
  if have curl; then
    curl --proto '=https' --tlsv1.2 -fsSL -o "$2" "$1" || die "download failed: $1"
  else
    wget --https-only -q -O "$2" "$1" || die "download failed: $1"
  fi
}

digest() {
  case $1 in
    256)
      if have sha256sum; then
        sha256sum "$2" | cut -d ' ' -f 1
      else
        shasum -a 256 "$2" | cut -d ' ' -f 1
      fi
      ;;
    512)
      if have sha512sum; then
        sha512sum "$2" | cut -d ' ' -f 1
      else
        shasum -a 512 "$2" | cut -d ' ' -f 1
      fi
      ;;
  esac
}

listed_hash() {
  awk -v n="$2" '{ f = $2; sub(/^\*/, "", f); if (f == n) { print $1; exit } }' "$1"
}

verify() {
  bits=$1
  file=$2
  expected=$3
  label=$4
  actual=$(digest "$bits" "$file")
  if [ "$actual" != "$expected" ]; then
    die "checksum mismatch for $label (expected $expected, got $actual)"
  fi
}

detect_platform() {
  os=$(uname -s)
  arch=$(uname -m)
  case $os in
    Linux) ;;
    Darwin) die "macOS is not supported by this script; install with Homebrew instead (see $REPO_URL)" ;;
    *) die "unsupported OS: $os (only Linux is supported)" ;;
  esac
  case $arch in
    x86_64 | amd64)
      TARGET=x86_64-unknown-linux-musl
      KUBO_ARCH=amd64
      KUBO_SHA512=$KUBO_SHA512_AMD64
      ;;
    aarch64 | arm64)
      TARGET=aarch64-unknown-linux-musl
      KUBO_ARCH=arm64
      KUBO_SHA512=$KUBO_SHA512_ARM64
      ;;
    *) die "unsupported architecture: $arch (only x86_64 and aarch64 are supported)" ;;
  esac
}

check_tools() {
  if ! have curl && ! have wget; then
    die "curl or wget is required"
  fi
  have tar || die "tar is required"
  if ! have sha256sum && ! have shasum; then
    die "sha256sum or shasum is required"
  fi
  if ! have sha512sum && ! have shasum; then
    die "sha512sum or shasum is required"
  fi
}

data_dir() {
  case ${XDG_DATA_HOME:-} in
    /*) printf '%s\n' "$XDG_DATA_HOME/swing" ;;
    *) printf '%s\n' "$HOME/.local/share/swing" ;;
  esac
}

user_unit() {
  case ${XDG_CONFIG_HOME:-} in
    /*) printf '%s\n' "$XDG_CONFIG_HOME/systemd/user/swing.service" ;;
    *) printf '%s\n' "$HOME/.config/systemd/user/swing.service" ;;
  esac
}

private_group() {
  have getent || return 1
  owner=$(getent passwd "$1") || return 1
  group=$(getent group "$2") || return 1
  [ "${group%%:*}" = "${owner%%:*}" ] || return 1
  case ${group##*:} in
    '' | "${owner%%:*}") return 0 ;;
  esac
  return 1
}

dir_problem() {
  target=$1
  PROBLEM=
  LOOSE=
  me=$(id -u)
  d=$target
  while [ ! -e "$d" ] && [ ! -L "$d" ]; do
    d=$(dirname -- "$d")
  done
  real=$(cd -P -- "$d" 2>/dev/null && pwd -P) || {
    PROBLEM="$d is not a directory"
    return 1
  }
  for p in "$d" "$real"; do
    leaf=
    if [ "$d" = "$target" ]; then
      leaf=1
    fi
    while :; do
      if [ ! -L "$p" ]; then
        # shellcheck disable=SC2046
        set -- $(ls -ldn -- "$p")
        if [ "$3" != 0 ] && [ "$3" != "$me" ]; then
          PROBLEM="$p is owned by another user"
          LOOSE=
          return 1
        fi
        loose=
        case $1 in
          ????????w*) loose=1 ;;
          ?????w*)
            private_group "$3" "$4" || loose=1
            ;;
        esac
        if [ -n "$loose" ] && [ -z "$LOOSE" ]; then
          sticky=
          case $1 in
            ?????????[tT]*) sticky=1 ;;
          esac
          if [ -n "$leaf" ] || [ -z "$sticky" ]; then
            PROBLEM="$p is writable by its group or other users"
            LOOSE=$p
          fi
        fi
      fi
      leaf=
      [ "$p" = / ] && break
      p=$(dirname -- "$p")
    done
  done
  [ -z "$LOOSE" ]
}

check_dir() {
  dir_problem "$1" && return 0
  if [ -n "$LOOSE" ]; then
    PROBLEM="$PROBLEM; install under a prefix only you or root can write to, for example --prefix /opt/swing"
    if [ -f "$LIB/manifest" ]; then
      PROBLEM="$PROBLEM, after removing the install in $PREFIX with: install.sh --uninstall --prefix $PREFIX"
    fi
  fi
  die "refusing to use $1: $PROBLEM"
}

manifest_entries() {
  while IFS= read -r f; do
    case $f in
      '' | manifest | .* | *[!A-Za-z0-9._-]*) continue ;;
    esac
    if [ -f "$1/$f" ] || [ -L "$1/$f" ]; then
      printf '%s\n' "$f"
    fi
  done <"$1/manifest"
}

links_into() {
  target=$(readlink -- "$1") || return 1
  case $target in
    /*/swing) ;;
    *) return 1 ;;
  esac
  [ "$(cd -P -- "${target%/swing}" 2>/dev/null && pwd -P)" = "$2" ]
}

owned_by_us() {
  # shellcheck disable=SC2046
  set -- $(ls -ldn -- "$1")
  [ "$3" = 0 ] || [ "$3" = "$(id -u)" ]
}

put() {
  tmp=$LIB/.$2.$$
  cp "$1" "$tmp"
  chmod "$3" "$tmp"
  mv -f "$tmp" "$LIB/$2"
}

service_is_ours() {
  check=$LIB/.swing-check.$$
  if [ ! -x "$check" ]; then
    cp "$src/swing" "$check"
    chmod 755 "$check"
  fi
  code=0
  "$check" service status "$@" --points-into "$LIB" >"$TMP/owner" 2>&1 || code=$?
  case $code in
    0) return 0 ;;
    3 | 4)
      info "leaving the swing service as is; it does not run swing from $LIB:"
      cat "$TMP/owner" >&2
      return 1
      ;;
    *)
      cat "$TMP/owner" >&2
      die "could not check whether the swing service runs swing from $LIB; nothing was changed"
      ;;
  esac
}

stop_service() {
  if ! have systemctl; then
    return 0
  fi
  if [ -f "$(user_unit)" ] && systemctl --user is-active --quiet swing 2>/dev/null; then
    service_is_ours || return 0
    info "stopping the swing service"
    if [ -x "$LIB/swing" ]; then
      "$LIB/swing" service stop || die "could not stop the swing service; nothing was changed"
    else
      systemctl --user stop swing || die "could not stop the swing service; nothing was changed"
    fi
    STOPPED=user
  elif [ -f "$SYSTEM_UNIT" ] && systemctl is-active --quiet swing 2>/dev/null; then
    service_is_ours --system || return 0
    if [ "$(id -u)" -eq 0 ]; then
      info "stopping the swing system service"
      systemctl stop swing || die "could not stop the swing system service; nothing was changed"
      STOPPED=system
    else
      warn "the swing system service is running and keeps using the old binary; restart it with: sudo systemctl restart swing"
    fi
  fi
}

restart_service() {
  case $STOPPED in
    user)
      info "starting the swing service"
      "$LIB/swing" service start || warn "could not start the swing service; run: swing service start"
      ;;
    system)
      info "starting the swing system service"
      systemctl start swing || warn "could not start the swing system service; run: systemctl start swing"
      ;;
  esac
  STOPPED=
}

obtain_self() {
  case ${0##*/} in
    install.sh | swing-uninstall.sh)
      if [ -f "$0" ]; then
        SELF=$0
        return 0
      fi
      ;;
  esac
  SELF=
  want=$(listed_hash "$TMP/SHA256SUMS" install.sh)
  if [ -z "$want" ]; then
    warn "install.sh is not listed in the release checksums; swing-uninstall.sh will not be installed"
    return 0
  fi
  fetch "$BASE_URL/install.sh" "$TMP/install.sh"
  verify 256 "$TMP/install.sh" "$want" install.sh
  SELF=$TMP/install.sh
}

do_install() {
  detect_platform
  check_tools

  TMP=$(mktemp -d)

  if [ -n "$VERSION" ]; then
    BASE_URL=$RELEASES_URL/download/$VERSION
  else
    BASE_URL=$RELEASES_URL/latest/download
  fi

  info "fetching the release checksums"
  fetch "$BASE_URL/SHA256SUMS" "$TMP/SHA256SUMS"

  if [ -n "$VERSION" ]; then
    archive=swing-$VERSION-$TARGET.tar.gz
  else
    archive=$(awk -v t="-$TARGET.tar.gz" '
      { f = $2; sub(/^\*/, "", f) }
      f ~ /^swing-/ && substr(f, length(f) - length(t) + 1) == t { print f; exit }
    ' "$TMP/SHA256SUMS")
    case $archive in
      '' | */*) die "the release has no archive for $TARGET" ;;
    esac
  fi
  want=$(listed_hash "$TMP/SHA256SUMS" "$archive")
  [ -n "$want" ] || die "$archive is not listed in the release checksums"

  info "downloading $archive"
  fetch "$BASE_URL/$archive" "$TMP/$archive"
  verify 256 "$TMP/$archive" "$want" "$archive"
  mkdir "$TMP/swing"
  tar -xzf "$TMP/$archive" -C "$TMP/swing"
  src=$TMP/swing/${archive%.tar.gz}
  [ -f "$src/swing" ] || die "$archive does not contain swing"

  obtain_self

  check_dir "$LIB"
  check_dir "$BIN"

  need_kubo=1
  if [ -x "$LIB/ipfs" ] && [ "$("$LIB/ipfs" version --number 2>/dev/null || true)" = "$KUBO_VERSION" ]; then
    need_kubo=0
  fi
  if [ "$need_kubo" -eq 1 ]; then
    kubo_archive=kubo_v${KUBO_VERSION}_linux-$KUBO_ARCH.tar.gz
    info "downloading Kubo v$KUBO_VERSION"
    fetch "$KUBO_BASE_URL/$kubo_archive" "$TMP/$kubo_archive"
    verify 512 "$TMP/$kubo_archive" "$KUBO_SHA512" "$kubo_archive"
    mkdir "$TMP/kubo"
    tar -xzf "$TMP/$kubo_archive" -C "$TMP/kubo"
    [ -f "$TMP/kubo/kubo/ipfs" ] || die "$kubo_archive does not contain ipfs"
  fi

  link=$BIN/swing
  if [ -d "$link" ] && [ ! -L "$link" ]; then
    die "$link is a directory"
  fi
  if [ -e "$link" ] || [ -L "$link" ]; then
    if [ -z "$FORCE" ] && { [ ! -L "$link" ] || [ "$(readlink "$link")" != "$LIB/swing" ]; }; then
      die "$link already exists and was not installed by this script; remove it or use --force"
    fi
  fi

  mkdir -p "$LIB" "$BIN" || die "cannot create $LIB and $BIN (permission denied? try sudo with --prefix)"
  check_dir "$LIB"
  check_dir "$BIN"

  stop_service
  rm -f "$LIB/.swing-check.$$"

  info "installing into $LIB"
  put "$src/swing" swing 755
  if [ "$need_kubo" -eq 1 ]; then
    put "$TMP/kubo/kubo/ipfs" ipfs 755
    for f in APACHE MIT; do
      if [ -f "$TMP/kubo/kubo/LICENSE-$f" ]; then
        put "$TMP/kubo/kubo/LICENSE-$f" "LICENSE-kubo-$f" 644
      fi
    done
  fi
  for f in LICENSE LICENSE-PixelMplus.txt swing.example.toml README.md; do
    if [ -f "$src/$f" ]; then
      put "$src/$f" "$f" 644
    fi
  done
  if [ -n "$SELF" ]; then
    put "$SELF" swing-uninstall.sh 755
  fi

  new_files=$(
    cd "$LIB"
    for f in swing ipfs LICENSE-kubo-APACHE LICENSE-kubo-MIT LICENSE LICENSE-PixelMplus.txt swing.example.toml README.md swing-uninstall.sh; do
      if [ -f "$f" ]; then
        printf '%s\n' "$f"
      fi
    done
  )
  if [ -f "$LIB/manifest" ]; then
    manifest_entries "$LIB" | while IFS= read -r old; do
      if ! printf '%s\n' "$new_files" | grep -qxF -- "$old"; then
        rm -f "$LIB/$old"
      fi
    done
  fi
  printf '%s\n' "$new_files" >"$LIB/manifest.$$"
  mv -f "$LIB/manifest.$$" "$LIB/manifest"

  ln -s "$LIB/swing" "$BIN/.swing.$$"
  mv -f "$BIN/.swing.$$" "$link" || {
    rm -f "$BIN/.swing.$$"
    die "could not create $link"
  }

  installed=$("$LIB/swing" --version 2>&1) || warn "$LIB/swing does not run on this system"
  info "installed ${installed:-swing} at $link"

  if [ -n "$SERVICE" ]; then
    info "registering the swing service"
    "$LIB/swing" service install || die "swing service install failed"
    STOPPED=
  else
    restart_service
  fi

  case ":$PATH:" in
    *":$BIN:"*) ;;
    *) warn "$BIN is not on your PATH; add it to your shell profile" ;;
  esac

  if [ -n "$SERVICE" ]; then
    printf '\nThe service starts in setup mode if there is no configuration yet.\nOpen the dashboard with:\n  swing dashboard open\n'
  else
    printf '\nNext steps:\n  swing service install   # run swing at login (or: swing up)\n  swing dashboard open    # open the dashboard to finish the setup\n'
  fi
  printf 'Your configuration and data live in %s\n' "$(safe "$(data_dir)")"
  printf 'Uninstall with: %s\n' "$(safe "$LIB/swing-uninstall.sh")"
}

confirm_purge() {
  if [ -n "$YES" ]; then
    return 0
  fi
  if ! (: </dev/tty) 2>/dev/null; then
    die "cannot ask for confirmation without a terminal; pass --yes to confirm --purge"
  fi
  printf 'Delete %s, including your swing.toml (with the secret key) and all stored data? [y/N] ' "$(safe "$1")" >&2
  read -r answer </dev/tty || answer=
  case $answer in
    y | Y | yes | YES) ;;
    *) die "aborted; nothing was removed" ;;
  esac
}

do_uninstall() {
  [ -f "$LIB/manifest" ] || die "no installation found in $LIB"
  if ! dir_problem "$LIB"; then
    if [ -z "$LOOSE" ]; then
      die "refusing to use $LIB: $PROBLEM; nothing was removed"
    fi
    die "refusing to use $LIB: $PROBLEM; nothing was removed. Make $LOOSE writable only by its owner and run this again"
  fi
  # Everything below goes through this pinned directory so that a swap of the $LIB path after the checks cannot redirect it.
  cd -P -- "$LIB" || die "cannot enter $LIB"
  pinned=$(pwd -P)
  owned_by_us . || die "refusing to use $LIB: $pinned is owned by another user"
  if [ ! -f manifest ] || [ -L manifest ] || ! owned_by_us manifest; then
    die "refusing to use $LIB: $pinned/manifest is not a file of root or the current user"
  fi

  data=$(data_dir)
  if [ -n "$PURGE" ]; then
    case $data in
      */swing) ;;
      *) die "refusing to purge $data" ;;
    esac
    confirm_purge "$data"
  fi

  if [ -f "$SYSTEM_UNIT" ] && [ -z "$FORCE" ]; then
    code=0
    if [ -x ./swing ]; then
      ./swing service status --system --points-into "$LIB" >/dev/null 2>&1 || code=$?
    fi
    if [ "$code" -ne 4 ]; then
      die "a system service exists; run 'sudo $LIB/swing service uninstall --system' first, or use --force"
    fi
    info "leaving the swing system service as is; it does not run swing from $LIB"
  fi

  unit=$(user_unit)
  if [ -f "$unit" ]; then
    info "removing the swing service if it runs swing from $LIB"
    if [ -x ./swing ]; then
      ./swing service uninstall --only-from "$LIB" || die "swing service uninstall failed; nothing was removed"
    else
      warn "$unit exists but $LIB/swing is missing; remove the service by hand"
    fi
  fi

  info "removing files from $LIB"
  manifest_entries . | while IFS= read -r f; do
    rm -f "./$f"
  done
  rm -f ./manifest
  (
    cd -P -- "$BIN" 2>/dev/null || exit 0
    if [ -L swing ] && links_into swing "$pinned"; then
      rm -f ./swing
    fi
  )
  cd -P .. || die "cannot leave $pinned"
  rmdir -- "${pinned##*/}" 2>/dev/null || warn "$LIB is not empty and was left in place"

  if [ -n "$PURGE" ]; then
    info "removing $data"
    rm -rf "$data"
  else
    printf 'Your configuration and data were kept in %s\nRemove them by running the uninstaller again with --purge\n' "$(safe "$data")"
  fi
}

main() {
  umask 022
  VERSION=
  PREFIX=
  SERVICE=
  FORCE=
  UNINSTALL=
  PURGE=
  YES=
  SELF=

  case ${0##*/} in
    swing-uninstall.sh)
      UNINSTALL=1
      here=$(cd "$(dirname "$0")" && pwd -P)
      case $here in
        */lib/swing) PREFIX=${here%/lib/swing} ;;
      esac
      ;;
  esac

  while [ $# -gt 0 ]; do
    case $1 in
      --version)
        [ $# -ge 2 ] || die "--version needs a value"
        VERSION=v${2#v}
        case ${VERSION#v} in
          '' | *[!0-9A-Za-z.+-]*) die "invalid version: $2" ;;
        esac
        shift
        ;;
      --prefix)
        [ $# -ge 2 ] || die "--prefix needs a value"
        PREFIX=$2
        shift
        ;;
      --service) SERVICE=1 ;;
      --force) FORCE=1 ;;
      --uninstall) UNINSTALL=1 ;;
      --purge) PURGE=1 ;;
      --yes | -y) YES=1 ;;
      -h | --help)
        usage
        exit 0
        ;;
      *) die "unknown option: $1 (see --help)" ;;
    esac
    shift
  done

  case ${HOME:-} in
    /*) ;;
    *) die "HOME must be set to an absolute path" ;;
  esac
  if [ -z "$PREFIX" ]; then
    PREFIX=$HOME/.local
  fi
  case $PREFIX in
    /*) ;;
    *) die "--prefix must be an absolute path" ;;
  esac
  PREFIX=${PREFIX%/}
  LIB=$PREFIX/lib/swing
  BIN=$PREFIX/bin

  if [ -n "$PURGE" ] && [ -z "$UNINSTALL" ]; then
    die "--purge only works with --uninstall"
  fi
  if [ -n "$SERVICE" ] && [ "$(id -u)" -eq 0 ]; then
    die "--service registers a per-user service; run it as your regular user, not root"
  fi

  trap cleanup EXIT
  trap 'exit 1' HUP INT TERM

  if [ -n "$UNINSTALL" ]; then
    do_uninstall
  else
    do_install
  fi
}

main "$@"
