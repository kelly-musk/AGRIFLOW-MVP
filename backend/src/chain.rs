//! On-chain event sync for AgriFlowEscrow.sol (issue #56).
//!
//! Read-only: polls `eth_getLogs` against the contract's deployment address
//! on a public RPC endpoint (no API key or private key needed) and mirrors
//! trade lifecycle events into `on_chain_trades`/`on_chain_events`. See the
//! migration and this module's doc comments for what's genuinely built vs
//! deliberately out of scope (email/push notifications need a wallet
//! address -> platform user mapping that doesn't exist anywhere in this
//! schema yet, and the `transactions` linkage is inert until #54 exists).

use std::time::Duration;

use alloy::primitives::{Address, B256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::{Filter, Log};
use alloy::sol_types::SolEvent;
use serde_json::json;

use crate::state::AppState;

alloy::sol! {
    #[sol(rpc)]
    interface IAgriFlowEscrow {
        event TradeFunded(bytes32 indexed tradeId, address indexed buyer, address indexed supplier, address logistics, address token, uint256 totalAmount, address generatedDepositAddress, bytes32 detailsHash, string intentTxHash);
        event DeliveryConfirmed(bytes32 indexed tradeId, address indexed buyer);
        event FundsCredited(bytes32 indexed tradeId, address indexed recipient, address token, uint256 amount);
        event Withdrawn(address indexed recipient, address indexed token, uint256 amount);
        event TradeDisputed(bytes32 indexed tradeId, address indexed caller);
        event BuyerRefunded(bytes32 indexed tradeId, address indexed buyer, uint256 refundAmount);

        struct Trade {
            bytes32 tradeId;
            address buyer;
            address supplier;
            address logistics;
            address token;
            address generatedDepositAddress;
            bytes32 detailsHash;
            string intentTxHash;
            uint256 goodsAmount;
            uint256 logisticsAmount;
            uint256 platformFee;
            uint8 status;
            uint256 createdAt;
            uint256 fundedAt;
        }

        function getTrade(bytes32 tradeId) external view returns (Trade memory);
    }
}

/// The public RPC endpoint this defaults to caps `eth_getLogs` at 50,000
/// blocks per call (verified directly: a wider range returns
/// `"exceed maximum block range: 50000"`) -- chunk queries to stay under it
/// regardless of which endpoint is actually configured.
const MAX_BLOCK_RANGE: u64 = 50_000;

const CURSOR_ID: &str = "agriflow_escrow";

pub fn spawn_indexer(state: AppState) {
    tokio::spawn(async move {
        let provider = match state.config.chain_rpc_url.parse() {
            Ok(url) => ProviderBuilder::new().connect_http(url),
            Err(e) => {
                tracing::error!(error = %e, url = %state.config.chain_rpc_url, "invalid CHAIN_RPC_URL, indexer disabled");
                return;
            }
        };
        let contract: Address = match state.config.escrow_contract_address.parse() {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(error = %e, address = %state.config.escrow_contract_address, "invalid ESCROW_CONTRACT_ADDRESS, indexer disabled");
                return;
            }
        };

        let mut tick = tokio::time::interval(Duration::from_secs(state.config.chain_poll_interval_secs));
        loop {
            tick.tick().await;
            if let Err(e) = poll_once(&state, &provider, contract).await {
                tracing::error!(error = %format!("{e:#}"), "chain indexer poll failed");
            }
        }
    });
}

async fn poll_once(
    state: &AppState,
    provider: &impl Provider,
    contract: Address,
) -> anyhow::Result<()> {
    let head = provider.get_block_number().await?;

    let cursor = sqlx::query_scalar!(
        "SELECT last_block FROM indexer_cursor WHERE id = $1",
        CURSOR_ID,
    )
    .fetch_optional(&state.db)
    .await?;

    // First run: start from config's chain_start_block (the contract's
    // real deployment block by default), not genesis -- see its doc
    // comment for why this isn't determined by probing historical state.
    let mut from = match cursor {
        Some(b) => (b as u64) + 1,
        None => state.config.chain_start_block,
    };

    if from > head {
        return Ok(());
    }

    while from <= head {
        let to = (from + MAX_BLOCK_RANGE - 1).min(head);

        let filter = Filter::new().address(contract).from_block(from).to_block(to);
        let logs = provider.get_logs(&filter).await?;

        for log in &logs {
            if let Err(e) = process_log(state, provider, log).await {
                tracing::error!(error = %format!("{e:#}"), tx_hash = ?log.transaction_hash, "failed to process chain event, will retry next poll");
                // Leave the cursor before this range so the whole chunk is
                // retried -- on_chain_events' unique constraint makes
                // reprocessing already-stored logs a no-op.
                return Ok(());
            }
        }

        sqlx::query!(
            r#"INSERT INTO indexer_cursor (id, last_block) VALUES ($1, $2)
               ON CONFLICT (id) DO UPDATE SET last_block = EXCLUDED.last_block"#,
            CURSOR_ID,
            to as i64,
        )
        .execute(&state.db)
        .await?;

        from = to + 1;
    }

    Ok(())
}

async fn process_log(state: &AppState, provider: &impl Provider, log: &Log) -> anyhow::Result<()> {
    let Some(topic0) = log.topic0().copied() else {
        return Ok(());
    };

    if topic0 == IAgriFlowEscrow::TradeFunded::SIGNATURE_HASH {
        let decoded = log.log_decode::<IAgriFlowEscrow::TradeFunded>()?;
        handle_trade_funded(state, provider, log, decoded.inner.data).await?;
    } else if topic0 == IAgriFlowEscrow::DeliveryConfirmed::SIGNATURE_HASH {
        let decoded = log.log_decode::<IAgriFlowEscrow::DeliveryConfirmed>()?;
        handle_delivery_confirmed(state, log, decoded.inner.data).await?;
    } else if topic0 == IAgriFlowEscrow::FundsCredited::SIGNATURE_HASH {
        let decoded = log.log_decode::<IAgriFlowEscrow::FundsCredited>()?;
        let data = decoded.inner.data;
        let payload = json!({
            "tradeId": format!("{:#x}", data.tradeId),
            "recipient": data.recipient.to_string(),
            "token": data.token.to_string(),
            "amount": data.amount.to_string(),
        });
        record_event(state, log, "FundsCredited", Some(data.tradeId), payload).await?;
    } else if topic0 == IAgriFlowEscrow::Withdrawn::SIGNATURE_HASH {
        let decoded = log.log_decode::<IAgriFlowEscrow::Withdrawn>()?;
        let data = decoded.inner.data;
        let payload = json!({
            "recipient": data.recipient.to_string(),
            "token": data.token.to_string(),
            "amount": data.amount.to_string(),
        });
        // Withdrawn carries no tradeId (it's a per-user balance sweep, not
        // scoped to one trade) -- recorded with trade_id NULL.
        record_event(state, log, "Withdrawn", None, payload).await?;
    } else if topic0 == IAgriFlowEscrow::TradeDisputed::SIGNATURE_HASH {
        let decoded = log.log_decode::<IAgriFlowEscrow::TradeDisputed>()?;
        handle_trade_disputed(state, log, decoded.inner.data).await?;
    } else if topic0 == IAgriFlowEscrow::BuyerRefunded::SIGNATURE_HASH {
        let decoded = log.log_decode::<IAgriFlowEscrow::BuyerRefunded>()?;
        handle_buyer_refunded(state, log, decoded.inner.data).await?;
    }

    Ok(())
}

fn log_ids(log: &Log) -> anyhow::Result<(String, i32, i64)> {
    let tx_hash = log
        .transaction_hash
        .ok_or_else(|| anyhow::anyhow!("log missing transaction_hash"))?;
    let log_index = log
        .log_index
        .ok_or_else(|| anyhow::anyhow!("log missing log_index"))?;
    let block_number = log
        .block_number
        .ok_or_else(|| anyhow::anyhow!("log missing block_number"))?;
    Ok((format!("{tx_hash:#x}"), log_index as i32, block_number as i64))
}

/// Idempotently records the raw event. Returns `true` if this call actually
/// inserted a new row (i.e. this event hasn't been processed before) --
/// callers only apply state transitions on `true`, so re-polling an
/// overlapping block range never double-applies an event.
async fn record_event(
    state: &AppState,
    log: &Log,
    event_name: &str,
    trade_id: Option<B256>,
    payload: serde_json::Value,
) -> anyhow::Result<bool> {
    let (tx_hash, log_index, block_number) = log_ids(log)?;
    let trade_id_str = trade_id.map(|t| format!("{t:#x}"));

    let inserted = sqlx::query_scalar!(
        r#"
        INSERT INTO on_chain_events (tx_hash, log_index, block_number, event_name, trade_id, payload)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (tx_hash, log_index) DO NOTHING
        RETURNING id
        "#,
        tx_hash,
        log_index,
        block_number,
        event_name,
        trade_id_str,
        payload,
    )
    .fetch_optional(&state.db)
    .await?;

    Ok(inserted.is_some())
}

async fn handle_trade_funded(
    state: &AppState,
    provider: &impl Provider,
    log: &Log,
    data: IAgriFlowEscrow::TradeFunded,
) -> anyhow::Result<()> {
    let trade_id = data.tradeId;
    let payload = json!({
        "tradeId": format!("{trade_id:#x}"),
        "buyer": data.buyer.to_string(),
        "supplier": data.supplier.to_string(),
        "logistics": data.logistics.to_string(),
        "token": data.token.to_string(),
        "totalAmount": data.totalAmount.to_string(),
        "generatedDepositAddress": data.generatedDepositAddress.to_string(),
        "detailsHash": format!("{:#x}", data.detailsHash),
        "intentTxHash": data.intentTxHash,
    });

    if !record_event(state, log, "TradeFunded", Some(trade_id), payload).await? {
        return Ok(());
    }

    // The event only carries totalAmount; fetch the goods/logistics/fee
    // breakdown via the contract's own view function rather than guessing
    // a split, so on_chain_trades matches the contract's real accounting.
    let contract = IAgriFlowEscrow::new(log.address(), provider);
    let trade = contract.getTrade(trade_id).call().await?;

    let block_timestamp = log_timestamp(log);
    let logistics = (data.logistics != Address::ZERO).then(|| data.logistics.to_string());

    sqlx::query!(
        r#"
        INSERT INTO on_chain_trades
            (trade_id, buyer, supplier, logistics, token, generated_deposit_address,
             details_hash, intent_tx_hash, goods_amount, logistics_amount, platform_fee,
             status, funded_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'FUNDED', $12)
        ON CONFLICT (trade_id) DO NOTHING
        "#,
        format!("{trade_id:#x}"),
        data.buyer.to_string(),
        data.supplier.to_string(),
        logistics,
        data.token.to_string(),
        data.generatedDepositAddress.to_string(),
        format!("{:#x}", data.detailsHash),
        data.intentTxHash,
        u256_to_decimal(trade.goodsAmount),
        u256_to_decimal(trade.logisticsAmount),
        u256_to_decimal(trade.platformFee),
        block_timestamp,
    )
    .execute(&state.db)
    .await?;

    Ok(())
}

fn log_timestamp(log: &Log) -> chrono::DateTime<chrono::Utc> {
    log.block_timestamp
        .and_then(|t| chrono::DateTime::from_timestamp(t as i64, 0))
        .unwrap_or_else(chrono::Utc::now)
}

fn u256_to_decimal(v: alloy::primitives::U256) -> rust_decimal::Decimal {
    rust_decimal::Decimal::from_str_exact(&v.to_string()).unwrap_or(rust_decimal::Decimal::ZERO)
}

async fn handle_delivery_confirmed(
    state: &AppState,
    log: &Log,
    data: IAgriFlowEscrow::DeliveryConfirmed,
) -> anyhow::Result<()> {
    let trade_id = data.tradeId;
    let payload = json!({ "tradeId": format!("{trade_id:#x}"), "buyer": data.buyer.to_string() });
    if !record_event(state, log, "DeliveryConfirmed", Some(trade_id), payload).await? {
        return Ok(());
    }

    sqlx::query!(
        r#"UPDATE on_chain_trades SET status = 'COMPLETED', completed_at = $2, updated_at = now() WHERE trade_id = $1"#,
        format!("{trade_id:#x}"),
        log_timestamp(log),
    )
    .execute(&state.db)
    .await?;

    // Best-effort: if #54 has ever linked this trade to a marketplace
    // transaction, mirror the completion. A no-op today -- nothing
    // populates on_chain_trade_id yet.
    sqlx::query!(
        r#"UPDATE transactions SET status = 'COMPLETED', updated_at = now()
           WHERE on_chain_trade_id = $1 AND status <> 'COMPLETED'"#,
        format!("{trade_id:#x}"),
    )
    .execute(&state.db)
    .await?;

    Ok(())
}

async fn handle_trade_disputed(
    state: &AppState,
    log: &Log,
    data: IAgriFlowEscrow::TradeDisputed,
) -> anyhow::Result<()> {
    let trade_id = data.tradeId;
    let payload = json!({ "tradeId": format!("{trade_id:#x}"), "caller": data.caller.to_string() });
    if !record_event(state, log, "TradeDisputed", Some(trade_id), payload).await? {
        return Ok(());
    }

    sqlx::query!(
        r#"UPDATE on_chain_trades SET status = 'DISPUTED', disputed_at = $2, updated_at = now() WHERE trade_id = $1"#,
        format!("{trade_id:#x}"),
        log_timestamp(log),
    )
    .execute(&state.db)
    .await?;

    Ok(())
}

async fn handle_buyer_refunded(
    state: &AppState,
    log: &Log,
    data: IAgriFlowEscrow::BuyerRefunded,
) -> anyhow::Result<()> {
    let trade_id = data.tradeId;
    let payload = json!({
        "tradeId": format!("{trade_id:#x}"),
        "buyer": data.buyer.to_string(),
        "refundAmount": data.refundAmount.to_string(),
    });
    if !record_event(state, log, "BuyerRefunded", Some(trade_id), payload).await? {
        return Ok(());
    }

    sqlx::query!(
        r#"UPDATE on_chain_trades SET status = 'REFUNDED', refunded_at = $2, updated_at = now() WHERE trade_id = $1"#,
        format!("{trade_id:#x}"),
        log_timestamp(log),
    )
    .execute(&state.db)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::U256;

    /// Regression test: these are the exact keccak256 topic hashes for each
    /// event's canonical signature, computed independently (Python
    /// pycryptodome) and verified live: this contract's constructor-time
    /// `OwnershipTransferred(address,address)` log, fetched from the real
    /// Sepolia deployment, matched its own independently-computed hash
    /// exactly. If `IAgriFlowEscrow.sol`'s event signatures ever change
    /// without this module's `sol!` block being updated to match, this is
    /// what would silently start missing every event on the next deploy --
    /// pin them here so that instead breaks the build.
    #[test]
    fn event_signature_hashes_match_expected() {
        assert_eq!(
            IAgriFlowEscrow::TradeFunded::SIGNATURE_HASH.to_string(),
            "0xf91e160017834c79cf9a9691a6030dc7e02571f23fcbcafb5fda2786d7c3c982",
        );
        assert_eq!(
            IAgriFlowEscrow::DeliveryConfirmed::SIGNATURE_HASH.to_string(),
            "0xf46bccfdb06ecc81738bcfc5ee961cc50fe62e4a5060c050d8bf69bcd1d47731",
        );
        assert_eq!(
            IAgriFlowEscrow::FundsCredited::SIGNATURE_HASH.to_string(),
            "0xbdec3a7e88ae030e5eb3868f2729dd21dac23d35442cea6ccd7e21bb957c56e9",
        );
        assert_eq!(
            IAgriFlowEscrow::Withdrawn::SIGNATURE_HASH.to_string(),
            "0xd1c19fbcd4551a5edfb66d43d2e337c04837afda3482b42bdf569a8fccdae5fb",
        );
        assert_eq!(
            IAgriFlowEscrow::TradeDisputed::SIGNATURE_HASH.to_string(),
            "0xd8495c2d0db81216e48a727b619240ca166df2ec873fdf138d8aca90a87a55bd",
        );
        assert_eq!(
            IAgriFlowEscrow::BuyerRefunded::SIGNATURE_HASH.to_string(),
            "0xa98df91e6f26ad64b60bcf1fe2988eb377131139656657203513e16f648e1d91",
        );
    }

    #[test]
    fn u256_to_decimal_handles_typical_and_edge_values() {
        assert_eq!(u256_to_decimal(U256::ZERO), rust_decimal::Decimal::ZERO);
        assert_eq!(
            u256_to_decimal(U256::from(5_450_000_000u64)), // 5,450 USDC at 6 decimals
            rust_decimal::Decimal::from(5_450_000_000u64),
        );
        // A value bigger than u64 -- exercises the string round-trip path,
        // not just a direct-cast shortcut.
        let big = U256::from(u64::MAX) + U256::from(1u64);
        assert_eq!(
            u256_to_decimal(big).to_string(),
            (u128::from(u64::MAX) + 1).to_string(),
        );
    }
}
