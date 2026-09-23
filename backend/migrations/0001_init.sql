-- AgriFlow core schema — mirrors src/types/index.ts on the frontend.

CREATE TABLE users (
    id                TEXT PRIMARY KEY,
    email             TEXT NOT NULL UNIQUE,
    password_hash     TEXT NOT NULL,
    name              TEXT NOT NULL,
    role              TEXT NOT NULL CHECK (role IN ('buyer', 'supplier', 'logistics', 'admin')),
    organization_name TEXT,
    phone             TEXT,
    location          TEXT,
    verified          BOOLEAN NOT NULL DEFAULT TRUE,
    profile_complete  BOOLEAN NOT NULL DEFAULT TRUE,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE supply_listings (
    id                 TEXT PRIMARY KEY,
    supplier_id        TEXT NOT NULL REFERENCES users(id),
    supplier_name      TEXT NOT NULL,
    supplier_verified  BOOLEAN NOT NULL,
    commodity          TEXT NOT NULL,
    quantity           DOUBLE PRECISION NOT NULL,
    unit               TEXT NOT NULL,
    quality_grade      TEXT NOT NULL,
    price_per_unit     DOUBLE PRECISION NOT NULL,
    currency           TEXT NOT NULL,
    location           TEXT NOT NULL,
    availability_date  TIMESTAMPTZ NOT NULL,
    description        TEXT NOT NULL DEFAULT '',
    status             TEXT NOT NULL DEFAULT 'active',
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_supply_listings_supplier ON supply_listings(supplier_id);
CREATE INDEX idx_supply_listings_status ON supply_listings(status);

CREATE TABLE demand_requests (
    id                    TEXT PRIMARY KEY,
    buyer_id              TEXT NOT NULL REFERENCES users(id),
    buyer_name            TEXT NOT NULL,
    commodity             TEXT NOT NULL,
    quantity              DOUBLE PRECISION NOT NULL,
    unit                  TEXT NOT NULL,
    quality_grade         TEXT NOT NULL,
    destination_location  TEXT NOT NULL,
    required_by_date      TIMESTAMPTZ NOT NULL,
    indicative_budget     DOUBLE PRECISION NOT NULL,
    currency              TEXT NOT NULL,
    notes                 TEXT,
    status                TEXT NOT NULL DEFAULT 'open',
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_demand_requests_buyer ON demand_requests(buyer_id);

CREATE TABLE matches (
    id           TEXT PRIMARY KEY,
    demand_id    TEXT NOT NULL REFERENCES demand_requests(id),
    listing_id   TEXT NOT NULL REFERENCES supply_listings(id),
    buyer_id     TEXT NOT NULL,
    supplier_id  TEXT NOT NULL,
    score        INTEGER NOT NULL,
    factors      JSONB NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (demand_id, listing_id)
);

CREATE TABLE transactions (
    id                       TEXT PRIMARY KEY,
    listing_id               TEXT NOT NULL REFERENCES supply_listings(id),
    demand_id                TEXT REFERENCES demand_requests(id),
    buyer_id                 TEXT NOT NULL REFERENCES users(id),
    buyer_name               TEXT NOT NULL,
    supplier_id              TEXT NOT NULL REFERENCES users(id),
    supplier_name            TEXT NOT NULL,
    commodity                TEXT NOT NULL,
    quantity                 DOUBLE PRECISION NOT NULL,
    unit                     TEXT NOT NULL,
    quality_grade            TEXT NOT NULL,
    price_per_unit           DOUBLE PRECISION NOT NULL,
    total_amount             DOUBLE PRECISION NOT NULL,
    currency                 TEXT NOT NULL,
    pickup_location          TEXT NOT NULL,
    delivery_location        TEXT NOT NULL,
    expected_delivery_date   TIMESTAMPTZ NOT NULL,
    status                   TEXT NOT NULL,
    payment_id               TEXT,
    logistics_job_id         TEXT,
    dispute_id               TEXT,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_transactions_buyer ON transactions(buyer_id);
CREATE INDEX idx_transactions_supplier ON transactions(supplier_id);

CREATE TABLE transaction_events (
    id              BIGSERIAL PRIMARY KEY,
    transaction_id  TEXT NOT NULL REFERENCES transactions(id),
    status          TEXT NOT NULL,
    ts              TIMESTAMPTZ NOT NULL DEFAULT now(),
    actor           TEXT NOT NULL,
    actor_role      TEXT NOT NULL,
    note            TEXT
);
CREATE INDEX idx_transaction_events_txn ON transaction_events(transaction_id);

CREATE TABLE payments (
    id                   TEXT PRIMARY KEY,
    transaction_id       TEXT NOT NULL REFERENCES transactions(id),
    payer_id             TEXT NOT NULL,
    amount               DOUBLE PRECISION NOT NULL,
    currency             TEXT NOT NULL,
    provider             TEXT NOT NULL,
    provider_reference   TEXT,
    status               TEXT NOT NULL,
    failure_reason       TEXT,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at         TIMESTAMPTZ
);
CREATE INDEX idx_payments_txn ON payments(transaction_id);

CREATE TABLE logistics_jobs (
    id                       TEXT PRIMARY KEY,
    transaction_id           TEXT NOT NULL REFERENCES transactions(id),
    commodity                TEXT NOT NULL,
    quantity                 DOUBLE PRECISION NOT NULL,
    unit                     TEXT NOT NULL,
    pickup_location          TEXT NOT NULL,
    delivery_location        TEXT NOT NULL,
    pickup_date              TIMESTAMPTZ NOT NULL,
    expected_delivery_date   TIMESTAMPTZ NOT NULL,
    logistics_cost           DOUBLE PRECISION NOT NULL,
    currency                 TEXT NOT NULL,
    provider_id              TEXT,
    provider_name            TEXT,
    status                   TEXT NOT NULL DEFAULT 'PENDING',
    proof_recipient_name     TEXT,
    proof_delivery_note      TEXT,
    proof_timestamp          TIMESTAMPTZ,
    proof_recorded_by        TEXT,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_logistics_jobs_txn ON logistics_jobs(transaction_id);
CREATE INDEX idx_logistics_jobs_provider ON logistics_jobs(provider_id);

CREATE TABLE disputes (
    id                TEXT PRIMARY KEY,
    transaction_id    TEXT NOT NULL REFERENCES transactions(id),
    raised_by_id      TEXT NOT NULL,
    raised_by_name    TEXT NOT NULL,
    reason            TEXT NOT NULL,
    description       TEXT NOT NULL,
    status            TEXT NOT NULL DEFAULT 'OPEN',
    resolution        TEXT,
    resolved_by_id    TEXT,
    resolved_by_name  TEXT,
    resolved_at       TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_disputes_txn ON disputes(transaction_id);

CREATE TABLE notifications (
    id              TEXT PRIMARY KEY,
    user_id         TEXT NOT NULL REFERENCES users(id),
    type            TEXT NOT NULL,
    title           TEXT NOT NULL,
    message         TEXT NOT NULL,
    transaction_id  TEXT,
    read            BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_notifications_user ON notifications(user_id);

CREATE TABLE audit_events (
    id              TEXT PRIMARY KEY,
    action          TEXT NOT NULL,
    actor_id        TEXT NOT NULL,
    actor_name      TEXT NOT NULL,
    actor_role      TEXT NOT NULL,
    entity_id       TEXT,
    entity_type     TEXT,
    detail          TEXT,
    transaction_id  TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_audit_events_txn ON audit_events(transaction_id);
