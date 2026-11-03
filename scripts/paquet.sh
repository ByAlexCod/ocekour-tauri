#!/usr/bin/env bash
# Paquet de diffusion : Ocekour-<version>.zip (dossier Ocekour/ : Ocekour.exe, ocekour-headless.exe,
# linux/ocekour-headless statique, docs/, LISEZMOI.txt), à partir des binaires déjà compilés.
#   scripts/paquet.sh <dossier de sortie>
# CARGO_TARGET_DIR : dossier de compilation (par défaut target/).
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=$(realpath -m "${1:?dossier de sortie}")
TARGET=${CARGO_TARGET_DIR:-target}
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
WIN="$TARGET/x86_64-pc-windows-msvc/release"
LINUX="$TARGET/x86_64-unknown-linux-musl/release/ocekour-headless"

file "$WIN/ocekour.exe" | grep -q 'PE32+ executable (GUI) x86-64'
file "$WIN/ocekour-headless.exe" | grep -q 'PE32+ executable (console) x86-64'
file "$LINUX" | grep -q 'static'

STAGE="$OUT/Ocekour"
rm -rf "$STAGE" "$OUT/Ocekour-$VERSION.zip" && mkdir -p "$STAGE/docs" "$STAGE/linux"
cp "$WIN/ocekour.exe" "$STAGE/Ocekour.exe"
cp "$WIN/ocekour-headless.exe" "$STAGE/"
cp "$LINUX" "$STAGE/linux/"
cp docs/guide/sonde.md "$STAGE/linux/"
cp docs/guide/*.md "$STAGE/docs/"
sed "s/@VERSION@/$VERSION/" packaging/LISEZMOI.txt > "$STAGE/LISEZMOI.txt"
# Notes de release : section de la version dans le CHANGELOG.
awk -v v="## $VERSION" '$0==v{on=1;next} /^## /{on=0} on' docs/guide/CHANGELOG.md \
  | sed -E 's/\[([^]]*)\]\([^)]*\.md\)/\1/g' > "$OUT/notes.md"
(cd "$OUT" && zip -qr "Ocekour-$VERSION.zip" Ocekour)
echo "$OUT/Ocekour-$VERSION.zip"
