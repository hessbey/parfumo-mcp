#!/bin/sh
# Builds the image, then checks what the specification requires of it: under 50 MB,
# non-root user, /healthz answering, under 100 MB of idle memory. Run it on a machine with
# Docker (the VPS, or CI). Sends no request to parfumo.com: the daily canary is off.
set -eu

IMAGE=${IMAGE:-parfumo-mcp-server:check}
NAME=parfumo-mcp-server-check
MAX_IMAGE_MB=50
MAX_RSS_MB=100
cd "$(dirname "$0")/.."

docker build -t "$IMAGE" .

size_bytes=$(docker image inspect "$IMAGE" --format '{{.Size}}')
size_mb=$((size_bytes / 1000000))
echo "image size: ${size_mb} MB (limit ${MAX_IMAGE_MB})"
[ "$size_mb" -lt "$MAX_IMAGE_MB" ] || { echo "FAIL: image too large"; exit 1; }

user=$(docker image inspect "$IMAGE" --format '{{.Config.User}}')
echo "image user: ${user}"
case "$user" in ""|0|0:*|root|root:*) echo "FAIL: the image runs as root"; exit 1;; esac

key=$(docker run --rm "$IMAGE" keygen)
docker rm -f "$NAME" >/dev/null 2>&1 || true
trap 'docker rm -f "$NAME" >/dev/null 2>&1 || true' EXIT
docker run -d --name "$NAME" --read-only --tmpfs /tmp --cap-drop ALL \
  --security-opt no-new-privileges:true \
  -e PARFUMO_MCP_MASTER_KEY="$key" -e PARFUMO_MCP_CANARY_HOUR=off \
  -e PARFUMO_MCP_PUBLIC_URL=http://localhost:8080 "$IMAGE" >/dev/null

i=0
until [ "$(docker inspect "$NAME" --format '{{.State.Health.Status}}')" = "healthy" ]; do
  i=$((i + 1))
  [ "$i" -lt 60 ] || { echo "FAIL: the container is not healthy"; docker logs "$NAME"; exit 1; }
  sleep 1
done
echo "container healthy (/healthz answers)"

docker exec "$NAME" parfumo-mcp-server user add trial >/dev/null
docker exec "$NAME" parfumo-mcp-server user list | grep -q trial
echo "user creation: ok (SQLite database writable on /data, read-only root)"

rss_kb=$(docker exec "$NAME" sh -c "grep VmRSS /proc/1/status" | tr -dc '0-9')
rss_mb=$((rss_kb / 1024))
echo "idle resident memory: ${rss_mb} MB (limit ${MAX_RSS_MB})"
[ "$rss_mb" -lt "$MAX_RSS_MB" ] || { echo "FAIL: too much idle memory"; exit 1; }

echo "image ok"
