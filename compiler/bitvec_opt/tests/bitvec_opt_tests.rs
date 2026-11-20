use std::collections::{BTreeMap, BTreeSet};

use bitvec_opt::{
    BehaviorBoundary, BitValue, BoundaryCheck, BoundaryInput, Budget, ChosenImplementation,
    CleanupAction, CmpOp, CostModel, CounterexampleReason, DomainConstraint, DomainTerm, EvalError,
    FallbackReason, InputEffect, InputIdentity, IrError, Justification, Node, OutputKind,
    PatternKind, ProofOutcome, ProofStatus, Region, RegionBuilder, SynthesisCache, TargetInst,
    TargetOp, TargetProfile, TargetProgram, TargetReg, TraceEvent, VERIFIER_VERSION,
    eliminate_zero_width, eval_region, eval_region_value, half_open_to_inclusive_extract,
    lower_to_baseline, recognize_patterns, simplify_region, synthesize, synthesize_with_budget,
    top_word_mask, verify_region_equivalence, verify_target_program,
};

fn validate_sexpr(rendered: &str) {
    assert!(
        !rendered.contains('\n') && !rendered.contains('\r'),
        "S-expression must not contain raw newlines: {rendered}"
    );
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for ch in rendered.chars() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                assert!(depth >= 0, "unbalanced ')' in S-expression: {rendered}");
            }
            _ => {}
        }
    }
    assert!(
        !in_string,
        "unterminated string in S-expression: {rendered}"
    );
    assert_eq!(depth, 0, "unclosed '(' in S-expression: {rendered}");
    assert!(
        rendered.starts_with('(') && rendered.ends_with(')'),
        "top-level S-expression must be parenthesized: {rendered}"
    );
}

fn env_one(id: InputIdentity, val: BitValue) -> BTreeMap<InputIdentity, BitValue> {
    let mut map = BTreeMap::new();
    map.insert(id, val);
    map
}

fn env_two(
    id1: InputIdentity,
    v1: BitValue,
    id2: InputIdentity,
    v2: BitValue,
) -> BTreeMap<InputIdentity, BitValue> {
    let mut map = BTreeMap::new();
    map.insert(id1, v1);
    map.insert(id2, v2);
    map
}

#[test]
fn test_bitvec_zero_empty_algebra_and_boundaries() {
    // 1. empty ++ x -> x, x ++ empty -> x, not empty -> empty, empty == empty -> true
    let mut b1 = RegionBuilder::new();
    let x_id = InputIdentity::pure("x", 1);
    let empty_id = InputIdentity::new("side_empty", 2, 0, 0);
    let x_node = b1
        .input_with_effect(x_id.clone(), 8, InputEffect::PureValue)
        .unwrap();
    let empty_node = b1
        .input_with_effect(
            empty_id.clone(),
            0,
            InputEffect::SideEffectCall {
                symbol: String::from("produce_empty"),
                call_id: 7,
            },
        )
        .unwrap();
    let not_empty = b1.not(empty_node).unwrap();
    let left_cat = b1.concat(not_empty, x_node).unwrap();
    let right_cat = b1.concat(left_cat, empty_node).unwrap();
    let r1 = b1.build(right_cat).unwrap();

    let zw = eliminate_zero_width(&r1).unwrap();
    // Pure DAG reduces down to just `x`, while boundary STILL keeps `side_empty` call!
    assert!(
        zw.region
            .nodes
            .iter()
            .all(|n| !matches!(n, Node::Concat(_, _) | Node::Not(_))),
        "expected empty concat/not to simplify: {}",
        zw.region
    );
    assert_eq!(zw.region.boundary.inputs.len(), 2);
    assert!(zw.region.boundary.inputs.iter().any(|bi| {
        bi.identity == empty_id
            && matches!(
                &bi.effect,
                InputEffect::SideEffectCall { symbol, call_id: 7 } if symbol == "produce_empty"
            )
    }));

    let inputs = env_two(
        x_id.clone(),
        BitValue::from_u64(8, 0xAF).unwrap(),
        empty_id.clone(),
        BitValue::empty(),
    );
    let ref_eval = eval_region(&r1, &inputs).unwrap();
    let zw_eval = eval_region(&zw.region, &inputs).unwrap();
    assert_eq!(ref_eval.value, BitValue::from_u64(8, 0xAF).unwrap());
    assert_eq!(zw_eval.value, ref_eval.value);
    assert_eq!(ref_eval.trace, zw_eval.trace);
    assert!(matches!(
        &zw_eval.trace[0],
        TraceEvent::InputEvaluated {
            effect: InputEffect::SideEffectCall { symbol, call_id: 7 },
            ..
        } if symbol == "produce_empty"
    ));

    // 2. empty == empty -> true (and zero_extend(empty, 16) -> 0)
    let mut b2 = RegionBuilder::new();
    let e0 = b2.const_u64(0, 0).unwrap();
    let e1 = b2.not(e0).unwrap();
    let eq_empty = b2.equal(e0, e1).unwrap();
    let r2 = b2.build_bool(eq_empty).unwrap();
    let zw2 = eliminate_zero_width(&r2).unwrap();
    assert_eq!(
        eval_region_value(&zw2.region, &BTreeMap::new())
            .unwrap()
            .to_bool(),
        Some(true)
    );
    assert!(matches!(
        zw2.region.nodes[zw2.region.root.0],
        Node::Constant { width: 1, .. }
    ));

    let mut b3 = RegionBuilder::new();
    let e_in = b3
        .input_with_effect(
            InputIdentity::new("call0", 10, 0, 0),
            0,
            InputEffect::SideEffectCall {
                symbol: String::from("check_zero_effect"),
                call_id: 99,
            },
        )
        .unwrap();
    let zext = b3.zero_extend(e_in, 16).unwrap();
    let r3 = b3.build(zext).unwrap();
    let zw3 = eliminate_zero_width(&r3).unwrap();
    assert_eq!(zw3.region.boundary.inputs.len(), 1);
    let e3 = eval_region(
        &zw3.region,
        &env_one(InputIdentity::new("call0", 10, 0, 0), BitValue::empty()),
    )
    .unwrap();
    assert_eq!(e3.value, BitValue::zeros(16));
    assert_eq!(e3.trace.len(), 1);

    // 3. Static and dynamic x[k..k] -> empty preserving dynamic bounds check!
    let mut b4 = RegionBuilder::new();
    let word = b4.pure_input("word", 1, 16);
    let dyn_k = b4.pure_input("k", 2, 16);
    let empty_static = b4.extract_const(word, 16, 0).unwrap();
    let empty_dyn = b4.extract_dynamic(word, dyn_k, 0).unwrap();
    let eq_both = b4.equal(empty_static, empty_dyn).unwrap();
    let r4 = b4.build_bool(eq_both).unwrap();
    let zw4 = eliminate_zero_width(&r4).unwrap();
    assert_eq!(zw4.region.boundary.checks.len(), 1);
    let in_ok = env_two(
        InputIdentity::pure("word", 1),
        BitValue::from_u64(16, 0x1234).unwrap(),
        InputIdentity::pure("k", 2),
        BitValue::from_u64(16, 16).unwrap(),
    );
    assert_eq!(
        eval_region_value(&zw4.region, &in_ok).unwrap().to_bool(),
        Some(true)
    );
    let in_oob = env_two(
        InputIdentity::pure("word", 1),
        BitValue::from_u64(16, 0x1234).unwrap(),
        InputIdentity::pure("k", 2),
        BitValue::from_u64(16, 17).unwrap(),
    );
    assert!(matches!(
        eval_region(&zw4.region, &in_oob),
        Err(EvalError::BoundaryCheckFailed(_))
    ));

    // 4. Out-of-range literal on BitVec<0> or BitVec<4> is rejected.
    assert!(matches!(
        BitValue::from_u64(0, 1),
        Err(IrError::LiteralOutOfRange { width: 0, .. })
    ));
    assert!(matches!(
        BitValue::from_u64(4, 16),
        Err(IrError::LiteralOutOfRange { width: 4, .. })
    ));
}

#[test]
fn test_single_and_cross_word_baseline_lowering_accuracy() {
    // Doc example: join_bytes(0xAF, 0x3C) == 0xAF3C
    let mut b_join = RegionBuilder::new();
    let hi = b_join.pure_input("high", 1, 8);
    let lo = b_join.pure_input("low", 2, 8);
    let joined = b_join.concat(hi, lo).unwrap();
    let r_join = b_join.build(joined).unwrap();
    let join_prog64 = lower_to_baseline(&r_join, 64).unwrap();
    let join_prog32 = lower_to_baseline(&r_join, 32).unwrap();
    let join_env = env_two(
        InputIdentity::pure("high", 1),
        BitValue::from_u64(8, 0xAF).unwrap(),
        InputIdentity::pure("low", 2),
        BitValue::from_u64(8, 0x3C).unwrap(),
    );
    assert_eq!(
        eval_region_value(&r_join, &join_env).unwrap().to_u64(),
        Some(0xAF3C)
    );
    assert_eq!(
        join_prog64.execute(&join_env).unwrap().value.to_u64(),
        Some(0xAF3C)
    );
    assert_eq!(
        join_prog32.execute(&join_env).unwrap().value.to_u64(),
        Some(0xAF3C)
    );

    // Cross-word test covering 64, 65, and 128 bits with concat, extract, and, or, not, zext, eq.
    let mut b = RegionBuilder::new();
    let a65 = b.pure_input("a65", 1, 65);
    let b63 = b.pure_input("b63", 2, 63);
    let cat128 = b.concat(a65, b63).unwrap(); // cross-word concat (65 ++ 63 = 128)
    let slice_cross = b.extract_const(cat128, 60, 8).unwrap(); // bits 60..68 cross word 0/1
    let slice_65 = b.extract_const(cat128, 1, 65).unwrap(); // 65-bit cross-word slice
    let not_65 = b.not(slice_65).unwrap(); // must clear padding bits 65..128!
    let zext_65 = b.zero_extend(slice_cross, 65).unwrap();
    let combined_65 = b.and(not_65, zext_65).unwrap();
    let or_65 = b.or(combined_65, a65).unwrap();
    let eq_64 = {
        let lo64 = b.extract_const(cat128, 0, 64).unwrap();
        let hi64 = b.extract_const(cat128, 64, 64).unwrap();
        let not_hi = b.not(hi64).unwrap();
        let xored_lo = b.or(lo64, not_hi).unwrap();
        let full = b.concat(or_65, xored_lo).unwrap(); // 65 ++ 64 = 129 bits!
        full
    };
    let region = b.build(eq_64).unwrap();

    let prog64 = lower_to_baseline(&region, 64).unwrap();
    let prog32 = lower_to_baseline(&region, 32).unwrap();

    let a_val = BitValue::from_u128(65, (1u128 << 64) | 0xF123_4567_89AB_CDEF).unwrap();
    let b_val = BitValue::from_u64(63, 0x5A5A_5A5A_F00D_BEEF).unwrap();
    let env = env_two(
        InputIdentity::pure("a65", 1),
        a_val,
        InputIdentity::pure("b63", 2),
        b_val,
    );

    let ref_val = eval_region_value(&region, &env).unwrap();
    let w64_val = prog64.execute(&env).unwrap().value;
    let w32_val = prog32.execute(&env).unwrap().value;
    assert_eq!(w64_val, ref_val);
    assert_eq!(w32_val, ref_val);
    // Verify top word padding bits are strictly masked to zero on BitVec<65> and BitVec<129>.
    assert_eq!(top_word_mask(64), u64::MAX);
    assert_eq!(top_word_mask(65), 1);
    assert_eq!(top_word_mask(128), u64::MAX);
    assert_eq!(ref_val.words()[2] & !top_word_mask(129), 0);

    // Exact 64-bit Not and 128-bit Not without shift-by-W UB
    let mut b_exact = RegionBuilder::new();
    let x64 = b_exact.pure_input("x64", 1, 64);
    let nx64 = b_exact.not(x64).unwrap();
    let x64_dup = b_exact.pure_input("x64_hi", 2, 64);
    let x128 = b_exact.concat(x64_dup, nx64).unwrap();
    let nx128 = b_exact.not(x128).unwrap();
    let r_exact = b_exact.build(nx128).unwrap();
    let p_exact = lower_to_baseline(&r_exact, 64).unwrap();
    let env_exact = env_two(
        InputIdentity::pure("x64", 1),
        BitValue::from_u64(64, 0x0123_4567_89AB_CDEF).unwrap(),
        InputIdentity::pure("x64_hi", 2),
        BitValue::from_u64(64, 0xDEAD_BEEF_CAFE_BABE).unwrap(),
    );
    assert_eq!(
        p_exact.execute(&env_exact).unwrap().value,
        eval_region_value(&r_exact, &env_exact).unwrap()
    );
}

#[test]
fn test_rewrites_and_pattern_recognition_with_justifications() {
    // 1. Algebraic simplifications: not(not(x)), x & x, De Morgan, zext chain, (a ++ b)[0..wb]
    let mut b = RegionBuilder::new();
    let x = b.pure_input("x", 1, 8);
    let y = b.pure_input("y", 2, 8);
    let nn_x = {
        let nx = b.not(x).unwrap();
        b.not(nx).unwrap()
    };
    let xx = b.and(nn_x, x).unwrap();
    let nx = b.not(xx).unwrap();
    let ny = b.not(y).unwrap();
    let dm = b.and(nx, ny).unwrap(); // (!x) & (!y) -> !(x | y)
    let zx1 = b.zero_extend(dm, 16).unwrap();
    let zx2 = b.zero_extend(zx1, 32).unwrap();
    let cat = b.concat(x, zx2).unwrap(); // 8 ++ 32 = 40
    let slice_lo = b.extract_const(cat, 0, 32).unwrap();
    let slice_inner = b.extract_const(slice_lo, 0, 8).unwrap(); // -> !(x | y)
    let region = b.build(slice_inner).unwrap();

    let res = simplify_region(&region).unwrap();
    assert!(res.steps.iter().any(|s| s.rule == "not-not-involution"));
    assert!(res.steps.iter().any(|s| s.rule == "and-idempotent"));
    assert!(res.steps.iter().any(|s| s.rule == "de-morgan-and-of-nots"));
    assert!(res.steps.iter().any(|s| s.rule == "zext-chain"));
    assert!(res.steps.iter().any(|s| s.rule == "concat-extract-low"));
    assert!(res.steps.iter().any(|s| s.rule == "zext-extract-low"));
    for step in &res.steps {
        validate_sexpr(&step.justification.to_string());
    }

    let mut budget = Budget::default();
    assert!(matches!(
        verify_region_equivalence(&region, &res.region, &mut budget).unwrap(),
        ProofOutcome::Equivalent(_)
    ));

    // 2. XOR, bswap16, bswap32, bswap64, rotate pattern recognition
    let mut b_xor = RegionBuilder::new();
    let a = b_xor.pure_input("a", 1, 8);
    let bv = b_xor.pure_input("b", 2, 8);
    let na = b_xor.not(a).unwrap();
    let nb = b_xor.not(bv).unwrap();
    let lhs = b_xor.and(a, nb).unwrap();
    let rhs = b_xor.and(na, bv).unwrap();
    let xor_n = b_xor.or(lhs, rhs).unwrap();
    let r_xor = b_xor.build(xor_n).unwrap();
    let pats_xor = recognize_patterns(&r_xor).unwrap();
    assert!(
        pats_xor
            .iter()
            .any(|p| matches!(p.kind, PatternKind::Xor { width: 8, .. }))
    );

    let mut b_rot = RegionBuilder::new();
    let rx = b_rot.pure_input("x", 1, 13);
    let lo = b_rot.extract_const(rx, 0, 5).unwrap();
    let hi = b_rot.extract_const(rx, 5, 8).unwrap();
    let rot_n = b_rot.concat(lo, hi).unwrap();
    let r_rot = b_rot.build(rot_n).unwrap();
    let pats_rot = recognize_patterns(&r_rot).unwrap();
    assert!(pats_rot.iter().any(|p| matches!(
        p.kind,
        PatternKind::RotateRight {
            rotr_amount: 5,
            rotl_amount: 8,
            width: 13,
            ..
        }
    )));

    let mut b_bswap32 = RegionBuilder::new();
    let w32 = b_bswap32.pure_input("w", 1, 32);
    let b0 = b_bswap32.extract_const(w32, 0, 8).unwrap();
    let b1 = b_bswap32.extract_const(w32, 8, 8).unwrap();
    let b2 = b_bswap32.extract_const(w32, 16, 8).unwrap();
    let b3 = b_bswap32.extract_const(w32, 24, 8).unwrap();
    let c01 = b_bswap32.concat(b0, b1).unwrap();
    let c23 = b_bswap32.concat(b2, b3).unwrap();
    let c_all = b_bswap32.concat(c01, c23).unwrap();
    let r_bswap32 = b_bswap32.build(c_all).unwrap();
    let pats_32 = recognize_patterns(&r_bswap32).unwrap();
    assert!(
        pats_32
            .iter()
            .any(|p| matches!(p.kind, PatternKind::Bswap32 { .. }))
    );

    let mut b_bswap64 = RegionBuilder::new();
    let w64 = b_bswap64.pure_input("w64", 1, 64);
    let mut acc = b_bswap64.extract_const(w64, 0, 8).unwrap();
    for i in 1..8 {
        let byte_i = b_bswap64.extract_const(w64, i * 8, 8).unwrap();
        acc = b_bswap64.concat(acc, byte_i).unwrap();
    }
    let r_bswap64 = b_bswap64.build(acc).unwrap();
    let pats_64 = recognize_patterns(&r_bswap64).unwrap();
    assert!(
        pats_64
            .iter()
            .any(|p| matches!(p.kind, PatternKind::Bswap64 { .. }))
    );
}

#[test]
fn test_synth_xor_bswap_rotate_and_cegis_pruning() {
    let mut cache = SynthesisCache::new();

    // 1. XOR synthesis: ((a & !b) | (!a & b)) -> xor(a, b) verified via SAT
    let mut b_xor = RegionBuilder::new();
    let a = b_xor.pure_input("a", 1, 8);
    let bv = b_xor.pure_input("b", 2, 8);
    let na = b_xor.not(a).unwrap();
    let nb = b_xor.not(bv).unwrap();
    let lhs = b_xor.and(a, nb).unwrap();
    let rhs = b_xor.and(na, bv).unwrap();
    let xor_n = b_xor.or(lhs, rhs).unwrap();
    let r_xor = b_xor.build(xor_n).unwrap();

    let out_xor = synthesize(&r_xor, TargetProfile::Generic, &mut cache).unwrap();
    assert!(matches!(out_xor.status, ProofStatus::Verified(_)));
    let ChosenImplementation::Target(ref prog_xor) = out_xor.implementation else {
        panic!("expected synthesized target program for XOR");
    };
    assert!(
        prog_xor
            .instructions
            .iter()
            .any(|inst| matches!(inst.op, TargetOp::Xor { width: 8, .. }))
    );
    let xor_env = env_two(
        InputIdentity::pure("a", 1),
        BitValue::from_u64(8, 0xAF).unwrap(),
        InputIdentity::pure("b", 2),
        BitValue::from_u64(8, 0x3C).unwrap(),
    );
    assert_eq!(prog_xor.eval(&xor_env).unwrap().value.to_u64(), Some(0x93));

    // 2. bswap16 on X86_64BmiBswap (doc swap_bytes(0x1234) == 0x3412)
    let mut b_16 = RegionBuilder::new();
    let w16 = b_16.pure_input("word", 1, 16);
    let lo8 = b_16.extract_const(w16, 0, 8).unwrap();
    let hi8 = b_16.extract_const(w16, 8, 8).unwrap();
    let sw16 = b_16.concat(lo8, hi8).unwrap();
    let r_16 = b_16.build(sw16).unwrap();
    let out_16 = synthesize(&r_16, TargetProfile::X86_64BmiBswap, &mut cache).unwrap();
    assert!(matches!(out_16.status, ProofStatus::Verified(_)));
    let ChosenImplementation::Target(ref prog_16) = out_16.implementation else {
        panic!("expected bswap16 target program");
    };
    assert!(
        prog_16
            .instructions
            .iter()
            .any(|inst| matches!(inst.op, TargetOp::Bswap16 { .. }))
    );
    assert_eq!(
        prog_16
            .eval(&env_one(
                InputIdentity::pure("word", 1),
                BitValue::from_u64(16, 0x1234).unwrap(),
            ))
            .unwrap()
            .value
            .to_u64(),
        Some(0x3412)
    );

    // 3. bswap32 on X86_64BmiBswap
    let mut b_32 = RegionBuilder::new();
    let w32 = b_32.pure_input("word32", 1, 32);
    let c0 = b_32.extract_const(w32, 0, 8).unwrap();
    let c1 = b_32.extract_const(w32, 8, 8).unwrap();
    let c2 = b_32.extract_const(w32, 16, 8).unwrap();
    let c3 = b_32.extract_const(w32, 24, 8).unwrap();
    let c01 = b_32.concat(c0, c1).unwrap();
    let c012 = b_32.concat(c01, c2).unwrap();
    let c0123 = b_32.concat(c012, c3).unwrap();
    let r_32 = b_32.build(c0123).unwrap();
    let out_32 = synthesize(&r_32, TargetProfile::X86_64BmiBswap, &mut cache).unwrap();
    let ChosenImplementation::Target(ref prog_32) = out_32.implementation else {
        panic!("expected bswap32 target program");
    };
    assert!(
        prog_32
            .instructions
            .iter()
            .any(|inst| matches!(inst.op, TargetOp::Bswap32 { .. }))
    );
    assert_eq!(
        prog_32
            .eval(&env_one(
                InputIdentity::pure("word32", 1),
                BitValue::from_u64(32, 0x1234_5678).unwrap(),
            ))
            .unwrap()
            .value
            .to_u64(),
        Some(0x7856_3412)
    );

    // 4. rotate on Aarch64
    let mut b_rot = RegionBuilder::new();
    let rx = b_rot.pure_input("rx", 1, 16);
    let rlo = b_rot.extract_const(rx, 0, 3).unwrap();
    let rhi = b_rot.extract_const(rx, 3, 13).unwrap();
    let rcat = b_rot.concat(rlo, rhi).unwrap();
    let r_rot = b_rot.build(rcat).unwrap();
    let out_rot = synthesize(&r_rot, TargetProfile::Aarch64, &mut cache).unwrap();
    let ChosenImplementation::Target(ref prog_rot) = out_rot.implementation else {
        panic!("expected rotate target program");
    };
    assert!(prog_rot.instructions.iter().any(|inst| matches!(
        inst.op,
        TargetOp::RotrConst {
            width: 16,
            amount: 3,
            ..
        }
    )));

    // 5. CEGIS non-trivial synthesis: (a & b) | (a & !b) -> Copy(a), pruning invalid candidates!
    let mut b_cegis = RegionBuilder::new();
    let ca = b_cegis.pure_input("ca", 1, 8);
    let cb = b_cegis.pure_input("cb", 2, 8);
    let cab = b_cegis.and(ca, cb).unwrap();
    let ncb = b_cegis.not(cb).unwrap();
    let canb = b_cegis.and(ca, ncb).unwrap();
    let cor = b_cegis.or(cab, canb).unwrap();
    let r_cegis = b_cegis.build(cor).unwrap();
    let out_cegis = synthesize(&r_cegis, TargetProfile::Generic, &mut cache).unwrap();
    assert!(matches!(out_cegis.status, ProofStatus::Verified(_)));
    assert_eq!(out_cegis.cost, 0);
    assert!(out_cegis.stats.counterexamples_pruned > 0);
}

#[test]
fn test_ub_shift_rejected_without_domain_and_accepted_with_domain() {
    // Dynamic slice window(word: BitVec<16>, start: BitVec<16>) -> BitVec<4>
    let word_id = InputIdentity::pure("word", 1);
    let start_id = InputIdentity::pure("start", 2);

    let build_window = |domain: DomainConstraint| -> Region {
        let mut b = RegionBuilder::new();
        let w = b
            .input_with_effect(word_id.clone(), 16, InputEffect::PureValue)
            .unwrap();
        let s = b
            .input_with_effect(start_id.clone(), 16, InputEffect::PureValue)
            .unwrap();
        let slice = b.extract_dynamic(w, s, 4).unwrap();
        b.add_domain(domain).unwrap();
        b.build(slice).unwrap()
    };

    let candidate_lshr = |domain: DomainConstraint, boundary: BehaviorBoundary| -> TargetProgram {
        TargetProgram {
            profile: TargetProfile::Generic,
            declared_inputs: BTreeMap::from([(word_id.clone(), 16), (start_id.clone(), 16)]),
            domain,
            boundary,
            instructions: vec![
                TargetInst {
                    dst: TargetReg(0),
                    width: 16,
                    op: TargetOp::Input {
                        width: 16,
                        identity: word_id.clone(),
                    },
                },
                TargetInst {
                    dst: TargetReg(1),
                    width: 16,
                    op: TargetOp::Input {
                        width: 16,
                        identity: start_id.clone(),
                    },
                },
                TargetInst {
                    dst: TargetReg(2),
                    width: 16,
                    op: TargetOp::Lshr {
                        input: TargetReg(0),
                        width: 16,
                        shift: TargetReg(1),
                    },
                },
                TargetInst {
                    dst: TargetReg(3),
                    width: 4,
                    op: TargetOp::ExtractConst {
                        input: TargetReg(2),
                        start: 0,
                        width: 4,
                    },
                },
            ],
            root: TargetReg(3),
            output_width: 4,
            justifications: vec![],
        }
    };

    // Case A: unconstrained domain allows start >= 16 -> UB in Lshr -> MUST BE REJECTED!
    let r_unconstrained = build_window(DomainConstraint::True);
    let cand_unconstrained =
        candidate_lshr(DomainConstraint::True, r_unconstrained.boundary.clone());
    let mut budget = Budget::default();
    let outcome_bad =
        verify_target_program(&r_unconstrained, &cand_unconstrained, &mut budget).unwrap();
    match outcome_bad {
        ProofOutcome::Counterexample(cex) => {
            assert!(matches!(
                cex.reason,
                CounterexampleReason::UndefinedCandidateExecution(_)
            ));
            let s_val = cex.inputs.get(&start_id).unwrap().to_u64().unwrap();
            assert!(
                s_val >= 16,
                "expected out-of-range shift >= 16, got {s_val}"
            );
        }
        other => panic!("expected Counterexample rejecting UB shift, got {other}"),
    }

    // Case B: constrained domain `start <= 12` -> shift < 16 always holds -> ACCEPTED!
    let dom_bounded = DomainConstraint::ule(
        DomainTerm::input(start_id.clone(), 16),
        DomainTerm::Constant(BitValue::from_u64(16, 12).unwrap()),
    );
    let r_bounded = build_window(dom_bounded.clone());
    let cand_bounded = candidate_lshr(dom_bounded, r_bounded.boundary.clone());
    let mut budget2 = Budget::default();
    let outcome_good = verify_target_program(&r_bounded, &cand_bounded, &mut budget2).unwrap();
    assert!(
        matches!(outcome_good, ProofOutcome::Equivalent(_)),
        "expected bounded window shift to verify, got {outcome_good}"
    );

    // Case C: ReadyFlag restricted encoding `bits in {0, 1}` vs unrestricted BitVec<8>
    let bits_id = InputIdentity::pure("flag_bits", 1);
    let mut b_flag = RegionBuilder::new();
    let fb = b_flag
        .input_with_effect(bits_id.clone(), 8, InputEffect::PureValue)
        .unwrap();
    let one = b_flag.const_u64(8, 1).unwrap();
    let masked = b_flag.and(fb, one).unwrap();
    let r_flag_unrestricted = b_flag.build(masked).unwrap();

    let copy_cand = |domain: DomainConstraint, boundary: BehaviorBoundary| TargetProgram {
        profile: TargetProfile::Generic,
        declared_inputs: BTreeMap::from([(bits_id.clone(), 8)]),
        domain,
        boundary,
        instructions: vec![TargetInst {
            dst: TargetReg(0),
            width: 8,
            op: TargetOp::Input {
                width: 8,
                identity: bits_id.clone(),
            },
        }],
        root: TargetReg(0),
        output_width: 8,
        justifications: vec![],
    };

    // Without ReadyFlag domain: `flag_bits & 1 != flag_bits` (e.g. flag_bits == 2)
    let mut b_sat = Budget::default();
    let rej = verify_target_program(
        &r_flag_unrestricted,
        &copy_cand(DomainConstraint::True, r_flag_unrestricted.boundary.clone()),
        &mut b_sat,
    )
    .unwrap();
    assert!(matches!(rej, ProofOutcome::Counterexample(_)));

    // With ReadyFlag domain `flag_bits in {0, 1}`: `(flag_bits & 1) == flag_bits`!
    let allowed = BTreeSet::from([
        BitValue::from_u64(8, 0).unwrap(),
        BitValue::from_u64(8, 1).unwrap(),
    ]);
    let flag_dom = DomainConstraint::InSet {
        term: DomainTerm::input(bits_id.clone(), 8),
        allowed,
    };
    let mut r_flag_restricted = r_flag_unrestricted;
    r_flag_restricted.domain = flag_dom.clone();
    let acc = verify_target_program(
        &r_flag_restricted,
        &copy_cand(flag_dom, r_flag_restricted.boundary.clone()),
        &mut b_sat,
    )
    .unwrap();
    assert!(matches!(acc, ProofOutcome::Equivalent(_)));

    // Also test exact inclusive vs half-open extract conversion helper
    assert_eq!(half_open_to_inclusive_extract(0, 0), None);
    assert_eq!(half_open_to_inclusive_extract(0, 1), Some((0, 0)));
    assert_eq!(half_open_to_inclusive_extract(4, 8), Some((11, 4)));
    assert_eq!(half_open_to_inclusive_extract(8, 8), Some((15, 8)));
}

#[test]
fn test_unknown_budget_fallback_preserves_valid_baseline_and_does_not_poison_cache() {
    let mut b = RegionBuilder::new();
    let a = b.pure_input("a", 1, 16);
    let bv = b.pure_input("b", 2, 16);
    let na = b.not(a).unwrap();
    let nb = b.not(bv).unwrap();
    let l = b.and(a, nb).unwrap();
    let r = b.and(na, bv).unwrap();
    let x = b.or(l, r).unwrap();
    let region = b.build(x).unwrap();

    let mut cache = SynthesisCache::new();
    let tiny_budget = Budget::new(1);
    let fallback = synthesize_with_budget(
        &region,
        TargetProfile::Generic,
        TargetProfile::Generic.default_cost_model(),
        tiny_budget,
        &mut cache,
    )
    .unwrap();

    assert!(matches!(
        fallback.status,
        ProofStatus::BaselineFallback(FallbackReason::SolverUnknown(_))
    ));
    assert!(matches!(
        fallback.implementation,
        ChosenImplementation::Baseline(_)
    ));
    assert!(
        cache.is_empty(),
        "Unknown(Budget) fallback must not poison synthesis cache"
    );

    // Fallback baseline still computes exact right answer!
    let env = env_two(
        InputIdentity::pure("a", 1),
        BitValue::from_u64(16, 0xCAFE).unwrap(),
        InputIdentity::pure("b", 2),
        BitValue::from_u64(16, 0xBABE).unwrap(),
    );
    assert_eq!(
        fallback.implementation.eval(&env).unwrap().value,
        eval_region_value(&region, &env).unwrap()
    );

    // Retrying with sufficient budget succeeds cleanly!
    let recovered = synthesize(&region, TargetProfile::Generic, &mut cache).unwrap();
    assert!(matches!(recovered.status, ProofStatus::Verified(_)));
    assert!(matches!(
        recovered.implementation,
        ChosenImplementation::Target(_)
    ));
    assert_eq!(cache.len(), 1);
}

#[test]
fn test_side_effect_and_memory_version_behavior_boundary_preservation() {
    // 1. `zero and produce_bits()` constant-folds pure value to 0, yet MUST still call produce_bits()!
    let prod_id = InputIdentity::new("produce", 1, 0, 0);
    let mut b = RegionBuilder::new();
    let zero = b.const_u64(8, 0).unwrap();
    let call_node = b
        .input_with_effect(
            prod_id.clone(),
            8,
            InputEffect::SideEffectCall {
                symbol: String::from("produce_bits"),
                call_id: 42,
            },
        )
        .unwrap();
    let res_node = b.and(zero, call_node).unwrap();
    b.add_cleanup(CleanupAction {
        cleanup_order: 0,
        resource_id: 500,
        symbol: String::from("drop_guard"),
    });
    let region = b.build(res_node).unwrap();

    let mut cache = SynthesisCache::new();
    let outcome = synthesize(&region, TargetProfile::Generic, &mut cache).unwrap();
    assert!(matches!(outcome.status, ProofStatus::Verified(_)));

    let env = env_one(prod_id.clone(), BitValue::from_u64(8, 0xAF).unwrap());
    let ref_res = eval_region(&region, &env).unwrap();
    let opt_res = outcome.implementation.eval(&env).unwrap();
    assert_eq!(opt_res.value, BitValue::zeros(8));
    assert_eq!(opt_res.value, ref_res.value);
    assert_eq!(opt_res.trace, ref_res.trace);
    assert!(opt_res.trace.iter().any(|ev| matches!(
        ev,
        TraceEvent::InputEvaluated {
            effect: InputEffect::SideEffectCall { symbol, call_id: 42 },
            ..
        } if symbol == "produce_bits"
    )));
    assert!(opt_res.trace.iter().any(|ev| matches!(
        ev,
        TraceEvent::CleanupExecuted { cleanup } if cleanup.symbol == "drop_guard"
    )));

    // Rogue target dropping the boundary event MUST be rejected!
    if let ChosenImplementation::Target(mut rogue) = outcome.implementation.clone() {
        rogue.boundary.inputs.clear();
        let mut budget = Budget::default();
        let rej = verify_target_program(&region, &rogue, &mut budget).unwrap();
        assert!(matches!(
            rej,
            ProofOutcome::Counterexample(cex) if cex.reason == CounterexampleReason::BoundaryContractMismatch
        ));
    }

    // 2. Distinct memory versions of the same place MUST NEVER merge (`v0 & v1 != v0`)!
    let v0 = InputIdentity::new("word", 77, 0, 0);
    let v1 = InputIdentity::new("word", 77, 1, 1);
    let mut bm = RegionBuilder::new();
    let r0 = bm
        .input_with_effect(
            v0.clone(),
            8,
            InputEffect::MemoryRead {
                place_id: 77,
                memory_version: 0,
            },
        )
        .unwrap();
    let r1 = bm
        .input_with_effect(
            v1.clone(),
            8,
            InputEffect::MemoryRead {
                place_id: 77,
                memory_version: 1,
            },
        )
        .unwrap();
    let both = bm.and(r0, r1).unwrap();
    let reg_mem = bm.build(both).unwrap();
    assert_ne!(
        r0, r1,
        "distinct memory versions must have distinct NodeIds"
    );

    let simp = simplify_region(&reg_mem).unwrap();
    assert!(
        simp.steps.is_empty(),
        "must not apply and-idempotent across distinct memory versions: {:?}",
        simp.steps
    );
    let mut b_only_v0 = RegionBuilder::new();
    let _ = b_only_v0
        .input_with_effect(
            v0.clone(),
            8,
            InputEffect::MemoryRead {
                place_id: 77,
                memory_version: 0,
            },
        )
        .unwrap();
    let reg_only_v0 = {
        let mut reg = reg_mem.clone();
        reg.root = r0;
        reg
    };
    let mut bud = Budget::default();
    assert!(matches!(
        verify_region_equivalence(&reg_mem, &reg_only_v0, &mut bud).unwrap(),
        ProofOutcome::Counterexample(_)
    ));
}

#[test]
fn test_synthesis_cache_isolation_across_targets_domains_and_alpha_equivalence() {
    let mut cache = SynthesisCache::new();

    // 1. Alpha-equivalent XOR regions hit the SAME cache entry under same target & domain.
    let build_xor = |name_a: &str, pa: u64, name_b: &str, pb: u64| {
        let mut b = RegionBuilder::new();
        let a = b.pure_input(name_a, pa, 8);
        let bv = b.pure_input(name_b, pb, 8);
        let na = b.not(a).unwrap();
        let nb = b.not(bv).unwrap();
        let l = b.and(a, nb).unwrap();
        let r = b.and(na, bv).unwrap();
        let x = b.or(l, r).unwrap();
        b.build(x).unwrap()
    };

    let xor1 = build_xor("alpha", 1, "beta", 2);
    let xor2 = build_xor("left_v", 50, "right_v", 99);

    let o1 = synthesize(&xor1, TargetProfile::Generic, &mut cache).unwrap();
    assert!(!o1.stats.cache_hit);
    let o2 = synthesize(&xor2, TargetProfile::Generic, &mut cache).unwrap();
    assert!(
        o2.stats.cache_hit,
        "alpha-equivalent region should hit cache"
    );
    assert_eq!(cache.len(), 1);

    // Cache hit rebound to xor2's own input identities!
    let xor2_env = env_two(
        InputIdentity::pure("left_v", 50),
        BitValue::from_u64(8, 0xAA).unwrap(),
        InputIdentity::pure("right_v", 99),
        BitValue::from_u64(8, 0x0F).unwrap(),
    );
    assert_eq!(
        o2.implementation.eval(&xor2_env).unwrap().value.to_u64(),
        Some(0xA5)
    );

    // 2. Different TargetProfile MUST NOT hit Generic's cache entry!
    let mut b16 = RegionBuilder::new();
    let w = b16.pure_input("w", 1, 16);
    let lo = b16.extract_const(w, 0, 8).unwrap();
    let hi = b16.extract_const(w, 8, 8).unwrap();
    let cat = b16.concat(lo, hi).unwrap();
    let r16 = b16.build(cat).unwrap();

    let out_gen = synthesize(&r16, TargetProfile::Generic, &mut cache).unwrap();
    let out_x86 = synthesize(&r16, TargetProfile::X86_64BmiBswap, &mut cache).unwrap();
    let out_arm = synthesize(&r16, TargetProfile::Aarch64, &mut cache).unwrap();
    assert!(!out_x86.stats.cache_hit);
    assert!(!out_arm.stats.cache_hit);
    assert!(matches!(
        out_gen.implementation,
        ChosenImplementation::Baseline(_)
    ));
    assert!(matches!(
        out_x86.implementation,
        ChosenImplementation::Target(_)
    ));
    assert!(matches!(
        out_arm.implementation,
        ChosenImplementation::Target(_)
    ));

    // 3. Different DomainConstraint MUST NOT hit another domain's cache entry!
    let bits_id = InputIdentity::pure("bits", 1);
    let mut b_dom = RegionBuilder::new();
    let bi = b_dom.pure_input("bits", 1, 8);
    let one = b_dom.const_u64(8, 1).unwrap();
    let masked = b_dom.and(bi, one).unwrap();
    let allowed = BTreeSet::from([
        BitValue::from_u64(8, 0).unwrap(),
        BitValue::from_u64(8, 1).unwrap(),
    ]);
    b_dom
        .add_domain(DomainConstraint::InSet {
            term: DomainTerm::input(bits_id.clone(), 8),
            allowed,
        })
        .unwrap();
    let r_restricted = b_dom.build(masked).unwrap();
    let mut r_unrestricted = r_restricted.clone();
    r_unrestricted.domain = DomainConstraint::True;

    let res_res = synthesize(&r_restricted, TargetProfile::Generic, &mut cache).unwrap();
    let res_unres = synthesize(&r_unrestricted, TargetProfile::Generic, &mut cache).unwrap();
    assert!(!res_unres.stats.cache_hit);
    assert!(matches!(
        res_res.implementation,
        ChosenImplementation::Target(_)
    ));
    assert!(matches!(
        res_unres.implementation,
        ChosenImplementation::Baseline(_)
    ));

    // 4. Different CostModel isolates cache entries even on same TargetProfile.
    let mut custom_cost: CostModel = TargetProfile::Generic.default_cost_model();
    custom_cost.xor_cost = 3;
    let res_custom = synthesize_with_budget(
        &xor1,
        TargetProfile::Generic,
        custom_cost,
        Budget::default(),
        &mut cache,
    )
    .unwrap();
    assert!(!res_custom.stats.cache_hit);
    assert_eq!(res_custom.cost, 3);
}

#[test]
fn test_doc_examples_static_errors_and_mult_word_sat_verification() {
    // 1. Doc pack_header(5, 17, 42) == 45354 and assemble_payload(0b10101, 0b101010) == 1386
    let mut b_hdr = RegionBuilder::new();
    let kind = b_hdr.pure_input("kind", 1, 3);
    let flags = b_hdr.pure_input("flags", 2, 5);
    let length = b_hdr.pure_input("length", 3, 8);
    let kf = b_hdr.concat(kind, flags).unwrap();
    let hdr = b_hdr.concat(kf, length).unwrap();
    let r_hdr = b_hdr.build(hdr).unwrap();
    let p_hdr = lower_to_baseline(&r_hdr, 64).unwrap();
    let mut hdr_env = BTreeMap::new();
    hdr_env.insert(
        InputIdentity::pure("kind", 1),
        BitValue::from_u64(3, 5).unwrap(),
    );
    hdr_env.insert(
        InputIdentity::pure("flags", 2),
        BitValue::from_u64(5, 17).unwrap(),
    );
    hdr_env.insert(
        InputIdentity::pure("length", 3),
        BitValue::from_u64(8, 42).unwrap(),
    );
    assert_eq!(p_hdr.execute(&hdr_env).unwrap().value.to_u64(), Some(45354));

    let mut b_pay = RegionBuilder::new();
    let hi = b_pay.pure_input("hi", 1, 5);
    let lo = b_pay.pure_input("lo", 2, 6);
    let hl = b_pay.concat(hi, lo).unwrap();
    let z = b_pay.zero_extend(hl, 32).unwrap();
    let r_pay = b_pay.build(z).unwrap();
    let pay_env = env_two(
        InputIdentity::pure("hi", 1),
        BitValue::from_u64(5, 0b10101).unwrap(),
        InputIdentity::pure("lo", 2),
        BitValue::from_u64(6, 0b101010).unwrap(),
    );
    assert_eq!(
        eval_region_value(&r_pay, &pay_env).unwrap().to_u64(),
        Some(1386)
    );

    // 2. Static errors: narrowing ZeroExtend, width mismatch, out-of-bounds static extract
    let mut b_err = RegionBuilder::new();
    let e16 = b_err.pure_input("e16", 1, 16);
    let e8 = b_err.pure_input("e8", 2, 8);
    assert!(matches!(
        b_err.zero_extend(e16, 8),
        Err(IrError::InvalidZeroExtend {
            input_width: 16,
            target_width: 8
        })
    ));
    assert!(matches!(
        b_err.and(e16, e8),
        Err(IrError::WidthMismatch {
            op: "and",
            left: 16,
            right: 8
        })
    ));
    assert!(matches!(
        b_err.extract_const(e8, 5, 4),
        Err(IrError::SliceOutOfBounds {
            input_width: 8,
            start: 5,
            slice_width: 4
        })
    ));

    // 3. Cross-word dynamic extract on BitVec<128> (word boundary at bit 64)
    let mut b_dyn = RegionBuilder::new();
    let big_id = InputIdentity::pure("big", 1);
    let idx_id = InputIdentity::pure("idx", 2);
    let big = b_dyn.pure_input("big", 1, 128);
    let idx = b_dyn.pure_input("idx", 2, 128);
    let dyn_slice = b_dyn.extract_dynamic(big, idx, 8).unwrap();
    b_dyn.add_boundary_check(BoundaryCheck::PreconditionCheck {
        label: String::from("idx_nonzero"),
        condition: DomainConstraint::UnsignedCmp {
            lhs: DomainTerm::input(idx_id.clone(), 128),
            op: CmpOp::Ne,
            rhs: DomainTerm::Constant(BitValue::zeros(128)),
        },
        check_order: 0,
    });
    let r_dyn = b_dyn.build(dyn_slice).unwrap();
    let p_dyn = lower_to_baseline(&r_dyn, 64).unwrap();
    let big_val = BitValue::from_u128(128, (0xAAu128 << 64) | (0xBBu128 << 56)).unwrap();
    let dyn_env = env_two(
        big_id,
        big_val,
        idx_id,
        BitValue::from_u64(128, 60).unwrap(),
    );
    let ref_dyn = eval_region(&r_dyn, &dyn_env).unwrap();
    let base_dyn = p_dyn.execute(&dyn_env).unwrap();
    assert_eq!(base_dyn.value, ref_dyn.value);
    assert_eq!(base_dyn.value.to_u64(), Some(0xAB));
    assert_eq!(base_dyn.trace, ref_dyn.trace);

    // 4. Cross-word SAT equivalence on BitVec<65> and 64-bit bswap64
    let mut b65 = RegionBuilder::new();
    let x65 = b65.pure_input("x65", 1, 65);
    let y65 = b65.pure_input("y65", 2, 65);
    let nx = b65.not(x65).unwrap();
    let ny = b65.not(y65).unwrap();
    let dm = b65.and(nx, ny).unwrap();
    let r_dm = b65.build(dm).unwrap();
    let simp65 = simplify_region(&r_dm).unwrap();
    let mut sat_budget = Budget::default();
    let eq65 = verify_region_equivalence(&r_dm, &simp65.region, &mut sat_budget).unwrap();
    assert!(matches!(eq65, ProofOutcome::Equivalent(_)));
    validate_sexpr(&eq65.to_string());

    // Manual TargetProgram bswap64 SAT verification
    let w64_id = InputIdentity::pure("w64", 1);
    let mut b64 = RegionBuilder::new();
    let w64 = b64.pure_input("w64", 1, 64);
    let mut acc = b64.extract_const(w64, 0, 8).unwrap();
    for i in 1..8 {
        let byte_i = b64.extract_const(w64, i * 8, 8).unwrap();
        acc = b64.concat(acc, byte_i).unwrap();
    }
    let r64 = b64.build(acc).unwrap();
    let cand64 = TargetProgram {
        profile: TargetProfile::X86_64BmiBswap,
        declared_inputs: BTreeMap::from([(w64_id.clone(), 64)]),
        domain: DomainConstraint::True,
        boundary: BehaviorBoundary {
            inputs: vec![BoundaryInput {
                identity: w64_id.clone(),
                width: 64,
                effect: InputEffect::PureValue,
            }],
            checks: Vec::new(),
            cleanups: Vec::new(),
        },
        instructions: vec![
            TargetInst {
                dst: TargetReg(0),
                width: 64,
                op: TargetOp::Input {
                    width: 64,
                    identity: w64_id,
                },
            },
            TargetInst {
                dst: TargetReg(1),
                width: 64,
                op: TargetOp::Bswap64 {
                    input: TargetReg(0),
                },
            },
        ],
        root: TargetReg(1),
        output_width: 64,
        justifications: vec![Justification::Axiom {
            rule: "bswap64-pattern",
            theorem: "8-byte reversal = bswap64",
            detail: String::from("manual bswap64 check"),
        }],
    };
    let eq64 = verify_target_program(&r64, &cand64, &mut sat_budget).unwrap();
    assert!(matches!(eq64, ProofOutcome::Equivalent(_)));
    validate_sexpr(&cand64.to_string());
}

#[test]
fn test_end_to_end_dynamic_window_synthesis_shift_safety_and_aarch64_orn() {
    let mut cache = SynthesisCache::new();
    let word_id = InputIdentity::pure("word", 1);
    let start_id = InputIdentity::pure("start", 2);

    let make_window_region = |dom: DomainConstraint| -> Region {
        let mut b = RegionBuilder::new();
        let w = b
            .input_with_effect(word_id.clone(), 16, InputEffect::PureValue)
            .unwrap();
        let s = b
            .input_with_effect(start_id.clone(), 16, InputEffect::PureValue)
            .unwrap();
        let slice = b.extract_dynamic(w, s, 4).unwrap();
        b.add_domain(dom).unwrap();
        b.build(slice).unwrap()
    };

    // Unconstrained domain: full pipeline MUST reject Lshr candidate and retain BaselineProgram!
    let r_open = make_window_region(DomainConstraint::True);
    let out_open = synthesize(&r_open, TargetProfile::Generic, &mut cache).unwrap();
    assert!(matches!(
        out_open.implementation,
        ChosenImplementation::Baseline(_)
    ));

    // Domain `start <= 12`: full pipeline proves Lshr + ExtractConst safe and synthesizes TargetProgram!
    let dom_12 = DomainConstraint::ule(
        DomainTerm::input(start_id.clone(), 16),
        DomainTerm::Constant(BitValue::from_u64(16, 12).unwrap()),
    );
    let r_bounded = make_window_region(dom_12);
    let out_bounded = synthesize(&r_bounded, TargetProfile::X86_64BmiBswap, &mut cache).unwrap();
    assert!(matches!(out_bounded.status, ProofStatus::Verified(_)));
    assert!(matches!(
        out_bounded.implementation,
        ChosenImplementation::Target(_)
    ));
    let win_env = env_two(
        word_id,
        BitValue::from_u64(16, 0xABCD).unwrap(),
        start_id,
        BitValue::from_u64(16, 4).unwrap(),
    );
    // Doc example: window(0xABCD, 4) == 12
    assert_eq!(
        out_bounded
            .implementation
            .eval(&win_env)
            .unwrap()
            .value
            .to_u64(),
        Some(12)
    );

    // Aarch64 supports OrNot (`orn`: a | !b) at cost 1 while X86_64BmiBswap does not!
    let mut b_orn = RegionBuilder::new();
    let a = b_orn.pure_input("a", 1, 32);
    let bv = b_orn.pure_input("b", 2, 32);
    let nb = b_orn.not(bv).unwrap();
    let a_or_nb = b_orn.or(a, nb).unwrap();
    let r_orn = b_orn.build(a_or_nb).unwrap();

    let out_arm = synthesize(&r_orn, TargetProfile::Aarch64, &mut cache).unwrap();
    let out_x86 = synthesize(&r_orn, TargetProfile::X86_64BmiBswap, &mut cache).unwrap();
    assert!(matches!(
        out_arm.implementation,
        ChosenImplementation::Target(ref p) if p.instructions.iter().any(|i| matches!(i.op, TargetOp::OrNot { .. }))
    ));
    assert!(matches!(
        out_x86.implementation,
        ChosenImplementation::Baseline(_)
    ));

    // Split-nibble bswap16 pattern recognition: (x[4..8] ++ x[0..4]) ++ (x[12..16] ++ x[8..12])
    let mut b_nib = RegionBuilder::new();
    let xn = b_nib.pure_input("x", 1, 16);
    let n0 = b_nib.extract_const(xn, 0, 4).unwrap();
    let n1 = b_nib.extract_const(xn, 4, 4).unwrap();
    let n2 = b_nib.extract_const(xn, 8, 4).unwrap();
    let n3 = b_nib.extract_const(xn, 12, 4).unwrap();
    let b_lo = b_nib.concat(n1, n0).unwrap();
    let b_hi = b_nib.concat(n3, n2).unwrap();
    let sw_nib = b_nib.concat(b_lo, b_hi).unwrap();
    let r_nib = b_nib.build(sw_nib).unwrap();
    let nib_pats = recognize_patterns(&r_nib).unwrap();
    assert!(
        nib_pats
            .iter()
            .any(|p| matches!(p.kind, PatternKind::Bswap16 { .. }))
    );

    // Non-equivalent wrong bswap16 (identity x[8..16] ++ x[0..8] vs Bswap16) gets counterexample!
    let mut b_id16 = RegionBuilder::new();
    let x16 = b_id16.pure_input("x16", 1, 16);
    let lo16 = b_id16.extract_const(x16, 0, 8).unwrap();
    let hi16 = b_id16.extract_const(x16, 8, 8).unwrap();
    let id16 = b_id16.concat(hi16, lo16).unwrap();
    let r_id16 = b_id16.build(id16).unwrap();
    let x16_id = InputIdentity::pure("x16", 1);
    let wrong_bswap = TargetProgram {
        profile: TargetProfile::X86_64BmiBswap,
        declared_inputs: BTreeMap::from([(x16_id.clone(), 16)]),
        domain: DomainConstraint::True,
        boundary: r_id16.boundary.clone(),
        instructions: vec![
            TargetInst {
                dst: TargetReg(0),
                width: 16,
                op: TargetOp::Input {
                    width: 16,
                    identity: x16_id,
                },
            },
            TargetInst {
                dst: TargetReg(1),
                width: 16,
                op: TargetOp::Bswap16 {
                    input: TargetReg(0),
                },
            },
        ],
        root: TargetReg(1),
        output_width: 16,
        justifications: vec![],
    };
    let mut b_chk = Budget::default();
    let rej_bswap = verify_target_program(&r_id16, &wrong_bswap, &mut b_chk).unwrap();
    assert!(matches!(
        rej_bswap,
        ProofOutcome::Counterexample(ref cex) if cex.reason == CounterexampleReason::ValueMismatch
    ));
    validate_sexpr(&rej_bswap.to_string());
}

#[test]
fn test_lisp_sexpr_display_stability_and_escaping() {
    let tricky_name = "a\"b\\c\nd\te";
    let mut b = RegionBuilder::new();
    let id = InputIdentity::new(tricky_name, 7, 2, 0);
    let node = b
        .input_with_effect(
            id.clone(),
            16,
            InputEffect::SideEffectCall {
                symbol: String::from("fn\"call\n1"),
                call_id: 3,
            },
        )
        .unwrap();
    let lo = b.extract_const(node, 0, 8).unwrap();
    let hi = b.extract_const(node, 8, 8).unwrap();
    let sw = b.concat(lo, hi).unwrap();
    b.add_domain(DomainConstraint::ule(
        DomainTerm::input(id, 16),
        DomainTerm::Constant(BitValue::from_u64(16, 0x7FFF).unwrap()),
    ))
    .unwrap();
    b.add_cleanup(CleanupAction {
        cleanup_order: 0,
        resource_id: 1,
        symbol: String::from("clean\"up"),
    });
    let region = b.build(sw).unwrap();

    validate_sexpr(&region.to_string());
    validate_sexpr(&region.canonicalize().unwrap().to_string());
    validate_sexpr(&region.boundary.to_string());
    validate_sexpr(&region.domain.to_string());
    for node in &region.nodes {
        validate_sexpr(&node.to_string());
    }

    let zw = eliminate_zero_width(&region).unwrap();
    validate_sexpr(&zw.to_string());

    let simp = simplify_region(&region).unwrap();
    validate_sexpr(&simp.to_string());

    let baseline = lower_to_baseline(&region, 64).unwrap();
    validate_sexpr(&baseline.to_string());

    let mut cache = SynthesisCache::new();
    let synth_out = synthesize(&region, TargetProfile::X86_64BmiBswap, &mut cache).unwrap();
    validate_sexpr(&synth_out.to_string());
    validate_sexpr(&cache.to_string());

    // Deterministic output across repeated serialization calls
    assert_eq!(synth_out.to_string(), synth_out.to_string());
    assert_eq!(cache.to_string(), cache.to_string());
    assert_eq!(VERIFIER_VERSION, "bitvec-cdcl-tseitin-v1");
    assert_eq!(OutputKind::Bool.bit_width(), 1);
}
