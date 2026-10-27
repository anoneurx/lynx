# Runbooks

Every alert links here by anchor (`runbooks.md#<AlertName>`). An alert without a
runbook does not ship. The deep procedures live in
[docs/security/runbooks/](../security/runbooks/README.md); this page is the alert
routing index, so an operator paging in at 3am lands in one paragraph: symptom,
first check, action, escalation.

## API plane (search quality of service)

### LynxAPIHighErrorRate — [search unavailable](../security/runbooks/search-unavailable.md)

Search 5xx rate above 5% for 2 minutes on any surface. Almost always reader reload
failure or memory pressure. First check: `/api/v1/status` degraded flag and the
reader generation vs. the indexer's current generation. Escalate into
[incident response](../security/incident-response.md) when degraded and failing
together with LynxIndexBuildFailed.

### LynxAPIErrorBudgetBurn — [search unavailable](../security/runbooks/search-unavailable.md)

The 30-day error budget burning at more than 14× sustainable pace. The monthly
budget is small by design; a 14×burn-rate hour consumes it in days. Same first
checks as LynxAPIHighErrorRate; treat budget burn as the trend warning, error rate
as the acute symptom.

### LynxAPILatencyP99Search — [latency spike](../security/runbooks/latency-spike.md)

Search p99 above 250 ms for 15 minutes. Check the cache-hit panel first — a cold
cache after generation reload is the single most common cause — then segment count
(merge backlog). Escalate to [index corruption](../security/runbooks/index-corruption.md)
only when the cache and segment panels are healthy and latency still climbs.

### LynxAPICacheMissFlood — [cache failure](../security/runbooks/cache-failure.md)

Cache hit ratio under 30% for 30 min at a stable index generation: generation flips
move the whole cache atomically, so a ratio this low means the cache itself is
failing, not just resized. Failover guidance in the runbook.

### LynxAPIStatusDegraded — [search unavailable](../security/runbooks/search-unavailable.md)

`/status` reports `degraded` continuously for 30 min. A symptom surface: its
specific checks (reader reload lag, generation age) name the panel to open.

### LynxClientDenialSpike — [latency spike](../security/runbooks/latency-spike.md)

Rate-limited responses beside the search surface above 8× the trailing-day median
for 15 min. Either a runaway client is burning the shared prefix buckets
(fingerprinted by daily salt — no IP is ever recorded) or the limit is misconfigured.
The limiter stores nothing durable, so there is nothing to purge.

## Crawler plane

### LynxCrawlerQueueBacklog — [crawler stalled](../security/runbooks/crawler-stalled.md)

Frontier depth above 100,000 for 10 min. Capacity problem: first check whether
politeness budgets or worker count is the limiter, not a stalled worker pool —
the fetch-success panel distinguishes them. Budgets are per host; see
[politeness](../crawler/politeness.md).

### LynxCrawlerFetchStall — [crawler stalled](../security/runbooks/crawler-stalled.md)

Zero successful fetches for 15 min while hosts want work. Ranked causes: DNS
(resolver error class panel), TLS validity, robots-fetch outage, network policy
egress rules. The crawler's own logs carry only class codes — safe to paste in an
incident.

### LynxSSRFBlocksSurge — [SSRF attempt](../security/runbooks/ssrf-attempt.md)

The layered SSRF firewall (RFC1918/link-local, metadata endpoints, port allowlist)
blocked connects above 10× the trailing hour. Either ambient scanning noise in
targets or a crawled page steering the crawler at private space. The fetch is
killed before any byte is parsed; triage is about whether a whole origin must be
de-prioritised. Analysis in [ssrf-protection.md](../security/ssrf-protection.md).

## Index plane

### LynxIndexGenerationStale — [index corruption](../security/runbooks/index-corruption.md)

Current generation age above 36 h. The read path is not stale — the build is.
Check the builder's last error and whether merge time soaked the budget; do not
re-run the builder before reading the corruption checks.

### LynxIndexBuildFailed — [index corruption](../security/runbooks/index-corruption.md)

An index build failed. Do **not** promote the previous generation until corruption
is ruled out (manifest checksums, sampled document reads). If the failing step is a
golden quality gate, the gate's name is the root-cause record — a ranking
regression was caught before it merged.

### LynxIndexGenSizeDrop — [index corruption](../security/runbooks/index-corruption.md)

Generation document count dropped > 2% no operator action. Treat as data loss
until a diff names the cause; the generation manifest diff is the first evidence.

## Capacity

### LynxDiskFreeLowPct — [disk pressure](../security/runbooks/disk-pressure.md)

Postgres WAL or index PVC free space under 15% and falling. Index generations are
bounded (latest five kept), so the fastest lever is usually backup retention, not
deleting a generation the reader may hold open. Never delete a live generation.

### LynxMemoryNearLimit — [disk pressure](../security/runbooks/disk-pressure.md)

Search-plane working set near the node limit, 15 min. Levers in order: drop
snippet length, drop candidate window, then shard movement. Do not thrash readers.

## Data handling

### LynxTakedownQueueAged — [data exposure](../security/runbooks/data-exposure.md)

The takedown backlog held an item older than 7 days. Handling rule in
[legal](../privacy/legal.md); queue age is an SLO. Deletion lands with the *next*
generation and is verified by that generation's checksum audit.

### LynxSecretsAccessAnomaly — [secret exposure](../security/runbooks/secret-exposure.md)

Vault audit hits from service identities outside their usual pattern. Rotate the
touched secrets, then read the audit trail; do not assume the anomaly is benign.

## Escalation

Two critical alerts on the same plane in one incident ⇒ open the full
[incident-response](../security/incident-response.md) flow with the security lead
in the loop. Unilateral mitigation is limited to the documented switches above;
anything else moves state without review.
