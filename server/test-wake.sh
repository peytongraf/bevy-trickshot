#!/usr/bin/env bash
# Checks that the prod server sleeps and wakes the way the client expects
# (`shared/src/wake.rs`, `server/fly.toml`):
#
#   1. stops the fly machine (kicks anyone playing — don't run it mid-game),
#   2. knocks on the game port over TCP exactly like the client does on launch
#      (`cargo run -p shared --example wake_probe`),
#   3. checks fly started the machine for it and the server answered.
#
# With --idle it then waits for the server to shut itself down after
# IDLE_SHUTDOWN_MINS with nobody connected, and checks the machine stays
# stopped instead of being restarted.
#
# Needs flyctl (logged in) and jq. Run from anywhere:
#   server/test-wake.sh            # stop → wake
#   server/test-wake.sh --idle     # stop → wake → idle shutdown (~10 min)
set -euo pipefail

APP="${APP:-bevy-trickshot-server}"
ADDR="${ADDR:-$APP.fly.dev:5000}"
cd "$(dirname "$0")/.."

state() { fly machine list -a "$APP" --json | jq -r --arg id "$1" '.[] | select(.id == $id) | .state'; }

wait_for_state() { # id, wanted state, timeout secs
    local deadline=$((SECONDS + $3))
    while [ "$(state "$1")" != "$2" ]; do
        if [ $SECONDS -ge $deadline ]; then
            echo "FAIL: machine never reached '$2' (it's '$(state "$1")')"
            exit 1
        fi
        sleep 5
    done
}

echo "== building the probe"
cargo build -q -p shared --example wake_probe

MACHINE="$(fly machine list -a "$APP" --json | jq -r '.[0].id')"
echo "== machine $MACHINE is $(state "$MACHINE")"

echo "== stopping it"
fly machine stop "$MACHINE" -a "$APP" >/dev/null
wait_for_state "$MACHINE" stopped 60
echo "   stopped"

echo "== knocking on $ADDR, like the client does"
if ! cargo run -q -p shared --example wake_probe -- "$ADDR"; then
    echo "FAIL: the server never answered — fly didn't start the machine for the TCP knock"
    exit 1
fi
echo "   machine is now $(state "$MACHINE")"
[ "$(state "$MACHINE")" = started ] || { echo "FAIL: machine isn't started"; exit 1; }
echo "PASS: a TCP knock wakes a stopped server"

if [ "${1:-}" = "--idle" ]; then
    MINS="$(sed -n 's/^ *IDLE_SHUTDOWN_MINS *= *"\([0-9.]*\)".*/\1/p' server/fly.toml)"
    echo "== waiting for the idle shutdown (IDLE_SHUTDOWN_MINS=$MINS) — don't connect a client"
    wait_for_state "$MACHINE" stopped $(( ${MINS%.*} * 60 + 180 ))
    echo "   stopped; checking it stays that way"
    sleep 30
    [ "$(state "$MACHINE")" = stopped ] || { echo "FAIL: fly restarted it after the clean exit"; exit 1; }
    echo "PASS: the idle server stopped itself and stayed stopped"
fi
