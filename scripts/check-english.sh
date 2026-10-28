#!/usr/bin/env bash
# Fails if any tracked text file contains Spanish: accented letters/¿¡ or common Spanish words.
# Run from the repo root. Exit 0 = clean, 1 = Spanish found (file:line listed).
set -euo pipefail
command -v rg >/dev/null || { echo "English check requires ripgrep (rg)." >&2; exit 1; }
cd "$(dirname "$0")/.."
words='que|también|aquí|está|están|según|después|porque|cuando|donde|vista|vistas|archivo|archivos|falta|listo|hecho|pendiente|usuario|usuarios|nombre|guardar|borrar|agregar|mensaje|ejemplo|prueba|pruebas|tarea|tareas|ningún|ninguna|todavía|ahora|siempre|nunca|entonces|desde|hasta|sobre|entre|cada|otro|otra|muy|pero|como|los|las|una|unos|unas'
hits=$(rg -n --hidden -g '!target' -g '!.git' -g '!node_modules' -g '!Cargo.lock' -g '!scripts/check-english.sh' \
  -e '[áéíóúñÁÉÍÓÚÑ¿¡]' -e "\\b(${words})\\b" .) || { scan_status=$?; if [[ "$scan_status" -ne 1 ]]; then exit "$scan_status"; fi; }
if [[ -n "$hits" ]]; then
  echo "$hits" | head -200
  echo "Spanish found in $(echo "$hits" | cut -d: -f1 | sort -u | wc -l | tr -d ' ') files ($(echo "$hits" | wc -l | tr -d ' ') lines)." >&2
  exit 1
fi
echo "English only: OK"
