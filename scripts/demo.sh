#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
demo_dir="${script_dir}/../examples/demo"
compose_file="${demo_dir}/compose.yml"

command -v docker >/dev/null 2>&1 || { echo "Docker is required" >&2; exit 1; }
docker compose version >/dev/null 2>&1 || { echo "Docker Compose v2 is required" >&2; exit 1; }
command -v curl >/dev/null 2>&1 || { echo "curl is required" >&2; exit 1; }

compose() {
  docker compose --project-directory "$demo_dir" --file "$compose_file" "$@"
}

compose pull

bootstrap_output=$(compose run --rm --no-deps senix credential bootstrap \
  --db /var/lib/senix/senix.db \
  --label demo-owner 2>&1) || bootstrap_status=$?
bootstrap_status=${bootstrap_status:-0}

if [ "$bootstrap_status" -eq 0 ]; then
  if [ -n "${SENIX_DEMO_PASSWORD:-}" ]; then
    owner_password=$SENIX_DEMO_PASSWORD
  elif command -v openssl >/dev/null 2>&1; then
    owner_password=$(openssl rand -hex 12)
  else
    owner_password=$(LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 24)
  fi
  printf '%s' "$owner_password" | compose run --rm --no-deps -T senix owner bootstrap \
    --db /var/lib/senix/senix.db \
    --username admin \
    --password-stdin >/dev/null
  initialized=true
elif printf '%s' "$bootstrap_output" | grep -q 'already initialized'; then
  initialized=false
else
  printf '%s\n' "$bootstrap_output" >&2
  exit "$bootstrap_status"
fi

compose up --detach

attempt=0
until curl -fsS http://127.0.0.1:9080/api/health >/dev/null; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 30 ]; then
    compose logs senix >&2
    echo "Senix did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

attempt=0
until response=$(curl -fsS -H 'Host: demo.senix.local' http://127.0.0.1:8080/); do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 15 ]; then
    compose logs senix >&2
    echo "Demo backends did not become ready" >&2
    exit 1
  fi
  sleep 1
done

echo
echo "Senix demo is ready."
echo "Admin: http://127.0.0.1:9080/admin/"
echo "Owner: admin"
if [ "$initialized" = true ]; then
  echo "Password: $owner_password"
else
  echo "Password: use the password created by the previous demo run"
fi
echo
echo "Proxy check:"
printf '%s\n' "$response" | sed -n '1,8p'
echo
echo "Stop: docker compose --project-directory examples/demo -f examples/demo/compose.yml down"
echo "Reset all demo data: append --volumes to the stop command"
