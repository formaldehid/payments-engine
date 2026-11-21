//! Writes a large input file to stdout, for measuring how long the program
//! takes on it and how much memory it needs.
//!
//! ```text
//! cargo run --example large_input -- 10000000 > target/large_input.csv
//! cargo run -- target/large_input.csv > target/accounts.csv
//! ```
//!
//! The argument is the number of rows to write after the header line. Rows
//! come in blocks of ten, so it is rounded down to a multiple of ten. Nothing
//! is random: the same argument always gives the same file.
//!
//! # The rows
//!
//! Every block of ten rows belongs to one client. The blocks go through all
//! 65,536 client ids in turn and then start again, so the program ends up
//! printing as many accounts as there can be.
//!
//! A block holds five deposits, three withdrawals, one dispute and the row
//! that ends the dispute. A transaction id is the number of its block
//! followed by one digit, so block 7 uses the ids 71 to 78 and no id is used
//! twice.
//!
//! The row that ends the dispute is a resolve. For the 656 clients whose id
//! is a multiple of 100 it is a chargeback instead: the first block locks
//! such a client, and each later block of theirs comes to eight refused
//! transactions and two ignored rows.
//!
//! # The result
//!
//! The accounts follow from the pattern, so a measured run can be checked:
//!
//! - A client whose id is not a multiple of 100 gains 277.9220 with every
//!   block, and has nothing held.
//! - A client whose id is a multiple of 100 ends with 226.9375 available,
//!   nothing held, and a locked account.
//!
//! For 10,000,000 rows that is 16,790 clients with 4446.7520, 48,090 clients
//! with 4168.8300 and 656 locked clients, with `refused transactions: 76800`
//! and `ignored rows: 18708` on stderr.

// The same rules as in the library. An example is a crate of its own, so it
// needs its own copy. It has no tests, so the rules cover the whole file.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::todo,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::print_stdout,
    clippy::print_stderr
)]

use std::env;
use std::fmt;
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

/// Rows in one block.
const ROWS_PER_BLOCK: u64 = 10;

/// The most rows that can be written. An id is the block number followed by a
/// digit from 1 to 8, and the largest transaction id is 4294967295, so block
/// 429496728 is the last whose ids are all transaction ids.
const MOST_ROWS: u64 = 4_294_967_280;

fn main() -> ExitCode {
    let Some(blocks) = blocks_asked_for() else {
        return fail(format_args!(
            "usage: cargo run --example large_input -- <rows>\n\
             <rows> is the number of rows to write, at most {MOST_ROWS}"
        ));
    };
    match write_input(io::stdout().lock(), blocks) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(format_args!("error: writing the rows failed: {error}")),
    }
}

/// The number of whole blocks in the row count that the only argument gives.
///
/// `None` if there is not exactly one argument, if it is not a number, or if
/// it asks for more rows than there are transaction ids for.
fn blocks_asked_for() -> Option<u64> {
    let mut arguments = env::args_os().skip(1);
    let (Some(rows), None) = (arguments.next(), arguments.next()) else {
        return None;
    };
    let rows: u64 = rows.to_str()?.parse().ok()?;
    (rows <= MOST_ROWS).then_some(rows / ROWS_PER_BLOCK)
}

/// Writes the header line and `blocks` blocks of ten rows to `output`.
fn write_input<W: Write>(output: W, blocks: u64) -> io::Result<()> {
    // Stdout alone would pass every line on as soon as it is written.
    let mut output = BufWriter::new(output);
    writeln!(output, "type,client,tx,amount")?;

    // Blocks are numbered from 1, and the clients go round all of their ids.
    let clients = (0..=u16::MAX).cycle();
    for (block, client) in (1..=blocks).zip(clients) {
        let dispute_ends_with = if client % 100 == 0 {
            "chargeback"
        } else {
            "resolve"
        };
        // Each id is the block number followed by one digit. The dispute and
        // the row that ends it name the second deposit.
        writeln!(
            output,
            "deposit,{client},{block}1,250\n\
             deposit,{client},{block}2,37.5\n\
             withdrawal,{client},{block}3,20\n\
             dispute,{client},{block}2\n\
             deposit,{client},{block}4,0.0625\n\
             withdrawal,{client},{block}5,3.125\n\
             {dispute_ends_with},{client},{block}2\n\
             deposit,{client},{block}6,12.75\n\
             deposit,{client},{block}7,1.2345\n\
             withdrawal,{client},{block}8,0.5"
        )?;
    }
    output.flush()
}

/// Writes `message` to stderr and returns the exit status of a failed run.
fn fail(message: impl fmt::Display) -> ExitCode {
    // `writeln!` and not `eprintln!`, which panics when the write fails.
    let _ = writeln!(io::stderr().lock(), "{message}");
    ExitCode::FAILURE
}
