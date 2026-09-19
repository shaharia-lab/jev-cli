//! Captures what `jev version` reports: the commit, the build date and the target triple.
//!
//! A release pipeline sets `JEV_BUILD_COMMIT` and `SOURCE_DATE_EPOCH`. Without them the commit is
//! read from git when the build happens inside a checkout, and is `unknown` otherwise (for example
//! when installing from crates.io).

use std::env;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=JEV_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=../../.git/HEAD");

    let commit = env::var("JEV_BUILD_COMMIT")
        .ok()
        .filter(|commit| !commit.trim().is_empty())
        .or_else(git_commit)
        .unwrap_or_else(|| "unknown".to_owned());
    let target = env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned());

    println!("cargo:rustc-env=JEV_BUILD_COMMIT={}", commit.trim());
    println!("cargo:rustc-env=JEV_BUILD_DATE={}", build_date());
    println!("cargo:rustc-env=JEV_BUILD_TARGET={target}");
}

fn git_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()?;
    let commit = String::from_utf8(output.stdout).ok()?;
    (output.status.success() && !commit.trim().is_empty()).then(|| commit.trim().to_owned())
}

/// The build date as `YYYY-MM-DD` in UTC, honouring `SOURCE_DATE_EPOCH` for reproducible builds.
fn build_date() -> String {
    let seconds = env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|epoch| epoch.trim().parse::<u64>().ok())
        .or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|now| now.as_secs())
        })
        .unwrap_or(0);
    let (year, month, day) = civil_from_days(seconds / 86_400);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's `civil_from_days`, for dates from
/// the epoch onwards).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}
