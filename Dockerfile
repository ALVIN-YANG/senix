FROM node:18-alpine AS frontend-builder
WORKDIR /app
COPY web/package*.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM golang:1.22-alpine AS backend-builder
WORKDIR /app
# Install gcc/musl-dev for CGO (SQLite needs CGO)
RUN apk add --no-cache gcc musl-dev
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=1 GOOS=linux go build -ldflags "-s -w -X main.Version=1.0.0" -o senix cmd/senix/main.go

FROM nginx:alpine
# Install necessary tools
RUN apk add --no-cache bash certbot wget curl tzdata ca-certificates

# Setup directories
RUN mkdir -p /opt/senix/bin \
    /opt/senix/configs/templates \
    /opt/senix/data/db \
    /opt/senix/data/certs \
    /opt/senix/data/configs \
    /opt/senix/logs \
    /opt/senix/web/dist \
    /etc/nginx/conf.d/senix

# Configure Nginx main conf to include senix configs
RUN sed -i '/include \/etc\/nginx\/conf\.d\/\*\.conf;/a \ \ \ \ include /etc/nginx/conf.d/senix/*.conf;' /etc/nginx/nginx.conf

# Copy frontend
COPY --from=frontend-builder /app/dist /opt/senix/web/dist

# Copy backend
COPY --from=backend-builder /app/senix /opt/senix/bin/senix
COPY configs/config.yaml /opt/senix/configs/config.yaml
COPY senix.conf /etc/nginx/conf.d/senix.conf

# Copy entrypoint script
COPY docker-entrypoint.sh /opt/senix/docker-entrypoint.sh
RUN chmod +x /opt/senix/docker-entrypoint.sh

WORKDIR /opt/senix

EXPOSE 80 443 8080

ENTRYPOINT ["/opt/senix/docker-entrypoint.sh"]
