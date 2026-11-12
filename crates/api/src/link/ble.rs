//! The BLE link: finds the XIAO by name, reconnects up to `attempts`
//! times when it drops, then times out.
//!
//! Depends on: `link`.

use crate::Error;

/// The link over Bluetooth Low Energy.
#[derive(Debug)]
pub struct Ble {
    _private: (),
}

impl Ble {
    /// Connects to the board advertising the default name,
    /// `XIAO-Sense`, with 3 attempts (API-1, API-2).
    ///
    /// # Errors
    ///
    /// [`Error::Timeout`] when every attempt failed.
    pub fn open() -> Result<Self, Error> {
        // DECISION: default 3 attempts, 1 s apart; the SRS leaves both open.
        todo!()
    }

    /// Starts a BLE link to a board advertising another name.
    pub fn named(name: &str) -> BleBuilder {
        let _ = name;
        todo!()
    }

    /// Starts a BLE link to the default name with another number of
    /// connection attempts (API-2).
    pub fn attempts(attempts: u8) -> BleBuilder {
        let _ = attempts;
        todo!()
    }
}

/// Settings for a [`Ble`] link before it is opened.
#[derive(Debug)]
pub struct BleBuilder {
    _private: (),
}

impl BleBuilder {
    /// Sets the name to look for.
    #[must_use]
    pub fn named(self, name: &str) -> Self {
        let _ = name;
        todo!()
    }

    /// Sets how many times to try connecting, at first and after every
    /// drop, before giving up with [`Error::Timeout`] (API-2).
    #[must_use]
    pub fn attempts(self, attempts: u8) -> Self {
        let _ = attempts;
        todo!()
    }

    /// Connects.
    ///
    /// # Errors
    ///
    /// [`Error::Timeout`] when every attempt failed.
    pub fn open(self) -> Result<Ble, Error> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn api_2_retries_then_times_out() {
        todo!()
    }

    #[test]
    fn retry_stops_at_first_success() {
        todo!()
    }
}
