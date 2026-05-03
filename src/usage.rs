use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::pricing;

#[derive(Deserialize)]
struct Entry {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    message: Option<Message>,
    #[serde(default)]
    timestamp: Option<DateTime<Utc>>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default, rename = "sessionId")]
    session_id: Option<String>,
}

#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize, Default, Clone, Copy)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

#[derive(Default, Clone)]
pub struct Aggregate {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub cost: f64,
    pub messages: u64,
}

impl Aggregate {
    fn add(&mut self, model: &str, u: Usage) {
        self.input += u.input_tokens;
        self.output += u.output_tokens;
        self.cache_write += u.cache_creation_input_tokens;
        self.cache_read += u.cache_read_input_tokens;
        self.cost += pricing::cost(
            pricing::lookup(model),
            u.input_tokens,
            u.output_tokens,
            u.cache_creation_input_tokens,
            u.cache_read_input_tokens,
        );
        self.messages += 1;
    }

    pub fn total_tokens(&self) -> u64 {
        self.input + self.output + self.cache_write + self.cache_read
    }
}

#[derive(Default, Clone)]
pub struct Snapshot {
    pub total: Aggregate,
    pub today: Aggregate,
    pub last_hour: Aggregate,
    pub by_model: Vec<(String, Aggregate)>,
    pub by_project: Vec<(String, Aggregate)>,
    pub last_activity: Option<DateTime<Utc>>,
    pub session_count: usize,
    pub computed_at: DateTime<Utc>,
    // Rolling-window quotas (mirrors what `/usage` shows).
    pub window_5h: Aggregate,
    /// Earliest message timestamp inside the active 5-hour window. Anthropic resets the
    /// window 5h after the first message, so reset = window_5h_start + 5h.
    pub window_5h_start: Option<DateTime<Utc>>,
    /// Sonnet-only slice of the 5-hour window (Sonnet has its own dedicated quota on Pro/Max).
    pub window_5h_sonnet: Aggregate,
    pub window_5h_sonnet_start: Option<DateTime<Utc>>,
    pub window_week: Aggregate,
    /// Earliest message timestamp inside the active 7-day window.
    pub window_week_start: Option<DateTime<Utc>>,
    pub current_session: Aggregate,
    pub current_session_id: Option<String>,
    /// Earliest message timestamp in the active session — basis for the session's own 5h reset.
    pub current_session_start: Option<DateTime<Utc>>,
    /// Calendar-month-to-date aggregate, used for the subscription-value view.
    pub month_to_date: Aggregate,
}

pub fn projects_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("projects"))
}

pub fn collect(root: &Path) -> Result<Snapshot> {
    let now = Utc::now();
    let today_start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    let hour_ago = now - chrono::Duration::hours(1);
    let five_h_ago = now - chrono::Duration::hours(5);
    let week_ago = now - chrono::Duration::days(7);
    let month_start = first_of_month(now);

    let mut total = Aggregate::default();
    let mut today = Aggregate::default();
    let mut last_hour = Aggregate::default();
    let mut month_to_date = Aggregate::default();
    let mut window_5h = Aggregate::default();
    let mut window_5h_start: Option<DateTime<Utc>> = None;
    let mut window_5h_sonnet = Aggregate::default();
    let mut window_5h_sonnet_start: Option<DateTime<Utc>> = None;
    let mut window_week = Aggregate::default();
    let mut window_week_start: Option<DateTime<Utc>> = None;
    let mut by_model: HashMap<String, Aggregate> = HashMap::new();
    let mut by_project: HashMap<String, Aggregate> = HashMap::new();
    // Per-session: aggregate + first_ts + last_ts.
    let mut by_session: HashMap<String, (Aggregate, DateTime<Utc>, DateTime<Utc>)> = HashMap::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut last_activity: Option<DateTime<Utc>> = None;
    let mut session_count = 0usize;

    if !root.exists() {
        return Ok(Snapshot {
            computed_at: now,
            ..Default::default()
        });
    }

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("jsonl"))
    {
        session_count += 1;
        let file = match File::open(entry.path()) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let reader = BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            if line.is_empty() {
                continue;
            }
            let parsed: Entry = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if parsed.r#type != "assistant" {
                continue;
            }
            let Some(msg) = parsed.message else { continue };
            let Some(usage) = msg.usage else { continue };
            let model = msg.model.unwrap_or_else(|| "unknown".to_string());

            // Dedup by message id (forked sessions replay the same assistant turn).
            if let Some(id) = msg.id.as_ref() {
                if !seen_ids.insert(id.clone()) {
                    continue;
                }
            }

            total.add(&model, usage);
            by_model.entry(model.clone()).or_default().add(&model, usage);

            let project = parsed
                .cwd
                .as_deref()
                .map(short_project_name)
                .unwrap_or_else(|| "unknown".to_string());
            by_project.entry(project).or_default().add(&model, usage);

            if let Some(ts) = parsed.timestamp {
                if ts >= today_start {
                    today.add(&model, usage);
                }
                if ts >= hour_ago {
                    last_hour.add(&model, usage);
                }
                if ts >= month_start {
                    month_to_date.add(&model, usage);
                }
                if ts >= five_h_ago {
                    window_5h.add(&model, usage);
                    window_5h_start = Some(window_5h_start.map_or(ts, |prev| prev.min(ts)));
                    if is_sonnet(&model) {
                        window_5h_sonnet.add(&model, usage);
                        window_5h_sonnet_start =
                            Some(window_5h_sonnet_start.map_or(ts, |prev| prev.min(ts)));
                    }
                }
                if ts >= week_ago {
                    window_week.add(&model, usage);
                    window_week_start = Some(window_week_start.map_or(ts, |prev| prev.min(ts)));
                }
                last_activity = Some(last_activity.map_or(ts, |prev| prev.max(ts)));

                if let Some(sid) = parsed.session_id.as_ref() {
                    let slot = by_session
                        .entry(sid.clone())
                        .or_insert_with(|| (Aggregate::default(), ts, ts));
                    slot.0.add(&model, usage);
                    if ts < slot.1 {
                        slot.1 = ts;
                    }
                    if ts > slot.2 {
                        slot.2 = ts;
                    }
                }
            }
        }
    }

    let (current_session_id, current_session, current_session_start) = by_session
        .into_iter()
        .max_by_key(|(_, (_, _, last))| *last)
        .map(|(id, (agg, first, _))| (Some(id), agg, Some(first)))
        .unwrap_or((None, Aggregate::default(), None));

    let mut by_model: Vec<_> = by_model.into_iter().collect();
    by_model.sort_by(|a, b| b.1.cost.partial_cmp(&a.1.cost).unwrap_or(std::cmp::Ordering::Equal));

    let mut by_project: Vec<_> = by_project.into_iter().collect();
    by_project.sort_by(|a, b| b.1.cost.partial_cmp(&a.1.cost).unwrap_or(std::cmp::Ordering::Equal));

    Ok(Snapshot {
        total,
        today,
        last_hour,
        by_model,
        by_project,
        last_activity,
        session_count,
        computed_at: now,
        window_5h,
        window_5h_start,
        window_5h_sonnet,
        window_5h_sonnet_start,
        window_week,
        window_week_start,
        current_session,
        current_session_id,
        current_session_start,
        month_to_date,
    })
}

fn is_sonnet(model: &str) -> bool {
    model.to_ascii_lowercase().contains("sonnet")
}

fn first_of_month(now: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::{Datelike, NaiveDate};
    let d = now.date_naive();
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1)
        .and_then(|nd| nd.and_hms_opt(0, 0, 0))
        .map(|ndt| ndt.and_utc())
        .unwrap_or(now)
}

fn short_project_name(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(cwd)
        .to_string()
}
