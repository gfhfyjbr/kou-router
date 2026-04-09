use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    models::{
        default_supported_endpoints, supports_endpoint, Combo, ComboStrategy, EndpointKind,
        ModelAlias, NewCombo, NewProviderConnection, OpenAiModel, ProviderConnection,
        SettingsPayload,
    },
};

#[derive(Clone)]
pub struct SqliteRepository {
    pool: SqlitePool,
}

impl SqliteRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list_provider_connections(&self) -> AppResult<Vec<ProviderConnection>> {
        let rows = sqlx::query(
            r#"
            SELECT id, provider, base_url, api_key, auth_type, auth_header, auth_prefix, extra_headers_json,
                   endpoint_paths_json, stream_endpoint_paths_json, model_prefix, name, enabled, priority,
                   default_model, supported_endpoints_json, rate_limit_protection, last_error, last_error_at,
                   last_error_type, last_error_source, rate_limited_until, circuit_open_until, last_used_at,
                   backoff_level, consecutive_use_count, test_status, created_at, updated_at, protocol_format
            FROM provider_connections
            ORDER BY priority ASC, provider ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_provider).collect()
    }

    pub async fn create_provider_connection(
        &self,
        input: NewProviderConnection,
    ) -> AppResult<ProviderConnection> {
        let now = Utc::now();
        let id = Uuid::new_v4().to_string();
        let priority = match input.priority {
            Some(value) => value,
            None => self.next_provider_priority(&input.provider).await?,
        };
        let model_prefix = input
            .model_prefix
            .clone()
            .unwrap_or_else(|| input.provider.clone());
        let supported_endpoints = input
            .supported_endpoints
            .clone()
            .unwrap_or_else(default_supported_endpoints);
        let extra_headers = input.extra_headers.clone();
        let endpoint_paths = input.endpoint_paths.clone().unwrap_or_default();
        let stream_endpoint_paths = input.stream_endpoint_paths.clone().unwrap_or_default();

        sqlx::query(
            r#"
            INSERT INTO provider_connections (
                id, provider, base_url, api_key, auth_type, auth_header, auth_prefix, extra_headers_json,
                endpoint_paths_json, stream_endpoint_paths_json, model_prefix, name, enabled, priority, default_model,
                supported_endpoints_json, rate_limit_protection, last_error, last_error_at, last_error_type,
                last_error_source, rate_limited_until, circuit_open_until, last_used_at, backoff_level,
                consecutive_use_count, test_status, created_at, updated_at, protocol_format
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0, 0, NULL, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&input.provider)
        .bind(&input.base_url)
        .bind(&input.api_key)
        .bind(&input.auth_type)
        .bind(&input.auth_header)
        .bind(&input.auth_prefix)
        .bind(serde_json::to_string(&extra_headers)?)
        .bind(serde_json::to_string(&endpoint_paths)?)
        .bind(serde_json::to_string(&stream_endpoint_paths)?)
        .bind(&model_prefix)
        .bind(&input.name)
        .bind(if input.enabled { 1_i64 } else { 0_i64 })
        .bind(priority)
        .bind(&input.default_model)
        .bind(serde_json::to_string(&supported_endpoints)?)
        .bind(if input.rate_limit_protection { 1_i64 } else { 0_i64 })
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(&input.protocol_format)
        .execute(&self.pool)
        .await?;

        Ok(ProviderConnection {
            id,
            provider: input.provider,
            base_url: input.base_url,
            api_key: input.api_key,
            auth_type: input.auth_type,
            auth_header: input.auth_header,
            auth_prefix: input.auth_prefix,
            extra_headers,
            endpoint_paths,
            stream_endpoint_paths,
            model_prefix,
            name: input.name,
            enabled: input.enabled,
            priority,
            default_model: input.default_model,
            supported_endpoints,
            rate_limit_protection: input.rate_limit_protection,
            last_error: None,
            last_error_at: None,
            last_error_type: None,
            last_error_source: None,
            rate_limited_until: None,
            circuit_open_until: None,
            last_used_at: None,
            backoff_level: 0,
            consecutive_use_count: 0,
            test_status: None,
            created_at: now,
            updated_at: now,
            protocol_format: input.protocol_format,
        })
    }

    pub async fn mark_provider_failure(&self, provider_id: &str, message: &str) -> AppResult<()> {
        let now = Utc::now();
        let lowered = message.to_ascii_lowercase();
        let is_rate_limit = lowered.contains("rate limit") || lowered.contains("quota") || lowered.contains("429");
        let rate_limited_until = if is_rate_limit {
            Some((now + chrono::Duration::seconds(30)).to_rfc3339())
        } else {
            None
        };
        let circuit_open_until = Some((now + chrono::Duration::seconds(15)).to_rfc3339());
        let error_type = if is_rate_limit { "rate_limit" } else { "upstream_error" };

        sqlx::query(
            r#"
            UPDATE provider_connections
            SET last_error = ?, last_error_at = ?, last_error_type = ?, last_error_source = 'gateway',
                rate_limited_until = COALESCE(?, rate_limited_until), circuit_open_until = ?,
                test_status = 'error', updated_at = ?, backoff_level = backoff_level + 1,
                consecutive_use_count = 0
            WHERE id = ?
            "#,
        )
        .bind(message)
        .bind(now.to_rfc3339())
        .bind(error_type)
        .bind(rate_limited_until)
        .bind(circuit_open_until)
        .bind(now.to_rfc3339())
        .bind(provider_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_provider_success(&self, provider_id: &str) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            UPDATE provider_connections
            SET last_error = NULL, last_error_at = NULL, last_error_type = NULL, last_error_source = NULL,
                rate_limited_until = NULL, circuit_open_until = NULL, test_status = 'ok', updated_at = ?,
                last_used_at = ?, backoff_level = 0, consecutive_use_count = consecutive_use_count + 1
            WHERE id = ?
            "#,
        )
        .bind(&now)
        .bind(&now)
        .bind(provider_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn next_provider_priority(&self, provider: &str) -> AppResult<i64> {
        let row = sqlx::query(
            "SELECT COALESCE(MAX(priority), -1) + 1 AS next_priority FROM provider_connections WHERE provider = ?",
        )
        .bind(provider)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.try_get::<i64, _>("next_priority")?)
    }

    pub async fn list_combos(&self) -> AppResult<Vec<Combo>> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, strategy, models_json, enabled, created_at, updated_at
            FROM combos
            ORDER BY name ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_combo).collect()
    }

    pub async fn create_combo(&self, input: NewCombo) -> AppResult<Combo> {
        if input.models.is_empty() {
            return Err(AppError::BadRequest("combo must include at least one model".into()));
        }
        let now = Utc::now();
        let id = Uuid::new_v4().to_string();
        let strategy = strategy_as_str(&input.strategy);
        let models_json = serde_json::to_string(&input.models)?;

        sqlx::query(
            r#"
            INSERT INTO combos (id, name, strategy, models_json, enabled, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&input.name)
        .bind(strategy)
        .bind(models_json)
        .bind(if input.enabled { 1_i64 } else { 0_i64 })
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(Combo {
            id,
            name: input.name,
            strategy: input.strategy,
            models: input.models,
            enabled: input.enabled,
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn find_combo_by_name(&self, name: &str) -> AppResult<Option<Combo>> {
        let row = sqlx::query(
            r#"
            SELECT id, name, strategy, models_json, enabled, created_at, updated_at
            FROM combos WHERE name = ?
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        row.map(map_combo).transpose()
    }

    pub async fn list_aliases(&self) -> AppResult<Vec<ModelAlias>> {
        let rows = sqlx::query("SELECT alias, target FROM model_aliases ORDER BY alias ASC")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ModelAlias {
                    alias: row.try_get("alias")?,
                    target: row.try_get("target")?,
                })
            })
            .collect()
    }

    pub async fn upsert_alias(&self, alias: &str, target: &str) -> AppResult<ModelAlias> {
        sqlx::query(
            r#"
            INSERT INTO model_aliases (alias, target) VALUES (?, ?)
            ON CONFLICT(alias) DO UPDATE SET target = excluded.target
            "#,
        )
        .bind(alias)
        .bind(target)
        .execute(&self.pool)
        .await?;

        Ok(ModelAlias {
            alias: alias.to_string(),
            target: target.to_string(),
        })
    }

    pub async fn resolve_alias(&self, model: &str) -> AppResult<String> {
        let row = sqlx::query("SELECT target FROM model_aliases WHERE alias = ?")
            .bind(model)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => Ok(row.try_get("target")?),
            None => Ok(model.to_string()),
        }
    }

    pub async fn get_openai_models_catalog(&self) -> AppResult<Vec<OpenAiModel>> {
        let providers = self.list_provider_connections().await?;
        let combos = self.list_combos().await?;
        let timestamp = Utc::now().timestamp();

        let mut result = Vec::new();
        for combo in combos.into_iter().filter(|combo| combo.enabled) {
            result.push(OpenAiModel {
                id: combo.name,
                object: "model".to_string(),
                owned_by: "combo".to_string(),
                created: Some(timestamp),
                kind: None,
                dimensions: None,
                supported_sizes: None,
            });
        }

        for provider in providers.into_iter().filter(|provider| provider.enabled) {
            let model_id = provider
                .default_model
                .clone()
                .unwrap_or_else(|| format!("{}/default", provider.model_prefix));
            result.push(OpenAiModel {
                id: model_id,
                object: "model".to_string(),
                owned_by: provider.provider,
                created: Some(timestamp),
                kind: None,
                dimensions: None,
                supported_sizes: None,
            });
        }

        result.sort_by(|a, b| a.id.cmp(&b.id));
        result.dedup_by(|a, b| a.id == b.id);
        Ok(result)
    }

    pub async fn get_models_catalog_for_endpoint(
        &self,
        endpoint: EndpointKind,
    ) -> AppResult<Vec<OpenAiModel>> {
        let providers = self.list_provider_connections().await?;
        let combos = self.list_combos().await?;
        let timestamp = Utc::now().timestamp();

        let mut result = Vec::new();
        if endpoint.is_chat_family() {
            for combo in combos.into_iter().filter(|combo| combo.enabled) {
                result.push(OpenAiModel {
                    id: combo.name,
                    object: "model".to_string(),
                    owned_by: "combo".to_string(),
                    created: Some(timestamp),
                    kind: Some(endpoint.capability().to_string()),
                    dimensions: None,
                    supported_sizes: None,
                });
            }
        }

        for provider in providers
            .into_iter()
            .filter(|provider| provider.enabled && supports_endpoint(&provider.supported_endpoints, endpoint))
        {
            let model_id = provider
                .default_model
                .clone()
                .unwrap_or_else(|| format!("{}/default", provider.model_prefix));
            let metadata = infer_model_metadata(endpoint, &model_id);
            result.push(OpenAiModel {
                id: model_id,
                object: "model".to_string(),
                owned_by: provider.provider,
                created: Some(timestamp),
                kind: Some(endpoint.capability().to_string()),
                dimensions: metadata.0,
                supported_sizes: metadata.1,
            });
        }

        result.sort_by(|a, b| a.id.cmp(&b.id));
        result.dedup_by(|a, b| a.id == b.id);
        Ok(result)
    }

    pub async fn get_settings(&self) -> AppResult<serde_json::Value> {
        let rows = sqlx::query("SELECT key, value_json FROM settings ORDER BY key ASC")
            .fetch_all(&self.pool)
            .await?;
        let mut map = serde_json::Map::new();
        for row in rows {
            let key: String = row.try_get("key")?;
            let value_json: String = row.try_get("value_json")?;
            let value = serde_json::from_str(&value_json)?;
            map.insert(key, value);
        }
        Ok(serde_json::Value::Object(map))
    }

    pub async fn put_settings(&self, payload: &SettingsPayload) -> AppResult<serde_json::Value> {
        let now = Utc::now().to_rfc3339();
        let object = payload
            .values
            .as_object()
            .ok_or_else(|| AppError::BadRequest("settings payload must be a JSON object".into()))?;

        for (key, value) in object {
            sqlx::query(
                r#"
                INSERT INTO settings (key, value_json, updated_at) VALUES (?, ?, ?)
                ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at
                "#,
            )
            .bind(key)
            .bind(serde_json::to_string(value)?)
            .bind(&now)
            .execute(&self.pool)
            .await?;
        }

        self.get_settings().await
    }

    // ── Auth helpers ────────────────────────────────────────────────

    pub async fn get_setting_string(&self, key: &str) -> AppResult<String> {
        let row = sqlx::query("SELECT value_json FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(row) => {
                let raw: String = row.try_get("value_json")?;
                let value: serde_json::Value = serde_json::from_str(&raw)?;
                Ok(value.as_str().unwrap_or(&raw.trim_matches('"')).to_string())
            }
            None => Err(AppError::NotFound(format!("setting '{key}' not found"))),
        }
    }

    pub async fn get_setting_bool(&self, key: &str) -> AppResult<bool> {
        let row = sqlx::query("SELECT value_json FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(row) => {
                let raw: String = row.try_get("value_json")?;
                Ok(raw == "true" || raw == "1")
            }
            None => Ok(false),
        }
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO settings (key, value_json, updated_at) VALUES (?, ?, ?)
            ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at
            "#,
        )
        .bind(key)
        .bind(value)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_api_key(
        &self,
        id: &str,
        name: &str,
        key_hash: &str,
        key_prefix: &str,
        allowed_models: &[String],
    ) -> AppResult<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let models_json = serde_json::to_string(allowed_models)?;
        sqlx::query(
            r#"
            INSERT INTO api_keys (id, name, key_hash, key_prefix, allowed_models_json, is_active, usage_count, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, 1, 0, ?, ?)
            "#,
        )
        .bind(id)
        .bind(name)
        .bind(key_hash)
        .bind(key_prefix)
        .bind(&models_json)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_api_key_by_hash(&self, key_hash: &str) -> AppResult<Option<crate::auth::ApiKeyRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, name, key_prefix, allowed_models_json, is_active, last_used_at, usage_count, created_at, updated_at
            FROM api_keys WHERE key_hash = ?
            "#,
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let models_json: String = row.try_get("allowed_models_json")?;
                let allowed_models: Vec<String> = serde_json::from_str(&models_json).unwrap_or_else(|_| vec!["*".to_string()]);
                Ok(Some(crate::auth::ApiKeyRecord {
                    id: row.try_get("id")?,
                    name: row.try_get("name")?,
                    key_prefix: row.try_get("key_prefix")?,
                    allowed_models,
                    is_active: row.try_get::<i64, _>("is_active")? != 0,
                    last_used_at: parse_optional_dt(row.try_get("last_used_at")?)?,
                    usage_count: row.try_get("usage_count")?,
                    created_at: parse_dt(row.try_get("created_at")?)?,
                    updated_at: parse_dt(row.try_get("updated_at")?)?,
                }))
            }
            None => Ok(None),
        }
    }

    pub async fn list_api_keys(&self) -> AppResult<Vec<crate::auth::ApiKeyRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, key_prefix, allowed_models_json, is_active, last_used_at, usage_count, created_at, updated_at
            FROM api_keys ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                let models_json: String = row.try_get("allowed_models_json")?;
                let allowed_models: Vec<String> = serde_json::from_str(&models_json).unwrap_or_else(|_| vec!["*".to_string()]);
                Ok(crate::auth::ApiKeyRecord {
                    id: row.try_get("id")?,
                    name: row.try_get("name")?,
                    key_prefix: row.try_get("key_prefix")?,
                    allowed_models,
                    is_active: row.try_get::<i64, _>("is_active")? != 0,
                    last_used_at: parse_optional_dt(row.try_get("last_used_at")?)?,
                    usage_count: row.try_get("usage_count")?,
                    created_at: parse_dt(row.try_get("created_at")?)?,
                    updated_at: parse_dt(row.try_get("updated_at")?)?,
                })
            })
            .collect()
    }

    pub async fn touch_api_key_usage(&self, key_id: &str) -> AppResult<()> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE api_keys SET last_used_at = ?, usage_count = usage_count + 1, updated_at = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(&now)
        .bind(key_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn revoke_api_key(&self, key_id: &str) -> AppResult<bool> {
        let result = sqlx::query("DELETE FROM api_keys WHERE id = ?")
            .bind(key_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

fn map_provider(row: sqlx::sqlite::SqliteRow) -> AppResult<ProviderConnection> {
    let supported_endpoints_json: String = row.try_get("supported_endpoints_json")?;
    let supported_endpoints: Vec<String> = serde_json::from_str(&supported_endpoints_json)
        .unwrap_or_else(|_| default_supported_endpoints());
    let extra_headers_json: String = row.try_get("extra_headers_json")?;
    let extra_headers = serde_json::from_str(&extra_headers_json).unwrap_or_default();
    let endpoint_paths_json: String = row.try_get("endpoint_paths_json")?;
    let endpoint_paths = serde_json::from_str(&endpoint_paths_json).unwrap_or_default();
    let stream_endpoint_paths_json: String = row.try_get("stream_endpoint_paths_json")?;
    let stream_endpoint_paths = serde_json::from_str(&stream_endpoint_paths_json).unwrap_or_default();
    Ok(ProviderConnection {
        id: row.try_get("id")?,
        provider: row.try_get("provider")?,
        base_url: row.try_get("base_url")?,
        api_key: row.try_get("api_key")?,
        auth_type: row.try_get("auth_type")?,
        auth_header: row.try_get("auth_header")?,
        auth_prefix: row.try_get("auth_prefix")?,
        extra_headers,
        endpoint_paths,
        stream_endpoint_paths,
        model_prefix: row.try_get("model_prefix")?,
        name: row.try_get("name")?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        priority: row.try_get("priority")?,
        default_model: row.try_get("default_model")?,
        supported_endpoints,
        rate_limit_protection: row.try_get::<i64, _>("rate_limit_protection")? != 0,
        last_error: row.try_get("last_error")?,
        last_error_at: parse_optional_dt(row.try_get("last_error_at")?)?,
        last_error_type: row.try_get("last_error_type")?,
        last_error_source: row.try_get("last_error_source")?,
        rate_limited_until: parse_optional_dt(row.try_get("rate_limited_until")?)?,
        circuit_open_until: parse_optional_dt(row.try_get("circuit_open_until")?)?,
        last_used_at: parse_optional_dt(row.try_get("last_used_at")?)?,
        backoff_level: row.try_get("backoff_level")?,
        consecutive_use_count: row.try_get("consecutive_use_count")?,
        test_status: row.try_get("test_status")?,
        created_at: parse_dt(row.try_get("created_at")?)?,
        updated_at: parse_dt(row.try_get("updated_at")?)?,
        protocol_format: row.try_get("protocol_format").ok().flatten(),
    })
}

fn map_combo(row: sqlx::sqlite::SqliteRow) -> AppResult<Combo> {
    let strategy_raw: String = row.try_get("strategy")?;
    let strategy = match strategy_raw.as_str() {
        "round-robin" => ComboStrategy::RoundRobin,
        _ => ComboStrategy::Priority,
    };
    let models_json: String = row.try_get("models_json")?;
    Ok(Combo {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        strategy,
        models: serde_json::from_str(&models_json)?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        created_at: parse_dt(row.try_get("created_at")?)?,
        updated_at: parse_dt(row.try_get("updated_at")?)?,
    })
}

fn parse_dt(value: String) -> AppResult<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(&value)
        .map_err(|err| AppError::BadRequest(format!("invalid datetime in db: {err}")))?
        .with_timezone(&Utc))
}

fn parse_optional_dt(value: Option<String>) -> AppResult<Option<DateTime<Utc>>> {
    value.map(parse_dt).transpose()
}

fn strategy_as_str(strategy: &ComboStrategy) -> &'static str {
    match strategy {
        ComboStrategy::Priority => "priority",
        ComboStrategy::RoundRobin => "round-robin",
    }
}

fn infer_model_metadata(endpoint: EndpointKind, model_id: &str) -> (Option<i64>, Option<Vec<String>>) {
    match endpoint {
        EndpointKind::Embeddings => {
            let lowered = model_id.to_ascii_lowercase();
            let dimensions = if lowered.contains("3072") {
                Some(3072)
            } else if lowered.contains("1536") {
                Some(1536)
            } else if lowered.contains("1024") {
                Some(1024)
            } else {
                None
            };
            (dimensions, None)
        }
        EndpointKind::ImagesGenerations => (
            None,
            Some(vec![
                "256x256".to_string(),
                "512x512".to_string(),
                "1024x1024".to_string(),
            ]),
        ),
        EndpointKind::MusicGenerations => (
            None,
            Some(vec![
                "wav".to_string(),
                "mp3".to_string(),
            ]),
        ),
        EndpointKind::VideosGenerations => (
            None,
            Some(vec![
                "mp4".to_string(),
                "webp".to_string(),
            ]),
        ),
        EndpointKind::Search => (
            None,
            Some(vec![
                "web".to_string(),
                "news".to_string(),
                "academic".to_string(),
            ]),
        ),
        EndpointKind::AudioSpeech => (
            None,
            Some(vec![
                "mp3".to_string(),
                "wav".to_string(),
                "opus".to_string(),
            ]),
        ),
        EndpointKind::AudioTranscriptions => (
            None,
            Some(vec![
                "json".to_string(),
                "text".to_string(),
                "srt".to_string(),
                "verbose_json".to_string(),
            ]),
        ),
        _ => (None, None),
    }
}
