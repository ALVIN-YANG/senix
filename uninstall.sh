#!/bin/bash

# Senix Gateway Uninstall Script

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo -e "${RED}==============================================${NC}"
echo -e "${RED}      Senix Gateway Uninstall Script          ${NC}"
echo -e "${RED}==============================================${NC}"

if [ "$EUID" -ne 0 ]; then
  echo -e "${RED}Please run as root (use sudo).${NC}"
  exit 1
fi

echo -n "Are you sure you want to completely remove Senix Gateway? (y/N) "
read -r response
if [[ ! "$response" =~ ^[Yy]$ ]]; then
  echo "Aborted."
  exit 0
fi

# 1. Stop services
echo -e "[1/4] Stopping and disabling senix service..."
systemctl stop senix || true
systemctl disable senix || true

# 2. Remove Systemd service
echo -e "[2/4] Removing systemd service..."
rm -f /etc/systemd/system/senix.service
systemctl daemon-reload

# 3. Remove Nginx includes
echo -e "[3/4] Cleaning up Nginx configurations..."
sed -i '/include \/etc\/nginx\/conf\.d\/senix\/\*\.conf;/d' /etc/nginx/nginx.conf || true
rm -rf /etc/nginx/conf.d/senix* || true
systemctl reload nginx || true

# 4. Remove installation files (Optionally keep database/certs)
echo -n "Do you want to delete all data (configs, certs, and database)? (y/N) "
read -r data_response
if [[ "$data_response" =~ ^[Yy]$ ]]; then
  echo -e "[4/4] Removing all files including data..."
  rm -rf /opt/senix
else
  echo -e "[4/4] Removing binaries and source, keeping data/..."
  rm -rf /opt/senix/bin
  rm -rf /opt/senix/web
  rm -rf /opt/senix/internal
  rm -rf /opt/senix/cmd
fi

echo -e "${GREEN}==============================================${NC}"
echo -e "${GREEN} Uninstallation Completed Successfully!       ${NC}"
echo -e "${GREEN}==============================================${NC}"
