//! Durations as people write them: `30`, `30s`, `500ms`, `2m`.

use std::time::Duration;

/// Parses a duration: a bare number is seconds, or a number with `ms`, `s` or `m`.
///
/// # Errors
///
/// A message, ready to show, when the text is not a positive duration.
pub(crate) fn parse(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let (number, scale) = if let Some(number) = text.strip_suffix("ms") {
        (number, 0.001)
    } else if let Some(number) = text.strip_suffix('s') {
        (number, 1.0)
    } else if let Some(number) = text.strip_suffix('m') {
        (number, 60.0)
    } else {
        (text, 1.0)
    };
    let seconds = number
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("`{text}` is not a duration such as 30, 30s, 500ms or 2m"))?;
    let duration = Duration::try_from_secs_f64(seconds * scale)
        .map_err(|_| format!("`{text}` is out of range"))?;
    if duration.is_zero() {
        return Err("the duration must be greater than zero".to_owned());
    }
    Ok(duration)
}

/// The shortest exact way to write a duration: `30s`, `500ms`, `1500ms`.
pub(crate) fn format(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{format, parse};

    #[test]
    fn takes_seconds_by_default_and_a_few_units() {
        assert_eq!(parse("30"), Ok(Duration::from_secs(30)));
        assert_eq!(parse("1.5s"), Ok(Duration::from_millis(1500)));
        assert_eq!(parse("500ms"), Ok(Duration::from_millis(500)));
        assert_eq!(parse(" 2m "), Ok(Duration::from_secs(120)));
        for wrong in ["", "soon", "-1", "0", "1h", "NaN", "1e99"] {
            assert!(parse(wrong).is_err(), "{wrong:?}");
        }
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        for duration in [
            Duration::from_secs(30),
            Duration::from_millis(500),
            Duration::from_millis(1500),
            Duration::from_secs(120),
        ] {
            assert_eq!(parse(&format(duration)), Ok(duration));
        }
        assert_eq!(format(Duration::from_secs(30)), "30s");
        assert_eq!(format(Duration::from_millis(1500)), "1500ms");
    }
}
