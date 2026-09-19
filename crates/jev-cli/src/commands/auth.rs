//! `jev auth login|status|logout`.

use std::fmt::Write as _;

use jev_client::{ApiKey, Transport};
use serde::Serialize;
use zeroize::Zeroizing;

use super::{Context, eval};
use crate::cli::{AuthCommand, LoginArgs, LogoutArgs, StatusArgs};
use crate::client;
use crate::config::{Key, Settings};
use crate::credentials::{
    API_KEY_VARIABLE, CredentialStore, Credentials, KEY_CONSOLE_URL, KeySource, fingerprint,
};
use crate::error::CliError;
use crate::notice::Notice;
use crate::output::{Cell, Render, Table, Ui};

pub(crate) fn run(command: &AuthCommand, context: &mut Context<'_>) -> Result<(), CliError> {
    match command {
        AuthCommand::Login(arguments) => login(arguments, context),
        AuthCommand::Status(arguments) => status(arguments, context),
        AuthCommand::Logout(arguments) => logout(arguments, context),
    }
}

fn login(arguments: &LoginArgs, context: &mut Context<'_>) -> Result<(), CliError> {
    let settings = context.settings()?.clone();
    let profile = settings.profile_name().to_owned();
    let key = read_key(arguments, context)?;

    if !arguments.skip_verify {
        verify(&key, &settings, context)?;
    }

    let credentials = Credentials::new(&context.env, context.store()?.dir());
    credentials.file.set(&profile, &key)?;

    if context.env.get(API_KEY_VARIABLE).is_some() {
        context.notify(
            &Notice::warning("env_overrides", format!("{API_KEY_VARIABLE} is set and takes precedence over the key that was just stored"))
                .hint(format!("unset {API_KEY_VARIABLE} to use the stored key")),
        );
    }
    let outcome = LoggedIn {
        profile,
        stored_in: credentials.file.path().display().to_string(),
        verified: !arguments.skip_verify,
        fingerprint: fingerprint(&key),
    };
    context.output.emit(&outcome, context.stdout)
}

/// Reads the key from stdin (`--with-token`) or from a prompt that does not echo.
fn read_key(arguments: &LoginArgs, context: &mut Context<'_>) -> Result<ApiKey, CliError> {
    let unusable = |error: jev_client::InvalidApiKey| {
        CliError::usage(format!("that is not a usable API key: {error}"))
            .hint(format!("keys are created at {KEY_CONSOLE_URL}"))
    };
    if arguments.with_token {
        let mut text = Zeroizing::new(String::new());
        context.stdin.read_to_string(&mut text).map_err(|error| {
            CliError::usage(format!("could not read the key from stdin: {error}"))
        })?;
        return ApiKey::new(text.to_string()).map_err(unusable);
    }
    context.interaction.ensure_can_prompt(
        "the API key",
        "pipe the key in: `printf %s \"$KEY\" | jev auth login --with-token`; or skip storing it and set TYPESAFE_API_KEY",
    )?;
    let typed = rpassword::prompt_password(format!(
        "API key for profile `{}` (input is hidden): ",
        context.settings()?.profile_name()
    ))
    .map_err(|error| {
        CliError::usage(format!("could not read the key: {error}"))
            .hint("use --with-token to read it from stdin")
    })?;
    ApiKey::new(typed).map_err(unusable)
}

/// Checks a key against the API before it is stored.
fn verify(key: &ApiKey, settings: &Settings, context: &Context<'_>) -> Result<(), CliError> {
    let (transport, notices) = client::transport(settings, context.connection, key.clone())?;
    for notice in &notices {
        context.notify(notice);
    }
    eval::list_models(&transport)
        .map(|_| ())
        .map_err(|mut error| {
            if error.exit == crate::exit::Exit::Auth {
                error.message = format!(
                    "the API rejected the key, so nothing was stored: {}",
                    error.message
                );
                error.hint = Some(format!(
                    "check the key, or create a new one at {KEY_CONSOLE_URL}"
                ));
            } else {
                error.message = format!(
                    "the key could not be verified, so nothing was stored: {}",
                    error.message
                );
                error.hint = Some(
                    "try again, or pass --skip-verify to store it without checking".to_owned(),
                );
            }
            error
        })
}

fn status(arguments: &StatusArgs, context: &mut Context<'_>) -> Result<(), CliError> {
    let settings = context.settings()?.clone();
    let credentials = Credentials::new(&context.env, context.store()?.dir());
    let profile = settings.profile_name().to_owned();
    let base_url = settings
        .get(Key::BaseUrl)
        .value
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();

    let found = credentials.find(&profile)?;
    let check = match (&found, arguments.offline) {
        (Some((key, _)), false) => Some(live_check(key, &settings, context)),
        _ => None,
    };
    let report = Status {
        profile,
        authenticated: found.is_some() && check.as_ref().is_none_or(|check| check.ok),
        source: found.as_ref().map(|(_, source)| source.name()),
        fingerprint: found.as_ref().map(|(key, _)| fingerprint(key)),
        base_url,
        check,
    };
    context.output.emit(&report, context.stdout)?;

    if report.authenticated {
        return Ok(());
    }
    Err(match report.check {
        Some(check) => CliError::auth(
            "key_rejected",
            format!(
                "the stored key did not pass the live check: {}",
                check.detail.unwrap_or_default()
            ),
        )
        .hint(format!(
            "store a working key with `jev auth login`; keys are created at {KEY_CONSOLE_URL}"
        )),
        None => credentials
            .api_key(&report.profile)
            .err()
            .unwrap_or_else(|| CliError::auth("no_api_key", "no API key is configured")),
    })
}

fn live_check(key: &ApiKey, settings: &Settings, context: &Context<'_>) -> LiveCheck {
    let outcome = client::transport(settings, context.connection, key.clone())
        .and_then(|(transport, _)| check_with(&transport));
    match outcome {
        Ok(()) => LiveCheck {
            ok: true,
            detail: None,
        },
        Err(error) => LiveCheck {
            ok: false,
            detail: Some(error.message),
        },
    }
}

fn check_with(transport: &impl Transport) -> Result<(), CliError> {
    eval::list_models(transport).map(|_| ())
}

fn logout(arguments: &LogoutArgs, context: &mut Context<'_>) -> Result<(), CliError> {
    let profiles: Vec<String> = if arguments.all {
        context
            .config_file()?
            .profile_names()
            .into_iter()
            .map(str::to_owned)
            .collect()
    } else {
        vec![context.settings()?.profile_name().to_owned()]
    };
    let credentials = Credentials::new(&context.env, context.store()?.dir());

    let mut removed = Vec::new();
    for profile in &profiles {
        if credentials.file.delete(profile)? {
            removed.push(profile.clone());
        }
    }
    if context.env.get(API_KEY_VARIABLE).is_some() {
        context.notify(
            &Notice::warning(
                "env_still_set",
                format!("{API_KEY_VARIABLE} is still set, so jev can still authenticate"),
            )
            .hint(format!(
                "unset {API_KEY_VARIABLE} to be logged out completely"
            )),
        );
    }
    context.output.emit(&LoggedOut { removed }, context.stdout)
}

/// Removes every stored key of a profile. Used when the profile itself is deleted.
pub(crate) fn forget(profile: &str, context: &Context<'_>) -> Result<(), CliError> {
    let credentials = Credentials::new(&context.env, context.store()?.dir());
    credentials.file.delete(profile).map(|_| ())
}

#[derive(Debug, Serialize)]
struct LoggedIn {
    profile: String,
    /// The credentials file the key was written to.
    stored_in: String,
    /// Whether the key was checked against the API before it was stored.
    verified: bool,
    /// The last four characters of the key. Nothing more of it is ever shown.
    fingerprint: String,
}

impl Render for LoggedIn {
    fn human(&self, _: Ui) -> String {
        let verified = if self.verified { "verified and " } else { "" };
        format!(
            "key {} {verified}stored for profile `{}` in {}\n",
            self.fingerprint, self.profile, self.stored_in
        )
    }
}

#[derive(Debug, Serialize)]
struct Status {
    profile: String,
    /// `true` when there is a key and, unless --offline, the API accepted it.
    authenticated: bool,
    /// `env` or `file`; `null` when there is no key.
    source: Option<&'static str>,
    /// The last four characters of the key. Nothing more of it is ever shown.
    fingerprint: Option<String>,
    base_url: String,
    /// The live check against the API; `null` with --offline, or when there is no key.
    check: Option<LiveCheck>,
}

#[derive(Debug, Serialize)]
struct LiveCheck {
    ok: bool,
    detail: Option<String>,
}

impl Render for Status {
    fn human(&self, ui: Ui) -> String {
        let check = match &self.check {
            Some(LiveCheck { ok: true, .. }) => "the API accepted the key".to_owned(),
            Some(LiveCheck { detail, .. }) => {
                format!("failed: {}", detail.clone().unwrap_or_default())
            }
            None if self.source.is_some() => "skipped (--offline)".to_owned(),
            None => "-".to_owned(),
        };
        let source = match self.source {
            Some(KEY_FROM_ENV) => format!("environment variable {API_KEY_VARIABLE}"),
            Some(source) => source.to_owned(),
            None => "none".to_owned(),
        };
        let mut table = Table::default();
        for (label, value) in [
            ("profile", self.profile.clone()),
            (
                "authenticated",
                if self.authenticated {
                    "yes".to_owned()
                } else {
                    "no".to_owned()
                },
            ),
            ("key source", source),
            (
                "key",
                self.fingerprint.clone().unwrap_or_else(|| "-".to_owned()),
            ),
            ("base URL", self.base_url.clone()),
            ("live check", check),
        ] {
            table.row(vec![Cell::styled(label, ui.dim(label)), Cell::plain(value)]);
        }
        table.render("")
    }
}

const KEY_FROM_ENV: &str = KeySource::Env.name();

#[derive(Debug, Serialize)]
struct LoggedOut {
    /// The profiles whose stored key was removed. Empty when there was nothing stored.
    removed: Vec<String>,
}

impl Render for LoggedOut {
    fn human(&self, _: Ui) -> String {
        if self.removed.is_empty() {
            return "no stored key to remove\n".to_owned();
        }
        let mut text = String::new();
        for profile in &self.removed {
            let _ = writeln!(text, "removed the stored key of profile `{profile}`");
        }
        text
    }
}
