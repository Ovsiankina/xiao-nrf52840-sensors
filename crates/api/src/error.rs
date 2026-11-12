//! Everything that can go wrong, in the developer's words: busy (and
//! who holds the sensor), link too slow, timeout after N attempts.
//!
//! Depends on: `xiao-proto::message`.

use crate::link::BytesPerSec;
use xiao_proto::message::Link;

/// Why a call failed.
#[derive(Debug)]
pub enum Error {
    /// Another link holds the sensor (FW-1).
    Busy {
        /// The link that holds it.
        held_by: Link,
    },
    /// The sensor's stream does not fit this link (FW-4).
    TooSlow {
        /// What the stream needs.
        needs: BytesPerSec,
        /// What the link carries.
        link_has: BytesPerSec,
    },
    /// The board did not answer, after this many tries (API-2).
    Timeout {
        /// How many times the connection was tried.
        attempts: u8,
    },
    /// No XIAO was found on this link.
    NotFound,
    /// The link went away while in use.
    Disconnected,
    /// The operating system refused something on the link.
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn error_messages_name_the_cause() {
        todo!()
    }
}
