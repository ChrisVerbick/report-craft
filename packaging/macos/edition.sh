#!/usr/bin/env bash
# Builds an edition of DesignCraft as a macOS app (docs/editions.md): DESIGNCRAFT_EDITION names the
# edition's folder, an absolute path. The app lands in target/edition/<name>.app, signed ad hoc, so
# it runs on this Mac only; a DMG and notarisation are package.sh's job, for DesignCraft itself.
#
#   DESIGNCRAFT_EDITION=/abs/path/to/edition packaging/macos/edition.sh
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

case "${DESIGNCRAFT_EDITION:-}" in
  "") echo "error: set DESIGNCRAFT_EDITION to the edition's folder (an absolute path)" >&2; exit 2 ;;
  /*) [ -d "$DESIGNCRAFT_EDITION" ] || { echo "error: no folder at DESIGNCRAFT_EDITION=$DESIGNCRAFT_EDITION" >&2; exit 2; } ;;
  *) echo "error: DESIGNCRAFT_EDITION must be an absolute path: $DESIGNCRAFT_EDITION" >&2; exit 2 ;;
esac

cd "$ROOT"
echo "==> building DesignCraft with the edition in $DESIGNCRAFT_EDITION"
cargo build --release -p designcraft
BIN="$ROOT/target/release/designcraft"

# The build's own account of the edition (apps/designcraft/src/main.rs: edition_info).
info="$("$BIN" --edition-info)"
field() { plutil -extract "$1" raw -o - - <<<"$info"; }
NAME="$(field name)"
BUNDLE_ID="$(field bundleId)"
VERSION="$(field version)"
SHORT_VERSION="${VERSION%%-*}"
SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
if [ "$BUNDLE_ID" = "ai.storyteller.designcraft" ]; then
  echo "error: the build carries no edition" >&2
  exit 1
fi
# The name is a file name and plist text: no slashes, backslashes or XML specials.
bad='[/\\<>&"]'
if [ -z "$NAME" ] || [[ "$NAME" =~ $bad ]]; then
  echo "error: the edition's name can't name an app: \"$NAME\"" >&2
  exit 1
fi

OUT="$ROOT/target/edition"
APP="$OUT/$NAME.app"
rm -rf "$APP" "$OUT/icon.iconset"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

echo "==> icon"
"$BIN" --edition-icon "$OUT/icon.iconset"
iconutil -c icns "$OUT/icon.iconset" -o "$OUT/$NAME.icns"

echo "==> assembling $APP"
# Executable and icon carry the app's name (CFBundleExecutable, CFBundleIconFile), as in package.sh.
cp "$BIN" "$APP/Contents/MacOS/$NAME"
cp "$OUT/$NAME.icns" "$APP/Contents/Resources/$NAME.icns"
cp "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$ROOT/NOTICE" "$APP/Contents/Resources/"
# Info.plist.in names DesignCraft; the edition's name and bundle ID take its place (the document
# types keep DesignCraft's identifiers, so both apps open .designcraft files).
esc() { printf '%s' "$1" | sed -e 's/[\\|&]/\\&/g'; }
sed -e "s|<string>DesignCraft</string>|<string>$(esc "$NAME")</string>|g" \
  -e "s|<string>ai\.storyteller\.designcraft</string>|<string>$(esc "$BUNDLE_ID")</string>|" \
  -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" -e "s/@BUILD_SHA@/$SHA/g" \
  "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"

echo "==> signing ad hoc (this Mac only)"
codesign --force --deep -s - "$APP"
codesign -v "$APP"
echo "==> $APP"
