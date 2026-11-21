//! The whole program without its command line: transactions in, accounts out.
//!
//! [`run`] joins the three parts. [`crate::input`] turns the rows of the
//! input into transactions, an [`Engine`] applies them, and [`crate::output`]
//! writes the accounts. It takes any reader and any writer, so it is tested
//! here without a file or a terminal.

use std::io;

use thiserror::Error;

use crate::engine::{Engine, Outcome};
use crate::input::{self, Start};
use crate::output;

/// What a completed run has to report besides the accounts it wrote.
///
/// A row whose transaction was applied is not counted. Every other row after
/// the header line is counted exactly once, under one of the three counts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// Malformed rows: rows that could not be read as a transaction. Each
    /// was skipped and changed nothing.
    pub malformed_rows: u64,
    /// Refused transactions: deposits and withdrawals that the rules
    /// declined.
    pub refused_transactions: u64,
    /// Ignored rows: disputes, resolves and chargebacks that had no effect.
    pub ignored_rows: u64,
    /// Whether the input had no line at all: it was zero bytes long, or it
    /// held only empty lines. The accounts written are then the header line
    /// alone, as for an input of the header line and no rows.
    pub empty_input: bool,
}

/// Why a run did not complete.
#[derive(Debug, Error)]
pub enum RunError {
    /// The first line of the input is not the header line. Nothing was
    /// written.
    #[error("the input does not begin with the header line type,client,tx,amount")]
    WrongHeader,
    /// The input could not be read to its end. Nothing was written.
    #[error("reading the input failed: {0}")]
    Read(io::Error),
    /// The accounts could not be written. Part of them may have been.
    #[error("writing the accounts failed: {0}")]
    Write(io::Error),
}

/// Reads transactions from `input`, applies them in the order of the input,
/// and writes the resulting accounts to `output`.
///
/// Nothing is written before the last row has been read. When this returns
/// [`RunError::WrongHeader`] or [`RunError::Read`], `output` is therefore
/// untouched, and a caller that sends it to a file never finds part of a
/// result there.
///
/// A malformed row, a refused transaction and an ignored row do not end the
/// run. Each is counted in the [`Summary`], and the run goes on with the
/// next row.
///
/// The input is read one row at a time and never held as a whole. Memory
/// grows with the number of clients and of stored transactions, not with the
/// length of the input.
///
/// # Errors
///
/// - [`RunError::WrongHeader`] if the first line of `input` is not the header
///   line.
/// - [`RunError::Read`] if reading `input` fails at any point.
/// - [`RunError::Write`] if writing to `output` fails.
pub fn run<R: io::Read, W: io::Write>(input: R, output: W) -> Result<Summary, RunError> {
    let mut engine = Engine::new();
    let mut summary = Summary::default();

    match input::read_header(input).map_err(RunError::Read)? {
        // No transactions is a valid result. It is noted, because a file
        // without even a header line may not be the file that was meant.
        Start::NoHeaderLine => summary.empty_input = true,
        Start::WrongHeader => return Err(RunError::WrongHeader),
        Start::Header(rows) => {
            for row in rows {
                match row.map_err(RunError::Read)? {
                    Err(_malformed) => count(&mut summary.malformed_rows),
                    Ok(transaction) => match engine.process(transaction) {
                        Outcome::Applied => {}
                        Outcome::Refused(_) => count(&mut summary.refused_transactions),
                        Outcome::Ignored(_) => count(&mut summary.ignored_rows),
                    },
                }
            }
        }
    }

    // Only now, after the last row. A read failure has returned above with
    // nothing written.
    output::write_accounts(output, &engine).map_err(RunError::Write)?;
    Ok(summary)
}

/// Adds one to a count. The addition saturates, so it cannot overflow. No
/// input has enough rows to reach the limit.
fn count(counter: &mut u64) {
    *counter = counter.saturating_add(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{FailingReader, FailingWriter};
    use proptest::prelude::*;

    // Each test gives a whole input as text and checks what a run writes and
    // what it reports. Account rows are compared sorted, so the order in
    // which they are written does not matter.

    /// The header line of the output.
    const OUTPUT_HEADER: &str = "client,available,held,total,locked";

    /// What `completed` returns when no client has a record.
    const NO_ROWS: [&str; 0] = [];

    /// An input: the header line, then `rows`, one to a line.
    fn input_of(rows: &[&str]) -> Vec<u8> {
        let mut input = String::from("type,client,tx,amount\n");
        for row in rows {
            input.push_str(row);
            input.push('\n');
        }
        input.into_bytes()
    }

    /// Runs `input`, which must complete. Returns the account rows that were
    /// written after the header line, sorted, and the summary.
    fn completed(input: &[u8]) -> (Vec<String>, Summary) {
        let mut output = Vec::new();
        let summary = run(input, &mut output).expect("the run completes");
        let text = String::from_utf8(output).expect("the output is text");
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some(OUTPUT_HEADER));
        let mut rows: Vec<String> = lines.map(str::to_owned).collect();
        rows.sort();
        (rows, summary)
    }

    /// Runs `input`, which must fail. Returns the error and what was written.
    fn failed<R: io::Read>(input: R) -> (RunError, Vec<u8>) {
        let mut output = Vec::new();
        let error = run(input, &mut output).expect_err("the run fails");
        (error, output)
    }

    /// The summary of a run that skipped `count` malformed rows and had
    /// nothing else to report.
    fn malformed_rows(count: u64) -> Summary {
        Summary {
            malformed_rows: count,
            ..Summary::default()
        }
    }

    // Rows.

    #[test]
    fn dispute_row_with_three_fields_is_processed() {
        let (rows, summary) = completed(&input_of(&["deposit,1,1,10", "dispute,1,1"]));
        assert_eq!(rows, ["1,0.0000,10.0000,10.0000,false"]);
        assert_eq!(summary, Summary::default());
    }

    #[test]
    fn row_with_five_fields_is_skipped() {
        // Dropping the fifth field would have deposited 1.
        let (rows, summary) = completed(&input_of(&["deposit,1,1,10", "deposit,1,7,1,000.50"]));
        assert_eq!(rows, ["1,10.0000,0.0000,10.0000,false"]);
        assert_eq!(summary, malformed_rows(1));
    }

    #[test]
    fn deposit_of_zero_is_skipped() {
        let (rows, summary) = completed(&input_of(&["deposit,1,1,10", "deposit,1,2,0"]));
        assert_eq!(rows, ["1,10.0000,0.0000,10.0000,false"]);
        assert_eq!(summary, malformed_rows(1));
    }

    #[test]
    fn amount_with_five_decimals_is_skipped() {
        let (rows, summary) = completed(&input_of(&["deposit,1,1,1", "deposit,1,2,1.00005"]));
        assert_eq!(rows, ["1,1.0000,0.0000,1.0000,false"]);
        assert_eq!(summary, malformed_rows(1));
    }

    #[test]
    fn amount_on_a_dispute_row_is_not_read() {
        let (rows, summary) = completed(&input_of(&["deposit,1,1,10", "dispute,1,1,3.0"]));
        assert_eq!(rows, ["1,0.0000,10.0000,10.0000,false"]);
        assert_eq!(summary, Summary::default());
    }

    #[test]
    fn amount_with_an_exponent_is_skipped() {
        let (rows, summary) = completed(&input_of(&["deposit,1,1,1", "deposit,1,2,1e2"]));
        assert_eq!(rows, ["1,1.0000,0.0000,1.0000,false"]);
        assert_eq!(summary, malformed_rows(1));
    }

    #[test]
    fn malformed_row_is_skipped_and_later_rows_are_processed() {
        let (rows, summary) = completed(&input_of(&[
            "deposit,1,1,10",
            "transfer,1,2,5",
            "deposit,1,3,1",
        ]));
        assert_eq!(rows, ["1,11.0000,0.0000,11.0000,false"]);
        assert_eq!(summary, malformed_rows(1));
    }

    #[test]
    fn row_that_is_not_utf8_is_skipped() {
        // The byte stands in the fourth field of a dispute row, which is not
        // looked at. The row is skipped all the same: the dispute is not
        // applied, and the deposit stays available.
        let (rows, summary) =
            completed(b"type,client,tx,amount\ndeposit,1,1,10\ndispute,1,1,\xff\n");
        assert_eq!(rows, ["1,10.0000,0.0000,10.0000,false"]);
        assert_eq!(summary, malformed_rows(1));
    }

    #[test]
    fn repeated_deposit_id_is_refused_and_counted() {
        let (rows, summary) = completed(&input_of(&["deposit,1,1,10", "deposit,1,1,10"]));
        assert_eq!(rows, ["1,10.0000,0.0000,10.0000,false"]);
        assert_eq!(
            summary,
            Summary {
                refused_transactions: 1,
                ..Summary::default()
            }
        );
    }

    #[test]
    fn type_in_capitals_is_skipped() {
        let (rows, summary) = completed(&input_of(&["deposit,1,1,1", "Deposit,1,2,10"]));
        assert_eq!(rows, ["1,1.0000,0.0000,1.0000,false"]);
        assert_eq!(summary, malformed_rows(1));
    }

    #[test]
    fn client_id_with_a_plus_sign_is_skipped() {
        let (rows, summary) = completed(&input_of(&["deposit,+1,1,5"]));
        assert_eq!(rows, NO_ROWS);
        assert_eq!(summary, malformed_rows(1));
    }

    #[test]
    fn client_id_with_leading_zeros_is_read() {
        let (rows, summary) = completed(&input_of(&["deposit,007,1,5"]));
        assert_eq!(rows, ["7,5.0000,0.0000,5.0000,false"]);
        assert_eq!(summary, Summary::default());
    }

    #[test]
    fn spaces_after_commas_and_a_dispute_row_with_three_fields() {
        let input = "type, client, tx, amount\n\
                     deposit, 4, 10, 25.5\n\
                     deposit, 9, 11, 3\n\
                     dispute, 4, 10\n\
                     withdrawal, 9, 12, 1.25\n";
        let (rows, summary) = completed(input.as_bytes());
        assert_eq!(
            rows,
            [
                "4,0.0000,25.5000,25.5000,false",
                "9,1.7500,0.0000,1.7500,false"
            ]
        );
        assert_eq!(summary, Summary::default());
    }

    // Quoted fields.

    #[test]
    fn quoted_header_and_quoted_fields_are_read() {
        let input = "\"type\",\"client\",\"tx\",\"amount\"\n\"deposit\",\"1\",\"1\",\"1.5\"\n";
        let (rows, summary) = completed(input.as_bytes());
        assert_eq!(rows, ["1,1.5000,0.0000,1.5000,false"]);
        assert_eq!(summary, Summary::default());
    }

    #[test]
    fn unclosed_quote_on_a_dispute_row_takes_the_rows_after_it() {
        // The quote is never closed, so the two deposits after it are part of
        // the fourth field of the dispute row. That field is not looked at:
        // the dispute is applied, and the deposits are lost without a count.
        let (rows, summary) = completed(&input_of(&[
            "deposit,1,1,10",
            "dispute,1,1,\"oops",
            "deposit,1,2,5",
            "deposit,1,3,5",
        ]));
        assert_eq!(rows, ["1,0.0000,10.0000,10.0000,false"]);
        assert_eq!(summary, Summary::default());
    }

    // The header line, and inputs without rows.

    #[test]
    fn data_row_in_place_of_the_header_prints_nothing() {
        let (error, output) = failed("deposit,1,1,10\ndeposit,1,2,5\n".as_bytes());
        assert!(matches!(error, RunError::WrongHeader));
        assert!(output.is_empty());
    }

    #[test]
    fn header_line_without_rows_prints_only_the_output_header() {
        for input in ["type,client,tx,amount\n", "type,client,tx,amount"] {
            let (rows, summary) = completed(input.as_bytes());
            assert_eq!(rows, NO_ROWS, "{input:?}");
            assert_eq!(summary, Summary::default(), "{input:?}");
        }
    }

    #[test]
    fn input_without_any_line_prints_only_the_output_header_and_is_noted() {
        // Zero bytes, and three empty lines.
        for input in ["", "\n\n\n"] {
            let (rows, summary) = completed(input.as_bytes());
            assert_eq!(rows, NO_ROWS, "{input:?}");
            assert_eq!(
                summary,
                Summary {
                    empty_input: true,
                    ..Summary::default()
                },
                "{input:?}"
            );
        }
    }

    // Failures to read and to write.

    #[test]
    fn read_failure_part_way_prints_nothing() {
        // Both deposits are read and applied before the input breaks off.
        let input = FailingReader::after(b"type,client,tx,amount\ndeposit,1,1,10\ndeposit,2,2,5\n");
        let (error, output) = failed(input);
        assert!(matches!(error, RunError::Read(_)));
        assert!(output.is_empty());
    }

    #[test]
    fn read_failure_in_the_middle_of_a_row_prints_nothing() {
        // The cut-off row is not counted as a malformed row and skipped. It
        // ends the run.
        let input = FailingReader::after(b"type,client,tx,amount\ndeposit,1,1,10\ndeposit,2,2");
        let (error, output) = failed(input);
        assert!(matches!(error, RunError::Read(_)));
        assert!(output.is_empty());
    }

    #[test]
    fn read_failure_on_the_first_read_is_not_an_empty_input() {
        // An input of zero bytes completes. One that cannot be read at all
        // does not, although neither yields a byte.
        let (error, output) = failed(FailingReader::after(b""));
        assert!(matches!(error, RunError::Read(_)));
        assert!(output.is_empty());
    }

    #[test]
    fn write_failure_is_reported() {
        let input = input_of(&["deposit,1,1,10"]);
        let result = run(input.as_slice(), FailingWriter);
        assert!(matches!(result, Err(RunError::Write(_))));
    }

    #[test]
    fn each_error_says_what_failed() {
        let broke_off = || io::Error::new(io::ErrorKind::Other, "it broke off");
        assert_eq!(
            RunError::WrongHeader.to_string(),
            "the input does not begin with the header line type,client,tx,amount"
        );
        assert_eq!(
            RunError::Read(broke_off()).to_string(),
            "reading the input failed: it broke off"
        );
        assert_eq!(
            RunError::Write(broke_off()).to_string(),
            "writing the accounts failed: it broke off"
        );
    }

    // Counting.

    #[test]
    fn each_row_that_is_not_applied_is_counted_once() {
        let (rows, summary) = completed(&input_of(&[
            "deposit,1,1,10",    // applied
            "deposit,1,1,10",    // refused: the id is that of a stored deposit
            "withdrawal,1,2,50", // refused: more than the available funds
            "dispute,1,9",       // ignored: no deposit 9 is stored
            "resolve,1,1",       // ignored: deposit 1 is not under dispute
            "chargeback,1,1",    // ignored: deposit 1 is not under dispute
            "transfer,1,3,5",    // malformed: not a type
            "dispute,1,1",       // applied
        ]));
        assert_eq!(rows, ["1,0.0000,10.0000,10.0000,false"]);
        assert_eq!(
            summary,
            Summary {
                malformed_rows: 1,
                refused_transactions: 2,
                ignored_rows: 3,
                empty_input: false,
            }
        );
    }

    // Properties. Random text is almost never a row that does anything, so
    // most generated rows are put together from a small vocabulary: the five
    // types, a few ids that come up again and again, a few amounts, and now
    // and then a field that breaks a rule.

    /// One field of a generated row, with the whitespace that the padded
    /// writing of the row puts around it.
    #[derive(Debug, Clone, Copy)]
    struct Field {
        text: &'static str,
        before: &'static str,
        after: &'static str,
    }

    /// A field whose text is one of `usual` nine times in ten, and one of
    /// `unusual` otherwise.
    fn field(
        usual: &'static [&'static str],
        unusual: &'static [&'static str],
    ) -> impl Strategy<Value = Field> {
        let text = prop_oneof![
            9 => prop::sample::select(usual),
            1 => prop::sample::select(unusual),
        ];
        let padding = || prop::sample::select(&["", " ", "  ", "\t", " \t "][..]);
        (text, padding(), padding()).prop_map(|(text, before, after)| Field {
            text,
            before,
            after,
        })
    }

    fn type_field() -> impl Strategy<Value = Field> {
        field(
            &["deposit", "withdrawal", "dispute", "resolve", "chargeback"],
            &["transfer", "Deposit", ""],
        )
    }

    fn client_field() -> impl Strategy<Value = Field> {
        field(&["1", "2"], &["002", "+1", "65536", ""])
    }

    fn tx_field() -> impl Strategy<Value = Field> {
        field(
            &["1", "2", "3", "4", "5", "6"],
            &["006", "-1", "1.0", "4294967296", "x"],
        )
    }

    fn amount_field() -> impl Strategy<Value = Field> {
        field(
            &["1", "2.5", "10", "0.0001", "400"],
            &["0", "1.00005", "1e2", "-1", "abc", ""],
        )
    }

    /// One row: usually four fields, often three, and now and then any
    /// fields at all, from one to six of them.
    fn row() -> impl Strategy<Value = Vec<Field>> {
        let any_field = || prop_oneof![type_field(), client_field(), tx_field(), amount_field()];
        prop_oneof![
            6 => (type_field(), client_field(), tx_field(), amount_field())
                .prop_map(|(kind, client, tx, amount)| vec![kind, client, tx, amount]),
            3 => (type_field(), client_field(), tx_field())
                .prop_map(|(kind, client, tx)| vec![kind, client, tx]),
            1 => prop::collection::vec(any_field(), 1..=6),
        ]
    }

    /// Up to forty rows. A row of one empty field is left out, because its
    /// two writings are different rows: bare it is an empty line, which is
    /// skipped, and padded it is a line of spaces, which is a malformed row.
    fn rows() -> impl Strategy<Value = Vec<Vec<Field>>> {
        let row = row().prop_filter(
            "a row of one empty field",
            |row| !matches!(row.as_slice(), [only] if only.text.is_empty()),
        );
        prop::collection::vec(row, 0..=40)
    }

    /// The rows as lines of text, each field written by `write`.
    fn lines(rows: &[Vec<Field>], write: impl Fn(&Field) -> String) -> String {
        let mut text = String::new();
        for row in rows {
            let fields: Vec<String> = row.iter().map(&write).collect();
            text.push_str(&fields.join(","));
            text.push('\n');
        }
        text
    }

    /// The field with nothing around it.
    fn bare(field: &Field) -> String {
        field.text.to_owned()
    }

    /// The field with its whitespace around it.
    fn padded(field: &Field) -> String {
        format!("{}{}{}", field.before, field.text, field.after)
    }

    /// Input bytes of three kinds: any bytes at all, text of the characters
    /// that mean something to the reader, and rows from the vocabulary. One
    /// input in two has the header line in front.
    fn any_input() -> impl Strategy<Value = Vec<u8>> {
        let body = prop_oneof![
            prop::collection::vec(any::<u8>(), 0..=200),
            "[a-z0-9,.\" \t\r\n+-]{0,200}".prop_map(String::into_bytes),
            rows().prop_map(|rows| lines(&rows, bare).into_bytes()),
        ];
        (any::<bool>(), body).prop_map(|(with_header, body)| {
            let header: &[u8] = if with_header {
                b"type,client,tx,amount\n"
            } else {
                b""
            };
            [header, body.as_slice()].concat()
        })
    }

    /// What a run on `input` returned and what it wrote. An error is given
    /// as its message, because the error itself cannot be compared.
    fn outcome_of(input: &[u8]) -> (Result<Summary, String>, Vec<u8>) {
        let mut output = Vec::new();
        let result = run(input, &mut output).map_err(|error| error.to_string());
        (result, output)
    }

    proptest! {
        #[test]
        fn prop_no_input_makes_a_run_panic_and_a_failed_run_writes_nothing(
            input in any_input(),
        ) {
            // Returning at all is the first half of the property.
            let (result, output) = outcome_of(&input);
            if result.is_ok() {
                // A completed run wrote the header line, then rows of five
                // fields.
                let text = String::from_utf8(output).expect("the output is text");
                let mut lines = text.lines();
                prop_assert_eq!(lines.next(), Some(OUTPUT_HEADER));
                for line in lines {
                    prop_assert_eq!(line.split(',').count(), 5, "{:?}", line);
                }
            } else {
                prop_assert!(output.is_empty());
            }

            // The same bytes from an input that breaks off after them. That
            // is a failure whatever the bytes are, and nothing is written.
            let mut output = Vec::new();
            prop_assert!(run(FailingReader::after(&input), &mut output).is_err());
            prop_assert!(output.is_empty());
        }

        #[test]
        fn prop_whitespace_around_fields_does_not_change_the_result(rows in rows()) {
            let bare_input = format!("type,client,tx,amount\n{}", lines(&rows, bare));
            let padded_input = format!(
                " type ,\tclient , tx\t,  amount \n{}",
                lines(&rows, padded)
            );
            prop_assert_eq!(
                outcome_of(padded_input.as_bytes()),
                outcome_of(bare_input.as_bytes())
            );
        }
    }
}
