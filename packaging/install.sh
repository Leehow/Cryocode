#!/bin/sh
# Build Cryocode.app, install it to /Applications, add the `code` command and
# (with --set-default) make it the default app for code and text formats.
set -eu
cd "$(dirname "$0")/.."
ROOT=$(pwd)
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
APP_ID=dev.cryocode.Cryocode
DEST=${CRYOCODE_APP_DIR:-/Applications}/Cryocode.app

cargo build --release

STAGE=$(mktemp -d)
trap 'chmod -R u+w "$STAGE" 2>/dev/null; rm -rf "$STAGE"' EXIT INT TERM
APP="$STAGE/Cryocode.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/cryocode "$APP/Contents/MacOS/cryocode"
cp packaging/Cryocode.icns "$APP/Contents/Resources/Cryocode.icns"
sed "s/__VERSION__/$VERSION/g" packaging/Info.plist > "$APP/Contents/Info.plist"
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true

# Replace the installed bundle (quit a running copy first so the binary isn't busy).
pkill -x cryocode 2>/dev/null || true
rm -rf "$DEST"
cp -R "$APP" "$DEST"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$DEST"
echo "installed $DEST"

# Terminal command: ~/.local/bin/code (put ~/.local/bin early in PATH).
mkdir -p "$HOME/.local/bin"
cat > "$HOME/.local/bin/code" <<SH
#!/bin/sh
# Installed by Cryocode
exec "$DEST/Contents/MacOS/cryocode" --cli "\$@"
SH
chmod 755 "$HOME/.local/bin/code"
echo "installed $HOME/.local/bin/code"

# Default-app registration is opt-in: recent macOS asks the user to confirm EVERY
# handler change with a dialog, so pass only the extensions you want, e.g.
#   packaging/install.sh --set-default md rs py ts json
if [ "${1:-}" = "--set-default" ]; then
  shift
  if [ $# -eq 0 ]; then
    echo "usage: $0 --set-default <ext> [ext…]   (macOS will ask to confirm each one)" >&2
    exit 1
  fi
  command -v duti >/dev/null || { echo "duti not found (brew install duti)" >&2; exit 1; }
  for ext in "$@"; do
    duti -s "$APP_ID" ".${ext#.}" all || echo "could not set .$ext" >&2
  done
fi
