//! Telling a coding agent's rate or usage limit apart from a real failure, and how long to wait.
//!
//! A mission that hits a limit should not burn an attempt, go to the judge with half-finished work,
//! or pause for the operator: nothing is wrong with the work, the agent just has to wait. The driver
//! asks [`detect`] about every job that fails, and if it says "rate limited" it parks the step and
//! re-runs it once [`wait_for`] has passed.

use chrono::{DateTime, Duration, Utc};

/// Phrases agent CLIs and their APIs use for a limit. Lower-case; matched against lower-cased text.
const MARKERS: &[&str] = &[
    "rate limit",
    "rate-limit",
    "rate_limit",
    "ratelimit",
    "usage limit",
    "usage_limit",
    "too many requests",
    "429",
    "quota exceeded",
    "exceeded your current quota",
    "resource_exhausted",
    "resource exhausted",
    "overloaded",
    "hit your limit",
    "limit reached",
    "limit will reset",
    "try again later",
];

/// The limit message, when `text` reads like one.
pub fn detect<'a>(texts: impl IntoIterator<Item = &'a str>) -> Option<String> {
    texts.into_iter().find_map(|text| {
        let lower = text.to_lowercase();
        MARKERS
            .iter()
            .any(|m| lower.contains(m))
            .then(|| text.trim().chars().take(300).collect())
    })
}

/// Back-off for the `streak`-th consecutive limit: 5, 15, 30, then every 60 minutes.
fn backoff(streak: u32) -> Duration {
    Duration::minutes(match streak {
        0 | 1 => 5,
        2 => 15,
        3 => 30,
        _ => 60,
    })
}

/// When to try again: the delay the message names ("try again in 23 minutes", "retry after 120s"),
/// plus a minute of slack, or else the back-off for this streak. Never less than a minute.
pub fn wait_for(message: &str, streak: u32, now: DateTime<Utc>) -> DateTime<Utc> {
    let named = named_delay(message).map(|d| d + Duration::minutes(1));
    let delay = named.unwrap_or_else(|| backoff(streak));
    now + delay.max(Duration::minutes(1)).min(Duration::hours(6))
}

/// A relative delay in the message: a number followed by a time unit, after "in" or "after".
fn named_delay(message: &str) -> Option<Duration> {
    let lower = message.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
        .filter(|w| !w.is_empty())
        .collect();
    for (i, w) in words.iter().enumerate() {
        if *w != "in" && *w != "after" {
            continue;
        }
        let Some(next) = words.get(i + 1) else { continue };
        // "23 minutes" or "23m" / "120s" / "2h".
        let digits: String = next.chars().take_while(|c| c.is_ascii_digit()).collect();
        let Ok(n) = digits.parse::<i64>() else { continue };
        let attached = &next[digits.len()..];
        let unit = if attached.is_empty() {
            words.get(i + 2).copied().unwrap_or_default()
        } else {
            attached
        };
        let d = match unit.trim_end_matches('.') {
            u if u.starts_with("sec") || u == "s" => Duration::seconds(n),
            u if u.starts_with("min") || u == "m" => Duration::minutes(n),
            u if u.starts_with("hour") || u.starts_with("hr") || u == "h" => Duration::hours(n),
            _ => continue,
        };
        return Some(d);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_limits_and_ignores_real_failures() {
        assert!(detect(["Claude AI usage limit reached|1759262400"]).is_some());
        assert!(detect(["HTTP 429 Too Many Requests"]).is_some());
        assert!(detect(["Error: rate_limit_exceeded, retry after 30s"]).is_some());
        assert!(detect(["cargo test failed: 3 tests failed"]).is_none());
        assert!(detect(["", "job 12 ended Failed"]).is_none());
    }

    #[test]
    fn uses_the_delay_the_message_names() {
        let now = Utc::now();
        assert_eq!(wait_for("try again in 23 minutes", 1, now), now + Duration::minutes(24));
        assert_eq!(wait_for("retry after 120s", 1, now), now + Duration::minutes(3));
        assert_eq!(wait_for("resets in 2h", 1, now), now + Duration::minutes(121));
    }

    #[test]
    fn otherwise_backs_off_up_to_hourly() {
        let now = Utc::now();
        assert_eq!(wait_for("usage limit reached", 1, now), now + Duration::minutes(5));
        assert_eq!(wait_for("usage limit reached", 2, now), now + Duration::minutes(15));
        assert_eq!(wait_for("usage limit reached", 3, now), now + Duration::minutes(30));
        assert_eq!(wait_for("usage limit reached", 9, now), now + Duration::minutes(60));
    }
}
