//! Command-line entry point. The logic lives in the library.

// The same rules as in `lib.rs`; the binary is a separate crate and needs its own copy.
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )
)]

fn main() {}
