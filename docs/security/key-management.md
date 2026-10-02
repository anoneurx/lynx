# Key management

Secrets in a system like LYNX are few and structural. The goal is that a compromise of the
application process yields **no** plaintext secret that outlives the process, and that rotation
is routine rather than an incident.

## Secret inventory

| Secret | Purpose | Where it lives | Rotation |
| ------ | ------- | -------------- | -------- |
| TLS certificate key | HTTPS termination | File on disk, 0400, unencrypted | Certbot, 60 d |
| DB password | Postgres | Envelope-encrypted in the environment | 90 d |
| Redis ACL password | Cache | Envelope-encrypted in the environment | 90 d |
| Cache HMAC secret | Cache-key derivation | KMS-wrapped, in memory at runtime | 30 d |
| Rate-limit salt | Bucket keying | Derived per day from the KMS root | Daily, automatic |
| Sealing key (KEK) | Wraps everything above | **KMS only**, never on a host | 180 d, re-wraps only |
| Metrics endpoint token | Prometheus scrape | File on disk | 90 d |
| JWT signing keys | None in v0.1 | No accounts exist | n/a |

Two absences are deliberate: there is no user credential store, and no third-party API key,
because there are no third-party APIs.

## Envelope encryption

```text
seal(plaintext) = nonce || AES-256-GCM(kek_wrap, DEK, plaintext, aad)
                  DEK is random per secret, wrapped by the KEK from the KMS
open(sealed)    = unwrap DEK with the KEK, then decrypt

aad = secret_name || host_id || deployment_id
```

Properties this buys:

| Property | How |
| -------- | --- |
| The KEK never leaves the KMS | It is not on any host, not in any image, not in any backup |
| Per-secret data keys | Compromise of one DEK does not reveal the others |
| AAD binding | A ciphertext from one host cannot be decrypted on another |
| Tamper detection | AES-GCM authentication fails closed |
| Rotation without re-encryption | Rotate the KEK, re-wrap the DEKs; plaintext is untouched |

The AAD binding is the detail worth keeping: without it, a sealed secret could be copied
between hosts, which turns one compromised host into a way to obtain secrets intended for
another.

## Runtime handling

```rust
struct Secret(String);                       // no Debug, no Display

impl Secret {
    fn expose(&self) -> &str { &self.0 }     // explicit, greppable call site
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        // zeroize, do not merely free
    }
}
```

The `Debug` implementation is what stops the most common leak, which is a `#[derive(Debug)]`
struct containing a secret reaching a log line. `expose()` is deliberately verbose and
therefore greppable: every place a secret becomes plaintext is findable with one search, which
is the property that makes an audit possible.

## KMS

```text
provider        Cloud KMS (or a software KMS for self-hosted)
key             lynx/sealing, AES-256-GCM wrap/unwrap
policy          the search and crawler roles may NOT unwrap
                only the deploy role and the instance-init role may unwrap
access          logged; alerts on any unwrap outside a deployment window
versioning      enabled; every KEK version recoverable for 90 d
```

Separating unwrap permission from application roles is the load-bearing control. The search
service has database access, but it cannot obtain the key that decrypts the database password,
because it does not need it — the connection pool holds it in memory from instance
initialisation.

## Rotation

| Secret | Procedure | Downtime |
| ------ | --------- | -------- |
| Cache HMAC | New secret active, old one accepted for one TTL, then dropped | None |
| DB password | Create a second user, migrate, drop the first | None |
| Redis ACL | Two passwords in the ACL, switch, remove the first | None |
| Rate-limit salt | Automatic daily derivation | None |
| KEK | Re-wrap every DEK under the new version | None |
| TLS certificate | Reload on SIGHUP | None |

Every rotation is overlap-based: new material is accepted alongside old material until the
old is provably unused. A rotation procedure with a cutover step is a procedure that gets
deferred, and a secret that gets deferred is a secret that stays.

Cache-key rotation needs no invalidation at all, because the HMAC secret is part of the key:
rotating it makes every key unresolvable, so stale entries simply become unreachable and expire
on their own TTL.

## Logs and errors

```rust
enum SecretHandling {
    Never,                                   // display nothing at all
    Redacted(&'static str),                  // "[REDACTED]"
    PartiallyRedacted { keep_prefix: usize },// "db:7f3a…"  — for correlation
}
```

- Connection strings are `PartiallyRedacted` with a prefix that is stable per host, enough to
  correlate a failing connection to a service without revealing the password.
- Error messages from a driver may embed a connection string; they pass through a scrubber
  before leaving the process.
- CI greps for the real secret patterns in test fixtures; a leaked-looking string fails the
  build.
- Metrics labels never contain secret material. There is a test asserting the label
  cardinality of every metric is bounded, which also prevents unbounded-label cardinality
  attacks.

## Audit

| Check | Frequency | Method |
| ----- | --------- | ------ |
| `Debug` implementations on secret-bearing types | Every CI run | Lint rule |
| Plaintext secret patterns in the repository | Every CI run | Secret scan |
| KMS unwrap events | Continuous | Alert outside deployment windows |
| Secret age versus rotation policy | Weekly | Dashboard |
| `expose()` call-site review | Every release | Grep, human review of the list |
| Backup inspection | Quarterly | Verify no plaintext secret in any backup |

The `expose()` call-site list is reviewed each release because it is the complete inventory of
every point at which a secret is plaintext in the system. It is a short list, and it is the
answer to "where could a secret leak from".

## Failure modes

| Failure | Behaviour |
| ------- | --------- |
| KMS unreachable | Serve from the last-sealed cache with an alert; refuse to start on a cold host |
| Unwrap fails | Refuse to start; no degraded crypto path exists |
| Wrong host's AAD | Authentication failure; refuses to start |
| Clock skew beyond the KMS skew window | KMS rejects; the operator is paged. Skew is monitored, not tolerated |
| A leaked secret | Rotate via the documented overlap procedure; audit logs identify the access window |
| Corrupted sealed value | Fails closed; the host is rebuilt from the deployment pipeline |

"No degraded crypto path exists" is intentional. A system that falls back to a weaker mode when
a key service is unavailable will eventually run permanently in the fallback, and that is how
encryption quietly stops being enforced.

## Related

- [../threat-model.md](../threat-model.md) — where secrets sit in the trust model
- [../operations/secrets-management.md](../operations/secrets-management.md) — the runbook
- [secure-development.md](./secure-development.md) — how secrets enter the repository (they do not)