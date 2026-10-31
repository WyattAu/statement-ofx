# statement-ofx

OFX bank-statement adapter producing [`recon-kit`](https://crates.io/crates/recon-kit)
statements. Sign-normalising, float-free amounts, hand-rolled SGML scan, zero
dependencies beyond the model.

## The two normalisations this adapter exists to perform

**Signs become direction.** OFX signs every amount; negative is money out of
the account. `recon-kit` carries an unsigned amount plus a `Direction`, because
mixing sign conventions is how a refund posts as a charge. The convention stops
at this boundary and never reaches the matching engine.

**Strings become integer minor units.** OFX amounts are decimal strings.
Converting through a float corrupts them: `0.07` as f64 times 100 is
`6.999999999999999`, and a truncated `6` versus a rounded `7` decides whether a
statement line matches. The parser splits the string into integer and fraction
parts and never constructs a float. More fraction digits than the declared
scale is an error, not a truncation; a truncated penny is a line that no
longer matches.

## What OFX does not carry

OFX publishes the **closing** balance but not the opening balance. The adapter
derives the opening balance as closing minus the signed sum of the lines, which
makes the model's balance identity trivially true for this format; it still
guards line-id uniqueness and zero amounts, but the truncated-import check that
format can offer is CAMT.053's, which carries both balances explicitly. That is
a property of the format, recorded rather than papered over.

## Scale

OFX does not declare decimal places. [`parse_ofx`] assumes **two** (the
common case); [`parse_ofx_with_scale`] takes the scale explicitly; JPY
statements use `0`, and an adapter that guessed wrong here would move every
decimal point in the file.

## Licence

MIT OR Apache-2.0.
