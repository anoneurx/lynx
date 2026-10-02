# Contributing — LYNX

LYNX is a real search engine. Changes to it are judged on whether they make the system
faster, safer, more private, more explainable, or more honest — and on whether they would
still be defensible in two years.

Read first: [ARCHITECTURE](./docs/architecture.md) · [PRIVACY](./PRIVACY.md) ·
[SECURITY](./SECURITY.md) · [ROADMAP](./ROADMAP.md) · [ADR process](./docs/adr/README.md)

## Where things live

```text
apps/web          the SERP                    apps/api       public HTTP surface
apps/admin        private ops console         services/*     domain logic (query, ranker,
                                                            crawler, indexer, parser,
                                                            frontier, suggestions, ai)
packages/*        shared libraries            database/*     migrations, schema docs, seeds
infrastructure/*  deploy and observe          research/*     experiments, not production
docs/*            all design documentation    tests/*        cross-crate test suites
```

New code goes in the place that matches its responsibility. Shared code belongs in
`packages/` only when at least two consumers need it *and* you can name its single
responsibility in one sentence. "Common" is not a responsibility.

## Getting started

```bash
git clone https://github.com/anoneurx/lynx && cd lynx
make bootstrap && make env && make up
make migrate && make db-seed && make index-build
make verify        # the full PR gate
```

Full details: [DEVELOPMENT.md](./DEVELOPMENT.md).

## Workflow

1. **Issue or discussion first** for anything architectural, privacy-related, or that
   changes a dependency. A small PR that turns out to need a design change is expensive.
2. **Branch**: `feature/…`, `fix/…`, `docs/…`, `research/…`.
3. **Small PRs, one concern each.** A PR that changes the parser and the ranking weights is
   two reviews wearing a trench coat.
4. **Self-review before requesting review.** Run `make verify`, read your own diff, and fix
   what you would flag.
5. **Fill in the PR template.** Every section is a real review gate.
6. **Never push directly to `main`.**

## Review requirements

| Change area | Approvals | Additional requirement |
| ----------- | --------- | ---------------------- |
| `packages/security` | **2** | one must not be the author; threat-model cross-reference; negative tests |
| `services/crawler/safety.rs`, network code | **2** | SSRF matrix passes; fuzz coverage; written safety rationale |
| Authentication, API keys, admin routes | **2** | authz matrix tests; audit-trail consideration |
| Database migrations | **2** | expand-only in the PR; no lock > 5 s; schema docs and ERD updated |
| Ranking weights or signals | **2** | golden diff reviewed; metric report attached; `ranking.config_version` bumped |
| Anything touching `[privacy]`, log writers, or the API edge | **2** | second reviewer must approve the privacy angle; ADR if the model changes |
| Docs | 1 | links resolve; mermaid renders; claims checkable |
| Everything else | 1 | `make verify` green |

CI is the floor; reviewers are the judgement. A green CI run is not approval.

## Definition of done

- [ ] `make verify` passes.
- [ ] Tests added or updated, including the **negative** case for behaviour changes.
- [ ] Documentation updated where behaviour, configuration, or a threat changed.
- [ ] No new secrets, credentials, or third-party endpoints on the query path.
- [ ] Performance impact stated, measured where relevant.
- [ ] Nothing added that stores user data without a data-inventory row.
- [ ] Nothing added that logs or labels anything derived from query text.
- [ ] Ranking changes ship with a version bump and a reviewable golden diff.

## Code style

Rust — enforced, not optional:

```text
cargo fmt (defaults)
cargo clippy --workspace --all-targets -- -D warnings
workspace lints: unsafe_code = "forbid"; clippy::pedantic; unwrap_used / expect_used /
                 panic / todo / dbg_macro = warn
edition 2021 · MSRV 1.78 · no `unwrap` in library paths
```

TypeScript — enforced:

```text
strict · noUncheckedIndexedAccess · exactOptionalPropertyTypes
ESLint flat config · Prettier · no default exports in modules
```

Documentation: markdownlint, prettier, resolvable internal links, renderable Mermaid, one
canonical location per fact.

## Tests

- Unit tests live next to the code they test. Cross-crate suites live in `tests/`.
- **Security controls need negative tests.** A test that only proves the happy path does
  not test a control.
- **No test may touch the public internet.** Use the fixture origin server.
- **Ranking changes fail on metric regression**, not only on test failure.
- **Privacy invariants are tests**, not conventions — see `tests/security/privacy`.
- Tests must be deterministic. If a test depends on wall-clock time or a random seed, it is
  wrong.

## Dependencies

Adding a dependency is a considered decision. Before opening the PR:

1. Can it be avoided? Can an existing one be extended?
2. Licence compatible with AGPL-3.0? Checked by `cargo deny` and `pnpm audit`.
3. Advisory database clean? Mature releases with a real history?
4. Does it touch the network, the filesystem, or deserialise untrusted input? Then it needs
   hand review and a note in the PR.
5. Runtime telemetry, build scripts that fetch from the network, and incompatible licences
   are disqualifying.

`make supply-scan` must pass. Dependency updates are separate PRs from feature changes.

## Privacy contributions

Privacy changes are reviewed by a second maintainer who did not write them. If a change
alters what LYNX stores, sends, or exposes:

1. Update [docs/privacy/data-inventory.md](./docs/privacy/data-inventory.md).
2. Update [PRIVACY.md](./PRIVACY.md) if a published commitment changes.
3. Write an ADR if the *model* changes, not just the implementation.
4. Add it to `CHANGELOG.md` **before** merge.
5. Update the tests in `tests/security/privacy`.

Never soften a privacy claim to make a feature easier. Say what the system actually does.

## Security contributions

Do not disclose vulnerabilities through public issues — see [SECURITY.md](./SECURITY.md).

## ADRs

Architectural decisions get an ADR before implementation, not after. It should state the
alternatives you rejected and why. If you change a decision, write a new ADR that
supersedes the old one; do not edit history.

## Research

Experiments live in `research/`, never in the production path. Each needs a falsifiable
hypothesis, a method, a result (including negative results), and a statement of what would
make it shippable. No experiment may require logging user queries.

## Commits and PRs

- Commit messages: imperative, subject ≤ 72 characters, body explains **why**.
- No unrelated changes in a commit. No drive-by reformatting.
- No secrets, ever, in any commit — including in a reverted one.
- Draft PRs are welcome for early feedback; mark them clearly.
- Reference issues with `Fixes #123`.

## Reporting problems

Bugs: the issue template. It asks for **query shape**, not query text — we do not need your
search to fix a ranking bug, and we would rather not have it. Security: private channel
per [SECURITY.md](./SECURITY.md).

## Code of conduct

[CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md). Be rigorous, be kind, assume good faith, and
criticise ideas rather than people.

## Licence

Contributions are accepted under [AGPL-3.0-or-later](./LICENSE), with commercial licensing
available from Anoneurx. By contributing you confirm you have the right to license the
work under those terms.