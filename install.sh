#!/bin/bash

# Senix Gateway Install Script
# This script will install Senix Gateway and its dependencies.

set -e

# Define colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${BLUE}==============================================${NC}"
echo -e "${GREEN}      Senix Gateway Installation Script       ${NC}"
echo -e "${BLUE}==============================================${NC}"

# 1. Check root
if [ "$EUID" -ne 0 ]; then
  echo -e "${RED}Please run as root (use sudo).${NC}"
  exit 1
fi

# 2. Install dependencies
echo -e "${BLUE}[1/5] Installing dependencies (nginx, certbot, etc.)...${NC}"
apt-get update -y
apt-get install -y curl wget gnupg2 ca-certificates lsb-release ubuntu-keyring git nginx certbot python3-certbot-nginx

# 3. Setup directories
echo -e "${BLUE}[2/5] Creating Senix directories...${NC}"
mkdir -p /opt/senix/bin
mkdir -p /opt/senix/configs/templates
mkdir -p /opt/senix/data/db
mkdir -p /opt/senix/data/certs
mkdir -p /opt/senix/data/configs
mkdir -p /opt/senix/logs
mkdir -p /opt/senix/web/dist
mkdir -p /etc/nginx/conf.d/senix

# 4. Clone or download latest release (Simulated download for now as it's a private setup, assuming files exist or git clone)
echo -e "${BLUE}[3/5] Deploying Senix files...${NC}"
# In a real open source scenario, this would download pre-built binaries and front-end dists.
# Example: curl -L https://github.com/ALVIN-YANG/senix/releases/latest/download/senix-linux-amd64.tar.gz | tar -xz -C /opt/senix
echo -e "${GREEN}Assuming binaries are already compiled and placed in /opt/senix.${NC}"

# 5. Configure Nginx
echo -e "${BLUE}[4/5] Configuring Nginx...${NC}"
if ! grep -q 'include /etc/nginx/conf.d/senix/\*.conf;' /etc/nginx/nginx.conf; then
  sed -i '/include \/etc\/nginx\/conf\.d\/\*\.conf;/a \ \ \ \ include /etc/nginx/conf.d/senix/*.conf;' /etc/nginx/nginx.conf
fi

# 6. Setup Systemd Service
echo -e "${BLUE}[5/5] Creating Systemd service...${NC}"
cat > /etc/systemd/system/senix.service << 'EOF'
[Unit]
Description=Senix Control Plane
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/opt/senix
ExecStart=/opt/senix/bin/senix -config /opt/senix/configs/config.yaml
Restart=always

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable senix --now
systemctl restart nginx

echo -e "${GREEN}==============================================${NC}"
echo -e "${GREEN} Installation Completed Successfully!         ${NC}"
echo -e "${GREEN}==============================================${NC}"
echo -e "Senix Control Plane is running on port 8080."
echo -e "Access it via: http://<your-server-ip>:8080 or map it with a domain."
echo -e "Default credentials:"
echo -e "Username: admin"
echo -e "Password: admin123"
echo -e "${BLUE}==============================================${NC}"
