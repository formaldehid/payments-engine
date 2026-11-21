# payments-engine

A command-line tool. It reads a CSV file of account transactions, applies them
in the order of the file, and prints the resulting client accounts as CSV. It
handles all five transaction types: deposit, withdrawal, dispute, resolve and
chargeback.

## Build and run

```sh
cargo build
cargo run -- input.csv > output.csv
cargo test
```

The path of the input file is the first and only argument. Rust 1.73 or newer
is needed.

One of the sample files in this repository, as an example:

```sh
cargo run -- tests/fixtures/chargeback_locks_account.csv
```

It prints the accounts to stdout:

```text
client,available,held,total,locked
30,25.5000,0.0000,25.5000,true
31,12.5000,0.0000,12.5000,false
```

and two counts to stderr, below cargo's own progress lines:

```text
refused transactions: 2
ignored rows: 3
```

- **Stdout** holds the accounts and nothing else: the header line, then one
  row for each client, in ascending order of client id.
- **Stderr** holds one line for each count that is above zero. There are three
  counts, and none of them stops the run:
  - *malformed rows*: rows that cannot be read as a transaction. They are
    skipped.
  - *refused transactions*: well-formed deposits and withdrawals that the
    rules decline, for example a withdrawal of more than the available funds.
  - *ignored rows*: disputes, resolves and chargebacks that have no effect,
    for example a dispute that names an unknown deposit.
- **The exit status** is 0 when the input was read to its end and the accounts
  were printed, whatever the counts are. Otherwise it is 1, and stderr holds
  one line that begins with `error:`. Stdout is then empty, unless writing to
  stdout is what failed.

## Assumptions

The task description leaves a number of cases open. This section says how I
decided each one, and why. Where I was in doubt I chose what a bank would do:
decline rather than guess, and never change an amount without saying so.

### Deposits and withdrawals

- **A client appears in the output once a well-formed deposit or withdrawal
  names it, even if that transaction is refused.** A withdrawal by a client
  never seen before gives that client a row of zeros. An ignored dispute,
  resolve or chargeback creates no client, and neither does a malformed row.
  *Why:* the task description says to create a record when a transaction
  names a new client. I read that as a deposit or a withdrawal. It also says
  that a dispute, resolve or chargeback which cannot be applied may be
  ignored, and an ignored row should leave nothing behind.
- **Only an applied deposit is stored. A refused transaction is not, so it
  cannot be disputed later.**
  *Why:* it never took effect, so there is nothing to reverse.
- **A transaction id that belongs to a stored deposit is not accepted again.**
  A deposit or withdrawal that reuses it is refused, and the original is
  untouched. The id of a withdrawal, or of a refused or malformed row, is not
  remembered, so a repeat of it is not detected. The outcome therefore
  depends on the order of the rows: of two deposits with one id, the first is
  the one that counts.
  *Why:* the task description says that ids are unique, and remembering every
  id would cost memory for every row.

### Disputes, resolves and chargebacks

- **Only deposits can be disputed.** A dispute, resolve or chargeback that
  names a withdrawal is ignored.
  *Why:* a dispute holds funds and a chargeback takes them out, which
  reverses a deposit. Applied to a withdrawal, the same steps would take the
  money from the client a second time.
- **A dispute holds the whole amount of the deposit, even if part of it has
  been withdrawn.** The available balance can therefore go below zero. Held
  never does. After a deposit of 80, a withdrawal of 65.5 and a dispute on
  the deposit, the account reads `-65.5000,80.0000,14.5000`.
  *Why:* someone can deposit, withdraw the money and then have the deposit
  reversed. Holding only what is left would hide that. A negative balance
  shows what the client owes.
- **The row must name the client that made the deposit.** Otherwise it is
  ignored.
  *Why:* a row that names another client contradicts itself, and one client's
  row must not move another client's funds.
- **What a row does depends on the state of the deposit it names:**

  | The deposit is | dispute | resolve | chargeback |
  |---|---|---|---|
  | not under dispute | holds the amount | ignored | ignored |
  | under dispute | ignored | releases the amount | reverses the deposit and locks the account |
  | charged back | ignored | ignored | ignored |

  So a resolved deposit can be disputed again, a second dispute on a deposit
  that is already under dispute is ignored, and a chargeback is final.
  *Why:* a resolve puts the deposit back where it was, so nothing stands in
  the way of a new dispute. Holding one deposit twice would hold more than was
  deposited. After a chargeback the money has left the account.

### Locked accounts

- **A locked account refuses every deposit and withdrawal.** Nothing unlocks
  it.
  *Why:* this is what a bank means by blocking an account: the client moves
  nothing in or out.
- **Disputes, resolves and chargebacks on its deposits are still processed.**
  *Why:* they concern deposits made before the lock. A dispute that is open
  still has to end one way or the other, and a lock should not stop a
  reversal.

### Amounts

- **An amount is a signed 64-bit count of ten-thousandths.** There is no
  floating point anywhere. Every value from -922,337,203,685,477.5808 to
  922,337,203,685,477.5807 can be held, and the largest amount a row can
  carry is 922337203685477.5807.
  *Why:* it is exact by construction, and four decimals are all that is asked
  for.
- **All arithmetic is checked.** A deposit after which the account's total
  would not fit that range is refused. A dispute whose result would not fit
  is ignored. An amount too large for the range is a malformed row.
  *Why:* a balance must never wrap around, and no row may stop the program.
- **Every amount is printed with exactly four decimals.** 1.5 is printed as
  `1.5000`.
  *Why:* with always four digits, two outputs can be compared as text.

### Rows

- **Whitespace around a field is not part of the field.**
- **A deposit or withdrawal row has four fields. A dispute, resolve or
  chargeback row has three or four, and its fourth field is never read.** So
  `dispute,1,5` and `dispute,1,5,` and `dispute,1,5,3.0` are the same row.
  *Why:* these rows name a deposit by its id and carry no amount of their own.
  Files write the empty amount with or without the last comma, and whatever
  stands there has no meaning.
- **A row with more than four fields is malformed.**
  *Why:* in `deposit,1,7,1,000.50` the amount was written with a thousands
  separator. Dropping the fifth field would deposit 1 instead of 1000.50.
- **The type is `deposit`, `withdrawal`, `dispute`, `resolve` or
  `chargeback`, in lower case.** `Deposit` is malformed.
  *Why:* accepting other spellings would mean guessing what a row meant.
- **A client or transaction id is plain digits**: one or more of `0` to `9`
  and nothing else, with a value of at most 65535 for a client and 4294967295
  for a transaction. `+1`, `1.0` and `0x10` are malformed. Leading zeros are
  accepted, so `007` is client 7.
  *Why:* it is the rule for amounts, applied to ids. Rust's own integer
  parsing would accept `+1`.
- **An amount is plain digits with an optional fraction**: one or more digits,
  then optionally a point and one to four digits. `1e2`, `+1.5`, `-1`, `.5`,
  `5.` and `1_000` are malformed.
  *Why:* one exact spelling. Nothing is accepted only because a number parser
  would take it.
- **An amount with more than four decimals is malformed.** It is not rounded
  or cut off. The digits are counted as written, so `1.00000` is malformed
  too.
  *Why:* rounding would change an amount of money without anyone being told.
- **An amount must be greater than zero.**
  *Why:* a deposit of zero moves nothing, and a negative amount would turn a
  deposit into a withdrawal.
- **A row that is not valid UTF-8 is malformed**, whichever field the bytes
  are in.
  *Why:* the input is text, and a row is read as text before any of its
  fields is looked at.
- **A malformed row is skipped and counted, and the run goes on.**
  *Why:* one bad row should not throw away the good rows around it. The count
  on stderr keeps it from passing unnoticed.

### The input file

- **The first line must be the header line**: the names `type,client,tx,amount`
  in this order and in lower case. Spaces around the names are accepted, and
  empty lines before it are skipped. If anything else stands there, the
  program prints nothing to stdout and exits with status 1.
  *Why:* a file that starts differently is probably not the file that was
  meant, and accounts computed from it would pass for a result.
- **An input with no line at all is not an error.** This is the one exception
  to the rule above. For zero bytes, or nothing but empty lines, the program
  prints the output header alone, exits with status 0 and writes a note to
  stderr. A header line without rows gives the same output without the note.
  *Why:* no transactions is a valid result, and I did not want to fail on
  harmless input.
- **Quoted fields are read as standard CSV.** `"deposit","7","31","2.5"` is
  the same row as `deposit,7,31,2.5`. A quote has this meaning only at the
  very start of a field.
  *Why:* it is standard CSV and the default of the csv crate. With quoting
  switched off, a correctly quoted file would be rejected.
  *The limit that comes with it:* a quote that is opened and never closed runs
  to the end of the input. Every line after it becomes part of that one field,
  and the field is held in memory whole. If it is the fourth field of a
  dispute, resolve or chargeback row, which is never read, the lines inside it
  are lost without being counted. In any other field the row is one malformed
  row. Text after a closing quote is added to the field, so `"1"5` reads as
  15.
- **Also accepted:** a UTF-8 byte-order mark, CRLF line ends, empty lines
  between rows, and a last line without a newline.

### Exit status and stderr

- **The exit status is 0 whenever the accounts were printed**, also when rows
  were malformed, refused or ignored.
  *Why:* a failed withdrawal or a bad dispute is an expected event. A
  non-zero status could make a caller throw away correct output.
- **If reading fails part-way, nothing is printed to stdout.**
  *Why:* stdout is usually redirected into a file. Half a table there would
  look like a finished one. An empty file cannot be taken for a result.
- **Stderr gets counts at the end, not a line for each row.**
  *Why:* a line for each row has no upper bound on a large input.

## How correctness was established

**The types rule out several mistakes before any test runs.**

- `Amount` is a type of its own around the 64-bit count. An amount comes only
  from reading text, from zero, or from checked addition and subtraction.
  There are no `+` and `-` operators and no conversion from a number, so an
  unchecked sum, or an amount that passed through a float, cannot be written.
- `ClientId` and `TxId` are different types. Neither can be passed where the
  other is expected.
- A `Transaction` is an enum. A deposit or withdrawal always carries an
  amount. A dispute, resolve or chargeback cannot carry one.
- Processing a transaction returns an `Outcome`: applied, refused with a
  reason, or ignored with a reason. The compiler warns if it is dropped.
- The fields of an `Account` are private, so only the engine changes a
  balance. The total is computed from available and held and never stored,
  so the three cannot disagree.
- The dispute state of a deposit is an enum of three states, and every
  `match` on it lists all three. A new state cannot be added without deciding
  what each kind of row does in it.
- Reading a row returns a result inside a result. The outer one says that the
  input could not be read, the inner one that the row is malformed. A failed
  read is therefore never counted as a bad row.

**194 tests** pass on Rust 1.73.0 and on 1.100.0, on macOS and on Linux. The
program has not been run on Windows.

- **164 unit tests** sit beside the code they test: 160 in the library, and 4
  for the lines that the binary writes to stderr. Each is named after the
  rule it checks, for example `plain_decimal_rejects_exponent` or
  `locked_account_refuses_deposit_and_withdrawal`. The table of disputes
  above has one test for each of its nine cells.
- **13 property tests** run on 256 generated cases each:
  - Amounts (6): a positive amount survives printing and reading back;
    printing keeps every digit and exactly four decimals; reading agrees with
    the same work done in 128-bit integers, and so do checked addition and
    subtraction; only digits with at most one point are ever accepted; no
    text makes reading panic.
  - The engine (5): held is never negative; available plus held always fits
    the amount range; no sequence of transactions makes the engine panic; a
    transaction that is not applied changes no balance and no stored deposit;
    held always equals the sum of the deposits under dispute.
  - A whole run (2): no input makes a run panic, and a run that fails writes
    nothing; whitespace around fields does not change the result.

  Random data almost never forms a row that does anything. The generators
  therefore draw from a handful of ids, so that disputes find their deposits,
  and from amounts near zero and near both ends of the range, where
  arithmetic goes wrong.
- **17 end-to-end tests** in `tests/end_to_end.rs` start the built binary and
  check its exit status, stdout and stderr. They also cover a missing file, a
  directory in place of the file, a wrong number of arguments, a byte-order
  mark with CRLF line ends, and a stdout that cannot be written.
- **Sample data** is in `tests/fixtures`: 11 input files, and beside each one
  that completes, the accounts it must give. The expected accounts were worked
  out from the rules by hand, not captured from the program. The comparison
  disregards the order of the rows.

## Errors and safety

**A problem with one row never stops the run.** The row is counted as
malformed, refused or ignored, and the next row is read. Every check on a
transaction happens before any balance changes, so no row is applied in part.

**Five things end the run**, with exit status 1 and one `error:` line on
stderr:

1. The program was not given exactly one argument.
2. The file cannot be opened.
3. The first line is not the header line.
4. Reading fails, at the start or part-way. On Unix a directory given as the
   file ends here.
5. Writing the accounts to stdout fails, for example into a pipe whose reader
   has gone.

Nothing is written to stdout before the last row has been read. In cases 1 to
4 stdout is therefore empty. In case 5 part of the accounts may be out
already, and the exit status says that they are not complete.

**No input can make this code panic.** That is enforced by the tools:

- `unsafe` is forbidden for the whole crate.
- Clippy denies, outside tests: `unwrap`, `expect`, `panic!`, `unreachable!`,
  `todo!`, indexing and slicing, integer arithmetic with plain operators, and
  `println!` and `eprintln!`. The last two panic when the write fails, so the
  program writes with `writeln!` and handles the error.
- A row is taken apart with an iterator, never by index. Amounts are added
  and subtracted only through checked operations.
- The argument is read as an OS string, so a file name that is not Unicode
  cannot cause a panic either.

**The one exception** is `Account::total()`. It adds available and held and
calls `expect` on the sum, with the lint lifted for that one function. No
input reaches the panic, because the engine keeps the sum inside the amount
range. A deposit that would push it past the top is refused, and nothing else
raises it. Held is never negative, so the sum is never below available, which
itself always fits. Property tests check this on generated sequences of
transactions. The alternative was an `Option` that every caller would have to
handle for a case that cannot occur.

**What this does not cover.** Running out of memory ends the process; the
next section says how much is needed. The code of the dependencies is not
under these lints, which is why one property test feeds a whole run with
arbitrary bytes.

Nothing here is dangerous in the usual sense: no `unsafe`, no threads, no
network, and the only file touched is the input, which is only read.

## Efficiency

**The input is streamed.** It is read in blocks, one row at a time into a
buffer that is reused, and never collected. The accounts are written through
a buffer after the last row.

**Memory grows with two things only:**

- one record for each client, of which there are at most 65,536;
- one entry for each applied deposit: its id, client, amount and dispute
  state, 24 bytes in the map. It has to be kept, because any later row may
  dispute it.

Withdrawals, disputes, resolves and chargebacks store nothing, and neither do
rows that are refused, ignored or malformed.

**`cargo run` builds with optimisation.** Plain `cargo run` uses the dev
profile, so `Cargo.toml` sets `opt-level = 3` there. Overflow checks stay on
in that profile.

**Measured** on an Apple M2 Max with 96 GiB of memory, under macOS 26.4.1,
with Rust 1.100.0. Time is wall-clock time, the median of five runs. Memory is
the peak resident set size. Both are as `/usr/bin/time -l` reports them.

| Rows | Input file | Stored deposits | `cargo run` | Binary alone | Peak memory |
|---:|---:|---:|---:|---:|---:|
| 10,000,000 | 268 MB | 4,951,918 | 2.20 s | 2.00 s | 423 MB |
| 100,000,000 | 2.78 GB | 49,501,478 | 25.57 s | 24.46 s | 3.36 GB |

That is about four million rows a second. Peak memory came to between 62 and
111 bytes for each stored deposit, over five input sizes. It is more than the
24 bytes of an entry because a hash map keeps spare slots, and while it grows
it holds its old table beside the new one.

The input comes from a generator in this repository. Its rows go round all
65,536 clients, and just under half of the rows are deposits that end up
stored. To repeat the measurement:

```sh
cargo run --example large_input -- 10000000 > target/large_input.csv
/usr/bin/time -l cargo run -- target/large_input.csv > target/accounts.csv
```

`-l` is the flag of the macOS `time`; GNU `time` takes `-v`. Time and memory
were not measured on Linux.

**The limit is memory.** Rows alone cost time, not memory. Stored deposits
cost memory, and there can be 4,294,967,296 of them, one for each possible
transaction id. At the measured 62 to 111 bytes each, that would be about 270
to 480 GB. This is an estimate from the measurements above, not a
measurement. An input of that kind needs the deposits on disk, in a key-value
store under the transaction id, or a rule that closes old deposits to
disputes. Neither is built.

## Many concurrent input streams

What already fits a server: the library's `run` function takes any reader and
any writer, so a network stream can stand where the file and stdout are. The
engine does no I/O and has no global state, so each stream can have an engine
of its own. Its memory is one row plus that stream's clients and stored
deposits.

What would have to change:

- Reads block. Thousands of streams need an asynchronous reader, or a bounded
  pool of threads, in front of the same row parser.
- Each connection needs limits that one trusted file does not: on the length
  of a row, because an unclosed quote grows one row without bound, and on the
  number of stored deposits.
- If all streams feed one ledger, each connection would parse its own rows
  and send typed transactions over a bounded channel to one task that owns
  the engine. Order is kept within a stream. Between streams it is the order
  of arrival, since order is only defined within one file. To use more cores
  the engine can be split by client id, because every rule but one concerns a
  single client. The exception is the refusal of a transaction id that is
  already stored for another client.

None of this is built or measured.

## Code layout

| File | What it holds |
|---|---|
| `src/main.rs` | The command line: the argument, the file, the exit status and the lines on stderr. |
| `src/lib.rs` | The root of the library, and the lint rules. |
| `src/amount.rs` | The amount type: reading, checked arithmetic, printing. |
| `src/engine.rs` | The rules for accounts, as pure logic without I/O. |
| `src/input.rs` | Reads CSV rows into transactions. The only module that uses the csv crate. |
| `src/output.rs` | Writes the accounts as CSV. |
| `src/run.rs` | The whole program without its command line: any reader in, any writer out. It counts the outcomes. |
| `src/test_support.rs` | Helpers shared by the unit tests. |
| `tests/end_to_end.rs`, `tests/fixtures/` | The end-to-end tests and their sample files. |
| `examples/large_input.rs` | The generator of the large input for the measurements. |

The engine knows nothing about CSV, files or stdout, and the reader knows
nothing about the rules. Each can be read, tested and replaced on its own.

## Toolchain and dependencies

- **Edition 2021, Rust 1.73 or newer.** `Cargo.toml` declares
  `rust-version = "1.73"`, which is the minimum that csv 1.4.0 declares. The
  code uses nothing newer from the language or the standard library. It is
  built and tested on Rust 1.73.0 and on 1.100.0, the current stable at the
  time of writing.
- **`Cargo.lock` is committed**, in format 3, which Rust 1.73 can read. It was
  resolved for Rust 1.73, so it pins thiserror 2.0.20 and proptest 1.7.0 and
  not their newest releases, which need a newer compiler.
- **These checks pass:** `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`,
  `cargo +1.73.0 build` and `cargo +1.73.0 test`.

| Crate | Why it is here |
|---|---|
| `csv` 1.4.0 | Splits the input into records and fields: quoting, line ends, the byte-order mark. |
| `thiserror` 2.0.20 | Derives `Display` and `Error` for the two error types. |
| `proptest` 1.7.0 | The property tests. It is a dev-dependency and not part of the program. |

The program itself compiles 12 crates: csv, thiserror and what those two pull
in.

Left out on purpose:

- **serde.** Rows are read by hand, for two reasons. The csv crate's serde
  route reads `0x10` as the id 16 and accepts `+5`, which the rule for ids
  excludes. And an amount is required on a deposit or withdrawal, yet must
  not even be read on a dispute, resolve or chargeback, which one derived row
  type does not express. The csv crate still compiles `serde_core` for its
  own use. This program never calls it.
- **A decimal crate.** The amount type is small and exact, and there is no
  float inside it.
- **A command-line framework, async and threads.** There is one argument and
  one input.

## AI use

I built this with an AI coding assistant, Claude Code. Before any code was
written, every case that the task description leaves open was settled in
writing: the assistant analysed the description and recommended, and I
decided. The work then went in five steps. For each one the assistant
proposed a plan, I approved or amended it, and only then did it write the
code and the tests. The assistant also wrote the first draft of this README.
I made every commit, and the transcripts of the sessions go with the
submission.
