#!/bin/zsh
# Builds Model Launcher.app (with the MCP server inside) and dist/Model Launcher.dmg.
#   ./packaging/build.sh            build
#   ./packaging/build.sh --install  build, then replace /Applications/Model Launcher.app
set -euo pipefail
cd "${0:A:h}/.."
export PATH="$HOME/.cargo/bin:$PATH"

cargo build --release -p ml-mcp
npx tauri build --bundles app --config app/src-tauri/tauri.conf.json

APP="target/release/bundle/macos/Model Launcher.app"
cp target/release/model-launcher-mcp "$APP/Contents/MacOS/model-launcher-mcp"
xattr -cr "$APP"
# Sign with the local "Computer Use Local Signing" certificate when this Mac has it (Computer Use's
# packaging makes it): the identity stays the same across builds, so macOS keeps file-access
# permissions after updates instead of asking again. Otherwise sign ad hoc.
SIGN=$(security find-certificate -c "Computer Use Local Signing" -Z 2>/dev/null | awk '/SHA-1/ {print $NF; exit}')
codesign --force --deep -s "${SIGN:--}" "$APP"

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
mkdir -p dist
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "dist/Model Launcher.dmg"
hdiutil create -volname "Model Launcher $VERSION" -srcfolder "$STAGE" -ov -format UDZO "dist/Model Launcher.dmg" >/dev/null
rm -rf "$STAGE"
echo "Built dist/Model Launcher.dmg ($VERSION)"

if [[ "${1:-}" == "--install" ]]; then
  osascript -e 'tell application "Model Launcher" to quit' 2>/dev/null || true
  sleep 1
  rm -rf "/Applications/Model Launcher.app"
  cp -R "$APP" "/Applications/Model Launcher.app"
  echo "Installed /Applications/Model Launcher.app"
fi
