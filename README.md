# claude-usage-monitor

A small Rust TUI that watches your Claude Code usage and refreshes every 1, 5, or 10 minutes.

It combines two sources:

1. **Local-derived stats** — parses `~/.claude/projects/**/*.jsonl` (the session logs Claude Code writes) to compute API-equivalent cost, token breakdowns, rolling 5-hour and 7-day windows, and a Sonnet-only slice.
2. **Live subscription quota** — calls `GET /api/oauth/usage` (the same undocumented endpoint Claude Code's `/usage` slash command uses) to show real percentage utilization and reset times for your Claude.ai Pro/Max plan. Credentials are read from the macOS Keychain (`Claude Code-credentials`) or `~/.claude/.credentials.json`.

## Build & run

```sh
cargo build --release
./target/release/claude-usage-monitor
```

## Tabs

- **Overview** — all-time / today / last-hour aggregates, plus top models and projects
- **Limits** — live plan quota at the top, then API-equivalent subscription value (month-to-date), then local-derived rolling windows (5h all models / 5h Sonnet / current session / weekly)
- **Config** — set your subscription plan (Pro / Max 5× / Max 20× / Team / Custom) and optional dollar limits for the rolling windows. Stored in `~/.config/claude-usage-monitor/config.json`. Env-var seeds (`CLAUDE_PLAN`, `CLAUDE_PLAN_COST_USD`, `CLAUDE_5H_LIMIT_USD`, `CLAUDE_WEEK_LIMIT_USD`) are honored on first run.

## Keys

| Key       | Action                                                |
|-----------|-------------------------------------------------------|
| `Tab` / `t` | switch tab                                          |
| `1` / `5` / `0` | refresh interval: 1 / 5 / 10 minutes            |
| `r`       | refresh now                                           |
| `q` / `Esc` | quit                                                |

In the Config tab: `↑/↓` or `j/k` move, `Enter`/`Space` cycles plan or starts a numeric edit, `x`/`Del` clears, `Enter` saves edits, `Esc` cancels.

## Caveats

- "Cost" is **API-equivalent** (pay-as-you-go pricing applied to your token counts). It is **not** what your Claude.ai subscription is billing you. The Subscription Value gauge expresses the ratio of API-equivalent spend to your plan's monthly cost.
- The live-quota endpoint is undocumented; Anthropic may change or remove it without notice. The local-derived panels work without it.
- OAuth token refresh is not implemented. If the cached token expires, run `claude` once to refresh credentials.

## Status

Personal tool. No tests, no stability guarantees.
