//! Reads transactions from CSV text.
//!
//! The input is a header line, `type,client,tx,amount`, followed by one row
//! for each transaction. [`read_header`] checks the header line and returns
//! the [`Rows`] after it. They are read one at a time, so the input is never
//! held in memory as a whole.
//!
//! This is the only module that uses the `csv` crate. The fields of a row are
//! read by hand, so that every rule below is stated in one place and reported
//! by a reason of its own.
//!
//! # What a row must look like
//!
//! The checks run in this order, and the first one that fails is the reason
//! the row is a *malformed row*:
//!
//! 1. The row is valid UTF-8, in every field.
//! 2. It has three fields or four. A fifth field is never dropped: in
//!    `deposit,1,7,1,000.50` that would turn 1000.50 into 1.
//! 3. The type is `deposit`, `withdrawal`, `dispute`, `resolve` or
//!    `chargeback`, spelled exactly so.
//! 4. The client id is plain digits with a value of at most 65535. Plain
//!    digits means one or more ASCII digits and nothing else, so `+1` is not
//!    an id.
//! 5. The transaction id is plain digits with a value of at most 4294967295.
//! 6. A deposit or a withdrawal has a fourth field, and it holds an amount
//!    that [`Amount`] accepts.
//!
//! A dispute, a resolve or a chargeback names a deposit by id and carries no
//! amount. Its fourth field may be missing, empty or filled, and is never
//! looked at.
//!
//! Whitespace around a field is not part of the field. An empty line is not a
//! row and is skipped. A line of nothing but spaces is a row with one empty
//! field, so it is malformed.
//!
//! # Quoted fields
//!
//! Fields may be quoted as in standard CSV: `"deposit","7","31","2.5"` reads
//! like `deposit,7,31,2.5`. A quote has this meaning only at the very start of
//! a field. One that is opened and never closed runs to the end of the input,
//! and the lines inside it become part of that one field. A row is therefore
//! not always one line, and a row with such a field is held in memory whole.
//! If the field is the unread fourth field of a dispute, resolve or chargeback
//! row, the lines inside it are lost without being counted.

use std::io;
use std::str::FromStr;

use csv::{ReaderBuilder, StringRecord};

use crate::amount::{Amount, ParseAmountError};
use crate::engine::{ClientId, Transaction, TxId};

/// The column names of the header line, in order.
const HEADER: [&str; 4] = ["type", "client", "tx", "amount"];

/// What stands at the start of an input, before any row.
#[derive(Debug)]
pub enum Start<R> {
    /// Nothing: the input has no line at all. It is zero bytes long, or it
    /// holds only empty lines.
    NoHeaderLine,
    /// A first line that is not the header line.
    WrongHeader,
    /// The header line. The value reads the rows after it.
    Header(Rows<R>),
}

/// Reads the first line of `input` and compares it with the header line.
///
/// The header line is the four names `type`, `client`, `tx` and `amount`, in
/// this order and in lower case, with any whitespace around each name. Empty
/// lines before it are skipped, and so is a byte-order mark. Anything else in
/// its place is reported as [`Start::WrongHeader`], including a line of
/// spaces and a line that is not valid UTF-8.
///
/// `input` needs no buffering of its own: it is read in blocks.
///
/// # Errors
///
/// Returns the error of `input` if reading fails before the first line is
/// complete. An input that cannot be read is therefore never taken for an
/// empty one.
pub fn read_header<R: io::Read>(input: R) -> io::Result<Start<R>> {
    let mut reader = ReaderBuilder::new()
        // The first record is returned like any other, so that it is compared
        // with the header line here. By default the reader would consume it
        // and compare it with nothing.
        .has_headers(false)
        // A dispute row may end after the transaction id. The reader then
        // checks no lengths at all, so `parse_row` counts the fields.
        .flexible(true)
        .from_reader(input);
    let mut record = StringRecord::new();

    Ok(match next_record(&mut reader, &mut record)? {
        Next::End => Start::NoHeaderLine,
        Next::Record if trimmed_fields(&record).eq(HEADER) => {
            Start::Header(Rows { reader, record })
        }
        // The check ends here. The line after a wrong first line is never
        // tried as the header.
        Next::Record | Next::NotUtf8 => Start::WrongHeader,
    })
}

/// The rows of an input after its header line, read one at a time.
///
/// Iterating yields one item for each row, in file order:
///
/// - `Ok(Ok(transaction))` for a well-formed row.
/// - `Ok(Err(reason))` for a malformed row. Reading continues with the next
///   row.
/// - `Err(failure)` if the input could not be read. The rows end after it.
///
/// A failure to read and a malformed row are different layers of the item,
/// so one cannot be taken for the other. One row is held in memory at a time,
/// however long the input is.
#[derive(Debug)]
pub struct Rows<R> {
    reader: csv::Reader<R>,
    /// Reused for every row, so that reading a row allocates nothing.
    record: StringRecord,
}

impl<R: io::Read> Iterator for Rows<R> {
    type Item = io::Result<Result<Transaction, MalformedRow>>;

    fn next(&mut self) -> Option<Self::Item> {
        match next_record(&mut self.reader, &mut self.record) {
            Ok(Next::Record) => Some(Ok(parse_row(&self.record))),
            Ok(Next::NotUtf8) => Some(Ok(Err(MalformedRow::NotUtf8))),
            Ok(Next::End) => None,
            Err(failure) => Some(Err(failure)),
        }
    }
}

/// Why a row cannot be read as a transaction.
///
/// The variants follow the order of the checks. A row that breaks several
/// rules is reported under the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MalformedRow {
    /// The row has bytes that are not valid UTF-8, in whichever field.
    NotUtf8,
    /// The row has fewer than three fields or more than four.
    WrongFieldCount,
    /// The type is not one of `deposit`, `withdrawal`, `dispute`, `resolve`
    /// and `chargeback`, spelled exactly so.
    UnknownType,
    /// The client id is not plain digits with a value of at most 65535.
    InvalidClient,
    /// The transaction id is not plain digits with a value of at most
    /// 4294967295.
    InvalidTx,
    /// A deposit or withdrawal row that ends after the transaction id.
    MissingAmount,
    /// A deposit or withdrawal row whose fourth field is not an amount that
    /// such a row may carry. An empty field is one such case.
    InvalidAmount(ParseAmountError),
}

/// What one attempt to read the next record found.
enum Next {
    /// A record. Its fields are in the buffer that was passed in.
    Record,
    /// A record that is not valid UTF-8. The reader has moved past it.
    NotUtf8,
    /// The end of the input.
    End,
}

/// Reads the next record of `reader` into `record`.
///
/// This is the one place that tells a failure to read apart from a record
/// that cannot be used. Only the first is returned as an error.
fn next_record<R: io::Read>(
    reader: &mut csv::Reader<R>,
    record: &mut StringRecord,
) -> io::Result<Next> {
    match reader.read_record(record) {
        Ok(true) => Ok(Next::Record),
        Ok(false) => Ok(Next::End),
        Err(error) => match error.into_kind() {
            // Had this been skipped like a bad record, the reader would
            // report a normal end of input next, and a cut-off input would
            // pass for a complete one.
            csv::ErrorKind::Io(failure) => Err(failure),
            // The reader reports every problem with a single record this way
            // too. With rows of any length allowed, the only one left is
            // text that is not valid UTF-8.
            _ => Ok(Next::NotUtf8),
        },
    }
}

/// The fields of `record`, each without the whitespace around it.
///
/// The reader's own `trim` option gives the same text, but it copies every
/// record twice to do so.
fn trimmed_fields(record: &StringRecord) -> impl Iterator<Item = &str> {
    record.iter().map(str::trim)
}

/// The kind of a row, as its `type` field names it.
#[derive(Clone, Copy)]
enum Kind {
    Deposit,
    Withdrawal,
    Dispute,
    Resolve,
    Chargeback,
}

impl Kind {
    /// Reads the `type` field. The five words are matched exactly, so
    /// `Deposit` is not a type.
    fn parse(text: &str) -> Option<Kind> {
        match text {
            "deposit" => Some(Kind::Deposit),
            "withdrawal" => Some(Kind::Withdrawal),
            "dispute" => Some(Kind::Dispute),
            "resolve" => Some(Kind::Resolve),
            "chargeback" => Some(Kind::Chargeback),
            _ => None,
        }
    }
}

/// Reads one record as a transaction, or says why it is a malformed row.
///
/// The checks run in the order that the module documentation lists.
fn parse_row(record: &StringRecord) -> Result<Transaction, MalformedRow> {
    // Three fields, an optional fourth, and no fifth. Nothing is indexed, so
    // a short row cannot cause a panic. `fuse` makes it sound to ask for a
    // field after the last one.
    let mut fields = trimmed_fields(record).fuse();
    let (Some(kind), Some(client), Some(tx), amount, None) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) else {
        return Err(MalformedRow::WrongFieldCount);
    };

    let kind = Kind::parse(kind).ok_or(MalformedRow::UnknownType)?;
    let client = ClientId(parse_id(client).ok_or(MalformedRow::InvalidClient)?);
    let tx = TxId(parse_id(tx).ok_or(MalformedRow::InvalidTx)?);

    Ok(match kind {
        Kind::Deposit => Transaction::Deposit {
            client,
            tx,
            amount: parse_amount(amount)?,
        },
        Kind::Withdrawal => Transaction::Withdrawal {
            client,
            tx,
            amount: parse_amount(amount)?,
        },
        // These three act on the amount stored with the deposit they name.
        // Whatever their own fourth field holds, it is not looked at.
        Kind::Dispute => Transaction::Dispute { client, tx },
        Kind::Resolve => Transaction::Resolve { client, tx },
        Kind::Chargeback => Transaction::Chargeback { client, tx },
    })
}

/// Reads a client or transaction id: one or more ASCII digits whose value
/// fits `T`. Leading zeros are accepted, as they are in an amount.
fn parse_id<T: FromStr>(text: &str) -> Option<T> {
    // `str::parse` alone would also accept a leading `+`.
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // Only digits are left, so this fails exactly when the value is too
    // large for `T`.
    text.parse().ok()
}

/// Reads the amount of a deposit or withdrawal row from its fourth field, if
/// the row has one.
fn parse_amount(field: Option<&str>) -> Result<Amount, MalformedRow> {
    field
        .ok_or(MalformedRow::MissingAmount)?
        .parse()
        .map_err(MalformedRow::InvalidAmount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{chargeback, deposit, dispute, resolve, withdrawal, FailingReader};

    /// The header line, as most tests write it.
    const HEADER_LINE: &str = "type,client,tx,amount";

    fn is_header(input: &[u8]) -> bool {
        matches!(read_header(input), Ok(Start::Header(_)))
    }

    fn is_wrong_header(input: &[u8]) -> bool {
        matches!(read_header(input), Ok(Start::WrongHeader))
    }

    fn has_no_header_line(input: &[u8]) -> bool {
        matches!(read_header(input), Ok(Start::NoHeaderLine))
    }

    /// Every row of `input`, in order. The input must begin with the header
    /// line.
    fn rows_of(input: &[u8]) -> Vec<Result<Transaction, MalformedRow>> {
        match read_header(input).expect("reading from memory cannot fail") {
            Start::Header(rows) => rows
                .map(|row| row.expect("reading from memory cannot fail"))
                .collect(),
            other => panic!("the header line was not accepted: {other:?}"),
        }
    }

    /// Reads `line` as the only row under a header line.
    fn row(line: &str) -> Result<Transaction, MalformedRow> {
        let input = format!("{HEADER_LINE}\n{line}\n");
        let mut rows = rows_of(input.as_bytes());
        assert_eq!(rows.len(), 1, "{line:?} was not read as one row");
        rows.remove(0)
    }

    // The header line.

    #[test]
    fn header_line_is_accepted() {
        assert!(is_header(b"type,client,tx,amount\n"));
        assert!(is_header(b"type,client,tx,amount"));
    }

    #[test]
    fn header_names_may_have_whitespace_around_them() {
        assert!(is_header(b"type, client, tx, amount\n"));
        assert!(is_header(b" type , client ,\ttx\t, amount \n"));
    }

    #[test]
    fn empty_lines_before_the_header_are_skipped() {
        assert!(is_header(b"\n\r\n\ntype,client,tx,amount\n"));
    }

    #[test]
    fn byte_order_mark_before_the_header_is_skipped() {
        let input = b"\xef\xbb\xbftype,client,tx,amount\r\ndeposit,1,1,10\r\n";
        assert_eq!(rows_of(input), [Ok(deposit(1, 1, "10"))]);
    }

    #[test]
    fn quoted_header_is_accepted() {
        assert!(is_header(b"\"type\",\"client\",\"tx\",\"amount\"\n"));
    }

    #[test]
    fn data_row_in_place_of_the_header_is_a_wrong_header() {
        assert!(is_wrong_header(b"deposit,1,1,10\ndeposit,1,2,5\n"));
    }

    #[test]
    fn header_in_capitals_is_a_wrong_header() {
        assert!(is_wrong_header(b"Type,Client,Tx,Amount\n"));
        assert!(is_wrong_header(b"TYPE,CLIENT,TX,AMOUNT\n"));
    }

    #[test]
    fn header_in_another_order_is_a_wrong_header() {
        assert!(is_wrong_header(b"client,type,tx,amount\n"));
    }

    #[test]
    fn header_with_another_number_of_columns_is_a_wrong_header() {
        assert!(is_wrong_header(b"type,client,tx\n"));
        assert!(is_wrong_header(b"type,client,tx,amount,note\n"));
        assert!(is_wrong_header(b"type,client,tx,amount,\n"));
    }

    #[test]
    fn line_of_spaces_in_place_of_the_header_is_a_wrong_header() {
        // The line of spaces is the first line that is not empty, and the
        // real header line after it is never tried.
        assert!(is_wrong_header(b"   \ntype,client,tx,amount\n"));
    }

    #[test]
    fn first_line_that_is_not_utf8_is_a_wrong_header() {
        assert!(is_wrong_header(
            b"type,client,tx,amo\xffunt\ntype,client,tx,amount\n"
        ));
    }

    #[test]
    fn input_of_zero_bytes_has_no_header_line() {
        assert!(has_no_header_line(b""));
    }

    #[test]
    fn input_of_only_empty_lines_has_no_header_line() {
        assert!(has_no_header_line(b"\n\n\n"));
        assert!(has_no_header_line(b"\r\n\r\n"));
    }

    #[test]
    fn lone_byte_order_mark_has_no_header_line() {
        assert!(has_no_header_line(b"\xef\xbb\xbf"));
    }

    #[test]
    fn read_failure_before_the_header_is_an_error() {
        // On the very first read, and part-way through the header line. A
        // directory given as the input file fails like the first.
        assert!(read_header(FailingReader::after(b"")).is_err());
        assert!(read_header(FailingReader::after(b"type,cli")).is_err());
    }

    // Well-formed rows.

    #[test]
    fn deposit_and_withdrawal_rows_are_read() {
        assert_eq!(row("deposit,7,31,2.5"), Ok(deposit(7, 31, "2.5")));
        assert_eq!(row("withdrawal,7,32,0.75"), Ok(withdrawal(7, 32, "0.75")));
    }

    #[test]
    fn dispute_resolve_and_chargeback_rows_with_three_fields_are_read() {
        assert_eq!(row("dispute,7,31"), Ok(dispute(7, 31)));
        assert_eq!(row("resolve,7,31"), Ok(resolve(7, 31)));
        assert_eq!(row("chargeback,7,31"), Ok(chargeback(7, 31)));
    }

    #[test]
    fn dispute_resolve_and_chargeback_rows_with_an_empty_fourth_field_are_read() {
        assert_eq!(row("dispute,7,31,"), Ok(dispute(7, 31)));
        assert_eq!(row("resolve,7,31,"), Ok(resolve(7, 31)));
        assert_eq!(row("chargeback,7,31,"), Ok(chargeback(7, 31)));
    }

    #[test]
    fn fourth_field_of_a_dispute_resolve_or_chargeback_row_is_not_looked_at() {
        assert_eq!(row("dispute,7,31,3.0"), Ok(dispute(7, 31)));
        assert_eq!(row("dispute,7,31,abc"), Ok(dispute(7, 31)));
        assert_eq!(row("resolve,7,31,-1"), Ok(resolve(7, 31)));
        assert_eq!(row("chargeback,7,31,1e2"), Ok(chargeback(7, 31)));
    }

    #[test]
    fn whitespace_around_fields_is_not_part_of_them() {
        assert_eq!(row("deposit, 7, 31, 2.5"), Ok(deposit(7, 31, "2.5")));
        assert_eq!(row(" deposit , 7 ,\t31\t, 2.5 "), Ok(deposit(7, 31, "2.5")));
        assert_eq!(row("dispute , 7 , 31 "), Ok(dispute(7, 31)));
        // A no-break space is whitespace too.
        assert_eq!(
            row("deposit,\u{a0}7\u{a0},31,2.5"),
            Ok(deposit(7, 31, "2.5"))
        );
    }

    #[test]
    fn ids_with_leading_zeros_are_read() {
        assert_eq!(row("deposit,007,0031,2.5"), Ok(deposit(7, 31, "2.5")));
    }

    #[test]
    fn largest_ids_are_read() {
        assert_eq!(
            row("deposit,65535,4294967295,1"),
            Ok(deposit(65535, 4_294_967_295, "1"))
        );
    }

    #[test]
    fn quoted_fields_are_read_without_their_quotes() {
        assert_eq!(
            row("\"deposit\",\"7\",\"31\",\"2.5\""),
            Ok(deposit(7, 31, "2.5"))
        );
    }

    #[test]
    fn empty_lines_between_rows_are_skipped() {
        let input = b"type,client,tx,amount\n\ndeposit,1,1,10\n\n\ndeposit,1,2,5\n\n";
        assert_eq!(
            rows_of(input),
            [Ok(deposit(1, 1, "10")), Ok(deposit(1, 2, "5"))]
        );
    }

    #[test]
    fn rows_with_crlf_line_ends_and_no_final_newline_are_read() {
        let input = b"type,client,tx,amount\r\ndeposit,1,1,10\r\ndispute,1,1";
        assert_eq!(rows_of(input), [Ok(deposit(1, 1, "10")), Ok(dispute(1, 1))]);
    }

    // Malformed rows. Each test names the rule that the row breaks, and
    // checks that this rule is the reason given.

    #[test]
    fn row_with_more_than_four_fields_is_malformed() {
        assert_eq!(
            row("deposit,1,7,1,000.50"),
            Err(MalformedRow::WrongFieldCount)
        );
        // A comma after the amount makes a fifth, empty field.
        assert_eq!(row("deposit,1,7,10,"), Err(MalformedRow::WrongFieldCount));
        assert_eq!(row("dispute,1,7,,"), Err(MalformedRow::WrongFieldCount));
    }

    #[test]
    fn row_with_fewer_than_three_fields_is_malformed() {
        assert_eq!(row("deposit"), Err(MalformedRow::WrongFieldCount));
        assert_eq!(row("deposit,1"), Err(MalformedRow::WrongFieldCount));
        assert_eq!(row("dispute,1"), Err(MalformedRow::WrongFieldCount));
    }

    #[test]
    fn line_of_spaces_is_a_malformed_row() {
        // Unlike an empty line, it is a row: one with a single empty field.
        assert_eq!(row("   "), Err(MalformedRow::WrongFieldCount));
    }

    #[test]
    fn type_in_capitals_is_malformed() {
        assert_eq!(row("Deposit,1,2,10"), Err(MalformedRow::UnknownType));
        assert_eq!(row("DISPUTE,1,2"), Err(MalformedRow::UnknownType));
    }

    #[test]
    fn unknown_type_is_malformed() {
        for line in [
            "transfer,1,2,5",
            ",1,2,5",
            "deposits,1,2,5",
            "de posit,1,2,5",
        ] {
            assert_eq!(row(line), Err(MalformedRow::UnknownType), "{line:?}");
        }
    }

    /// Text that is not plain digits, in the ways an id is likely to be
    /// written wrongly. The last one is a digit, but not an ASCII digit.
    const NOT_PLAIN_DIGITS: [&str; 8] = ["+1", "-1", "1.0", "0x10", "1_0", "", "one", "\u{663}"];

    #[test]
    fn client_that_is_not_plain_digits_is_malformed() {
        for text in NOT_PLAIN_DIGITS {
            let line = format!("deposit,{text},2,5");
            assert_eq!(row(&line), Err(MalformedRow::InvalidClient), "{line:?}");
        }
    }

    #[test]
    fn client_above_65535_is_malformed() {
        assert_eq!(row("deposit,65536,1,1"), Err(MalformedRow::InvalidClient));
    }

    #[test]
    fn tx_that_is_not_plain_digits_is_malformed() {
        for text in NOT_PLAIN_DIGITS {
            let line = format!("deposit,1,{text},5");
            assert_eq!(row(&line), Err(MalformedRow::InvalidTx), "{line:?}");
        }
        // The id is checked on rows without an amount as well.
        assert_eq!(row("dispute,1,x"), Err(MalformedRow::InvalidTx));
    }

    #[test]
    fn tx_above_4294967295_is_malformed() {
        assert_eq!(row("deposit,1,4294967296,1"), Err(MalformedRow::InvalidTx));
        let long = format!("deposit,1,{},1", "9".repeat(40));
        assert_eq!(row(&long), Err(MalformedRow::InvalidTx));
    }

    #[test]
    fn deposit_or_withdrawal_without_a_fourth_field_is_malformed() {
        assert_eq!(row("deposit,1,2"), Err(MalformedRow::MissingAmount));
        assert_eq!(row("withdrawal,1,2"), Err(MalformedRow::MissingAmount));
    }

    #[test]
    fn deposit_or_withdrawal_with_an_empty_amount_is_malformed() {
        let empty = Err(MalformedRow::InvalidAmount(
            ParseAmountError::NotPlainDecimal,
        ));
        assert_eq!(row("deposit,1,2,"), empty);
        assert_eq!(row("withdrawal,1,2,"), empty);
        assert_eq!(row("deposit,1,2,   "), empty);
    }

    #[test]
    fn amount_of_zero_is_malformed() {
        let zero = Err(MalformedRow::InvalidAmount(ParseAmountError::Zero));
        assert_eq!(row("deposit,1,2,0"), zero);
        assert_eq!(row("withdrawal,1,2,0.0000"), zero);
    }

    #[test]
    fn amount_with_more_than_four_decimals_is_malformed() {
        assert_eq!(
            row("deposit,1,2,1.00005"),
            Err(MalformedRow::InvalidAmount(
                ParseAmountError::TooManyDecimals
            ))
        );
    }

    #[test]
    fn amount_that_is_not_a_plain_decimal_is_malformed() {
        for text in ["1e2", "+1.5", "-1", ".5", "5.", "1_000"] {
            let line = format!("deposit,1,2,{text}");
            assert_eq!(
                row(&line),
                Err(MalformedRow::InvalidAmount(
                    ParseAmountError::NotPlainDecimal
                )),
                "{line:?}"
            );
        }
    }

    #[test]
    fn amount_too_large_for_the_type_is_malformed() {
        assert_eq!(
            row("deposit,1,2,922337203685477.5808"),
            Err(MalformedRow::InvalidAmount(ParseAmountError::OutOfRange))
        );
    }

    #[test]
    fn quote_after_a_space_is_an_ordinary_character() {
        // Only a quote at the very start of a field opens a quoted field.
        // This one stays in the text, which is then not an id.
        assert_eq!(
            row("deposit, \"7\", 31, 2.5"),
            Err(MalformedRow::InvalidClient)
        );
    }

    #[test]
    fn first_broken_rule_is_the_reason() {
        // Every field of the first row is wrong. Each later row has one
        // more field put right.
        assert_eq!(
            row("transfer,x,y,z,extra"),
            Err(MalformedRow::WrongFieldCount)
        );
        assert_eq!(row("transfer,x,y,z"), Err(MalformedRow::UnknownType));
        assert_eq!(row("deposit,x,y,z"), Err(MalformedRow::InvalidClient));
        assert_eq!(row("deposit,1,y,z"), Err(MalformedRow::InvalidTx));
        assert_eq!(
            row("deposit,1,2,z"),
            Err(MalformedRow::InvalidAmount(
                ParseAmountError::NotPlainDecimal
            ))
        );
        // The ids are checked before the amount is asked for.
        assert_eq!(row("deposit,x,2"), Err(MalformedRow::InvalidClient));
    }

    #[test]
    fn row_that_is_not_utf8_is_malformed_and_the_next_row_is_read() {
        // The byte is in the fourth field of a dispute row, which would not
        // have been looked at. The row is malformed all the same.
        let input = b"type,client,tx,amount\ndeposit,1,1,10\ndispute,1,1,\xff\ndeposit,1,3,1\n";
        assert_eq!(
            rows_of(input),
            [
                Ok(deposit(1, 1, "10")),
                Err(MalformedRow::NotUtf8),
                Ok(deposit(1, 3, "1")),
            ]
        );
    }

    #[test]
    fn unclosed_quote_takes_the_rest_of_the_input_into_one_field() {
        // In the unread fourth field of a dispute row: the two deposits
        // after it are never seen.
        let input = b"type,client,tx,amount\n\
                      deposit,1,1,10\n\
                      dispute,1,1,\"oops\n\
                      deposit,1,2,5\n\
                      deposit,1,3,5\n";
        assert_eq!(rows_of(input), [Ok(deposit(1, 1, "10")), Ok(dispute(1, 1))]);

        // In the amount of a deposit row: one malformed row.
        let input = b"type,client,tx,amount\ndeposit,1,1,\"5\ndeposit,1,2,5\n";
        assert_eq!(
            rows_of(input),
            [Err(MalformedRow::InvalidAmount(
                ParseAmountError::NotPlainDecimal
            ))]
        );
    }

    // Failures to read.

    #[test]
    fn rows_before_a_read_failure_are_yielded_first() {
        let input = FailingReader::after(b"type,client,tx,amount\ndeposit,1,1,10\ndeposit,1,2,5\n");
        let Ok(Start::Header(mut rows)) = read_header(input) else {
            panic!("the header line was not accepted");
        };
        assert!(matches!(rows.next(), Some(Ok(Ok(row))) if row == deposit(1, 1, "10")));
        assert!(matches!(rows.next(), Some(Ok(Ok(row))) if row == deposit(1, 2, "5")));
        assert!(matches!(rows.next(), Some(Err(_))));
    }

    #[test]
    fn read_failure_is_yielded_once_and_ends_the_rows() {
        // The input breaks off in the middle of the second row.
        let input = FailingReader::after(b"type,client,tx,amount\ndeposit,1,1,10\ndeposit,1,2");
        let Ok(Start::Header(mut rows)) = read_header(input) else {
            panic!("the header line was not accepted");
        };
        assert!(matches!(rows.next(), Some(Ok(Ok(_)))));
        // An error, and not a malformed row.
        assert!(matches!(rows.next(), Some(Err(_))));
        assert!(rows.next().is_none());
    }
}
