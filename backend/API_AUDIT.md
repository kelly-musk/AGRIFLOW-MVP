# AgriFlow API — Test & Standards Audit

**Date:** 2026-09-23
**Scope:** Full functional pass against all 14 endpoints, plus an
adversarial/standards pass (SQL injection, XSS-shaped input, malformed auth,
oversized payloads, a 500-request ID-collision stress test, Content-Type
handling, CORS preflight).
**Method:** Tested locally against a fresh migrated database; also
spot-checked against the live Railway deployment.

---

## Result summary

**Functional: pass.** Auth, role gating, listing ownership, stock
validation, and — most importantly — the transaction state machine (illegal
transitions, wrong-actor transitions, terminal-state lockout, unknown status
strings) all behave exactly as coded, with correct HTTP status codes and
accurate error messages. SQL injection is **not exploitable** — parameterized
queries throughout; a `'; DROP TABLE users;--'` payload was stored as inert
text and the `users` table was confirmed intact afterward. Password hashing
is Argon2, correctly implemented.

The rest of this document is everything that is **not** standard and should
be treated as a punch list.

---

## Required changes (blocking)

### 1. Four endpoints require no authentication at all
`GET /listings`, `GET /listings/{id}`, `GET /demands`, `GET /demands/{id}`.
Flagged as undesirable for this product. Fix: add the `auth: AuthUser`
extractor to all four handlers in `src/routes/listings.rs` /
`src/routes/demands.rs` (same pattern every other handler already uses),
decide the intended role scope, and update the API surface table in
`README.md` once changed.

### 2. Decimal fields serialize as JSON strings, not numbers
`quantity`, `pricePerUnit`, `totalAmount`, and `indicativeBudget` all come
back as `"20"` instead of `20` (a `rust_decimal::Decimal` default). The
frontend's `src/types/index.ts` declares these fields as `number` — wired
up as-is, arithmetic on them will silently string-concatenate or produce
`NaN`. Fix: serialize `Decimal` as a JSON number (e.g. via
`rust_decimal::serde::float`), or explicitly document that every consumer
must `parseFloat()` on receipt.

### 3. The trade lifecycle dead-ends at `PAYMENT_PENDING`
Confirmed structurally, not just by the "not built yet" note in
`README.md`: `PAYMENT_CONFIRMED` only accepts the `system` actor in
`src/state_machine.rs`, and no endpoint can authenticate as `system`. No
client — not even admin — can move a transaction past payment-pending
today. Nothing downstream (logistics, delivery, completion) is reachable
until a payment/webhook endpoint exists.

### 4. Entity ID generation has an unhandled collision window
`src/ids.rs::generate()` draws a random 5-digit suffix (`10_000..99_999`,
~90,000 values) with no collision check or retry, and that value is the
literal `TEXT PRIMARY KEY` for users, listings, demands, and transactions.
By the birthday paradox, ~375 inserts of one entity type give roughly 50%
odds of a collision, which would surface as a raw, unhandled `500 A
database error occurred.` A 500-request rapid listing-creation stress test
came back clean (zero collisions) — that's luck, not a guarantee, and isn't
evidence the risk is safe to ignore. Fix: switch to UUIDs, or add a
uniqueness check + retry loop around ID generation.

### 5. Input validation is inconsistent between near-identical fields
- `pricePerUnit` on listings rejects negative values; `indicativeBudget` on
  demands does **not** — `-999999` was accepted with `200 OK`.
- Empty strings are accepted for `commodity`, `unit`, `qualityGrade`, and
  `location` on both listings and demands (a listing with `commodity: ""`
  was created successfully).
- Email format is not validated on register — `"not-an-email"` was accepted
  as a valid email.

Fix: apply the same validation consistently across listing and demand
creation, and add basic email-format and non-empty-string checks.

### 6. Error response shape is inconsistent — ✅ FIXED (2026-09-24)
Hand-written `AppError` responses returned `{"error": "..."}` with correct
status codes, but Axum's built-in request-rejection paths bypassed
`AppError` and returned **plain text** instead: malformed JSON body (`400`),
invalid enum variant (`422`), missing/wrong `Content-Type` (`415`).

Fixed with a new `AppJson<T>` extractor (`src/json_extractor.rs`) that
wraps `axum::Json<T>` and converts any `JsonRejection` into `AppError`
before it reaches the client — preserving the original status code
(`rejection.status()`) and message (`rejection.body_text()`), just now
wrapped in the same `{"error": "..."}` shape as every other response.
Every handler's request-body extractor (`Json<Body>` → `AppJson<Body>`)
was updated across `auth.rs`, `listings.rs`, `demands.rs`, and
`transactions.rs` — 8 usages in total. Response-side `Json<T>` (return
types) is untouched; only the request-extraction path changed.

Verified: malformed JSON, an invalid `role`/`status` enum value, and a
missing/wrong `Content-Type` header all now return `{"error": "..."}`
with their original status code unchanged. Valid requests and existing
hand-written errors (e.g. duplicate email → `409`) are unaffected.
`cargo test` passes.

---

## Recommended (not blocking, but standard practice before production)

| # | Finding | Why it matters |
|---|---|---|
| 1 | `CorsLayer::permissive()` allows any origin/method/header | Fine for local dev; must be locked to the real frontend origin(s) before any public deployment |
| 2 | No rate limiting on `/auth/login` or `/auth/register` | Unbounded brute-force / credential-stuffing surface |
| 3 | Password minimum is 6 characters | Below common baseline (8+); consider raising and/or adding complexity or breach-list checks |
| 4 | No `/health` endpoint | Most PaaS/orchestration platforms (including Railway) expect one for readiness checks |
| 5 | No pagination on `/listings`, `/demands`, `/transactions` | Fine at current scale; will need `limit`/`offset` or cursor pagination before real usage |
| 6 | No max length enforced on text fields | A 10,000-character `commodity` string was accepted without complaint |
| 7 | JWT is stateless with no revocation | Normal for JWT, but means there's no real "logout" — a leaked token stays valid until it expires (24h) |
| 8 | No API version prefix (`/api/v1/...`) | Not urgent pre-launch, but retrofitting versioning after clients exist is painful |

---

## Confirmed solid (no action needed)

Argon2 password hashing, parameterized queries (no SQL injection surface
anywhere tested), the transaction state machine's transition/actor
enforcement (the most-tested and most-correct part of the system), CORS
preflight handling, and consistent camelCase field naming matching the
frontend's type definitions apart from item 2 above.
