#!/bin/sh
# The file you ALREADY HAVE: a `docker-compose.yml`, run by kern, with the verbs a deploy script uses.
#
# Every other compose example here uses kern's own TOML. This one starts from Docker's file on
# purpose, because that is what a reader arrives with, and it walks the sequence a CI job or a
# teardown script actually runs:
#
#   kern compose docker-compose.yml -p NAME up -d --wait   start, and BLOCK until the health checks pass
#   kern compose docker-compose.yml ps --format json  the machine-readable channel, Docker's field names
#   kern compose docker-compose.yml exec web ...      a command in a RUNNING service
#   kern compose docker-compose.yml cp web:/p ./p     copy a file out of one
#   kern compose docker-compose.yml down -v           stop it and remove THIS project's named volumes
#
# No daemon, no root, no `docker` on the host. Fully rootless and self-contained: the stack, its
# volume and its temp directory are all removed at the end.
set -eu
kern="${KERN:-kern}"
# WHICH BINARY IS THIS. Printed to stderr on every run, because `${KERN:-kern}` silently resolves to
# whatever `kern` is on PATH: a validation that forgets to set KERN measures the INSTALLED release
# while believing it measured the build under test.
printf '# using %s (%s)\n' "$(command -v "$kern" || echo "$kern")" "$("$kern" --version 2>&1 | head -1)" >&2

work="$(mktemp -d)"
trap 'cd /; "$kern" compose "$work/docker-compose.yml" -p compose-demo down -v >/dev/null 2>&1 || true; rm -rf "$work"' EXIT
cd "$work"

# A file Docker would run unchanged: a health check, a dependency that WAITS for it, a published
# port, a named volume, and the one line nobody guesses - `host.docker.internal` is not a magic name
# in compose, it is an `extra_hosts` entry pointing at `host-gateway`.
cat > docker-compose.yml <<'YML'
services:
  cache:
    image: alpine:latest
    command: ["/bin/sh", "-c", "mkdir -p /data && echo ready > /data/state && sleep 3600"]
    volumes:
      - cachedata:/data
    healthcheck:
      test: ["CMD", "/bin/sh", "-c", "test -f /data/state"]
      interval: 1s
      timeout: 2s
      retries: 10

  web:
    image: nginx:alpine
    depends_on:
      cache:
        condition: service_healthy
    ports:
      - "18923:80"
    extra_hosts:
      - "host.docker.internal:host-gateway"

volumes:
  cachedata:
YML

echo "==> 1. up -d --wait: returns only once every health check has passed"
"$kern" compose docker-compose.yml -p compose-demo up -d --wait

echo
echo "==> 2. ps --format json: Docker's field names, for a script rather than a human"
"$kern" compose docker-compose.yml -p compose-demo ps --format json | head -4

echo
echo "==> 3. exec, into a service that is already running"
"$kern" compose docker-compose.yml -p compose-demo exec -T web -- /bin/sh -c 'echo "  nginx sees hostname: $(hostname)"'

echo
echo "==> 4. host.docker.internal resolves inside the box, because extra_hosts put it there"
"$kern" compose docker-compose.yml -p compose-demo exec -T web -- /bin/sh -c \
    'getent hosts host.docker.internal || grep host.docker.internal /etc/hosts'

echo
echo "==> 5. cp, out of a service"
"$kern" compose docker-compose.yml -p compose-demo cp cache:/data/state ./state-from-the-box
echo "  copied: $(cat ./state-from-the-box)"

echo
echo "==> 6. the published port answers on the host"
if command -v curl >/dev/null 2>&1; then
    curl -sS -m 5 -o /dev/null -w "  http://127.0.0.1:18923 -> %{http_code}\n" http://127.0.0.1:18923/ || \
        echo "  (no answer: another process may hold 18923)"
else
    echo "  (curl not installed, skipping the fetch)"
fi

echo
echo "==> 7. down -v: stops the stack and removes THIS project's named volume"
"$kern" compose docker-compose.yml -p compose-demo down -v

echo
echo "done - the file was Docker's, the runtime was rootless, and nothing is left running."
