//! OFX bank-statement adapter: parse OFX 1.x into a [`recon_kit::Statement`].
//!
//! # The two normalisations this adapter exists to perform
//!
//! **Signs become direction.** OFX signs every amount — negative is money out
//! of the account, positive is money in. [`recon_kit`] carries an unsigned
//! amount plus a [`recon_kit::Direction`], because mixing sign conventions is
//! how a refund posts as a charge. The convention stops at this boundary and
//! never reaches the matching engine.
//!
//! **Strings become integer minor units.** OFX amounts are decimal strings
//! (`"-1200.00"`). Converting through a float corrupts them: `0.07` as f64
//! times 100 is `6.999999999999999`, and a truncated `6` or a rounded `7`
//! decides whether a statement line matches. This adapter parses the string
//! into an integer part and a fraction part and never constructs a float.
//!
//! # What OFX does not carry, and what this adapter does about it
//!
//! OFX carries the **closing** balance (`LEDGERBAL`/`BALAMT`) but not the
//! opening balance. The adapter therefore derives the opening balance as
//! closing minus the signed sum of the lines. That makes [`Statement::validate`]'s
//! balance identity trivially true for this format — it still guards line-id
//! uniqueness and zero amounts, but the truncation check this adapter cannot
//! offer is CAMT.053's, which carries both balances explicitly. That is a
//! property of the format, recorded here rather than papered over.
//!
//! # Scale
//!
//! OFX does not declare how many decimal places an amount carries. Most
//! statements use two; JPY uses zero. [`parse_ofx`] assumes **two** and
//! [`parse_ofx_with_scale`] takes the scale explicitly — an adapter that
//! guessed wrong here would move every decimal point in the file.

use recon_kit::{Direction, Statement, StatementLine};

/// Why an OFX document cannot be trusted as a statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfxError {
    /// A `<STMTTRN>` had no `<FITID>`.
    MissingFitId,
    /// A `<STMTTRN>` had no `<DTPOSTED>`.
    MissingDatePosted,
    /// A `<STMTTRN>` had no `<TRNAMT>`.
    MissingAmount,
    /// An amount was not a decimal string this adapter accepts.
    MalformedAmount {
        /// The raw string as it appeared.
        raw: String,
    },
    /// A date was not `YYYYMMDD[HHMMSS[..]]`.
    MalformedDate {
        /// The raw string as it appeared.
        raw: String,
    },
    /// No `<CURDEF>` was found.
    MissingCurrency,
    /// No `<BALAMT>` was found in a `LEDGERBAL` block.
    MissingClosingBalance,
    /// An amount string carried more fraction digits than the declared scale.
    TooManyFractionDigits {
        /// The raw string.
        raw: String,
        /// The scale in force.
        scale: u32,
    },
    /// The document contained no `<STMTTRN>` records.
    NoTransactions,
}

/// Parse an OFX 1.x banking statement at the common two-decimal scale.
pub fn parse_ofx(input: &str) -> Result<Statement, OfxError> {
    parse_ofx_with_scale(input, 2)
}

/// Parse an OFX 1.x banking statement with an explicit minor-unit scale
/// (JPY statements use `0`; the common case is `2`).
pub fn parse_ofx_with_scale(input: &str, scale: u32) -> Result<Statement, OfxError> {
    let currency = first_tag(input, "CURDEF").ok_or(OfxError::MissingCurrency)?;

    let mut lines = Vec::new();
    for block in split_tag_blocks(input, "STMTTRN") {
        let fitid = first_tag(&block, "FITID").ok_or(OfxError::MissingFitId)?;
        let posted = first_tag(&block, "DTPOSTED").ok_or(OfxError::MissingDatePosted)?;
        let raw_amount = first_tag(&block, "TRNAMT").ok_or(OfxError::MissingAmount)?;
        let signed = parse_signed_amount(&raw_amount, scale)?;
        let (direction, magnitude) = Direction::from_signed(signed);
        let description = first_tag(&block, "NAME")
            .or_else(|| first_tag(&block, "MEMO"))
            .unwrap_or_default();

        lines.push(StatementLine {
            id: fitid,
            booked_on: normalize_date(&posted)?,
            amount: magnitude,
            direction,
            description,
        });
    }
    if lines.is_empty() {
        return Err(OfxError::NoTransactions);
    }
    lines.sort_by(|a, b| a.booked_on.cmp(&b.booked_on).then(a.id.cmp(&b.id)));

    // OFX carries the closing balance only; the opening balance is derived.
    // The arithmetic is the identity check's own terms, so it cannot be wrong
    // — but a bank that also publishes an opening balance would be checked
    // against it, which is what the CAMT adapter will do.
    let closing = first_tag(input, "BALAMT")
        .ok_or(OfxError::MissingClosingBalance)
        .and_then(|raw| parse_signed_amount(&raw, scale))?;
    let sum: i64 = lines.iter().map(|l| l.direction.signed(l.amount)).sum();
    let opening = closing - sum;

    Ok(Statement {
        currency,
        opening_balance: opening,
        closing_balance: closing,
        lines,
    })
}

/// The text of the first `<tag>value` occurrence, trimmed.
///
/// OFX 1.x is SGML: tags do not close, and values run to end of line. The scan
/// matches `<tag>` case-insensitively and takes the rest of the line, which is
/// the entire grammar this adapter needs — a bank statement is not the place
/// for a permissive XML parser's silence in the face of malformity.
fn first_tag(input: &str, tag: &str) -> Option<String> {
    let needle = format!("<{tag}>");
    let lower_needle = needle.to_lowercase();
    let mut search_from = 0usize;
    while let Some(rel) = input[search_from..].to_lowercase().find(&lower_needle) {
        let start = search_from + rel + needle.len();
        let rest = &input[start..];
        let end = rest.find('\n').unwrap_or(rest.len());
        let value = rest[..end].trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
        search_from = start;
    }
    None
}

/// Split the input into the text of each `<open>…</open>` block.
fn split_tag_blocks(input: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let lower_open = open.to_lowercase();
    let lower_close = close.to_lowercase();
    let mut blocks = Vec::new();
    let mut rest = input;
    while let Some(open_pos) = rest.to_lowercase().find(&lower_open) {
        let after_open = open_pos + open.len();
        let Some(close_rel) = rest[after_open..].to_lowercase().find(&lower_close) else {
            break;
        };
        blocks.push(rest[after_open..after_open + close_rel].to_string());
        rest = &rest[after_open + close_rel + close.len()..];
    }
    blocks
}

/// Parse a signed decimal string into minor units without constructing a
/// float anywhere on the path.
///
/// `"-1200.5"` at scale 2 is `-120050`. The fraction is zero-padded to the
/// scale; more fraction digits than the scale allows is an error rather than a
/// silent truncation, because a truncated penny is a line that no longer
/// matches.
fn parse_signed_amount(raw: &str, scale: u32) -> Result<i64, OfxError> {
    let trimmed = raw.trim();
    let (sign, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1i64, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let (integer_part, fraction_part) = match digits.split_once('.') {
        Some((i, f)) => (i, f),
        None => (digits, ""),
    };
    if integer_part.is_empty() && fraction_part.is_empty() {
        return Err(OfxError::MalformedAmount {
            raw: raw.to_string(),
        });
    }
    if !integer_part.chars().all(|c| c.is_ascii_digit())
        || !fraction_part.chars().all(|c| c.is_ascii_digit())
    {
        return Err(OfxError::MalformedAmount {
            raw: raw.to_string(),
        });
    }
    if fraction_part.len() as u64 > u64::from(scale) {
        return Err(OfxError::TooManyFractionDigits {
            raw: raw.to_string(),
            scale,
        });
    }
    let integer: i64 = if integer_part.is_empty() {
        0
    } else {
        integer_part
            .parse()
            .map_err(|_| OfxError::MalformedAmount {
                raw: raw.to_string(),
            })?
    };
    let mut fraction: i64 = if fraction_part.is_empty() {
        0
    } else {
        fraction_part
            .parse()
            .map_err(|_| OfxError::MalformedAmount {
                raw: raw.to_string(),
            })?
    };
    for _ in fraction_part.len() as u64..u64::from(scale) {
        fraction *= 10;
    }
    Ok(sign * (integer * 10_i64.pow(scale) + fraction))
}

/// `YYYYMMDD[HHMMSS…]` to `YYYY-MM-DD`, refusing anything else.
fn normalize_date(raw: &str) -> Result<String, OfxError> {
    let trimmed = raw.trim();
    let Some(digits) = trimmed.get(..8) else {
        return Err(OfxError::MalformedDate {
            raw: raw.to_string(),
        });
    };
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(OfxError::MalformedDate {
            raw: raw.to_string(),
        });
    }
    let (year, rest) = digits.split_at(4);
    let (month, day) = rest.split_at(2);
    Ok(format!("{year}-{month}-{day}"))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A realistic OFX 1.x statement: SGML-style unclosed tags, header block,
    /// signed amounts, a debit with NAME and a credit with only MEMO.
    const FIXTURE: &str = r#"OFXHEADER:100
DATA:OFXSGML
VERSION:102
SECURITY:NONE
ENCODING:USASCII

<OFX>
<BANKMSGSRSV1>
<STMTTRNRS>
<TRNUID>0
<STATUS>
<CODE>0
<SEVERITY>INFO
<STMTRS>
<CURDEF>GBP
<BANKTRANLIST>
<DTSTART>20260701
<DTEND>20260731
<STMTTRN>
<TRNTYPE>DEBIT
<DTPOSTED>20260703120000
<TRNAMT>-1200.00
<FITID>FIT-100
<NAME>ACME LTD
<MEMO>INVOICE 2026-071
</STMTTRN>
<STMTTRN>
<TRNTYPE>CREDIT
<DTPOSTED>20260708
<TRNAMT>1056.00
<FITID>FIT-101
<MEMO>GLOBEX PAYMENT
</STMTTRN>
</BANKTRANLIST>
<LEDGERBAL>
<BALAMT>233600.00
<DTASOF>20260731
</LEDGERBAL>
</STMTRS>
</STMTTRNRS>
</BANKMSGSRSV1>
</OFX>
"#;

    #[test]
    fn the_fixture_parses_into_a_trustworthy_statement() {
        let statement = parse_ofx(FIXTURE).expect("the fixture is well-formed");
        assert_eq!(statement.currency, "GBP");
        assert_eq!(statement.lines.len(), 2);

        // Signs became direction; magnitudes lost their sign.
        let debit = statement.lines.first().expect("two fixture lines");
        assert_eq!(debit.id, "FIT-100");
        assert_eq!(debit.direction, Direction::Debit);
        assert_eq!(debit.amount, 120_000);
        assert_eq!(
            debit.booked_on, "2026-07-03",
            "YYYYMMDDHHMMSS truncates to date"
        );
        assert_eq!(debit.description, "ACME LTD");

        let credit = statement.lines.get(1).expect("two fixture lines");
        assert_eq!(credit.direction, Direction::Credit);
        assert_eq!(credit.amount, 105_600);
        assert_eq!(
            credit.description, "GLOBEX PAYMENT",
            "MEMO fills in where NAME is absent"
        );

        // The derived opening balance: closing minus the signed lines.
        assert_eq!(statement.closing_balance, 23_360_000);
        assert_eq!(statement.opening_balance, 23_360_000 - (105_600 - 120_000));
        // And the statement is trustworthy by the model's own rules.
        assert_eq!(statement.validate(), Ok(()));
    }

    #[test]
    fn amounts_are_parsed_without_a_float_anywhere_on_the_path() {
        // 0.07 through a float is 6.999999999999999; through this parser it is
        // exactly 7 minor units. This test is the reason the parser exists.
        assert_eq!(parse_signed_amount("0.07", 2).expect("parses"), 7);
        assert_eq!(parse_signed_amount("-0.07", 2).expect("parses"), -7);
        assert_eq!(parse_signed_amount("1200.00", 2).expect("parses"), 120_000);
        assert_eq!(parse_signed_amount("-1200.5", 2).expect("parses"), -120_050);
        assert_eq!(
            parse_signed_amount("1200", 2).expect("parses"),
            120_000,
            "no fraction means .00"
        );
        assert_eq!(
            parse_signed_amount("5", 0).expect("parses"),
            5,
            "JPY scale has no fraction"
        );
        assert_eq!(parse_signed_amount("0", 2).expect("parses"), 0);

        for garbage in ["", "-", ".", "-.", "12.3.4", "abc", "1,200.00"] {
            assert!(
                parse_signed_amount(garbage, 2).is_err(),
                "{garbage:?} must not parse"
            );
        }
        // More fraction digits than the scale is refused, not truncated: a
        // truncated penny is a line that no longer matches.
        assert!(matches!(
            parse_signed_amount("1.234", 2),
            Err(OfxError::TooManyFractionDigits { scale: 2, .. })
        ));
    }

    #[test]
    fn a_missing_fitid_is_an_error_not_a_generated_id() {
        let input = FIXTURE.replace("<FITID>FIT-100\n", "");
        assert!(matches!(parse_ofx(&input), Err(OfxError::MissingFitId)));
        // Idempotent re-import keys off fitids; inventing one would break the
        // one guarantee the format gives.
    }

    #[test]
    fn malformed_dates_and_empty_documents_are_errors() {
        let bad_date = FIXTURE.replace("<DTPOSTED>20260703120000", "<DTPOSTED>3rd July");
        assert!(matches!(
            parse_ofx(&bad_date),
            Err(OfxError::MalformedDate { .. })
        ));

        let no_transactions = "<OFX><STMTRS><CURDEF>GBP</STMTRS></OFX>";
        assert_eq!(parse_ofx(no_transactions), Err(OfxError::NoTransactions));

        let no_currency = FIXTURE.replace("<CURDEF>GBP\n", "");
        assert_eq!(parse_ofx(&no_currency), Err(OfxError::MissingCurrency));

        let no_balance = FIXTURE.replace("<BALAMT>233600.00\n", "");
        assert_eq!(parse_ofx(&no_balance), Err(OfxError::MissingClosingBalance));
    }

    #[test]
    fn jpy_statements_parse_at_scale_zero() {
        let jpy = FIXTURE
            .replace("<CURDEF>GBP", "<CURDEF>JPY")
            .replace("<TRNAMT>-1200.00", "<TRNAMT>-1200")
            .replace("<TRNAMT>1056.00", "<TRNAMT>1056")
            .replace("<BALAMT>233600.00", "<BALAMT>233600");
        let statement = parse_ofx_with_scale(&jpy, 0).expect("JPY has no minor unit");
        assert_eq!(
            statement.lines.first().expect("parses").amount,
            1200,
            "no decimal points moved"
        );
        assert_eq!(statement.validate(), Ok(()));
        // The default scale would have been wrong by two orders of magnitude.
    }

    #[test]
    fn lines_come_out_in_booking_order_even_if_the_file_is_not() {
        let swapped = FIXTURE.replace("FIT-100", "FIT-ZZZ");
        let statement = parse_ofx(&swapped).expect("parses");
        assert_eq!(
            statement.lines.first().expect("parses").booked_on,
            "2026-07-03"
        );
        assert_eq!(
            statement.lines.get(1).expect("parses").booked_on,
            "2026-07-08"
        );
    }
}
