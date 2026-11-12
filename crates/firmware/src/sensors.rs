//! Who holds which sensor. Decides every request: grant, refuse as
//! busy (naming the holding link), or refuse as too fast for the link.
//! Pure logic, no hardware.
//!
//! Depends on: `xiao-proto::message`.
