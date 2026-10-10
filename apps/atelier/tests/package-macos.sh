#!/bin/sh
set -eu
PROJECT_ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
TEST_ROOT=$(mktemp -d "$PROJECT_ROOT/.cargo/tmp/atelier-packaging.XXXXXX")
trap 'rm -rf "$TEST_ROOT"' EXIT HUP INT TERM
mkdir -p "$TEST_ROOT/apps/atelier" "$TEST_ROOT/bin"
cp "$PROJECT_ROOT/apps/atelier/package-macos.sh" "$TEST_ROOT/apps/atelier/"
export CALLS="$TEST_ROOT/calls"
export PATH="$TEST_ROOT/bin:$PATH"
cat > "$TEST_ROOT/bin/cargo" <<'STUB'
#!/bin/sh
printf 'cargo %s\n' "$*" >> "$CALLS"
profile=debug
for arg in "$@"; do
    if [ "$arg" = --release ]; then profile=release; fi
done
mkdir -p "target/$profile"
printf '%s\n' "$profile" > "target/$profile/atelier"
STUB
cat > "$TEST_ROOT/bin/codesign" <<'STUB'
#!/bin/sh
printf 'codesign %s\n' "$*" >> "$CALLS"
STUB
chmod +x "$TEST_ROOT/bin/cargo" "$TEST_ROOT/bin/codesign"
cd "$TEST_ROOT"
sh apps/atelier/package-macos.sh > result 2>&1
rg -q -- '--sign - --identifier com.storyfella.atelier' "$CALLS"
test "$(cat dist/atelier/Atelier.app/Contents/MacOS/Atelier)" = debug
sh apps/atelier/package-macos.sh --release > result 2>&1
test "$(cat dist/atelier/Atelier.app/Contents/MacOS/Atelier)" = release
if sh apps/atelier/package-macos.sh --unknown > result 2>&1; then
    printf 'Unknown packaging option was accepted\n' >&2
    exit 1
fi
printf 'Atelier packaging checks passed\n'
