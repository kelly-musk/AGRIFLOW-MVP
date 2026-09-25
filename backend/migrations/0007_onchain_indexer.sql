-- On-chain event sync for AgriFlowEscrow.sol (issue #56). Two tables:
--
-- on_chain_events: append-only, idempotent raw log of every decoded event
-- (unique on tx_hash+log_index so re-polling an overlapping block range is
-- always safe). This is what "record payout receipt" means for Withdrawn/
-- FundsCredited -- the event itself, with its decoded payload, is the
-- receipt. Not duplicating the contract's own withdrawableBalances mapping
-- here since it's already queryable on-chain via getWithdrawableBalance.
--
-- on_chain_trades: materialized current state per tradeId, kept in sync by
-- replaying events onto it. The link back to a marketplace transaction
-- lives on transactions.on_chain_trade_id (below), not here -- there's no
-- way yet to populate it, since nothing calls fundTradeFromIntent yet
-- (that's issue #54, not built). The column exists now so #54 has
-- somewhere to write the link once it exists.
--
-- indexer_cursor: single-row table tracking the last block fully
-- processed, so a restart resumes instead of re-scanning from genesis.

CREATE TABLE on_chain_events (
    id              BIGSERIAL PRIMARY KEY,
    tx_hash         TEXT NOT NULL,
    log_index       INTEGER NOT NULL,
    block_number    BIGINT NOT NULL,
    event_name      TEXT NOT NULL,
    trade_id        TEXT,
    payload         JSONB NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tx_hash, log_index)
);

CREATE INDEX idx_on_chain_events_trade ON on_chain_events(trade_id);
CREATE INDEX idx_on_chain_events_block ON on_chain_events(block_number);

CREATE TABLE on_chain_trades (
    trade_id                   TEXT PRIMARY KEY,
    buyer                      TEXT NOT NULL,
    supplier                   TEXT NOT NULL,
    logistics                  TEXT,
    token                      TEXT NOT NULL,
    generated_deposit_address  TEXT NOT NULL,
    details_hash               TEXT NOT NULL,
    intent_tx_hash             TEXT,
    goods_amount                NUMERIC NOT NULL,
    logistics_amount            NUMERIC NOT NULL,
    platform_fee                 NUMERIC NOT NULL,
    status                        TEXT NOT NULL DEFAULT 'FUNDED'
                                    CHECK (status IN ('FUNDED', 'COMPLETED', 'DISPUTED', 'REFUNDED')),
    funded_at                     TIMESTAMPTZ NOT NULL,
    completed_at                  TIMESTAMPTZ,
    disputed_at                   TIMESTAMPTZ,
    refunded_at                   TIMESTAMPTZ,
    created_at                    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE indexer_cursor (
    id          TEXT PRIMARY KEY,
    last_block  BIGINT NOT NULL
);

-- Forward-compat link for #54 -- see comment above. Nullable, unpopulated here.
ALTER TABLE transactions ADD COLUMN on_chain_trade_id TEXT REFERENCES on_chain_trades(trade_id);
