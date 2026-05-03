use chrono::{DateTime, Duration as ChronoDuration, Utc};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table, Tabs},
    Frame,
};

use crate::app::{App, ConfigField, Tab};
use crate::config::{Config, PlanKind};
use crate::quota::QuotaWindow;
use crate::usage::{Aggregate, Snapshot};

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_header(f, chunks[0], app);
    draw_tabs(f, chunks[1], app);

    match app.tab {
        Tab::Overview => draw_overview(f, chunks[2], app),
        Tab::Limits => draw_limits(f, chunks[2], app),
        Tab::Config => draw_config(f, chunks[2], app),
    }

    draw_footer(f, chunks[3], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let interval = format!("{} min", app.interval_minutes());
    let next_in = app
        .next_refresh_in()
        .map(|d| format!("{}s", d.as_secs()))
        .unwrap_or_else(|| "—".into());
    let last = app
        .snapshot
        .as_ref()
        .map(|s| s.computed_at.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "never".into());

    let line = Line::from(vec![
        Span::styled("Claude Usage Monitor", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("   refresh: "),
        Span::styled(interval, Style::default().fg(Color::Yellow)),
        Span::raw("   next in: "),
        Span::styled(next_in, Style::default().fg(Color::Green)),
        Span::raw("   last: "),
        Span::styled(last, Style::default().fg(Color::Magenta)),
    ]);

    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn draw_tabs(f: &mut Frame, area: Rect, app: &App) {
    let titles = vec!["Overview", "Limits", "Config"];
    let selected = match app.tab {
        Tab::Overview => 0,
        Tab::Limits => 1,
        Tab::Config => 2,
    };
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL))
        .select(selected)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        );
    f.render_widget(tabs, area);
}

// ── Overview tab ──────────────────────────────────────────────────────────

fn draw_overview(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(8)])
        .split(area);

    draw_summary(f, rows[0], app);
    draw_breakdowns(f, rows[1], app);
}

fn draw_summary(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

    if let Some(snap) = &app.snapshot {
        f.render_widget(agg_panel("All time", &snap.total), cols[0]);
        f.render_widget(agg_panel("Today", &snap.today), cols[1]);
        f.render_widget(agg_panel("Last hour", &snap.last_hour), cols[2]);
    } else if let Some(err) = &app.error {
        let p = Paragraph::new(format!("Error: {err}"))
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title("Status"));
        f.render_widget(p, area);
    } else {
        let p = Paragraph::new("Loading…").block(Block::default().borders(Borders::ALL));
        f.render_widget(p, area);
    }
}

fn agg_panel<'a>(title: &'a str, a: &'a Aggregate) -> Paragraph<'a> {
    let lines = vec![
        Line::from(vec![
            Span::styled("$", Style::default().fg(Color::Green)),
            Span::styled(format!("{:.4}", a.cost), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(format!("msgs:    {}", fmt_n(a.messages))),
        Line::from(format!("input:   {}", fmt_n(a.input))),
        Line::from(format!("output:  {}", fmt_n(a.output))),
        Line::from(format!("cache W: {}", fmt_n(a.cache_write))),
        Line::from(format!("cache R: {}", fmt_n(a.cache_read))),
    ];
    Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title))
}

fn draw_breakdowns(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let Some(snap) = &app.snapshot else { return };

    f.render_widget(table("By model", &snap.by_model, 12), cols[0]);
    f.render_widget(table("By project", &snap.by_project, 12), cols[1]);
}

fn table<'a>(title: &'a str, rows: &'a [(String, Aggregate)], limit: usize) -> Table<'a> {
    let header = Row::new(vec![
        Cell::from("name"),
        Cell::from("msgs"),
        Cell::from("tokens"),
        Cell::from("cost $"),
    ])
    .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    let body: Vec<Row> = rows
        .iter()
        .take(limit)
        .map(|(name, a)| {
            Row::new(vec![
                Cell::from(truncate(name, 36)),
                Cell::from(fmt_n(a.messages)),
                Cell::from(fmt_n(a.total_tokens())),
                Cell::from(format!("{:.4}", a.cost)),
            ])
        })
        .collect();

    Table::new(
        body,
        [
            Constraint::Min(20),
            Constraint::Length(8),
            Constraint::Length(12),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(title))
}

// ── Limits tab (mirrors `/usage`) ─────────────────────────────────────────

fn draw_limits(f: &mut Frame, area: Rect, app: &App) {
    let Some(snap) = &app.snapshot else {
        let p = Paragraph::new("Loading…").block(Block::default().borders(Borders::ALL));
        f.render_widget(p, area);
        return;
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),  // live plan quota (real subscription data)
            Constraint::Length(7),  // subscription value (API-equivalent)
            Constraint::Length(6),  // 5h all models (local-derived)
            Constraint::Length(6),  // 5h sonnet (local-derived)
            Constraint::Length(6),  // current session (5h-anchored)
            Constraint::Min(0),     // weekly
        ])
        .split(area);

    draw_live_quota_block(f, rows[0], app);
    draw_subscription_block(f, rows[1], snap, &app.config);
    draw_window_block(
        f,
        rows[2],
        "5-hour window — all models (local-derived)",
        &snap.window_5h,
        snap.window_5h_start,
        ChronoDuration::hours(5),
        app.config.limit_5h_usd,
    );
    draw_window_block(
        f,
        rows[3],
        "5-hour window — Sonnet only (local-derived)",
        &snap.window_5h_sonnet,
        snap.window_5h_sonnet_start,
        ChronoDuration::hours(5),
        app.config.limit_5h_usd,
    );
    draw_session_window_block(f, rows[4], snap);
    draw_window_block(
        f,
        rows[5],
        "Weekly window (local-derived)",
        &snap.window_week,
        snap.window_week_start,
        ChronoDuration::days(7),
        app.config.limit_week_usd,
    );
}

fn draw_live_quota_block(f: &mut Frame, area: Rect, app: &App) {
    let plan_label = app
        .credentials
        .as_ref()
        .and_then(|c| {
            let sub = c.subscription_type.as_deref()?;
            let tier = c.rate_limit_tier.as_deref().unwrap_or("");
            Some(format!("{sub} • {tier}"))
        })
        .unwrap_or_else(|| "live plan quota".into());
    let title = format!("Live plan quota — {plan_label}");
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(q) = &app.quota else {
        let msg = match &app.quota_error {
            Some(e) => Line::from(vec![
                Span::styled("error: ", Style::default().fg(Color::Red)),
                Span::raw(e.clone()),
            ]),
            None => Line::from(Span::styled(
                "fetching from api.anthropic.com/api/oauth/usage…",
                Style::default().fg(Color::DarkGray),
            )),
        };
        f.render_widget(Paragraph::new(msg), inner);
        return;
    };

    let mut lines: Vec<Line> = vec![
        quota_line("5-hour", q.five_hour.as_ref()),
        quota_line("7-day  (all models)", q.seven_day.as_ref()),
        quota_line("7-day  (Sonnet)", q.seven_day_sonnet.as_ref()),
        quota_line("7-day  (Opus)", q.seven_day_opus.as_ref()),
    ];

    if let Some(extra) = &q.extra_usage {
        if extra.is_enabled {
            let used = extra.used_credits.unwrap_or(0.0);
            let limit = extra.monthly_limit.unwrap_or(0.0);
            let cur = extra.currency.as_deref().unwrap_or("USD");
            lines.push(Line::from(vec![
                Span::styled(format!("{:<22}", "extra credits"), Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("{used:.2}/{limit:.2} {cur}"),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
        }
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn quota_line<'a>(label: &str, w: Option<&'a QuotaWindow>) -> Line<'a> {
    match w {
        None => Line::from(vec![
            Span::styled(format!("{label:<22}"), Style::default().fg(Color::DarkGray)),
            Span::styled("—", Style::default().fg(Color::DarkGray)),
        ]),
        Some(w) => {
            let pct = w.utilization;
            let color = if pct >= 90.0 {
                Color::Red
            } else if pct >= 70.0 {
                Color::Yellow
            } else {
                Color::Green
            };
            let bar = render_bar(pct, 24);
            let resets = w
                .resets_at
                .map(|t| {
                    let secs = (t - chrono::Utc::now()).num_seconds();
                    if secs <= 0 {
                        "(reset due)".to_string()
                    } else {
                        format!("resets in {}", fmt_remaining(secs))
                    }
                })
                .unwrap_or_default();
            Line::from(vec![
                Span::styled(format!("{label:<22}"), Style::default().fg(Color::Gray)),
                Span::styled(format!("{:>5.1}% ", pct), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                Span::styled(bar, Style::default().fg(color)),
                Span::raw("  "),
                Span::styled(resets, Style::default().fg(Color::DarkGray)),
            ])
        }
    }
}

fn render_bar(pct: f64, width: usize) -> String {
    let filled = ((pct.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

fn draw_subscription_block(f: &mut Frame, area: Rect, snap: &Snapshot, cfg: &Config) {
    let block = Block::default().borders(Borders::ALL).title("Subscription value (month-to-date)");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(monthly) = cfg.plan_monthly_usd() else {
        let p = Paragraph::new(Line::from(vec![
            Span::styled("(no plan configured) ", Style::default().fg(Color::DarkGray)),
            Span::raw("press "),
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::raw(" → "),
            Span::styled("Config", Style::default().fg(Color::Cyan)),
            Span::raw(" to set one."),
        ]));
        f.render_widget(p, inner);
        return;
    };

    let mtd = &snap.month_to_date;
    let ratio = if monthly > 0.0 { mtd.cost / monthly } else { 0.0 };
    let ratio_color = if ratio < 1.0 {
        Color::Red
    } else if ratio < 2.0 {
        Color::Yellow
    } else {
        Color::Green
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let header = Line::from(vec![
        Span::styled(cfg.plan.label(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(format!("   ${:.2}/mo", monthly)),
        Span::raw("    "),
        Span::styled(format!("${:.2}", mtd.cost), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" API-equivalent so far    "),
        Span::raw(format!("{} msgs", fmt_n(mtd.messages))),
    ]);
    f.render_widget(Paragraph::new(header), rows[0]);

    let display_ratio = ratio.clamp(0.0, 1.0);
    let recouped = if ratio >= 1.0 {
        "100%+".to_string()
    } else {
        format!("{:.0}%", ratio * 100.0)
    };
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(ratio_color))
        .ratio(display_ratio)
        .label(format!("{:.1}× value  ({recouped} of plan cost recouped)", ratio));
    f.render_widget(gauge, rows[1]);
}

fn draw_session_window_block(f: &mut Frame, area: Rect, snap: &Snapshot) {
    let id = snap
        .current_session_id
        .as_deref()
        .map(|s| s.chars().take(8).collect::<String>())
        .unwrap_or_else(|| "—".into());
    let title = format!("Current session — {id}");
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let a = &snap.current_session;
    // The session shares the same 5h reset rule as a quota window: starts at the
    // first message and resets 5h later.
    let reset_in = snap
        .current_session_start
        .map(|start| {
            let reset_at = start + ChronoDuration::hours(5);
            let remaining = (reset_at - chrono::Utc::now()).num_seconds();
            if remaining <= 0 {
                "session window expired".to_string()
            } else {
                format!("session resets in {}", fmt_remaining(remaining))
            }
        })
        .unwrap_or_else(|| "no activity yet".to_string());

    let header = Line::from(vec![
        Span::styled("$", Style::default().fg(Color::Green)),
        Span::styled(
            format!("{:.4}", a.cost),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "    {} msgs    {} tokens    ",
            fmt_n(a.messages),
            fmt_n(a.total_tokens())
        )),
        Span::styled(reset_in, Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(header), rows[0]);

    let detail = Line::from(format!(
        "in {} • out {} • cache W {} • cache R {}",
        fmt_n(a.input),
        fmt_n(a.output),
        fmt_n(a.cache_write),
        fmt_n(a.cache_read),
    ));
    f.render_widget(
        Paragraph::new(detail).style(Style::default().fg(Color::DarkGray)),
        rows[1],
    );
}

fn draw_window_block(
    f: &mut Frame,
    area: Rect,
    title: &str,
    a: &Aggregate,
    window_start: Option<DateTime<Utc>>,
    window_len: ChronoDuration,
    limit_usd: Option<f64>,
) {
    let block = Block::default().borders(Borders::ALL).title(title.to_string());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let reset_in = window_start
        .map(|start| {
            let reset_at = start + window_len;
            let remaining = (reset_at - Utc::now()).num_seconds().max(0);
            format!("resets in {}", fmt_remaining(remaining))
        })
        .unwrap_or_else(|| "no activity in window".to_string());

    let header = Line::from(vec![
        Span::styled("$", Style::default().fg(Color::Green)),
        Span::styled(
            format!("{:.4}", a.cost),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("    {} msgs    {} tokens    ", fmt_n(a.messages), fmt_n(a.total_tokens()))),
        Span::styled(reset_in, Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(header), rows[0]);

    if let Some(limit) = limit_usd {
        let pct = if limit > 0.0 { (a.cost / limit).clamp(0.0, 1.0) } else { 0.0 };
        let color = if pct >= 0.9 {
            Color::Red
        } else if pct >= 0.7 {
            Color::Yellow
        } else {
            Color::Green
        };
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(color))
            .ratio(pct)
            .label(format!("{:.1}% of ${:.2}", pct * 100.0, limit));
        f.render_widget(gauge, rows[1]);
    } else {
        let p = Paragraph::new(Span::styled(
            "(no limit configured)",
            Style::default().fg(Color::DarkGray),
        ));
        f.render_widget(p, rows[1]);
    }
}

fn fmt_remaining(secs: i64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{secs}s")
    }
}

// ── Config tab ────────────────────────────────────────────────────────────

fn draw_config(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(ConfigField::ALL.len() as u16 + 4),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    draw_config_form(f, rows[0], app);
    draw_config_status(f, rows[1], app);
    draw_config_help(f, rows[2]);
}

fn draw_config_form(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("Configuration");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    for (idx, field) in ConfigField::ALL.iter().enumerate() {
        let selected = idx == app.selected_field;
        let editing = selected && app.editing;

        let marker = if selected { "▶ " } else { "  " };
        let marker_style = if selected {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let label_style = if selected {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        let value_text = if editing {
            format!("{}_", app.edit_buffer)
        } else {
            field_value(field, &app.config)
        };
        let value_style = if editing {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };

        lines.push(Line::from(vec![
            Span::styled(marker, marker_style),
            Span::styled(format!("{:<28}", field.label()), label_style),
            Span::styled(value_text, value_style),
        ]));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn field_value(field: &ConfigField, cfg: &Config) -> String {
    match field {
        ConfigField::Plan => match cfg.plan {
            PlanKind::None => "—".into(),
            PlanKind::Custom => match cfg.custom_plan_cost_usd {
                Some(c) => format!("Custom (${c:.2}/mo)"),
                None => "Custom (cost unset)".into(),
            },
            other => format!("{} (${:.2}/mo)", other.label(), other.default_cost().unwrap_or(0.0)),
        },
        ConfigField::CustomCost => match cfg.custom_plan_cost_usd {
            Some(c) => format!("${c:.2}"),
            None => "—".into(),
        },
        ConfigField::Limit5h => match cfg.limit_5h_usd {
            Some(c) => format!("${c:.2}"),
            None => "—".into(),
        },
        ConfigField::LimitWeek => match cfg.limit_week_usd {
            Some(c) => format!("${c:.2}"),
            None => "—".into(),
        },
    }
}

fn draw_config_status(f: &mut Frame, area: Rect, app: &App) {
    let text = app.config_status.clone().unwrap_or_else(|| {
        crate::config::Config::path()
            .map(|p| format!("config path: {}", p.display()))
            .unwrap_or_else(|| "no writable config dir".into())
    });
    let p = Paragraph::new(Span::styled(text, Style::default().fg(Color::DarkGray)))
        .block(Block::default().borders(Borders::ALL).title("Status"));
    f.render_widget(p, area);
}

fn draw_config_help(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(vec![
            Span::styled("↑/↓ or j/k", Style::default().fg(Color::Cyan)),
            Span::raw(" select field    "),
            Span::styled("Enter / Space", Style::default().fg(Color::Cyan)),
            Span::raw(" cycle plan or edit number    "),
            Span::styled("x / Del", Style::default().fg(Color::Cyan)),
            Span::raw(" clear field"),
        ]),
        Line::from(vec![
            Span::styled("while editing: ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" save    "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(" cancel    "),
            Span::styled("Backspace", Style::default().fg(Color::Cyan)),
            Span::raw(" delete digit"),
        ]),
        Line::from(Span::styled(
            "All changes auto-save to the config file.",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let p = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Keys"));
    f.render_widget(p, area);
}

// ── Footer ────────────────────────────────────────────────────────────────

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let activity = app
        .snapshot
        .as_ref()
        .and_then(|s| s.last_activity)
        .map(|t| {
            let secs = (Utc::now() - t).num_seconds().max(0);
            format!("last activity {}", fmt_duration(secs))
        })
        .unwrap_or_else(|| "no activity yet".into());

    let sessions = app
        .snapshot
        .as_ref()
        .map(|s| format!("sessions: {}", s.session_count))
        .unwrap_or_default();

    let line = match app.tab {
        Tab::Config => Line::from(vec![
            Span::styled("[Tab]", Style::default().fg(Color::Cyan)),
            Span::raw(" view  "),
            Span::styled("[q/Esc]", Style::default().fg(Color::Cyan)),
            Span::raw(" quit (Esc cancels edit)    "),
            Span::styled(sessions, Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled(activity, Style::default().fg(Color::DarkGray)),
        ]),
        _ => Line::from(vec![
            Span::styled("[Tab]", Style::default().fg(Color::Cyan)),
            Span::raw(" view  "),
            Span::styled("[1/5/0]", Style::default().fg(Color::Cyan)),
            Span::raw(" interval  "),
            Span::styled("[r]", Style::default().fg(Color::Cyan)),
            Span::raw(" refresh  "),
            Span::styled("[q]", Style::default().fg(Color::Cyan)),
            Span::raw(" quit    "),
            Span::styled(sessions, Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled(activity, Style::default().fg(Color::DarkGray)),
        ]),
    };
    f.render_widget(Paragraph::new(line), area);
}

fn fmt_n(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

fn fmt_duration(secs: i64) -> String {
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}
