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
        match self {
            Self::Busy { held_by } => {
                write!(f, "sensor is busy: held by the {} link", link_name(*held_by))
            }
            Self::TooSlow { needs, link_has } => write!(
                f,
                "link too slow: stream needs {} B/s, link carries {} B/s",
                needs.0, link_has.0
            ),
            Self::Timeout { attempts } => {
                write!(f, "board did not answer after {attempts} attempt(s)")
            }
            Self::NotFound => f.write_str("no XIAO found on this link"),
            Self::Disconnected => f.write_str("link disconnected while in use"),
            Self::Io(error) => write!(f, "link I/O error: {error}"),
        }
    }
}

// DECISION: private name helper because `Link` lives in xiao-proto and has no
// Display; adding one there would change a finished crate.
fn link_name(link: Link) -> &'static str {
    match link {
        Link::Usb => "USB",
        Link::Ble => "BLE",
        Link::Uart => "UART",
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::Error;
    use crate::link::BytesPerSec;
    use xiao_proto::message::Link;

    #[test]
    fn error_messages_name_the_cause() {
        let busy = Error::Busy { held_by: Link::Ble }.to_string();
        assert!(busy.contains("BLE"), "{busy}");
        assert!(busy.contains("busy"), "{busy}");

        let slow = Error::TooSlow {
            needs: BytesPerSec(32_000),
            link_has: BytesPerSec(11_520),
        }
        .to_string();
        assert!(slow.contains("32000"), "{slow}");
        assert!(slow.contains("11520"), "{slow}");

        let timeout = Error::Timeout { attempts: 5 }.to_string();
        assert!(timeout.contains('5'), "{timeout}");

        let io = Error::from(std::io::Error::other("port gone"));
        assert!(io.to_string().contains("port gone"));
        assert!(std::error::Error::source(&io).is_some());
    }
}
