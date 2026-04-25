use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Generates Claude Code fingerprints and headers for Anthropic API requests.
/// When a non-Claude-Code client sends a request that gets routed to an Anthropic
/// upstream provider, this module injects the necessary headers, attribution,
/// and metadata to mimic a Claude Code client.
#[derive(Clone, Debug)]
pub struct ClaudeCodeFingerprint {
    /// Claude Code CLI version (default "2.2.0", configurable via KOU_CC_VERSION)
    version: String,
    /// 64 hex-char device identifier, stable per machine (persisted to ~/.config/kou-router/device_id)
    device_id: String,
    /// UUID v4 session identifier, generated once per instance
    session_id: String,
    /// Whether fingerprint injection is enabled (KOU_CC_FINGERPRINT env var)
    enabled: bool,
    /// Entrypoint identifier (default "cli", configurable via KOU_CC_ENTRYPOINT)
    entrypoint: String,
    /// User type for User-Agent (default "external", configurable via KOU_CC_USER_TYPE)
    user_type: String,
    /// Optional workload tag for billing attribution (configurable via KOU_CC_WORKLOAD)
    workload: Option<String>,
    /// Optional agent-sdk version for User-Agent (configurable via KOU_CC_AGENT_SDK_VERSION)
    agent_sdk_version: Option<String>,
    /// Optional client-app identifier for User-Agent (configurable via KOU_CC_CLIENT_APP)
    client_app: Option<String>,
}

const FINGERPRINT_SALT: &str = "59cf53e54c78";
const FINGERPRINT_INDICES: [usize; 3] = [4, 7, 20];

impl ClaudeCodeFingerprint {
    pub fn new() -> Self {
        let version = std::env::var("KOU_CC_VERSION").unwrap_or_else(|_| "2.2.0".to_string());
        let enabled = std::env::var("KOU_CC_FINGERPRINT")
            .map(|v| v != "false" && v != "0")
            .unwrap_or(true);
        let entrypoint = std::env::var("KOU_CC_ENTRYPOINT").unwrap_or_else(|_| "cli".to_string());
        let user_type =
            std::env::var("KOU_CC_USER_TYPE").unwrap_or_else(|_| "external".to_string());
        let workload = std::env::var("KOU_CC_WORKLOAD")
            .ok()
            .filter(|v| !v.is_empty());
        let agent_sdk_version = std::env::var("KOU_CC_AGENT_SDK_VERSION")
            .ok()
            .filter(|v| !v.is_empty());
        let client_app = std::env::var("KOU_CC_CLIENT_APP")
            .ok()
            .filter(|v| !v.is_empty());

        // Device ID: stable per-machine, persisted to ~/.config/kou-router/device_id.
        // Can be overridden via KOU_CC_DEVICE_ID env var.
        // Falls back to random if persistence fails.
        let device_id = get_or_create_device_id();

        let session_id = Uuid::new_v4().to_string();

        Self {
            version,
            device_id,
            session_id,
            enabled,
            entrypoint,
            user_type,
            workload,
            agent_sdk_version,
            client_app,
        }
    }

    /// Compute the 3-char hex fingerprint from the first user message text.
    ///
    /// Algorithm:
    /// - Extract chars at indices [4, 7, 20] from the first user message text
    /// - If the message is shorter than an index, use '0' as fallback
    /// - Concatenate: "{SALT}{chars}{VERSION}"
    /// - SHA256 hash and take first 3 hex characters
    pub fn compute_fingerprint(&self, messages: &Value) -> String {
        let msg_text = extract_first_user_message_text(messages);
        let chars: String = FINGERPRINT_INDICES
            .iter()
            .map(|&idx| msg_text.chars().nth(idx).unwrap_or('0'))
            .collect();

        let input = format!("{}{}{}", FINGERPRINT_SALT, chars, self.version);
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let result = hasher.finalize();
        hex_encode(&result)[..3].to_string()
    }

    /// Generate the attribution header string for the system prompt.
    ///
    /// Format: `x-anthropic-billing-header: cc_version={VER}.{FP}; cc_entrypoint={EP}; cch=00000;[ cc_workload={TAG};]`
    ///
    /// The `cch=00000` is a native client attestation placeholder. In real Claude Code,
    /// Bun's HTTP stack overwrites the zeros with a computed hash. We include the
    /// placeholder for format compatibility.
    pub fn attribution_header(&self, messages: &Value) -> String {
        let fingerprint = self.compute_fingerprint(messages);
        let workload_pair = self
            .workload
            .as_ref()
            .map(|w| format!(" cc_workload={w};"))
            .unwrap_or_default();
        format!(
            "x-anthropic-billing-header: cc_version={ver}.{fp}; cc_entrypoint={ep}; cch=00000;{wl}",
            ver = self.version,
            fp = fingerprint,
            ep = self.entrypoint,
            wl = workload_pair,
        )
    }

    /// Generate the metadata.user_id JSON string.
    ///
    /// Returns a JSON-encoded string (not an object!) containing:
    /// - device_id: 64 hex chars
    /// - account_uuid: empty string (no OAuth)
    /// - session_id: same as X-Claude-Code-Session-Id
    pub fn metadata_user_id(&self) -> String {
        let obj = json!({
            "device_id": self.device_id,
            "account_uuid": "",
            "session_id": self.session_id,
        });
        // Return as a JSON string, not an object
        serde_json::to_string(&obj).unwrap_or_default()
    }

    /// Return the beta headers string for the given model.
    ///
    /// `is_first_party` indicates whether the target provider is Anthropic 1P API.
    /// Some betas are only valid on 1P and sending them to 3P providers may cause errors.
    ///
    /// Betas included:
    /// - claude-code-20250219 (non-Haiku)
    /// - interleaved-thinking-2025-05-14 (non-claude-3-* models)
    /// - redact-thinking-2026-02-12 (1P only, ISP-capable models)
    /// - prompt-caching-scope-2026-01-05 (1P only)
    /// - effort-2025-11-24 (always)
    /// - context-management-2025-06-27 (Claude 4+, 1P only)
    /// - fast-mode-2026-02-01 (1P only)
    /// - afk-mode-2026-01-31 (Claude 4+, 1P only)
    /// - task-budgets-2026-03-13 (1P only)
    /// - advisor-tool-2026-03-01 (1P only)
    /// - context-1m-2025-08-07 (1P only, Claude 4+ with 1M context)
    /// - structured-outputs-2025-12-15 (Claude 4.5+, 1P only)
    /// - advanced-tool-use-2025-11-20 (1P only, tool search)
    /// - tool-search-tool-2025-10-19 (3P only, tool search)
    /// - cli-internal-2026-02-09 (1P only, ant-internal, gated by KOU_CC_ANT_INTERNAL)
    /// - oauth-2025-04-20 (1P only, OAuth subscribers, gated by KOU_CC_OAUTH)
    pub fn beta_headers(&self, model: &str, is_first_party: bool) -> String {
        let model_lower = model.to_lowercase();

        let is_haiku = model_lower.contains("haiku");
        let is_claude3 = model_lower.contains("claude-3-");
        let is_claude4_plus = is_claude4_or_newer(&model_lower);

        let mut betas: Vec<&str> = Vec::new();

        // claude-code beta: all non-Haiku models
        if !is_haiku {
            betas.push("claude-code-20250219");
        }

        // interleaved-thinking: NOT for claude-3-* models (they don't support ISP)
        if !is_claude3 {
            betas.push("interleaved-thinking-2025-05-14");
        }

        // redact-thinking: 1P only, ISP-capable models (non-claude-3-*).
        // In Claude Code this also checks !interactive && showThinkingSummaries !== true,
        // but as a proxy we include it unconditionally for ISP models on 1P.
        if is_first_party && !is_claude3 {
            betas.push("redact-thinking-2026-02-12");
        }

        // effort: always included
        betas.push("effort-2025-11-24");

        // --- 1P-only betas ---
        if is_first_party {
            // prompt-caching-scope: 1P only (global cache scope)
            betas.push("prompt-caching-scope-2026-01-05");

            // context-management: Claude 4+ on 1P
            if is_claude4_plus {
                betas.push("context-management-2025-06-27");
            }

            // fast-mode: 1P only, server treats as no-op if not used
            betas.push("fast-mode-2026-02-01");

            // task-budgets: 1P only
            betas.push("task-budgets-2026-03-13");

            // advisor-tool: 1P only
            betas.push("advisor-tool-2026-03-01");

            // context-1m: 1P only, for models with 1M extended context support
            if has_1m_context(&model_lower) {
                betas.push("context-1m-2025-08-07");
            }

            // afk-mode (auto mode): Claude 4+ on 1P
            if is_claude4_plus {
                betas.push("afk-mode-2026-01-31");
            }

            // structured-outputs: Claude 4.5+ on 1P
            if is_claude45_or_newer(&model_lower) {
                betas.push("structured-outputs-2025-12-15");
            }

            // tool-search: 1P/Foundry gets advanced-tool-use beta
            betas.push("advanced-tool-use-2025-11-20");

            // cli-internal: ant-only, only when entrypoint is 'cli'.
            // Gated behind KOU_CC_ANT_INTERNAL env var since this is Anthropic-internal only.
            if self.entrypoint == "cli" {
                if std::env::var("KOU_CC_ANT_INTERNAL")
                    .map(|v| v == "true" || v == "1")
                    .unwrap_or(false)
                {
                    betas.push("cli-internal-2026-02-09");
                }
            }

            // oauth: Only for OAuth subscribers. Gated behind KOU_CC_OAUTH env var.
            if std::env::var("KOU_CC_OAUTH")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false)
            {
                betas.push("oauth-2025-04-20");
            }
        } else {
            // 3P: context-management for Claude 4+ (opus-4/sonnet-4)
            if is_claude4_plus {
                betas.push("context-management-2025-06-27");
            }

            // tool-search: 3P (Vertex/Bedrock) gets different beta name
            betas.push("tool-search-tool-2025-10-19");
        }

        betas.join(",")
    }

    /// Generate all passthrough headers that a Claude Code client would send.
    /// Returns a Vec of (header_name, header_value) pairs.
    ///
    /// `is_first_party` controls which beta headers are included.
    pub fn generate_headers(&self, model: &str, is_first_party: bool) -> Vec<(String, String)> {
        let mut headers = Vec::new();

        headers.push(("x-app".to_string(), "cli".to_string()));

        // Enriched User-Agent matching Claude Code format:
        // claude-cli/{VER} ({USER_TYPE}, {ENTRYPOINT}[, agent-sdk/{SDK}][, client-app/{APP}][, workload/{TAG}])
        let agent_sdk_suffix = self
            .agent_sdk_version
            .as_ref()
            .map(|v| format!(", agent-sdk/{v}"))
            .unwrap_or_default();
        let client_app_suffix = self
            .client_app
            .as_ref()
            .map(|v| format!(", client-app/{v}"))
            .unwrap_or_default();
        let workload_suffix = self
            .workload
            .as_ref()
            .map(|w| format!(", workload/{w}"))
            .unwrap_or_default();
        headers.push((
            "user-agent".to_string(),
            format!(
                "claude-cli/{ver} ({ut}, {ep}{sdk}{app}{wl})",
                ver = self.version,
                ut = self.user_type,
                ep = self.entrypoint,
                sdk = agent_sdk_suffix,
                app = client_app_suffix,
                wl = workload_suffix,
            ),
        ));

        headers.push((
            "x-claude-code-session-id".to_string(),
            self.session_id.clone(),
        ));
        // x-client-request-id: Only sent on 1P (firstParty API).
        // Unknown headers risk rejection by strict 3P proxies (inc-4029 class).
        if is_first_party {
            headers.push((
                "x-client-request-id".to_string(),
                Uuid::new_v4().to_string(),
            ));
        }
        headers.push((
            "anthropic-beta".to_string(),
            self.beta_headers(model, is_first_party),
        ));

        headers
    }

    /// Check whether fingerprint injection is needed:
    /// - Target provider must be Anthropic (Claude protocol)
    /// - Client must NOT have already sent Claude Code headers (x-app)
    /// - Fingerprinting must be enabled via env var
    pub fn needs_injection(
        &self,
        target_format: &crate::translate::ProtocolFormat,
        passthrough: &Option<crate::upstream::PassthroughHeaders>,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        if *target_format != crate::translate::ProtocolFormat::Claude {
            return false;
        }
        // If client already sent x-app header, it's likely Claude Code — don't inject
        let has_cc = passthrough
            .as_ref()
            .map(|p| p.headers.iter().any(|(n, _)| n.to_lowercase() == "x-app"))
            .unwrap_or(false);
        !has_cc
    }

    /// Inject Claude Code fingerprint data into the request body.
    ///
    /// Modifications:
    /// 1. Prepend attribution header text block to system prompt
    /// 2. Add metadata.user_id JSON string
    ///
    /// `body` is the translated body in Claude format.
    /// `messages_source` is used to extract the first user message for fingerprint computation.
    pub fn inject_body(&self, body: &mut Value, messages_source: &Value) {
        // Determine where to get messages from
        let messages = if messages_source.get("messages").is_some() {
            &messages_source["messages"]
        } else {
            &body["messages"]
        };

        let attribution = self.attribution_header(messages);

        // Check if system already has attribution (avoid duplicates)
        if let Some(system) = body.get("system") {
            if let Some(arr) = system.as_array() {
                for block in arr {
                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                        if text.starts_with("x-anthropic-billing-header") {
                            // Attribution already present — skip system injection
                            self.inject_metadata(body);
                            return;
                        }
                    }
                }
            }
            // system might be a string (OpenAI-style), handle that too
            if let Some(text) = system.as_str() {
                if text.starts_with("x-anthropic-billing-header") {
                    self.inject_metadata(body);
                    return;
                }
            }
        }

        // Inject attribution into system prompt
        let attribution_block = json!({"type": "text", "text": attribution});

        if let Some(system) = body.get_mut("system") {
            if let Some(arr) = system.as_array_mut() {
                // Prepend attribution block
                arr.insert(0, attribution_block);
            } else if system.is_string() {
                // Convert string system to array format
                let existing_text = system.as_str().unwrap_or("").to_string();
                *system = json!([
                    {"type": "text", "text": attribution},
                    {"type": "text", "text": existing_text}
                ]);
            }
        } else {
            // No system prompt — create one
            body["system"] = json!([{"type": "text", "text": attribution}]);
        }

        self.inject_metadata(body);
    }

    /// Inject metadata.user_id into the body if not already present.
    fn inject_metadata(&self, body: &mut Value) {
        if body.get("metadata").is_none() {
            body["metadata"] = json!({"user_id": self.metadata_user_id()});
        } else if let Some(meta) = body.get_mut("metadata") {
            if meta.get("user_id").is_none() {
                meta["user_id"] = json!(self.metadata_user_id());
            }
        }
    }
}

/// Extract text from the first user message in an array of messages.
/// Handles both OpenAI format (content as string) and Claude format (content as array of blocks).
fn extract_first_user_message_text(messages: &Value) -> String {
    if let Some(arr) = messages.as_array() {
        for msg in arr {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if role == "user" {
                // OpenAI format: content is a string
                if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
                    return text.to_string();
                }
                // Claude format: content is an array of blocks
                if let Some(blocks) = msg.get("content").and_then(|c| c.as_array()) {
                    for block in blocks {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            return text.to_string();
                        }
                    }
                }
            }
        }
    }
    String::new()
}

/// Check if a model name suggests Claude 4 or newer.
fn is_claude4_or_newer(model_lower: &str) -> bool {
    // Claude 4.x, 5.x, etc.
    model_lower.contains("claude-4")
        || model_lower.contains("claude-5")
        || model_lower.contains("claude-6")
        // Sonnet 4, Opus 4, etc.
        || model_lower.contains("sonnet-4")
        || model_lower.contains("opus-4")
        // Generic future-proofing
        || model_lower.contains("claude-sonnet-4")
        || model_lower.contains("claude-opus-4")
}

/// Check if a model name suggests Claude 4.5 or newer (for structured-outputs beta).
/// Claude Code gates structured-outputs on specific 4.5+/4.6+ model IDs.
fn is_claude45_or_newer(model_lower: &str) -> bool {
    model_lower.contains("sonnet-4-5")
        || model_lower.contains("sonnet-4-6")
        || model_lower.contains("opus-4-1")
        || model_lower.contains("opus-4-5")
        || model_lower.contains("opus-4-6")
        || model_lower.contains("haiku-4-5")
        || model_lower.contains("claude-5")
        || model_lower.contains("claude-6")
}

/// Check if a model supports 1M extended context.
/// In Claude Code, this is determined by `has1mContext(model)` which checks against
/// a list of specific model IDs. Claude 4+ models generally support 1M context,
/// while Claude 3.x models are limited to 200K.
fn has_1m_context(model_lower: &str) -> bool {
    // Claude 4+ models have 1M context support
    is_claude4_or_newer(model_lower)
}

/// Simple getrandom using rand crate (already a dependency)
fn getrandom(buf: &mut [u8]) {
    use rand::RngCore;
    rand::rng().fill_bytes(buf);
}

/// Encode bytes to lowercase hex string
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Get or create a stable device_id.
///
/// Priority:
/// 1. KOU_CC_DEVICE_ID env var (manual override)
/// 2. Read from ~/.config/kou-router/device_id (persistent)
/// 3. Generate random 64 hex chars and try to persist
///
/// This mirrors Claude Code's `getOrCreateUserID()` which stores a stable
/// identifier per machine rather than generating randomly per session.
fn get_or_create_device_id() -> String {
    // 1. Check env override
    if let Ok(id) = std::env::var("KOU_CC_DEVICE_ID") {
        if id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()) {
            return id;
        }
    }

    // 2. Try to read from persistent storage
    if let Some(path) = device_id_path() {
        if let Ok(contents) = std::fs::read_to_string(&path) {
            let trimmed = contents.trim().to_string();
            if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
                return trimmed;
            }
        }
    }

    // 3. Generate new device_id
    let device_id = {
        let mut bytes = [0u8; 32];
        getrandom(&mut bytes);
        hex_encode(&bytes)
    };

    // Try to persist for future runs
    if let Some(path) = device_id_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, &device_id);
    }

    device_id
}

/// Return the path to the persistent device_id file, if determinable.
fn device_id_path() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|home| {
        std::path::PathBuf::from(home)
            .join(".config")
            .join("kou-router")
            .join("device_id")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Create a test instance with known values for deterministic testing
    fn test_fingerprint() -> ClaudeCodeFingerprint {
        ClaudeCodeFingerprint {
            version: "2.1.88".to_string(),
            device_id: "a".repeat(64),
            session_id: "test-session-id-1234".to_string(),
            enabled: true,
            entrypoint: "cli".to_string(),
            user_type: "external".to_string(),
            workload: None,
            agent_sdk_version: None,
            client_app: None,
        }
    }

    #[test]
    fn test_compute_fingerprint_basic() {
        let fp = test_fingerprint();
        // Message: "Hello, world! This is a test message."
        // Index:     0123456789...
        // chars at [4, 7, 20]: 'o', 'w', 's'
        let messages = json!([
            {"role": "user", "content": "Hello, world! This is a test message."}
        ]);
        let result = fp.compute_fingerprint(&messages);
        assert_eq!(result.len(), 3);
        // Verify it's valid hex
        assert!(result.chars().all(|c| c.is_ascii_hexdigit()));

        // Compute expected:
        // input = "59cf53e54c78" + "ows" + "2.1.88"
        let input = format!("{}ows2.1.88", FINGERPRINT_SALT);
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let hash = hasher.finalize();
        let expected = hex_encode(&hash)[..3].to_string();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_compute_fingerprint_short_message() {
        let fp = test_fingerprint();
        // Message only 5 chars: "Hello"
        // chars at [4, 7, 20]: 'o', '0', '0'
        let messages = json!([
            {"role": "user", "content": "Hello"}
        ]);
        let result = fp.compute_fingerprint(&messages);
        assert_eq!(result.len(), 3);

        let input = format!("{}o002.1.88", FINGERPRINT_SALT);
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let hash = hasher.finalize();
        let expected = hex_encode(&hash)[..3].to_string();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_compute_fingerprint_empty_messages() {
        let fp = test_fingerprint();
        // No user messages at all
        let messages = json!([]);
        let result = fp.compute_fingerprint(&messages);
        assert_eq!(result.len(), 3);

        // All indices fall back to '0'
        let input = format!("{}0002.1.88", FINGERPRINT_SALT);
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let hash = hasher.finalize();
        let expected = hex_encode(&hash)[..3].to_string();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_compute_fingerprint_claude_format_messages() {
        let fp = test_fingerprint();
        // Claude format: content is array of blocks
        let messages = json!([
            {"role": "user", "content": [{"type": "text", "text": "Hello, world! This is a test message."}]}
        ]);
        let result = fp.compute_fingerprint(&messages);
        assert_eq!(result.len(), 3);

        // Should extract same text as OpenAI format
        let messages_openai = json!([
            {"role": "user", "content": "Hello, world! This is a test message."}
        ]);
        let result_openai = fp.compute_fingerprint(&messages_openai);
        assert_eq!(result, result_openai);
    }

    #[test]
    fn test_attribution_header_format() {
        let fp = test_fingerprint();
        let messages = json!([
            {"role": "user", "content": "test"}
        ]);
        let header = fp.attribution_header(&messages);
        assert!(header.starts_with("x-anthropic-billing-header: cc_version=2.1.88."));
        assert!(header.contains("; cc_entrypoint=cli; cch=00000;"));
        // The fingerprint part should be 3 hex chars
        let after_version = header
            .strip_prefix("x-anthropic-billing-header: cc_version=2.1.88.")
            .unwrap();
        let fingerprint = &after_version[..3];
        assert_eq!(fingerprint.len(), 3);
        assert!(fingerprint.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_metadata_user_id_format() {
        let fp = test_fingerprint();
        let user_id = fp.metadata_user_id();

        // Should be a valid JSON string
        let parsed: Value = serde_json::from_str(&user_id).unwrap();
        assert_eq!(parsed["device_id"].as_str().unwrap().len(), 64);
        assert_eq!(parsed["account_uuid"].as_str().unwrap(), "");
        assert_eq!(
            parsed["session_id"].as_str().unwrap(),
            "test-session-id-1234"
        );
    }

    #[test]
    fn test_beta_headers_non_haiku_1p() {
        let fp = test_fingerprint();
        let betas = fp.beta_headers("claude-sonnet-4-20250514", true);
        assert!(betas.contains("claude-code-20250219"));
        assert!(betas.contains("interleaved-thinking-2025-05-14"));
        assert!(betas.contains("redact-thinking-2026-02-12"));
        assert!(betas.contains("prompt-caching-scope-2026-01-05"));
        assert!(betas.contains("effort-2025-11-24"));
        assert!(betas.contains("context-management-2025-06-27"));
        // New 1P-only betas
        assert!(betas.contains("fast-mode-2026-02-01"));
        assert!(betas.contains("task-budgets-2026-03-13"));
        assert!(betas.contains("advisor-tool-2026-03-01"));
        assert!(betas.contains("context-1m-2025-08-07"));
        assert!(betas.contains("afk-mode-2026-01-31"));
        assert!(betas.contains("advanced-tool-use-2025-11-20"));
        // 1P should NOT have 3P tool-search beta
        assert!(!betas.contains("tool-search-tool-2025-10-19"));
    }

    #[test]
    fn test_beta_headers_haiku_1p() {
        let fp = test_fingerprint();
        let betas = fp.beta_headers("claude-3-5-haiku-20241022", true);
        // Haiku should NOT have claude-code beta
        assert!(!betas.contains("claude-code-20250219"));
        // claude-3-* should NOT have interleaved-thinking
        assert!(!betas.contains("interleaved-thinking-2025-05-14"));
        // claude-3-* should NOT have redact-thinking (not ISP-capable)
        assert!(!betas.contains("redact-thinking-2026-02-12"));
        assert!(betas.contains("effort-2025-11-24"));
        // Haiku is not Claude 4+, no context management
        assert!(!betas.contains("context-management-2025-06-27"));
        // But should have other 1P betas
        assert!(betas.contains("prompt-caching-scope-2026-01-05"));
        assert!(betas.contains("fast-mode-2026-02-01"));
        // Haiku (claude-3) should NOT have context-1m (not Claude 4+)
        assert!(!betas.contains("context-1m-2025-08-07"));
    }

    #[test]
    fn test_beta_headers_claude35_1p() {
        let fp = test_fingerprint();
        let betas = fp.beta_headers("claude-3-5-sonnet-20241022", true);
        assert!(betas.contains("claude-code-20250219"));
        // claude-3-* should NOT have interleaved-thinking
        assert!(!betas.contains("interleaved-thinking-2025-05-14"));
        // Not Claude 4+, no context management
        assert!(!betas.contains("context-management-2025-06-27"));
        // Should have prompt-caching-scope on 1P
        assert!(betas.contains("prompt-caching-scope-2026-01-05"));
        // claude-3-* should NOT have redact-thinking
        assert!(!betas.contains("redact-thinking-2026-02-12"));
        // claude-3-5-sonnet should NOT have context-1m (not Claude 4+)
        assert!(!betas.contains("context-1m-2025-08-07"));
    }

    #[test]
    fn test_beta_headers_3p_provider() {
        let fp = test_fingerprint();
        let betas = fp.beta_headers("claude-sonnet-4-20250514", false);
        assert!(betas.contains("claude-code-20250219"));
        assert!(betas.contains("interleaved-thinking-2025-05-14"));
        assert!(betas.contains("effort-2025-11-24"));
        assert!(betas.contains("context-management-2025-06-27"));
        // 3P should NOT have 1P-only betas
        assert!(!betas.contains("prompt-caching-scope-2026-01-05"));
        assert!(!betas.contains("fast-mode-2026-02-01"));
        assert!(!betas.contains("task-budgets-2026-03-13"));
        // 3P should NOT have redact-thinking
        assert!(!betas.contains("redact-thinking-2026-02-12"));
        // 3P should NOT have advanced-tool-use (1P-only)
        assert!(!betas.contains("advanced-tool-use-2025-11-20"));
        // 3P should have tool-search-tool
        assert!(betas.contains("tool-search-tool-2025-10-19"));
    }

    #[test]
    fn test_beta_headers_structured_outputs() {
        let fp = test_fingerprint();
        // Claude 4.5+ should get structured-outputs on 1P
        let betas = fp.beta_headers("claude-sonnet-4-5-20250514", true);
        assert!(betas.contains("structured-outputs-2025-12-15"));
        // Claude 4.0 should NOT
        let betas = fp.beta_headers("claude-sonnet-4-20250514", true);
        assert!(!betas.contains("structured-outputs-2025-12-15"));
    }

    #[test]
    fn test_needs_injection_with_cc_headers() {
        let fp = test_fingerprint();
        let pt = Some(crate::upstream::PassthroughHeaders {
            headers: vec![("x-app".to_string(), "cli".to_string())],
        });
        assert!(!fp.needs_injection(&crate::translate::ProtocolFormat::Claude, &pt));
    }

    #[test]
    fn test_needs_injection_without_cc_headers() {
        let fp = test_fingerprint();
        let pt: Option<crate::upstream::PassthroughHeaders> = None;
        assert!(fp.needs_injection(&crate::translate::ProtocolFormat::Claude, &pt));
    }

    #[test]
    fn test_needs_injection_with_empty_passthrough() {
        let fp = test_fingerprint();
        let pt = Some(crate::upstream::PassthroughHeaders { headers: vec![] });
        assert!(fp.needs_injection(&crate::translate::ProtocolFormat::Claude, &pt));
    }

    #[test]
    fn test_needs_injection_non_anthropic() {
        let fp = test_fingerprint();
        let pt: Option<crate::upstream::PassthroughHeaders> = None;
        assert!(!fp.needs_injection(&crate::translate::ProtocolFormat::OpenAI, &pt));
        assert!(!fp.needs_injection(&crate::translate::ProtocolFormat::Gemini, &pt));
    }

    #[test]
    fn test_needs_injection_disabled() {
        let mut fp = test_fingerprint();
        fp.enabled = false;
        let pt: Option<crate::upstream::PassthroughHeaders> = None;
        assert!(!fp.needs_injection(&crate::translate::ProtocolFormat::Claude, &pt));
    }

    #[test]
    fn test_inject_body_with_existing_system() {
        let fp = test_fingerprint();
        let mut body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}],
            "system": [{"type": "text", "text": "You are helpful."}]
        });
        let messages_source = body.clone();
        fp.inject_body(&mut body, &messages_source);

        let system = body["system"].as_array().unwrap();
        assert_eq!(system.len(), 2); // attribution + original
        assert!(
            system[0]["text"]
                .as_str()
                .unwrap()
                .starts_with("x-anthropic-billing-header")
        );
        assert_eq!(system[1]["text"].as_str().unwrap(), "You are helpful.");

        // Check metadata was added
        assert!(body.get("metadata").is_some());
        let user_id_str = body["metadata"]["user_id"].as_str().unwrap();
        let user_id: Value = serde_json::from_str(user_id_str).unwrap();
        assert_eq!(user_id["device_id"].as_str().unwrap().len(), 64);
    }

    #[test]
    fn test_inject_body_without_system() {
        let fp = test_fingerprint();
        let mut body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let messages_source = body.clone();
        fp.inject_body(&mut body, &messages_source);

        let system = body["system"].as_array().unwrap();
        assert_eq!(system.len(), 1);
        assert!(
            system[0]["text"]
                .as_str()
                .unwrap()
                .starts_with("x-anthropic-billing-header")
        );

        assert!(body.get("metadata").is_some());
    }

    #[test]
    fn test_inject_body_no_duplicate_attribution() {
        let fp = test_fingerprint();
        let mut body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{"role": "user", "content": "Hello"}],
            "system": [{"type": "text", "text": "x-anthropic-billing-header: cc_version=2.1.88.abc; cc_entrypoint=cli;"}]
        });
        let messages_source = body.clone();
        fp.inject_body(&mut body, &messages_source);

        // Should NOT have added another attribution block
        let system = body["system"].as_array().unwrap();
        assert_eq!(system.len(), 1);
    }

    #[test]
    fn test_inject_body_existing_metadata() {
        let fp = test_fingerprint();
        let mut body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{"role": "user", "content": "Hello"}],
            "metadata": {"some_key": "some_value"}
        });
        let messages_source = body.clone();
        fp.inject_body(&mut body, &messages_source);

        // Should add user_id but not overwrite existing metadata
        assert_eq!(body["metadata"]["some_key"].as_str().unwrap(), "some_value");
        assert!(body["metadata"]["user_id"].as_str().is_some());
    }

    #[test]
    fn test_generate_headers() {
        let fp = test_fingerprint();
        let headers = fp.generate_headers("claude-sonnet-4-20250514", true);

        let find = |name: &str| -> Option<String> {
            headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };

        assert_eq!(find("x-app").unwrap(), "cli");
        let ua = find("user-agent").unwrap();
        assert!(ua.starts_with("claude-cli/2.1.88"));
        assert!(ua.contains("(external, cli)"));
        assert_eq!(
            find("x-claude-code-session-id").unwrap(),
            "test-session-id-1234"
        );
        assert!(find("x-client-request-id").is_some());
        let betas = find("anthropic-beta").unwrap();
        assert!(betas.contains("claude-code-20250219"));
        // 1P-specific betas should be present
        assert!(betas.contains("prompt-caching-scope-2026-01-05"));
    }

    #[test]
    fn test_generate_headers_3p_no_client_request_id() {
        let fp = test_fingerprint();
        let headers = fp.generate_headers("claude-sonnet-4-20250514", false);

        let find = |name: &str| -> Option<String> {
            headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };

        // x-client-request-id should NOT be present on 3P
        assert!(
            find("x-client-request-id").is_none(),
            "x-client-request-id should not be sent to 3P providers"
        );
        // Other headers should still be present
        assert_eq!(find("x-app").unwrap(), "cli");
        assert!(find("x-claude-code-session-id").is_some());
        assert!(find("anthropic-beta").is_some());
    }

    #[test]
    fn test_generate_headers_with_workload() {
        let mut fp = test_fingerprint();
        fp.workload = Some("cron-task".to_string());
        let headers = fp.generate_headers("claude-sonnet-4-20250514", true);

        let ua = headers
            .iter()
            .find(|(n, _)| n == "user-agent")
            .unwrap()
            .1
            .clone();
        assert!(
            ua.contains("workload/cron-task"),
            "UA should contain workload: {ua}"
        );
    }

    #[test]
    fn test_generate_headers_with_agent_sdk_and_client_app() {
        let mut fp = test_fingerprint();
        fp.agent_sdk_version = Some("0.1.17".to_string());
        fp.client_app = Some("my-app".to_string());
        fp.workload = Some("cron".to_string());
        let headers = fp.generate_headers("claude-sonnet-4-20250514", true);

        let ua = headers
            .iter()
            .find(|(n, _)| n == "user-agent")
            .unwrap()
            .1
            .clone();
        // Full UA: claude-cli/2.1.88 (external, cli, agent-sdk/0.1.17, client-app/my-app, workload/cron)
        assert!(
            ua.contains("agent-sdk/0.1.17"),
            "UA should contain agent-sdk: {ua}"
        );
        assert!(
            ua.contains("client-app/my-app"),
            "UA should contain client-app: {ua}"
        );
        assert!(
            ua.contains("workload/cron"),
            "UA should contain workload: {ua}"
        );
        // Verify ordering: agent-sdk before client-app before workload
        let sdk_pos = ua.find("agent-sdk").unwrap();
        let app_pos = ua.find("client-app").unwrap();
        let wl_pos = ua.find("workload").unwrap();
        assert!(sdk_pos < app_pos, "agent-sdk should come before client-app");
        assert!(app_pos < wl_pos, "client-app should come before workload");
    }

    #[test]
    fn test_attribution_with_workload() {
        let mut fp = test_fingerprint();
        fp.workload = Some("cron-task".to_string());
        let messages = json!([{"role": "user", "content": "test"}]);
        let header = fp.attribution_header(&messages);
        assert!(
            header.contains("cc_workload=cron-task;"),
            "Header should contain workload: {header}"
        );
    }

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex_encode(&[0x00, 0xff, 0xab]), "00ffab");
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn test_extract_first_user_message_text_openai() {
        let messages = json!([
            {"role": "system", "content": "System prompt"},
            {"role": "user", "content": "Hello world"},
            {"role": "assistant", "content": "Hi there"}
        ]);
        assert_eq!(extract_first_user_message_text(&messages), "Hello world");
    }

    #[test]
    fn test_extract_first_user_message_text_claude() {
        let messages = json!([
            {"role": "user", "content": [{"type": "text", "text": "Hello Claude"}]}
        ]);
        assert_eq!(extract_first_user_message_text(&messages), "Hello Claude");
    }

    #[test]
    fn test_extract_first_user_message_text_no_user() {
        let messages = json!([
            {"role": "system", "content": "System prompt"},
            {"role": "assistant", "content": "Hi"}
        ]);
        assert_eq!(extract_first_user_message_text(&messages), "");
    }

    #[test]
    fn test_is_claude4_or_newer() {
        assert!(is_claude4_or_newer("claude-4-20250514"));
        assert!(is_claude4_or_newer("claude-sonnet-4-20250514"));
        assert!(is_claude4_or_newer("claude-opus-4-20250514"));
        assert!(is_claude4_or_newer("sonnet-4-latest"));
        assert!(is_claude4_or_newer("opus-4-latest"));
        assert!(!is_claude4_or_newer("claude-3-5-sonnet-20241022"));
        assert!(!is_claude4_or_newer("claude-3-haiku-20240307"));
    }

    #[test]
    fn test_is_claude45_or_newer() {
        assert!(is_claude45_or_newer("claude-sonnet-4-5-20250514"));
        assert!(is_claude45_or_newer("claude-sonnet-4-6-20250514"));
        assert!(is_claude45_or_newer("claude-opus-4-1-20250514"));
        assert!(is_claude45_or_newer("claude-opus-4-5-20250514"));
        assert!(is_claude45_or_newer("claude-haiku-4-5-20250514"));
        assert!(is_claude45_or_newer("claude-5-sonnet"));
        assert!(!is_claude45_or_newer("claude-sonnet-4-20250514"));
        assert!(!is_claude45_or_newer("claude-opus-4-20250514"));
        assert!(!is_claude45_or_newer("claude-3-5-sonnet-20241022"));
    }
}
