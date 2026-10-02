# `infrastructure/monitoring` — observability stack

Declarative Prometheus, Grafana, Alertmanager configuration. Dashboards as code.

```text
infrastructure/monitoring/
├── prometheus/
│   ├── scrape.yml            scrape jobs per deployment
│   ├── rules/
│   │   ├── availability.yml   SLOs, error budget burn
│   │   ├── search.yml         latency, result count, cache hit rate
│   │   ├── crawler.yml        fetch rate, error classes, politeness denials
│   │   ├── index.yml          doc count, commit latency, segment growth, compaction
│   │   ├── queue.yml          frontier depth, oldest item age, claim latency
│   │   └── capacity.yml       disk, memory, CPU, connection pools, object store
├── grafana/dashboards/        JSON dashboards (search, crawler, index, system, privacy)
├── alertmanager/              routing, silences, escalation policy
└── log-pipeline.yml           redaction rules applied before logs are persisted
```

## Cardinality is a privacy control, not just a cost control

Every metric and log rule is reviewed for cardinality *and* for privacy. A high-cardinality
label sourced from user input is a privacy leak with a very short time constant.

Banned label names (CI greps the config and the source for these):

```text
query, q, term, terms, url, doc_id, document_id, path (when user-controlled),
ip, client_ip, user_agent, request_uri (query string), email
```

The permitted set is documented in
[docs/operations/observability.md](../../docs/operations/observability.md) with the reason
each metric exists.

## Log pipeline

Redaction happens **before** persistence, in the pipeline, not only in the application:

1. Application emits shape-only logs by construction.
2. Pipeline drops any field matching the banned list above.
3. Pipeline rewrites the access-log path to remove anything after `?`.
4. Retention: 14 days hot, then dropped (no long-term log archive — there is nothing in
   them worth archiving that is not in Postgres).

## Alerting philosophy

Alert on things a human must act on: SLO burn rate, crawl stall, index staleness, disk
pressure, security denials spiking, takedown backlog. Do not alert on every anomaly — an
alert nobody acts on is a way of hiding alerts that matter.

Every alert links to a runbook in [docs/operations/runbooks.md](../../docs/operations/runbooks.md).
An alert without a runbook does not ship.