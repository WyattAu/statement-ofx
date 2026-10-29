//! Property tests for the OFX parser boundary: arbitrary input is `Ok` or a
//! shaped `Err`, never a panic. Same rule as the CAMT side — a bank
//! statement parser is fed the internet by definition.

#![allow(clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use statement_ofx::parse_ofx_with_scale;

proptest! {
    /// Arbitrary text never panics.
    #[test]
    fn arbitrary_text_never_panics(input in ".*") {
        let _ = parse_ofx_with_scale(&input, 2);
    }

    /// A seeded `<STMTTRN>` block with hostile field values exercises the
    /// record scanner against tag soup rather than only clean documents.
    #[test]
    fn hostile_record_fields_never_panics(
        fitid in "[<>]{0,10}[A-Za-z0-9]{0,10}",
        amount in "[0-9.+-]{0,12}",
        posted in "[0-9]{0,16}",
        cur in "[A-Za-z]{0,3}",
    ) {
        let doc = format!(
            "<OFX><STMTRS><CURDEF>{cur}</CURDEF>\
             <STMTTRN><FITID>{fitid}</FITID><DTPOSTED>{posted}</DTPOSTED>\
             <TRNAMT>{amount}</TRNAMT></STMTTRN>\
             <BALAMT>{amount}</BALAMT></STMTRS></OFX>"
        );
        let _ = parse_ofx_with_scale(&doc, 2);
    }

    /// The scale parameter is honoured at the boundary: whatever the scale,
    /// an amount with more fraction digits than it allows is an error, not
    /// a truncation.
    #[test]
    fn scale_never_silently_truncates(
        whole in 0u32..1000u32,
        fraction in "[0-9]{3,6}",
        scale in 0u32..3u32,
    ) {
        let doc = format!(
            "<OFX><STMTRS><CURDEF>USD</CURDEF>\
             <STMTTRN><FITID>X</FITID><DTPOSTED>20260101</DTPOSTED>\
             <TRNAMT>{whole}.{fraction}</TRNAMT></STMTTRN>\
             <BALAMT>0</BALAMT></STMTRS></OFX>"
        );
        let result = parse_ofx_with_scale(&doc, scale);
        // Fraction digits beyond the scale are refused — never truncated.
        if fraction.len() as u32 > scale {
            prop_assert!(result.is_err());
        }
    }
}
