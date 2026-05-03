use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PlanKind {
    #[default]
    None,
    Pro,
    Max5x,
    Max20x,
    Team,
    Custom,
}

impl PlanKind {
    pub fn label(self) -> &'static str {
        match self {
            PlanKind::None => "—",
            PlanKind::Pro => "Claude Pro",
            PlanKind::Max5x => "Claude Max 5×",
            PlanKind::Max20x => "Claude Max 20×",
            PlanKind::Team => "Claude Team (per seat)",
            PlanKind::Custom => "Custom",
        }
    }

    pub fn default_cost(self) -> Option<f64> {
        match self {
            PlanKind::None => None,
            PlanKind::Pro => Some(20.0),
            PlanKind::Max5x => Some(100.0),
            PlanKind::Max20x => Some(200.0),
            PlanKind::Team => Some(25.0),
            PlanKind::Custom => None,
        }
    }

    pub fn cycle_next(self) -> Self {
        match self {
            PlanKind::None => PlanKind::Pro,
            PlanKind::Pro => PlanKind::Max5x,
            PlanKind::Max5x => PlanKind::Max20x,
            PlanKind::Max20x => PlanKind::Team,
            PlanKind::Team => PlanKind::Custom,
            PlanKind::Custom => PlanKind::None,
        }
    }

    pub fn from_env_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "pro" => PlanKind::Pro,
            "max5x" | "max-5x" | "max_5x" => PlanKind::Max5x,
            "max20x" | "max-20x" | "max_20x" => PlanKind::Max20x,
            "team" => PlanKind::Team,
            "custom" => PlanKind::Custom,
            _ => PlanKind::None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default = "plan_kind_default")]
    pub plan: PlanKind,
    /// Only consulted when `plan == PlanKind::Custom`. For known plans, the
    /// monthly cost comes from `PlanKind::default_cost`.
    #[serde(default)]
    pub custom_plan_cost_usd: Option<f64>,
    #[serde(default)]
    pub limit_5h_usd: Option<f64>,
    #[serde(default)]
    pub limit_week_usd: Option<f64>,
}

fn plan_kind_default() -> PlanKind {
    PlanKind::None
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("claude-usage-monitor").join("config.json"))
    }

    pub fn load_or_seed() -> Self {
        if let Some(p) = Self::path() {
            if let Ok(bytes) = fs::read(&p) {
                if let Ok(cfg) = serde_json::from_slice::<Config>(&bytes) {
                    return cfg;
                }
            }
        }
        Self::from_env()
    }

    fn from_env() -> Self {
        let plan = std::env::var("CLAUDE_PLAN")
            .ok()
            .map(|s| PlanKind::from_env_str(&s))
            .unwrap_or(PlanKind::None);
        Self {
            plan,
            custom_plan_cost_usd: std::env::var("CLAUDE_PLAN_COST_USD")
                .ok()
                .and_then(|s| s.parse().ok()),
            limit_5h_usd: std::env::var("CLAUDE_5H_LIMIT_USD")
                .ok()
                .and_then(|s| s.parse().ok()),
            limit_week_usd: std::env::var("CLAUDE_WEEK_LIMIT_USD")
                .ok()
                .and_then(|s| s.parse().ok()),
        }
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path().context("no config dir on this platform")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {parent:?}"))?;
        }
        let bytes = serde_json::to_vec_pretty(self)?;
        fs::write(&path, bytes).with_context(|| format!("write {path:?}"))?;
        Ok(path)
    }

    /// Effective monthly subscription cost in USD, or None if no plan configured.
    pub fn plan_monthly_usd(&self) -> Option<f64> {
        match self.plan {
            PlanKind::None => None,
            PlanKind::Custom => self.custom_plan_cost_usd,
            other => other.default_cost(),
        }
    }
}
