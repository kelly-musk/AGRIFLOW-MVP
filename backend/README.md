# agriflow-api

Rust backend for AgriFlow. Replaces `localStorage` as the source of truth for
users, supply listings, demand requests, and transactions — ported 1:1 from
the business logic in the React app's `src/services/*`, so the frontend can
be repointed from `localStorage` to HTTP calls without behavior changes.

This slice covers **auth, users, listings, demands, and the transaction state
machine**. Escrow/payments, logistics jobs, disputes, notifications, and the
audit log are not built yet — see "Not built yet" below.

## Stack

- **Axum** (web framework) + **Tokio** (async runtime)
- **Postgres** via **sqlx** (compile-time checked queries)
- **JWT** auth (`jsonwebtoken`) + **Argon2** password hashing
- **rust_decimal** for money/quantity fields (no float rounding on prices)

## Running it

```bash
# 1. Start Postgres
docker-compose up -d

# 2. Copy env and adjust if needed (defaults match docker-compose.yml)
cp .env.example .env

# 3. Apply migrations
cargo run --bin migrate

# 4. Run the API
cargo run --bin agriflow-api
# -> listening on 0.0.0.0:8080
```

Run tests (currently the transaction state machine's parity tests):

```bash
cargo test
```

## Architecture notes

- `src/state_machine.rs` is a **1:1 port** of
  `src/services/transactionStateMachine.ts` from the React app — same
  transition table, same actor-role gating, same idempotent-reaccept special
  case. This is now the authoritative copy; the TS file becomes display-only
  once the frontend is repointed at this API. If the transition rules ever
  need to change, change them here first and mirror back to the TS file
  (or delete it once the frontend no longer needs its own copy).
- Every DB-row struct doubles as the API response shape via `#[serde(rename_all
  = "camelCase")]`, matching the field names already used by
  `src/types/index.ts` in the frontend (e.g. `pricePerUnit`, not
  `price_per_unit`) — the goal is a drop-in swap for the frontend's
  `services/*.ts` fetch calls with no reshaping logic needed.
- IDs are generated in the same human-readable shape the frontend/demo data
  already uses (`USR-BUY-12345`, `TXN-AGF-87726`, ...) rather than switching
  to UUIDs, so existing fixtures/screenshots/docs referencing these IDs stay
  legible.
- Auth is stateless JWT (`Authorization: Bearer <token>`), decoded per
  request via the `AuthUser` extractor — no session table. Role checks
  (`AuthUser::require_role`) mirror the frontend's `UserRole` gating.

## API surface

All routes are under `/api`.

| Method | Path                          | Auth           | Notes |
|--------|-------------------------------|----------------|-------|
| POST   | `/auth/register`              | —              | buyer / supplier / logistics; `admin` needs the `X-Admin-Registration-Key` header. Sends a welcome email |
| POST   | `/auth/login`                 | —              | |
| GET    | `/auth/me`                    | any            | |
| GET    | `/listings`                   | any            | `?commodity=&status=` (default `status=active`) |
| GET    | `/listings/mine`               | supplier        | |
| POST   | `/listings`                    | supplier        | |
| GET    | `/listings/:id`                | any             | |
| PATCH  | `/listings/:id`                 | supplier (owner) | |
| GET    | `/demands`                       | any               | `?commodity=&status=` (default `status=open`) |
| GET    | `/demands/mine`                  | buyer             | |
| POST   | `/demands`                        | buyer             | |
| GET    | `/demands/:id`                     | any                | |
| POST   | `/transactions`                    | buyer              | Creates from a listing; validates stock |
| GET    | `/transactions`                     | any                 | Role-scoped: buyer/supplier see their own, admin sees all |
| GET    | `/transactions/:id`                  | participant or admin | Includes full event history |
| POST   | `/transactions/:id/transition`        | participant (role-gated by state machine) | `{ "to": "ACCEPTED", "note": "..." }` |
| GET    | `/transactions/:id/payment`           | participant or admin | The payment row for this transaction, if any yet (`null` before `payment/initiate`) |
| POST   | `/transactions/:id/payment/initiate`  | buyer              | Creates a `PENDING` payment row: `{ "amount": ..., "currency": "NGN" }`. Idempotent |
| POST   | `/transactions/:id/payment/confirm`   | buyer              | Settles a **non-Bachs** payment (mock escrow / on-chain-verified paths). Refuses `409` if `payment.provider == "Bachs"` — see below |
| POST   | `/transactions/:id/payment/fail`      | buyer              | `{ "reason": "..." }` |
| POST   | `/transactions/:id/payment/bachs/checkout-session` | buyer | Creates a Bachs.io hosted checkout session server-side (secret key never reaches the browser); returns `{ "checkoutUrl": "..." }`. Sets `payment.provider = "Bachs"` |
| POST   | `/webhooks/bachs`                     | none (HMAC-verified) | Bachs's server-to-server webhook. The **only** legitimate way a Bachs-sourced payment gets confirmed — see "Payments & escrow" below |

## Payments & escrow

Three payment rails exist, at different levels of real verification:

- **Bachs.io (Naira)** — real. Checkout session creation happens server-side
  (`src/bachs.rs`), and confirmation only ever happens via the signature-
  verified webhook (`bachs_webhook` in `src/routes/transactions.rs`), never
  via a buyer's own request or the checkout redirect's `?payment=success`
  query param (which is just as forgeable). `mock_confirm_payment` explicitly
  refuses any payment whose `provider` is `Bachs`.
- **Mock escrow (everything else)** — `mock_confirm_payment`/
  `mock_fail_payment` are buyer-scoped, self-reported settlement with no
  external verification, for rails with no real provider integrated yet
  (e.g. the current USDC/NEAR-intents path). This is intentionally a mock —
  don't confuse "buyer-scoped" with "verified."
- **Stellar/Soroban on-chain** — `payments.stellar_tx_hash` can record a
  real on-chain transaction hash, but nothing currently *verifies* it
  against the chain before confirming (that would mean calling
  `get_trade` on the Soroban contract server-side, same category of fix as
  the Bachs webhook). Recording the hash is a real improvement over losing
  it in React state on refresh; it is not yet the same as verification.

## Not built yet (next slices)

- **Logistics jobs** — provider assignment, milestone updates, proof of
  delivery. The state machine already supports these statuses
  (`LOGISTICS_ASSIGNED`, `IN_TRANSIT`, etc.); only the `logistics_jobs`
  table and endpoints are missing.
- **Disputes, notifications, audit log** — same story: state machine and
  data model are ready to extend, tables/endpoints aren't built.
- **Matching engine** — the weighted scoring algorithm from
  `matchingService.ts` hasn't been ported.
- **On-chain payment verification** — see "Payments & escrow" above.

## Environment variables

See `.env.example`. `JWT_SECRET` must be changed before any real deployment
— the example value is a placeholder.

| Variable | Required | Purpose |
|---|---|---|
| `DATABASE_URL` | yes | Postgres connection string |
| `JWT_SECRET` | yes | Signs auth tokens |
| `JWT_EXPIRY_HOURS` | no (24) | Token lifetime |
| `PORT` / `SERVER_ADDR` | no | Bind address; `PORT` (set by Railway) wins |
| `ADMIN_REGISTRATION_KEY` | no | Registering `role: admin` requires sending this value as the `X-Admin-Registration-Key` header. Unset → no admin can register. The seed and integration-test scripts read it from the same env var name |
| `RESEND_API_KEY` | no | [Resend](https://resend.com) key for the welcome email sent on registration. Unset → emails are skipped (logged) |
| `EMAIL_FROM` | no | Sender address. Defaults to Resend's test sender `onboarding@resend.dev`, which only delivers to the Resend account owner — verify a domain in Resend and set this before sending to real users |
| `BACHS_SECRET_KEY` | no | [Bachs.io](https://docs.bachs.io) API secret key. Defaults to the shared sandbox key already in the frontend's `src/lib/bachs.ts` — intentionally reusable team credentials per the maintainer, not a per-deployment secret |
| `BACHS_API_URL` | no | Defaults to the Bachs sandbox checkout-sessions endpoint |
| `BACHS_WEBHOOK_SECRET` | no | Verifies incoming webhook signatures (HMAC-SHA256). Defaults to the shared sandbox webhook secret. Must match whatever's configured on the Bachs webhook endpoint if that's ever regenerated |

Welcome emails are sent in the background after the account is created, so
an email failure never fails a registration — check the logs for
`email provider rejected send` or `email send failed`.

## Live deployment

The API and its database are deployed and running on Railway — this is a
real, hosted backend, not a local-only project:

- **API**: `https://agriflow-api-production.up.railway.app` (all routes
  under `/api`, e.g. `/api/listings` — there is no page at `/`, this is a
  JSON API with no frontend of its own, so hitting the bare domain in a
  browser correctly 404s)
- **Database**: managed Postgres 18 on a persistent volume, in the same
  Railway project as the API (`renewed-nurturing`), reached over Railway's
  private network — not exposed publicly
- Both services live in one Railway project with two services:
  `agriflow-api` (this backend) and `Postgres`

### How it's wired together

- `agriflow-api`'s `DATABASE_URL` variable is set to
  `${{Postgres.DATABASE_URL}}` — a live reference to the Postgres service's
  connection string, not a copy-pasted value. If Railway ever rotates the
  DB credentials, this updates automatically.
- Migrations run automatically on every boot (`sqlx::migrate!` is embedded
  into the binary at compile time — the `migrations/` directory doesn't
  need to exist at runtime, and re-running an already-applied migration is
  a no-op).
- The Dockerfile builds in **sqlx offline mode**: `.sqlx/` (committed to
  this repo) holds a snapshot of every query's expected schema, generated
  via `cargo sqlx prepare` against a real Postgres instance. This means the
  Docker build never needs live DB access — required, since Railway's build
  containers can't reach the database while building. If you add or change
  any `sqlx::query*!` macro call, you **must** re-run `cargo sqlx prepare`
  (with a real `DATABASE_URL` in `.env` pointing at a migrated DB) and
  commit the updated `.sqlx/` files, or the next deploy's build will fail.

### Redeploying

Deploys are automatic — merging a PR into this repo's `main` ships the
backend, with no manual `railway up`. The chain:

1. A PR is merged into `reactjay/AGRIFLOW-MVP` `main`.
2. The fork `Manuel1234477/AGRIFLOW-MVP` pulls it in within ~15 minutes via
   a scheduled GitHub Action on the fork (`.github/workflows/sync-upstream.yml`,
   which calls GitHub's merge-upstream API). It can also be run on demand
   from the fork's Actions tab, or with **Sync fork** on GitHub.
3. The `agriflow-api` Railway service is connected to the fork's `main`
   with **Root Directory** `/backend`, so every sync triggers a build of
   `backend/Dockerfile` and a deploy.

Railway deploys from the fork rather than this repo because installing
Railway's GitHub app here needs owner access to this repo.

Things that can break the chain:

- **The sync fails if an upstream change touches `.github/workflows/`** —
  the Action's `GITHUB_TOKEN` isn't allowed to push workflow files. Click
  **Sync fork** on the fork's GitHub page once to get past it.
- **GitHub disables scheduled workflows after 60 days of repo inactivity.**
  If deploys stop, check the fork's Actions tab first.
- **A failing Docker build keeps the previous deployment live** — the most
  likely cause is a stale `.sqlx/` cache (see above). Check the build logs
  in the Railway dashboard.

To deploy a local, unmerged change by hand (e.g. to test a hotfix), the CLI
still works — the next GitHub-triggered deploy will replace it:

```bash
cd backend
railway link -p renewed-nurturing -s agriflow-api -e production   # first time only
railway up -s agriflow-api -e production
```

## Handoff: what's next

**The frontend is not connected to this backend yet.** `src/services/*.ts`
in the React app still read/write `localStorage` exclusively — nothing in
the UI calls this API. That wiring is intentionally left undone here; it's
the next piece of work. Roughly, for whoever picks this up:

1. Each `src/services/*.ts` file (`authService`, `supplyService`,
   `demandService`, `transactionService`, ...) needs its `storageService`
   calls replaced with `fetch` calls against
   `https://agriflow-api-production.up.railway.app/api/...` (or a local
   instance during development).
2. The JWT returned from `/api/auth/login` / `/api/auth/register` needs
   somewhere to live client-side (e.g. alongside `AuthSession` in
   `AppContext`), and every subsequent request needs it attached as
   `Authorization: Bearer <token>`.
3. Response shapes already match `src/types/index.ts` field-for-field
   (camelCase, same ID formats), so this should mostly be a mechanical
   swap rather than a reshaping exercise — see "Architecture notes" above.
4. Endpoints not yet built (payments/escrow, logistics jobs, disputes,
   notifications, audit log, matching) will still need `localStorage` or
   stubbing until their backend slices exist — see "Not built yet" above.
