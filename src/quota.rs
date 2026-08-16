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
    pub used_credits: Option<f64>,
    #[serde(default)]
    pub monthly_limit: Option<f64>,
    #[serde(default)]
    pub currency: Option<String>,
    /// Number of decimal digits `used_credits`/`monthly_limit` are shifted by (e.g. `2` means
    /// the API reported cents). Absent when the API already reports major currency units.
    #[serde(default)]
    pub decimal_places: Option<u32>,
}

impl ExtraUsage {
    fn minor_to_major_divisor(&self) -> f64 {
        10f64.powi(self.decimal_places.unwrap_or(0) as i32)
    }

    /// Credits used, converted from minor units (e.g. cents) to major currency units.
    pub fn used_major(&self) -> f64 {
        self.used_credits.unwrap_or(0.0) / self.minor_to_major_divisor()
    }

    /// Monthly spend limit, converted from minor units (e.g. cents) to major currency units.
    pub fn limit_major(&self) -> f64 {
        self.monthly_limit.unwrap_or(0.0) / self.minor_to_major_divisor()
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct Money {
    pub amount_minor: f64,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub exponent: Option<u32>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct PrepaidBalance {
    #[serde(default)]
    pub money: Option<Money>,
}

/// Response of `GET /api/oauth/organizations/{org_uuid}/prepaid/credits` — the prepaid
/// usage-credits balance that replaced the postpaid `extra_usage` model for migrated accounts.
#[derive(Deserialize, Clone, Debug)]
pub struct PrepaidCredits {
    /// Top-level balance in minor units (cents). Kept as a fallback; `balance.money`
    /// is preferred because it carries an explicit exponent.
    #[serde(default)]
    pub amount: Option<f64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub balance: Option<PrepaidBalance>,
    #[serde(default)]
    pub next_expires_at: Option<DateTime<Utc>>,
}

impl PrepaidCredits {
    /// Remaining balance in major currency units (e.g. euros).
    pub fn balance_major(&self) -> f64 {
        if let Some(m) = self.balance.as_ref().and_then(|b| b.money.as_ref()) {
            return m.amount_minor / 10f64.powi(m.exponent.unwrap_or(2) as i32);
        }
        self.amount.unwrap_or(0.0) / 100.0
    }

    pub fn currency(&self) -> &str {
        self.balance
            .as_ref()
            .and_then(|b| b.money.as_ref())
            .and_then(|m| m.currency.as_deref())
            .or(self.currency.as_deref())
            .unwrap_or("USD")
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct ProfileResponse {
    #[serde(default)]
    pub organization: Option<ProfileOrg>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ProfileOrg {
    pub uuid: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_minor_units_to_major_using_decimal_places() {
        let extra = ExtraUsage {
            is_enabled: true,
            used_credits: Some(709.0),
            monthly_limit: Some(4000.0),
            currency: Some("EUR".to_string()),
            decimal_places: Some(2),
        };
        assert_eq!(extra.used_major(), 7.09);
        assert_eq!(extra.limit_major(), 40.0);
    }

    #[test]
    fn parses_prepaid_credits_and_converts_to_major_units() {
        // Trimmed real response from GET /api/oauth/organizations/{org}/prepaid/credits.
        let json = r#"{
            "amount": 2934,
            "currency": "EUR",
            "balance": {"money": {"amount_minor": 2934, "currency": "EUR", "exponent": 2}, "credits": null},
            "auto_reload_settings": null,
            "tranches": [{"remaining_amount_minor_units": 2933, "currency": "EUR", "expires_at": "2027-07-14T00:00:00Z"}],
            "next_expires_at": "2027-07-14T00:00:00Z"
        }"#;
        let credits: PrepaidCredits = serde_json::from_str(json).unwrap();
        assert_eq!(credits.balance_major(), 29.34);
        assert_eq!(credits.currency(), "EUR");
        assert_eq!(
            credits.next_expires_at.unwrap(),
            "2027-07-14T00:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn prepaid_credits_fall_back_to_top_level_amount_as_cents() {
        let json = r#"{"amount": 2934, "currency": "EUR"}"#;
        let credits: PrepaidCredits = serde_json::from_str(json).unwrap();
        assert_eq!(credits.balance_major(), 29.34);
        assert_eq!(credits.currency(), "EUR");
        assert!(credits.next_expires_at.is_none());
    }

    #[test]
    fn parses_org_uuid_from_profile_response() {
        let json = r#"{
            "account": {"uuid": "184f5e94-95fb-4c02-b80f-89c5fe33c0ef", "email": "x@y.z"},
            "organization": {"uuid": "e6cf8ec6-61c6-4f63-a308-9e3a340c52ba", "name": "org"}
        }"#;
        let profile: ProfileResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            profile.organization.unwrap().uuid,
            "e6cf8ec6-61c6-4f63-a308-9e3a340c52ba"
        );
    }

    #[test]
    fn defaults_to_no_conversion_when_decimal_places_absent() {
        let extra = ExtraUsage {
            is_enabled: true,
            used_credits: Some(6.78),
            monthly_limit: Some(40.0),
            currency: Some("EUR".to_string()),
            decimal_places: None,
        };
        assert_eq!(extra.used_major(), 6.78);
        assert_eq!(extra.limit_major(), 40.0);
    }
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

fn get_json<T: serde::de::DeserializeOwned>(creds: &Credentials, url: &str, what: &str) -> Result<T> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();
    let resp = agent
        .get(url)
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
    resp.into_json::<T>()
        .with_context(|| format!("parse {what} JSON"))
}

pub fn fetch_quota(creds: &Credentials) -> Result<QuotaResponse> {
    get_json(creds, "https://api.anthropic.com/api/oauth/usage", "quota")
}

/// Fetches the prepaid usage-credits balance. Requires an extra round-trip to `/profile`
/// for the organization UUID. Errors (404, org-less accounts, …) are expected for accounts
/// still on the postpaid extra-usage model — callers fall back to `extra_usage`.
pub fn fetch_prepaid_credits(creds: &Credentials) -> Result<PrepaidCredits> {
    let profile: ProfileResponse =
        get_json(creds, "https://api.anthropic.com/api/oauth/profile", "profile")?;
    let org = profile
        .organization
        .ok_or_else(|| anyhow!("profile has no organization"))?;
    get_json(
        creds,
        &format!("https://api.anthropic.com/api/oauth/organizations/{}/prepaid/credits", org.uuid),
        "prepaid credits",
    )
}
