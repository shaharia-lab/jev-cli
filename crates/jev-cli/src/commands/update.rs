//! `jev update`, and the automatic update around every other command.

use semver::Version;
use serde::Serialize;

use super::Context;
use crate::cli::{Command, UpdateArgs};
use crate::error::CliError;
use crate::exit::Exit;
use crate::notice::Notice;
use crate::output::{Render, Ui};
use crate::update::auto::{self, StateFile};
use crate::update::{
    self, Channel, GitHubReleases, InstallMethod, Installation, MINIMUM_VERSION, Newest, Release,
    TrustedKeys, UpdateSource,
};

pub(crate) fn run(arguments: &UpdateArgs, context: &mut Context<'_>) -> Result<Exit, CliError> {
    if arguments.background {
        return background(context);
    }
    let installation = Installation::current()?;
    let method = InstallMethod::detect(&installation);
    let current = update::current_version(&context.env);
    let report = Report {
        status: Status::UpToDate,
        current_version: current.to_string(),
        version: None,
        install_method: method,
        path: installation.exe().display().to_string(),
        release_url: None,
        command: None,
    };

    if arguments.rollback {
        return rollback(&installation, report, context);
    }
    if let Some(version) = &arguments.version
        && *version < MINIMUM_VERSION
    {
        return Err(CliError::usage(format!(
            "jev {version} is older than {MINIMUM_VERSION}, the oldest version this jev installs"
        ))
        .hint("releases before it are known to have problems; pick a newer one"));
    }
    // A package manager owns the binary: say how to update it, and change nothing. There is
    // nothing to look up for that, so nothing is sent.
    if method.package_manager().is_some() && !arguments.check {
        return emit(
            context,
            &Report {
                status: Status::Managed,
                version: arguments.version.as_ref().map(ToString::to_string),
                command: method.upgrade_command(arguments.version.as_ref()),
                ..report
            },
        );
    }

    let source = GitHubReleases::new(&context.env, Channel::from_settings(context.settings()))?;
    let release = match &arguments.version {
        Some(version) => source.release(version)?.ok_or_else(|| {
            CliError::usage(format!("there is no published release of jev {version}")).hint(
                format!(
                    "the releases are listed at https://github.com/{}/releases",
                    update::REPOSITORY
                ),
            )
        })?,
        None => match update::newest(&source, &current)? {
            Newest::NoRelease => return emit(context, &report),
            Newest::Available(release) => release,
            Newest::UpToDate(release) => {
                if release.version < current {
                    context.notify(&refused_downgrade(&current, &release, method));
                }
                return emit(context, &Report::about(report, &release, None));
            }
        },
    };

    if arguments.check {
        let command = method
            .upgrade_command(None)
            .unwrap_or_else(|| "jev update".to_owned());
        let available = Report {
            status: Status::UpdateAvailable,
            ..report
        };
        emit(context, &Report::about(available, &release, Some(command)))?;
        return Ok(Exit::UpdateAvailable);
    }
    if release.version == current {
        return emit(context, &Report::about(report, &release, None));
    }

    install(&source, &release, &installation, context)?;
    let updated = Report {
        status: Status::Updated,
        ..report
    };
    emit(context, &Report::about(updated, &release, None))
}

/// `jev update --rollback`.
fn rollback(
    installation: &Installation,
    report: Report,
    context: &mut Context<'_>,
) -> Result<Exit, CliError> {
    if let Some(manager) = report.install_method.package_manager() {
        return Err(CliError::usage(format!(
            "this jev was installed with {manager}, so jev does not keep a previous version"
        ))
        .hint(format!(
            "install the version you want with {manager}; `jev update --version <x.y.z>` prints \
the command"
        )));
    }
    let restored = installation.rollback(&|exe| update::probe(exe, None))?;
    emit(
        context,
        &Report {
            status: Status::RolledBack,
            version: Some(restored.to_string()),
            ..report
        },
    )
}

/// The warning for a `jev` newer than the latest release, which is never downgraded by itself.
fn refused_downgrade(current: &Version, latest: &Release, method: InstallMethod) -> Notice {
    let command = method
        .upgrade_command(Some(&latest.version))
        .unwrap_or_else(|| format!("jev update --version {}", latest.version));
    Notice::warning(
        "newer_than_latest",
        format!(
            "this jev ({current}) is newer than the latest release ({}); jev never downgrades by \
itself",
            latest.version
        ),
    )
    .hint(format!("to move to it anyway, run `{command}`"))
}

/// Downloads, verifies, stages and swaps in `release`.
fn install(
    source: &dyn UpdateSource,
    release: &Release,
    installation: &Installation,
    context: &Context<'_>,
) -> Result<(), CliError> {
    tracing::info!(version = %release.version, "downloading and verifying");
    update::stage(
        source,
        release,
        &TrustedKeys::for_run(&context.env),
        installation,
    )?;
    tracing::info!(version = %release.version, "verified; swapping it in");
    installation.apply(&release.version, &|exe| {
        update::probe(exe, Some(&release.version)).map(|_| ())
    })
}

/// `jev update --background`: the automatic check, started by `jev` itself after a command, with
/// no standard streams. It stages a newer release and records what it found; it prints nothing,
/// and a failure waits for the next check.
fn background(context: &Context<'_>) -> Result<Exit, CliError> {
    let installation = Installation::current()?;
    let state = StateFile::new(context.store()?.dir().to_owned());
    let method = InstallMethod::detect(&installation);
    let outcome = match update::guard(method, &context.env, context.settings()) {
        // Asked again, in case anything changed since the command that started this check.
        Some(reason) => Ok(format!("not checked: {reason}")),
        None => GitHubReleases::new(&context.env, Channel::from_settings(context.settings()))
            .and_then(|source| {
                auto::check(
                    &source,
                    &TrustedKeys::for_run(&context.env),
                    &installation,
                    &update::current_version(&context.env),
                )
            }),
    };
    state.record(outcome.unwrap_or_else(|error| format!("failed: {}", error.message)));
    Ok(Exit::Success)
}

/// Automatic updates for one run of a command other than `jev update` and `jev version`.
struct Automatic {
    installation: Installation,
    state: StateFile,
    current: Version,
}

impl Automatic {
    /// Automatic updates for this run, or `None` when a guard turns them off.
    fn new(command: &Command, context: &Context<'_>) -> Option<Self> {
        if !command.updates_itself() {
            return None;
        }
        let installation = Installation::current().ok()?;
        let method = InstallMethod::detect(&installation);
        if let Some(reason) = update::guard(method, &context.env, context.settings()) {
            tracing::debug!(reason, "automatic updates are off");
            return None;
        }
        Some(Self {
            installation,
            state: StateFile::new(context.store().ok()?.dir().to_owned()),
            current: update::current_version(&context.env),
        })
    }

    /// Says, once, that `jev` updates itself and how to stop it.
    fn notice_once(&self, context: &Context<'_>) {
        // Nobody would see it, so it is kept for a run that shows it.
        if context.notifier.quiet
            || self.state.read().notice_shown
            || self.installation.writable().is_err()
        {
            return;
        }
        let marked = self.state.change(true, |state| {
            (!state.notice_shown).then(|| state.notice_shown = true)
        });
        if marked.is_some() {
            context.notify(
                &Notice::info(
                    "auto_update_enabled",
                    "jev updates itself: at most once a day, after a command, it looks for a new \
release in the background, and installs it when the next command starts",
                )
                .hint(
                    "to turn this off, run `jev config set update.auto false` or set \
JEV_AUTO_UPDATE=false; `jev version` shows whether it is on",
                ),
            );
        }
    }

    /// Swaps in the update a background check staged, if it is newer than this `jev`.
    fn apply_staged(&self, context: &Context<'_>) {
        let Some(staged) = self.installation.staged() else {
            return;
        };
        if !update::is_upgrade(&self.current, &staged) {
            tracing::info!(%staged, "discarding a staged update that is not newer than this jev");
            self.installation.discard_staged();
            return;
        }
        let applied = self.installation.apply(&staged, &|exe| {
            update::probe(exe, Some(&staged)).map(|_| ())
        });
        match applied {
            Ok(()) => {
                let ui = context.notifier.ui;
                let (arrow, dash) = if ui.has_unicode() {
                    ("→", "—")
                } else {
                    ("->", "-")
                };
                context.notify(&Notice::info(
                    "updated",
                    format!(
                        "jev updated {} {arrow} {staged} {dash} changelog: {}",
                        self.current,
                        update::release_page(&staged)
                    ),
                ));
            }
            Err(error) => {
                tracing::info!(
                    code = error.code,
                    "the staged update was not applied: {}",
                    error.message
                );
                self.installation.discard_staged();
            }
        }
    }

    /// Starts the background check when one is due.
    fn start_check(&self) {
        let now = auto::now();
        let state = self.state.read();
        if let Some(outcome) = &state.last_outcome {
            tracing::info!(outcome, "the last automatic update check");
        }
        if !state.due(now) {
            return;
        }
        if let Err(reason) = self.installation.writable() {
            tracing::info!(reason, "no automatic update check");
            return;
        }
        if !self.state.claim(now) {
            return;
        }
        match auto::spawn_check(self.installation.exe()) {
            Ok(()) => tracing::info!("looking for a newer release in the background"),
            Err(error) => tracing::info!(%error, "the automatic update check could not start"),
        }
    }
}

/// Before a command: swaps in an update staged by an earlier background check, and says once
/// that `jev` updates itself. Nothing here fails the command.
pub(crate) fn before(command: &Command, context: &Context<'_>) {
    if let Some(automatic) = Automatic::new(command, context) {
        automatic.notice_once(context);
        automatic.apply_staged(context);
    }
}

/// After a command: starts the background check, at most once a day. It never writes to stdout,
/// never changes the exit code, and is not waited for.
pub(crate) fn after(command: &Command, context: &Context<'_>) {
    if let Some(automatic) = Automatic::new(command, context) {
        automatic.start_check();
    }
}

fn emit(context: &mut Context<'_>, report: &Report) -> Result<Exit, CliError> {
    context.output.emit(report, context.stdout)?;
    Ok(Exit::Success)
}

/// What `jev update` did or found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    /// Nothing newer: this is the latest release, or newer, or nothing is released yet.
    UpToDate,
    /// `--check` found a newer release.
    UpdateAvailable,
    /// A new binary was installed and passed its self-test.
    Updated,
    /// `--rollback` put the previous binary back.
    RolledBack,
    /// A package manager owns this install; `command` updates it. Nothing was changed.
    Managed,
}

/// The result of `jev update`. Every field is always present.
#[derive(Debug, Serialize)]
struct Report {
    status: Status,
    /// The version that was running when the command started.
    current_version: String,
    /// The release concerned: the latest, the one installed, or the one restored.
    version: Option<String>,
    install_method: InstallMethod,
    /// The binary that is, or would be, replaced.
    path: String,
    /// The release page, with its changelog.
    release_url: Option<String>,
    /// What to run next: `jev update`, or the package manager's command.
    command: Option<String>,
}

impl Report {
    fn about(report: Self, release: &Release, command: Option<String>) -> Self {
        Self {
            version: Some(release.version.to_string()),
            release_url: Some(release.url.clone()),
            command,
            ..report
        }
    }
}

impl Render for Report {
    fn human(&self, ui: Ui) -> String {
        let arrow = if ui.has_unicode() { "→" } else { "->" };
        let current = &self.current_version;
        let version = self.version.as_deref().unwrap_or("?");
        let mut lines = vec![match self.status {
            Status::UpToDate if self.version.is_none() => {
                format!("jev {current}: no release is published yet")
            }
            Status::UpToDate => format!("jev {current} is up to date (latest release: {version})"),
            Status::UpdateAvailable => {
                format!("jev {} is available (this is {current})", ui.bold(version))
            }
            Status::Updated => format!("jev updated {current} {arrow} {}", ui.bold(version)),
            Status::RolledBack => format!("jev rolled back {current} {arrow} {}", ui.bold(version)),
            Status::Managed => format!(
                "jev was installed with {}, which updates it",
                self.install_method
                    .package_manager()
                    .unwrap_or("a package manager")
            ),
        }];
        if let Some(command) = &self.command {
            lines.push(format!("  {} {command}", ui.dim("run:")));
        }
        if let Some(url) = &self.release_url
            && matches!(self.status, Status::UpdateAvailable | Status::Updated)
        {
            lines.push(format!("  {} {url}", ui.dim("changelog:")));
        }
        if self.status == Status::Updated {
            lines.push(format!("  {} jev update --rollback", ui.dim("undo:")));
        }
        lines.join("\n") + "\n"
    }
}
