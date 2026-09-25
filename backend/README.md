# agriflow-api

Rust backend for AgriFlow. Replaces `localStorage` as the source of truth for
users, supply listings, demand requests, and transactions — ported 1:1 from
the business logic in the React app's `src/services/*`, so the frontend can
be repointed from `localStorage` to HTTP calls without behavior changes.

This slice covers **auth, users, admin, listings, demands, the transaction
state machine, mock escrow payments, logistics job tracking, disputes, and
supplier/logistics wallet payouts**. Notifications and the audit log are
not built yet — see "Not built yet" below.

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

# 4. (optional) Seed the default admin account -- needs ADMIN_SEED_PASSWORD set
cargo run --bin seed_admin

# 5. Run the API
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
| GET    | `/health`                     | —              | `{"status":"ok","db":"connected"}`, or `503` + `{"status":"error","db":"disconnected"}` if the DB is unreachable -- actually checks, not hardcoded |
| POST   | `/auth/register`              | —              | buyer / supplier / logistics; `admin` needs the `X-Admin-Registration-Key` header. Sends a welcome email |
| POST   | `/auth/login`                 | —              | |
| POST   | `/auth/admin/login`           | —              | Same as `/auth/login`, but rejects non-admin credentials with the same generic error as a wrong password |
| GET    | `/auth/me`                    | any            | |
| GET    | `/admin/users`                | admin          | `?role=&search=&page=&limit=` (page default 1, limit default 20, max 100). Returns `{ users, total, page, limit }`. `search` matches name/email/organization (case-insensitive substring) |
| PATCH  | `/admin/users/:id/verify`     | admin          | `{ "verified": true \| false }` -- can un-verify, not just verify |
| GET    | `/listings`                   | any            | `?commodity=&status=` (default `status=active`) |
| GET    | `/listings/mine`               | supplier        | |
| POST   | `/listings`                    | supplier        | Optional `mediaIds: [...]` attaches uploads made before the listing existed, in that order; the first photo becomes the cover |
| GET    | `/listings/:id`                | any             | |
| POST   | `/media/presigned-url`         | supplier        | `{ filename, contentType, sizeBytes, listingId? }` → `{ mediaId, uploadUrl, method: "PUT", headers, expiresAt, key }`. JPEG/PNG/WebP ≤ 20 MB, MP4/WebM/MOV ≤ 80 MB. The URL expires in 15 min and is signed for that exact `Content-Type` and size. `listingId` (must be yours) attaches immediately; max 8 per listing, 20 unattached per supplier |
| POST   | `/media/:id/complete`          | uploader or admin | Call after the PUT. Verifies the object and its size, then queues processing (`status: processing` → `ready`/`failed`). Idempotent |
| GET    | `/media/:id`                   | uploader or admin | Status of one upload, for polling |
| PATCH  | `/media/:id`                   | uploader or admin | `{ caption?, isCover?, sortOrder? }`. One cover per listing (photos only); `caption: ""` clears it |
| DELETE | `/media/:id`                   | uploader or admin | Deletes the file and its renditions from the bucket, then the row. `204` |
| GET    | `/media/:id/content`           | —               | `?variant=original` (default) \| `small` \| `large` \| `thumbnail`. `302` to a 1-hour presigned bucket URL, served as the validated type. Only uploaded media; `404` for pending/failed |
| PATCH  | `/listings/:id`                 | supplier (owner) | |
| GET    | `/demands`                       | any               | `?commodity=&status=` (default `status=open`) |
| GET    | `/demands/mine`                  | buyer             | |
| POST   | `/demands`                        | buyer             | |
| GET    | `/demands/:id`                     | any                | |
| POST   | `/transactions`                    | buyer              | Creates from a listing; validates stock |
| GET    | `/transactions`                     | any                 | Role-scoped: buyer/supplier see their own, admin sees all |
| GET    | `/transactions/:id`                  | participant or admin | Includes full event history |
| POST   | `/transactions/:id/transition`        | participant (role-gated by state machine) | `{ "to": "ACCEPTED", "note": "..." }` |
| GET    | `/transactions/:id/payment`            | participant or admin | The transaction's payment record, if one exists |
| POST   | `/transactions/:id/payment/initiate`   | buyer (owner)      | `{ "amount": 10000, "currency": "NGN" }`. Idempotent. Also moves an `ACCEPTED` transaction to `PAYMENT_PENDING`, so the frontend's own `/transition` call is optional |
| POST   | `/transactions/:id/payment/confirm`    | buyer (owner)      | Mock escrow settlement -- see `transactions.rs::mock_confirm_payment` doc comment for why this is buyer-triggered rather than a real payment webhook. Atomically settles the payment, drives the transaction to `LOGISTICS_PENDING`, and auto-creates its `logistics_jobs` row |
| POST   | `/transactions/:id/payment/fail`       | buyer (owner)      | `{ "reason": "..." }` |
| POST   | `/transactions/:id/payment/bachs/checkout-session` | buyer (owner) | `{ "successUrl"?, "cancelUrl"? }` → `{ checkoutId, checkoutUrl }`. Creates a Bachs hosted checkout for the initiated payment's amount (never taken from the request). Transaction must be `PAYMENT_PENDING` or `PAYMENT_FAILED` (`409` otherwise). Redirect URLs off `FRONTEND_BASE_URL` are replaced with defaults. `503` if Bachs isn't configured, `502` if Bachs rejects the call. After this, `payment/confirm` and `payment/fail` return `409` for the payment |
| POST   | `/webhooks/bachs`                      | Bachs signature      | `collection.succeeded` → payment `CONFIRMED`, transaction → `LOGISTICS_PENDING` + logistics job. `collection.failed` → payment `FAILED`, transaction → `PAYMENT_FAILED`. `401` on a bad or stale signature. Deduplicated on the event id |
| GET    | `/logistics/jobs`                     | logistics or admin | Logistics sees unclaimed (`PENDING`) jobs plus their own; admin sees all |
| POST   | `/logistics/jobs/:id/claim`           | logistics           | Self-assigns an unclaimed job. `409` if already claimed |
| POST   | `/logistics/jobs/:id/assign`          | admin                | `{ "providerId": "..." }` -- must be a `logistics`-role user |
| PATCH  | `/logistics/jobs/:id/status`          | logistics (job's assigned provider only) | `{ "status": "IN_TRANSIT", "proofOfDelivery": {...} }`. Advances the transaction state machine for statuses that map to one (`ACCEPTED`, `REJECTED`, `READY_FOR_PICKUP`, `PICKED_UP`, `IN_TRANSIT`, `DELIVERED`, `COMPLETED`) |
| POST   | `/disputes`                            | buyer (owner)        | `{ "transactionId": "...", "reason": "...", "description": "..." }`. Buyer-only: the transaction state machine's `DISPUTED` status only allows `Actor::Buyer` (see `disputes.rs` doc comment). Atomically transitions the transaction to `DISPUTED` |
| GET    | `/disputes`                            | any                  | Admin sees all; buyer/supplier see disputes on transactions they're a party to |
| POST   | `/disputes/:id/resolve`                | admin                | `{ "decision": "...", "outcome": "completed" \| "cancelled" }`. Atomically transitions the transaction to `COMPLETED` or `CANCELLED` |
| GET    | `/wallet/summary`                      | supplier or logistics | `{ totalEarned, pendingEscrow, withdrawn, available, currency, completedCount }`. No real payout rail -- see below |
| POST   | `/wallet/withdraw`                     | supplier or logistics | `{ "amount": 500000, "method": "bank_transfer" \| "crypto_usdc", "bankDetails": {...} }` (or `"stellarPublicKey"` for `crypto_usdc`). `400` if `amount` exceeds the freshly-recomputed `available` balance |

Every listing response includes `media: [...]` (processing/ready items,
cover first, then `sortOrder`), in the shape of the frontend's
`ListingMedia`: `url` (the `large` WebP for photos once ready, else the
original), `thumbnailUrl` (`small` WebP / video first frame), `variants`,
`status`, `caption`, `isCover`.

### Listing media (issue #32)

Files never pass through the API on the way in: the browser `PUT`s straight
to the bucket with the presigned URL, then calls `/complete`. Processing
(`src/media.rs`) runs each upload through **ffmpeg**, which generates the
derivatives -- `small` (480px) and `large` (1280px) WebP renditions for
photos, a 960px WebP first-frame thumbnail for videos -- and proves the
file is really an image/video: anything ffmpeg can't decode (e.g. HTML
renamed `.png`) is deleted and marked `failed`. At most two jobs run at
once; jobs interrupted by a restart resume at boot, and uploads never
attached to a listing are deleted after 24 hours.

Keys follow `listings/{listingId}/images|videos/{id}.{ext}`; uploads made
before the listing exists live under `listings/pending/{supplierId}/...`
and keep that key once attached. Renditions sit next to the original as
`{id}-small.webp` etc.

Local setup: any S3-compatible server works, e.g.

```bash
docker run -d -p 8333:8333 chrislusf/seaweedfs server -s3 -dir=/data \
  -master.volumeSizeLimitMB=64 -volume.max=50   # then create a bucket
```

with `S3_ENDPOINT=http://127.0.0.1:8333`, `S3_PATH_STYLE=true`, and
`ffmpeg` installed. `e2e/listing-media.spec.ts` covers the browser flow.

A `logistics_jobs` row is auto-created (idempotently) the moment a
transaction's payment is confirmed (`mock_confirm_payment` /
`transactions.logistics_job_id`), mirroring
`logisticsService.createJobForTransaction`'s "called automatically after
payment is confirmed" behavior -- there's no manual "create job" endpoint.

`/wallet/withdraw` doesn't call any real payout rail -- there's no NIBSS or
on-chain minting integration in this codebase to call. `payoutTxHash` is a
generated placeholder reference (`NIBSS_PAY_########` / `0x_payout_...`),
matching the frontend's own fallback format for when its real on-chain
mint attempt fails. Real payouts belong with the Web3 relayer/indexer work
(issues #54-56), not reimplemented here. Withdrawal amount is checked
against a freshly recomputed `available` balance inside a
`pg_advisory_xact_lock`-protected transaction, so two concurrent
withdrawal requests from the same user can't both read the same balance
and jointly overdraw it.

## Not built yet (next slices)

- **Notifications, audit log** — state machine and data model are ready to
  extend, tables/endpoints aren't built. The `GET /admin/audit` endpoint
  from issue #33 isn't implemented for this reason -- there's no
  `audit_logs` table to back it yet, and the `require_role(Admin)` pattern
  `admin::list_users`/`set_verified` use is ready to reuse once it exists.
- **Real on-chain escrow and payouts** — payments and withdrawals today are
  both mock flows (buyer-triggered settlement, generated payout
  references), not a real payment provider or on-chain contract.
- **Commodity inspection metadata** — tracked as issue #34.
- **Matching engine** — the weighted scoring algorithm from
  `matchingService.ts` hasn't been ported.

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
| `ADMIN_SEED_PASSWORD` | no | Read only by `cargo run --bin seed_admin`, which upserts the default `admin@agriflow.africa` account. Not read by the API server itself |
| `RESEND_API_KEY` | no | [Resend](https://resend.com) key for the welcome email sent on registration. Unset → emails are skipped (logged) |
| `EMAIL_FROM` | no | Sender address. Defaults to Resend's test sender `onboarding@resend.dev`, which only delivers to the Resend account owner — verify a domain in Resend and set this before sending to real users |
| `BACHS_SECRET_KEY` | no | [Bachs.io](https://docs.bachs.io) API key. `sk_sandbox_...` uses `sandbox-api.bachs.io`, anything else `api.bachs.io`. Unset → the checkout-session endpoint returns `503` |
| `BACHS_WEBHOOK_SECRET` | no | Signing secret of the Bachs webhook endpoint (developer portal → Webhooks), which should point at `<API origin>/api/webhooks/bachs` and subscribe to `collection.succeeded` and `collection.failed`. Unset → every delivery is rejected |
| `FRONTEND_BASE_URL` | no | Public origin of the React app, e.g. `https://agri-flowmvp.vercel.app`. Checkout redirects are restricted to it and default to its `/app/transactions/:id` pages. Bachs rejects `localhost` redirect URLs |
| `S3_ENDPOINT`, `S3_BUCKET`, `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY` | no | S3-compatible bucket for listing media. All four or none; none → the `/media` endpoints return `503`. On Railway, reference the bucket service's `ENDPOINT`, `BUCKET`, `ACCESS_KEY_ID`, `SECRET_ACCESS_KEY` |
| `S3_REGION` | no (`auto`) | Region to sign for. `auto` suits Railway Buckets and R2; AWS needs the real region |
| `S3_PATH_STYLE` | no (`false`) | `true` for path-style URLs (MinIO/SeaweedFS, older Railway Buckets) |
| `S3_CORS_ORIGINS` | no | Extra comma-separated origins (e.g. `http://localhost:5173`) allowed to upload from a browser. At boot the API sets the bucket's CORS rules to these plus `FRONTEND_BASE_URL`; without them browsers can't `PUT` to the bucket |

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
