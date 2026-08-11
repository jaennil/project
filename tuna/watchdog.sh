#!/bin/sh

set -u

TUNA_BIN="${TUNA_BIN:-/usr/local/bin/tuna}"
TUNNEL_URL="${TUNA_WATCHDOG_URL:-https://project-ai-usage.ru.tuna.am/api/v1/status/buildinfo}"
CHECK_INTERVAL="${TUNA_WATCHDOG_INTERVAL:-60}"
CHECK_TIMEOUT="${TUNA_WATCHDOG_TIMEOUT:-10}"
FAILURE_THRESHOLD="${TUNA_WATCHDOG_FAILURE_THRESHOLD:-3}"
STARTUP_GRACE="${TUNA_WATCHDOG_STARTUP_GRACE:-20}"
RESTART_DELAY="${TUNA_WATCHDOG_RESTART_DELAY:-5}"

tuna_pid=""

stop_tuna() {
    if [ -n "$tuna_pid" ] && kill -0 "$tuna_pid" 2>/dev/null; then
        kill -TERM "$tuna_pid" 2>/dev/null || true
        wait "$tuna_pid" 2>/dev/null || true
    fi
}

shutdown() {
    trap - INT TERM
    stop_tuna
    exit 0
}

check_tunnel() {
    probe_output="$(
        wget -S -O /dev/null -T "$CHECK_TIMEOUT" "$TUNNEL_URL" 2>&1 || true
    )"

    case "$probe_output" in
        *" 200 "*|*" 401 "*|*" 403 "*)
            return 0
            ;;
        *" 404 "*)
            return 1
            ;;
        *)
            return 2
            ;;
    esac
}

trap shutdown INT TERM

while true; do
    "$TUNA_BIN" "$@" &
    tuna_pid=$!
    failures=0

    printf '%s\n' "tuna watchdog: monitoring $TUNNEL_URL"
    sleep "$STARTUP_GRACE"

    while kill -0 "$tuna_pid" 2>/dev/null; do
        check_tunnel
        check_result=$?
        case "$check_result" in
            0)
                if [ "$failures" -gt 0 ]; then
                    printf '%s\n' "tuna watchdog: tunnel recovered"
                fi
                failures=0
                ;;
            1)
                failures=$((failures + 1))
                printf '%s\n' \
                    "tuna watchdog: tunnel missing ($failures/$FAILURE_THRESHOLD)"

                if [ "$failures" -ge "$FAILURE_THRESHOLD" ]; then
                    printf '%s\n' "tuna watchdog: restarting Tuna"
                    stop_tuna
                    break
                fi
                ;;
            *)
                failures=0
                printf '%s\n' "tuna watchdog: check inconclusive"
                ;;
        esac

        sleep "$CHECK_INTERVAL"
    done

    wait "$tuna_pid" 2>/dev/null || true
    tuna_pid=""
    printf '%s\n' "tuna watchdog: retrying in ${RESTART_DELAY}s"
    sleep "$RESTART_DELAY"
done
