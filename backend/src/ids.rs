use rand::RngExt;

fn rand_digits(n: u32) -> String {
    let mut rng = rand::rng();
    let low = 10u32.pow(n - 1);
    let high = 10u32.pow(n) - 1;
    rng.random_range(low..=high).to_string()
}

fn ts_suffix() -> String {
    let millis = chrono::Utc::now().timestamp_millis();
    format!("{}", millis % 1_000_000)
}

pub fn user_id(role: &str) -> String {
    let prefix = match role {
        "buyer" => "USR-BUY",
        "supplier" => "USR-SUP",
        "logistics" => "USR-LOG",
        "admin" => "USR-ADM",
        _ => "USR-UNK",
    };
    format!("{prefix}-{}", ts_suffix())
}

pub fn supply_id() -> String {
    format!("SUP-AGF-{}", ts_suffix())
}

pub fn demand_id() -> String {
    format!("DEM-AGF-{}", ts_suffix())
}

pub fn match_id(demand_id: &str, listing_id: &str) -> String {
    format!("MATCH-{demand_id}-{listing_id}")
}

pub fn transaction_id() -> String {
    format!("TXN-AGF-{}", rand_digits(5))
}

pub fn payment_id(transaction_id: &str) -> String {
    let n = transaction_id.split('-').next_back().unwrap_or("00000");
    format!("PAY-AGF-{n}")
}

pub fn logistics_job_id() -> String {
    format!("LOG-AGF-{}", rand_digits(5))
}

pub fn dispute_id() -> String {
    format!("DIS-AGF-{}", ts_suffix())
}

pub fn notification_id() -> String {
    format!("NOTIF-{}-{}", chrono::Utc::now().timestamp_millis(), rand_digits(5))
}

pub fn audit_id() -> String {
    format!("AUD-{}-{}", chrono::Utc::now().timestamp_millis(), rand_digits(5))
}

pub fn provider_reference() -> String {
    let date = chrono::Utc::now().format("%Y%m%d");
    let mut rng = rand::rng();
    let rand_num: u32 = rng.random_range(100000..900000);
    format!("AF-PAY-{date}-{rand_num}")
}
