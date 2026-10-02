# Code of Conduct — LYNX

## Our commitment

LYNX exists because we think search should not be an instrument of surveillance. That
standard applies to how we work together. We want rigorous technical disagreement,
candour about limitations, and basic respect for the people in the room.

## Expected behaviour

- **Critique the work, not the person.** "This allocation strategy will thrash" is useful.
  "You clearly did not think about this" is not.
- **Assume good faith.** Most disagreements are about information or priorities, not
  motives.
- **Be precise about uncertainty.** Say what you know, what you believe, and what you would
  need to find out. "I don't know, let me check" is a complete and respectable answer.
- **Be honest about limits.** Overstating what a system does is a correctness bug with a
  reputation attached. This is a project about not making unfounded claims.
- **Respect people's time.** Review promptly, say when you cannot, and avoid demanding
  work that is not in scope.
- **Include newcomers.** Someone who asks an obvious question deserves a real answer, not
  a display of expertise.
- **One conversation at a time.** Do not relitigate settled decisions in unrelated threads;
  if a decision is wrong, supersede it with an ADR and move on.

## Unacceptable behaviour

- Harassment, personal attacks, or demeaning comments.
- Sexualised language or imagery.
- Discriminatory or exclusionary behaviour of any kind.
- Publishing others' private information without explicit permission.
- Deliberate intimidation, threats, or sustained disruption.
- Abuse of a vulnerability you disclosed. See [SECURITY.md](./SECURITY.md) for coordinated
  disclosure, which protects researchers who act in good faith.
- Publishing someone's private search history or identifying them as a LYNX user.
- Using project spaces to promote unrelated commercial or political activity.

## Why this matters specifically here

Two failure modes are common in search-engine projects and both are conduct problems:

1. **Privacy theatre** — claiming protections the system does not provide, or quietly
   weakening a guarantee to make a feature work. It is a review failure and a governance
   failure, and this project treats it as both.
2. **Silent scope capture** — a contributor steadily reshaping the project toward their own
   priorities without the roadmap or the maintainers noticing. Big architectural changes get
   ADRs precisely so that this cannot happen quietly.

If you see either, say so.

## Scope

This applies to all project spaces: repositories, issues, pull requests, discussions,
chat, mailing lists, and any space representing the project. It also applies when an
individual is officially representing the project in public.

## Reporting

Report unacceptable behaviour to `conduct@anoneurx.com`. Reports are handled privately and
confidentially by the maintainers. Include what happened, where, when, and who was
involved, plus anything that would help us assess it. You will get an acknowledgement, and
you will be told the outcome.

Retaliation against someone who reports in good faith is itself a violation.

## Enforcement

Maintainers will respond proportionally and privately. Possible actions:

```text
1. A private correction and clarification of what is wrong.
2. A warning, with consequences stated for continued behaviour.
3. A temporary ban from project spaces.
4. A permanent ban from project spaces.
```

Severity, pattern, and impact determine the response. Intent is considered, and so is
impact. A single careless comment and a sustained campaign get different responses.

Anyone asked to stop a behaviour is expected to comply immediately. Maintainers who do not
enforce this policy in good faith may themselves face consequences.

## Attribution

Adapted from the [Contributor Covenant](https://www.contributor-covenant.org/) (v2.1) and
the [Rust Code of Conduct](https://www.rust-lang.org/en/conduct.html), with sections
specific to search-engine privacy work.