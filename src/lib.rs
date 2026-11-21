//! Library half of the crate. All logic lives here, so that it can be tested
//! and reused without the command line in `main.rs`.
//!
//! [`run::run`] is the whole program without its command line. It reads
//! transactions from any reader with [`input`], applies them with an
//! [`engine::Engine`], and writes the accounts to any writer with [`output`].
//! [`amount`] holds the money type that all of them share.

// No input may cause a panic or a silent overflow. The constructs that could are
// therefore denied outside tests: `unsafe` by the compiler on every build, the
// rest by `cargo clippy`. `println!` and `eprintln!` are on the list because
// they panic when the write fails, for instance into a closed pipe. `main.rs`
// repeats the block because the binary is a separate crate.
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
        clippy::arithmetic_side_effects,
        clippy::print_stdout,
        clippy::print_stderr
    )
)]

pub mod amount;
pub mod engine;
pub mod input;
pub mod output;
pub mod run;

#[cfg(test)]
mod test_support;
