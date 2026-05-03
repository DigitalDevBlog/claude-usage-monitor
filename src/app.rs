use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::{Config, PlanKind};
use crate::quota::{self, Credentials, QuotaResponse};
use crate::usage::{self, Snapshot};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Interval {
    One,
    Five,
    Ten,
}

impl Interval {
    pub fn minutes(self) -> u64 {
        match self {
            Interval::One => 1,
            Interval::Five => 5,
            Interval::Ten => 10,
        }
    }
    pub fn as_duration(self) -> Duration {
        Duration::from_secs(self.minutes() * 60)
    }
}

pub enum Msg {
    Snapshot(Box<Snapshot>),
    Error(String),
    Quota(Box<QuotaResponse>, Credentials),
    QuotaError(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Limits,
    Config,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConfigField {
    Plan,
    CustomCost,
    Limit5h,
    LimitWeek,
}

impl ConfigField {
    pub const ALL: [ConfigField; 4] = [
        ConfigField::Plan,
        ConfigField::CustomCost,
        ConfigField::Limit5h,
        ConfigField::LimitWeek,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ConfigField::Plan => "Subscription plan",
            ConfigField::CustomCost => "Custom plan cost (USD/mo)",
            ConfigField::Limit5h => "5-hour limit (USD)",
            ConfigField::LimitWeek => "Weekly limit (USD)",
        }
    }
}

pub struct App {
    pub snapshot: Option<Snapshot>,
    pub error: Option<String>,
    pub quota: Option<QuotaResponse>,
    pub quota_error: Option<String>,
    pub credentials: Option<Credentials>,
    pub interval: Interval,
    pub tab: Tab,
    pub config: Config,
    pub config_status: Option<String>,
    pub selected_field: usize,
    pub editing: bool,
    pub edit_buffer: String,
    pub should_quit: bool,
    last_refresh: Instant,
    in_flight: bool,
    tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
}

impl App {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            snapshot: None,
            error: None,
            quota: None,
            quota_error: None,
            credentials: None,
            interval: Interval::Five,
            tab: Tab::Overview,
            config: Config::load_or_seed(),
            config_status: None,
            selected_field: 0,
            editing: false,
            edit_buffer: String::new(),
            should_quit: false,
            last_refresh: Instant::now() - Duration::from_secs(3600),
            in_flight: false,
            tx,
            rx,
        };
        app.trigger_refresh();
        app
    }

    pub fn cycle_tab(&mut self) {
        self.tab = match self.tab {
            Tab::Overview => Tab::Limits,
            Tab::Limits => Tab::Config,
            Tab::Config => Tab::Overview,
        };
        // Cancel any in-progress edit on tab switch.
        self.editing = false;
        self.edit_buffer.clear();
    }

    pub fn interval_minutes(&self) -> u64 {
        self.interval.minutes()
    }

    pub fn set_interval(&mut self, i: Interval) {
        self.interval = i;
        self.trigger_refresh();
    }

    pub fn next_refresh_in(&self) -> Option<Duration> {
        let elapsed = self.last_refresh.elapsed();
        let target = self.interval.as_duration();
        if elapsed >= target {
            Some(Duration::ZERO)
        } else {
            Some(target - elapsed)
        }
    }

    pub fn tick(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Snapshot(s) => {
                    self.snapshot = Some(*s);
                    self.error = None;
                    self.in_flight = false;
                }
                Msg::Error(e) => {
                    self.error = Some(e);
                    self.in_flight = false;
                }
                Msg::Quota(q, creds) => {
                    self.quota = Some(*q);
                    self.credentials = Some(creds);
                    self.quota_error = None;
                }
                Msg::QuotaError(e) => self.quota_error = Some(e),
            }
        }
        if !self.in_flight && self.last_refresh.elapsed() >= self.interval.as_duration() {
            self.trigger_refresh();
        }
    }

    pub fn trigger_refresh(&mut self) {
        if self.in_flight {
            return;
        }
        self.in_flight = true;
        self.last_refresh = Instant::now();
        let tx = self.tx.clone();
        thread::spawn(move || {
            // Local JSONL parse.
            let snap_msg = match usage::projects_dir() {
                Some(dir) => match usage::collect(&dir) {
                    Ok(s) => Msg::Snapshot(Box::new(s)),
                    Err(e) => Msg::Error(e.to_string()),
                },
                None => Msg::Error("could not locate ~/.claude/projects".into()),
            };
            let _ = tx.send(snap_msg);

            // Live OAuth quota fetch — best-effort; failure just leaves the panel empty.
            match quota::load_credentials() {
                Ok(creds) => match quota::fetch_quota(&creds) {
                    Ok(q) => {
                        let _ = tx.send(Msg::Quota(Box::new(q), creds));
                    }
                    Err(e) => {
                        let _ = tx.send(Msg::QuotaError(e.to_string()));
                    }
                },
                Err(e) => {
                    let _ = tx.send(Msg::QuotaError(format!("no credentials: {e}")));
                }
            }
        });
    }

    // ── Config tab actions ────────────────────────────────────────────────

    pub fn current_field(&self) -> ConfigField {
        ConfigField::ALL[self.selected_field.min(ConfigField::ALL.len() - 1)]
    }

    pub fn select_prev_field(&mut self) {
        if self.editing {
            return;
        }
        self.selected_field = (self.selected_field + ConfigField::ALL.len() - 1) % ConfigField::ALL.len();
    }

    pub fn select_next_field(&mut self) {
        if self.editing {
            return;
        }
        self.selected_field = (self.selected_field + 1) % ConfigField::ALL.len();
    }

    /// Cycles plan or starts a numeric edit on the current field.
    pub fn activate_field(&mut self) {
        match self.current_field() {
            ConfigField::Plan => {
                self.config.plan = self.config.plan.cycle_next();
                self.persist();
            }
            ConfigField::CustomCost | ConfigField::Limit5h | ConfigField::LimitWeek => {
                self.editing = true;
                self.edit_buffer = match self.current_field() {
                    ConfigField::CustomCost => fmt_opt(self.config.custom_plan_cost_usd),
                    ConfigField::Limit5h => fmt_opt(self.config.limit_5h_usd),
                    ConfigField::LimitWeek => fmt_opt(self.config.limit_week_usd),
                    _ => String::new(),
                };
            }
        }
    }

    pub fn clear_field(&mut self) {
        if self.editing {
            return;
        }
        match self.current_field() {
            ConfigField::Plan => self.config.plan = PlanKind::None,
            ConfigField::CustomCost => self.config.custom_plan_cost_usd = None,
            ConfigField::Limit5h => self.config.limit_5h_usd = None,
            ConfigField::LimitWeek => self.config.limit_week_usd = None,
        }
        self.persist();
    }

    pub fn edit_push(&mut self, c: char) {
        if !self.editing {
            return;
        }
        if c.is_ascii_digit() || (c == '.' && !self.edit_buffer.contains('.')) {
            self.edit_buffer.push(c);
        }
    }

    pub fn edit_backspace(&mut self) {
        if self.editing {
            self.edit_buffer.pop();
        }
    }

    pub fn edit_cancel(&mut self) {
        self.editing = false;
        self.edit_buffer.clear();
    }

    pub fn edit_commit(&mut self) {
        if !self.editing {
            return;
        }
        let parsed: Option<f64> = if self.edit_buffer.trim().is_empty() {
            None
        } else {
            self.edit_buffer.parse::<f64>().ok()
        };
        match self.current_field() {
            ConfigField::CustomCost => self.config.custom_plan_cost_usd = parsed,
            ConfigField::Limit5h => self.config.limit_5h_usd = parsed,
            ConfigField::LimitWeek => self.config.limit_week_usd = parsed,
            ConfigField::Plan => {} // not editable via buffer
        }
        self.editing = false;
        self.edit_buffer.clear();
        self.persist();
    }

    fn persist(&mut self) {
        match self.config.save() {
            Ok(p) => self.config_status = Some(format!("saved → {}", p.display())),
            Err(e) => self.config_status = Some(format!("save failed: {e}")),
        }
    }
}

fn fmt_opt(v: Option<f64>) -> String {
    v.map(|n| format!("{n}")).unwrap_or_default()
}
