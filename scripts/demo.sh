#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
demo_dir="${script_dir}/../examples/demo"
compose_file="${demo_dir}/compose.yml"
proxy_port=${SENIX_DEMO_PROXY_PORT:-8080}
admin_port=${SENIX_DEMO_ADMIN_PORT:-9080}
demo_host=${SENIX_DEMO_HOST:-demo.senix.local}

for port in "$proxy_port" "$admin_port"; do
  case "$port" in
    ''|*[!0-9]*|??????*) echo "Demo ports must be integers from 1 to 65535" >&2; exit 2 ;;
  esac
  if [ "$port" -lt 1 ] || [ "$port" -gt 65535 ]; then
    echo "Demo ports must be integers from 1 to 65535" >&2
    exit 2
  fi
done
proxy_url="http://127.0.0.1:${proxy_port}"
admin_url="http://127.0.0.1:${admin_port}"

command -v docker >/dev/null 2>&1 || { echo "Docker is required" >&2; exit 1; }
docker compose version >/dev/null 2>&1 || { echo "Docker Compose v2 is required" >&2; exit 1; }
command -v curl >/dev/null 2>&1 || { echo "curl is required" >&2; exit 1; }

compose() {
  docker compose --project-directory "$demo_dir" --file "$compose_file" "$@"
}

case "${SENIX_DEMO_PULL:-1}" in
  1) compose pull ;;
  0) ;;
  *) echo "SENIX_DEMO_PULL must be 0 or 1" >&2; exit 2 ;;
esac

# A one-off Compose container inherits the service's static IP. Reuse the running
# container for local bootstrap commands so a repeated demo run cannot collide with it.
running_container=$(compose ps --status running --quiet senix)
senix_command() {
  if [ -n "$running_container" ]; then
    compose exec -T senix /usr/local/bin/senixd "$@"
  else
    compose run --rm --no-deps -T senix "$@"
  fi
}

bootstrap_output=$(senix_command credential bootstrap \
  --db /var/lib/senix/senix.db \
  --label demo-owner 2>&1) || bootstrap_status=$?
bootstrap_status=${bootstrap_status:-0}

if [ "$bootstrap_status" -ne 0 ] && ! printf '%s' "$bootstrap_output" | grep -Eq 'management credential bootstrap has already completed|already initialized'; then
  printf '%s\n' "$bootstrap_output" >&2
  exit "$bootstrap_status"
fi

# Credential creation and Owner account creation are separate durable steps.
# Retry the Owner step even if an earlier attempt already created the credential.
if [ -n "${SENIX_DEMO_PASSWORD:-}" ]; then
  owner_password=$SENIX_DEMO_PASSWORD
elif command -v openssl >/dev/null 2>&1; then
  owner_password=$(openssl rand -hex 12)
else
  owner_password=$(LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 24)
fi
owner_output=$(printf '%s' "$owner_password" | senix_command owner bootstrap \
  --db /var/lib/senix/senix.db \
  --username admin \
  --password-stdin 2>&1) || owner_status=$?
owner_status=${owner_status:-0}

if [ "$owner_status" -eq 0 ]; then
  initialized=true
  echo "Owner account initialized. Save these credentials before startup continues:"
  echo "Owner: admin"
  echo "Password: $owner_password"
elif printf '%s' "$owner_output" | grep -Fq 'owner account bootstrap has already completed'; then
  initialized=false
else
  printf '%s\n' "$owner_output" >&2
  exit "$owner_status"
fi

compose up --detach

attempt=0
until curl -fsS --connect-timeout 1 --max-time 2 "${admin_url}/healthz" >/dev/null; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 30 ]; then
    compose logs senix >&2
    echo "Senix did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

attempt=0
until response=$(curl -fsS --connect-timeout 1 --max-time 2 -H "Host: ${demo_host}" "${proxy_url}/"); do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 15 ]; then
    compose logs senix >&2
    echo "Proxy check failed for Host ${demo_host}; set SENIX_DEMO_HOST if the saved route changed" >&2
    exit 1
  fi
  sleep 1
done

echo
echo "Senix demo is ready."
echo "Admin: ${admin_url}/admin/"
echo "Proxy: ${proxy_url}/ (Host: ${demo_host})"
echo "Owner: admin"
if [ "$initialized" = true ]; then
  echo "Use the Owner password printed before startup."
else
  echo "Password: use the password created by the previous demo run"
fi
echo
echo "Proxy check:"
printf '%s\n' "$response" | sed -n '1,8p'
echo
echo "Stop: docker compose --project-directory examples/demo -f examples/demo/compose.yml down"
echo "Reset all demo data: append --volumes to the stop command"
echo "AI configuration guide: docs/ai-configuration.md (new tools require the current source build)"
