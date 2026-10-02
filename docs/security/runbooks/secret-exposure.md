# RB-07 · Suspected secret exposure

**Severity:** SEV1 · **Owner:** security, with the incident commander

**Rotate first, investigate second.** A leak that is still live while being diagnosed is still
live, and rotation is fast.

## First action

```bash
# 1. contain: stop the secret being used
lynx-secrets rotate --name <secret> --emergency

# 2. for a compromised credential, revoke at the provider as well
#    the rotation above only changes what LYNX uses; it does not un-issue the credential
```

Both steps are needed. Rotating what LYNX uses stops the leak; revoking at the provider ensures
the leaked value is dead everywhere.

## Identify what leaked and how

| Source | How to check | What to assume |
| ------ | ------------ | -------------- |
| Logs | `lynx-audit logs --since 48h --pattern <key-prefix>` | Assume it reached anyone with log access |
| Metrics labels | `lynx-audit metric-labels` | Assume it is public if the metrics endpoint is exposed |
| Traces | `lynx-audit traces` | Assume it left the process |
| Git history | `lynx-audit history` | Assume it was cloned; history rewrite is required |
| A compromised host | Full host inspection | Assume everything on the host was read |
| A public repository | Immediate; assume it was scraped | Revoke within the hour |

The escalating assumption matters: a leak in logs is a narrower problem than a leak in git, and
a leak on a compromised host means every secret on that host is suspect.

## Containment

```text
1  rotate the secret, using the overlap procedure where possible
2  revoke at the provider
3  invalidate every cache entry derived from the rotated secret
     — a cache HMAC secret rotation invalidates all keys automatically
     — a TLS key requires a reload
4  if it reached git: rewrite history, then re-clone or hard-reset everywhere
5  if a host was compromised: rebuild it from the deployment pipeline, do not clean it
6  audit log access over the exposure window
```

Rebuilding rather than cleaning a compromised host is deliberate. A rootkit that survived one
incident survives the remediation of that incident's symptoms.

## Rotation procedure

| Secret | Overlap | Verified by |
| ------ | ------- | ----------- |
| Cache HMAC | New active, old accepted for one TTL | Old keys unreachable; new lookups miss |
| DB password | Second user created, then drop the first | Both connections succeed, then the old user is gone |
| Redis ACL | Two passwords, then remove the first | Same |
| KEK | Re-wrap DEKs under the new version | Every service starts and reads its secrets |
| TLS certificate | Reload on SIGHUP | Certificate served matches the new one |

## Investigation

```text
1  how did it leak?  logs, metrics, traces, history, host, or a mistake in review
2  who accessed it?  audit logs over the window
3  what was it used for?  provider-side logs: what operations happened with the credential
4  what should have caught it?
     — an invariant, a gate, or a review step
5  is this the same vector as a previous incident?
```

Step 4 is what makes the incident productive. A secret that leaked through logs gets an
invariant; one that leaked through code review gets a gate; one that leaked because a host was
compromised gets an isolation control.

## Data exposure assessment

For credentials with access to user data — there are none by design, so this section should
always conclude quickly:

```bash
lynx-verify-privacy
```

| Finding | Assessment |
| ------- | ---------- |
| All checks pass | No user data was accessible to the leaked credential. The exposure is the credential itself |
| A check fails | SEV1 with the privacy lead; the architecture changed and the exposure assessment must be redone |

The design property that matters here: a leaked application credential gives an attacker
capacity, not user data, because there is no user data to take. It can still be used to poison
the index, corrupt availability, or read crawl state — which is why rotation is still urgent.

## Post-incident

```text
1  verify the old credential is dead at the provider
2  verify no secret remains in git history
3  add the missing invariant
4  review every other secret for the same exposure vector
5  check whether backups captured the secret, and rotate if so
```

Step 5 is commonly forgotten. A secret that reached a backup before it leaked is still leaked
after the rotation, because the backup still contains the old value.

## Escalate

- Any user data reachable → SEV1 with the privacy lead, immediately
- Credential used for provider-side operations → assume full credential abuse
- Secret was in git history at any point → treat as public from that moment, regardless of how
  brief the exposure