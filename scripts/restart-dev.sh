#!/usr/bin/env bash
# Rebuild and restart the dev server on 127.0.0.1:5000, then verify the new build is the one answering.
# Static assets are compiled into the binary, so a restart is required after any frontend change.
set -euo pipefail

cd "$(dirname "$0")/.."
PORT=5000
BIN=./target/debug/ssm-remote-session-gateway
LOG=/tmp/ssmgw.log

# 1. Stop whatever owns the port. Matching by process name missed the old server before, so use the port owner.
owner_pids() { ss -ltnpH "sport = :${PORT}" 2>/dev/null | grep -o 'pid=[0-9]*' | cut -d= -f2 | sort -u; }
for pid in $(owner_pids); do kill "$pid" 2>/dev/null || true; done
for _ in $(seq 1 20); do [ -z "$(owner_pids)" ] && break; sleep 0.5; done
if [ -n "$(owner_pids)" ]; then
  echo "FAILED: port ${PORT} is still held by pid(s) $(owner_pids)" >&2
  exit 1
fi

# 2. Build so the embedded assets match the sources on disk.
cargo build --quiet

# 3. Start and wait for the listener.
nohup "$BIN" > "$LOG" 2>&1 &
NEW_PID=$!
for _ in $(seq 1 20); do
  if [ "$(owner_pids)" = "$NEW_PID" ]; then break; fi
  sleep 0.5
done

# 4. Verify: the port must belong to the new process, and the served session module must match disk.
if [ "$(owner_pids)" != "$NEW_PID" ]; then
  echo "FAILED: port ${PORT} is owned by $(owner_pids), not the new process ${NEW_PID}. Log:" >&2
  tail -n 5 "$LOG" >&2
  exit 1
fi
SERVED=$(curl -s "http://127.0.0.1:${PORT}/static/js/modules/session.js" | wc -c)
DISK=$(wc -c < src/static/js/modules/session.js)
if [ "$SERVED" != "$DISK" ]; then
  echo "FAILED: served session.js is ${SERVED} bytes, disk is ${DISK} bytes" >&2
  exit 1
fi
echo "OK: pid ${NEW_PID} serves the current build on port ${PORT} (session.js ${DISK} bytes)"
