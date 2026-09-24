-- Correlates a payment row with its Bachs.io checkout session, so the
-- webhook handler can look up which payment a given event belongs to.
ALTER TABLE payments ADD COLUMN bachs_session_id TEXT;
CREATE INDEX idx_payments_bachs_session ON payments(bachs_session_id);
