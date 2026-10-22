#!/usr/bin/env bash
# Builds an edition of DesignCraft as a macOS app (docs/editions.md): DESIGNCRAFT_EDITION names the
# edition's folder, an absolute path. The app lands in $CARGO_TARGET_DIR/edition/<name>.app
# (target/edition by default), signed ad hoc, so it runs on this Mac only; a DMG and notarisation
# are package.sh's job, for DesignCraft itself.
#
#   DESIGNCRAFT_EDITION=/abs/path/to/edition packaging/macos/edition.sh
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# ROOT, CARGO_TARGET_DIR, the build commit and date the binary carries, copy_font_licences.
# shellcheck source=../env.sh
. "$HERE/../env.sh"

case "${DESIGNCRAFT_EDITION:-}" in
  "") echo "error: set DESIGNCRAFT_EDITION to the edition's folder (an absolute path)" >&2; exit 2 ;;
  /*) [ -d "$DESIGNCRAFT_EDITION" ] || { echo "error: no folder at DESIGNCRAFT_EDITION=$DESIGNCRAFT_EDITION" >&2; exit 2; } ;;
  *) echo "error: DESIGNCRAFT_EDITION must be an absolute path: $DESIGNCRAFT_EDITION" >&2; exit 2 ;;
esac

# As package.sh: it must match LSMinimumSystemVersion in Info.plist.in.
export MACOSX_DEPLOYMENT_TARGET=11.0

cd "$ROOT"
echo "==> building DesignCraft with the edition in $DESIGNCRAFT_EDITION"
cargo build --release --locked -p designcraft
BIN="$CARGO_TARGET_DIR/release/designcraft"

# The build's own account of the edition (apps/designcraft/src/main.rs: edition_info).
info="$("$BIN" --edition-info)"
field() { plutil -extract "$1" raw -o - - <<<"$info"; }
NAME="$(field name)"
BUNDLE_ID="$(field bundleId)"
VERSION="$(field version)"
SHORT_VERSION="${VERSION%%-*}"
SHA="${DESIGNCRAFT_BUILD_SHA:-unknown}"
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

OUT="$CARGO_TARGET_DIR/edition"
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
# The craft-fonts licences, when the build embeds them (CRAFT_FONTS_DIR; see env.sh).
copy_font_licences "$APP/Contents/Resources"
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
