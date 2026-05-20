//! Cron schedule parsing and firing logic.
//!
//! Supported schedule expressions:
//! - `* * * * *` — five-field POSIX crontab (minute hour day month dow)
//! - `@hourly`, `@daily`, `@weekly`, `@monthly`, `@midnight`
//! - `@every 5m` / `@every 1h30m` — interval relative to last_run
//!
//! Decision is in `should_fire_cron`; CLI fire dispatch is in
//! `run_cron_command`. The crontab matcher is minute-resolution and uses
//! `last_run` to guard against double-fires within the same minute.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A cron-scheduled recurring task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub schedule: CronSchedule,
    /// Free-form human description ("nightly housekeeping").
    pub description: String,
    /// Shell command to execute when the job fires.
    pub command: String,
    pub enabled: bool,
    pub last_run: Option<SystemTime>,
    pub created_at: SystemTime,
}

/// Schedule expressions supported by the cron parser.
///
/// Mirrors the v132 `tengu_cron_*` syntax surface:
/// - `* * * * *` — five-field POSIX crontab (minute hour day month dow)
/// - `@hourly`   — alias for `0 * * * *`
/// - `@daily`    — alias for `0 0 * * *`
/// - `@weekly`   — alias for `0 0 * * 0`
/// - `@every 5m` / `@every 1h30m` — interval relative to last run
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CronSchedule {
    /// Five-field crontab. Field values are stored as-is; matching uses
    /// minute-resolution (the daemon polls every minute).
    Crontab {
        minute: CronField,
        hour: CronField,
        day: CronField,
        month: CronField,
        weekday: CronField,
    },
    /// Re-run when at least `period` has elapsed since `last_run`. Fires
    /// immediately when `last_run` is None.
    Every {
        #[serde(with = "duration_secs")]
        period: Duration,
    },
}

/// One field of a five-field crontab expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CronField {
    /// `*` — match anything.
    Any,
    /// Literal value (`5`).
    Exact(u32),
    /// `*/N` step — match values where `value % step == 0`.
    Step(u32),
}

impl CronField {
    fn matches(&self, value: u32) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(v) => *v == value,
            Self::Step(step) => *step > 0 && value % step == 0,
        }
    }

    fn parse(s: &str) -> Result<Self, String> {
        if s == "*" {
            return Ok(Self::Any);
        }
        if let Some(rest) = s.strip_prefix("*/") {
            let n: u32 = rest.parse().map_err(|_| format!("bad step `{s}`"))?;
            if n == 0 {
                return Err(format!("step must be > 0 (`{s}`)"));
            }
            return Ok(Self::Step(n));
        }
        let n: u32 = s.parse().map_err(|_| format!("bad cron field `{s}`"))?;
        Ok(Self::Exact(n))
    }
}

mod duration_secs {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let s = u64::deserialize(d)?;
        Ok(Duration::from_secs(s))
    }
}

/// Parse a schedule expression into a `CronSchedule`.
///
/// Accepted forms:
/// - `"* * * * *"` (and any 5-field variant where each field is `*`,
///   a literal integer, or `*/N`)
/// - `"@hourly"`, `"@daily"`, `"@weekly"`
/// - `"@every <duration>"` where duration uses `Ns/Nm/Nh/Nd` chunks
///   (e.g. `5m`, `1h30m`, `2d`).
pub fn parse_schedule(expr: &str) -> Result<CronSchedule, String> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err("empty schedule".into());
    }

    // Aliases.
    let aliased = match trimmed {
        "@hourly" => Some("0 * * * *"),
        "@daily" | "@midnight" => Some("0 0 * * *"),
        "@weekly" => Some("0 0 * * 0"),
        "@monthly" => Some("0 0 1 * *"),
        _ => None,
    };
    if let Some(replacement) = aliased {
        return parse_schedule(replacement);
    }

    // @every N{s,m,h,d}
    if let Some(rest) = trimmed.strip_prefix("@every ") {
        let period = parse_duration_spec(rest.trim())?;
        if period.is_zero() {
            return Err("@every period must be > 0".into());
        }
        return Ok(CronSchedule::Every { period });
    }

    // Five-field crontab.
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "expected 5 cron fields or `@<alias>`, got `{expr}`"
        ));
    }
    Ok(CronSchedule::Crontab {
        minute: CronField::parse(fields[0])?,
        hour: CronField::parse(fields[1])?,
        day: CronField::parse(fields[2])?,
        month: CronField::parse(fields[3])?,
        weekday: CronField::parse(fields[4])?,
    })
}

/// Parse `"5m"`, `"1h30m"`, `"2d"`, `"45s"` etc. into a `Duration`.
fn parse_duration_spec(s: &str) -> Result<Duration, String> {
    let mut total = Duration::ZERO;
    let mut num = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
            continue;
        }
        if num.is_empty() {
            return Err(format!("bad duration `{s}`: unit `{ch}` without number"));
        }
        let n: u64 = num.parse().map_err(|_| format!("bad duration `{s}`"))?;
        let chunk = match ch {
            's' => Duration::from_secs(n),
            'm' => Duration::from_secs(n * 60),
            'h' => Duration::from_secs(n * 3600),
            'd' => Duration::from_secs(n * 86_400),
            _ => return Err(format!("unknown duration unit `{ch}`")),
        };
        total += chunk;
        num.clear();
    }
    if !num.is_empty() {
        // Bare number — assume seconds for compatibility with `@every 30`.
        let n: u64 = num.parse().map_err(|_| format!("bad duration `{s}`"))?;
        total += Duration::from_secs(n);
    }
    Ok(total)
}

// ─────────────────────────────────────────────────────────────────────────────
// Firing logic
// ─────────────────────────────────────────────────────────────────────────────

/// Decide whether a cron job should fire at `now`.
///
/// `Every { period }` fires immediately if `last_run` is `None`, then
/// re-fires once at least `period` has elapsed.
///
/// `Crontab { … }` fires at most once per minute, when the minute /
/// hour / day / month / weekday fields all match the local-time
/// components of `now`. The "at most once" guard uses `last_run` so a
/// 30-second poll loop can't fire the same minute twice.
pub fn should_fire_cron(job: &CronJob, now: SystemTime) -> bool {
    match &job.schedule {
        CronSchedule::Every { period } => match job.last_run {
            None => true,
            Some(last) => {
                // `duration_since` errors if `now < last` (system clock went
                // backward). Saturate to ZERO and warn — a clock skew should
                // not silently re-fire jobs nor crash the daemon.
                match now.duration_since(last) {
                    Ok(elapsed) => elapsed >= *period,
                    Err(_) => {
                        tracing::warn!(
                            target: "jfc::daemon",
                            "clock skew detected: now < last_run for cron job {}",
                            job.id
                        );
                        false
                    }
                }
            }
        },
        CronSchedule::Crontab {
            minute,
            hour,
            day,
            month,
            weekday,
        } => {
            let parts = match local_parts(now) {
                Some(p) => p,
                None => return false,
            };
            if !minute.matches(parts.minute) {
                return false;
            }
            if !hour.matches(parts.hour) {
                return false;
            }
            if !day.matches(parts.day) {
                return false;
            }
            if !month.matches(parts.month) {
                return false;
            }
            if !weekday.matches(parts.weekday) {
                return false;
            }
            // Don't refire within the same minute.
            if let Some(last) = job.last_run {
                let last_parts = match local_parts(last) {
                    Some(p) => p,
                    None => return true,
                };
                if last_parts.same_minute(&parts) {
                    return false;
                }
            }
            true
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LocalParts {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    /// 0 = Sunday … 6 = Saturday.
    weekday: u32,
}

impl LocalParts {
    fn same_minute(&self, other: &Self) -> bool {
        self.year == other.year
            && self.month == other.month
            && self.day == other.day
            && self.hour == other.hour
            && self.minute == other.minute
    }
}

/// Decompose `t` into local-time year/month/day/hour/minute/weekday using
/// chrono. Returns `None` if `t` predates the UNIX epoch.
fn local_parts(t: SystemTime) -> Option<LocalParts> {
    use chrono::{Datelike, Local, TimeZone, Timelike};
    let secs = t.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    let dt = Local.timestamp_opt(secs, 0).single()?;
    Some(LocalParts {
        year: dt.year(),
        month: dt.month(),
        day: dt.day(),
        hour: dt.hour(),
        minute: dt.minute(),
        weekday: dt.weekday().num_days_from_sunday(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Pretty-printers (used by `daemon list` rendering)
// ─────────────────────────────────────────────────────────────────────────────

pub(super) fn describe_schedule(s: &CronSchedule) -> String {
    match s {
        CronSchedule::Every { period } => format!("@every {}s", period.as_secs()),
        CronSchedule::Crontab {
            minute,
            hour,
            day,
            month,
            weekday,
        } => format!(
            "{} {} {} {} {}",
            field_str(minute),
            field_str(hour),
            field_str(day),
            field_str(month),
            field_str(weekday),
        ),
    }
}

pub(super) fn field_str(f: &CronField) -> String {
    match f {
        CronField::Any => "*".to_string(),
        CronField::Exact(n) => n.to_string(),
        CronField::Step(n) => format!("*/{n}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CLI dispatch
// ─────────────────────────────────────────────────────────────────────────────

pub(super) async fn run_cron_command(job: &CronJob) -> std::io::Result<()> {
    use tokio::process::Command;
    let status = Command::new("bash")
        .arg("-c")
        .arg(&job.command)
        .status()
        .await?;
    tracing::info!(
        target: "jfc::daemon",
        cron_id = %job.id,
        exit = ?status.code(),
        "cron command exited"
    );
    Ok(())
}
