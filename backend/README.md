# AgriFlow API

Rust (Axum + SQLx/PostgreSQL) backend implementing the trade, escrow, logistics
and dispute workflow described in the root [`README.md`](../README.md). It is
a full port of the business logic that previously lived only in the
frontend's `localStorage`-backed `src/services/*` — the same matching engine,
the same transaction state machine, the same audit trail — now enforced
server-side behind a real API with JWT auth and password hashing.

Payments are simulated (an internal "AgriFlow Escrow Service" that mirrors
the state transitions a real processor webhook would drive) so the trade
loop is fully testable today; swapping in Paystack/Flutterwave later only
touches `src/handlers/payments.rs`.

## Run it

Requires Rust and a PostgreSQL instance.

```bash
# 1. Start Postgres (or point DATABASE_URL at one you already have)
docker run -d --name agriflow-postgres \
  -e POSTGRES_USER=agriflow -e POSTGRES_PASSWORD=agriflow_dev_pw -e POSTGRES_DB=agriflow \
  -p 5433:5432 postgres:16-alpine

# 2. Configure
cp .env.example .env   # edit if your DB/port differ

# 3. Run — migrations and demo-account seeding happen automatically on boot
cargo run
```

The server listens on `:8080` (override with `PORT`). Health check:
`GET /api/health`.

On first boot it seeds the same four demo accounts as the frontend
(`buyer@kolafarms.com`, `supplier@adeyemi.com`, `logistics@swifthaul.com`,
`admin@agriflow.ng`, all password `agriflow123`), plus one sample listing and
demand, so the existing demo walkthrough works unchanged. Seeding is
skipped if the `users` table is already populated.

## API shape

All endpoints are under `/api`. Authenticated routes expect
`Authorization: Bearer <token>` from `/api/auth/login` or `/api/auth/register`.

| Area | Routes |
|---|---|
| Auth | `POST /auth/register`, `POST /auth/login`, `GET /auth/me`, `PATCH /auth/profile` |
| Supply | `POST\|GET /supply`, `GET /supply/mine`, `GET\|PATCH /supply/{id}`, `GET /supply/{id}/matches` |
| Demand | `POST\|GET /demand`, `GET /demand/mine`, `GET /demand/{id}`, `POST /demand/{id}/matches` |
| Transactions | `POST\|GET /transactions`, `GET /transactions/{id}`, `POST /transactions/{id}/transition` |
| Payments | `POST /payments/initiate`, `POST /payments/{id}/confirm`, `POST /payments/{id}/fail`, `GET /payments/transaction/{id}` |
| Logistics | `GET /logistics/jobs/pending\|mine`, `GET /logistics/jobs/{id}`, `POST /logistics/jobs/{id}/assign\|accept\|reject`, `PATCH /logistics/jobs/{id}/status`, `GET /logistics/providers` |
| Disputes | `POST\|GET /disputes`, `GET /disputes/{id}`, `POST /disputes/{id}/resolve` |
| Notifications | `GET /notifications`, `GET /notifications/unread-count`, `POST /notifications/{id}/read`, `POST /notifications/read-all` |
| Audit | `GET /audit` (admin), `GET /audit/transaction/{id}` |

The transaction status machine and role permissions (`src/state_machine.rs`)
and the matching-score algorithm (`src/matching.rs`) are direct ports of
`src/services/transactionStateMachine.ts` and `src/services/matchingService.ts`
— keep both sides in sync if the rules change.

## Wiring up the frontend

The frontend's `src/services/*.ts` files currently read/write `localStorage`
directly. To point the UI at this API, replace each service's storage calls
with `fetch` calls to the matching endpoint above and store the JWT (e.g. in
`localStorage` under `agriflow_session`) instead of the whole session object.
The response shapes match the frontend's `src/types/index.ts` field names in
snake_case (Rust/Postgres convention) rather than camelCase — either adjust
the TypeScript types or map fields at the fetch boundary.
