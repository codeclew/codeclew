#!/bin/sh
set -eu
umask 077

REPOSITORY=codeclew/codeclew
RELEASE_BASE=${CODECLEW_RELEASE_BASE:-https://github.com/$REPOSITORY/releases}
INSTALL_ROOT=${CODECLEW_INSTALL_ROOT:-"$HOME/.local/share/codeclew"}
BIN_DIR=${CODECLEW_BIN_DIR:-"$HOME/.local/bin"}
REQUESTED_VERSION=${CODECLEW_VERSION:-latest}

fail() {
  printf 'codeclew installer: %s\n' "$1" >&2
  exit 1
}

progress() {
  printf '[codeclew] %s\n' "$1" >&2
}

progress '[1/6] Checking macOS and required tools...'

case "$RELEASE_BASE" in
  https://*) ;;
  http://127.0.0.1:*|http://localhost:*)
    [ "${CODECLEW_ALLOW_INSECURE_DOWNLOAD:-}" = 1 ] || fail "release URL must use HTTPS"
    ;;
  *) fail "release URL must use HTTPS" ;;
esac

case "$REQUESTED_VERSION" in
  latest) DOWNLOAD_ROOT=$RELEASE_BASE/latest/download ;;
  v[0-9]*.[0-9]*.[0-9]*) DOWNLOAD_ROOT=$RELEASE_BASE/download/$REQUESTED_VERSION ;;
  *) fail "CODECLEW_VERSION must be latest or vMAJOR.MINOR.PATCH" ;;
esac

[ "$(uname -s)" = Darwin ] || fail "the public pilot currently supports macOS only"
case "$(uname -m)" in
  arm64) ARCH=arm64 ;;
  x86_64) ARCH=x86_64 ;;
  *) fail "unsupported macOS architecture" ;;
esac

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v git >/dev/null 2>&1 || fail "Git is required"
command -v python3 >/dev/null 2>&1 || fail "Python 3.11 or newer is required"
python3 -I -S -c 'import sys; raise SystemExit(0 if sys.version_info >= (3, 11) else 1)' \
  || fail "Python 3.11 or newer is required"

ASSET=codeclew-macos-$ARCH.tar.gz
CHECKSUM=$ASSET.sha256
TMP_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/codeclew-install.XXXXXX")
cleanup() {
  chmod -R u+w "$TMP_ROOT" 2>/dev/null || true
  rm -rf -- "$TMP_ROOT"
}
trap cleanup EXIT HUP INT TERM

download() {
  case "$RELEASE_BASE" in
    https://*)
      curl --fail --show-error --location --progress-bar --proto '=https' --tlsv1.2 \
        "$1" --output "$2"
      ;;
    *)
      curl --fail --show-error --location --progress-bar "$1" --output "$2"
      ;;
  esac
}

progress "[2/6] Downloading the macOS $ARCH release (about 265 MB)..."
download "$DOWNLOAD_ROOT/$ASSET" "$TMP_ROOT/$ASSET"
progress '[3/6] Downloading and verifying the SHA-256 checksum...'
download "$DOWNLOAD_ROOT/$CHECKSUM" "$TMP_ROOT/$CHECKSUM"

EXPECTED=$(awk 'NR == 1 { print $1 }' "$TMP_ROOT/$CHECKSUM")
case "$EXPECTED" in
  *[!0-9a-f]*|'') fail "release checksum is invalid" ;;
esac
[ "${#EXPECTED}" -eq 64 ] || fail "release checksum is invalid"
ACTUAL=$(shasum -a 256 "$TMP_ROOT/$ASSET" | awk '{ print $1 }')
[ "$ACTUAL" = "$EXPECTED" ] || fail "release checksum mismatch"
progress '[3/6] Checksum verified.'

UNPACKED=$TMP_ROOT/unpacked
mkdir -m 700 "$UNPACKED"
progress '[4/6] Extracting the sealed runtime...'
python3 -I -S - "$TMP_ROOT/$ASSET" "$UNPACKED" <<'PY'
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import sys
import tarfile

archive = Path(sys.argv[1])
destination = Path(sys.argv[2])
member_count = 0
total_size = 0
with tarfile.open(archive, mode="r:gz") as bundle:
    members = bundle.getmembers()
    directory_modes = []
    if not members or len(members) > 100_000:
        raise SystemExit("release archive has an invalid entry count")
    for member in members:
        member_count += 1
        relative = PurePosixPath(member.name)
        if (
            relative.is_absolute()
            or ".." in relative.parts
            or not relative.parts
            or relative.parts[0] != "codeclew"
            or member.issym()
            or member.islnk()
            or member.isdev()
            or member.isfifo()
        ):
            raise SystemExit("release archive contains an unsafe entry")
        total_size += member.size
        if total_size > 2 * 1024 * 1024 * 1024:
            raise SystemExit("release archive is oversized")
        target = destination.joinpath(*relative.parts)
        if member.isdir():
            target.mkdir(mode=0o700, parents=True, exist_ok=True)
            directory_modes.append((target, member.mode & 0o700))
            continue
        if not member.isfile():
            raise SystemExit("release archive contains an unsupported entry")
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        source = bundle.extractfile(member)
        if source is None:
            raise SystemExit("release archive entry is unavailable")
        descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, member.mode & 0o700)
        try:
            with os.fdopen(descriptor, "wb", closefd=False) as output:
                shutil.copyfileobj(source, output, length=64 * 1024)
                output.flush()
                os.fsync(output.fileno())
        finally:
            os.close(descriptor)
        if target.stat().st_size != member.size:
            raise SystemExit("release archive entry changed during extraction")
    for directory, mode in sorted(directory_modes, key=lambda row: len(row[0].parts), reverse=True):
        directory.chmod(mode)
PY

PACKAGE=$UNPACKED/codeclew
[ -x "$PACKAGE/bin/clew" ] || fail "release launcher is missing"
[ -f "$PACKAGE/VERSION" ] || fail "release version is missing"
VERSION=$(sed -n '1p' "$PACKAGE/VERSION")
case "$VERSION" in
  v[0-9]*.[0-9]*.[0-9]*) ;;
  *) fail "release version is invalid" ;;
esac
if [ "$REQUESTED_VERSION" != latest ] && [ "$VERSION" != "$REQUESTED_VERSION" ]; then
  fail "downloaded release version does not match CODECLEW_VERSION"
fi

progress "[5/6] Activating Codeclew $VERSION..."
mkdir -p "$INSTALL_ROOT/releases" "$BIN_DIR"
chmod 700 "$INSTALL_ROOT" "$INSTALL_ROOT/releases" "$BIN_DIR"
DESTINATION=$INSTALL_ROOT/releases/$VERSION-macos-$ARCH
if [ -e "$DESTINATION" ]; then
  [ -x "$DESTINATION/bin/clew" ] || fail "existing release directory is incomplete"
else
  mv "$PACKAGE" "$DESTINATION"
  chmod 700 "$DESTINATION"
fi

if [ -e "$BIN_DIR/clew" ] && [ ! -L "$BIN_DIR/clew" ]; then
  fail "$BIN_DIR/clew exists and is not a symlink"
fi
LINK=$BIN_DIR/.clew-link.$$
ln -s "$DESTINATION/bin/clew" "$LINK"
mv -f "$LINK" "$BIN_DIR/clew"

progress '[6/6] Verifying the installed runtime...'
"$BIN_DIR/clew" capabilities >/dev/null
progress '[6/6] Runtime verification passed.'
printf 'Codeclew %s installed for macOS %s.\n' "$VERSION" "$ARCH"
printf 'Launcher: %s\n' "$BIN_DIR/clew"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) printf 'Add %s to PATH, then run: clew doctor\n' "$BIN_DIR" ;;
esac
