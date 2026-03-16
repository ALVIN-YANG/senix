#!/bin/bash
set -e

# Start Nginx in background
nginx

# Start Senix Control Plane in foreground
echo "Starting Senix Control Plane..."
exec /opt/senix/bin/senix -config /opt/senix/configs/config.yaml
