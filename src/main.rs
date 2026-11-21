//! Command-line entry point.
//!
//! `payments-engine <input.csv>` reads the transactions in the named file and
//! prints the resulting client accounts to stdout as CSV. Nothing else goes
//! to stdout.
//!
//! The exit status is 0 when the input was read to its end and the accounts
//! were printed. That includes inputs with rows that were malformed, refused
//! or ignored: they are counted, and each count above zero is reported on a
//! line of stderr.
//!
//! In every other case the exit status is 1, and stderr holds one line that
//! begins with `error:`. Stdout then holds nothing, unless printing to it is
//! what failed, so part of a result is not mistaken for a result.
//!
//! The work is done by the library. This file connects it to the arguments,
//! the file and the standard streams.

// The same rules as in `lib.rs`. The binary is a separate crate and needs its
// own copy.
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

use std::env;
use std::fmt;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use payments_engine::run::{run, Summary};

fn main() -> ExitCode {
    // `args_os`, because `args` panics on an argument that is not Unicode,
    // and a file name need not be.
    let mut arguments = env::args_os().skip(1);
    let (Some(path), None) = (arguments.next(), arguments.next()) else {
        return fail("expected exactly one argument, the path of the input file");
    };

    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) => {
            return fail(format_args!(
                "cannot open {}: {error}",
                Path::new(&path).display()
            ));
        }
    };

    match run(file, io::stdout().lock()) {
        Ok(summary) => {
            // The accounts are printed and complete. If the report cannot be
            // written, there is nobody left to tell and nothing to take back.
            let _ = write_report(io::stderr().lock(), &summary);
            ExitCode::SUCCESS
        }
        Err(error) => fail(error),
    }
}

/// Writes `error: <message>` to stderr and returns the exit status of a
/// failed run.
fn fail(message: impl fmt::Display) -> ExitCode {
    // `writeln!` and not `eprintln!`, which panics when the write fails. A
    // failure to report a failure has nowhere to go, and the exit status
    // still tells the caller.
    let _ = writeln!(io::stderr().lock(), "error: {message}");
    ExitCode::FAILURE
}

/// Writes what a completed run has to report: a note if the input was empty,
/// then one line for each count that is above zero. A run with nothing to
/// report writes nothing.
fn write_report<W: Write>(mut stderr: W, summary: &Summary) -> io::Result<()> {
    if summary.empty_input {
        writeln!(stderr, "note: the input is empty (no header line, no rows)")?;
    }

    let counts = [
        ("malformed rows", summary.malformed_rows),
        ("refused transactions", summary.refused_transactions),
        ("ignored rows", summary.ignored_rows),
    ];
    for (label, count) in counts {
        if count > 0 {
            writeln!(stderr, "{label}: {count}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `write_report` writes for `summary`, as text.
    fn report(summary: &Summary) -> String {
        let mut stderr = Vec::new();
        write_report(&mut stderr, summary).expect("writing to memory cannot fail");
        String::from_utf8(stderr).expect("the report is text")
    }

    #[test]
    fn run_with_nothing_to_report_writes_nothing() {
        assert_eq!(report(&Summary::default()), "");
    }

    #[test]
    fn each_count_above_zero_gets_one_line() {
        let summary = Summary {
            malformed_rows: 3,
            refused_transactions: 1,
            ignored_rows: 12,
            empty_input: false,
        };
        assert_eq!(
            report(&summary),
            "malformed rows: 3\nrefused transactions: 1\nignored rows: 12\n"
        );
    }

    #[test]
    fn count_of_zero_gets_no_line() {
        let only_refused = Summary {
            refused_transactions: 2,
            ..Summary::default()
        };
        assert_eq!(report(&only_refused), "refused transactions: 2\n");

        let only_ignored = Summary {
            ignored_rows: 1,
            ..Summary::default()
        };
        assert_eq!(report(&only_ignored), "ignored rows: 1\n");
    }

    #[test]
    fn empty_input_gets_a_note() {
        let summary = Summary {
            empty_input: true,
            ..Summary::default()
        };
        assert_eq!(
            report(&summary),
            "note: the input is empty (no header line, no rows)\n"
        );
    }
}
