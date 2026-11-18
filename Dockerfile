# nrzMonitor als eigenes Image (ausserhalb von umbrelOS). Programm und DuckDB-Bibliothek werden
# vorher mit scripts/docker-image.sh fuer beide Architekturen gebaut und nach dist/<arch>/ gelegt.
FROM debian:trixie-slim
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata \
 && rm -rf /var/lib/apt/lists/*
COPY dist/${TARGETARCH}/nrzmonitor dist/${TARGETARCH}/libduckdb.so /app/
ENV NRZMON_BIND=0.0.0.0:8571 NRZMON_DATA_DIR=/data
VOLUME /data
EXPOSE 8571
HEALTHCHECK --interval=30s --timeout=5s CMD ["/app/nrzmonitor", "healthcheck"]
ENTRYPOINT ["/app/nrzmonitor"]
