use sqlx::PgPool;

use crate::auth;

/// Seeds the four demo accounts from the README (same emails/password as the
/// original localStorage-backed frontend) plus one sample listing, so the
/// existing demo walkthrough keeps working against the real backend.
pub async fn run(db: &PgPool) -> anyhow::Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(db).await?;
    if count > 0 {
        tracing::info!("database already seeded, skipping");
        return Ok(());
    }

    tracing::info!("seeding demo accounts");
    let password_hash = auth::hash_password("agriflow123").map_err(|e| anyhow::anyhow!("{e}"))?;

    let users = [
        ("USR-BUY-001", "buyer@kolafarms.com", "Kola Farms Ltd", "buyer", "Kola Farms Ltd (Off-taker)", "+234 802 345 6789", "Ikeja, Lagos"),
        ("USR-SUP-001", "supplier@adeyemi.com", "Adeyemi Produce Co.", "supplier", "Adeyemi Produce Co. (Farmer Aggregator)", "+234 803 456 7890", "Ogbomoso, Oyo"),
        ("USR-LOG-001", "logistics@swifthaul.com", "SwiftHaul Logistics", "logistics", "SwiftHaul Logistics (Haulage Provider)", "+234 804 567 8901", "Lagos"),
        ("USR-ADM-001", "admin@agriflow.ng", "AgriFlow Operations", "admin", "AgriFlow Operations (Internal Admin)", "+234 805 678 9012", "Abuja"),
    ];

    for (id, email, name, role, org, phone, location) in users {
        sqlx::query(
            r#"INSERT INTO users (id, email, password_hash, name, role, organization_name, phone, location, verified, profile_complete)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, TRUE, TRUE)"#,
        )
        .bind(id)
        .bind(email)
        .bind(&password_hash)
        .bind(name)
        .bind(role)
        .bind(org)
        .bind(phone)
        .bind(location)
        .execute(db)
        .await?;
    }

    sqlx::query(
        r#"INSERT INTO supply_listings
             (id, supplier_id, supplier_name, supplier_verified, commodity, quantity, unit, quality_grade,
              price_per_unit, currency, location, availability_date, description, status)
           VALUES ('SUP-AGF-00337', 'USR-SUP-001', 'Adeyemi Produce Co.', TRUE, 'maize', 20, 'tonnes', 'A',
                   480000, 'NGN', 'Ogbomoso, Oyo, Nigeria', now(), 'Premium white maize, freshly harvested and dried to 12% moisture.', 'active')"#,
    )
    .execute(db)
    .await?;

    sqlx::query(
        r#"INSERT INTO demand_requests
             (id, buyer_id, buyer_name, commodity, quantity, unit, quality_grade, destination_location,
              required_by_date, indicative_budget, currency, notes, status)
           VALUES ('DEM-AGF-01043', 'USR-BUY-001', 'Kola Farms Ltd', 'maize', 12, 'tonnes', 'A', 'Ikeja, Lagos, Nigeria',
                   now() + interval '14 days', 5760000, 'NGN', 'Required for Q4 processing run.', 'open')"#,
    )
    .execute(db)
    .await?;

    tracing::info!("seed complete: 4 demo accounts (password: agriflow123)");
    Ok(())
}
