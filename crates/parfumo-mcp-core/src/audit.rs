//! Reading the audit log from the command line: the `audit` subcommand of both binaries.

use crate::db::{self, AuditFilter, Db};

/// What the `audit` subcommand was asked for.
#[derive(Debug, Clone, Default)]
pub struct AuditOptions {
    /// Only this user.
    pub user: Option<String>,
    /// Only this tool, for example `parfumo_send_message`.
    pub tool: Option<String>,
    /// Only entries newer than this: `90m`, `24h`, `7d`.
    pub since: Option<String>,
    /// Only calls that really sent a write to Parfumo (no read, no dry run).
    pub writes: bool,
    /// Only calls that ended in an error.
    pub errors: bool,
    /// Per-tool counters instead of the entries.
    pub stats: bool,
    /// One JSON object per line.
    pub json: bool,
    pub limit: u32,
}

/// `90m`, `24h`, `7d` → seconds.
pub fn parse_since(spec: &str) -> Result<i64, String> {
    const HELP: &str = "--since takes a number of minutes, hours or days, for example 90m, 24h or 7d";
    let spec = spec.trim();
    let split = spec.char_indices().next_back().map_or(0, |(i, _)| i);
    let (digits, unit) = spec.split_at(split);
    let per = match unit {
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return Err(HELP.into()),
    };
    digits.parse::<i64>().ok().filter(|n| *n > 0).map(|n| n * per).ok_or_else(|| HELP.into())
}

/// A database timestamp as `YYYY-MM-DD HH:MM UTC`.
pub fn db_time(ts: i64) -> String {
    let secs = ts.rem_euclid(86_400);
    format!("{} {:02}:{:02} UTC", parfumo_client::parse::iso_date(ts), secs / 3600, secs % 3600 / 60)
}

/// The lines the `audit` subcommand prints, latest entry first.
pub fn report(db: &Db, options: &AuditOptions) -> Result<Vec<String>, String> {
    let since = options.since.as_deref().map(parse_since).transpose()?.map(|secs| db::now() - secs);
    let mut lines = Vec::new();
    if options.stats {
        if !options.json {
            lines.push(format!("{:<36} {:>7} {:>7} {:>7} {:>9} {:>9}", "tool", "calls", "errors", "writes", "avg ms", "max ms"));
        }
        for s in db.audit_stats(options.user.as_deref(), since.unwrap_or(0)).map_err(|e| e.to_string())? {
            lines.push(if options.json {
                serde_json::json!({"tool": s.tool, "calls": s.calls, "errors": s.errors, "writes": s.writes, "avg_ms": s.avg_ms.round(), "max_ms": s.max_ms})
                    .to_string()
            } else {
                format!("{:<36} {:>7} {:>7} {:>7} {:>9.0} {:>9}", s.tool, s.calls, s.errors, s.writes, s.avg_ms, s.max_ms)
            });
        }
        return Ok(lines);
    }
    let filter =
        AuditFilter { user: options.user.clone(), tool: options.tool.clone(), since, writes_only: options.writes, errors_only: options.errors };
    for r in db.audit_query(&filter, options.limit).map_err(|e| e.to_string())? {
        lines.push(if options.json {
            serde_json::json!({"ts": r.ts, "time": db_time(r.ts), "user": r.user_name, "tool": r.tool, "result": r.result, "writes": r.writes, "duration_ms": r.duration_ms, "target": r.target})
                .to_string()
        } else {
            format!("{}\t{}\t{}\t{}\twrites={}\t{}ms\t{}", db_time(r.ts), r.user_name, r.tool, r.result, r.writes, r.duration_ms, r.target)
        });
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_is_minutes_hours_or_days() {
        assert_eq!(parse_since("90m"), Ok(5400));
        assert_eq!(parse_since(" 24h "), Ok(86_400));
        assert_eq!(parse_since("7d"), Ok(604_800));
        for bad in ["", "7", "d", "0d", "-1h", "7w", "1.5h", "7é"] {
            assert!(parse_since(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn times_are_utc_minutes() {
        assert_eq!(db_time(0), "1970-01-01 00:00 UTC");
        assert_eq!(db_time(86_400 + 3_660), "1970-01-02 01:01 UTC");
    }
}
