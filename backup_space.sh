#!/usr/bin/env bash
# Backup local state for one top-level space (presumes subs is stopped).
#
# Archives everything under $SUBS_DATA_DIR/<space>/ — typically:
#   subs.db          SQLite (handles, commitments, receipts)
#   <space>.sdb      SpaceDB Merkle tree
# plus any companion files beside the .sdb (e.g. hash indexes).
#
# Usage:
#   SUBS_BACKUP_DIR=./backups ./backup_space.sh @space
#   SUBS_DATA_DIR=./datamad SUBS_BACKUP_DIR=./backups ./backup_space.sh space
#   ./backup_space.sh -d /data -o /mnt/backups @mad
#
# Output: $SUBS_BACKUP_DIR/<space>-YYYYMMDD.tar.gz without leading @
#         (then …a, …b, … if taken)

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: backup_space.sh [options] <space>

  Backup one operated space's on-disk state into a dated tar.gz archive.
  Stop subs before running so SQLite/SpaceDB files are consistent.

Arguments:
  <space>              Top-level space name, with or without leading @
                       (e.g. @space or space)

Options:
  -d, --data-dir DIR   Subs data directory (default: $SUBS_DATA_DIR or ./data)
  -o, --output-dir DIR Directory for the archive (default: $SUBS_BACKUP_DIR)
  -h, --help           Show this help

Environment:
  SUBS_DATA_DIR        Used when -d / --data-dir is not given
  SUBS_BACKUP_DIR      Destination for archives (required unless -o is set)
EOF
}

DATA_DIR="${SUBS_DATA_DIR:-./data}"
OUTPUT_DIR="${SUBS_BACKUP_DIR:-}"
SPACE_ARG=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    -d|--data-dir)
      [[ $# -ge 2 ]] || { echo "error: $1 requires a value" >&2; exit 2; }
      DATA_DIR="$2"
      shift 2
      ;;
    -o|--output-dir)
      [[ $# -ge 2 ]] || { echo "error: $1 requires a value" >&2; exit 2; }
      OUTPUT_DIR="$2"
      shift 2
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
      if [[ -n "$SPACE_ARG" ]]; then
        echo "error: unexpected argument: $1" >&2
        usage >&2
        exit 2
      fi
      SPACE_ARG="$1"
      shift
      ;;
  esac
done

if [[ -z "$SPACE_ARG" ]]; then
  echo "error: space name is required" >&2
  usage >&2
  exit 2
fi

if [[ -z "$OUTPUT_DIR" ]]; then
  echo "error: set SUBS_BACKUP_DIR or pass -o / --output-dir" >&2
  usage >&2
  exit 2
fi

# Normalize to leading @ (matches Operator::data_dir.join(space.to_string())).
if [[ "$SPACE_ARG" == @* ]]; then
  SPACE="$SPACE_ARG"
else
  SPACE="@${SPACE_ARG}"
fi

# Archive filename without leading @ (paths inside the archive still use @space/).
SPACE_FILE="${SPACE#@}"
SPACE_FILE="${SPACE_FILE//\//_}"
SPACE_FILE="${SPACE_FILE//\\/_}"

SPACE_DIR="${DATA_DIR%/}/${SPACE}"
if [[ ! -d "$SPACE_DIR" ]]; then
  echo "error: space directory not found: $SPACE_DIR" >&2
  exit 1
fi
if [[ ! -f "$SPACE_DIR/subs.db" ]]; then
  echo "error: missing subs.db in $SPACE_DIR (not a space data dir?)" >&2
  exit 1
fi

SDB="${SPACE_DIR}/${SPACE}.sdb"
if [[ ! -f "$SDB" ]]; then
  echo "error: missing ${SPACE}.sdb in $SPACE_DIR" >&2
  exit 1
fi

if ! command -v tar >/dev/null 2>&1; then
  echo "error: tar is required but not installed" >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"
# Resolve absolute path before building the archive path.
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
DATE="$(date +%Y%m%d)"
BASE="${OUTPUT_DIR}/${SPACE_FILE}-${DATE}"

OUT="${BASE}.tar.gz"
if [[ -e "$OUT" ]]; then
  suffix=a
  while [[ -e "${BASE}${suffix}.tar.gz" ]]; do
    # next letter after z is unsupported — fail clearly
    if [[ "$suffix" == z ]]; then
      echo "error: exhausted suffixes a–z for ${BASE}*.tar.gz" >&2
      exit 1
    fi
    # increment single letter a..z
    suffix="$(printf '%s' "$suffix" | tr 'a-y' 'b-z')"
  done
  OUT="${BASE}${suffix}.tar.gz"
fi

echo "Assuming subs is stopped."
echo "Backing up ${SPACE} from ${SPACE_DIR}"
echo "Writing ${OUT}"

# Store paths as <space>/… so restore can unpack into a data dir.
# Prefix with ./ so BSD tar does not treat leading @ as an archive-include op.
tar -czf "$OUT" -C "${DATA_DIR%/}" "./${SPACE}"

echo "Done: $OUT"
ls -lh "$OUT"
