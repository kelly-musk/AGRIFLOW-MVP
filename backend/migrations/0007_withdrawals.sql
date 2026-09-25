-- Persists payout/withdrawal requests, previously tracked only in the
-- frontend's localStorage (src/services/walletService.ts). Method values
-- match that file's current (post-Stellar-rename) shape: 'bank_transfer'
-- or 'crypto_usdc' -- 'stellar_usdc' is accepted as a legacy input synonym
-- and normalized to 'crypto_usdc' at the handler, same as the frontend
-- does, but never stored as its own value.

CREATE TABLE withdrawals (
    id                   TEXT PRIMARY KEY,
    user_id              TEXT NOT NULL REFERENCES users(id),
    user_name            TEXT NOT NULL,
    user_role            TEXT NOT NULL CHECK (user_role IN ('supplier', 'logistics')),
    amount               NUMERIC NOT NULL CHECK (amount > 0),
    currency             TEXT NOT NULL DEFAULT 'NGN',
    method               TEXT NOT NULL CHECK (method IN ('bank_transfer', 'crypto_usdc')),
    bank_name            TEXT,
    account_number       TEXT,
    account_name         TEXT,
    stellar_public_key   TEXT,
    status               TEXT NOT NULL DEFAULT 'COMPLETED'
                           CHECK (status IN ('PROCESSING', 'COMPLETED', 'FAILED')),
    payout_tx_hash       TEXT,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_withdrawals_user ON withdrawals(user_id);
