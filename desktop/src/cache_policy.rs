//! Default cache expiry for the player, with diagnostic overrides.
use std::time::Duration;

const DEFAULT_IDLE_SECONDS: u64 = 90;

pub(crate) fn idle_limit(variable: &str) -> Option<Duration> {
    let value = std::env::var(variable);
    let seconds = match &value {
        Ok(value) => parse_idle_seconds(Some(value)),
        Err(std::env::VarError::NotPresent) => parse_idle_seconds(None),
        Err(std::env::VarError::NotUnicode(_)) => None,
    };
    match seconds {
        Some(0) => None,
        Some(seconds) => Some(Duration::from_secs(seconds)),
        None => {
            tracing::warn!(variable, "Invalid cache idle limit; disabling expiry");
            None
        }
    }
}

fn parse_idle_seconds(value: Option<&str>) -> Option<u64> {
    match value {
        None => Some(DEFAULT_IDLE_SECONDS),
        Some(value) => value
            .parse::<u64>()
            .ok()
            .filter(|seconds| *seconds <= 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_idle_seconds;

    #[test]
    fn ordinary_launch_and_explicit_opt_out_have_distinct_policies() {
        assert_eq!(parse_idle_seconds(None), Some(90));
        assert_eq!(parse_idle_seconds(Some("0")), Some(0));
        assert_eq!(parse_idle_seconds(Some("90")), Some(90));
        assert_eq!(parse_idle_seconds(Some("86400")), Some(86400));
    }

    #[test]
    fn invalid_overrides_cannot_create_an_unbounded_cache_lifetime() {
        for value in ["", "-1", "90seconds", "86401", "18446744073709551616"] {
            assert_eq!(parse_idle_seconds(Some(value)), None);
        }
    }
}
