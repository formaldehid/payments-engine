//! Library half of the crate. All logic lives here, so that it can be tested
//! and reused without the command line in `main.rs`.

// No input may cause a panic or a silent overflow. The constructs that could are
// therefore denied outside tests: `unsafe` by the compiler on every build, the
// rest by `cargo clippy`. `main.rs` repeats the block because the binary is a
// separate crate.
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

pub mod amount;
pub mod engine;
