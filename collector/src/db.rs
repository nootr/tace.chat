use rusqlite::{params, Connection, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use tace_lib::metrics::NetworkMetrics;

#[derive(Debug, Serialize, Deserialize)]
pub struct StoredNetworkMetrics {
    pub timestamp: u64,
    pub node_churn_rate: f64,
    pub routing_table_health: f64,
    pub operation_success_rate: f64,
    pub operation_latency_millis: u64,
    pub message_type_ratio: f64,
    pub local_key_count: u64,
    pub total_network_keys_estimate: f64,
    pub network_size_estimate: f64,
}

pub fn init_db(db_path: &str) -> SqliteResult<Connection> {
    let conn = Connection::open(db_path)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS metrics (
            timestamp INTEGER PRIMARY KEY,
            node_churn_rate REAL,
            routing_table_health REAL,
            operation_success_rate REAL,
            operation_latency_millis INTEGER,
            message_type_ratio REAL,
            local_key_count INTEGER,
            total_network_keys_estimate REAL,
            network_size_estimate REAL
        )",
        [],
    )?;
    Ok(conn)
}

pub fn insert_metrics(conn: &Connection, metrics: &NetworkMetrics) -> SqliteResult<()> {
    conn.execute(
        "INSERT INTO metrics (
            timestamp, node_churn_rate, routing_table_health, operation_success_rate,
            operation_latency_millis, message_type_ratio, local_key_count,
            total_network_keys_estimate, network_size_estimate
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            metrics.node_churn_rate,
            metrics.routing_table_health,
            metrics.operation_success_rate,
            metrics.operation_latency.as_millis() as u64,
            metrics.message_type_ratio,
            metrics.local_key_count,
            metrics.total_network_keys_estimate,
            metrics.network_size_estimate,
        ],
    )?;
    Ok(())
}

pub fn get_latest_metrics(conn: &Connection) -> SqliteResult<Vec<StoredNetworkMetrics>> {
    let mut stmt = conn.prepare("SELECT * FROM metrics ORDER BY timestamp DESC LIMIT 1000")?;
    let metrics = stmt
        .query_map([], |row| {
            Ok(StoredNetworkMetrics {
                timestamp: row.get(0)?,
                node_churn_rate: row.get(1)?,
                routing_table_health: row.get(2)?,
                operation_success_rate: row.get(3)?,
                operation_latency_millis: row.get(4)?,
                message_type_ratio: row.get(5)?,
                local_key_count: row.get(6)?,
                total_network_keys_estimate: row.get(7)?,
                network_size_estimate: row.get(8)?,
            })
        })?
        .filter_map(Result::ok)
        .collect();

    Ok(metrics)
}
