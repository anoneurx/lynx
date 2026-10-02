# Sandboxing and resource limits

The crawler processes content written by strangers, on purpose, thousands of times a day. That
is the most dangerous code path in LYNX by a wide margin, and it is the one that gets the
strongest isolation.

## Why the parser is the highest-risk component

```text
the crawler is the only component that touches untrusted bytes
its job is to interpret attacker-controlled structure (HTML, CSS, JSON-LD, robots.txt, headers)
it runs continuously, against an attacker-chosen corpus
a parser bug is memory corruption at best and remote code execution at worst
```

No other component has that profile. The search path handles our own data; the API handles
requests that are validated before anything is interpreted. So the parser gets isolation that
would be unreasonable elsewhere.

## Process isolation

| Component | Isolation | Why |
| --------- | --------- | -------- |
| `lynx-crawler` | seccomp-bpf, no-new-privs, read-only root, separate network namespace | Untrusted content |
| `lynx-parser` | **Separate process**, seccomp, drop-all capabilities, `RLIMIT_AS` | Memory-safety bugs in HTML parsing are expected, not hypothetical |
| `lynx-indexer` | seccomp, no network access | Integrity-critical writes |
| `lynx-search` | No sandbox, but read-only filesystem and no network egress | Needs performance; input is our own |
| `lynx-api` | No sandbox, no egress except internal services | Trust boundary is the request, not the process |

`lynx-parser` is separated from `lynx-crawler` rather than linked into it. That costs a
serialisation boundary per document, which is measurable, and buys the ability to kill and
restart the parser without restarting the crawl — and to run it with a memory limit that cannot
be escaped by an allocator bug in the fetch path.

The deserialisation format is length-prefixed binary, not JSON, for the same reason: a bounded
parser on a bounded format, with a size check before allocation.

```text
lynx-crawler
   │  length-prefixed frame (max 4 MiB, length checked before allocation)
   ▼
lynx-parser ──► ParsedDoc
   │             text blocks, links, metadata, features
   ▼
back to the crawler, which applies policy and writes to the index
```

Policy decisions — robots compliance, resource accounting, index admission — stay in the
crawler. The parser only interprets. That separation means a parser compromise cannot grant a
document permission to be indexed.

## seccomp and capabilities

```text
lynx-parser:
  capabilities      drop ALL, including CAP_NET_BIND_SERVICE
  seccomp           allow: read, write, exit, futex, clock_gettime, mmap, munmap,
                    brk, rt_sigaction, getrandom, fstat
  seccomp           deny:  execve, execveat, ptrace, socket, connect, openat (absolute),
                    mount, ptrace, bpf, perf_event_open
  no_new_privs      set
  root filesystem   read-only
  writable paths    a private tmpfs only
```

`openat` with an absolute path is denied, so a compromised parser cannot read `/etc/shadow`
even if it somehow obtained a capability. `ptrace` and `bpf` are denied, closing the paths by
which an attacker usually escalates from a memory-safety bug inside a sandbox.

`socket` is denied outright: the parser has no network access at all, so a parser compromise
cannot exfiltrate anything.

## Resource limits

| Limit | Value | Protects against |
| ----- | ----- | ---------------- |
| `RLIMIT_AS` (parser) | 1 GiB | Allocation bombs in malformed HTML |
| `RLIMIT_NOFILE` | 256 | Descriptor exhaustion |
| `RLIMIT_NPROC` | 64 | Fork bombs |
| `RLIMIT_FSIZE` | 64 MiB | Disk exhaustion from a single process |
| `RLIMIT_CPU` | 30 s | Parser livelock |
| `RLIMIT_CORE` | 0 | No core dumps of attacker-shaped memory |
| Response body | 5 MiB | Oversized downloads |
| Redirect chain | 5 | Redirect loops |
| Redirect time | 15 s total | Slow redirects |
| Connect + TLS | 10 s | Slowloris |
| First-byte | 20 s | Slow response |
| HTML nesting depth | 100 | Nested-element bombs |
| CSS rules parsed | 5 000 | Stylesheet bombs |
| Attribute count per element | 1 000 | Attribute bombs |
| Entities expanded | 50 000 | Billion-laughs style expansion |
| Total page time | 60 s | Anything pathological |
| `robots.txt` size | 500 KiB | Oversized robots |
| Response header count | 100 | Header bombs |
| Decompression ratio | 20:1 | Zip bombs |

The decompression-ratio cap deserves emphasis: `Content-Length` is attacker-controlled and
frequently lies, so the limit is on the ratio of bytes produced to bytes consumed, enforced
incrementally during streaming decompression.

## Process lifecycle

```text
1  spawn a parser worker per batch, not per document (amortises fork cost)
2  supervise it: RSS, CPU time, wall time, output frame count
3  on any limit breach: kill immediately, discard the batch, record the URL
4  a worker is replaced after N documents regardless of health (bounds leaks)
5  SIGKILL, never SIGTERM, for a worker that missed its deadline
```

Batch-level spawning means a worker handles hundreds of documents, so a slow-leak or
degradation issue is bounded by replacement count rather than by an incident. Replacing
healthy workers too is what makes a leak a non-event.

## Language-level hardening

| Measure | Detail |
| ------- | ------ |
| Release builds | Overflow checks on in debug, `overflow-checks = true` in release for parse paths |
| `unsafe` policy | Denied in parse code by lint; where genuinely needed, an `// SAFETY:` comment is mandatory and reviewed |
| `cargo deny` | Advisories, bans, and licence policy checked in CI |
| `cargo audit` | Vulnerability database scan in CI |
| Safe wrappers | `String`/`Vec`/`HashMap` only; no `from_raw_parts` without justification |
| Integer casts | Explicit and bounded; `as` conversions reviewed |
| UTF-8 | `str` where possible; invalid sequences replaced, never transmuted |

`unsafe` is denied in parse code by lint rather than by convention. A parser bug is already a
worst-case scenario, so removing a whole class of memory-unsafety bugs from that path is worth
some lost expressiveness.

## Network isolation

```text
lynx-crawler:  egress to the internet, ingress to none, DNS via the resolver
lynx-parser:   no network namespace at all
lynx-indexer:  no network egress
lynx-search:   ingress from the API, egress to Postgres read-only, Redis read-write
lynx-api:      egress to the internal service set only
```

The crawler is the only component with broad egress, which is inherent — it fetches the web.
The mitigation is that the crawler has no access to user data to exfiltrate, and it holds no
long-lived secrets beyond its own service credentials. Separate processes and separate network
policies mean compromising the crawler is not compromising the user plane.

## Resource accounting

```text
per host      bytes, requests, wall time, failures      → politeness budget
per document  size, parse time, node count, depth      → cost model
per crawl     total bytes, total requests               → operational budget
global        disk, memory, open files                  → capacity alarms
```

Cost models matter more than raw limits: a page that costs a hundred times the average is not a
failure, it is a site that should be crawled less often. The politeness budget makes that
adjustment automatic rather than requiring an operator to notice.

## Testing

- **Fuzzing**, continuous: `lynx-parser` against a corpus of real crawled HTML plus a
  structured grammar, with libFuzzer targets for HTML, CSS, JSON-LD, robots.txt, and headers.
  The corpus is seeded from the crawl archive, which is real attacker-shaped input.
- **Limit tests:** a suite of hostile fixtures — billion laughs, a 10 000-level nesting
  bomb, a zip bomb, an infinite-redirect loop, a slow-drip response, a 2 GiB declared
  `Content-Length` with 10 bytes of body. Each must fail closed without a crash.
- **Sandbox tests:** asserting `execve`, `socket`, `ptrace`, and absolute `openat` are all
  blocked in the parser's seccomp filter.
- **Lifecycle tests:** a deliberately leaky parser fixture must be replaced by the supervisor
  without operator intervention.
- **Regression:** every fuzzing finding gets a permanent seed in the corpus. The crash
  returns, and the test fails.