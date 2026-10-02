# Backup & Disaster Recovery

## What gets backed up

| Data | Tool | Schedule | Retention | RPO | RTO |
| ---- | ---- | -------- | --------- | --- | --- |
| Postgres (operational DB) | **WAL-G** | Continuous (WAL push) + daily base backup | 30 days (daily), 1 year (monthly) | < 5 s | < 30 min |
| Tantivy index segments | **rsync + snapshot** | On every successful index generation | 5 generations | 0 (immutable) | < 5 min (reader reload) |
| Redis | **RDB + AOF** | Hourly RDB, fsync everysec | 7 days | < 1 h | < 5 min |
| Secrets (Vault) | **Vault raft snapshot** | Daily | 90 days | 0 | < 15 min |

## Postgres – WAL-G

### Configuration

```bash
# /etc/wal-g.d/env
WALE_S3_PREFIX=s3://lynx-backups/prod/postgres
AWS_REGION=us-east-1
WALG_COMPRESSION_METHOD=zstd
WALG_DELTA_MAX_STEPS=5
WALG_UPLOAD_CONCURRENCY=8
```

### Base backup (daily, 03:00 UTC)

```bash
wal-g backup-push /var/lib/postgresql/data
```

### Point-in-time recovery (PITR)

```bash
# Stop Patroni, restore base backup + WAL
wal-g backup-fetch LATEST /var/lib/postgresql/data
echo "restore_command = 'wal-g wal-fetch %f %p'" > recovery.signal
# Set recovery_target_time in postgresql.auto.conf
patroni start
```

### Verification

```bash
# Weekly: restore to staging, run pg_dump --schema-only, compare checksums
wal-g backup-list --detail
```

## Tantivy index

### Immutable generations

Each index build writes to `/var/lib/lynx/index/gen-<N>/`.
Readers open `gen-<N>/` atomically via symlink swap:

```bash
# Indexer does this on success:
ln -sfn /var/lib/lynx/index/gen-42 /var/lib/lynx/index/current
redis-cli PUBLISH INDEX_RELOAD 42
```

### Backup = snapshot the generation directory

```bash
# Runs after indexer publishes SUCCESS
rsync -a --delete /var/lib/lynx/index/gen-42/ \
  s3://lynx-backups/prod/index/gen-42/
```

### Recovery

```bash
# 1. Fetch desired generation
aws s3 sync s3://lynx-backups/prod/index/gen-42/ /var/lib/lynx/index/gen-42/
# 2. Point symlink
ln -sfn /var/lib/lynx/index/gen-42 /var/lib/lynx/index/current
# 3. Trigger reload
redis-cli PUBLISH INDEX_RELOAD 42
```

No downtime; API pods pick up new reader within ~100 ms.

## Redis

### Backup (cron, hourly)

```bash
redis-cli --rdb /tmp/redis-dump-$(date +%s).rdb
aws s3 cp /tmp/redis-dump-*.rdb s3://lynx-backups/prod/redis/
```

### Restore

```bash
# Stop Redis, replace dump.rdb, restart
systemctl stop redis
aws s3 cp s3://lynx-backups/prod/redis/redis-dump-LATEST.rdb /var/lib/redis/dump.rdb
systemctl start redis
```

Acceptable data loss: cached search results + politeness tokens (rebuilt automatically).

## Secrets (Vault)

### Snapshot (daily, 02:00 UTC)

```bash
vault operator raft snapshot save /tmp/vault-snap-$(date +%s).snap
aws s3 cp /tmp/vault-snap-*.snap s3://lynx-backups/prod/vault/
```

### Restore (DR region)

```bash
# 1. Stand up new Vault cluster (Raft)
# 2. Restore snapshot
vault operator raft snapshot restore /tmp/vault-snap-LATEST.snap
# 3. Unseal, verify ExternalSecrets operator syncs
```

## Disaster scenarios

| Scenario | Detection | Recovery steps | Owner |
| -------- | --------- | -------------- | ----- |
| **Region outage (primary)** | CloudWatch / PagerDuty | 1. Promote DR Postgres read replica → primary<br>2. Point DNS to DR API pods<br>3. Restore index from S3 (cross-region replication)<br>4. Restore Vault snapshot | SRE on-call |
| **Postgres corruption** | `pg_verifybackup` fails, checksum errors | 1. Stop writers<br>2. WAL-G PITR to last good LSN<br>3. Verify, resume | DBA |
| **Bad index generation deployed** | Search quality drop, alerts | 1. `ln -sfn gen-<N-1> current`<br>2. `redis-cli PUBLISH INDEX_RELOAD <N-1>`<br>3. Investigate indexer logs | Search engineer |
| **Secrets compromised** | Vault audit log, external report | 1. Rotate all keys via Vault<br>2. Re-issue TLS certs<br>3. Re-deploy all workloads | Security |

## Runbook links

- [RB-02: Data Exfiltration Attempt](../security/RB-02.md)
- [RB-03: Service Degradation / Outage](../security/RB-03.md)
- [RB-07: Key Compromise](../security/RB-07.md)

## Testing

| Test | Frequency | Success criteria |
| ---- | --------- | ---------------- |
| Postgres PITR to staging | Weekly | `pg_dump --schema-only` matches prod checksum |
| Index generation restore | Per release (CI) | API serves queries from restored gen |
| Vault snapshot restore | Monthly | ExternalSecrets syncs within 5 min |
| Full region failover | Quarterly (game day) | RTO < 30 min, RPO < 5 s |

## Encryption at rest

- **S3:** SSE-KMS (CMK `arn:aws:kms:us-east-1:123456789:key/...`), key rotation yearly.
- **EBS (Postgres):** AWS managed CMK.
- **Vault storage:** Auto-encrypted by Raft.

## Access control

- Backup buckets: `s3:GetObject`/`PutObject` only for `backup-writer` IAM role.
- WAL-G and rsync run as `postgres`/`lynx` service accounts, no human access.
- Vault snapshots: `vault operator raft snapshot` requires `root` or `dr-recovery` policy.