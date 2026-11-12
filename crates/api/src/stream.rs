//! Readings as they arrive: iterate over them, ask for the latest one,
//! and learn how many were dropped (from the counter in each message).
//!
//! Depends on: `link`, `error`.

/// How many readings were lost on the way since the sensor was turned
/// on (API-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Dropped(pub u64);

#[cfg(test)]
mod tests {
    #[test]
    fn api_3_stream_and_latest() {
        todo!()
    }

    #[test]
    fn api_4_counter_gap_is_reported() {
        todo!()
    }

    #[test]
    fn counter_wrap_is_not_a_drop() {
        todo!()
    }

    #[test]
    fn dead_link_ends_the_stream_with_one_error() {
        todo!()
    }
}
