#!/bin/sh
set -eu
# Build a native development bundle without touching the user's Applications folder.
PROJECT_ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$PROJECT_ROOT"
PROFILE=debug
for arg in "$@"; do
    case "$arg" in
        --release) PROFILE=release ;;
        *) printf 'Unknown option: %s\n' "$arg" >&2; exit 2 ;;
    esac
done
if [ "$PROFILE" = release ]; then
    cargo build -p storyfella-atelier --release
else
    cargo build -p storyfella-atelier
fi
BUNDLE="$PROJECT_ROOT/dist/atelier/Atelier.app"
mkdir -p "$BUNDLE/Contents/MacOS"
# Replace the executable atomically so a running build keeps its original signed image.
cp "$PROJECT_ROOT/target/$PROFILE/atelier" "$BUNDLE/Contents/MacOS/Atelier.new"
mv "$BUNDLE/Contents/MacOS/Atelier.new" "$BUNDLE/Contents/MacOS/Atelier"
cat > "$BUNDLE/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Atelier</string>
<key>CFBundleDisplayName</key><string>Atelier</string>
<key>CFBundleIdentifier</key><string>com.storyfella.atelier</string>
<key>CFBundleExecutable</key><string>Atelier</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
</dict></plist>
PLIST
codesign --force --sign - --identifier com.storyfella.atelier --timestamp=none "$BUNDLE"
codesign --verify --strict "$BUNDLE"
printf 'Built %s\n' "$BUNDLE"
