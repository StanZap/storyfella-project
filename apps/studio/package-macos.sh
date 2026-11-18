#!/bin/sh
set -eu
# Build Studio as a signed development .app in dist/studio. Nothing outside the
# checkout is touched; `cargo xtask bundle --install` copies it to ~/Applications.
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
    cargo build -p storyfella-studio --release
else
    cargo build -p storyfella-studio
fi
VERSION=$(cargo pkgid -p storyfella-studio | sed 's/.*[#@]//')
BUNDLE="$PROJECT_ROOT/dist/studio/Storyfella Studio.app"
mkdir -p "$BUNDLE/Contents/MacOS"
# Replace the executable atomically so a running build keeps its original signed image.
cp "${CARGO_TARGET_DIR:-target}/$PROFILE/storyfella-studio" "$BUNDLE/Contents/MacOS/storyfella-studio.new"
mv "$BUNDLE/Contents/MacOS/storyfella-studio.new" "$BUNDLE/Contents/MacOS/storyfella-studio"
cat > "$BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Storyfella Studio</string>
<key>CFBundleDisplayName</key><string>Storyfella Studio</string>
<key>CFBundleIdentifier</key><string>dev.storyfella.studio</string>
<key>CFBundleExecutable</key><string>storyfella-studio</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$VERSION</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
</dict></plist>
PLIST
codesign --force --sign - --identifier dev.storyfella.studio --timestamp=none "$BUNDLE"
codesign --verify --strict "$BUNDLE"
printf 'Built %s\n' "$BUNDLE"
