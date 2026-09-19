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
    #[allow(dead_code)] // Reserved by the contract; first used by `jev batch run`.
    BatchPartial = 7,
    /// Evaluated successfully, and the gate condition is false. Never used for an error.
    #[allow(dead_code)] // Reserved by the contract; first used by the gating flags.
    GateFalse = 10,
    /// Evaluated successfully, and the answer is inside the abstain band.
    #[allow(dead_code)] // Reserved by the contract; first used by the gating flags.
    Abstain = 11,
    /// `jev update --check` found a newer version.
    #[allow(dead_code)] // Reserved by the contract; first used by `jev update`.
    UpdateAvailable = 20,
    /// Interrupted by SIGINT.
    #[allow(dead_code)]
    // Reserved by the contract; first used by the commands that run for long.
    Interrupted = 130,
}

impl Exit {
    /// The numeric code.
    pub(crate) const fn code(self) -> u8 {
        self as u8
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
    }
}
