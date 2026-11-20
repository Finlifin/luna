//! Maranget-style runtime pattern-matching decision-DAG compiler and reference evaluator.
//!
//! Implements Flurry pattern matrix compilation (`if ... is do` / `match` / `while ... is do`),
//! structural hash-consing for `Or`-patterns, guard-aware source-order fallthrough, exact arm
//! reachability, `BitVec<N>` segment alignment & mask partitioning, and counterexample witness
//! extraction for exhaustiveness diagnostics.

pub mod bitvec;
pub mod compile;
pub mod decision;
pub mod eval;
pub mod pattern;
mod sexpr;

pub use bitvec::{BitInterval, BitValue, BitVecError};
pub use compile::{
    CompileOutput, CompileStats, MissingWitness, WitnessPayload, compile, compile_arms,
    compile_with_budget,
};
pub use decision::{
    Binding, DagBuilder, DagNode, DecisionDag, LenInterval, NodeId, Place, PlaceStep, SwitchKind,
};
pub use eval::{EnumValuePayload, MatchOutcome, Value, eval_dag, eval_linear};
pub use pattern::{
    BitSegmentPattern, BitVecPattern, CharInterval, CompileBudget, CompileError, EnumType,
    ErrorRowType, ErrorTag, FiniteInt, IntInterval, IntLit, IntType, LowerPoint, MatchArm, Pattern,
    PatternMatrix, RangeBound, TypeDesc, UpperPoint, VariantPayloadPattern, VariantPayloadType,
    VariantSchema, VariantTag,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_int_extremes_and_formatting() {
        let min_s = FiniteInt::from_i128(i128::MIN);
        let below_min = min_s.sub_one();
        assert_eq!(below_min.add_one(), min_s);
        assert_eq!(
            below_min.to_string(),
            "-170141183460469231731687303715884105729"
        );

        let max_u = FiniteInt::from_u128(u128::MAX);
        let above_max = max_u.add_one();
        assert_eq!(above_max.sub_one(), max_u);
        assert_eq!(
            above_max.to_string(),
            "340282366920938463463374607431768211456"
        );
    }

    #[test]
    fn bit_value_operations_and_radix_parsing() {
        let bin = BitValue::from_radix_str(2, "1010_0110").unwrap();
        assert_eq!(bin.width(), 8);
        assert_eq!(bin.to_u128(), Some(0b1010_0110));

        let oct = BitValue::from_radix_str(8, "7_3").unwrap();
        assert_eq!(oct.width(), 6);
        assert_eq!(oct.to_u128(), Some(0o73));

        let hex = BitValue::from_radix_str(16, "AF_3C").unwrap();
        assert_eq!(hex.width(), 16);
        assert_eq!(hex.to_u128(), Some(0xAF3C));

        let hi = BitValue::from_radix_str(16, "AF").unwrap();
        let lo = BitValue::from_radix_str(16, "3C").unwrap();
        assert_eq!(hi.concat(&lo), hex);
        assert_eq!(hex.slice(8, 16), hi);
        assert_eq!(hex.slice(0, 8), lo);

        let neg = BitValue::from_i128(4, -3).unwrap();
        assert_eq!(neg.to_u128(), Some(0b1101));
        assert_eq!(neg.to_signed_i128(), Some(-3));

        let one_bit_neg = BitValue::from_i128(1, -1).unwrap();
        assert_eq!(one_bit_neg.to_signed_i128(), Some(-1));
        assert!(BitValue::from_i128(1, 1).is_err());

        let zero = BitValue::zero(4);
        assert_eq!(zero.checked_sub_one(), None);
        let ones = BitValue::all_ones(4);
        assert_eq!(ones.checked_add_one(), None);
        assert_eq!(ones.checked_sub_one().unwrap().to_u128(), Some(14));
    }

    #[test]
    fn dag_builder_hash_consing_and_dead_switch_collapse() {
        let mut builder = DagBuilder::new(CompileBudget::default());
        let leaf0 = builder.leaf(0, Vec::new()).unwrap();
        let leaf0_dup = builder.leaf(0, Vec::new()).unwrap();
        assert_eq!(leaf0, leaf0_dup);

        // Collapses redundant bool switch whose branches point to the same node.
        let collapsed = builder
            .switch(
                Place::root(),
                SwitchKind::Bool {
                    on_false: leaf0,
                    on_true: leaf0_dup,
                },
            )
            .unwrap();
        assert_eq!(collapsed, leaf0);
        assert_eq!(builder.allocated_nodes(), 1);
    }
}
