use std::{fs, path::Path};

use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

pub async fn init_db(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = database_url
        .parse::<SqliteConnectOptions>()?
        .create_if_missing(true);

    let filename = options.get_filename();
    if filename != Path::new(":memory:") {
        if let Some(parent) = filename.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(sqlx::Error::Io)?;
            }
        }
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("PRAGMA foreign_keys = ON")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS provider_connections (
            id TEXT PRIMARY KEY,
            provider TEXT NOT NULL,
            base_url TEXT NOT NULL,
            api_key TEXT,
            auth_type TEXT NOT NULL DEFAULT 'apikey',
            auth_header TEXT NOT NULL DEFAULT 'bearer',
            auth_prefix TEXT,
            extra_headers_json TEXT NOT NULL DEFAULT '{}',
            endpoint_paths_json TEXT NOT NULL DEFAULT '{}',
            stream_endpoint_paths_json TEXT NOT NULL DEFAULT '{}',
            model_prefix TEXT NOT NULL,
            name TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            priority INTEGER NOT NULL DEFAULT 0,
            default_model TEXT,
            supported_endpoints_json TEXT NOT NULL DEFAULT '["chat","messages","responses","ollama.chat","embeddings","images"]',
            rate_limit_protection INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            last_error_at TEXT,
            last_error_type TEXT,
            last_error_source TEXT,
            rate_limited_until TEXT,
            circuit_open_until TEXT,
            last_used_at TEXT,
            backoff_level INTEGER NOT NULL DEFAULT 0,
            consecutive_use_count INTEGER NOT NULL DEFAULT 0,
            test_status TEXT,
            account_routing_strategy TEXT NOT NULL DEFAULT 'priority',
            protocol_format TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

    let provider_columns: Vec<String> = sqlx::query("PRAGMA table_info(provider_connections)")
        .fetch_all(&pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect();
    let has_column = |name: &str| provider_columns.iter().any(|value| value == name);

    if !has_column("supported_endpoints_json") {
        sqlx::query(
            "ALTER TABLE provider_connections ADD COLUMN supported_endpoints_json TEXT NOT NULL DEFAULT '[\"chat\",\"messages\",\"responses\",\"ollama.chat\",\"embeddings\",\"images\"]'",
        )
        .execute(&pool)
        .await?;
    }
    if !has_column("rate_limit_protection") {
        sqlx::query(
            "ALTER TABLE provider_connections ADD COLUMN rate_limit_protection INTEGER NOT NULL DEFAULT 0",
        )
        .execute(&pool)
        .await?;
    }
    if !has_column("auth_type") {
        sqlx::query(
            "ALTER TABLE provider_connections ADD COLUMN auth_type TEXT NOT NULL DEFAULT 'apikey'",
        )
        .execute(&pool)
        .await?;
    }
    if !has_column("auth_header") {
        sqlx::query(
            "ALTER TABLE provider_connections ADD COLUMN auth_header TEXT NOT NULL DEFAULT 'bearer'",
        )
        .execute(&pool)
        .await?;
    }
    if !has_column("auth_prefix") {
        sqlx::query("ALTER TABLE provider_connections ADD COLUMN auth_prefix TEXT")
            .execute(&pool)
            .await?;
    }
    if !has_column("extra_headers_json") {
        sqlx::query(
            "ALTER TABLE provider_connections ADD COLUMN extra_headers_json TEXT NOT NULL DEFAULT '{}'",
        )
        .execute(&pool)
        .await?;
    }
    if !has_column("endpoint_paths_json") {
        sqlx::query(
            "ALTER TABLE provider_connections ADD COLUMN endpoint_paths_json TEXT NOT NULL DEFAULT '{}'",
        )
        .execute(&pool)
        .await?;
    }
    if !has_column("stream_endpoint_paths_json") {
        sqlx::query(
            "ALTER TABLE provider_connections ADD COLUMN stream_endpoint_paths_json TEXT NOT NULL DEFAULT '{}'",
        )
        .execute(&pool)
        .await?;
    }
    if !has_column("last_error_type") {
        sqlx::query("ALTER TABLE provider_connections ADD COLUMN last_error_type TEXT")
            .execute(&pool)
            .await?;
    }
    if !has_column("last_error_source") {
        sqlx::query("ALTER TABLE provider_connections ADD COLUMN last_error_source TEXT")
            .execute(&pool)
            .await?;
    }
    if !has_column("circuit_open_until") {
        sqlx::query("ALTER TABLE provider_connections ADD COLUMN circuit_open_until TEXT")
            .execute(&pool)
            .await?;
    }
    if !has_column("last_used_at") {
        sqlx::query("ALTER TABLE provider_connections ADD COLUMN last_used_at TEXT")
            .execute(&pool)
            .await?;
    }
    if !has_column("backoff_level") {
        sqlx::query(
            "ALTER TABLE provider_connections ADD COLUMN backoff_level INTEGER NOT NULL DEFAULT 0",
        )
        .execute(&pool)
        .await?;
    }
    if !has_column("consecutive_use_count") {
        sqlx::query(
            "ALTER TABLE provider_connections ADD COLUMN consecutive_use_count INTEGER NOT NULL DEFAULT 0",
        )
        .execute(&pool)
        .await?;
    }
    if !has_column("account_routing_strategy") {
        sqlx::query(
            "ALTER TABLE provider_connections ADD COLUMN account_routing_strategy TEXT NOT NULL DEFAULT 'priority'",
        )
        .execute(&pool)
        .await?;
    }
    if !has_column("protocol_format") {
        sqlx::query("ALTER TABLE provider_connections ADD COLUMN protocol_format TEXT")
            .execute(&pool)
            .await?;
    }

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS provider_accounts (
            id TEXT PRIMARY KEY,
            provider_connection_id TEXT NOT NULL,
            label TEXT,
            auth_mode TEXT NOT NULL,
            api_key TEXT,
            access_token TEXT,
            refresh_token TEXT,
            expires_at TEXT,
            scopes_json TEXT NOT NULL DEFAULT '[]',
            remote_account_id TEXT,
            remote_email TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            priority INTEGER NOT NULL DEFAULT 0,
            last_used_at TEXT,
            rate_limited_until TEXT,
            circuit_open_until TEXT,
            last_error TEXT,
            last_error_type TEXT,
            last_refresh_at TEXT,
            refresh_error TEXT,
            backoff_level INTEGER NOT NULL DEFAULT 0,
            consecutive_use_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY(provider_connection_id) REFERENCES provider_connections(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_provider_accounts_connection_priority ON provider_accounts(provider_connection_id, priority, created_at)",
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_provider_accounts_enabled ON provider_accounts(provider_connection_id, enabled)",
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS oauth_sessions (
            id TEXT PRIMARY KEY,
            state TEXT NOT NULL UNIQUE,
            provider_connection_id TEXT NOT NULL,
            provider_account_id TEXT,
            redirect_uri TEXT NOT NULL,
            code_verifier TEXT NOT NULL,
            scopes_json TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            consumed_at TEXT,
            FOREIGN KEY(provider_connection_id) REFERENCES provider_connections(id) ON DELETE CASCADE,
            FOREIGN KEY(provider_account_id) REFERENCES provider_accounts(id) ON DELETE SET NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_oauth_sessions_provider_connection ON oauth_sessions(provider_connection_id, created_at)",
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_oauth_sessions_expires_at ON oauth_sessions(expires_at)",
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS request_debug_logs (
            id TEXT PRIMARY KEY,
            request_id TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            provider_account_id TEXT,
            model TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            sequence_no INTEGER NOT NULL,
            raw_body TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY(provider_id) REFERENCES provider_connections(id) ON DELETE CASCADE,
            FOREIGN KEY(provider_account_id) REFERENCES provider_accounts(id) ON DELETE SET NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS response_debug_logs (
            id TEXT PRIMARY KEY,
            request_id TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            provider_account_id TEXT,
            model TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            sequence_no INTEGER NOT NULL DEFAULT 0,
            upstream_status INTEGER NOT NULL,
            raw_body TEXT NOT NULL,
            reasoning_summary_json TEXT,
            obfuscation_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            FOREIGN KEY(provider_id) REFERENCES provider_connections(id) ON DELETE CASCADE,
            FOREIGN KEY(provider_account_id) REFERENCES provider_accounts(id) ON DELETE SET NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

    let response_debug_log_columns: Vec<String> =
        sqlx::query("PRAGMA table_info(response_debug_logs)")
            .fetch_all(&pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .collect();
    if !response_debug_log_columns
        .iter()
        .any(|value| value == "sequence_no")
    {
        sqlx::query(
            "ALTER TABLE response_debug_logs ADD COLUMN sequence_no INTEGER NOT NULL DEFAULT 0",
        )
        .execute(&pool)
        .await?;
    }

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_request_debug_logs_request_id ON request_debug_logs(request_id, sequence_no, created_at)",
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_request_debug_logs_provider_created_at ON request_debug_logs(provider_id, created_at)",
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_response_debug_logs_request_id ON response_debug_logs(request_id, sequence_no, created_at)",
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_response_debug_logs_provider_created_at ON response_debug_logs(provider_id, created_at)",
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS combos (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            strategy TEXT NOT NULL,
            models_json TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS model_aliases (
            alias TEXT PRIMARY KEY,
            target TEXT NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS api_keys (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            key_hash TEXT NOT NULL,
            key_prefix TEXT NOT NULL,
            allowed_models_json TEXT NOT NULL DEFAULT '["*"]',
            is_active INTEGER NOT NULL DEFAULT 1,
            last_used_at TEXT,
            usage_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash)")
        .execute(&pool)
        .await?;

    Ok(pool)
}


#[cfg(test)]
mod tests {
    use super::init_db;
    use sqlx::Row;

    #[tokio::test]
    async fn test_init_db_creates_request_debug_logs_and_response_sequence() {
        let pool = init_db("sqlite::memory:").await.unwrap();

        let request_columns = sqlx::query("PRAGMA table_info(request_debug_logs)")
            .fetch_all(&pool)
            .await
            .unwrap();
        let request_column_names: Vec<String> = request_columns
            .into_iter()
            .map(|row| row.try_get::<String, _>("name").unwrap())
            .collect();
        assert!(request_column_names.iter().any(|name| name == "sequence_no"));
        assert!(request_column_names.iter().any(|name| name == "raw_body"));

        let response_columns = sqlx::query("PRAGMA table_info(response_debug_logs)")
            .fetch_all(&pool)
            .await
            .unwrap();
        let response_column_names: Vec<String> = response_columns
            .into_iter()
            .map(|row| row.try_get::<String, _>("name").unwrap())
            .collect();
        assert!(response_column_names.iter().any(|name| name == "sequence_no"));
    }
}
