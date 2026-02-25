#!/usr/bin/env bash
# Post-build script: copies English content from out/en/ to out/ so that
# existing English URLs (/docs/...) continue to work alongside the
# locale-prefixed paths (/en/docs/..., /zh/docs/...).
#
# This is necessary because Next.js static export places [lang] segment
# content under /en/ and /zh/ directories, but hideLocale: 'default-locale'
# generates internal links without the /en/ prefix.

set -euo pipefail

OUT_DIR="$(dirname "$0")/../out"

if [ -d "$OUT_DIR/en" ]; then
  echo "Copying English content from out/en/ to out/ for unprefixed URLs..."
  cp -r "$OUT_DIR/en/"* "$OUT_DIR/" 2>/dev/null || true
  echo "Done. English content available at both /en/... and /... paths."
else
  echo "No out/en/ directory found — skipping i18n post-build step."
fi
