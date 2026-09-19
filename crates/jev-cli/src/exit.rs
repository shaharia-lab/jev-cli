//! The exit-code contract. These numbers are public API: scripts branch on them.

use std::process::ExitCode;

/// Every exit code `jev` can return. See the README for what a caller should do with each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum Exit {
    /// Success, and any gate condition satisfied.
    Success = 0,
    /// An unexpected internal error: a bug in `jev`, or a response it cannot understand.
    Internal = 1,
    /// The command line or a request file is wrong. Nothing was sent.
    Usage = 2,
    /// No API key, or the API refused it (401, 403).
    Auth = 3,
    /// The API rejected the request (400, 404, 422).
    ApiRejected = 4,
    /// Rate limited or overloaded, after retries (429, 529).
    RateLimited = 5,
    /// Network failure, timeout, or a server error after retries (408, 5xx).
    Network = 6,
    /// A batch finished, but some rows failed.
    BatchPartial = 7,
    /// Evaluated successfully, and the gate condition is false. Never used for an error.
    GateFalse = 10,
    /// Evaluated successfully, and the answer is inside the abstain band.
    Abstain = 11,
    /// `jev update --check` found a newer version.
    #[allow(dead_code)] // Reserved by the contract; first used by `jev update`.
    UpdateAvailable = 20,
    /// Interrupted by SIGINT or SIGTERM.
    Interrupted = 130,
}

impl Exit {
    /// Every code, in numeric order.
    pub(crate) const ALL: [Self; 12] = [
        Self::Success,
        Self::Internal,
        Self::Usage,
        Self::Auth,
        Self::ApiRejected,
        Self::RateLimited,
        Self::Network,
        Self::BatchPartial,
        Self::GateFalse,
        Self::Abstain,
        Self::UpdateAvailable,
        Self::Interrupted,
    ];

    /// The numeric code.
    pub(crate) const fn code(self) -> u8 {
        self as u8
    }

    /// A stable name for the code, for `jev spec`.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Internal => "internal",
            Self::Usage => "usage",
            Self::Auth => "auth",
            Self::ApiRejected => "api_rejected",
            Self::RateLimited => "rate_limited",
            Self::Network => "network",
            Self::BatchPartial => "batch_partial",
            Self::GateFalse => "gate_false",
            Self::Abstain => "abstain",
            Self::UpdateAvailable => "update_available",
            Self::Interrupted => "interrupted",
        }
    }

    /// What the code means, whichever command returns it. A command's help may say it more
    /// precisely for that command.
    pub(crate) const fn meaning(self) -> &'static str {
        match self {
            Self::Success => "success, and any gate condition holds",
            Self::Internal => "internal error: a bug in jev, please report it",
            Self::Usage => "usage or validation error; nothing was sent",
            Self::Auth => "no API key, or the API refused it",
            Self::ApiRejected => "the API rejected the request",
            Self::RateLimited => "rate limited or overloaded, after retries",
            Self::Network => "network failure, timeout or server error, after retries",
            Self::BatchPartial => "a batch finished, but some rows failed",
            Self::GateFalse => "evaluated, and the gate condition is false (never an error)",
            Self::Abstain => "evaluated, and the answer is inside the abstain band",
            Self::UpdateAvailable => "a newer version is available",
            Self::Interrupted => "interrupted",
        }
    }
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        Self::from(exit.code())
    }
}

#[cfg(test)]
mod tests {
    use super::Exit;

    #[test]
    fn the_numbers_are_the_documented_contract() {
        let contract = [
            (Exit::Success, 0),
            (Exit::Internal, 1),
            (Exit::Usage, 2),
            (Exit::Auth, 3),
            (Exit::ApiRejected, 4),
            (Exit::RateLimited, 5),
            (Exit::Network, 6),
            (Exit::BatchPartial, 7),
            (Exit::GateFalse, 10),
            (Exit::Abstain, 11),
            (Exit::UpdateAvailable, 20),
            (Exit::Interrupted, 130),
        ];

        for (exit, code) in contract {
            assert_eq!(exit.code(), code, "{exit:?}");
        }
        assert_eq!(Exit::ALL.to_vec(), contract.map(|(exit, _)| exit).to_vec());
    }
}
