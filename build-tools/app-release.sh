#!/usr/bin/env bash
# Release dell'APP Mac PIM Tomato con auto-update.
# Uso:  ./build-tools/app-release.sh 0.2.1 "Titolo/nota della versione"
# Fa: bump versione (Cargo.toml + tauri.conf.json) -> build firmata (chiave dal
# Keychain) -> stage artefatti + latest.json -> commit/tag/push -> GitHub release.
set -euo pipefail

VER="${1:?versione mancante, es. 0.2.1}"
TITLE="${2:-PIM Tomato $VER}"
REPO="progettoimmagina/pim-tomato-app"
APP="$(cd "$(dirname "$0")/.." && pwd)"
cd "$APP"

echo "▶ Release app v$VER: $TITLE"

# 1) chiave di firma updater dal Keychain (nessun segreto in chiaro)
KEY="$(security find-generic-password -a pimtomato-updater -s pimtomato-updater-key -w)"
export TAURI_SIGNING_PRIVATE_KEY="$KEY"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""

# 2) bump versione
/usr/bin/sed -i '' -E "s/^version = \"[0-9.]+\"/version = \"$VER\"/" src-tauri/Cargo.toml
/usr/bin/sed -i '' -E "s/(\"version\"[[:space:]]*:[[:space:]]*\")[0-9.]+(\")/\1$VER\2/" src-tauri/tauri.conf.json
echo "  ✓ versione -> $VER"

# 3) build firmata (app.tar.gz + .sig + dmg)
source "$HOME/.cargo/env"
echo "▶ Build (firma updater attiva)…"
npx tauri build >/tmp/pt-appbuild-$VER.log 2>&1 || { echo "❌ build fallita"; tail -20 /tmp/pt-appbuild-$VER.log; exit 1; }
echo "  ✓ build ok"

# 4) stage artefatti + latest.json
DIST="dist-release"; rm -rf "$DIST"; mkdir -p "$DIST"
TARGZ="src-tauri/target/release/bundle/macos/PIM Tomato.app.tar.gz"
DMG="src-tauri/target/release/bundle/dmg/PIM Tomato_${VER}_aarch64.dmg"
cp "$TARGZ" "$DIST/pim-tomato_${VER}_aarch64.app.tar.gz"
cp "$DMG"   "$DIST/PIM-Tomato_${VER}_aarch64.dmg"
SIG="$(cat "$TARGZ.sig")"
PUB="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cat > "$DIST/latest.json" <<JSON
{
  "version": "${VER}",
  "notes": "${TITLE}",
  "pub_date": "${PUB}",
  "platforms": {
    "darwin-aarch64": {
      "signature": "${SIG}",
      "url": "https://github.com/${REPO}/releases/download/v${VER}/pim-tomato_${VER}_aarch64.app.tar.gz"
    }
  }
}
JSON
echo "  ✓ artefatti + latest.json pronti"

# 5) commit + tag + push
git add -A
git commit -q -m "release v$VER: $TITLE" || true
git tag "v$VER" 2>/dev/null || true
git push -q origin main
git push -q origin "v$VER"
echo "  ✓ push + tag"

# 6) GitHub release (l'endpoint updater legge /releases/latest/download/latest.json)
gh release create "v$VER" --repo "$REPO" --title "$TITLE" --notes "$TITLE" \
  "$DIST/pim-tomato_${VER}_aarch64.app.tar.gz" \
  "$DIST/latest.json" \
  "$DIST/PIM-Tomato_${VER}_aarch64.dmg"
echo "✓ Release app v$VER pubblicata — le app installate si aggiorneranno da sole al prossimo avvio."
