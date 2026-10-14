use comptime_runtime::{Budget, Integer};
fn main() {
    let mut budget = Budget::default();
    let high = Integer::from_u128(0xaf).to_bitvec(8, &mut budget).unwrap();
    let low = Integer::from_u128(0x3c).to_bitvec(8, &mut budget).unwrap();
    let joined = high.concat(&low, &mut budget).unwrap();
    let middle = joined.slice(4, 12, &mut budget).unwrap();
    println!(
        "(bitvec-example (concatenated {joined}) (slice {middle}) (inverted {}))",
        middle.not(&mut budget).unwrap()
    );
}
