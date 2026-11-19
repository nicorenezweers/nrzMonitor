# nrzMonitor

Überwachung wie Netdata/Grafana – komplett in Rust. Liest jede Sekunde den Host, jeden
Docker-Container und das Netzwerk aus, speichert sparsam in DuckDB und zeigt alles live im nrz-Design.

Aufbau, Schnittstellen und Regeln: [ARCHITECTURE.md](ARCHITECTURE.md).

```bash
cargo xtask dev         # Entwicklung mit Beispieldaten, http://127.0.0.1:8571
```

## MCP-Server und Dashboards

nrzMonitor ist zugleich ein MCP-Server: KI-Programme, Skripte und externe Dashboards fragen alles ab –
Live-Werte, Zeitreihen, Ereignisse, Container, Logs, Alarme, DNS, Strom, Speicher. Adresse und Schlüssel
stehen unter **Einstellungen → MCP-Server & Dashboards**.

```bash
# Claude Code (HTTP)
claude mcp add --transport http nrzmonitor http://<host>:8570/mcp --header "Authorization: Bearer <SCHLUESSEL>"
# ohne HTTP: Brücke im Container (findet Adresse und Schlüssel selbst)
docker exec -i nrz-monitor_app_1 /app/nrzmonitor mcp
# Dashboard: Live-Strom (Server-Sent Events) und REST-API mit demselben Schlüssel
curl -N "http://<host>:8570/mcp/stream?section=system&snaps=docker.containers&events=1&token=<SCHLUESSEL>"
curl -H "Authorization: Bearer <SCHLUESSEL>" http://<host>:8570/mcp/api/v1/snap/docker.containers
```

Werkzeuge: `overview`, `list_sections`, `list_charts`, `get_chart`, `live_values`, `query_timeseries`,
`chart_stats`, `top_instances`, `what_changed`, `export_csv`, `events`, `list_snapshots`, `get_snapshot`,
`docker_containers`, `docker_container`, `docker_logs`, `docker_processes`, `docker_resources`, `alerts`,
`system_info`, `dns`, `integrations`, `power_summary`, `storage_stats`, `watch_live`, `api_request`.
Ressourcen (abonnierbar): `nrzmon://overview`, `nrzmon://chart/{id}`, `nrzmon://context/{context}`,
`nrzmon://section/{section}`, `nrzmon://snap/{key}`, `nrzmon://events/recent`, `nrzmon://history/{id}`.
Andere Container erreichen den Server unter umbrelOS direkt: `http://10.21.0.1:8571/mcp`.
