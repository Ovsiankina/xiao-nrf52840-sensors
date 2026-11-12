//! How a message becomes bytes on a link and back: postcard encoding
//! with COBS framing, so a reader can find message edges in a byte
//! stream after a drop.
//!
//! Depends on: `message`.
