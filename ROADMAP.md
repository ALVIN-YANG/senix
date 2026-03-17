# Senix Gateway Roadmap

This document outlines the strategic plan and upcoming milestones for Senix Gateway.

## Unreleased
- Improve observability: add OpenTelemetry tracing, Prometheus metrics, and Grafana dashboards.
- Stabilize release process: finalize CI/CD, automate changelog generation, and add release notes.
- Documentation blitz: expand API docs (OpenAPI/Swagger) and migrate to a wiki.
- Security hardening: add static analysis, SBOM, and vulnerability scans to CI.
- Improve developer experience: add developer docker-compose for local testing, and improve README with quick-start examples.

## v1.x (2026)
- GA release with polished UX, robust WAF integration, and enterprise-ready features.
- Docker Compose + Helm charts for Kubernetes deployment; provide sample deployments for minikube/kind.
- End-to-end tests across backend (Go) and frontend (React) with Cypress/Playwright.
- Observability stack: Prometheus, Grafana, Loki, OpenTelemetry.

## v2.x (2027+)
- Multi-cloud deployment support, enhanced RBAC, more provider integrations for ACME DNS.
- Advanced routing policies and blue/green deployments.
- Community and marketplace for plug-ins.

Notes: Roadmap items are subject to change based on user feedback and project priorities.
