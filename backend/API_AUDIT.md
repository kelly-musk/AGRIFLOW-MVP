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

### 1. Four endpoints require no authentication at all — ✅ FIXED (2026-09-24)
`GET /listings`, `GET /listings/{id}`, `GET /demands`, `GET /demands/{id}`.
Flagged as undesirable for this product. Fixed by adding the `auth:
AuthUser` extractor to all four handlers — any authenticated role (buyer,
supplier, logistics, admin) can now browse, but an anonymous caller gets
`401 Missing Authorization header.`. No role restriction beyond "must be
logged in," since both buyers and suppliers legitimately need to browse
both listings and demands. Verified: unauthenticated requests to all four
routes now 401; authenticated requests (any role) succeed unchanged.
See the updated API surface table in `README.md`.

A resource-key-based gate (a secret issued by the backend, separate from
user login) was considered and deliberately rejected in favor of this
simpler fix — see discussion history for the reasoning: a statically
embedded key in a public SPA build is trivially extractable from the
browser bundle regardless of expiry, and a dynamically-issued key with no
credential check on issuance doesn't stop scripted abuse either. User-role
gating was judged sufficient for now.

### 2. Decimal fields serialize as JSON strings, not numbers — ✅ FIXED (2026-09-24)
`quantity`, `pricePerUnit`, `totalAmount`, and `indicativeBudget` all came
back as `"20"` instead of `20` (a `rust_decimal::Decimal` default). Fixed
by enabling the `serde-with-float` feature on `rust_decimal` and annotating
every response-facing `Decimal` field (`SupplyListing`, `DemandRequest`,
`Transaction`) with `#[serde(with = "rust_decimal::serde::float")]`. Only
the *response* side was changed — request DTOs (`CreateListingRequest`,
`CreateDemandRequest`, `CreateTransactionRequest`, `UpdateListingRequest`)
were left on the default (flexible) `Decimal` deserializer, since they
already accepted numbers correctly and nothing needed fixing there.
Verified: `GET /listings`, `/demands`, `/transactions` now return unquoted
JSON numbers; `POST /listings` still accepts and round-trips correctly.
The frontend's `apiMappers.ts` coercion (`num()`) is unaffected — it
already tolerates receiving real numbers instead of strings.

### 3. The trade lifecycle dead-ends at `PAYMENT_PENDING` — ✅ FIXED, then partly regressed, now re-fixed (2026-09-24)
Originally: `PAYMENT_CONFIRMED` only accepted the `system` actor, and no
endpoint could authenticate as `system`, so no client could move a
transaction past payment-pending. Fixed by an earlier commit
(`mock_confirm_payment`/`mock_fail_payment`, buyer-scoped endpoints acting
as `Actor::System` internally) — but that fix was undone by a later commit
that loosened `PaymentConfirmed`/`PaymentFailed`/`PaymentCancelled`/
`LogisticsPending` in `state_machine.rs` to allow `Buyer` directly. That
meant **any buyer could self-confirm their own payment** via
`POST /transactions/:id/transition {"to":"PAYMENT_CONFIRMED"}` — a live,
provable exploit (verified: the exact call succeeded with `200` before this
fix), since the backend never checked that money actually moved.

Re-fixed by:
- Restoring `PaymentConfirmed`/`PaymentFailed`/`LogisticsPending` to
  `System`-only and `PaymentCancelled` to `Buyer`-only, with a doc comment
  and two new regression tests (`buyer_cannot_self_confirm_or_fail_payment`,
  `buyer_cannot_self_drive_logistics_pending`) so this can't silently
  regress a third time.
- Adding a real `payments` table (migration `0002_payments.sql`) — payments
  were never persisted anywhere before this, only inferred from transaction
  status.
- A proper three-endpoint payment flow: `POST .../payment/initiate` (buyer,
  idempotent, creates a `PENDING` row), `POST .../payment/confirm` (buyer,
  settles it — amount/currency come only from the row `initiate` created,
  never from the confirm request body, so a buyer can't "confirm" a lower
  amount than they owed), `POST .../payment/fail`, `GET .../payment`.
- A `stellar_tx_hash` column, so the Soroban escrow flow's on-chain
  transaction hash has somewhere durable to live instead of only existing
  in React state until the next page refresh.

Verified: the self-confirm exploit now returns `409 A buyer cannot perform
this action`; the full initiate → confirm (with a Stellar hash) → read
flow persists correctly; a second buyer/supplier cannot touch another
buyer's payment (`403`); confirming an already-settled payment twice is a
clean idempotent `200`, not a `409` (a pre-existing bug surfaced by this
work, fixed alongside it). `cargo test` passes (10 tests, 4 new).

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

### 5. Input validation is inconsistent between near-identical fields — ✅ FIXED (2026-09-24)
- `pricePerUnit` on listings rejected negative values; `indicativeBudget` on
  demands did **not** — `-999999` was accepted with `200 OK`.
- Empty strings were accepted for `commodity`, `unit`, `qualityGrade`, and
  `location`/`destinationLocation` on both listings and demands (a listing
  with `commodity: ""` was created successfully).
- Email format was not validated on register — `"not-an-email"` was
  accepted as a valid email.

Fixed with a new shared `src/validation.rs` module (`require_non_empty`,
`is_valid_email`) instead of each handler inventing its own version of the
check:
- `demands::create` now rejects a negative `indicativeBudget`, matching
  `listings::create`'s existing `pricePerUnit` check.
- Both `listings::create` and `demands::create` now reject blank
  `commodity`, `unit`, `qualityGrade`, and `location`/`destinationLocation`.
- `auth::register` now rejects an email without an `@`, a non-empty local
  part, and a domain containing a `.` (deliberately permissive — not full
  RFC 5322 validation, just enough to catch obviously-invalid input).

Verified: all four previously-accepted invalid inputs now return `400`
with a clear message; valid listings, demands, and registrations are
unaffected. Two new unit tests for `is_valid_email` plus the existing
suite all pass (`cargo test`, 8 tests).

### 6. Error response shape is inconsistent
Hand-written `AppError` responses return `{"error": "..."}` with correct
status codes. Axum's built-in request-rejection paths do not go through
`AppError` and return **plain text** instead of JSON:
- Malformed JSON body → `400`, plain text
- Invalid enum variant (e.g. bad `role` or `status` value) → `422`, plain
  text
- Missing/wrong `Content-Type` header → `415`, plain text ("Expected
  request with `Content-Type: application/json`")

Any frontend expecting a uniform `{error: string}` envelope will mishandle
these specific cases. Fix: add a custom JSON-rejection handler (or a
`FromRequest` wrapper around `Json<T>`) so every error path returns the
same shape.

### 7. Bachs.io (Naira) payments had no real settlement verification — ✅ FIXED (2026-09-24)
Found during a follow-up review of the payments work above, not part of
the original audit pass. The frontend called Bachs.io's checkout-session
API **directly from the browser**, using a hardcoded API secret key and
webhook signing secret in `src/lib/bachs.ts` — both committed in a public
repo. Separately from that exposure: even with valid credentials, the
return flow (`?payment=success` on the checkout redirect) was **unverified
self-attestation**, the same class of bug as item 3's original exploit,
just relocated. A buyer could skip Bachs's checkout page entirely and
either hit the redirect URL directly or call
`POST /transactions/:id/payment/confirm` themselves — nothing checked with
Bachs that Naira actually moved. Confirmed this concretely: the mock
confirm endpoint has no path-of-truth to Bachs at all.

Fixed with a proper server-side integration:
- **Checkout session creation moved server-side**
  (`POST /transactions/:id/payment/bachs/checkout-session`, buyer-scoped)
  — the secret key (`BACHS_SECRET_KEY`) now lives only in backend env vars
  and never reaches the browser. Verified against the **real Bachs sandbox
  API**, not a mock: returned a genuine `checkout_id`/`checkout_url` pair.
- **A real webhook listener** (`POST /webhooks/bachs`, public, no JWT —
  Bachs has no user session to present) verifies the HMAC-SHA256 signature
  over the raw request body per Bachs's own spec
  (`X-Bachs-Signature-V2` / `X-Bachs-Timestamp`, 300s replay tolerance,
  `src/bachs.rs`) before trusting anything in the payload. This is now the
  **only** path that can confirm a Bachs-sourced payment.
- **`mock_confirm_payment` now refuses any payment with `provider =
  "Bachs"`** (`409`) — closes the self-attestation hole directly rather
  than just adding a parallel "more correct" path alongside the old
  forgeable one.
- 5 new unit tests on the signature verification itself (valid signature,
  wrong secret, tampered body, stale timestamp, malformed input) — this is
  the security-critical piece, so it gets direct test coverage rather than
  only end-to-end checks.

Verified end-to-end against the real Bachs sandbox, not mocked: created a
real checkout session; confirmed a buyer calling `payment/confirm`
directly on a Bachs-sourced payment is refused (`409`); a forged webhook
signature is rejected (`401`); a correctly-signed webhook (computed
exactly per Bachs's documented scheme) settles the payment and drives the
transaction to `LOGISTICS_PENDING`; a re-delivered webhook is idempotent;
an unknown `checkout_id` is acknowledged rather than erroring (so Bachs
doesn't retry forever); a `collection.failed` webhook correctly marks the
payment `FAILED` and the transaction `PAYMENT_FAILED`. `cargo test` passes
(15 tests, 5 new).

**Not fixed, deliberately out of scope here**: the frontend still calls
the *old* browser-to-Bachs client directly (`src/lib/bachs.ts`) and the
`?payment=success` redirect handler still calls `confirm()` client-side.
Wiring the frontend to the new backend endpoints — and changing the
redirect handler to poll for the webhook's result instead of trusting the
query param — is the necessary next step; this fix alone means that old
frontend path will now correctly fail closed (`409`) rather than silently
"succeeding," but the UI won't reflect real payment status until that
frontend work lands.

**Also carried over from before this fix, unresolved**: the credential
exposure itself. `BACHS_SECRET_KEY` and `BACHS_WEBHOOK_SECRET` still
default to the same values already public in `src/lib/bachs.ts` and this
repo's git history — per the maintainer, these are intentionally shared/
reusable team credentials, not treated as a leak requiring rotation. Worth
being precise about what that does and doesn't cover: it's a reasonable
call for the API key (limits blast radius to "abuse of a shared sandbox
account," not a live financial account). The webhook secret is a
different risk class — if it stays public, anyone who reads this repo can
forge a validly-signed webhook, which fully defeats the verification this
fix just built. Flagged explicitly; the maintainer's call to make.

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
