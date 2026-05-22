#!/bin/sh
# Dispatch to subs, subs-prover, or registry-server.
# By default, starting subs also starts co-located services when their binaries exist.
# Usage:
#   docker run <image> subs [flags...]
#   docker run <image> subs-prover --server
#   docker run <image> registry-server --port 8080

set -eu

PROVER_PORT="${SUBS_PROVER_PORT:-8888}"
REGISTRY_PORT="${REGISTRY_SERVER_PORT:-8080}"

# Apply image defaults from build (only for unset variables).
load_image_defaults() {
    if [ ! -f /etc/subs-image.env ]; then
        return 0
    fi
    while IFS= read -r line || [ -n "$line" ]; do
        case "$line" in
            ''|\#*) continue ;;
        esac
        key="${line%%=*}"
        value="${line#*=}"
        eval "if [ -z \"\${$key+x}\" ]; then export $key=\"$value\"; fi"
    done < /etc/subs-image.env
}

load_image_defaults

if [ -n "${SUBS_PROVER_GPU_ACCELERATION:-}" ]; then
    echo "entrypoint: subs-prover GPU acceleration: ${SUBS_PROVER_GPU_ACCELERATION}"
fi

require_binary() {
    if [ ! -x "$1" ]; then
        echo "entrypoint: $1 is not available in this image (rebuild with ENABLE_PROVER/ENABLE_REGISTRY enabled)" >&2
        exit 1
    fi
}

# Start subs-prover in the background (co-located with subs).
start_prover_server() {
    if [ "${SUBS_START_PROVER:-1}" = "0" ] || [ ! -x /usr/local/bin/subs-prover ]; then
        return 0
    fi
    echo "entrypoint: starting subs-prover on 127.0.0.1:${PROVER_PORT}"
    SUBS_PROVER_SERVER=1 SUBS_PROVER_PORT="${PROVER_PORT}" \
        /usr/local/bin/subs-prover --server --server-port "${PROVER_PORT}" &
}

# Start registry-server in the background (co-located with subs).
start_registry_server() {
    if [ "${SUBS_START_REGISTRY:-1}" = "0" ] || [ ! -x /usr/local/bin/registry-server ]; then
        return 0
    fi
    echo "entrypoint: starting registry-server on 127.0.0.1:${REGISTRY_PORT}"
    /usr/local/bin/registry-server --port "${REGISTRY_PORT}" &
}

resolve_component() {
    if [ -n "${SUBS_COMPONENT:-}" ]; then
        printf '%s' "$SUBS_COMPONENT"
        return
    fi

    if [ "$#" -gt 0 ]; then
        case "$1" in
            subs|subs-prover|prover|registry-server|registry)
                printf '%s' "$1"
                return
                ;;
        esac
    fi

    printf '%s' "subs"
}

COMPONENT="$(resolve_component)"

case "$COMPONENT" in
    subs)
        BIN=/usr/local/bin/subs
        ;;
    subs-prover|prover)
        BIN=/usr/local/bin/subs-prover
        COMPONENT=subs-prover
        require_binary "$BIN"
        ;;
    registry-server|registry)
        BIN=/usr/local/bin/registry-server
        COMPONENT=registry-server
        require_binary "$BIN"
        ;;
    *)
        echo "entrypoint: unknown SUBS_COMPONENT '$COMPONENT' (expected subs, subs-prover, or registry-server)" >&2
        exit 1
        ;;
esac

# If the first argument was the component name, shift it off before exec.
if [ "$#" -gt 0 ]; then
    case "$1" in
        subs|subs-prover|prover|registry-server|registry)
            shift
            ;;
    esac
fi

if [ "$COMPONENT" = "subs" ]; then
    start_prover_server
    start_registry_server
    if [ -x /usr/local/bin/subs-prover ] && [ -z "${SUBS_PROVER_ENDPOINT:-}" ]; then
        export SUBS_PROVER_ENDPOINT="http://127.0.0.1:${PROVER_PORT}"
    fi
    if [ -x /usr/local/bin/registry-server ] && [ -z "${SUBS_REGISTRY_ENDPOINT:-}" ]; then
        export SUBS_REGISTRY_ENDPOINT="http://127.0.0.1:${REGISTRY_PORT}"
    fi
fi

exec "$BIN" "$@"
