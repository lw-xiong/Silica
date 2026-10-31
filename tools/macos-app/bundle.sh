#!/usr/bin/env bash
# Wrap the silica binary in Silica.app, which is the only place macOS looks for an icon.
#
#   tools/macos-app/bundle.sh target/release/silica dist
#
# writes dist/Silica.app. The plist is crates/silica/resources/macos/Info.plist.in with the
# workspace version filled in, and the icon crates/silica/resources/macos/silica.icns, both
# cut from the logo master by tools/app-icons/cut.py. Runs anywhere; the ad-hoc signature at
# the end is the one step that needs a real Mac, and it is skipped elsewhere.
set -euo pipefail

binary=${1:?usage: bundle.sh <binary> <out-dir>}
out=${2:?usage: bundle.sh <binary> <out-dir>}

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
resources="$root/crates/silica/resources/macos"

version=$(sed -n '/^\[workspace\.package\]/,/^\[/p' "$root/Cargo.toml" \
  | sed -n 's/^version = "\(.*\)"$/\1/p')
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }

app="$out/Silica.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

install -m 755 "$binary" "$app/Contents/MacOS/silica"
install -m 644 "$resources/silica.icns" "$app/Contents/Resources/silica.icns"
sed "s/@VERSION@/$version/g" "$resources/Info.plist.in" > "$app/Contents/Info.plist"
# Launch Services still reads PkgInfo, and an app without one shows up as a folder on older
# systems; the two codes are the CFBundlePackageType and an unset creator.
printf 'APPL????' > "$app/Contents/PkgInfo"

# An ad-hoc signature seals the bundle so macOS does not call it damaged once something else
# has touched it. It is not notarization: a downloaded copy still needs right-click > Open.
if command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - --timestamp=none "$app"
  codesign --verify --strict "$app"
else
  echo "codesign not found; leaving $app unsigned (not a Mac?)" >&2
fi

echo "$app ($version)"
