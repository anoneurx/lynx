# `tests/e2e` — end-to-end browser tests

Playwright. Full local stack (web + api + seeded index), real browser, real HTTP.

## Critical path (must never break)

```text
load /  → type query  → submit  → results render  → click a result  → paginate
       → apply a filter  → use an operator  → no JS-required fallback page
```

## Also covered

- Keyboard-only operation: tab order, visible focus, submit with <kbd>Enter</kbd>,
  skip link, ARIA roles and labels.
- Theme toggle persists; `prefers-color-scheme` respected; `prefers-reduced-motion` honoured.
- Error states: rate limited, service unavailable, empty result — each renders a real
  message with a `request_id`, not a stack trace.
- Privacy assertions: no cookies set, no third-party requests, no query in `localStorage`.
- CSP violations in the browser console cause a failure.

Runs in CI on every push; runs against staging nightly.