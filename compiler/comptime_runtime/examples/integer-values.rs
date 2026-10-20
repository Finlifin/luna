use comptime_runtime::{Budget, Integer};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut budget = Budget::default();
    let value = Integer::parse_literal(
        "115792089237316195423570985008687907853269984665640564039457584007913129639936",
        &mut budget,
    )?;
    let divisor = Integer::from_u128(97);
    let (quotient, remainder) = value.div_rem_trunc(&divisor, &mut budget)?;
    let reconstructed = quotient
        .multiply(&divisor, &mut budget)?
        .add(&remainder, &mut budget)?;
    assert_eq!(reconstructed, value);
    println!(
        "(comptime-integer-example (value {value}) (divisor {divisor}) (quotient {quotient}) (remainder {remainder}) (negative {}) (remaining-steps {}))",
        value.negated(),
        budget.steps
    );
    Ok(())
}
