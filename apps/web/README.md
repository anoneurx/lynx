# `apps/web` — LYNX search UI

**Stack:** Vite · React 19 · TypeScript (strict) · Tailwind CSS · self-hosted fonts.
**Rendering:** static prerender for `/`, `/about`, `/privacy`, `/docs`, `/status`;
client-side for `/search`.

## Responsibilities

- Render the search box, results, and result metadata.
- Own client-side filter/pagination state (the server owns result truth).
- Progressive enhancement: the result page must be readable and usable before JS runs,
  and results must be reachable with keyboard only.
- Enforce the local theme preference (system / light / dark) and never fingerprint.

## Privacy constraints (non-negotiable)

- **No cookies.** Ever. No `localStorage` identifiers, no session storage of queries
  beyond the active tab, no fingerprinting, no third-party analytics.
- **No third-party requests of any kind.** Fonts, icons, and JS are served from this
  origin. CSP `default-src 'self'`, `connect-src 'self'`, `font-src 'self'`.
- **Query stays in the URL** for shareability — and that URL is therefore visible to the
  browser history and any user-installed referrer policy. Because of this the SERP page
  is served with `Referrer-Policy: no-referrer` and `Cache-Control: no-store`.
- Search requests carry `Sec-GPC: 1` when the browser sends it. We act on it (skip
  response cache) and we never echo it anywhere that persists.

## Routes

| Route | Rendering | Notes |
| ----- | --------- | ----- |
| `/` | prerendered | Search box, positioning, privacy summary |
| `/search?q=…` | CSR | The SERP. Never prerendered, never cached by CDN |
| `/about` | prerendered | What LYNX is and is not |
| `/privacy` | prerendered | The published privacy commitments |
| `/docs` | prerendered | Operator + developer docs surface |
| `/status` | CSR | Live index/crawl health, low frequency, cached 60 s |
| `/about/operators` | prerendered | Search operator reference (see docs/search/query-language.md) |

## Visual identity

LYNX must not be a recoloured Google. Design intent is documented in
[docs/frontend/design-system.md](../../docs/frontend/design-system.md). Principles:

- Type-first, high legibility, generous line height, serif/display pairing that reads as
  editorial rather than dashboard.
- Motion is opt-in and subtle; `prefers-reduced-motion` is honoured.
- A dense, information-forward result layout that favours scanning over decoration.
- Distinctive accent, not the default bootstrap blue.

## Testing

- Component + hook tests with Vitest + Testing Library.
- Playwright E2E for the search request → rendered result path.
- Lighthouse budget enforced in CI: LCP < 1.5 s, CLS < 0.05, JS < 120 kB gzip on `/`.
- A CSP + third-party-request test asserts the built bundle contains no off-origin URL.