//! Money amounts as exact fixed-point numbers.
//!
//! An amount is a whole number of ten-thousandths. Reading, arithmetic and
//! printing are therefore exact, and no floating point is involved anywhere.

use std::fmt;
use std::iter;
use std::str::FromStr;

use thiserror::Error;

/// Digits an amount keeps after the decimal point.
const DECIMALS: usize = 4;

/// Units in one whole: ten to the power of [`DECIMALS`].
const UNITS_PER_WHOLE: u64 = 10_000;

/// An amount of money, held as a signed 64-bit count of ten-thousandths.
///
/// - The value is exact. Equality and ordering follow the numeric value.
/// - Every value from -922337203685477.5808 to 922337203685477.5807 can be held.
/// - An amount comes only from [`Amount::ZERO`], from reading text with
///   [`str::parse`], or from the checked operations. There is no conversion
///   from an integer or a float.
///
/// There are no `+` and `-` operators, because an operator cannot report a
/// result that does not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Amount(i64);

impl Amount {
    /// The amount 0.0000, the balance a new account starts with.
    pub const ZERO: Amount = Amount(0);

    /// Returns the exact sum of the two amounts, or `None` if the sum is
    /// outside the range of the type. Never panics, wraps or saturates.
    #[must_use]
    pub fn checked_add(self, other: Amount) -> Option<Amount> {
        self.0.checked_add(other.0).map(Amount)
    }

    /// Returns `self` minus `other` exactly, or `None` if the difference is
    /// outside the range of the type. The difference may be negative, because
    /// a balance can go below zero. Never panics, wraps or saturates.
    #[must_use]
    pub fn checked_sub(self, other: Amount) -> Option<Amount> {
        self.0.checked_sub(other.0).map(Amount)
    }
}

/// Reads an amount as a deposit or withdrawal row writes it.
///
/// The text is accepted exactly when all of these hold, checked in this order:
///
/// 1. Plain decimal: it is one or more ASCII digits, optionally followed by a
///    point and one or more digits, and nothing else: no sign, exponent,
///    separator or space.
/// 2. Precision: it has at most four digits after the point.
/// 3. Range: its value is at most 922337203685477.5807.
/// 4. Positive: its value is greater than zero.
///
/// The amount returned is exactly the value written. Text that is not accepted
/// is reported under the first rule it breaks. The text is not trimmed, and no
/// text can make this panic.
impl FromStr for Amount {
    type Err = ParseAmountError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        // Plain decimal. A point, when present, needs digits on both sides.
        let (whole, fraction) = match text.split_once('.') {
            Some((whole, fraction)) if is_digits(fraction) => (whole, fraction),
            Some(_) => return Err(ParseAmountError::NotPlainDecimal),
            None => (text, ""),
        };
        if !is_digits(whole) {
            return Err(ParseAmountError::NotPlainDecimal);
        }

        // Precision. The digits are counted as written, so a fifth decimal of
        // zero is one too many as well. Every byte is an ASCII digit by now, so
        // the length in bytes is the number of digits.
        if fraction.len() > DECIMALS {
            return Err(ParseAmountError::TooManyDecimals);
        }

        // Range. Append the digits one at a time, with the fraction padded to
        // four digits so that "1.5" counts 15000. Every intermediate count is a
        // prefix of the full number and never larger than it, so a step fails
        // exactly when the full value does not fit.
        let padded_fraction = fraction.chars().chain(iter::repeat('0')).take(DECIMALS);
        let mut units: i64 = 0;
        for character in whole.chars().chain(padded_fraction) {
            // `is_digits` has passed, so this cannot fail. Returning the error
            // keeps the function free of `unwrap`.
            let digit = character
                .to_digit(10)
                .ok_or(ParseAmountError::NotPlainDecimal)?;
            units = units
                .checked_mul(10)
                .and_then(|shifted| shifted.checked_add(i64::from(digit)))
                .ok_or(ParseAmountError::OutOfRange)?;
        }

        // Positive.
        if units == 0 {
            return Err(ParseAmountError::Zero);
        }
        Ok(Amount(units))
    }
}

/// Writes the amount with exactly four decimals: an optional `-`, the whole
/// part, a point and four digits, as in `-0.5000`.
///
/// Every value can be written, including both ends of the range, and zero is
/// written as `0.0000`. Reading the text of any amount greater than zero gives
/// that amount back.
impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        // `unsigned_abs` is defined for every count. Negating would overflow on
        // the smallest one, and dividing the signed count would drop the sign
        // of amounts between -1 and 0.
        let magnitude = self.0.unsigned_abs();
        // Unsigned division by a non-zero constant can neither overflow nor panic.
        let whole = magnitude / UNITS_PER_WHOLE;
        let fraction = magnitude % UNITS_PER_WHOLE;
        write!(f, "{sign}{whole}.{fraction:0width$}", width = DECIMALS)
    }
}

/// Why a piece of text is not an amount that a deposit or withdrawal may carry.
///
/// There is one variant for each rule of reading an amount, in the order the
/// rules are checked: plain decimal, precision, range, positive. Text that
/// breaks several rules is reported under the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ParseAmountError {
    /// The text is not one or more digits, optionally followed by a point and
    /// one or more digits.
    #[error("amount is not plain digits with an optional fraction")]
    NotPlainDecimal,
    /// The text has more than four digits after the point. It is neither
    /// rounded nor truncated.
    #[error("amount has more than four decimals")]
    TooManyDecimals,
    /// The value is larger than the amount type can represent.
    #[error("amount is larger than the amount type can represent")]
    OutOfRange,
    /// The value is zero, and an amount must be greater than zero.
    #[error("amount must be greater than zero")]
    Zero,
}

/// Whether `text` is one or more ASCII digits.
fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Reads `text` as an amount.
    fn read(text: &str) -> Result<Amount, ParseAmountError> {
        text.parse()
    }

    /// Whether `text` is one or more ASCII digits. Kept apart from the code
    /// under test, so that a property can check what that code accepted.
    fn only_digits(text: &str) -> bool {
        !text.is_empty() && text.chars().all(|character| character.is_ascii_digit())
    }

    // Reading, accepted. Each test names the rule it exercises, and each
    // expected value is the count of ten-thousandths.

    #[test]
    fn plain_decimal_accepts_whole_number() {
        assert_eq!(read("1"), Ok(Amount(10_000)));
        assert_eq!(read("10"), Ok(Amount(100_000)));
    }

    #[test]
    fn plain_decimal_accepts_one_to_four_decimals() {
        assert_eq!(read("1.5"), Ok(Amount(15_000)));
        assert_eq!(read("3.0"), Ok(Amount(30_000)));
        assert_eq!(read("1.50"), Ok(Amount(15_000)));
        assert_eq!(read("1.2345"), Ok(Amount(12_345)));
    }

    #[test]
    fn plain_decimal_accepts_leading_zeros() {
        assert_eq!(read("007"), Ok(Amount(70_000)));
        // The range is tested on the value, not on the length of the text.
        let long = format!("{}1", "0".repeat(24));
        assert_eq!(read(&long), Ok(Amount(10_000)));
    }

    #[test]
    fn precision_accepts_exactly_four_decimals() {
        assert_eq!(read("1.0000"), Ok(Amount(10_000)));
    }

    #[test]
    fn positive_accepts_smallest_amount_above_zero() {
        assert_eq!(read("0.0001"), Ok(Amount(1)));
    }

    #[test]
    fn range_accepts_large_whole_number() {
        assert_eq!(
            read("900000000000000"),
            Ok(Amount(9_000_000_000_000_000_000))
        );
    }

    #[test]
    fn range_accepts_largest_amount() {
        assert_eq!(read("922337203685477.5807"), Ok(Amount(i64::MAX)));
    }

    // Reading, rejected. Each test names the rule that rejects the text.

    #[test]
    fn plain_decimal_rejects_exponent() {
        assert_eq!(read("1e2"), Err(ParseAmountError::NotPlainDecimal));
    }

    #[test]
    fn plain_decimal_rejects_plus_sign() {
        assert_eq!(read("+1.5"), Err(ParseAmountError::NotPlainDecimal));
    }

    #[test]
    fn plain_decimal_rejects_minus_sign() {
        assert_eq!(read("-1"), Err(ParseAmountError::NotPlainDecimal));
    }

    #[test]
    fn plain_decimal_rejects_missing_whole_part() {
        assert_eq!(read(".5"), Err(ParseAmountError::NotPlainDecimal));
    }

    #[test]
    fn plain_decimal_rejects_missing_fraction_digits() {
        assert_eq!(read("5."), Err(ParseAmountError::NotPlainDecimal));
    }

    #[test]
    fn plain_decimal_rejects_underscore() {
        assert_eq!(read("1_000"), Err(ParseAmountError::NotPlainDecimal));
    }

    #[test]
    fn plain_decimal_rejects_empty_text() {
        assert_eq!(read(""), Err(ParseAmountError::NotPlainDecimal));
    }

    #[test]
    fn plain_decimal_rejects_surrounding_whitespace() {
        assert_eq!(read(" 1.5"), Err(ParseAmountError::NotPlainDecimal));
        assert_eq!(read("1.5 "), Err(ParseAmountError::NotPlainDecimal));
    }

    #[test]
    fn plain_decimal_rejects_second_point() {
        assert_eq!(read("1.2.3"), Err(ParseAmountError::NotPlainDecimal));
    }

    #[test]
    fn plain_decimal_rejects_comma() {
        assert_eq!(read("1,5"), Err(ParseAmountError::NotPlainDecimal));
    }

    #[test]
    fn plain_decimal_rejects_float_words() {
        assert_eq!(read("NaN"), Err(ParseAmountError::NotPlainDecimal));
        assert_eq!(read("inf"), Err(ParseAmountError::NotPlainDecimal));
    }

    #[test]
    fn plain_decimal_rejects_hex_prefix() {
        assert_eq!(read("0x10"), Err(ParseAmountError::NotPlainDecimal));
    }

    #[test]
    fn plain_decimal_rejects_non_ascii_digits() {
        // Full-width "12": digits in Unicode, but not ASCII digits.
        assert_eq!(
            read("\u{ff11}\u{ff12}"),
            Err(ParseAmountError::NotPlainDecimal)
        );
    }

    #[test]
    fn precision_rejects_five_decimals() {
        assert_eq!(read("1.00005"), Err(ParseAmountError::TooManyDecimals));
    }

    #[test]
    fn precision_rejects_fifth_decimal_even_when_zero() {
        assert_eq!(read("1.00000"), Err(ParseAmountError::TooManyDecimals));
    }

    #[test]
    fn precision_does_not_round_small_fraction_to_zero() {
        assert_eq!(read("0.00001"), Err(ParseAmountError::TooManyDecimals));
    }

    #[test]
    fn positive_rejects_zero() {
        assert_eq!(read("0"), Err(ParseAmountError::Zero));
    }

    #[test]
    fn positive_rejects_zero_in_any_spelling() {
        assert_eq!(read("0.0"), Err(ParseAmountError::Zero));
        assert_eq!(read("0.0000"), Err(ParseAmountError::Zero));
        assert_eq!(read("000"), Err(ParseAmountError::Zero));
    }

    #[test]
    fn range_rejects_one_past_largest_amount() {
        assert_eq!(
            read("922337203685477.5808"),
            Err(ParseAmountError::OutOfRange)
        );
    }

    #[test]
    fn range_rejects_whole_part_too_large() {
        assert_eq!(read("922337203685478"), Err(ParseAmountError::OutOfRange));
        // Fits an `i64` as a plain integer, but not as a count of ten-thousandths.
        assert_eq!(
            read("9223372036854775807"),
            Err(ParseAmountError::OutOfRange)
        );
    }

    #[test]
    fn precision_is_reported_before_range_and_positive() {
        let too_large_and_too_precise = format!("{}.00005", "9".repeat(20));
        assert_eq!(
            read(&too_large_and_too_precise),
            Err(ParseAmountError::TooManyDecimals)
        );
        assert_eq!(read("0.00000"), Err(ParseAmountError::TooManyDecimals));
    }

    // Arithmetic.

    #[test]
    fn checked_add_returns_exact_sum() {
        assert_eq!(
            Amount(15_000).checked_add(Amount(25_000)),
            Some(Amount(40_000))
        );
        assert_eq!(
            Amount(-80_000).checked_add(Amount(100_000)),
            Some(Amount(20_000))
        );
    }

    #[test]
    fn checked_add_returns_none_past_largest_amount() {
        assert_eq!(
            Amount(i64::MAX - 1).checked_add(Amount(1)),
            Some(Amount(i64::MAX))
        );
        assert_eq!(Amount(i64::MAX).checked_add(Amount(1)), None);
    }

    #[test]
    fn checked_sub_returns_exact_difference() {
        assert_eq!(
            Amount(40_000).checked_sub(Amount(15_000)),
            Some(Amount(25_000))
        );
    }

    #[test]
    fn checked_sub_goes_below_zero() {
        assert_eq!(
            Amount(20_000).checked_sub(Amount(100_000)),
            Some(Amount(-80_000))
        );
    }

    #[test]
    fn checked_sub_returns_none_past_smallest_amount() {
        assert_eq!(
            Amount(i64::MIN + 1).checked_sub(Amount(1)),
            Some(Amount(i64::MIN))
        );
        assert_eq!(Amount(i64::MIN).checked_sub(Amount(1)), None);
    }

    // Printing.

    #[test]
    fn prints_exactly_four_decimals() {
        assert_eq!(Amount(15_000).to_string(), "1.5000");
        assert_eq!(Amount(100_000).to_string(), "10.0000");
        assert_eq!(Amount(10_500).to_string(), "1.0500");
        assert_eq!(Amount(1).to_string(), "0.0001");
    }

    #[test]
    fn prints_zero_without_sign() {
        assert_eq!(Amount::ZERO.to_string(), "0.0000");
    }

    #[test]
    fn prints_negative_amount() {
        assert_eq!(Amount(-80_000).to_string(), "-8.0000");
    }

    #[test]
    fn prints_amounts_between_minus_one_and_zero() {
        assert_eq!(Amount(-5_000).to_string(), "-0.5000");
        assert_eq!(Amount(-1).to_string(), "-0.0001");
        assert_eq!(Amount(-9_999).to_string(), "-0.9999");
    }

    #[test]
    fn prints_largest_amount() {
        assert_eq!(Amount(i64::MAX).to_string(), "922337203685477.5807");
    }

    #[test]
    fn prints_smallest_amount() {
        assert_eq!(Amount(i64::MIN).to_string(), "-922337203685477.5808");
    }

    // The type itself.

    #[test]
    fn amount_is_eight_bytes() {
        assert_eq!(std::mem::size_of::<Amount>(), 8);
    }

    #[test]
    fn units_per_whole_is_ten_to_the_decimals() {
        let decimals = u32::try_from(DECIMALS).unwrap();
        assert_eq!(UNITS_PER_WHOLE, 10_u64.pow(decimals));
    }

    #[test]
    fn ordering_follows_numeric_value() {
        assert!(Amount(-1) < Amount::ZERO);
        assert!(Amount::ZERO < Amount(1));
        assert!(Amount(i64::MIN) < Amount(i64::MAX));
    }

    // Properties. Uniform draws almost never land near zero, at the ends of the
    // range or on a near miss, which is where reading and printing can go
    // wrong, so each strategy below aims part of its draws there.

    /// The whole part of the largest amount.
    const LARGEST_WHOLE: &str = "922337203685477";

    /// Counts anywhere in the range, around zero, and at both ends.
    fn any_count() -> impl Strategy<Value = i64> {
        prop_oneof![
            any::<i64>(),
            -20_000_i64..=20_000,
            i64::MIN..=i64::MIN + 20_000,
            i64::MAX - 20_000..=i64::MAX,
        ]
    }

    /// Counts greater than zero: anywhere, below two whole units, and at the top.
    fn positive_count() -> impl Strategy<Value = i64> {
        prop_oneof![1..=i64::MAX, 1..=20_000_i64, i64::MAX - 20_000..=i64::MAX]
    }

    /// A well-formed amount with one more character inserted somewhere. The
    /// result is often a near miss such as `+1.5`, `1e5` or `1.2.3`.
    fn nearly_an_amount() -> impl Strategy<Value = String> {
        (
            "[0-9]{1,6}(\\.[0-9]{1,4})?",
            "[0-9.+\\-eE_ ]",
            any::<prop::sample::Index>(),
        )
            .prop_map(|(mut text, extra, position)| {
                text.insert_str(position.index(text.len() + 1), &extra);
                text
            })
    }

    proptest! {
        #[test]
        fn prop_positive_amount_survives_printing_and_rereading(units in positive_count()) {
            let amount = Amount(units);
            prop_assert_eq!(read(&amount.to_string()), Ok(amount));
        }

        #[test]
        fn prop_printing_keeps_every_digit_and_four_decimals(units in any_count()) {
            let printed = Amount(units).to_string();
            let (whole, fraction) = printed.split_once('.').expect("a printed amount has a point");
            prop_assert_eq!(fraction.len(), DECIMALS);
            prop_assert!(only_digits(fraction));
            // A minus sign exactly when the count is negative, and no padding.
            let whole_digits = whole.strip_prefix('-').unwrap_or(whole);
            prop_assert_eq!(whole.starts_with('-'), units < 0);
            prop_assert!(only_digits(whole_digits));
            prop_assert!(whole_digits == "0" || !whole_digits.starts_with('0'));
            // Without the point, the text is the count itself.
            prop_assert_eq!(format!("{whole}{fraction}").parse::<i64>(), Ok(units));
        }

        #[test]
        fn prop_reading_matches_wide_integer_reference(
            // Any digits, a zero whole part, and the largest whole part.
            whole in prop_oneof!["[0-9]{1,18}", "0{1,3}", Just(LARGEST_WHOLE.to_string())],
            fraction in prop::option::of("[0-9]{1,4}"),
        ) {
            let text = match &fraction {
                Some(fraction) => format!("{whole}.{fraction}"),
                None => whole.clone(),
            };
            // The same value worked out in 128 bits, where nothing here overflows.
            let padded_fraction = format!("{:0<4}", fraction.unwrap_or_default());
            let units = whole.parse::<u128>().unwrap() * 10_000
                + padded_fraction.parse::<u128>().unwrap();
            let expected = match i64::try_from(units) {
                Ok(0) => Err(ParseAmountError::Zero),
                Ok(units) => Ok(Amount(units)),
                Err(_) => Err(ParseAmountError::OutOfRange),
            };
            prop_assert_eq!(read(&text), expected);
        }

        #[test]
        fn prop_accepted_text_is_digits_with_at_most_one_point(text in nearly_an_amount()) {
            if read(&text).is_ok() {
                match text.split_once('.') {
                    Some((whole, fraction)) => {
                        prop_assert!(only_digits(whole));
                        prop_assert!(only_digits(fraction));
                        prop_assert!(fraction.len() <= DECIMALS);
                    }
                    None => prop_assert!(only_digits(&text)),
                }
            }
        }

        #[test]
        fn prop_reading_arbitrary_text_never_panics(text in any::<String>()) {
            // Returning at all is the property; either result is acceptable.
            let _ = read(&text);
        }

        #[test]
        fn prop_checked_add_and_sub_match_wide_integer_reference(
            a in any_count(),
            b in any_count(),
        ) {
            let sum = i64::try_from(i128::from(a) + i128::from(b)).ok().map(Amount);
            prop_assert_eq!(Amount(a).checked_add(Amount(b)), sum);
            let difference = i64::try_from(i128::from(a) - i128::from(b)).ok().map(Amount);
            prop_assert_eq!(Amount(a).checked_sub(Amount(b)), difference);
        }
    }
}
