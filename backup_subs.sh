#!/usr/bin/env bash
# Backup all operated spaces under $SUBS_DATA_DIR, copy config.db, then
# archive the *contents* of $SUBS_BACKUP_DIR as ../subs-YYYYMMDD.tar.gz
# (…a, …b, … if taken) — the archive sits beside $SUBS_BACKUP_DIR and
# restores flat (no nested backup-dir folder).
#
# Presumes subs is stopped. Uses backup_space.sh for each space directory
# that contains a subs.db (same discovery rule as the operator).
#
# Usage:
#   SUBS_DATA_DIR=./datamad SUBS_BACKUP_DIR=./data_backup ./backup_subs.sh
#   ./backup_subs.sh -d ./datamad -o ./data_backup --clear

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKUP_SPACE="${SCRIPT_DIR}/backup_space.sh"

usage() {
  cat <<'EOF'
Usage: backup_subs.sh [options]

  Backup every operated space in the data dir, copy config.db into the
  backup dir, then tar the backup dir's contents as subs-YYYYMMDD.tar.gz
  placed beside $SUBS_BACKUP_DIR (its parent). Restore is flat — no
  nested backup-directory folder is created.

  Stop subs before running so SQLite/SpaceDB files are consistent.

Options:
  -d, --data-dir DIR   Subs data directory (default: $SUBS_DATA_DIR or ./data)
  -o, --output-dir DIR Backup directory (default: $SUBS_BACKUP_DIR)
  --clear              Delete all files/dirs inside $SUBS_BACKUP_DIR before
                       creating this run's space archives and config.db copy
                       (so the aggregate archive only contains this run)
  -h, --help           Show this help

Environment:
  SUBS_DATA_DIR        Used when -d / --data-dir is not given
  SUBS_BACKUP_DIR      Directory for per-space archives and config.db
                       (required unless -o is set). The aggregate
                       subs-YYYYMMDD.tar.gz is written next to this directory.
EOF
}

DATA_DIR="${SUBS_DATA_DIR:-./data}"
BACKUP_DIR="${SUBS_BACKUP_DIR:-}"
CLEAR=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    -d|--data-dir)
      [[ $# -ge 2 ]] || { echo "error: $1 requires a value" >&2; exit 2; }
      DATA_DIR="$2"
      shift 2
      ;;
    -o|--output-dir)
      [[ $# -ge 2 ]] || { echo "error: $1 requires a value" >&2; exit 2; }
      BACKUP_DIR="$2"
      shift 2
      ;;
    --clear)
      CLEAR=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      echo "error: unexpected argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$BACKUP_DIR" ]]; then
  echo "error: set SUBS_BACKUP_DIR or pass -o / --output-dir" >&2
  usage >&2
  exit 2
fi

if [[ ! -x "$BACKUP_SPACE" ]]; then
  echo "error: backup_space.sh not found or not executable at $BACKUP_SPACE" >&2
  exit 1
fi

if [[ ! -d "$DATA_DIR" ]]; then
  echo "error: data directory not found: $DATA_DIR" >&2
  exit 1
fi

if ! command -v tar >/dev/null 2>&1; then
  echo "error: tar is required but not installed" >&2
  exit 1
fi

mkdir -p "$BACKUP_DIR"
BACKUP_DIR="$(cd "$BACKUP_DIR" && pwd)"
DATA_DIR="$(cd "$DATA_DIR" && pwd)"
BACKUP_PARENT="$(dirname "$BACKUP_DIR")"

echo "Assuming subs is stopped."
echo "Data dir:   $DATA_DIR"
echo "Backup dir: $BACKUP_DIR"

if [[ "$CLEAR" -eq 1 ]]; then
  echo "Clearing $BACKUP_DIR ..."
  # Remove contents only; keep the directory itself.
  shopt -s nullglob dotglob
  for item in "$BACKUP_DIR"/*; do
    rm -rf "$item"
  done
  shopt -u nullglob dotglob
fi

# Discover spaces the same way as Operator: dirs containing subs.db.
spaces=()
shopt -s nullglob
for path in "$DATA_DIR"/*/; do
  dir="${path%/}"
  base="$(basename "$dir")"
  # Skip non-space dirs (e.g. nested data copies); require @ prefix and subs.db.
  if [[ "$base" == @* && -f "$dir/subs.db" ]]; then
    spaces+=("$base")
  fi
done
shopt -u nullglob

if [[ ${#spaces[@]} -eq 0 ]]; then
  echo "warning: no operated spaces found under $DATA_DIR" >&2
else
  echo "Found ${#spaces[@]} space(s): ${spaces[*]}"
fi

for space in "${spaces[@]}"; do
  echo
  echo "=== $space ==="
  "$BACKUP_SPACE" -d "$DATA_DIR" -o "$BACKUP_DIR" "$space"
done

CONFIG_SRC="${DATA_DIR}/config.db"
CONFIG_DST="${BACKUP_DIR}/config.db"
if [[ ! -f "$CONFIG_SRC" ]]; then
  echo "error: missing config.db at $CONFIG_SRC" >&2
  exit 1
fi
echo
echo "Copying config.db -> $CONFIG_DST"
cp -f "$CONFIG_SRC" "$CONFIG_DST"

DATE="$(date +%Y%m%d)"
# Aggregate archive lives beside $SUBS_BACKUP_DIR (same folder level).
BASE="${BACKUP_PARENT}/subs-${DATE}"
OUT="${BASE}.tar.gz"
if [[ -e "$OUT" ]]; then
  suffix=a
  while [[ -e "${BASE}${suffix}.tar.gz" ]]; do
    if [[ "$suffix" == z ]]; then
      echo "error: exhausted suffixes a–z for ${BASE}*.tar.gz" >&2
      exit 1
    fi
    suffix="$(printf '%s' "$suffix" | tr 'a-y' 'b-z')"
  done
  OUT="${BASE}${suffix}.tar.gz"
fi

echo
echo "Archiving contents of $BACKUP_DIR -> $OUT"
# Archive files inside the backup dir (not the dir itself), so restore
# does not recreate a nested data_backup/ folder.
tar -czf "$OUT" -C "$BACKUP_DIR" .

echo
echo "Done: $OUT"
ls -lh "$OUT"
