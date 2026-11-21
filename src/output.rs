//! Writes client accounts as CSV text.
//!
//! The output is the header line `client,available,held,total,locked`,
//! followed by one row for each client. Every amount is written with exactly
//! four decimals, and `locked` as `true` or `false`. The rows are in ascending
//! order of client id.

use std::io::{self, BufWriter, Write};

use crate::engine::{Account, ClientId, Engine};

/// The header line of the output.
const HEADER: &str = "client,available,held,total,locked";

/// Writes the header line and one row for each client of `engine` to
/// `output`, and flushes it.
///
/// The rows are in ascending order of client id, so the same accounts always
/// give the same bytes. An engine without clients gives the header line
/// alone. `output` needs no buffering of its own: it is written in blocks.
///
/// # Errors
///
/// Returns the first error of `output`. Part of the text may have been
/// written by then.
pub fn write_accounts<W: Write>(output: W, engine: &Engine) -> io::Result<()> {
    let mut output = BufWriter::new(output);
    writeln!(output, "{HEADER}")?;

    // The engine yields its accounts in an order that changes from run to
    // run. Sorted, the output of a run can be compared with that of another
    // byte for byte.
    let mut accounts: Vec<(ClientId, &Account)> = engine.accounts().collect();
    accounts.sort_unstable_by_key(|(client, _)| client.0);

    for (client, account) in accounts {
        writeln!(
            output,
            "{},{},{},{},{}",
            client.0,
            account.available(),
            account.held(),
            account.total(),
            account.is_locked()
        )?;
    }
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Outcome, Transaction};
    use crate::test_support::{chargeback, deposit, dispute, withdrawal, FailingWriter};

    /// An engine that has applied `transactions` in order.
    fn engine_after(transactions: &[Transaction]) -> Engine {
        let mut engine = Engine::new();
        for transaction in transactions {
            assert_eq!(
                engine.process(*transaction),
                Outcome::Applied,
                "{transaction:?}"
            );
        }
        engine
    }

    /// What is written for `engine`, as text.
    fn written(engine: &Engine) -> String {
        let mut output = Vec::new();
        write_accounts(&mut output, engine).expect("writing to memory cannot fail");
        String::from_utf8(output).expect("the output is text")
    }

    #[test]
    fn amounts_are_written_with_four_decimals() {
        let engine = engine_after(&[deposit(1, 1, "1.5")]);
        assert_eq!(
            written(&engine),
            "client,available,held,total,locked\n\
             1,1.5000,0.0000,1.5000,false\n"
        );
    }

    #[test]
    fn engine_without_clients_gives_the_header_line_alone() {
        assert_eq!(
            written(&Engine::new()),
            "client,available,held,total,locked\n"
        );
    }

    #[test]
    fn rows_are_in_ascending_order_of_client_id() {
        // The order is that of the numbers, in which 41 comes before 300. As
        // text it would come after.
        let engine = engine_after(&[
            deposit(300, 1, "3"),
            deposit(2, 2, "2"),
            deposit(65535, 3, "5"),
            deposit(0, 4, "4"),
            deposit(41, 5, "1"),
        ]);
        assert_eq!(
            written(&engine),
            "client,available,held,total,locked\n\
             0,4.0000,0.0000,4.0000,false\n\
             2,2.0000,0.0000,2.0000,false\n\
             41,1.0000,0.0000,1.0000,false\n\
             300,3.0000,0.0000,3.0000,false\n\
             65535,5.0000,0.0000,5.0000,false\n"
        );
    }

    #[test]
    fn balance_below_zero_is_written_with_a_minus_sign() {
        let engine = engine_after(&[deposit(1, 1, "10"), withdrawal(1, 2, "8"), dispute(1, 1)]);
        assert_eq!(
            written(&engine),
            "client,available,held,total,locked\n\
             1,-8.0000,10.0000,2.0000,false\n"
        );
    }

    #[test]
    fn locked_account_is_written_as_true() {
        let engine = engine_after(&[deposit(1, 1, "10"), dispute(1, 1), chargeback(1, 1)]);
        assert_eq!(
            written(&engine),
            "client,available,held,total,locked\n\
             1,0.0000,0.0000,0.0000,true\n"
        );
    }

    #[test]
    fn write_failure_is_returned() {
        let engine = engine_after(&[deposit(1, 1, "10")]);
        assert!(write_accounts(FailingWriter, &engine).is_err());
    }
}
