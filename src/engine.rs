//! The rules for client accounts, as pure logic.
//!
//! An [`Engine`] takes one [`Transaction`] at a time and returns its
//! [`Outcome`]: applied, refused with a reason, or ignored with a reason. It
//! reads no input and writes no output. Turning rows of text into
//! transactions, counting the outcomes and printing the [`Account`]s is the
//! caller's work.
//!
//! # Deposits and withdrawals
//!
//! The checks run in this order, and the first one that fails is the reason
//! the transaction is refused:
//!
//! 1. The amount is greater than zero.
//! 2. The account is not locked.
//! 3. The id is not the id of a stored transaction.
//! 4. For a deposit: the new available balance and the new total fit the
//!    amount type. For a withdrawal: the available funds cover the amount.
//!
//! The client's record is created once the first check has passed. A client
//! therefore has an account as soon as a deposit or withdrawal with an amount
//! above zero names it, even if that transaction is then refused.
//!
//! An applied deposit is kept as a *stored transaction*: its client, its
//! amount and its dispute state, under its id. Nothing else is remembered, so
//! the id of a withdrawal or of a refused transaction can appear again.
//!
//! # Disputes, resolves and chargebacks
//!
//! Each names a stored transaction by id. The checks run in this order, and
//! the first one that fails is the reason the row is ignored:
//!
//! 1. The id is the id of a stored transaction. Only an applied deposit is
//!    stored, so nothing else can be disputed.
//! 2. The stored transaction belongs to the client that the row names.
//! 3. The state of the deposit allows the row, as in the table below.
//! 4. The new balances fit the amount type.
//!
//! | The deposit is | dispute         | resolve            | chargeback                   |
//! |----------------|-----------------|--------------------|------------------------------|
//! | normal         | hold the amount | ignored            | ignored                      |
//! | disputed       | ignored         | release the amount | reverse it, lock the account |
//! | charged back   | ignored         | ignored            | ignored                      |
//!
//! A dispute holds the whole amount of the deposit even when part of it has
//! already been withdrawn, so the available balance can go below zero. A
//! locked account refuses every deposit and withdrawal, but disputes, resolves
//! and chargebacks on its deposits are still processed. An ignored row never
//! creates a client record.

// The engine performs no I/O. Printing is the easy way to break that by
// accident, so the printing macros are denied in this file, tests included.
#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

use std::collections::HashMap;

use crate::amount::Amount;

/// Identifies a client. Every `u16` is a valid client id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientId(pub u16);

/// Identifies a transaction. Every `u32` is a valid transaction id.
///
/// It is a different type from [`ClientId`], so neither can be passed where
/// the other is expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TxId(pub u32);

/// One transaction, as the engine receives it.
///
/// A deposit or a withdrawal always carries an amount. A dispute, a resolve or
/// a chargeback cannot carry one: it names a stored transaction by id and acts
/// on the amount stored with it.
///
/// In every variant `client` is the client that the transaction names. `tx` is
/// the transaction's own id for a deposit or a withdrawal, and the id of the
/// deposit in question for the other three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transaction {
    /// Adds `amount` to the client's available funds.
    Deposit {
        client: ClientId,
        tx: TxId,
        amount: Amount,
    },
    /// Takes `amount` from the client's available funds.
    Withdrawal {
        client: ClientId,
        tx: TxId,
        amount: Amount,
    },
    /// Starts a dispute on the deposit `tx`: its amount moves from available
    /// to held.
    Dispute { client: ClientId, tx: TxId },
    /// Ends the dispute on the deposit `tx` and keeps the deposit: its amount
    /// moves from held back to available.
    Resolve { client: ClientId, tx: TxId },
    /// Ends the dispute on the deposit `tx` by reversing the deposit: its
    /// amount leaves held, and the account is locked.
    Chargeback { client: ClientId, tx: TxId },
}

/// The balances of one client.
///
/// - `held` is never negative. `available` can be, after a dispute on a
///   deposit that was partly withdrawn.
/// - `available + held` always fits the amount type, so the total can always
///   be shown.
/// - Only an [`Engine`] creates or changes an account. There is no public
///   constructor and no setter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Account {
    available: Amount,
    held: Amount,
    locked: bool,
}

impl Account {
    /// A client's record as it is created: no funds, not locked.
    const NEW: Account = Account {
        available: Amount::ZERO,
        held: Amount::ZERO,
        locked: false,
    };

    /// The funds the client can withdraw. Negative if more was held for
    /// disputes than the account had left.
    pub fn available(&self) -> Amount {
        self.available
    }

    /// The funds held for deposits under dispute. Never negative.
    pub fn held(&self) -> Amount {
        self.held
    }

    /// All funds, available or held: exactly `available + held`.
    ///
    /// The total is worked out on request and never stored, so it cannot
    /// disagree with the two balances it is made of.
    ///
    /// # Panics
    ///
    /// Only if the sum did not fit the amount type. That cannot happen to an
    /// account held by an [`Engine`]: a deposit that would push the sum past
    /// the top of the range is refused, and because `held` is never negative
    /// the sum is never below `available`.
    //
    // This is the only place in the crate that can panic, and the lint that
    // forbids it is lifted for this one function. The alternative was to
    // return an `Option` and make every caller handle a case that the engine
    // rules out.
    #[allow(clippy::expect_used)]
    pub fn total(&self) -> Amount {
        self.available
            .checked_add(self.held)
            .expect("available + held always fits: a deposit that would not is refused")
    }

    /// Whether the account is locked. A chargeback locks it, and nothing
    /// unlocks it.
    pub fn is_locked(&self) -> bool {
        self.locked
    }
}

/// What the engine did with one transaction.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The transaction took effect.
    Applied,
    /// A deposit or withdrawal that the rules declined. No balance changed and
    /// the transaction was not stored.
    Refused(RefusalReason),
    /// A dispute, resolve or chargeback that had no effect at all.
    Ignored(IgnoreReason),
}

/// Why a deposit or withdrawal was refused.
///
/// The variants follow the order of the checks. A transaction that would fail
/// several checks is reported under the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    /// The amount is zero or negative. Nothing changes, and no client record
    /// is created.
    AmountNotPositive,
    /// The account is locked. A locked account takes no deposit and no
    /// withdrawal.
    AccountLocked,
    /// The id is already the id of a stored transaction, of this client or of
    /// another. The stored transaction is untouched.
    RepeatedId,
    /// A withdrawal of more than the available funds. Held funds do not count.
    InsufficientFunds,
    /// A deposit after which the available balance, or the total, would not
    /// fit the amount type.
    Overflow,
}

/// Why a dispute, resolve or chargeback was ignored.
///
/// The variants follow the order of the checks. A row that would fail several
/// checks is reported under the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreReason {
    /// The id is not the id of a stored transaction: it was never seen, or it
    /// belongs to a withdrawal or to a refused deposit.
    NotStored,
    /// The stored transaction belongs to another client than the one the row
    /// names.
    ClientMismatch,
    /// A dispute on a deposit that is already under dispute.
    AlreadyDisputed,
    /// A resolve or a chargeback on a deposit that is not under dispute.
    NotDisputed,
    /// Any row on a deposit that was charged back. A chargeback is final.
    AlreadyChargedBack,
    /// The new balances would not fit the amount type. Only a dispute can run
    /// into this: a resolve or a chargeback moves an amount that is already
    /// held.
    Overflow,
}

/// A stored transaction: an applied deposit, kept so that it can be disputed.
///
/// Only what a later dispute needs is kept. The id is the key under which the
/// engine files it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoredDeposit {
    client: ClientId,
    amount: Amount,
    state: DisputeState,
}

/// Where a stored deposit stands. Every `match` on this lists all three
/// states, so a new state cannot be added without deciding what each kind of
/// row does in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisputeState {
    /// Not under dispute: never disputed, or disputed and resolved.
    Normal,
    /// Under dispute: its amount is held.
    Disputed,
    /// Reversed by a chargeback. Final.
    ChargedBack,
}

/// Applies transactions to client accounts, one at a time, in the order given.
///
/// - A transaction is checked completely before anything changes. One that is
///   refused or ignored leaves every balance and every stored transaction as
///   it was.
/// - All arithmetic is checked, and no sequence of transactions makes
///   [`Engine::process`] panic.
/// - The engine performs no I/O.
/// - Memory grows only with the number of clients and the number of stored
///   transactions. Withdrawals, disputes, resolves and chargebacks add nothing.
#[derive(Debug, Default)]
pub struct Engine {
    /// One record for each client that a deposit or withdrawal with an amount
    /// above zero has named.
    accounts: HashMap<ClientId, Account>,
    /// Every applied deposit, under its id. The only record of the past.
    deposits: HashMap<TxId, StoredDeposit>,
}

impl Engine {
    /// An engine with no clients and no stored transactions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies the rules to one transaction and reports what happened.
    ///
    /// Returns [`Outcome::Applied`] if the transaction took effect. Otherwise
    /// the outcome carries the first check that failed, and nothing has
    /// changed except that a refused deposit or withdrawal may have created
    /// its client's record. Never panics.
    pub fn process(&mut self, transaction: Transaction) -> Outcome {
        match transaction {
            Transaction::Deposit { client, tx, amount } => {
                refused_or_applied(self.deposit(client, tx, amount))
            }
            Transaction::Withdrawal { client, tx, amount } => {
                refused_or_applied(self.withdraw(client, tx, amount))
            }
            Transaction::Dispute { client, tx } => ignored_or_applied(self.dispute(client, tx)),
            Transaction::Resolve { client, tx } => ignored_or_applied(self.resolve(client, tx)),
            Transaction::Chargeback { client, tx } => {
                ignored_or_applied(self.chargeback(client, tx))
            }
        }
    }

    /// Every client record with its client id, in no particular order.
    ///
    /// There is one entry for each client that a deposit or withdrawal with an
    /// amount above zero has named, whether or not that transaction was
    /// applied.
    pub fn accounts(&self) -> impl Iterator<Item = (ClientId, &Account)> {
        self.accounts
            .iter()
            .map(|(client, account)| (*client, account))
    }

    /// Credits the client and stores the deposit, so that it can be disputed.
    fn deposit(&mut self, client: ClientId, tx: TxId, amount: Amount) -> Result<(), RefusalReason> {
        let account = self.admit(client, tx, amount)?;

        // The total is worked out on request and never stored. So it is not
        // enough for the new available balance to fit: the new total has to
        // fit as well, or a later request for it would fail.
        let available = account
            .available
            .checked_add(amount)
            .ok_or(RefusalReason::Overflow)?;
        if available.checked_add(account.held).is_none() {
            return Err(RefusalReason::Overflow);
        }

        account.available = available;
        self.deposits.insert(
            tx,
            StoredDeposit {
                client,
                amount,
                state: DisputeState::Normal,
            },
        );
        Ok(())
    }

    /// Debits the client. A withdrawal is not stored, so it cannot be disputed.
    fn withdraw(
        &mut self,
        client: ClientId,
        tx: TxId,
        amount: Amount,
    ) -> Result<(), RefusalReason> {
        let account = self.admit(client, tx, amount)?;

        // The funds are sufficient when what remains is not negative. If the
        // remainder does not even fit the amount type it is below the range,
        // which is negative too. Held funds are not part of `available`.
        let available = account
            .available
            .checked_sub(amount)
            .filter(|remaining| *remaining >= Amount::ZERO)
            .ok_or(RefusalReason::InsufficientFunds)?;

        account.available = available;
        Ok(())
    }

    /// Holds the amount of a deposit that is not under dispute.
    fn dispute(&mut self, client: ClientId, tx: TxId) -> Result<(), IgnoreReason> {
        let (deposit, account) = self.stored_deposit(client, tx)?;
        match deposit.state {
            DisputeState::Normal => {}
            DisputeState::Disputed => return Err(IgnoreReason::AlreadyDisputed),
            DisputeState::ChargedBack => return Err(IgnoreReason::AlreadyChargedBack),
        }

        // The whole amount is held, even if less than that is available, so
        // `available` may go below zero. Either step can fail when deposits
        // near the top of the range were made, withdrawn and then disputed.
        let available = account
            .available
            .checked_sub(deposit.amount)
            .ok_or(IgnoreReason::Overflow)?;
        let held = account
            .held
            .checked_add(deposit.amount)
            .ok_or(IgnoreReason::Overflow)?;

        account.available = available;
        account.held = held;
        deposit.state = DisputeState::Disputed;
        Ok(())
    }

    /// Releases the amount of a deposit under dispute. The deposit can be
    /// disputed again afterwards.
    fn resolve(&mut self, client: ClientId, tx: TxId) -> Result<(), IgnoreReason> {
        let (deposit, account) = self.stored_deposit(client, tx)?;
        match deposit.state {
            DisputeState::Disputed => {}
            DisputeState::Normal => return Err(IgnoreReason::NotDisputed),
            DisputeState::ChargedBack => return Err(IgnoreReason::AlreadyChargedBack),
        }

        // Neither step can fail. `held` is the sum of the amounts under
        // dispute, this one included, and releasing it leaves `available` no
        // higher than the total, which fits. The checked operations still
        // return an `Option`; a row that did not fit would simply be ignored.
        let held = account
            .held
            .checked_sub(deposit.amount)
            .ok_or(IgnoreReason::Overflow)?;
        let available = account
            .available
            .checked_add(deposit.amount)
            .ok_or(IgnoreReason::Overflow)?;

        account.held = held;
        account.available = available;
        deposit.state = DisputeState::Normal;
        Ok(())
    }

    /// Reverses a deposit under dispute and locks the account.
    fn chargeback(&mut self, client: ClientId, tx: TxId) -> Result<(), IgnoreReason> {
        let (deposit, account) = self.stored_deposit(client, tx)?;
        match deposit.state {
            DisputeState::Disputed => {}
            DisputeState::Normal => return Err(IgnoreReason::NotDisputed),
            DisputeState::ChargedBack => return Err(IgnoreReason::AlreadyChargedBack),
        }

        // This cannot fail, for the same reason as in a resolve. The held
        // funds leave the account, so the total drops by the amount, and it
        // may go below zero if the deposit was partly withdrawn.
        let held = account
            .held
            .checked_sub(deposit.amount)
            .ok_or(IgnoreReason::Overflow)?;

        account.held = held;
        account.locked = true;
        deposit.state = DisputeState::ChargedBack;
        Ok(())
    }

    /// The checks that a deposit and a withdrawal share, in order. Returns the
    /// client's account, which is created here if the client is new.
    fn admit(
        &mut self,
        client: ClientId,
        tx: TxId,
        amount: Amount,
    ) -> Result<&mut Account, RefusalReason> {
        // An amount read from text is always above zero. One made by
        // arithmetic may not be, and a negative amount would turn a deposit
        // into a debit. Such a transaction is not well-formed, so nothing
        // changes: the client does not even get a record.
        if amount <= Amount::ZERO {
            return Err(RefusalReason::AmountNotPositive);
        }

        // From here on the client has a record, whether or not the
        // transaction goes through.
        let account = self.accounts.entry(client).or_insert(Account::NEW);
        if account.locked {
            return Err(RefusalReason::AccountLocked);
        }

        // Ids are unique across all clients. A second deposit under a stored
        // id would replace the stored one, and a later dispute could no
        // longer tell which transaction it means.
        if self.deposits.contains_key(&tx) {
            return Err(RefusalReason::RepeatedId);
        }
        Ok(account)
    }

    /// The checks that a dispute, a resolve and a chargeback share, in order.
    /// Returns the stored deposit that the row names and the account it
    /// belongs to. Creates nothing.
    fn stored_deposit(
        &mut self,
        client: ClientId,
        tx: TxId,
    ) -> Result<(&mut StoredDeposit, &mut Account), IgnoreReason> {
        let deposit = self.deposits.get_mut(&tx).ok_or(IgnoreReason::NotStored)?;
        if deposit.client != client {
            return Err(IgnoreReason::ClientMismatch);
        }

        // The client of a stored deposit always has an account, because the
        // deposit was applied to it. The lookup still returns an `Option`; if
        // the account were missing, the client could not own the deposit.
        let account = self
            .accounts
            .get_mut(&client)
            .ok_or(IgnoreReason::ClientMismatch)?;
        Ok((deposit, account))
    }
}

/// A deposit or withdrawal is either applied or refused. It is never ignored.
fn refused_or_applied(result: Result<(), RefusalReason>) -> Outcome {
    match result {
        Ok(()) => Outcome::Applied,
        Err(reason) => Outcome::Refused(reason),
    }
}

/// A dispute, resolve or chargeback is either applied or ignored. It is never
/// refused.
fn ignored_or_applied(result: Result<(), IgnoreReason>) -> Outcome {
    match result {
        Ok(()) => Outcome::Applied,
        Err(reason) => Outcome::Ignored(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::IgnoreReason::{
        AlreadyChargedBack, AlreadyDisputed, ClientMismatch, NotDisputed, NotStored,
    };
    use super::Outcome::{Applied, Ignored, Refused};
    use super::RefusalReason::{AccountLocked, AmountNotPositive, InsufficientFunds, RepeatedId};
    use super::*;
    use proptest::prelude::*;

    // Each test lists its transactions in order, with the outcome expected of
    // each beside it, and then checks the accounts at the end. Accounts are
    // compared as `client,available,held,total,locked` lines.

    /// An amount that fits an account once but not twice.
    const HUGE: &str = "900000000000000";

    /// What `rows` returns when no client has a record.
    const NO_ROWS: [&str; 0] = [];

    /// Reads `text` as an amount.
    fn amount(text: &str) -> Amount {
        text.parse().expect("a test amount is well-formed")
    }

    fn deposit(client: u16, tx: u32, amount_text: &str) -> Transaction {
        Transaction::Deposit {
            client: ClientId(client),
            tx: TxId(tx),
            amount: amount(amount_text),
        }
    }

    fn withdrawal(client: u16, tx: u32, amount_text: &str) -> Transaction {
        Transaction::Withdrawal {
            client: ClientId(client),
            tx: TxId(tx),
            amount: amount(amount_text),
        }
    }

    fn dispute(client: u16, tx: u32) -> Transaction {
        Transaction::Dispute {
            client: ClientId(client),
            tx: TxId(tx),
        }
    }

    fn resolve(client: u16, tx: u32) -> Transaction {
        Transaction::Resolve {
            client: ClientId(client),
            tx: TxId(tx),
        }
    }

    fn chargeback(client: u16, tx: u32) -> Transaction {
        Transaction::Chargeback {
            client: ClientId(client),
            tx: TxId(tx),
        }
    }

    /// Processes the transactions in order on a new engine and checks the
    /// outcome of each against the one written beside it. Returns the engine.
    fn run(steps: &[(Transaction, Outcome)]) -> Engine {
        let mut engine = Engine::new();
        for (index, (transaction, expected)) in steps.iter().enumerate() {
            let outcome = engine.process(*transaction);
            assert_eq!(
                outcome,
                *expected,
                "transaction {}: {transaction:?}",
                index + 1
            );
        }
        engine
    }

    /// The accounts as `client,available,held,total,locked` lines, sorted,
    /// because the engine yields them in no particular order.
    fn rows(engine: &Engine) -> Vec<String> {
        let mut rows: Vec<String> = engine
            .accounts()
            .map(|(client, account)| {
                format!(
                    "{},{},{},{},{}",
                    client.0,
                    account.available(),
                    account.held(),
                    account.total(),
                    account.is_locked()
                )
            })
            .collect();
        rows.sort();
        rows
    }

    /// The dispute state of the stored deposit `tx`.
    fn state_of(engine: &Engine, tx: u32) -> DisputeState {
        engine.deposits[&TxId(tx)].state
    }

    /// An engine in which deposit 1, of 10 for client 1, is in `state`.
    fn deposit_in_state(state: DisputeState) -> Engine {
        let mut steps = vec![(deposit(1, 1, "10"), Applied)];
        match state {
            DisputeState::Normal => {}
            DisputeState::Disputed => steps.push((dispute(1, 1), Applied)),
            DisputeState::ChargedBack => {
                steps.extend([(dispute(1, 1), Applied), (chargeback(1, 1), Applied)]);
            }
        }
        let engine = run(&steps);
        assert_eq!(state_of(&engine, 1), state);
        engine
    }

    // Deposits and withdrawals.

    #[test]
    fn deposit_increases_available_and_total() {
        let engine = run(&[
            (deposit(1, 1, "2.5"), Applied),
            (deposit(1, 2, "1.25"), Applied),
        ]);
        assert_eq!(rows(&engine), ["1,3.7500,0.0000,3.7500,false"]);
    }

    #[test]
    fn deposits_to_different_clients_stay_separate() {
        let engine = run(&[
            (deposit(2, 1, "7"), Applied),
            (deposit(1, 2, "0.5"), Applied),
            (deposit(2, 3, "3"), Applied),
        ]);
        assert_eq!(
            rows(&engine),
            [
                "1,0.5000,0.0000,0.5000,false",
                "2,10.0000,0.0000,10.0000,false"
            ]
        );
    }

    #[test]
    fn withdrawal_decreases_available_and_total() {
        let engine = run(&[
            (deposit(1, 1, "9"), Applied),
            (withdrawal(1, 2, "2.75"), Applied),
        ]);
        assert_eq!(rows(&engine), ["1,6.2500,0.0000,6.2500,false"]);
    }

    #[test]
    fn withdrawal_of_the_whole_available_balance_leaves_zero() {
        let engine = run(&[
            (deposit(1, 1, "4.2"), Applied),
            (withdrawal(1, 2, "4.2"), Applied),
        ]);
        assert_eq!(rows(&engine), ["1,0.0000,0.0000,0.0000,false"]);
    }

    #[test]
    fn withdrawal_with_insufficient_available_funds_is_refused() {
        let engine = run(&[
            (deposit(1, 1, "6"), Applied),
            (withdrawal(1, 2, "6.0001"), Refused(InsufficientFunds)),
        ]);
        assert_eq!(rows(&engine), ["1,6.0000,0.0000,6.0000,false"]);
    }

    #[test]
    fn withdrawal_cannot_use_held_funds() {
        // The account holds 15 in total, but 10 of it is held for the dispute.
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (deposit(1, 2, "5"), Applied),
            (dispute(1, 1), Applied),
            (withdrawal(1, 3, "6"), Refused(InsufficientFunds)),
        ]);
        assert_eq!(rows(&engine), ["1,5.0000,10.0000,15.0000,false"]);
    }

    // An amount that is not above zero.

    /// Zero, and an amount below zero. Reading text yields neither, but
    /// arithmetic can.
    fn amounts_not_above_zero() -> [Amount; 2] {
        let below_zero = Amount::ZERO
            .checked_sub(amount("5"))
            .expect("minus five fits");
        [Amount::ZERO, below_zero]
    }

    #[test]
    fn deposit_with_amount_not_above_zero_is_refused() {
        for not_positive in amounts_not_above_zero() {
            let bad_deposit = Transaction::Deposit {
                client: ClientId(1),
                tx: TxId(1),
                amount: not_positive,
            };
            // It is not stored, and the client does not even get a record.
            let engine = run(&[
                (bad_deposit, Refused(AmountNotPositive)),
                (dispute(1, 1), Ignored(NotStored)),
            ]);
            assert_eq!(rows(&engine), NO_ROWS);
        }
    }

    #[test]
    fn withdrawal_with_amount_not_above_zero_is_refused() {
        for not_positive in amounts_not_above_zero() {
            let bad_withdrawal = |client, tx| Transaction::Withdrawal {
                client: ClientId(client),
                tx: TxId(tx),
                amount: not_positive,
            };
            // A withdrawal below zero would have added funds. Client 2 is new
            // here and does not even get a record.
            let engine = run(&[
                (deposit(1, 1, "10"), Applied),
                (bad_withdrawal(1, 2), Refused(AmountNotPositive)),
                (bad_withdrawal(2, 3), Refused(AmountNotPositive)),
            ]);
            assert_eq!(rows(&engine), ["1,10.0000,0.0000,10.0000,false"]);
        }
    }

    // Disputes.

    #[test]
    fn dispute_and_chargeback_on_a_withdrawal_are_ignored() {
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (withdrawal(1, 2, "4"), Applied),
            (dispute(1, 2), Ignored(NotStored)),
            (chargeback(1, 2), Ignored(NotStored)),
        ]);
        assert_eq!(rows(&engine), ["1,6.0000,0.0000,6.0000,false"]);
    }

    #[test]
    fn dispute_holds_the_full_deposit_when_less_is_available() {
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (withdrawal(1, 2, "8"), Applied),
            (dispute(1, 1), Applied),
        ]);
        assert_eq!(rows(&engine), ["1,-8.0000,10.0000,2.0000,false"]);
    }

    #[test]
    fn chargeback_of_an_overdrawn_deposit_leaves_total_negative() {
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (withdrawal(1, 2, "8"), Applied),
            (dispute(1, 1), Applied),
            (chargeback(1, 1), Applied),
        ]);
        assert_eq!(rows(&engine), ["1,-8.0000,0.0000,-8.0000,true"]);
    }

    #[test]
    fn resolved_deposit_can_be_disputed_again() {
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (dispute(1, 1), Applied),
            (resolve(1, 1), Applied),
            (dispute(1, 1), Applied),
        ]);
        assert_eq!(rows(&engine), ["1,0.0000,10.0000,10.0000,false"]);
    }

    #[test]
    fn dispute_naming_another_client_is_ignored() {
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (deposit(2, 2, "5"), Applied),
            (dispute(2, 1), Ignored(ClientMismatch)),
        ]);
        assert_eq!(
            rows(&engine),
            [
                "1,10.0000,0.0000,10.0000,false",
                "2,5.0000,0.0000,5.0000,false"
            ]
        );
    }

    #[test]
    fn refused_deposit_is_not_stored_so_cannot_be_disputed() {
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (dispute(1, 1), Applied),
            (chargeback(1, 1), Applied),
            (deposit(1, 2, "5"), Refused(AccountLocked)),
            (dispute(1, 2), Ignored(NotStored)),
        ]);
        assert_eq!(rows(&engine), ["1,0.0000,0.0000,0.0000,true"]);
    }

    // What each row does to a stored deposit, one test for each state and row.

    #[test]
    fn dispute_on_normal_deposit_holds_the_amount() {
        let mut engine = deposit_in_state(DisputeState::Normal);
        assert_eq!(engine.process(dispute(1, 1)), Applied);
        assert_eq!(rows(&engine), ["1,0.0000,10.0000,10.0000,false"]);
        assert_eq!(state_of(&engine, 1), DisputeState::Disputed);
    }

    #[test]
    fn resolve_on_normal_deposit_is_ignored() {
        let mut engine = deposit_in_state(DisputeState::Normal);
        assert_eq!(engine.process(resolve(1, 1)), Ignored(NotDisputed));
        assert_eq!(rows(&engine), ["1,10.0000,0.0000,10.0000,false"]);
        assert_eq!(state_of(&engine, 1), DisputeState::Normal);
    }

    #[test]
    fn chargeback_on_normal_deposit_is_ignored() {
        let mut engine = deposit_in_state(DisputeState::Normal);
        assert_eq!(engine.process(chargeback(1, 1)), Ignored(NotDisputed));
        assert_eq!(rows(&engine), ["1,10.0000,0.0000,10.0000,false"]);
        assert_eq!(state_of(&engine, 1), DisputeState::Normal);
    }

    #[test]
    fn dispute_on_disputed_deposit_is_ignored() {
        let mut engine = deposit_in_state(DisputeState::Disputed);
        assert_eq!(engine.process(dispute(1, 1)), Ignored(AlreadyDisputed));
        assert_eq!(rows(&engine), ["1,0.0000,10.0000,10.0000,false"]);
        assert_eq!(state_of(&engine, 1), DisputeState::Disputed);
    }

    #[test]
    fn resolve_on_disputed_deposit_releases_the_amount() {
        let mut engine = deposit_in_state(DisputeState::Disputed);
        assert_eq!(engine.process(resolve(1, 1)), Applied);
        assert_eq!(rows(&engine), ["1,10.0000,0.0000,10.0000,false"]);
        assert_eq!(state_of(&engine, 1), DisputeState::Normal);
    }

    #[test]
    fn chargeback_on_disputed_deposit_reverses_it_and_locks_the_account() {
        let mut engine = deposit_in_state(DisputeState::Disputed);
        assert_eq!(engine.process(chargeback(1, 1)), Applied);
        assert_eq!(rows(&engine), ["1,0.0000,0.0000,0.0000,true"]);
        assert_eq!(state_of(&engine, 1), DisputeState::ChargedBack);
    }

    #[test]
    fn dispute_on_charged_back_deposit_is_ignored() {
        let mut engine = deposit_in_state(DisputeState::ChargedBack);
        assert_eq!(engine.process(dispute(1, 1)), Ignored(AlreadyChargedBack));
        assert_eq!(rows(&engine), ["1,0.0000,0.0000,0.0000,true"]);
        assert_eq!(state_of(&engine, 1), DisputeState::ChargedBack);
    }

    #[test]
    fn resolve_on_charged_back_deposit_is_ignored() {
        let mut engine = deposit_in_state(DisputeState::ChargedBack);
        assert_eq!(engine.process(resolve(1, 1)), Ignored(AlreadyChargedBack));
        assert_eq!(rows(&engine), ["1,0.0000,0.0000,0.0000,true"]);
        assert_eq!(state_of(&engine, 1), DisputeState::ChargedBack);
    }

    #[test]
    fn chargeback_on_charged_back_deposit_is_ignored() {
        let mut engine = deposit_in_state(DisputeState::ChargedBack);
        assert_eq!(
            engine.process(chargeback(1, 1)),
            Ignored(AlreadyChargedBack)
        );
        assert_eq!(rows(&engine), ["1,0.0000,0.0000,0.0000,true"]);
        assert_eq!(state_of(&engine, 1), DisputeState::ChargedBack);
    }

    // Locked accounts.

    #[test]
    fn locked_account_refuses_deposit_and_withdrawal() {
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (deposit(1, 2, "5"), Applied),
            (dispute(1, 1), Applied),
            (chargeback(1, 1), Applied),
            (deposit(1, 3, "7"), Refused(AccountLocked)),
            (withdrawal(1, 4, "2"), Refused(AccountLocked)),
        ]);
        assert_eq!(rows(&engine), ["1,5.0000,0.0000,5.0000,true"]);
    }

    #[test]
    fn locked_account_still_processes_resolve() {
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (deposit(1, 2, "5"), Applied),
            (dispute(1, 1), Applied),
            (dispute(1, 2), Applied),
            (chargeback(1, 1), Applied),
            (resolve(1, 2), Applied),
        ]);
        assert_eq!(rows(&engine), ["1,5.0000,0.0000,5.0000,true"]);
    }

    #[test]
    fn locked_account_still_processes_dispute() {
        // Deposit 2 is disputed only after the chargeback has locked the account.
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (deposit(1, 2, "5"), Applied),
            (dispute(1, 1), Applied),
            (chargeback(1, 1), Applied),
            (dispute(1, 2), Applied),
        ]);
        assert_eq!(rows(&engine), ["1,0.0000,5.0000,5.0000,true"]);
    }

    #[test]
    fn locked_account_still_processes_chargeback() {
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (deposit(1, 2, "5"), Applied),
            (dispute(1, 1), Applied),
            (dispute(1, 2), Applied),
            (chargeback(1, 1), Applied),
            (chargeback(1, 2), Applied),
        ]);
        assert_eq!(rows(&engine), ["1,0.0000,0.0000,0.0000,true"]);
    }

    // Repeated ids.

    #[test]
    fn repeated_deposit_id_is_refused_and_original_is_untouched() {
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (deposit(1, 1, "10"), Refused(RepeatedId)),
        ]);
        assert_eq!(rows(&engine), ["1,10.0000,0.0000,10.0000,false"]);
    }

    #[test]
    fn withdrawal_with_a_stored_deposit_id_is_refused() {
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (withdrawal(1, 1, "3"), Refused(RepeatedId)),
        ]);
        assert_eq!(rows(&engine), ["1,10.0000,0.0000,10.0000,false"]);
    }

    #[test]
    fn repeated_id_from_another_client_is_refused() {
        // Ids are unique across clients. Client 2 still gets a record, and the
        // stored deposit is still client 1's deposit of 10.
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (deposit(2, 1, "4"), Refused(RepeatedId)),
            (dispute(2, 1), Ignored(ClientMismatch)),
            (dispute(1, 1), Applied),
        ]);
        assert_eq!(
            rows(&engine),
            [
                "1,0.0000,10.0000,10.0000,false",
                "2,0.0000,0.0000,0.0000,false"
            ]
        );
    }

    #[test]
    fn withdrawal_id_is_not_remembered() {
        // Only deposits are stored, so the id of a withdrawal can be used
        // again, by a withdrawal or by a deposit.
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (withdrawal(1, 2, "3"), Applied),
            (withdrawal(1, 2, "3"), Applied),
            (deposit(1, 2, "1"), Applied),
        ]);
        assert_eq!(rows(&engine), ["1,5.0000,0.0000,5.0000,false"]);
    }

    #[test]
    fn refused_deposit_id_is_not_remembered() {
        // Deposit 3 is refused because client 1 is locked. Its id was not
        // stored, so client 2 can use it.
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (dispute(1, 1), Applied),
            (chargeback(1, 1), Applied),
            (deposit(1, 3, "7"), Refused(AccountLocked)),
            (deposit(2, 3, "7"), Applied),
        ]);
        assert_eq!(
            rows(&engine),
            [
                "1,0.0000,0.0000,0.0000,true",
                "2,7.0000,0.0000,7.0000,false"
            ]
        );
    }

    // Client records.

    #[test]
    fn refused_withdrawal_still_creates_the_client() {
        let engine = run(&[(withdrawal(5, 1, "10"), Refused(InsufficientFunds))]);
        assert_eq!(rows(&engine), ["5,0.0000,0.0000,0.0000,false"]);
    }

    #[test]
    fn ignored_dispute_creates_no_client() {
        let engine = run(&[(dispute(7, 99), Ignored(NotStored))]);
        assert_eq!(rows(&engine), NO_ROWS);
    }

    #[test]
    fn ignored_resolve_and_chargeback_create_no_client() {
        let engine = run(&[
            (resolve(7, 99), Ignored(NotStored)),
            (chargeback(7, 99), Ignored(NotStored)),
        ]);
        assert_eq!(rows(&engine), NO_ROWS);
    }

    #[test]
    fn dispute_naming_an_unknown_client_creates_no_client() {
        let engine = run(&[
            (deposit(1, 1, "10"), Applied),
            (dispute(3, 1), Ignored(ClientMismatch)),
        ]);
        assert_eq!(rows(&engine), ["1,10.0000,0.0000,10.0000,false"]);
    }

    // Balances at the ends of the range.

    #[test]
    fn deposit_that_would_not_fit_is_refused() {
        let engine = run(&[
            (deposit(1, 1, HUGE), Applied),
            (deposit(1, 2, HUGE), Refused(RefusalReason::Overflow)),
        ]);
        assert_eq!(
            rows(&engine),
            ["1,900000000000000.0000,0.0000,900000000000000.0000,false"]
        );
    }

    #[test]
    fn deposit_is_refused_when_available_plus_held_would_not_fit() {
        // The new available balance would fit on its own. The total would not.
        let engine = run(&[
            (deposit(1, 1, HUGE), Applied),
            (dispute(1, 1), Applied),
            (deposit(1, 2, HUGE), Refused(RefusalReason::Overflow)),
        ]);
        assert_eq!(
            rows(&engine),
            ["1,0.0000,900000000000000.0000,900000000000000.0000,false"]
        );
    }

    #[test]
    fn dispute_is_ignored_when_held_would_not_fit() {
        // Available could drop by the amount, but held cannot grow by it.
        let engine = run(&[
            (deposit(1, 1, HUGE), Applied),
            (withdrawal(1, 2, HUGE), Applied),
            (deposit(1, 3, HUGE), Applied),
            (withdrawal(1, 4, HUGE), Applied),
            (deposit(1, 5, HUGE), Applied),
            (dispute(1, 1), Applied),
            (dispute(1, 3), Ignored(IgnoreReason::Overflow)),
        ]);
        assert_eq!(
            rows(&engine),
            ["1,0.0000,900000000000000.0000,900000000000000.0000,false"]
        );
        assert_eq!(state_of(&engine, 3), DisputeState::Normal);
    }

    #[test]
    fn dispute_is_ignored_when_available_would_not_fit() {
        // Held could grow by the amount, but available cannot drop by it.
        let engine = run(&[
            (deposit(1, 1, HUGE), Applied),
            (withdrawal(1, 2, HUGE), Applied),
            (deposit(1, 3, "100000000000000"), Applied),
            (withdrawal(1, 4, "100000000000000"), Applied),
            (dispute(1, 1), Applied),
            (chargeback(1, 1), Applied),
            (dispute(1, 3), Ignored(IgnoreReason::Overflow)),
        ]);
        assert_eq!(
            rows(&engine),
            ["1,-900000000000000.0000,0.0000,-900000000000000.0000,true"]
        );
        assert_eq!(state_of(&engine, 3), DisputeState::Normal);
    }

    #[test]
    fn withdrawal_from_far_below_zero_is_refused() {
        // Available is so far below zero that the remainder would not fit the
        // amount type. That still means the funds are insufficient.
        let engine = run(&[
            (deposit(1, 1, HUGE), Applied),
            (withdrawal(1, 2, HUGE), Applied),
            (dispute(1, 1), Applied),
            (withdrawal(1, 3, HUGE), Refused(InsufficientFunds)),
        ]);
        assert_eq!(
            rows(&engine),
            ["1,-900000000000000.0000,900000000000000.0000,0.0000,false"]
        );
    }

    // Memory.

    #[test]
    fn stored_deposit_entry_is_24_bytes() {
        // What one stored deposit costs in the map, id included. Memory use on
        // a large input is this figure times the number of deposits.
        assert_eq!(std::mem::size_of::<(TxId, StoredDeposit)>(), 24);
    }

    // Properties. A dispute only does something when it names a stored deposit
    // and that deposit's client, which random ids and clients would almost
    // never do. So deposits use a handful of ids, each id usually comes with
    // the same client, and the rows that refer to deposits draw from the same
    // ids. Amounts come from the bands where the arithmetic can go wrong.

    /// The amount of `units` ten-thousandths. It is written out and read back,
    /// because an amount cannot be made from a number.
    fn amount_of(units: i64) -> Amount {
        amount(&format!("{}.{:04}", units / 10_000, units % 10_000))
    }

    /// Amounts above zero: everyday ones, ones near half the largest amount
    /// (two fit an account, three do not), and ones at the top of the range.
    fn positive_amount() -> impl Strategy<Value = Amount> {
        let half = i64::MAX / 2;
        prop_oneof![
            4 => 1..=20_000_i64,
            2 => half - 10_000..=half + 10_000,
            1 => i64::MAX - 20_000..=i64::MAX,
        ]
        .prop_map(amount_of)
    }

    /// Any amount a transaction can carry, including the ones that only
    /// arithmetic produces: zero, and amounts below zero.
    fn any_amount() -> impl Strategy<Value = Amount> {
        prop_oneof![
            8 => positive_amount(),
            1 => Just(Amount::ZERO),
            1 => positive_amount().prop_map(|amount| {
                Amount::ZERO
                    .checked_sub(amount)
                    .expect("the negative of an amount fits")
            }),
        ]
    }

    /// A client and an id, for a deposit or for a row that refers to one.
    /// There are eight ids, so later rows find stored deposits and ids repeat.
    /// Odd ids usually come with client 1 and even ids with client 2, so most
    /// rows name the client that owns the deposit and one in seven names the
    /// other client.
    fn deposit_key() -> impl Strategy<Value = (ClientId, TxId)> {
        (1..=8_u32, 1..=7_u8).prop_map(|(tx, roll)| {
            let owner = if tx % 2 == 1 { 1 } else { 2 };
            let client = if roll == 7 { 3 - owner } else { owner };
            (ClientId(client), TxId(tx))
        })
    }

    /// A client and an id for a withdrawal. One id in five is also used by
    /// deposits. The rest are free, as the id of a withdrawal should be.
    fn withdrawal_key() -> impl Strategy<Value = (ClientId, TxId)> {
        (1..=2_u16, 1..=40_u32).prop_map(|(client, tx)| (ClientId(client), TxId(tx)))
    }

    fn any_transaction() -> impl Strategy<Value = Transaction> {
        prop_oneof![
            4 => (deposit_key(), any_amount())
                .prop_map(|((client, tx), amount)| Transaction::Deposit { client, tx, amount }),
            2 => (withdrawal_key(), any_amount())
                .prop_map(|((client, tx), amount)| Transaction::Withdrawal { client, tx, amount }),
            4 => deposit_key().prop_map(|(client, tx)| Transaction::Dispute { client, tx }),
            3 => deposit_key().prop_map(|(client, tx)| Transaction::Resolve { client, tx }),
            1 => deposit_key().prop_map(|(client, tx)| Transaction::Chargeback { client, tx }),
        ]
    }

    fn any_sequence() -> impl Strategy<Value = Vec<Transaction>> {
        prop::collection::vec(any_transaction(), 0..=80)
    }

    /// The client that a transaction names.
    fn client_of(transaction: Transaction) -> ClientId {
        match transaction {
            Transaction::Deposit { client, .. }
            | Transaction::Withdrawal { client, .. }
            | Transaction::Dispute { client, .. }
            | Transaction::Resolve { client, .. }
            | Transaction::Chargeback { client, .. } => client,
        }
    }

    proptest! {
        #[test]
        fn prop_held_is_never_negative(transactions in any_sequence()) {
            let mut engine = Engine::new();
            for transaction in transactions {
                let _ = engine.process(transaction);
                for (_, account) in engine.accounts() {
                    prop_assert!(account.held() >= Amount::ZERO);
                }
            }
        }

        #[test]
        fn prop_available_plus_held_always_fits(transactions in any_sequence()) {
            let mut engine = Engine::new();
            for transaction in transactions {
                let _ = engine.process(transaction);
                for (_, account) in engine.accounts() {
                    // Added here and not through `total`, so that a sum that
                    // does not fit fails this assertion with the account shown.
                    let sum = account.available().checked_add(account.held());
                    prop_assert!(sum.is_some(), "{:?}", account);
                }
            }
        }

        #[test]
        fn prop_no_sequence_of_transactions_panics(transactions in any_sequence()) {
            let mut engine = Engine::new();
            for transaction in transactions {
                // Returning at all is the property. Every total is read as
                // well, because `total` is the one function that could panic.
                let _ = engine.process(transaction);
                for (_, account) in engine.accounts() {
                    let _ = account.total();
                }
            }
        }

        #[test]
        fn prop_transaction_that_is_not_applied_changes_nothing(
            transactions in any_sequence(),
        ) {
            let mut engine = Engine::new();
            for transaction in transactions {
                let mut accounts = engine.accounts.clone();
                let deposits = engine.deposits.clone();
                match engine.process(transaction) {
                    Applied => continue,
                    Ignored(_) | Refused(AmountNotPositive) => {}
                    // The one thing a refused transaction may leave behind is
                    // an empty record for its client.
                    Refused(_) => {
                        accounts.entry(client_of(transaction)).or_insert(Account::NEW);
                    }
                }
                prop_assert_eq!(&engine.accounts, &accounts);
                prop_assert_eq!(&engine.deposits, &deposits);
            }
        }

        // The facts behind the branches that are marked as unable to fail.
        #[test]
        fn prop_held_equals_the_sum_of_disputed_deposits(transactions in any_sequence()) {
            let mut engine = Engine::new();
            for transaction in transactions {
                let outcome = engine.process(transaction);

                // A resolve or a chargeback never fails for lack of room.
                let ends_a_dispute = matches!(
                    transaction,
                    Transaction::Resolve { .. } | Transaction::Chargeback { .. }
                );
                prop_assert!(!ends_a_dispute || outcome != Ignored(IgnoreReason::Overflow));

                // The client of every stored deposit has an account.
                for deposit in engine.deposits.values() {
                    prop_assert!(engine.accounts.contains_key(&deposit.client));
                }

                for (client, account) in engine.accounts() {
                    let disputed = engine
                        .deposits
                        .values()
                        .filter(|deposit| deposit.client == client)
                        .filter(|deposit| deposit.state == DisputeState::Disputed)
                        .try_fold(Amount::ZERO, |sum, deposit| sum.checked_add(deposit.amount));
                    prop_assert_eq!(Some(account.held()), disputed);
                }
            }
        }
    }
}
