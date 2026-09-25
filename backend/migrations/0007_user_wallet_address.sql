-- Links a platform account to an EVM wallet address (prerequisite for
-- issue #54: AgriFlowEscrow.fundTradeFromIntent needs real addresses for
-- buyer/supplier/logistics, and nothing in this schema mapped a user to
-- one before this). Self-attested, not cryptographically verified -- see
-- routes/users.rs's doc comment for why that's an acceptable trust
-- boundary here (a user can only set their own address, so a mistake only
-- ever hurts themselves).

ALTER TABLE users ADD COLUMN wallet_address TEXT;

-- Prevents two accounts from claiming the same address, which would make
-- fundTradeFromIntent's buyer/supplier/logistics resolution ambiguous.
-- Partial (WHERE NOT NULL) so multiple users can each have no address set.
CREATE UNIQUE INDEX idx_users_wallet_address ON users(wallet_address) WHERE wallet_address IS NOT NULL;
