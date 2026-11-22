//! Host LLVM bridge for `compiler/bitvec_opt`.
//!
//! Translates pure finite-DAG BitVec expressions into [`bitvec_opt::Region`]s,
//! preserves strict left-to-right operand evaluation order, memory mutation
//! isolation (`memory_version`), dynamic bounds checks, and cleanup hooks via
//! [`BehaviorBoundary`], runs zero-width elimination + algebraic rewrites +
//! bounded target synthesis with caching, and emits LLVM IR for either the
//! verified [`TargetProgram`], [`BaselineProgram`] fallback, or simplified [`Region`].

use super::*;
use bitvec_opt::{
    BaselineOp, BaselineProgram, BitValue, Budget, ChosenImplementation, ExtractStart, InputEffect,
    InputIdentity, Node, OutputKind, Region, RegionBuilder, SynthesisCache, TargetOp,
    TargetProfile, TargetProgram, eliminate_zero_width, simplify_region, synthesize_with_budget,
};

const MAX_DAG_DEPTH: usize = 64;

const MAX_SYNTH_NODES: usize = 64;

const MAX_SYNTH_INPUTS: usize = 4;

const MAX_SYNTH_WIDTH: u32 = 64;

const SYNTH_MAX_STEPS: u64 = 200_000;

const SYNTH_MAX_CONFLICTS: u64 = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum LeafKey {
    Local(LocalId),
    TakeBitsLocal(LocalId),
}

struct BitVecExtractor {
    builder: RegionBuilder,
    memory_version: u64,
    next_order: u32,
    pure_leaf_map: BTreeMap<(LeafKey, u64), bitvec_opt::NodeId>,
    leaf_values: BTreeMap<InputIdentity, Value>,
}

impl BitVecExtractor {
    fn new() -> Self {
        Self {
            builder: RegionBuilder::new(),
            memory_version: 0,
            next_order: 0,
            pure_leaf_map: BTreeMap::new(),
            leaf_values: BTreeMap::new(),
        }
    }

    fn alloc_identity(&mut self, place_id: u64) -> InputIdentity {
        let eval_order = self.next_order;
        self.next_order = self.next_order.saturating_add(1);
        InputIdentity::new(
            format!("v{eval_order}"),
            place_id,
            self.memory_version,
            eval_order,
        )
    }

    fn extract(
        &mut self,
        emitter: &mut Emitter<'_>,
        expr: &Expr,
        depth: usize,
        dag_depth: usize,
    ) -> Result<Option<bitvec_opt::NodeId>, String> {
        *emitter.fuel = emitter
            .fuel
            .checked_sub(1)
            .ok_or("host BitVec extraction budget exhausted")?;
        if dag_depth >= MAX_DAG_DEPTH || (dag_depth > 0 && emitter.has_cleanup_hook(expr.source)) {
            return self.extract_leaf(emitter, expr, depth, dag_depth);
        }

        match &expr.kind {
            K::BitConcat(high, low) => {
                let Some(hi_id) = self.extract(emitter, high, depth, dag_depth + 1)? else {
                    return Ok(None);
                };
                let Some(lo_id) = self.extract(emitter, low, depth, dag_depth + 1)? else {
                    return Ok(None);
                };
                self.builder
                    .concat(hi_id, lo_id)
                    .map(Some)
                    .map_err(|e| format!("host BitVec concat region error: {e}"))
            }
            K::BitSlice { value, start, end } => {
                let Some(in_id) = self.extract(emitter, value, depth, dag_depth + 1)? else {
                    return Ok(None);
                };
                let start_u32 =
                    u32::try_from(*start).map_err(|_| "host BitVec slice start overflow")?;
                let slice_w =
                    u32::try_from(*end - *start).map_err(|_| "host BitVec slice width overflow")?;
                self.builder
                    .extract_const(in_id, start_u32, slice_w)
                    .map(Some)
                    .map_err(|e| format!("host BitVec slice region error: {e}"))
            }
            K::BitEqual(left, right) => {
                let Some(l_id) = self.extract(emitter, left, depth, dag_depth + 1)? else {
                    return Ok(None);
                };
                let Some(r_id) = self.extract(emitter, right, depth, dag_depth + 1)? else {
                    return Ok(None);
                };
                self.builder
                    .equal(l_id, r_id)
                    .map(Some)
                    .map_err(|e| format!("host BitVec equal region error: {e}"))
            }
            K::And(left, right) if bitvec_width(&expr.ty).is_some() => {
                let Some(l_id) = self.extract(emitter, left, depth, dag_depth + 1)? else {
                    return Ok(None);
                };
                let Some(r_id) = self.extract(emitter, right, depth, dag_depth + 1)? else {
                    return Ok(None);
                };
                self.builder
                    .and(l_id, r_id)
                    .map(Some)
                    .map_err(|e| format!("host BitVec and region error: {e}"))
            }
            K::Or(left, right) if bitvec_width(&expr.ty).is_some() => {
                let Some(l_id) = self.extract(emitter, left, depth, dag_depth + 1)? else {
                    return Ok(None);
                };
                let Some(r_id) = self.extract(emitter, right, depth, dag_depth + 1)? else {
                    return Ok(None);
                };
                self.builder
                    .or(l_id, r_id)
                    .map(Some)
                    .map_err(|e| format!("host BitVec or region error: {e}"))
            }
            K::Not(inner) if bitvec_width(&expr.ty).is_some() => {
                let Some(in_id) = self.extract(emitter, inner, depth, dag_depth + 1)? else {
                    return Ok(None);
                };
                self.builder
                    .not(in_id)
                    .map(Some)
                    .map_err(|e| format!("host BitVec not region error: {e}"))
            }
            K::BitConvert(inner)
                if bitvec_width(&expr.ty).is_some() && bitvec_width(&inner.ty).is_some() =>
            {
                let dst_w = bitvec_width(&expr.ty).expect("checked BitVec width");
                let src_w = bitvec_width(&inner.ty).expect("checked BitVec width");
                let Some(in_id) = self.extract(emitter, inner, depth, dag_depth + 1)? else {
                    return Ok(None);
                };
                if dst_w == src_w {
                    Ok(Some(in_id))
                } else {
                    self.builder
                        .zero_extend(in_id, dst_w)
                        .map(Some)
                        .map_err(|e| format!("host BitVec zero-extend region error: {e}"))
                }
            }
            K::Literal(Literal::BitVec { width, words })
                if u32::try_from(*width).is_ok_and(|w| w <= bits::MAX_BIT_WIDTH) =>
            {
                let w = *width as u32;
                let bits = bitvalue_from_u32_words(w, words)?;
                Ok(Some(self.builder.constant(bits)))
            }
            _ => self.extract_leaf(emitter, expr, depth, dag_depth),
        }
    }

    fn extract_leaf(
        &mut self,
        emitter: &mut Emitter<'_>,
        expr: &Expr,
        depth: usize,
        dag_depth: usize,
    ) -> Result<Option<bitvec_opt::NodeId>, String> {
        let width = bitvec_width(&expr.ty)
            .ok_or("host BitVec DAG leaf must have a bounded BitVec representation")?;
        if let Some(key) = try_dedup_leaf_key(emitter, expr) {
            if let Some(&existing_node) = self.pure_leaf_map.get(&(key, self.memory_version)) {
                return Ok(Some(existing_node));
            }
            let val = emitter.expr(expr, depth.saturating_add(dag_depth))?;
            if emitter.terminated {
                return Ok(None);
            }
            let place_id = match key {
                LeafKey::Local(id) | LeafKey::TakeBitsLocal(id) => {
                    (u64::from(id.owner) << 32) | u64::from(id.index)
                }
            };
            let identity = self.alloc_identity(place_id);
            let node_id = self
                .builder
                .input_with_effect(identity.clone(), width, InputEffect::PureValue)
                .map_err(|e| format!("host BitVec pure leaf error: {e}"))?;
            self.leaf_values.insert(identity, val);
            self.pure_leaf_map
                .insert((key, self.memory_version), node_id);
            Ok(Some(node_id))
        } else {
            let val = emitter.expr(expr, depth.saturating_add(dag_depth))?;
            if emitter.terminated {
                return Ok(None);
            }
            let identity = self.alloc_identity(u64::MAX);
            let call_id = u64::from(identity.eval_order);
            let node_id = self
                .builder
                .input_with_effect(
                    identity.clone(),
                    width,
                    InputEffect::SideEffectCall {
                        symbol: format!("leaf_{call_id}"),
                        call_id,
                    },
                )
                .map_err(|e| format!("host BitVec side-effect leaf error: {e}"))?;
            self.leaf_values.insert(identity, val);
            // Any non-pure leaf may observe or mutate storage, so subsequent pure reads
            // must not alias pre-effect reads.
            self.memory_version = self.memory_version.saturating_add(1);
            Ok(Some(node_id))
        }
    }
}

fn bitvec_width(ty: &Term) -> Option<u32> {
    let Term::Primitive(P::BitVec(w)) = ty else {
        return None;
    };
    let w = u32::try_from(*w).ok()?;
    (w <= bits::MAX_BIT_WIDTH).then_some(w)
}

fn is_copy_bit_primitive(ty: &Term) -> bool {
    matches!(
        ty,
        Term::Primitive(
            P::BitVec(_)
                | P::Char
                | P::I8
                | P::I16
                | P::I32
                | P::I64
                | P::I128
                | P::U8
                | P::U16
                | P::U32
                | P::U64
                | P::U128
                | P::Isize
                | P::Usize
                | P::F16
                | P::Bf16
                | P::F32
                | P::F64
        )
    )
}

fn try_dedup_leaf_key(emitter: &mut Emitter<'_>, expr: &Expr) -> Option<LeafKey> {
    if emitter.has_cleanup_hook(expr.source) || emitter.is_volatile_type(&expr.ty) {
        return None;
    }
    match &expr.kind {
        K::Local(id)
            if is_copy_bit_primitive(&expr.ty)
                && emitter.locals.get(id).is_some_and(|slot| !slot.volatile) =>
        {
            Some(LeafKey::Local(*id))
        }
        K::TakeBits(inner) => {
            let K::Local(id) = &inner.kind else {
                return None;
            };
            if emitter.has_cleanup_hook(inner.source) || emitter.is_volatile_type(&inner.ty) {
                return None;
            }
            if is_copy_bit_primitive(&inner.ty)
                && emitter.locals.get(id).is_some_and(|slot| !slot.volatile)
            {
                Some(LeafKey::TakeBitsLocal(*id))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn bitvalue_from_u32_words(width: u32, words: &[u32]) -> Result<BitValue, String> {
    if width == 0 {
        return Ok(BitValue::empty());
    }
    let expected_u64 = width.div_ceil(64) as usize;
    let mut u64_words = vec![0u64; expected_u64];
    for (idx, chunk) in words.chunks(2).enumerate() {
        let lo = u64::from(chunk[0]);
        let hi = chunk.get(1).map_or(0, |&w| u64::from(w));
        if let Some(slot) = u64_words.get_mut(idx) {
            *slot = lo | (hi << 32);
        } else if (lo | hi) != 0 {
            return Err("host BitVec constant has excess words".into());
        }
    }
    BitValue::from_words(width, u64_words).map_err(|e| format!("host BitVec constant error: {e}"))
}

fn bitvalue_decimal(bits: &BitValue) -> String {
    if bits.width() == 0 {
        return "zeroinitializer".into();
    }
    if let Some(val) = bits.to_u128() {
        return val.to_string();
    }
    let mut u32_words = Vec::with_capacity(bits.words().len() * 2);
    for &w in bits.words() {
        u32_words.push(w as u32);
        u32_words.push((w >> 32) as u32);
    }
    let needed_u32 = bits.width().div_ceil(32) as usize;
    u32_words.truncate(needed_u32);
    bits::decimal_words(&u32_words)
}

fn host_target_profile(graph: &iir::Package) -> TargetProfile {
    if graph.target.is_some_and(|t| t.pointer_bits() != 64) {
        return TargetProfile::Generic;
    }
    if cfg!(target_arch = "aarch64") {
        TargetProfile::Aarch64
    } else if cfg!(target_arch = "x86_64") {
        TargetProfile::X86_64BmiBswap
    } else {
        TargetProfile::Generic
    }
}

fn non_trivial_op_count(region: &Region) -> usize {
    region
        .nodes
        .iter()
        .filter(|node| !matches!(node, Node::Input { .. } | Node::Constant { .. }))
        .count()
}

fn max_region_width(region: &Region) -> u32 {
    region
        .node_widths()
        .map(|ws| ws.into_iter().max().unwrap_or(0))
        .unwrap_or(u32::MAX)
}

fn finalize_root_value(value: Value, output_kind: OutputKind) -> Result<Value, String> {
    match output_kind {
        OutputKind::Bool => {
            if value.ty == Ty::Bool {
                Ok(value)
            } else if value.ty == Ty::Int(1, false) {
                Ok(Value {
                    ty: Ty::Bool,
                    text: value.text,
                })
            } else {
                Err("host BitVec bool root width mismatch".into())
            }
        }
        OutputKind::BitVec(_) => Ok(value),
    }
}

fn empty_bits() -> Value {
    Value {
        ty: Ty::Int(0, false),
        text: "zeroinitializer".into(),
    }
}

fn const_i64_value(value: u64) -> Value {
    Value {
        ty: Ty::Int(64, false),
        text: value.to_string(),
    }
}

fn const_int_value(width: u32, bits: &BitValue) -> Value {
    if width == 0 {
        empty_bits()
    } else {
        Value {
            ty: Ty::Int(width, false),
            text: bitvalue_decimal(bits),
        }
    }
}

impl Emitter<'_> {
    fn has_cleanup_hook(&self, source: nir::program::SourceRange) -> bool {
        let Some(cleanup) = self.cleanup else {
            return false;
        };
        // `cleanup.copies` is static verification proof metadata for Copy reads
        // and emits no runtime action or ownership transfer.
        cleanup.transfers.iter().any(|t| t.source == source)
            || cleanup.pattern_transfers.iter().any(|t| t.source == source)
            || cleanup.sites.iter().any(|s| s.source == source)
            || cleanup
                .discards
                .iter()
                .any(|d| d.source == source || d.value_source == source)
            || cleanup.pattern_orders.iter().any(|o| o.source == source)
    }

    fn can_extract_bitvec_expr(&self, expr: &Expr, dag_depth: usize, is_root: bool) -> bool {
        if !is_root && self.has_cleanup_hook(expr.source) {
            return bitvec_width(&expr.ty).is_some();
        }
        if dag_depth >= MAX_DAG_DEPTH {
            return !is_root && bitvec_width(&expr.ty).is_some();
        }
        match &expr.kind {
            K::BitConcat(high, low) => {
                let Some(hw) = bitvec_width(&high.ty) else {
                    return false;
                };
                let Some(lw) = bitvec_width(&low.ty) else {
                    return false;
                };
                hw.checked_add(lw)
                    .filter(|&sum| sum <= bits::MAX_BIT_WIDTH)
                    .is_some_and(|sum| bitvec_width(&expr.ty) == Some(sum))
                    && self.can_extract_bitvec_expr(high, dag_depth + 1, false)
                    && self.can_extract_bitvec_expr(low, dag_depth + 1, false)
            }
            K::BitSlice { value, start, end } => {
                let Some(in_w) = bitvec_width(&value.ty) else {
                    return false;
                };
                if *start > *end || *end > in_w as usize {
                    return false;
                }
                let expected = u32::try_from(*end - *start).ok();
                expected.is_some()
                    && bitvec_width(&expr.ty) == expected
                    && self.can_extract_bitvec_expr(value, dag_depth + 1, false)
            }
            K::BitEqual(left, right) if is_root => {
                let Some(lw) = bitvec_width(&left.ty) else {
                    return false;
                };
                let Some(rw) = bitvec_width(&right.ty) else {
                    return false;
                };
                lw == rw
                    && expr.ty == Term::Primitive(P::Bool)
                    && self.can_extract_bitvec_expr(left, dag_depth + 1, false)
                    && self.can_extract_bitvec_expr(right, dag_depth + 1, false)
            }
            K::And(left, right) | K::Or(left, right) => {
                let Some(w) = bitvec_width(&expr.ty) else {
                    return false;
                };
                bitvec_width(&left.ty) == Some(w)
                    && bitvec_width(&right.ty) == Some(w)
                    && self.can_extract_bitvec_expr(left, dag_depth + 1, false)
                    && self.can_extract_bitvec_expr(right, dag_depth + 1, false)
            }
            K::Not(inner) => {
                let Some(w) = bitvec_width(&expr.ty) else {
                    return false;
                };
                bitvec_width(&inner.ty) == Some(w)
                    && self.can_extract_bitvec_expr(inner, dag_depth + 1, false)
            }
            K::BitConvert(inner) => {
                let Some(dst_w) = bitvec_width(&expr.ty) else {
                    return false;
                };
                let Some(src_w) = bitvec_width(&inner.ty) else {
                    return false;
                };
                dst_w >= src_w && self.can_extract_bitvec_expr(inner, dag_depth + 1, false)
            }
            K::Literal(Literal::BitVec { width, words }) if !is_root => {
                bitvec_width(&expr.ty).is_some_and(|w| w as usize == *width)
                    && words.len() == width.div_ceil(32)
            }
            _ => !is_root && bitvec_width(&expr.ty).is_some(),
        }
    }

    pub(super) fn try_lower_bitvec_expr(
        &mut self,
        expr: &Expr,
        depth: usize,
    ) -> Result<Option<Value>, String> {
        if !self.can_extract_bitvec_expr(expr, 0, true) {
            return Ok(None);
        }
        let mut extractor = BitVecExtractor::new();
        let Some(root_id) = extractor.extract(self, expr, depth, 0)? else {
            return Ok(Some(Value::unit()));
        };
        let region = if matches!(expr.kind, K::BitEqual(..)) {
            extractor
                .builder
                .build_bool(root_id)
                .map_err(|e| format!("host BitVec bool region error: {e}"))?
        } else {
            extractor
                .builder
                .build(root_id)
                .map_err(|e| format!("host BitVec region error: {e}"))?
        };
        self.optimize_and_lower_region(&region, &extractor.leaf_values)
            .map(Some)
    }

    fn optimize_and_lower_region(
        &mut self,
        region: &Region,
        leaf_values: &BTreeMap<InputIdentity, Value>,
    ) -> Result<Value, String> {
        let zw = eliminate_zero_width(region)
            .map_err(|e| format!("host BitVec zero-width pass failed: {e}"))?;
        let simplified = simplify_region(&zw.region)
            .map_err(|e| format!("host BitVec rewrite pass failed: {e}"))?;
        let max_w = max_region_width(&simplified.region);
        let non_trivial = non_trivial_op_count(&simplified.region);
        let can_synth = max_w <= MAX_SYNTH_WIDTH
            && non_trivial >= 2
            && simplified.region.nodes.len() <= MAX_SYNTH_NODES
            && simplified.region.inputs.len() <= MAX_SYNTH_INPUTS;

        if can_synth {
            let profile = host_target_profile(self.graph);
            let budget = Budget::with_limits(SYNTH_MAX_STEPS, SYNTH_MAX_CONFLICTS);
            let mut fallback_cache = SynthesisCache::new();
            let outcome = if let Some(cache) = self.bitvec_cache.take() {
                let res = synthesize_with_budget(
                    &simplified.region,
                    profile,
                    profile.default_cost_model(),
                    budget,
                    cache,
                );
                self.bitvec_cache = Some(cache);
                res
            } else {
                synthesize_with_budget(
                    &simplified.region,
                    profile,
                    profile.default_cost_model(),
                    budget,
                    &mut fallback_cache,
                )
            }
            .map_err(|e| format!("host BitVec synthesis failed: {e}"))?;

            return match outcome.implementation {
                ChosenImplementation::Target(target) => {
                    self.emit_target_program(&target, region.output_kind, leaf_values)
                }
                ChosenImplementation::Baseline(baseline) => {
                    self.emit_baseline_program(&baseline, region.output_kind, leaf_values)
                }
            };
        }

        self.emit_region(&simplified.region, leaf_values)
    }

    fn emit_region(
        &mut self,
        region: &Region,
        leaf_values: &BTreeMap<InputIdentity, Value>,
    ) -> Result<Value, String> {
        let mut values: Vec<Value> = Vec::with_capacity(region.nodes.len());
        for node in &region.nodes {
            let val = match node {
                Node::Input { width, identity } => {
                    let val = leaf_values
                        .get(identity)
                        .cloned()
                        .ok_or("host BitVec region input missing")?;
                    if bit_width(&val.ty) != Some(*width) {
                        return Err("host BitVec region input width mismatch".into());
                    }
                    val
                }
                Node::Constant { width, bits } => const_int_value(*width, bits),
                Node::Concat(hi, lo) => {
                    self.bit_concat(values[hi.0].clone(), values[lo.0].clone())?
                }
                Node::Extract {
                    input,
                    start,
                    width,
                } => match start {
                    ExtractStart::Const(lo) => {
                        let start_usize = *lo as usize;
                        self.bit_slice(
                            values[input.0].clone(),
                            start_usize,
                            start_usize + *width as usize,
                        )?
                    }
                    ExtractStart::Dynamic(idx) => self.dynamic_slice_checked(
                        values[input.0].clone(),
                        values[idx.0].clone(),
                        *width,
                    )?,
                },
                Node::And(l, r) => {
                    self.bit_binary("and", values[l.0].clone(), values[r.0].clone())?
                }
                Node::Or(l, r) => {
                    self.bit_binary("or", values[l.0].clone(), values[r.0].clone())?
                }
                Node::Not(inner) => self.emit_bit_not(&values[inner.0])?,
                Node::Equal(l, r) => {
                    let eq =
                        self.bit_binary("icmp eq", values[l.0].clone(), values[r.0].clone())?;
                    Value {
                        ty: Ty::Int(1, false),
                        text: eq.text,
                    }
                }
                Node::ZeroExtend { input, width } => {
                    self.emit_zero_extend(&values[input.0], *width)?
                }
            };
            values.push(val);
        }
        finalize_root_value(values[region.root.0].clone(), region.output_kind)
    }

    pub(super) fn emit_target_program(
        &mut self,
        program: &TargetProgram,
        output_kind: OutputKind,
        leaf_values: &BTreeMap<InputIdentity, Value>,
    ) -> Result<Value, String> {
        let mut regs: Vec<Value> = Vec::with_capacity(program.instructions.len());
        for inst in &program.instructions {
            let val = match &inst.op {
                TargetOp::ZeroWidthEmpty { .. } => empty_bits(),
                TargetOp::Const { width, bits } => const_int_value(*width, bits),
                TargetOp::Input { width, identity } => {
                    let val = leaf_values
                        .get(identity)
                        .cloned()
                        .ok_or("host BitVec target input missing")?;
                    if bit_width(&val.ty) != Some(*width) {
                        return Err("host BitVec target input width mismatch".into());
                    }
                    val
                }
                TargetOp::Copy { src, .. } => regs[src.0].clone(),
                TargetOp::And { lhs, rhs, .. } => {
                    self.bit_binary("and", regs[lhs.0].clone(), regs[rhs.0].clone())?
                }
                TargetOp::Or { lhs, rhs, .. } => {
                    self.bit_binary("or", regs[lhs.0].clone(), regs[rhs.0].clone())?
                }
                TargetOp::Xor { lhs, rhs, .. } => {
                    self.bit_binary("xor", regs[lhs.0].clone(), regs[rhs.0].clone())?
                }
                TargetOp::Not { src, .. } => self.emit_bit_not(&regs[src.0])?,
                TargetOp::AndNot { lhs, inverted, .. } => {
                    let inv = self.emit_bit_not(&regs[inverted.0])?;
                    self.bit_binary("and", regs[lhs.0].clone(), inv)?
                }
                TargetOp::OrNot { lhs, inverted, .. } => {
                    let inv = self.emit_bit_not(&regs[inverted.0])?;
                    self.bit_binary("or", regs[lhs.0].clone(), inv)?
                }
                TargetOp::Equal { lhs, rhs, .. } => {
                    let eq =
                        self.bit_binary("icmp eq", regs[lhs.0].clone(), regs[rhs.0].clone())?;
                    Value {
                        ty: Ty::Int(1, false),
                        text: eq.text,
                    }
                }
                TargetOp::Concat { high, low, .. } => {
                    self.bit_concat(regs[high.0].clone(), regs[low.0].clone())?
                }
                TargetOp::ExtractConst {
                    input,
                    start,
                    width,
                } => {
                    let start_usize = *start as usize;
                    self.bit_slice(
                        regs[input.0].clone(),
                        start_usize,
                        start_usize + (*width as usize),
                    )?
                }
                TargetOp::ExtractDynamic {
                    input,
                    start,
                    width,
                    ..
                } => self.dynamic_slice_checked(
                    regs[input.0].clone(),
                    regs[start.0].clone(),
                    *width,
                )?,
                TargetOp::ZeroExtend {
                    input,
                    target_width,
                } => self.emit_zero_extend(&regs[input.0], *target_width)?,
                TargetOp::Bswap16 { input } => self.emit_bswap(&regs[input.0], 16)?,
                TargetOp::Bswap32 { input } => self.emit_bswap(&regs[input.0], 32)?,
                TargetOp::Bswap64 { input } => self.emit_bswap(&regs[input.0], 64)?,
                TargetOp::RotrConst {
                    input,
                    width,
                    amount,
                } => self.emit_const_rotate(&regs[input.0], *width, *amount, true)?,
                TargetOp::RotlConst {
                    input,
                    width,
                    amount,
                } => self.emit_const_rotate(&regs[input.0], *width, *amount, false)?,
                TargetOp::ShlConst {
                    input,
                    width,
                    amount,
                } => self.emit_const_shift(&regs[input.0], *width, *amount, false)?,
                TargetOp::LshrConst {
                    input,
                    width,
                    amount,
                } => self.emit_const_shift(&regs[input.0], *width, *amount, true)?,
                TargetOp::Shl {
                    input,
                    width,
                    shift,
                } => self.emit_dynamic_shift(&regs[input.0], *width, &regs[shift.0], false)?,
                TargetOp::Lshr {
                    input,
                    width,
                    shift,
                } => self.emit_dynamic_shift(&regs[input.0], *width, &regs[shift.0], true)?,
                TargetOp::Rotr {
                    input,
                    width,
                    shift,
                } => self.emit_dynamic_rotate(&regs[input.0], *width, &regs[shift.0], true)?,
                TargetOp::Rotl {
                    input,
                    width,
                    shift,
                } => self.emit_dynamic_rotate(&regs[input.0], *width, &regs[shift.0], false)?,
            };
            regs.push(val);
        }
        finalize_root_value(regs[program.root.0].clone(), output_kind)
    }

    pub(super) fn emit_baseline_program(
        &mut self,
        program: &BaselineProgram,
        output_kind: OutputKind,
        leaf_values: &BTreeMap<InputIdentity, Value>,
    ) -> Result<Value, String> {
        if program.word_bits != 64 {
            return Err(format!(
                "host baseline emission requires 64-bit machine words, got {}",
                program.word_bits
            ));
        }
        let mut regs: Vec<Vec<Value>> = Vec::with_capacity(program.reg_widths.len());
        for _ in 0..program.reg_widths.len() {
            regs.push(Vec::new());
        }

        for op in &program.ops {
            match op {
                BaselineOp::ZeroWidthEmpty { dst } => {
                    regs[dst.0] = Vec::new();
                }
                BaselineOp::LoadConst { dst, words, .. } => {
                    regs[dst.0] = words.iter().map(|&w| const_i64_value(w)).collect();
                }
                BaselineOp::AcquireInput {
                    dst,
                    identity,
                    width,
                } => {
                    let val = leaf_values
                        .get(identity)
                        .cloned()
                        .ok_or("host BitVec baseline input missing")?;
                    if bit_width(&val.ty) != Some(*width) {
                        return Err("host BitVec baseline input width mismatch".into());
                    }
                    regs[dst.0] = self.unpack_to_i64_words(&val, *width)?;
                }
                BaselineOp::WordAnd {
                    dst,
                    lhs,
                    rhs,
                    num_words,
                } => {
                    regs[dst.0] = self.word_bitwise("and", &regs[lhs.0], &regs[rhs.0], *num_words);
                }
                BaselineOp::WordOr {
                    dst,
                    lhs,
                    rhs,
                    num_words,
                } => {
                    regs[dst.0] = self.word_bitwise("or", &regs[lhs.0], &regs[rhs.0], *num_words);
                }
                BaselineOp::WordNot {
                    dst,
                    src,
                    num_words,
                    tail_mask,
                } => {
                    let mut out = Vec::with_capacity(*num_words);
                    for i in 0..*num_words {
                        let inv = self.instruction(
                            Ty::Int(64, false),
                            format!("xor i64 {}, -1", regs[src.0][i].text),
                        );
                        out.push(inv);
                    }
                    self.mask_tail_word(&mut out, *tail_mask);
                    regs[dst.0] = out;
                }
                BaselineOp::WordEqual {
                    dst,
                    lhs,
                    rhs,
                    num_words,
                } => {
                    let mut acc = Value {
                        ty: Ty::Bool,
                        text: "true".into(),
                    };
                    for i in 0..*num_words {
                        let cmp = self.instruction(
                            Ty::Bool,
                            format!(
                                "icmp eq i64 {}, {}",
                                regs[lhs.0][i].text, regs[rhs.0][i].text
                            ),
                        );
                        acc = if i == 0 {
                            cmp
                        } else {
                            self.instruction(Ty::Bool, format!("and i1 {}, {}", acc.text, cmp.text))
                        };
                    }
                    let word = self
                        .instruction(Ty::Int(64, false), format!("zext i1 {} to i64", acc.text));
                    regs[dst.0] = vec![word];
                }
                BaselineOp::WordZeroExtend {
                    dst,
                    src,
                    src_words,
                    dst_words,
                } => {
                    let mut out = Vec::with_capacity(*dst_words);
                    out.extend(regs[src.0][..*src_words].iter().cloned());
                    while out.len() < *dst_words {
                        out.push(const_i64_value(0));
                    }
                    regs[dst.0] = out;
                }
                BaselineOp::WordConcatAligned {
                    dst,
                    high,
                    low,
                    low_words,
                    high_words,
                } => {
                    let mut out = Vec::with_capacity(*low_words + *high_words);
                    out.extend(regs[low.0][..*low_words].iter().cloned());
                    out.extend(regs[high.0][..*high_words].iter().cloned());
                    regs[dst.0] = out;
                }
                BaselineOp::WordConcatShifted {
                    dst,
                    high,
                    low,
                    low_words,
                    high_words,
                    shift,
                    inv_shift,
                    total_words,
                    tail_mask,
                } => {
                    let mut out = vec![const_i64_value(0); *total_words];
                    for i in 0..*low_words {
                        out[i] = regs[low.0][i].clone();
                    }
                    let word_off = *low_words - 1;
                    for j in 0..*high_words {
                        let hw = &regs[high.0][j];
                        let lo_part = self.instruction(
                            Ty::Int(64, false),
                            format!("shl i64 {}, {shift}", hw.text),
                        );
                        out[word_off + j] = self.instruction(
                            Ty::Int(64, false),
                            format!("or i64 {}, {}", out[word_off + j].text, lo_part.text),
                        );
                        if word_off + j + 1 < *total_words {
                            let hi_part = self.instruction(
                                Ty::Int(64, false),
                                format!("lshr i64 {}, {inv_shift}", hw.text),
                            );
                            out[word_off + j + 1] = self.instruction(
                                Ty::Int(64, false),
                                format!("or i64 {}, {}", out[word_off + j + 1].text, hi_part.text),
                            );
                        }
                    }
                    self.mask_tail_word(&mut out, *tail_mask);
                    regs[dst.0] = out;
                }
                BaselineOp::WordExtractSingleWord {
                    dst,
                    src,
                    word_idx,
                    bit_shift,
                    mask,
                } => {
                    let mut word = regs[src.0][*word_idx].clone();
                    if *bit_shift != 0 {
                        word = self.instruction(
                            Ty::Int(64, false),
                            format!("lshr i64 {}, {bit_shift}", word.text),
                        );
                    }
                    if *mask != u64::MAX {
                        word = self.instruction(
                            Ty::Int(64, false),
                            format!("and i64 {}, {mask}", word.text),
                        );
                    }
                    regs[dst.0] = vec![word];
                }
                BaselineOp::WordExtractAligned {
                    dst,
                    src,
                    word_offset,
                    dst_words,
                    tail_mask,
                } => {
                    let mut out = regs[src.0][*word_offset..*word_offset + *dst_words].to_vec();
                    self.mask_tail_word(&mut out, *tail_mask);
                    regs[dst.0] = out;
                }
                BaselineOp::WordExtractCrossWord {
                    dst,
                    src,
                    word_offset,
                    shift,
                    inv_shift,
                    dst_words,
                    tail_mask,
                } => {
                    let src_words = &regs[src.0];
                    let mut out = Vec::with_capacity(*dst_words);
                    for j in 0..*dst_words {
                        let lo = self.instruction(
                            Ty::Int(64, false),
                            format!("lshr i64 {}, {shift}", src_words[*word_offset + j].text),
                        );
                        let merged = if *word_offset + j + 1 < src_words.len() {
                            let hi = self.instruction(
                                Ty::Int(64, false),
                                format!(
                                    "shl i64 {}, {inv_shift}",
                                    src_words[*word_offset + j + 1].text
                                ),
                            );
                            self.instruction(
                                Ty::Int(64, false),
                                format!("or i64 {}, {}", lo.text, hi.text),
                            )
                        } else {
                            lo
                        };
                        out.push(merged);
                    }
                    self.mask_tail_word(&mut out, *tail_mask);
                    regs[dst.0] = out;
                }
                BaselineOp::WordExtractDynamic {
                    dst,
                    src,
                    start_reg,
                    src_width,
                    width,
                    ..
                } => {
                    let src_val = self.pack_from_i64_words(&regs[src.0], *src_width);
                    let start_width = program.reg_widths[start_reg.0];
                    let start_val = self.pack_from_i64_words(&regs[start_reg.0], start_width);
                    let sliced = self.dynamic_slice_checked(src_val, start_val, *width)?;
                    regs[dst.0] = self.unpack_to_i64_words(&sliced, *width)?;
                }
            }
        }

        let root_val = self.pack_from_i64_words(&regs[program.output.0], program.output_width);
        finalize_root_value(root_val, output_kind)
    }

    fn emit_bit_not(&mut self, value: &Value) -> Result<Value, String> {
        let width = bit_width(&value.ty).ok_or("host bit not input has no width")?;
        if width == 0 {
            return Ok(empty_bits());
        }
        Ok(self.instruction(
            Ty::Int(width, false),
            format!("xor i{width} {}, -1", value.text),
        ))
    }

    fn emit_zero_extend(&mut self, value: &Value, target_width: u32) -> Result<Value, String> {
        if target_width == 0 {
            return Ok(empty_bits());
        }
        let src_w = bit_width(&value.ty).ok_or("host zext input has no width")?;
        if src_w == target_width {
            return Ok(Value {
                ty: Ty::Int(target_width, false),
                text: value.text.clone(),
            });
        }
        if src_w == 0 {
            return Ok(Value {
                ty: Ty::Int(target_width, false),
                text: "0".into(),
            });
        }
        if src_w > target_width {
            return Err("host zext cannot narrow bit width".into());
        }
        Ok(self.instruction(
            Ty::Int(target_width, false),
            format!("zext i{src_w} {} to i{target_width}", value.text),
        ))
    }

    fn emit_bswap(&mut self, value: &Value, width: u32) -> Result<Value, String> {
        let declaration = format!("declare i{width} @llvm.bswap.i{width}(i{width})");
        self.declarations.insert(declaration);
        Ok(self.instruction(
            Ty::Int(width, false),
            format!(
                "call i{width} @llvm.bswap.i{width}(i{width} {})",
                value.text
            ),
        ))
    }

    fn emit_const_rotate(
        &mut self,
        value: &Value,
        width: u32,
        amount: u32,
        is_right: bool,
    ) -> Result<Value, String> {
        if width == 0 {
            return Ok(empty_bits());
        }
        if width == 1 {
            return Ok(value.clone());
        }
        let k = amount % width;
        if k == 0 {
            return Ok(value.clone());
        }
        if matches!(width, 8 | 16 | 32 | 64 | 128) {
            let intrinsic = if is_right { "fshr" } else { "fshl" };
            self.declarations.insert(format!(
                "declare i{width} @llvm.{intrinsic}.i{width}(i{width}, i{width}, i{width})"
            ));
            return Ok(self.instruction(
                Ty::Int(width, false),
                format!(
                    "call i{width} @llvm.{intrinsic}.i{width}(i{width} {0}, i{width} {0}, i{width} {k})",
                    value.text
                ),
            ));
        }
        let inv_k = width - k;
        let (op_a, sh_a, op_b, sh_b) = if is_right {
            ("lshr", k, "shl", inv_k)
        } else {
            ("shl", k, "lshr", inv_k)
        };
        let ty = Ty::Int(width, false);
        let part_a = self.instruction(
            ty.clone(),
            format!("{op_a} i{width} {}, {sh_a}", value.text),
        );
        let part_b = self.instruction(
            ty.clone(),
            format!("{op_b} i{width} {}, {sh_b}", value.text),
        );
        Ok(self.instruction(ty, format!("or i{width} {}, {}", part_a.text, part_b.text)))
    }

    fn emit_const_shift(
        &mut self,
        value: &Value,
        width: u32,
        amount: u32,
        is_right: bool,
    ) -> Result<Value, String> {
        if width == 0 {
            self.guard(&Value {
                ty: Ty::Bool,
                text: "false".into(),
            })?;
            return Ok(empty_bits());
        }
        if amount >= width {
            self.guard(&Value {
                ty: Ty::Bool,
                text: "false".into(),
            })?;
            return Ok(Value {
                ty: Ty::Int(width, false),
                text: "0".into(),
            });
        }
        if amount == 0 {
            return Ok(value.clone());
        }
        let op = if is_right { "lshr" } else { "shl" };
        Ok(self.instruction(
            Ty::Int(width, false),
            format!("{op} i{width} {}, {amount}", value.text),
        ))
    }

    fn check_and_convert_shift_amount(
        &mut self,
        shift: &Value,
        target_width: u32,
    ) -> Result<Value, String> {
        let sw = bit_width(&shift.ty).ok_or("host shift amount has no width")?;
        if sw == 0 {
            return Err("host shift amount cannot be zero-width".into());
        }
        let cmp_w = sw.max(target_width).max(64);
        let shift_wide = if sw < cmp_w {
            self.instruction(
                Ty::Int(cmp_w, false),
                format!("zext i{sw} {} to i{cmp_w}", shift.text),
            )
        } else {
            shift.clone()
        };
        let valid = self.instruction(
            Ty::Bool,
            format!("icmp ult i{cmp_w} {}, {target_width}", shift_wide.text),
        );
        self.guard(&valid)?;
        if sw == target_width {
            Ok(shift.clone())
        } else if sw < target_width {
            Ok(self.instruction(
                Ty::Int(target_width, false),
                format!("zext i{sw} {} to i{target_width}", shift.text),
            ))
        } else {
            Ok(self.instruction(
                Ty::Int(target_width, false),
                format!("trunc i{sw} {} to i{target_width}", shift.text),
            ))
        }
    }

    fn emit_dynamic_shift(
        &mut self,
        value: &Value,
        width: u32,
        shift: &Value,
        is_right: bool,
    ) -> Result<Value, String> {
        if width == 0 {
            self.guard(&Value {
                ty: Ty::Bool,
                text: "false".into(),
            })?;
            return Ok(empty_bits());
        }
        let shift_val = self.check_and_convert_shift_amount(shift, width)?;
        let op = if is_right { "lshr" } else { "shl" };
        Ok(self.instruction(
            Ty::Int(width, false),
            format!("{op} i{width} {}, {}", value.text, shift_val.text),
        ))
    }

    fn emit_dynamic_rotate(
        &mut self,
        value: &Value,
        width: u32,
        shift: &Value,
        is_right: bool,
    ) -> Result<Value, String> {
        if width == 0 {
            self.guard(&Value {
                ty: Ty::Bool,
                text: "false".into(),
            })?;
            return Ok(empty_bits());
        }
        let shift_val = self.check_and_convert_shift_amount(shift, width)?;
        if width == 1 {
            return Ok(value.clone());
        }
        if matches!(width, 8 | 16 | 32 | 64 | 128) {
            let intrinsic = if is_right { "fshr" } else { "fshl" };
            self.declarations.insert(format!(
                "declare i{width} @llvm.{intrinsic}.i{width}(i{width}, i{width}, i{width})"
            ));
            return Ok(self.instruction(
                Ty::Int(width, false),
                format!(
                    "call i{width} @llvm.{intrinsic}.i{width}(i{width} {0}, i{width} {0}, i{width} {1})",
                    value.text, shift_val.text
                ),
            ));
        }
        let ty = Ty::Int(width, false);
        let result_slot = self.slot(ty.clone());
        let is_zero = self.instruction(Ty::Bool, format!("icmp eq i{width} {}, 0", shift_val.text));
        let zero_lbl = self.fresh();
        let nz_lbl = self.fresh();
        let end_lbl = self.fresh();
        self.line(format!(
            "br i1 {}, label %{zero_lbl}, label %{nz_lbl}",
            is_zero.text
        ));
        self.label(&zero_lbl);
        self.store(&result_slot, value)?;
        self.branch(&end_lbl);
        self.label(&nz_lbl);
        let inv = self.instruction(
            ty.clone(),
            format!("sub i{width} {width}, {}", shift_val.text),
        );
        let (op_a, sh_a, op_b, sh_b) = if is_right {
            ("lshr", shift_val.text.as_str(), "shl", inv.text.as_str())
        } else {
            ("shl", shift_val.text.as_str(), "lshr", inv.text.as_str())
        };
        let part_a = self.instruction(
            ty.clone(),
            format!("{op_a} i{width} {}, {sh_a}", value.text),
        );
        let part_b = self.instruction(
            ty.clone(),
            format!("{op_b} i{width} {}, {sh_b}", value.text),
        );
        let combined =
            self.instruction(ty, format!("or i{width} {}, {}", part_a.text, part_b.text));
        self.store(&result_slot, &combined)?;
        self.branch(&end_lbl);
        self.label(&end_lbl);
        Ok(self.load(&result_slot))
    }

    fn dynamic_slice_checked(
        &mut self,
        input: Value,
        start: Value,
        slice_width: u32,
    ) -> Result<Value, String> {
        let in_w = bit_width(&input.ty).ok_or("host dynamic slice input has no width")?;
        if slice_width > in_w {
            return Err("host bit window needs a in-range width".into());
        }
        let start_w = bit_width(&start.ty).ok_or("host dynamic slice index has no width")?;
        if start_w == 0 {
            return Err("host dynamic slice index cannot be zero-width".into());
        }
        let start_i128 = if start_w > 128 {
            let valid = self.instruction(
                Ty::Bool,
                format!("icmp ule i{start_w} {}, {}", start.text, in_w - slice_width),
            );
            self.guard(&valid)?;
            self.instruction(
                Ty::Int(128, false),
                format!("trunc i{start_w} {} to i128", start.text),
            )
        } else {
            start
        };
        self.bit_window(input, start_i128, None, slice_width)
    }

    fn word_bitwise(
        &mut self,
        op: &str,
        lhs: &[Value],
        rhs: &[Value],
        num_words: usize,
    ) -> Vec<Value> {
        let mut out = Vec::with_capacity(num_words);
        for i in 0..num_words {
            out.push(self.instruction(
                Ty::Int(64, false),
                format!("{op} i64 {}, {}", lhs[i].text, rhs[i].text),
            ));
        }
        out
    }

    fn mask_tail_word(&mut self, words: &mut [Value], tail_mask: u64) {
        if tail_mask != u64::MAX
            && let Some(last) = words.last_mut()
        {
            *last = self.instruction(
                Ty::Int(64, false),
                format!("and i64 {}, {tail_mask}", last.text),
            );
        }
    }

    fn unpack_to_i64_words(&mut self, value: &Value, width: u32) -> Result<Vec<Value>, String> {
        if width == 0 {
            return Ok(Vec::new());
        }
        if width < 64 {
            let ext = self.instruction(
                Ty::Int(64, false),
                format!("zext i{width} {} to i64", value.text),
            );
            return Ok(vec![ext]);
        }
        if width == 64 {
            return Ok(vec![Value {
                ty: Ty::Int(64, false),
                text: value.text.clone(),
            }]);
        }
        let num_words = width.div_ceil(64) as usize;
        let mut out = Vec::with_capacity(num_words);
        for j in 0..num_words {
            let shifted = if j == 0 {
                value.clone()
            } else {
                self.instruction(
                    Ty::Int(width, false),
                    format!("lshr i{width} {}, {}", value.text, j * 64),
                )
            };
            let word = self.instruction(
                Ty::Int(64, false),
                format!("trunc i{width} {} to i64", shifted.text),
            );
            out.push(word);
        }
        Ok(out)
    }

    fn pack_from_i64_words(&mut self, words: &[Value], width: u32) -> Value {
        if width == 0 {
            return empty_bits();
        }
        if width < 64 {
            return self.instruction(
                Ty::Int(width, false),
                format!("trunc i64 {} to i{width}", words[0].text),
            );
        }
        if width == 64 {
            return words[0].clone();
        }
        let target = Ty::Int(width, false);
        let mut acc = self.instruction(
            target.clone(),
            format!("zext i64 {} to i{width}", words[0].text),
        );
        for (j, word) in words.iter().enumerate().skip(1) {
            let ext = self.instruction(
                target.clone(),
                format!("zext i64 {} to i{width}", word.text),
            );
            let shifted = self.instruction(
                target.clone(),
                format!("shl i{width} {}, {}", ext.text, j * 64),
            );
            acc = self.instruction(
                target.clone(),
                format!("or i{width} {}, {}", acc.text, shifted.text),
            );
        }
        acc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitvec_opt::{FallbackReason, ProofStatus, lower_to_baseline};

    fn make_test_emitter<'a>(
        graph: &'a iir::Package,
        lambdas: &'a BTreeMap<usize, usize>,
        template_identities: &'a callables::TemplateIdentities<'a>,
        declarations: &'a mut BTreeSet<String>,
        text_constants: &'a mut strings::TextConstants,
        bitvec_cache: &'a mut SynthesisCache,
        fuel: &'a mut usize,
    ) -> Emitter<'a> {
        Emitter {
            graph,
            cleanup: None,
            schedules: None,
            resource_unwinds: vec![],
            checked: false,
            function: None,
            closure_body: None,
            failure_result: None,
            effects: vec![],
            effect_resume: None,
            native_return: None,
            loop_destinations: Default::default(),
            outlined_return: None,
            outlined_loops: BTreeMap::new(),
            lambdas,
            template_identities,
            locals: BTreeMap::new(),
            live: BTreeMap::new(),
            temporaries: BTreeMap::new(),
            temporary_live: BTreeMap::new(),
            prepared: BTreeMap::new(),
            receipt_paths: vec![],
            pattern_orders: BTreeMap::new(),
            temporary_fields: BTreeMap::new(),
            loops: BTreeMap::new(),
            prologue: String::new(),
            body: String::new(),
            declarations,
            text_constants,
            attr_table: None,
            bitvec_cache: Some(bitvec_cache),
            serial: 0,
            terminated: false,
            fuel,
        }
    }

    fn run_native_checks(ir: &str) {
        for optimization in [
            crate::host::OptimizationLevel::O0,
            crate::host::OptimizationLevel::O3,
        ] {
            let toolchain = crate::host::Toolchain {
                optimization,
                ..Default::default()
            };
            let output = toolchain.execute_test_ir(ir).unwrap();
            assert!(output.status.success(), "{optimization:?}:\n{ir}");
        }
    }

    #[test]
    fn emits_bswap_xor_rotate_shift_and_andnot_targets_and_executes_natively() {
        let graph = iir::Package::default();
        let lambdas = BTreeMap::new();
        let template_identities = callables::TemplateIdentities::default();
        let mut declarations = BTreeSet::new();
        let mut text_constants = strings::TextConstants::default();
        let mut cache = SynthesisCache::new();
        let mut fuel = 100_000;

        // 1. bswap16, bswap32, bswap64
        let make_bswap = |bits: u32| {
            let mut b = RegionBuilder::new();
            let x = b.pure_input("x", 0, bits);
            let mut acc = b.extract_const(x, 0, 8).unwrap();
            for byte in 1..(bits / 8) {
                let sl = b.extract_const(x, byte * 8, 8).unwrap();
                acc = b.concat(acc, sl).unwrap();
            }
            b.build(acc).unwrap()
        };

        // 2. XOR via De Morgan / cofactor: (a & !b) | (!a & b)
        let xor8_region = {
            let mut b = RegionBuilder::new();
            let a = b.pure_input("a", 0, 8);
            let b_in = b.pure_input("b", 1, 8);
            let na = b.not(a).unwrap();
            let nb = b.not(b_in).unwrap();
            let l = b.and(a, nb).unwrap();
            let r = b.and(na, b_in).unwrap();
            let res = b.or(l, r).unwrap();
            b.build(res).unwrap()
        };

        // 3. Rotate right by 5 on 16 bits
        let rotr16_region = {
            let mut b = RegionBuilder::new();
            let x = b.pure_input("x", 0, 16);
            let lo = b.extract_const(x, 0, 5).unwrap();
            let hi = b.extract_const(x, 5, 11).unwrap();
            let res = b.concat(lo, hi).unwrap();
            b.build(res).unwrap()
        };

        // 4. ShlConst on 32 bits: x[0..24] ++ 0_8
        let shl32_region = {
            let mut b = RegionBuilder::new();
            let x = b.pure_input("x", 0, 32);
            let lo = b.extract_const(x, 0, 24).unwrap();
            let z = b.const_u64(8, 0).unwrap();
            let res = b.concat(lo, z).unwrap();
            b.build(res).unwrap()
        };

        // 5. LshrConst on 32 bits: zext(x[8..32], 32)
        let lshr32_region = {
            let mut b = RegionBuilder::new();
            let x = b.pure_input("x", 0, 32);
            let hi = b.extract_const(x, 8, 24).unwrap();
            let res = b.zero_extend(hi, 32).unwrap();
            b.build(res).unwrap()
        };

        // 6. AndNot on 16 bits: a & !b
        let andn16_region = {
            let mut b = RegionBuilder::new();
            let a = b.pure_input("a", 0, 16);
            let b_in = b.pure_input("b", 1, 16);
            let nb = b.not(b_in).unwrap();
            let res = b.and(a, nb).unwrap();
            b.build(res).unwrap()
        };

        let mut helper_fns = String::new();
        {
            let mut emit_fn =
                |name: &str,
                 params: &[(&str, Ty)],
                 ret_ty: Ty,
                 region: &Region,
                 profile: TargetProfile,
                 leaves: BTreeMap<InputIdentity, Value>| {
                    let outcome = synthesize_with_budget(
                        region,
                        profile,
                        profile.default_cost_model(),
                        Budget::with_limits(500_000, 50_000),
                        &mut cache,
                    )
                    .unwrap();
                    assert!(
                        matches!(outcome.status, ProofStatus::Verified(_)),
                        "{name}: {}",
                        outcome.status
                    );
                    let ChosenImplementation::Target(target) = outcome.implementation else {
                        panic!("{name} expected Target implementation");
                    };
                    let mut e = make_test_emitter(
                        &graph,
                        &lambdas,
                        &template_identities,
                        &mut declarations,
                        &mut text_constants,
                        &mut cache,
                        &mut fuel,
                    );
                    let out = e
                        .emit_target_program(&target, region.output_kind, &leaves)
                        .unwrap();
                    let sig_params: Vec<String> = params
                        .iter()
                        .map(|(p, t)| format!("{} {p}", t.llvm()))
                        .collect();
                    helper_fns.push_str(&format!(
                        "define {} @{name}({}) {{\nentry:\n{}{}  ret {} {}\n}}\n",
                        ret_ty.llvm(),
                        sig_params.join(", "),
                        e.prologue,
                        e.body,
                        out.ty.llvm(),
                        out.text
                    ));
                };

            let b16 = make_bswap(16);
            emit_fn(
                "fn_bswap16",
                &[("%x", Ty::Int(16, false))],
                Ty::Int(16, false),
                &b16,
                TargetProfile::X86_64BmiBswap,
                BTreeMap::from([(
                    InputIdentity::pure("x", 0),
                    Value {
                        ty: Ty::Int(16, false),
                        text: "%x".into(),
                    },
                )]),
            );

            let b32 = make_bswap(32);
            emit_fn(
                "fn_bswap32",
                &[("%x", Ty::Int(32, false))],
                Ty::Int(32, false),
                &b32,
                TargetProfile::X86_64BmiBswap,
                BTreeMap::from([(
                    InputIdentity::pure("x", 0),
                    Value {
                        ty: Ty::Int(32, false),
                        text: "%x".into(),
                    },
                )]),
            );

            let b64 = make_bswap(64);
            emit_fn(
                "fn_bswap64",
                &[("%x", Ty::Int(64, false))],
                Ty::Int(64, false),
                &b64,
                TargetProfile::X86_64BmiBswap,
                BTreeMap::from([(
                    InputIdentity::pure("x", 0),
                    Value {
                        ty: Ty::Int(64, false),
                        text: "%x".into(),
                    },
                )]),
            );

            emit_fn(
                "fn_xor8",
                &[("%a", Ty::Int(8, false)), ("%b", Ty::Int(8, false))],
                Ty::Int(8, false),
                &xor8_region,
                TargetProfile::X86_64BmiBswap,
                BTreeMap::from([
                    (
                        InputIdentity::pure("a", 0),
                        Value {
                            ty: Ty::Int(8, false),
                            text: "%a".into(),
                        },
                    ),
                    (
                        InputIdentity::pure("b", 1),
                        Value {
                            ty: Ty::Int(8, false),
                            text: "%b".into(),
                        },
                    ),
                ]),
            );

            emit_fn(
                "fn_rotr16",
                &[("%x", Ty::Int(16, false))],
                Ty::Int(16, false),
                &rotr16_region,
                TargetProfile::X86_64BmiBswap,
                BTreeMap::from([(
                    InputIdentity::pure("x", 0),
                    Value {
                        ty: Ty::Int(16, false),
                        text: "%x".into(),
                    },
                )]),
            );

            emit_fn(
                "fn_shl32",
                &[("%x", Ty::Int(32, false))],
                Ty::Int(32, false),
                &shl32_region,
                TargetProfile::X86_64BmiBswap,
                BTreeMap::from([(
                    InputIdentity::pure("x", 0),
                    Value {
                        ty: Ty::Int(32, false),
                        text: "%x".into(),
                    },
                )]),
            );

            emit_fn(
                "fn_lshr32",
                &[("%x", Ty::Int(32, false))],
                Ty::Int(32, false),
                &lshr32_region,
                TargetProfile::X86_64BmiBswap,
                BTreeMap::from([(
                    InputIdentity::pure("x", 0),
                    Value {
                        ty: Ty::Int(32, false),
                        text: "%x".into(),
                    },
                )]),
            );

            emit_fn(
                "fn_andn16",
                &[("%a", Ty::Int(16, false)), ("%b", Ty::Int(16, false))],
                Ty::Int(16, false),
                &andn16_region,
                TargetProfile::X86_64BmiBswap,
                BTreeMap::from([
                    (
                        InputIdentity::pure("a", 0),
                        Value {
                            ty: Ty::Int(16, false),
                            text: "%a".into(),
                        },
                    ),
                    (
                        InputIdentity::pure("b", 1),
                        Value {
                            ty: Ty::Int(16, false),
                            text: "%b".into(),
                        },
                    ),
                ]),
            );
        }

        // Second XOR synthesis hits the canonical SynthesisCache.
        let cache_len_before = cache.len();
        let hit = synthesize_with_budget(
            &xor8_region,
            TargetProfile::X86_64BmiBswap,
            TargetProfile::X86_64BmiBswap.default_cost_model(),
            Budget::with_limits(500_000, 50_000),
            &mut cache,
        )
        .unwrap();
        assert!(hit.stats.cache_hit);
        assert_eq!(hit.stats.smt_queries, 0);
        assert_eq!(cache.len(), cache_len_before);

        // Also explicitly test RotlConst and non-power-of-2 dynamic/const rotate on i12.
        let mut e_rotl = make_test_emitter(
            &graph,
            &lambdas,
            &template_identities,
            &mut declarations,
            &mut text_constants,
            &mut cache,
            &mut fuel,
        );
        let rotl16_prog = TargetProgram {
            profile: TargetProfile::X86_64BmiBswap,
            declared_inputs: BTreeMap::from([(InputIdentity::pure("x", 0), 16)]),
            domain: bitvec_opt::DomainConstraint::True,
            boundary: bitvec_opt::BehaviorBoundary::default(),
            instructions: vec![
                bitvec_opt::TargetInst {
                    dst: bitvec_opt::TargetReg(0),
                    width: 16,
                    op: TargetOp::Input {
                        width: 16,
                        identity: InputIdentity::pure("x", 0),
                    },
                },
                bitvec_opt::TargetInst {
                    dst: bitvec_opt::TargetReg(1),
                    width: 16,
                    op: TargetOp::RotlConst {
                        input: bitvec_opt::TargetReg(0),
                        width: 16,
                        amount: 4,
                    },
                },
            ],
            root: bitvec_opt::TargetReg(1),
            output_width: 16,
            justifications: vec![],
        };
        let rotl_out = e_rotl
            .emit_target_program(
                &rotl16_prog,
                OutputKind::BitVec(16),
                &BTreeMap::from([(
                    InputIdentity::pure("x", 0),
                    Value {
                        ty: Ty::Int(16, false),
                        text: "%x".into(),
                    },
                )]),
            )
            .unwrap();
        helper_fns.push_str(&format!(
            "define i16 @fn_rotl16(i16 %x) {{\nentry:\n{}{}  ret i16 {}\n}}\n",
            e_rotl.prologue, e_rotl.body, rotl_out.text
        ));

        assert!(helper_fns.contains("@llvm.bswap.i16"), "{helper_fns}");
        assert!(helper_fns.contains("@llvm.bswap.i32"), "{helper_fns}");
        assert!(helper_fns.contains("@llvm.bswap.i64"), "{helper_fns}");
        assert!(helper_fns.contains("xor i8"), "{helper_fns}");
        assert!(helper_fns.contains("@llvm.fshr.i16"), "{helper_fns}");
        assert!(helper_fns.contains("@llvm.fshl.i16"), "{helper_fns}");
        assert!(helper_fns.contains("shl i32"), "{helper_fns}");
        assert!(helper_fns.contains("lshr i32"), "{helper_fns}");

        let decls: String = declarations.iter().map(|d| format!("{d}\n")).collect();
        let b16_exp = 0x1234_u16.swap_bytes();
        let b32_exp = 0x1234_5678_u32.swap_bytes();
        let b64_in = 0x0123_4567_89AB_CDEF_u64 as i64;
        let b64_exp = 0x0123_4567_89AB_CDEF_u64.swap_bytes() as i64;
        let xor_exp = 0xAF_u8 ^ 0x3C_u8;
        let rotr_exp = 0x1234_u16.rotate_right(5);
        let rotl_exp = 0x1234_u16.rotate_left(4);
        let shl_exp = 0x0012_3456_u32 << 8;
        let lshr_exp = 0x1234_5678_u32 >> 8;
        let andn_exp = 0xFFFF_u16 & !0x0F0F_u16;

        let main_body = format!(
            r#"
define i32 @main() {{
entry:
  %b16 = call i16 @fn_bswap16(i16 {b16_in})
  %ok16 = icmp eq i16 %b16, {b16_exp}
  br i1 %ok16, label %c1, label %fail
c1:
  %b32 = call i32 @fn_bswap32(i32 {b32_in})
  %ok32 = icmp eq i32 %b32, {b32_exp}
  br i1 %ok32, label %c2, label %fail
c2:
  %b64 = call i64 @fn_bswap64(i64 {b64_in})
  %ok64 = icmp eq i64 %b64, {b64_exp}
  br i1 %ok64, label %c3, label %fail
c3:
  %x8 = call i8 @fn_xor8(i8 175, i8 60)
  %okx = icmp eq i8 %x8, {xor_exp}
  br i1 %okx, label %c4, label %fail
c4:
  %rr = call i16 @fn_rotr16(i16 {b16_in})
  %okrr = icmp eq i16 %rr, {rotr_exp}
  br i1 %okrr, label %c5, label %fail
c5:
  %rl = call i16 @fn_rotl16(i16 {b16_in})
  %okrl = icmp eq i16 %rl, {rotl_exp}
  br i1 %okrl, label %c6, label %fail
c6:
  %sl = call i32 @fn_shl32(i32 {lshr_exp})
  %oksl = icmp eq i32 %sl, {shl_exp}
  br i1 %oksl, label %c7, label %fail
c7:
  %sr = call i32 @fn_lshr32(i32 {b32_in})
  %oksr = icmp eq i32 %sr, {lshr_exp}
  br i1 %oksr, label %c8, label %fail
c8:
  %an = call i16 @fn_andn16(i16 65535, i16 3855)
  %okan = icmp eq i16 %an, {andn_exp}
  br i1 %okan, label %ok, label %fail
ok:
  ret i32 0
fail:
  call void @llvm.trap()
  unreachable
}}
"#,
            b16_in = 0x1234_u16,
            b32_in = 0x1234_5678_u32,
        );
        let ir = format!("declare void @llvm.trap()\n{decls}{helper_fns}{main_body}");
        run_native_checks(&ir);
    }

    #[test]
    fn emits_baseline_fallback_on_unknown_budget_and_cross_word_operations() {
        let graph = iir::Package::default();
        let lambdas = BTreeMap::new();
        let template_identities = callables::TemplateIdentities::default();
        let mut declarations = BTreeSet::new();
        let mut text_constants = strings::TextConstants::default();
        let mut cache = SynthesisCache::new();
        let mut fuel = 100_000;

        // 1. Force SAT solver Unknown(Budget) and verify verified BaselineProgram IR runs identically.
        let xor_region = {
            let mut b = RegionBuilder::new();
            let a = b.pure_input("a", 0, 16);
            let b_in = b.pure_input("b", 1, 16);
            let na = b.not(a).unwrap();
            let nb = b.not(b_in).unwrap();
            let l = b.and(a, nb).unwrap();
            let r = b.and(na, b_in).unwrap();
            let res = b.or(l, r).unwrap();
            b.build(res).unwrap()
        };
        let outcome = synthesize_with_budget(
            &xor_region,
            TargetProfile::X86_64BmiBswap,
            TargetProfile::X86_64BmiBswap.default_cost_model(),
            Budget::new(1),
            &mut cache,
        )
        .unwrap();
        assert!(
            matches!(
                outcome.status,
                ProofStatus::BaselineFallback(FallbackReason::SolverUnknown(_))
            ),
            "{}",
            outcome.status
        );
        let ChosenImplementation::Baseline(baseline_xor) = outcome.implementation else {
            panic!("expected Baseline fallback under exhausted budget");
        };

        // 2. Also build a 130-bit cross-word BaselineProgram testing WordConcatShifted,
        // WordExtractCrossWord, WordNot with tail mask, and WordEqual.
        let wide_region = {
            let mut b = RegionBuilder::new();
            let x = b.pure_input("x", 0, 70);
            let y = b.pure_input("y", 1, 60);
            let cat = b.concat(x, y).unwrap(); // 130 bits, shifted concat across 64-bit words
            let slice = b.extract_const(cat, 5, 70).unwrap(); // cross-word extract
            let inv = b.not(slice).unwrap();
            let res = b.zero_extend(inv, 130).unwrap();
            b.build(res).unwrap()
        };
        let baseline_wide = lower_to_baseline(&wide_region, 64).unwrap();
        let test_x = BitValue::from_u128(70, (1u128 << 65) | 0x1234_5678_9ABC_DEF0).unwrap();
        let test_y = BitValue::from_u64(60, 0x0AAA_BBBB_CCCC_DDDD).unwrap();
        let expected_wide = baseline_wide
            .execute(&BTreeMap::from([
                (InputIdentity::pure("x", 0), test_x.clone()),
                (InputIdentity::pure("y", 1), test_y.clone()),
            ]))
            .unwrap()
            .value;

        let mut e1 = make_test_emitter(
            &graph,
            &lambdas,
            &template_identities,
            &mut declarations,
            &mut text_constants,
            &mut cache,
            &mut fuel,
        );
        let out_xor = e1
            .emit_baseline_program(
                &baseline_xor,
                xor_region.output_kind,
                &BTreeMap::from([
                    (
                        InputIdentity::pure("a", 0),
                        Value {
                            ty: Ty::Int(16, false),
                            text: "%a".into(),
                        },
                    ),
                    (
                        InputIdentity::pure("b", 1),
                        Value {
                            ty: Ty::Int(16, false),
                            text: "%b".into(),
                        },
                    ),
                ]),
            )
            .unwrap();
        let fn1 = format!(
            "define i16 @base_xor16(i16 %a, i16 %b) {{\nentry:\n{}{}  ret i16 {}\n}}\n",
            e1.prologue, e1.body, out_xor.text
        );

        let mut e2 = make_test_emitter(
            &graph,
            &lambdas,
            &template_identities,
            &mut declarations,
            &mut text_constants,
            &mut cache,
            &mut fuel,
        );
        let out_wide = e2
            .emit_baseline_program(
                &baseline_wide,
                wide_region.output_kind,
                &BTreeMap::from([
                    (
                        InputIdentity::pure("x", 0),
                        Value {
                            ty: Ty::Int(70, false),
                            text: "%x".into(),
                        },
                    ),
                    (
                        InputIdentity::pure("y", 1),
                        Value {
                            ty: Ty::Int(60, false),
                            text: "%y".into(),
                        },
                    ),
                ]),
            )
            .unwrap();
        let fn2 = format!(
            "define i130 @base_wide(i70 %x, i60 %y) {{\nentry:\n{}{}  ret i130 {}\n}}\n",
            e2.prologue, e2.body, out_wide.text
        );

        let x_dec = bitvalue_decimal(&test_x);
        let y_dec = bitvalue_decimal(&test_y);
        let exp_dec = bitvalue_decimal(&expected_wide);
        let ir = format!(
            "declare void @llvm.trap()\n{fn1}{fn2}
define i32 @main() {{
entry:
  %r1 = call i16 @base_xor16(i16 43690, i16 13107)
  %ok1 = icmp eq i16 %r1, 39321
  br i1 %ok1, label %w, label %fail
w:
  %r2 = call i130 @base_wide(i70 {x_dec}, i60 {y_dec})
  %ok2 = icmp eq i130 %r2, {exp_dec}
  br i1 %ok2, label %ok, label %fail
ok:
  ret i32 0
fail:
  call void @llvm.trap()
  unreachable
}}
"
        );
        run_native_checks(&ir);
    }
}
