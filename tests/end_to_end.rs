//! End-to-end tests: the built program, run through its command line.
//!
//! Each test starts the program the way a user does and checks everything a
//! caller can observe: the exit status, stdout and stderr.
//!
//! The sample inputs are in `tests/fixtures`. Beside each `<name>.csv` lies
//! `<name>.expected.csv`, the accounts that the program must print for it.
//! The expected accounts were derived from the rules, not captured from the
//! program's output.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The header line of the output.
const OUTPUT_HEADER: &str = "client,available,held,total,locked";

/// What stderr holds when the program is not given exactly one argument.
const WRONG_ARGUMENTS: &str = "error: expected exactly one argument, the path of the input file\n";

/// A command that starts the built program. Cargo builds the binary before it
/// builds these tests, and passes its path in this variable.
fn program() -> Command {
    Command::new(env!("CARGO_BIN_EXE_payments-engine"))
}

/// Runs the program with `arguments` and returns its exit status and what it
/// wrote to stdout and stderr.
fn run(arguments: &[&Path]) -> Output {
    program()
        .args(arguments)
        .output()
        .expect("the program can be started")
}

/// The directory that holds the sample files.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// The path of the sample file `name`.
fn fixture(name: &str) -> PathBuf {
    fixtures().join(name)
}

/// What the program wrote to stdout or to stderr, as text.
fn text(stream: &[u8]) -> &str {
    std::str::from_utf8(stream).expect("the program writes text")
}

/// The account rows of an accounts table, sorted.
///
/// The order of the rows in the output is free, so two tables are compared by
/// their sorted rows. Nothing but the order may differ: the rows are compared
/// as text, which also holds every amount to exactly four decimals.
fn sorted_account_rows(table: &str) -> Vec<&str> {
    let mut lines = table.lines();
    assert_eq!(
        lines.next(),
        Some(OUTPUT_HEADER),
        "the first line of an accounts table"
    );
    let mut rows: Vec<&str> = lines.collect();
    rows.sort_unstable();
    rows
}

/// Checks a run that completed: exit status 0, the accounts of
/// `expected_accounts` on stdout in any order, and exactly `expected_stderr`
/// on stderr.
fn assert_completed(output: &Output, expected_accounts: &str, expected_stderr: &str) {
    let stderr = text(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "exit status, with stderr {stderr:?}"
    );
    assert_eq!(
        sorted_account_rows(text(&output.stdout)),
        sorted_account_rows(expected_accounts)
    );
    assert_eq!(stderr, expected_stderr);
}

/// Runs the program on the sample input `<name>.csv`. The run must complete
/// with the accounts of `<name>.expected.csv` and with exactly
/// `expected_stderr` on stderr.
fn assert_fixture_completes(name: &str, expected_stderr: &str) {
    let output = run(&[&fixture(&format!("{name}.csv"))]);
    let expected_accounts = fs::read_to_string(fixture(&format!("{name}.expected.csv")))
        .expect("the expected accounts can be read");
    assert_completed(&output, &expected_accounts, expected_stderr);
}

/// Checks what every failed run has in common: exit status 1, nothing on
/// stdout, and a single line on stderr that begins with `error: `. Returns
/// that line, so that the test can check what it says.
fn error_line(output: &Output) -> &str {
    let stderr = text(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "exit status, with stderr {stderr:?}"
    );
    assert_eq!(text(&output.stdout), "", "stdout of a failed run");
    assert!(stderr.starts_with("error: "), "stderr {stderr:?}");
    assert_eq!(stderr.lines().count(), 1, "stderr {stderr:?}");
    stderr
}

// Sample inputs with rows.

#[test]
fn deposits_and_withdrawals_are_applied_for_each_client() {
    // Two withdrawals ask for more than is available. One of them is the only
    // row of its client, who gets an account all the same.
    //
    // The expected file lists the clients in the order of their first rows,
    // which is not the order in which the program prints them. This test
    // passes only if the comparison disregards the order.
    assert_fixture_completes("basics", "refused transactions: 2\n");
}

#[test]
fn spaces_around_fields_are_accepted() {
    assert_fixture_completes("spaces_around_fields", "");
}

#[test]
fn dispute_rows_with_three_or_four_fields_are_processed() {
    // The fourth field of such a row may be missing, empty or filled with
    // anything. None of that is counted as a malformed row.
    assert_fixture_completes("dispute_rows_three_and_four_fields", "");
}

#[test]
fn resolved_deposit_can_be_disputed_again() {
    // The two ignored rows are a dispute on the deposit while it is already
    // under dispute, and a resolve once it no longer is.
    assert_fixture_completes("dispute_resolve_dispute_again", "ignored rows: 2\n");
}

#[test]
fn chargeback_locks_the_account() {
    // The locked account refuses a deposit and a withdrawal, and a resolve on
    // its other disputed deposit still goes through. The three ignored rows
    // are a dispute on the refused deposit, which was never stored, and a
    // dispute and a chargeback on the deposit that was charged back.
    assert_fixture_completes(
        "chargeback_locks_account",
        "refused transactions: 2\nignored rows: 3\n",
    );
}

#[test]
fn dispute_can_take_the_available_balance_below_zero() {
    assert_fixture_completes("negative_available_balance", "");
}

#[test]
fn malformed_rows_are_skipped_and_counted() {
    // Seventeen rows break one rule each, and the rows around them are still
    // applied. The empty line is not a row and is not counted.
    assert_fixture_completes("malformed_rows", "malformed rows: 17\n");
}

#[test]
fn each_count_above_zero_is_reported_on_its_own_line() {
    // Malformed: a type that does not exist. Refused: a withdrawal while all
    // of the client's funds are held, and a deposit under the id of a stored
    // deposit. Ignored: a dispute that names another client's deposit.
    assert_fixture_completes(
        "refused_ignored_and_malformed",
        "malformed rows: 1\nrefused transactions: 2\nignored rows: 1\n",
    );
}

// Inputs without rows, and an input without the header line.

#[test]
fn header_line_without_rows_prints_only_the_output_header() {
    assert_fixture_completes("header_only", "");
}

#[test]
fn input_of_zero_bytes_prints_only_the_output_header_and_a_note() {
    assert_fixture_completes(
        "empty",
        "note: the input is empty (no header line, no rows)\n",
    );
}

#[test]
fn data_row_in_place_of_the_header_line_fails() {
    let output = run(&[&fixture("missing_header.csv")]);
    assert_eq!(
        error_line(&output),
        "error: the input does not begin with the header line type,client,tx,amount\n"
    );
}

// Arguments and files that are not what the program expects.

#[test]
fn missing_file_fails() {
    let path = fixture("no_such_file.csv");
    let output = run(&[&path]);
    // After the path comes the system's own description of the failure,
    // which is not the same text on every system.
    let expected_start = format!("error: cannot open {}: ", path.display());
    let line = error_line(&output);
    assert!(line.starts_with(&expected_start), "stderr {line:?}");
}

#[test]
fn directory_in_place_of_the_file_fails() {
    let output = run(&[&fixtures()]);
    let line = error_line(&output);
    // Which step fails depends on the system. On Unix a directory can be
    // opened, and it is the first read that fails.
    if cfg!(unix) {
        assert!(
            line.starts_with("error: reading the input failed: "),
            "stderr {line:?}"
        );
    }
}

#[test]
fn no_argument_fails() {
    assert_eq!(error_line(&run(&[])), WRONG_ARGUMENTS);
}

#[test]
fn two_arguments_fail() {
    // Each names a file that could be read, so it is their number that is
    // refused.
    let first = fixture("basics.csv");
    let second = fixture("header_only.csv");
    assert_eq!(error_line(&run(&[&first, &second])), WRONG_ARGUMENTS);
}

#[test]
fn byte_order_mark_and_crlf_line_ends_are_accepted() {
    // Written here and not kept as a sample file: the three bytes of the mark
    // show in the source, and no checkout can convert the line ends.
    let input = Path::new(env!("CARGO_TARGET_TMPDIR")).join("byte_order_mark_and_crlf.csv");
    fs::write(
        &input,
        b"\xEF\xBB\xBFtype,client,tx,amount\r\n\
          deposit,81,1,64.5\r\n\
          deposit,82,2,7\r\n\
          withdrawal,81,3,4.25\r\n\
          dispute,82,2\r\n",
    )
    .expect("the input can be written");

    let expected_accounts = "client,available,held,total,locked\n\
                             81,60.2500,0.0000,60.2500,false\n\
                             82,0.0000,7.0000,7.0000,false\n";
    assert_completed(&run(&[&input]), expected_accounts, "");
}

#[test]
fn failed_write_of_the_accounts_fails() {
    // Stdout is a pipe that nobody reads from: the stdin of a process that
    // has already ended. That process is the program itself, which ends at
    // once when it is given no argument.
    let mut ended = program()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the program can be started");
    let pipe = ended.stdin.take().expect("stdin is a pipe");
    ended.wait().expect("the program ends");

    let output = program()
        .arg(fixture("basics.csv"))
        .stdout(pipe)
        .output()
        .expect("the program can be started");

    // Part of the accounts may have gone into the pipe before the write
    // failed, so stdout says nothing here. The exit status and stderr must
    // still report the failure.
    let stderr = text(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "exit status, with stderr {stderr:?}"
    );
    assert!(
        stderr.starts_with("error: writing the accounts failed: "),
        "stderr {stderr:?}"
    );
}
