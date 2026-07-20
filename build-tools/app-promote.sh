#!/usr/bin/env bash
# Promuove una release BETA a UFFICIALE (stable): i dipendenti la ricevono.
# Uso:  ./build-tools/app-promote.sh 0.2.29
set -euo pipefail
VER="${1:?versione mancante, es. 0.2.29}"
REPO="progettoimmagina/pim-tomato-app"
echo "▶ Promuovo v$VER a UFFICIALE (canale stable = GitHub latest)…"
# togliere il flag prerelease + marcarla "latest" → /releases/latest/download/latest.json = v$VER
gh release edit "v$VER" --repo "$REPO" --prerelease=false --latest
echo "✓ v$VER è ora UFFICIALE: le app dei dipendenti si aggiorneranno al prossimo controllo (entro ~1h, o al riavvio)."
