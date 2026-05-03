//! Reads the live subscription quota from Anthropic's undocumented OAuth usage endpoint.
//!
//! Claude Code authenticates to the Anthropic web account via OAuth and stores the resulting
//! access token in the OS credential store. This module loads that token (macOS Keychain, with
//! a JSON-file fallback for other platforms) and calls `GET /api/oauth/usage` — the same
//! endpoint the Claude Code CLI uses internally to render its `/usage` view.
//!
//! The endpoint is undocumented and may change without notice.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::process::Command;
use std::time::Duration;

#[derive(Deserialize, Clone, Debug)]
pub struct QuotaWindow {
    pub utilization: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct ExtraUsage {
    #[serde(default)]
    pub is_enabled: bool,
    #[serde(default)]
    pub utilization: Option<f64>,
    #[serde(default)]
    pub used_credits: Option<f64>,
    #[serde(default)]
    pub monthly_limit: Option<f64>,
    #[serde(default)]
    pub currency: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct QuotaResponse {
    #[serde(default)]
    pub five_hour: Option<QuotaWindow>,
    #[serde(default)]
    pub seven_day: Option<QuotaWindow>,
    #[serde(default)]
    pub seven_day_sonnet: Option<QuotaWindow>,
    #[serde(default)]
    pub seven_day_opus: Option<QuotaWindow>,
    #[serde(default)]
    pub extra_usage: Option<ExtraUsage>,
}

#[derive(Clone, Debug)]
pub struct Credentials {
    pub access_token: String,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
}

#[derive(Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: OAuth,
}

#[derive(Deserialize)]
struct OAuth {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "subscriptionType", default)]
    subscription_type: Option<String>,
    #[serde(rename = "rateLimitTier", default)]
    rate_limit_tier: Option<String>,
}

pub fn load_credentials() -> Result<Credentials> {
    // macOS Keychain first.
    if cfg!(target_os = "macos") {
        let out = Command::new("security")
            .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
            .output();
        if let Ok(out) = out {
            if out.status.success() {
                let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let parsed: CredentialsFile = serde_json::from_str(&raw)
                    .context("parse keychain credentials JSON")?;
                return Ok(parsed.into());
            }
        }
    }

    // Fallback: ~/.claude/.credentials.json (Linux / non-keychain installs).
    let path = dirs::home_dir()
        .map(|h| h.join(".claude").join(".credentials.json"))
        .ok_or_else(|| anyhow!("no home dir"))?;
    let bytes = std::fs::read(&path)
        .with_context(|| format!("read {}", path.display()))?;
    let parsed: CredentialsFile = serde_json::from_slice(&bytes)
        .context("parse credentials JSON")?;
    Ok(parsed.into())
}

impl From<CredentialsFile> for Credentials {
    fn from(c: CredentialsFile) -> Self {
        Credentials {
            access_token: c.claude_ai_oauth.access_token,
            subscription_type: c.claude_ai_oauth.subscription_type,
            rate_limit_tier: c.claude_ai_oauth.rate_limit_tier,
        }
    }
}

pub fn fetch_quota(creds: &Credentials) -> Result<QuotaResponse> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();
    let resp = agent
        .get("https://api.anthropic.com/api/oauth/usage")
        .set("Authorization", &format!("Bearer {}", creds.access_token))
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("User-Agent", "claude-usage-monitor (oauth)")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, r) => anyhow!(
                "HTTP {code}: {}",
                r.into_string().unwrap_or_default()
            ),
            ureq::Error::Transport(t) => anyhow!("transport: {t}"),
        })?;
    resp.into_json::<QuotaResponse>().context("parse quota JSON")
}
