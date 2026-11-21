//! Helpers shared by the unit tests of more than one module.

use std::io::{self, Read, Write};

use crate::amount::Amount;
use crate::engine::{ClientId, Transaction, TxId};

/// Reads `text` as an amount.
pub(crate) fn amount(text: &str) -> Amount {
    text.parse().expect("a test amount is well-formed")
}

// The five transactions, written as briefly as a test wants them: plain
// numbers for the ids, and the amount as text.

pub(crate) fn deposit(client: u16, tx: u32, amount_text: &str) -> Transaction {
    Transaction::Deposit {
        client: ClientId(client),
        tx: TxId(tx),
        amount: amount(amount_text),
    }
}

pub(crate) fn withdrawal(client: u16, tx: u32, amount_text: &str) -> Transaction {
    Transaction::Withdrawal {
        client: ClientId(client),
        tx: TxId(tx),
        amount: amount(amount_text),
    }
}

pub(crate) fn dispute(client: u16, tx: u32) -> Transaction {
    Transaction::Dispute {
        client: ClientId(client),
        tx: TxId(tx),
    }
}

pub(crate) fn resolve(client: u16, tx: u32) -> Transaction {
    Transaction::Resolve {
        client: ClientId(client),
        tx: TxId(tx),
    }
}

pub(crate) fn chargeback(client: u16, tx: u32) -> Transaction {
    Transaction::Chargeback {
        client: ClientId(client),
        tx: TxId(tx),
    }
}

/// An input that breaks off: it hands out its bytes, then fails every later
/// read. It never reports the end of the input.
pub(crate) struct FailingReader<'a> {
    remaining: &'a [u8],
}

impl<'a> FailingReader<'a> {
    /// A reader that yields `bytes` and then fails.
    pub(crate) fn after(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }
}

impl Read for FailingReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.remaining.is_empty() {
            // `io::Error::other` would be shorter, but it needs Rust 1.74.
            return Err(io::Error::new(io::ErrorKind::Other, "the input broke off"));
        }
        self.remaining.read(buffer)
    }
}

/// An output that cannot be written: every write fails.
pub(crate) struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::Other, "the output is closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
