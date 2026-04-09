use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};

pub async fn init_db(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(database_url)
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
        sqlx::query("ALTER TABLE provider_connections ADD COLUMN backoff_level INTEGER NOT NULL DEFAULT 0")
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
    if !has_column("protocol_format") {
        sqlx::query("ALTER TABLE provider_connections ADD COLUMN protocol_format TEXT")
            .execute(&pool)
            .await?;
    }

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

    // API keys table
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

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash)",
    )
    .execute(&pool)
    .await?;

    Ok(pool)
}
