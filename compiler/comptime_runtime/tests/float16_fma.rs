use comptime_runtime::{Budget, float16::Float16Format as F};

#[test]
fn fused_multiply_add_matches_exact_fraction_vectors() {
    for line in include_str!("data/float16_fma.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let mut fields = line.split_whitespace();
        let format = match fields.next().unwrap() {
            "f16" => F::Binary16,
            "bf16" => F::BFloat16,
            _ => panic!("unknown format"),
        };
        let values = fields
            .map(|s| u16::from_str_radix(s, 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            format
                .fused_multiply_add(values[0], values[1], values[2], &mut Budget::default())
                .unwrap(),
            values[3],
            "{line}"
        );
    }
}

#[test]
fn fused_multiply_add_preserves_budget_failure_and_recovery() {
    let format = F::BFloat16;
    assert!(
        format
            .fused_multiply_add(
                0x7f7f,
                0x7f7f,
                1,
                &mut Budget {
                    steps: 0,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    assert_eq!(
        format
            .fused_multiply_add(0x3f80, 0x3f80, 0x3f80, &mut Budget::default())
            .unwrap(),
        0x4000
    );
}
