//! Bridge between checked NIR patterns/types and `pattern_automata::compile_with_budget`.

use nir::DeclarationId;
use nir::Primitive;
use nir::Term;
use nir::match_plan::{ListObservationMode, ListViewOps, MatchPlan};
use nir::program::{self as n, SourceRange, VariantShape};
use pattern_automata as pa;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(crate) struct ListPatternInfo {
    pub protocol: DeclarationId,
    pub mode: Option<ListObservationMode>,
    pub by_value: bool,
    pub size_type: Term,
    pub element_type: Term,
    pub rest_type: Option<Term>,
    pub length_method: Option<DeclarationId>,
    pub element_method: Option<DeclarationId>,
    pub rest_method: Option<DeclarationId>,
    pub prefix_patterns: Vec<n::Pattern>,
    pub rest_pattern: Option<n::Pattern>,
}

impl ListPatternInfo {
    fn to_view_ops(&self) -> Option<ListViewOps> {
        let mode = self.mode?;
        Some(ListViewOps {
            protocol: self.protocol,
            mode,
            by_value: self.by_value,
            size_type: self.size_type.clone(),
            element_type: self.element_type.clone(),
            rest_type: self.rest_type.clone(),
            length_method: self.length_method,
            element_method: self.element_method,
            rest_method: self.rest_method,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) enum RangePatternInfo {
    Int {
        start: pa::RangeBound<pa::IntLit>,
        end: pa::RangeBound<pa::IntLit>,
    },
    Char {
        start: pa::RangeBound<char>,
        end: pa::RangeBound<char>,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum BitSegmentInfo {
    Fixed(pa::bitvec::BitValue),
    Typed {
        width: u32,
        ty: Term,
        pattern: n::Pattern,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct BitVecPatternInfo {
    pub total_width: u32,
    pub segments: Vec<BitSegmentInfo>,
}

pub(crate) struct AutomataOutcome {
    pub plan: MatchPlan,
    pub missing_witnesses: Vec<pa::MissingWitness>,
}

pub(crate) trait AutomataEnv {
    fn structural(&self, ty: &Term) -> Option<Term>;

    fn integer_layout(&self, primitive: Primitive) -> Option<(bool, u32)>;

    fn list_element_type<'t>(&self, ty: &'t Term) -> Option<&'t Term>;

    fn enum_def(&self, id: DeclarationId) -> Option<&n::Enum>;

    fn struct_def(&self, id: DeclarationId) -> Option<&n::Struct>;

    fn instantiate_enum_variant_fields(
        &self,
        def: &n::Enum,
        args: &[Term],
    ) -> Option<Vec<Vec<Term>>>;

    fn instantiate_struct_fields(
        &self,
        def: &n::Struct,
        args: &[Term],
    ) -> Option<Vec<(String, Term)>>;

    fn list_pattern_info(&self, binding: n::LocalId) -> Option<&ListPatternInfo>;

    fn range_pattern_info(&self, binding: n::LocalId) -> Option<&RangePatternInfo>;

    fn bitvec_pattern_info(&self, binding: n::LocalId) -> Option<&BitVecPatternInfo>;
}

pub(crate) fn compile_match_plan(
    env: &dyn AutomataEnv,
    patterns: &[&n::Pattern],
    ty: &Term,
    source: SourceRange,
) -> Option<AutomataOutcome> {
    let mut list_views: BTreeMap<pa::Place, ListViewOps> = BTreeMap::new();
    let mut impure_list_places: BTreeSet<pa::Place> = BTreeSet::new();
    let mut record_places: BTreeSet<pa::Place> = BTreeSet::new();
    let mut exact = true;
    let mut structural_exact = true;
    let mut arms = Vec::with_capacity(patterns.len());
    for (arm_index, pat) in patterns.iter().enumerate() {
        let (inner_pat, top_guard) = match *pat {
            n::Pattern::Guard { pattern, .. } => (pattern.as_ref(), true),
            other => (other, false),
        };
        let mut arm_exact = true;
        let (converted, inner_guard) = convert_pattern(
            env,
            inner_pat,
            &pa::Place::root(),
            &mut list_views,
            &mut impure_list_places,
            &mut record_places,
            &mut arm_exact,
            0,
        );
        if top_guard || !arm_exact {
            exact = false;
        }
        if inner_guard || !arm_exact {
            structural_exact = false;
        }
        let arm = pa::MatchArm::new(arm_index, converted, top_guard || inner_guard).ok()?;
        arms.push(arm);
    }

    // If a single match mixes struct field patterns and stable list patterns at the
    // same normalized path, or mixes impure list observation with stable list patterns
    // at the same normalized path, decline decision-DAG specialization to preserve soundness.
    if list_views.keys().any(|lp| {
        record_places
            .iter()
            .any(|rp| same_normalized_path(&lp.steps, &rp.steps))
            || impure_list_places
                .iter()
                .any(|ip| same_normalized_path(&lp.steps, &ip.steps))
    }) {
        return None;
    }

    let mut visiting = BTreeSet::new();
    let type_desc = term_to_type_desc_at(
        env,
        ty,
        &pa::Place::root(),
        &list_views,
        &mut exact,
        &mut visiting,
        0,
    );

    let matrix = pa::PatternMatrix::new(type_desc.clone(), arms).ok()?;
    let output = pa::compile_with_budget(&matrix, pa::CompileBudget::default()).ok()?;
    let unreachable_arms = output.unreachable_arms;
    let is_exhaustive = output.is_exhaustive;
    let stats = output.stats;
    let dag = output.dag;

    Some(AutomataOutcome {
        plan: MatchPlan {
            source,
            scrutinee_type: ty.clone(),
            type_desc,
            dag,
            list_views,
            unreachable_arms,
            is_exhaustive,
            stats,
            exact,
            structural_exact,
        },
        missing_witnesses: output.missing_witnesses,
    })
}

pub(crate) fn format_missing_witnesses(
    env: &dyn AutomataEnv,
    witnesses: &[pa::MissingWitness],
    ty: &Term,
    list_views: &BTreeMap<pa::Place, ListViewOps>,
) -> String {
    let mut out = String::new();
    let shown = witnesses.len().min(3);
    let root = pa::Place::root();
    for (idx, witness) in witnesses.iter().take(shown).enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        format_witness_into(env, witness, ty, &root, list_views, &mut out, 0);
    }
    if witnesses.len() > shown {
        out.push_str(", ...");
    }
    out
}

fn same_normalized_path(a: &[pa::PlaceStep], b: &[pa::PlaceStep]) -> bool {
    use pa::PlaceStep as S;
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .all(|(left, right)| match (left, right) {
            (S::ListElem(_), S::ListElem(_)) => true,
            _ => left == right,
        })
}

fn find_list_view_for_place<'a>(
    place: &pa::Place,
    list_views: &'a BTreeMap<pa::Place, ListViewOps>,
) -> Option<&'a ListViewOps> {
    if let Some(exact) = list_views.get(place) {
        return Some(exact);
    }
    list_views
        .iter()
        .find(|(candidate, _)| same_normalized_path(&place.steps, &candidate.steps))
        .map(|(_, ops)| ops)
}

fn merge_list_view(slot: &mut Option<ListViewOps>, incoming: ListViewOps) {
    match slot {
        None => *slot = Some(incoming),
        Some(existing) => {
            if matches!(
                existing.element_type,
                Term::AssociatedType { .. } | Term::Primitive(Primitive::Void)
            ) && !matches!(
                incoming.element_type,
                Term::AssociatedType { .. } | Term::Primitive(Primitive::Void)
            ) {
                existing.element_type = incoming.element_type;
            }
            if existing.rest_type.is_none() {
                existing.rest_type = incoming.rest_type;
            }
            if existing.rest_method.is_none() {
                existing.rest_method = incoming.rest_method;
            }
            if existing.length_method.is_none() {
                existing.length_method = incoming.length_method;
            }
            if existing.element_method.is_none() {
                existing.element_method = incoming.element_method;
            }
        }
    }
}

fn record_list_view(
    list_views: &mut BTreeMap<pa::Place, ListViewOps>,
    place: pa::Place,
    incoming: ListViewOps,
) {
    let mut entry = list_views.remove(&place);
    merge_list_view(&mut entry, incoming);
    if let Some(merged) = entry {
        list_views.insert(place, merged);
    }
}

fn error_tag_for(env: &dyn AutomataEnv, ty: &Term) -> pa::ErrorTag {
    let normalized = env.structural(ty).unwrap_or_else(|| ty.clone());
    pa::ErrorTag::new(normalized.to_string())
}

fn convert_bit_words(width: u32, words: &[u32]) -> Option<pa::bitvec::BitValue> {
    let u64_len = words.len().div_ceil(2);
    let mut out = Vec::with_capacity(u64_len);
    for pair in words.chunks(2) {
        let lo = u64::from(pair[0]);
        let hi = pair.get(1).copied().map(u64::from).unwrap_or(0);
        out.push(lo | (hi << 32));
    }
    pa::bitvec::BitValue::from_words(width, out).ok()
}

fn convert_literal(lit: &n::Literal) -> Option<pa::Pattern> {
    match lit {
        n::Literal::Bool(b) => Some(pa::Pattern::Bool(*b)),
        n::Literal::Char(c) => Some(pa::Pattern::Char(*c)),
        n::Literal::SignedInteger(v) => Some(pa::Pattern::Int(pa::IntLit::Signed(*v))),
        n::Literal::UnsignedInteger(v) => Some(pa::Pattern::Int(pa::IntLit::Unsigned(*v))),
        n::Literal::Integer(s) => {
            if let Ok(v) = s.parse::<i128>() {
                Some(pa::Pattern::Int(pa::IntLit::Signed(v)))
            } else if let Ok(u) = s.parse::<u128>() {
                Some(pa::Pattern::Int(pa::IntLit::Unsigned(u)))
            } else {
                None
            }
        }
        n::Literal::BitVec { width, words } => {
            let w = u32::try_from(*width).ok()?;
            let bv = convert_bit_words(w, words)?;
            Some(pa::Pattern::BitVec(pa::BitVecPattern::with_width(
                w,
                vec![pa::BitSegmentPattern::fixed(bv)],
            )))
        }
        n::Literal::Float16(_)
        | n::Literal::BFloat16(_)
        | n::Literal::Float32(_)
        | n::Literal::Float64(_)
        | n::Literal::Float(_)
        | n::Literal::String(_)
        | n::Literal::Symbol(_)
        | n::Literal::PendingInteger { .. }
        | n::Literal::PendingFloat { .. }
        | n::Literal::Description(_) => None,
    }
}

fn rename_pa_pattern_bindings(
    pat: &mut pa::Pattern,
    rename_map: &BTreeMap<String, String>,
    depth: usize,
    exact: &mut bool,
) {
    if depth > 128 {
        *exact = false;
        return;
    }
    match pat {
        pa::Pattern::Bind { name, subpattern } => {
            if let Some(target) = rename_map.get(name.as_str()) {
                *name = target.clone();
            }
            rename_pa_pattern_bindings(subpattern, rename_map, depth + 1, exact);
        }
        pa::Pattern::OptionalPresent(inner)
        | pa::Pattern::ErrorOk(inner)
        | pa::Pattern::ErrorErr { inner, .. } => {
            rename_pa_pattern_bindings(inner, rename_map, depth + 1, exact);
        }
        pa::Pattern::EnumVariant { payload, .. } => match payload {
            pa::VariantPayloadPattern::Unit => {}
            pa::VariantPayloadPattern::Tuple(elems) => {
                for elem in elems {
                    rename_pa_pattern_bindings(elem, rename_map, depth + 1, exact);
                }
            }
            pa::VariantPayloadPattern::Record(fields) => {
                for field in fields.values_mut() {
                    rename_pa_pattern_bindings(field, rename_map, depth + 1, exact);
                }
            }
        },
        pa::Pattern::Tuple(elems) | pa::Pattern::Or(elems) => {
            for elem in elems {
                rename_pa_pattern_bindings(elem, rename_map, depth + 1, exact);
            }
        }
        pa::Pattern::Record(fields) => {
            for field in fields.values_mut() {
                rename_pa_pattern_bindings(field, rename_map, depth + 1, exact);
            }
        }
        pa::Pattern::List { prefix, rest } => {
            for elem in prefix {
                rename_pa_pattern_bindings(elem, rename_map, depth + 1, exact);
            }
            if let Some(rest_pat) = rest {
                rename_pa_pattern_bindings(rest_pat, rename_map, depth + 1, exact);
            }
        }
        pa::Pattern::BitVec(bv) => {
            for seg in &mut bv.segments {
                if let pa::BitSegmentPattern::Typed { pattern, .. } = seg {
                    rename_pa_pattern_bindings(pattern, rename_map, depth + 1, exact);
                }
            }
        }
        pa::Pattern::Wild
        | pa::Pattern::Bool(_)
        | pa::Pattern::Char(_)
        | pa::Pattern::CharRange { .. }
        | pa::Pattern::Int(_)
        | pa::Pattern::IntRange { .. }
        | pa::Pattern::OptionalNull => {}
    }
}

fn convert_pattern(
    env: &dyn AutomataEnv,
    pat: &n::Pattern,
    place: &pa::Place,
    list_views: &mut BTreeMap<pa::Place, ListViewOps>,
    impure_list_places: &mut BTreeSet<pa::Place>,
    record_places: &mut BTreeSet<pa::Place>,
    exact: &mut bool,
    depth: usize,
) -> (pa::Pattern, bool) {
    if depth > 128 {
        *exact = false;
        return (pa::Pattern::Wild, true);
    }
    match pat {
        n::Pattern::Wild => (pa::Pattern::Wild, false),
        n::Pattern::Unit => (pa::Pattern::Tuple(Vec::new()), false),
        n::Pattern::Binding { binding, inner } => {
            let (sub, guard) = convert_pattern(
                env,
                inner,
                place,
                list_views,
                impure_list_places,
                record_places,
                exact,
                depth + 1,
            );
            (
                pa::Pattern::Bind {
                    name: format!("_{}_{}", binding.owner, binding.index),
                    subpattern: Box::new(sub),
                },
                guard,
            )
        }
        n::Pattern::Remap { pattern, bindings } => {
            let (mut converted, guard) = convert_pattern(
                env,
                pattern,
                place,
                list_views,
                impure_list_places,
                record_places,
                exact,
                depth + 1,
            );
            if !bindings.is_empty() {
                let rename_map: BTreeMap<String, String> = bindings
                    .iter()
                    .map(|(from, to)| {
                        (
                            format!("_{}_{}", from.owner, from.index),
                            format!("_{}_{}", to.owner, to.index),
                        )
                    })
                    .collect();
                rename_pa_pattern_bindings(&mut converted, &rename_map, 0, exact);
            }
            (converted, guard)
        }
        n::Pattern::Literal(lit) => match convert_literal(lit) {
            Some(p) => (p, false),
            None => {
                *exact = false;
                (pa::Pattern::Wild, true)
            }
        },
        n::Pattern::OptionalAbsent => (pa::Pattern::OptionalNull, false),
        n::Pattern::OptionalPresent(inner) => {
            let (sub, guard) = convert_pattern(
                env,
                inner,
                &place.optional_inner(),
                list_views,
                impure_list_places,
                record_places,
                exact,
                depth + 1,
            );
            (pa::Pattern::OptionalPresent(Box::new(sub)), guard)
        }
        n::Pattern::ErrorResult { label: None, inner } => {
            let (sub, guard) = convert_pattern(
                env,
                inner,
                &place.error_ok_inner(),
                list_views,
                impure_list_places,
                record_places,
                exact,
                depth + 1,
            );
            (pa::Pattern::ErrorOk(Box::new(sub)), guard)
        }
        n::Pattern::ErrorResult {
            label: Some(err_ty),
            inner,
        } => {
            let (sub, guard) = convert_pattern(
                env,
                inner,
                &place.error_err_inner(),
                list_views,
                impure_list_places,
                record_places,
                exact,
                depth + 1,
            );
            (
                pa::Pattern::ErrorErr {
                    tag: Some(error_tag_for(env, err_ty)),
                    inner: Box::new(sub),
                },
                guard,
            )
        }
        n::Pattern::Tuple(elems) => {
            let mut out = Vec::with_capacity(elems.len());
            let mut has_guard = false;
            for (idx, elem) in elems.iter().enumerate() {
                let (p, g) = convert_pattern(
                    env,
                    elem,
                    &place.tuple_field(idx),
                    list_views,
                    impure_list_places,
                    record_places,
                    exact,
                    depth + 1,
                );
                out.push(p);
                has_guard |= g;
            }
            (pa::Pattern::Tuple(out), has_guard)
        }
        n::Pattern::AnonymousRecord { names, fields } => {
            record_places.insert(place.clone());
            let mut map = BTreeMap::new();
            let mut has_guard = false;
            for (name, field) in names.iter().zip(fields.iter()) {
                let (p, g) = convert_pattern(
                    env,
                    field,
                    &place.record_field(name.as_str()),
                    list_views,
                    impure_list_places,
                    record_places,
                    exact,
                    depth + 1,
                );
                map.insert(name.clone(), p);
                has_guard |= g;
            }
            (pa::Pattern::Record(map), has_guard)
        }
        n::Pattern::Record {
            declaration,
            fields,
        } => {
            let Some(def) = env.struct_def(*declaration) else {
                *exact = false;
                return (pa::Pattern::Wild, true);
            };
            if def.fields.len() != fields.len() {
                *exact = false;
                return (pa::Pattern::Wild, true);
            }
            record_places.insert(place.clone());
            let mut map = BTreeMap::new();
            let mut has_guard = false;
            for ((name, _), field) in def.fields.iter().zip(fields.iter()) {
                let (p, g) = convert_pattern(
                    env,
                    field,
                    &place.record_field(name.as_str()),
                    list_views,
                    impure_list_places,
                    record_places,
                    exact,
                    depth + 1,
                );
                map.insert(name.clone(), p);
                has_guard |= g;
            }
            (pa::Pattern::Record(map), has_guard)
        }
        n::Pattern::Variant {
            declaration,
            index,
            fields,
        } => {
            let Some(def) = env.enum_def(*declaration) else {
                *exact = false;
                return (pa::Pattern::Wild, true);
            };
            let Some(variant) = def.variants.get(*index) else {
                *exact = false;
                return (pa::Pattern::Wild, true);
            };
            let tag = pa::VariantTag::named(variant.name.clone());
            let mut has_guard = false;
            let payload = match &variant.shape {
                VariantShape::Unit => pa::VariantPayloadPattern::Unit,
                VariantShape::Tuple => {
                    let mut out = Vec::with_capacity(fields.len());
                    for (i, field) in fields.iter().enumerate() {
                        let (p, g) = convert_pattern(
                            env,
                            field,
                            &place.variant_tuple_field(tag.clone(), i),
                            list_views,
                            impure_list_places,
                            record_places,
                            exact,
                            depth + 1,
                        );
                        out.push(p);
                        has_guard |= g;
                    }
                    pa::VariantPayloadPattern::Tuple(out)
                }
                VariantShape::Named(names) => {
                    if names.len() != fields.len() {
                        *exact = false;
                        return (pa::Pattern::Wild, true);
                    }
                    let mut map = BTreeMap::new();
                    for (name, field) in names.iter().zip(fields.iter()) {
                        let (p, g) = convert_pattern(
                            env,
                            field,
                            &place.variant_record_field(tag.clone(), name.as_str()),
                            list_views,
                            impure_list_places,
                            record_places,
                            exact,
                            depth + 1,
                        );
                        map.insert(name.clone(), p);
                        has_guard |= g;
                    }
                    pa::VariantPayloadPattern::Record(map)
                }
            };
            (pa::Pattern::EnumVariant { tag, payload }, has_guard)
        }
        n::Pattern::Or(alts) => {
            let mut out = Vec::with_capacity(alts.len());
            let mut has_guard = false;
            for alt in alts {
                let (p, g) = convert_pattern(
                    env,
                    alt,
                    place,
                    list_views,
                    impure_list_places,
                    record_places,
                    exact,
                    depth + 1,
                );
                out.push(p);
                has_guard |= g;
            }
            (pa::Pattern::Or(out), has_guard)
        }
        n::Pattern::Guard { pattern, .. } => {
            let (inner, _) = convert_pattern(
                env,
                pattern,
                place,
                list_views,
                impure_list_places,
                record_places,
                exact,
                depth + 1,
            );
            *exact = false;
            (inner, true)
        }
        n::Pattern::Observe { binding, .. } => {
            if let Some(info) = env.list_pattern_info(*binding) {
                let Some(ops) = info.to_view_ops() else {
                    // Impure general `ListPatternView`: preserve per-candidate re-observation and require `_`.
                    impure_list_places.insert(place.clone());
                    *exact = false;
                    return (pa::Pattern::Wild, true);
                };
                record_list_view(list_views, place.clone(), ops);
                let mut prefix = Vec::with_capacity(info.prefix_patterns.len());
                let mut has_guard = false;
                for (idx, elem_pat) in info.prefix_patterns.iter().enumerate() {
                    let (p, g) = convert_pattern(
                        env,
                        elem_pat,
                        &place.list_elem(idx),
                        list_views,
                        impure_list_places,
                        record_places,
                        exact,
                        depth + 1,
                    );
                    prefix.push(p);
                    has_guard |= g;
                }
                let rest = if let Some(rest_pat) = &info.rest_pattern {
                    let (p, g) = convert_pattern(
                        env,
                        rest_pat,
                        &place.list_rest(info.prefix_patterns.len()),
                        list_views,
                        impure_list_places,
                        record_places,
                        exact,
                        depth + 1,
                    );
                    has_guard |= g;
                    Some(Box::new(p))
                } else {
                    None
                };
                return (pa::Pattern::List { prefix, rest }, has_guard);
            }
            if let Some(range) = env.range_pattern_info(*binding) {
                return match range {
                    RangePatternInfo::Int { start, end } => (
                        pa::Pattern::IntRange {
                            start: *start,
                            end: *end,
                        },
                        false,
                    ),
                    RangePatternInfo::Char { start, end } => (
                        pa::Pattern::CharRange {
                            start: *start,
                            end: *end,
                        },
                        false,
                    ),
                };
            }
            if let Some(bv) = env.bitvec_pattern_info(*binding) {
                let mut offset = bv.total_width;
                let mut segments = Vec::with_capacity(bv.segments.len());
                let mut has_guard = false;
                let mut ok = true;
                for seg in &bv.segments {
                    match seg {
                        BitSegmentInfo::Fixed(val) => {
                            let Some(next) = offset.checked_sub(val.width()) else {
                                ok = false;
                                break;
                            };
                            offset = next;
                            segments.push(pa::BitSegmentPattern::Fixed {
                                width: val.width(),
                                value: val.clone(),
                            });
                        }
                        BitSegmentInfo::Typed { width, ty, pattern } => {
                            let Some(next) = offset.checked_sub(*width) else {
                                ok = false;
                                break;
                            };
                            let mut seg_visiting = BTreeSet::new();
                            let seg_desc = term_to_type_desc_plain(
                                env,
                                ty,
                                exact,
                                &mut seg_visiting,
                                depth + 1,
                            );
                            let slice_place = place.bit_slice(next, offset, seg_desc.clone());
                            offset = next;
                            let (sub, g) = convert_pattern(
                                env,
                                pattern,
                                &slice_place,
                                list_views,
                                impure_list_places,
                                record_places,
                                exact,
                                depth + 1,
                            );
                            has_guard |= g;
                            segments.push(pa::BitSegmentPattern::Typed {
                                width: *width,
                                ty: seg_desc,
                                pattern: Box::new(sub),
                            });
                        }
                    }
                }
                if ok && offset == 0 {
                    return (
                        pa::Pattern::BitVec(pa::BitVecPattern::with_width(
                            bv.total_width,
                            segments,
                        )),
                        has_guard,
                    );
                }
                *exact = false;
                return (pa::Pattern::Wild, true);
            }
            *exact = false;
            (pa::Pattern::Wild, true)
        }
        n::Pattern::Then { .. }
        | n::Pattern::NominalType { .. }
        | n::Pattern::TypeTuple { .. }
        | n::Pattern::NormalForm { .. }
        | n::Pattern::CodeEvidence { .. } => {
            *exact = false;
            (pa::Pattern::Wild, true)
        }
    }
}

fn term_to_type_desc_plain(
    env: &dyn AutomataEnv,
    ty: &Term,
    exact: &mut bool,
    visiting: &mut BTreeSet<DeclarationId>,
    depth: usize,
) -> pa::TypeDesc {
    static EMPTY: BTreeMap<pa::Place, ListViewOps> = BTreeMap::new();
    term_to_type_desc_at(env, ty, &pa::Place::root(), &EMPTY, exact, visiting, depth)
}

fn term_to_type_desc_at(
    env: &dyn AutomataEnv,
    ty: &Term,
    place: &pa::Place,
    list_views: &BTreeMap<pa::Place, ListViewOps>,
    exact: &mut bool,
    visiting: &mut BTreeSet<DeclarationId>,
    depth: usize,
) -> pa::TypeDesc {
    if depth > 32 {
        *exact = false;
        return pa::TypeDesc::Any;
    }

    if let Some(ops) = find_list_view_for_place(place, list_views) {
        let elem_desc = term_to_type_desc_at(
            env,
            &ops.element_type,
            &place.list_elem(0),
            list_views,
            exact,
            visiting,
            depth + 1,
        );
        return pa::TypeDesc::List(Box::new(elem_desc));
    }

    if let Some(element) = env.list_element_type(ty) {
        let elem_desc = term_to_type_desc_at(
            env,
            element,
            &place.list_elem(0),
            list_views,
            exact,
            visiting,
            depth + 1,
        );
        return pa::TypeDesc::List(Box::new(elem_desc));
    }

    let shape = env.structural(ty).unwrap_or_else(|| ty.clone());
    match &shape {
        Term::Primitive(prim) => match prim {
            Primitive::Bool => pa::TypeDesc::Bool,
            Primitive::Char => pa::TypeDesc::Char,
            Primitive::Void => pa::TypeDesc::Tuple(Vec::new()),
            Primitive::Never => pa::TypeDesc::Enum(pa::EnumType::closed("Never", BTreeMap::new())),
            Primitive::Integer => pa::TypeDesc::Int(pa::IntType::OpenSigned),
            Primitive::BitVec(w) => match u32::try_from(*w) {
                Ok(bits) => pa::TypeDesc::BitVec(bits),
                Err(_) => {
                    *exact = false;
                    pa::TypeDesc::Any
                }
            },
            Primitive::Usize => match env.integer_layout(*prim) {
                Some((false, bits)) if bits <= 128 => {
                    pa::TypeDesc::Int(pa::IntType::UnsignedBits(bits as u8))
                }
                _ => pa::TypeDesc::Int(pa::IntType::OpenUnsigned),
            },
            Primitive::Isize => match env.integer_layout(*prim) {
                Some((true, bits)) if bits <= 128 => {
                    pa::TypeDesc::Int(pa::IntType::SignedBits(bits as u8))
                }
                _ => pa::TypeDesc::Int(pa::IntType::OpenSigned),
            },
            other => {
                if let Some((signed, bits)) = env.integer_layout(*other)
                    && (1..=128).contains(&bits)
                {
                    let int_ty = if signed {
                        pa::IntType::SignedBits(bits as u8)
                    } else {
                        pa::IntType::UnsignedBits(bits as u8)
                    };
                    pa::TypeDesc::Int(int_ty)
                } else {
                    pa::TypeDesc::Any
                }
            }
        },
        Term::Optional(inner) => pa::TypeDesc::Optional(Box::new(term_to_type_desc_at(
            env,
            inner,
            &place.optional_inner(),
            list_views,
            exact,
            visiting,
            depth + 1,
        ))),
        Term::ErrorResult { errors, result } => {
            let ok_type = Box::new(term_to_type_desc_at(
                env,
                result,
                &place.error_ok_inner(),
                list_views,
                exact,
                visiting,
                depth + 1,
            ));
            let mut error_tags = BTreeMap::new();
            for err_ty in errors {
                let tag = error_tag_for(env, err_ty);
                let err_desc = term_to_type_desc_at(
                    env,
                    err_ty,
                    &place.error_err_inner(),
                    list_views,
                    exact,
                    visiting,
                    depth + 1,
                );
                error_tags.insert(tag, err_desc);
            }
            pa::TypeDesc::ErrorRow(pa::ErrorRowType {
                ok_type,
                error_tags,
                is_open: false,
            })
        }
        Term::Tuple(elems) => pa::TypeDesc::Tuple(
            elems
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    term_to_type_desc_at(
                        env,
                        e,
                        &place.tuple_field(i),
                        list_views,
                        exact,
                        visiting,
                        depth + 1,
                    )
                })
                .collect(),
        ),
        Term::Record(fields) => pa::TypeDesc::Record(
            fields
                .iter()
                .map(|(name, field_ty)| {
                    (
                        name.clone(),
                        term_to_type_desc_at(
                            env,
                            field_ty,
                            &place.record_field(name.as_str()),
                            list_views,
                            exact,
                            visiting,
                            depth + 1,
                        ),
                    )
                })
                .collect(),
        ),
        Term::Nominal {
            declaration,
            arguments,
        } => {
            if !visiting.insert(*declaration) {
                *exact = false;
                return pa::TypeDesc::Any;
            }
            let out = if let Some(def) = env.enum_def(*declaration) {
                if let Some(instantiated) = env.instantiate_enum_variant_fields(def, arguments) {
                    let mut variants = BTreeMap::new();
                    for (variant, field_tys) in def.variants.iter().zip(instantiated.iter()) {
                        let tag = pa::VariantTag::named(variant.name.clone());
                        let payload = match &variant.shape {
                            VariantShape::Unit => pa::VariantPayloadType::Unit,
                            VariantShape::Tuple => pa::VariantPayloadType::Tuple(
                                field_tys
                                    .iter()
                                    .enumerate()
                                    .map(|(i, ft)| {
                                        term_to_type_desc_at(
                                            env,
                                            ft,
                                            &place.variant_tuple_field(tag.clone(), i),
                                            list_views,
                                            exact,
                                            visiting,
                                            depth + 1,
                                        )
                                    })
                                    .collect(),
                            ),
                            VariantShape::Named(names) => pa::VariantPayloadType::Record(
                                names
                                    .iter()
                                    .zip(field_tys.iter())
                                    .map(|(name, ft)| {
                                        (
                                            name.clone(),
                                            term_to_type_desc_at(
                                                env,
                                                ft,
                                                &place.variant_record_field(
                                                    tag.clone(),
                                                    name.as_str(),
                                                ),
                                                list_views,
                                                exact,
                                                visiting,
                                                depth + 1,
                                            ),
                                        )
                                    })
                                    .collect(),
                            ),
                        };
                        variants.insert(
                            tag,
                            pa::VariantSchema {
                                discrete_code: None,
                                payload,
                            },
                        );
                    }
                    pa::TypeDesc::Enum(pa::EnumType::closed(def.name.clone(), variants))
                } else {
                    *exact = false;
                    pa::TypeDesc::Any
                }
            } else if let Some(def) = env.struct_def(*declaration) {
                if let Some(fields) = env.instantiate_struct_fields(def, arguments) {
                    pa::TypeDesc::Record(
                        fields
                            .into_iter()
                            .map(|(name, ft)| {
                                let child_place = place.record_field(name.as_str());
                                let desc = term_to_type_desc_at(
                                    env,
                                    &ft,
                                    &child_place,
                                    list_views,
                                    exact,
                                    visiting,
                                    depth + 1,
                                );
                                (name, desc)
                            })
                            .collect(),
                    )
                } else {
                    *exact = false;
                    pa::TypeDesc::Any
                }
            } else {
                pa::TypeDesc::Any
            };
            visiting.remove(declaration);
            out
        }
        _ => pa::TypeDesc::Any,
    }
}

fn format_witness_into(
    env: &dyn AutomataEnv,
    witness: &pa::MissingWitness,
    ty: &Term,
    place: &pa::Place,
    list_views: &BTreeMap<pa::Place, ListViewOps>,
    out: &mut String,
    depth: usize,
) {
    use std::fmt::Write;
    if depth > 32 {
        out.push('_');
        return;
    }
    let shape = env.structural(ty).unwrap_or_else(|| ty.clone());
    match witness {
        pa::MissingWitness::Wild | pa::MissingWitness::Opaque => out.push('_'),
        pa::MissingWitness::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        pa::MissingWitness::Char(c) => {
            let _ = write!(out, "{c:?}");
        }
        pa::MissingWitness::Int(lit) => match lit {
            pa::IntLit::Signed(v) => {
                let _ = write!(out, "{v}");
            }
            pa::IntLit::Unsigned(v) => {
                let _ = write!(out, "{v}");
            }
        },
        pa::MissingWitness::OptionalNull => out.push_str("null"),
        pa::MissingWitness::OptionalPresent(inner) => {
            if let Term::Optional(inner_ty) = &shape {
                if matches!(
                    inner.as_ref(),
                    pa::MissingWitness::Wild | pa::MissingWitness::Opaque
                ) {
                    out.push_str("_?");
                } else {
                    format_witness_into(
                        env,
                        inner,
                        inner_ty,
                        &place.optional_inner(),
                        list_views,
                        out,
                        depth + 1,
                    );
                    out.push('?');
                }
            } else {
                format_witness_into(
                    env,
                    inner,
                    &shape,
                    &place.optional_inner(),
                    list_views,
                    out,
                    depth + 1,
                );
                out.push('?');
            }
        }
        pa::MissingWitness::ErrorOk(inner) => {
            if let Term::ErrorResult { result, .. } = &shape {
                if matches!(
                    inner.as_ref(),
                    pa::MissingWitness::Wild | pa::MissingWitness::Opaque
                ) {
                    out.push_str("_!");
                } else {
                    format_witness_into(
                        env,
                        inner,
                        result,
                        &place.error_ok_inner(),
                        list_views,
                        out,
                        depth + 1,
                    );
                    out.push('!');
                }
            } else {
                out.push_str("_!");
            }
        }
        pa::MissingWitness::ErrorErr { tag, inner } => {
            let matched_err_ty = if let Term::ErrorResult { errors, .. } = &shape {
                errors
                    .iter()
                    .find(|e| error_tag_for(env, e) == *tag)
                    .cloned()
            } else {
                None
            };
            out.push_str("error ");
            if let Some(err_ty) = matched_err_ty {
                if matches!(
                    inner.as_ref(),
                    pa::MissingWitness::Wild | pa::MissingWitness::Opaque
                ) {
                    if let Term::Nominal { declaration, .. } = &err_ty
                        && let Some(def) = env.enum_def(*declaration)
                    {
                        let _ = write!(out, "{}(_)", def.name);
                    } else if let Term::Nominal { declaration, .. } = &err_ty
                        && let Some(def) = env.struct_def(*declaration)
                    {
                        let _ = write!(out, "{} {{ .. }}", def.name);
                    } else {
                        out.push('_');
                    }
                } else {
                    format_witness_into(
                        env,
                        inner,
                        &err_ty,
                        &place.error_err_inner(),
                        list_views,
                        out,
                        depth + 1,
                    );
                }
            } else {
                out.push('_');
            }
        }
        pa::MissingWitness::ErrorOtherTag { .. } | pa::MissingWitness::EnumOtherVariant => {
            out.push('_')
        }
        pa::MissingWitness::EnumVariant { tag, payload } => {
            match tag {
                pa::VariantTag::Named(name) => {
                    out.push('.');
                    out.push_str(name);
                }
                pa::VariantTag::Discrete(code) => {
                    let _ = write!(out, "{code}");
                }
            }
            let variant_fields: Option<Vec<(Option<String>, Term)>> = if let Term::Nominal {
                declaration,
                arguments,
            } = &shape
                && let Some(def) = env.enum_def(*declaration)
                && let Some(inst) = env.instantiate_enum_variant_fields(def, arguments)
                && let pa::VariantTag::Named(name) = tag
                && let Some((variant, tys)) = def
                    .variants
                    .iter()
                    .zip(inst.into_iter())
                    .find(|(v, _)| &v.name == name)
            {
                Some(match &variant.shape {
                    VariantShape::Unit => Vec::new(),
                    VariantShape::Tuple => tys.into_iter().map(|t| (None, t)).collect(),
                    VariantShape::Named(names) => {
                        names.iter().cloned().map(Some).zip(tys).collect()
                    }
                })
            } else {
                None
            };
            match payload {
                pa::WitnessPayload::Unit => {}
                pa::WitnessPayload::Tuple(elems) => {
                    out.push('(');
                    for (i, w) in elems.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        let elem_ty = variant_fields
                            .as_ref()
                            .and_then(|fs| fs.get(i))
                            .map(|(_, t)| t)
                            .unwrap_or(&shape);
                        format_witness_into(
                            env,
                            w,
                            elem_ty,
                            &place.variant_tuple_field(tag.clone(), i),
                            list_views,
                            out,
                            depth + 1,
                        );
                    }
                    out.push(')');
                }
                pa::WitnessPayload::Record(fields) => {
                    out.push_str(" { ");
                    for (i, (name, w)) in fields.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        out.push_str(name);
                        out.push_str(": ");
                        let field_ty = variant_fields
                            .as_ref()
                            .and_then(|fs| {
                                fs.iter().find(|(n, _)| n.as_deref() == Some(name.as_str()))
                            })
                            .map(|(_, t)| t)
                            .unwrap_or(&shape);
                        format_witness_into(
                            env,
                            w,
                            field_ty,
                            &place.variant_record_field(tag.clone(), name.as_str()),
                            list_views,
                            out,
                            depth + 1,
                        );
                    }
                    out.push_str(" }");
                }
            }
        }
        pa::MissingWitness::Tuple(elems) => {
            if elems.is_empty() {
                out.push_str("()");
                return;
            }
            let elem_tys = match &shape {
                Term::Tuple(ts) => Some(ts.as_slice()),
                _ => None,
            };
            out.push('(');
            for (i, w) in elems.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                let ety = elem_tys.and_then(|ts| ts.get(i)).unwrap_or(&shape);
                format_witness_into(
                    env,
                    w,
                    ety,
                    &place.tuple_field(i),
                    list_views,
                    out,
                    depth + 1,
                );
            }
            if elems.len() == 1 {
                out.push(',');
            }
            out.push(')');
        }
        pa::MissingWitness::Record(fields) => {
            let field_tys = match &shape {
                Term::Record(fs) => Some(fs.clone()),
                Term::Nominal {
                    declaration,
                    arguments,
                } => env
                    .struct_def(*declaration)
                    .and_then(|def| env.instantiate_struct_fields(def, arguments)),
                _ => None,
            };
            out.push_str("{ ");
            for (i, (name, w)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(name);
                out.push_str(": ");
                let fty = field_tys
                    .as_ref()
                    .and_then(|fs| fs.iter().find(|(n, _)| n == name))
                    .map(|(_, t)| t)
                    .unwrap_or(&shape);
                format_witness_into(
                    env,
                    w,
                    fty,
                    &place.record_field(name.as_str()),
                    list_views,
                    out,
                    depth + 1,
                );
            }
            out.push_str(" }");
        }
        pa::MissingWitness::List(elems) => {
            let elem_ty = find_list_view_for_place(place, list_views)
                .map(|ops| &ops.element_type)
                .or_else(|| env.list_element_type(ty))
                .unwrap_or(&shape);
            out.push('[');
            for (i, w) in elems.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_witness_into(
                    env,
                    w,
                    elem_ty,
                    &place.list_elem(i),
                    list_views,
                    out,
                    depth + 1,
                );
            }
            out.push(']');
        }
        pa::MissingWitness::BitVec(bv) => {
            let width = bv.width();
            let _ = write!(out, "0b");
            if width == 0 {
                out.push('0');
            } else {
                for bit in (0..width).rev() {
                    let word = bv.words().get((bit / 64) as usize).copied().unwrap_or(0);
                    let set = ((word >> (bit % 64)) & 1) != 0;
                    out.push(if set { '1' } else { '0' });
                }
            }
        }
    }
}
