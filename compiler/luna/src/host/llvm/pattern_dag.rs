//! Lower checked `pattern_automata` decision DAGs from `iir::Package::match_plans` to LLVM IR.

use super::*;
use nir::match_plan::{ListViewOps, MatchPlan};
use nir::pattern_automata as pa;
use nir::pattern_automata::{
    BitInterval, DagNode, FiniteInt, IntInterval, LenInterval, LowerPoint, NodeId, Place,
    PlaceStep, SwitchKind, UpperPoint, VariantTag,
};
use nir::program::{MatchArm, Pattern};

struct ArmMeta<'a> {
    outer_guard: Option<&'a Expr>,
    remaps: BTreeMap<LocalId, LocalId>,
    binding_tys: BTreeMap<LocalId, Term>,
    exported_locals: BTreeSet<LocalId>,
}

#[derive(Clone, Copy)]
struct ListCallRef<'a> {
    obs_binding: LocalId,
    expr: &'a Expr,
}

#[derive(Default)]
struct HarvestedListPlace<'a> {
    len: Option<ListCallRef<'a>>,
    elems: BTreeMap<usize, ListCallRef<'a>>,
    rests: BTreeMap<usize, ListCallRef<'a>>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ListCacheOp {
    Len,
    Elem(usize),
    Rest(usize),
}

struct ListCacheSlot {
    obs_binding: LocalId,
    expr_idx: CacheExprSource,
    parent_slot: Option<Slot>,
    val_slot: Slot,
    init_flag: Slot,
}

enum CacheExprSource {
    Borrowed(*const Expr),
}

impl CacheExprSource {
    fn new(expr: &Expr) -> Self {
        Self::Borrowed(expr as *const Expr)
    }

    unsafe fn get(&self) -> &Expr {
        let Self::Borrowed(ptr) = *self;
        unsafe { &*ptr }
    }
}

struct ValidatedPlaceInfo {
    term: Term,
    ty: Ty,
}

struct DagPlanValidation<'a> {
    plan: &'a MatchPlan,
    arm_metas: Vec<ArmMeta<'a>>,
    harvested_lists: BTreeMap<Place, HarvestedListPlace<'a>>,
    place_infos: BTreeMap<Place, ValidatedPlaceInfo>,
    active_error_branches: BTreeMap<NodeId, BTreeMap<Place, Option<usize>>>,
    needed_list_ops: BTreeSet<(Place, ListCacheOp)>,
    reachable_nodes: Vec<NodeId>,
    reachable_arms: BTreeSet<usize>,
    arm_exported_tys: BTreeMap<usize, BTreeMap<LocalId, Term>>,
}

impl Emitter<'_> {
    pub(super) fn try_lower_match_dag(
        &mut self,
        source: nir::program::SourceRange,
        value: &Value,
        value_ty: &Term,
        arms: &[MatchArm],
        match_ty: &Term,
        depth: usize,
        selected: Option<&effects::Environment>,
    ) -> Result<Option<Value>, String> {
        let decl = self.current_instance().map(|inst| inst.key.declaration);
        let Some(plan) = self.graph.match_plans.get(decl, source) else {
            return Ok(None);
        };
        if !plan.structural_exact
            || plan.dag.nodes.is_empty()
            || plan.dag.root.0 >= plan.dag.nodes.len()
        {
            return Ok(None);
        }
        if arms.iter().any(|arm| arm.guard.is_some()) {
            return Ok(None);
        }
        if !self.cleanup_compatible_with_dag(arms) {
            return Ok(None);
        }
        let Some(validated) = self.validate_match_dag(plan, value, value_ty, arms)? else {
            return Ok(None);
        };
        let result =
            self.emit_match_dag(validated, value, value_ty, arms, match_ty, depth, selected)?;
        Ok(Some(result))
    }

    fn cleanup_compatible_with_dag(&self, arms: &[MatchArm]) -> bool {
        let Some(schedule) = self.cleanup else {
            return true;
        };
        for arm in arms {
            if schedule
                .pattern_orders
                .iter()
                .any(|order| order.source == arm.body.source)
            {
                return false;
            }
            if pattern_has_active_then_cleanup(&arm.pattern, schedule) {
                return false;
            }
        }
        true
    }

    fn validate_match_dag<'a>(
        &self,
        plan: &'a MatchPlan,
        value: &Value,
        value_ty: &Term,
        arms: &'a [MatchArm],
    ) -> Result<Option<DagPlanValidation<'a>>, String> {
        let mut arm_metas = Vec::with_capacity(arms.len());
        let mut harvested_lists: BTreeMap<Place, HarvestedListPlace<'a>> = BTreeMap::new();
        for arm in arms {
            let (outer_guard, structural_pat) = match &arm.pattern {
                Pattern::Guard { pattern, condition } => {
                    (Some(condition.as_ref()), pattern.as_ref())
                }
                other => (None, other),
            };
            let mut remaps = BTreeMap::new();
            let mut binding_tys = BTreeMap::new();
            let mut exported_locals = BTreeSet::new();
            if !self.harvest_pattern(
                structural_pat,
                &Place::root(),
                value_ty,
                &plan.list_views,
                &mut harvested_lists,
                &mut remaps,
                &mut binding_tys,
                &mut exported_locals,
                true,
                0,
            )? {
                return Ok(None);
            }
            // Also record direct bindings outside list/bitvec if not yet seen; resolve remaps.
            arm_metas.push(ArmMeta {
                outer_guard,
                remaps,
                binding_tys,
                exported_locals,
            });
        }

        // Validate DAG reachability, error branch provenance, places, switches, and bindings.
        let nodes = &plan.dag.nodes;
        let mut active_error_branches: BTreeMap<NodeId, BTreeMap<Place, Option<usize>>> =
            BTreeMap::new();
        let mut queue = std::collections::VecDeque::new();
        let mut in_queue = BTreeSet::new();
        active_error_branches.insert(plan.dag.root, BTreeMap::new());
        queue.push_back(plan.dag.root);
        in_queue.insert(plan.dag.root);

        let mut visit_budget = nodes.len().saturating_mul(8).saturating_add(64);
        while let Some(node_id) = queue.pop_front() {
            in_queue.remove(&node_id);
            if visit_budget == 0 {
                return Ok(None);
            }
            visit_budget -= 1;
            let Some(node) = nodes.get(node_id.0) else {
                return Ok(None);
            };
            let incoming_errs = active_error_branches
                .get(&node_id)
                .cloned()
                .unwrap_or_default();
            let child_updates = |child: NodeId,
                                 extra: Option<(Place, Option<usize>)>,
                                 map: &mut BTreeMap<NodeId, BTreeMap<Place, Option<usize>>>,
                                 queue: &mut std::collections::VecDeque<NodeId>,
                                 in_queue: &mut BTreeSet<NodeId>| {
                let mut next_env = incoming_errs.clone();
                if let Some((err_place, err_idx)) = extra {
                    next_env.insert(err_place, err_idx);
                }
                let changed = match map.get_mut(&child) {
                    None => {
                        map.insert(child, next_env);
                        true
                    }
                    Some(existing) => {
                        let mut modified = false;
                        for (k, v) in next_env {
                            match existing.get_mut(&k) {
                                None => {
                                    existing.insert(k, v);
                                    modified = true;
                                }
                                Some(prev) if *prev != v && prev.is_some() => {
                                    *prev = None;
                                    modified = true;
                                }
                                _ => {}
                            }
                        }
                        modified
                    }
                };
                if changed && in_queue.insert(child) {
                    queue.push_back(child);
                }
            };

            match node {
                DagNode::Switch { place, kind } => match kind {
                    SwitchKind::Bool { on_false, on_true } => {
                        child_updates(
                            *on_false,
                            None,
                            &mut active_error_branches,
                            &mut queue,
                            &mut in_queue,
                        );
                        child_updates(
                            *on_true,
                            None,
                            &mut active_error_branches,
                            &mut queue,
                            &mut in_queue,
                        );
                    }
                    SwitchKind::Optional {
                        on_null,
                        on_present,
                    } => {
                        child_updates(
                            *on_null,
                            None,
                            &mut active_error_branches,
                            &mut queue,
                            &mut in_queue,
                        );
                        child_updates(
                            *on_present,
                            None,
                            &mut active_error_branches,
                            &mut queue,
                            &mut in_queue,
                        );
                    }
                    SwitchKind::Error {
                        on_ok,
                        cases,
                        default_err,
                    } => {
                        let Some(info) = self.resolve_place_type(
                            place,
                            value_ty,
                            value,
                            &harvested_lists,
                            &incoming_errs,
                        )?
                        else {
                            return Ok(None);
                        };
                        let Ty::ErrorResult(layout) = &info.ty else {
                            return Ok(None);
                        };
                        child_updates(
                            *on_ok,
                            None,
                            &mut active_error_branches,
                            &mut queue,
                            &mut in_queue,
                        );
                        for (tag, target) in cases {
                            let Some(idx) = self.resolve_error_tag_index(layout, tag) else {
                                return Ok(None);
                            };
                            child_updates(
                                *target,
                                Some((place.clone(), Some(idx))),
                                &mut active_error_branches,
                                &mut queue,
                                &mut in_queue,
                            );
                        }
                        if let Some(def) = default_err {
                            let sole_missing = if cases.len() + 2 == layout.branches.len() {
                                (1..layout.branches.len()).find(|i| {
                                    !cases.keys().any(|tag| {
                                        self.resolve_error_tag_index(layout, tag) == Some(*i)
                                    })
                                })
                            } else {
                                None
                            };
                            child_updates(
                                *def,
                                Some((place.clone(), sole_missing)),
                                &mut active_error_branches,
                                &mut queue,
                                &mut in_queue,
                            );
                        }
                    }
                    SwitchKind::Enum { cases, default } => {
                        for &target in cases.values() {
                            child_updates(
                                target,
                                None,
                                &mut active_error_branches,
                                &mut queue,
                                &mut in_queue,
                            );
                        }
                        if let Some(def) = default {
                            child_updates(
                                *def,
                                None,
                                &mut active_error_branches,
                                &mut queue,
                                &mut in_queue,
                            );
                        }
                    }
                    SwitchKind::Int { cases, default } => {
                        for &(_, target) in cases {
                            child_updates(
                                target,
                                None,
                                &mut active_error_branches,
                                &mut queue,
                                &mut in_queue,
                            );
                        }
                        if let Some(def) = default {
                            child_updates(
                                *def,
                                None,
                                &mut active_error_branches,
                                &mut queue,
                                &mut in_queue,
                            );
                        }
                    }
                    SwitchKind::Char { cases, default } => {
                        for &(_, target) in cases {
                            child_updates(
                                target,
                                None,
                                &mut active_error_branches,
                                &mut queue,
                                &mut in_queue,
                            );
                        }
                        if let Some(def) = default {
                            child_updates(
                                *def,
                                None,
                                &mut active_error_branches,
                                &mut queue,
                                &mut in_queue,
                            );
                        }
                    }
                    SwitchKind::ListLen { cases, default } => {
                        for &(_, target) in cases {
                            child_updates(
                                target,
                                None,
                                &mut active_error_branches,
                                &mut queue,
                                &mut in_queue,
                            );
                        }
                        if let Some(def) = default {
                            child_updates(
                                *def,
                                None,
                                &mut active_error_branches,
                                &mut queue,
                                &mut in_queue,
                            );
                        }
                    }
                    SwitchKind::BitSlice { cases, default, .. } => {
                        for &(_, target) in cases {
                            child_updates(
                                target,
                                None,
                                &mut active_error_branches,
                                &mut queue,
                                &mut in_queue,
                            );
                        }
                        if let Some(def) = default {
                            child_updates(
                                *def,
                                None,
                                &mut active_error_branches,
                                &mut queue,
                                &mut in_queue,
                            );
                        }
                    }
                },
                DagNode::Bind { next, .. } => {
                    child_updates(
                        *next,
                        None,
                        &mut active_error_branches,
                        &mut queue,
                        &mut in_queue,
                    );
                }
                DagNode::GuardCheck {
                    on_pass, on_fail, ..
                } => {
                    child_updates(
                        *on_pass,
                        None,
                        &mut active_error_branches,
                        &mut queue,
                        &mut in_queue,
                    );
                    child_updates(
                        *on_fail,
                        None,
                        &mut active_error_branches,
                        &mut queue,
                        &mut in_queue,
                    );
                }
                DagNode::Leaf { .. } | DagNode::Fail => {}
            }
        }

        let reachable_nodes: Vec<NodeId> = active_error_branches.keys().copied().collect();
        let mut place_infos: BTreeMap<Place, ValidatedPlaceInfo> = BTreeMap::new();
        let mut needed_list_ops: BTreeSet<(Place, ListCacheOp)> = BTreeSet::new();
        let mut reachable_arms: BTreeSet<usize> = BTreeSet::new();
        let mut arm_exported_tys: BTreeMap<usize, BTreeMap<LocalId, Term>> = BTreeMap::new();

        for &node_id in &reachable_nodes {
            let err_env = &active_error_branches[&node_id];
            match &nodes[node_id.0] {
                DagNode::Switch { place, kind } => {
                    let Some(info) = self.validate_place(
                        place,
                        value_ty,
                        value,
                        &harvested_lists,
                        err_env,
                        &mut place_infos,
                        &mut needed_list_ops,
                    )?
                    else {
                        return Ok(None);
                    };
                    match kind {
                        SwitchKind::Bool { .. } => {
                            if info.ty != Ty::Bool {
                                return Ok(None);
                            }
                        }
                        SwitchKind::Optional { .. } => {
                            if !matches!(info.ty, Ty::Optional(_)) {
                                return Ok(None);
                            }
                        }
                        SwitchKind::Error { cases, .. } => {
                            let Ty::ErrorResult(layout) = &info.ty else {
                                return Ok(None);
                            };
                            for tag in cases.keys() {
                                if self.resolve_error_tag_index(layout, tag).is_none() {
                                    return Ok(None);
                                }
                            }
                        }
                        SwitchKind::Enum { cases, .. } => {
                            let Ty::Enum(layout) = &info.ty else {
                                return Ok(None);
                            };
                            for tag in cases.keys() {
                                let Some(tag_name) = variant_tag_name(tag) else {
                                    return Ok(None);
                                };
                                if !layout.names.iter().any(|name| name == tag_name) {
                                    return Ok(None);
                                }
                            }
                        }
                        SwitchKind::Int { cases, .. } => {
                            let Ty::Int(bits, signed) = info.ty else {
                                return Ok(None);
                            };
                            if !(1..=128).contains(&bits) {
                                return Ok(None);
                            }
                            for &(iv, _) in cases {
                                if signed {
                                    if clamp_signed_interval(iv, bits).is_none() {
                                        return Ok(None);
                                    }
                                } else if clamp_unsigned_interval(iv, bits).is_none() {
                                    return Ok(None);
                                }
                            }
                        }
                        SwitchKind::Char { cases, .. } => {
                            if info.ty != Ty::Int(32, false) {
                                return Ok(None);
                            }
                            for &(iv, _) in cases {
                                if iv.lo > iv.hi {
                                    return Ok(None);
                                }
                            }
                        }
                        SwitchKind::ListLen { cases, .. } => {
                            let Some(place_ops) = harvested_lists.get(place) else {
                                return Ok(None);
                            };
                            let Some(len_ref) = place_ops.len else {
                                return Ok(None);
                            };
                            let len_ty = repr(self.graph, &len_ref.expr.ty, self.checked)?;
                            let Ty::Int(bits, false) = len_ty else {
                                return Ok(None);
                            };
                            if !(1..=128).contains(&bits) {
                                return Ok(None);
                            }
                            for &(iv, _) in cases {
                                if !len_interval_fits(iv, bits) {
                                    return Ok(None);
                                }
                            }
                            needed_list_ops.insert((place.clone(), ListCacheOp::Len));
                        }
                        SwitchKind::BitSlice { lo, hi, cases, .. } => {
                            let Some(parent_w) = bit_width(&info.ty) else {
                                return Ok(None);
                            };
                            if lo > hi || *hi > parent_w {
                                return Ok(None);
                            }
                            let slice_w = hi - lo;
                            if slice_w > bits::MAX_BIT_WIDTH {
                                return Ok(None);
                            }
                            for (iv, _) in cases {
                                if iv.width() != slice_w {
                                    return Ok(None);
                                }
                            }
                        }
                    }
                }
                DagNode::Bind {
                    arm_index,
                    bindings,
                    ..
                } => {
                    if !self.validate_bindings(
                        *arm_index,
                        bindings,
                        &arm_metas,
                        value_ty,
                        value,
                        &harvested_lists,
                        err_env,
                        &mut place_infos,
                        &mut needed_list_ops,
                        &mut arm_exported_tys,
                    )? {
                        return Ok(None);
                    }
                }
                DagNode::GuardCheck {
                    guard_id, bindings, ..
                } => {
                    let Some(meta) = arm_metas.get(*guard_id) else {
                        return Ok(None);
                    };
                    if meta.outer_guard.is_none() {
                        return Ok(None);
                    }
                    if !self.validate_bindings(
                        *guard_id,
                        bindings,
                        &arm_metas,
                        value_ty,
                        value,
                        &harvested_lists,
                        err_env,
                        &mut place_infos,
                        &mut needed_list_ops,
                        &mut arm_exported_tys,
                    )? {
                        return Ok(None);
                    }
                }
                DagNode::Leaf {
                    arm_index,
                    bindings,
                } => {
                    if *arm_index >= arms.len() {
                        return Ok(None);
                    }
                    reachable_arms.insert(*arm_index);
                    if !self.validate_bindings(
                        *arm_index,
                        bindings,
                        &arm_metas,
                        value_ty,
                        value,
                        &harvested_lists,
                        err_env,
                        &mut place_infos,
                        &mut needed_list_ops,
                        &mut arm_exported_tys,
                    )? {
                        return Ok(None);
                    }
                }
                DagNode::Fail => {}
            }
        }

        // Ensure every binding exported by any reachable arm has been statically typed.
        for &arm_index in &reachable_arms {
            let meta = &arm_metas[arm_index];
            for id in &meta.exported_locals {
                let target_id = resolve_remap_id(*id, &meta.remaps);
                if !arm_exported_tys
                    .get(&arm_index)
                    .is_some_and(|map| map.contains_key(&target_id))
                {
                    if let Some(ty) = meta.binding_tys.get(id) {
                        arm_exported_tys
                            .entry(arm_index)
                            .or_default()
                            .insert(target_id, ty.clone());
                    } else {
                        return Ok(None);
                    }
                }
            }
        }

        Ok(Some(DagPlanValidation {
            plan,
            arm_metas,
            harvested_lists,
            place_infos,
            active_error_branches,
            needed_list_ops,
            reachable_nodes,
            reachable_arms,
            arm_exported_tys,
        }))
    }

    fn validate_bindings<'a>(
        &self,
        arm_index: usize,
        bindings: &[pa::Binding],
        arm_metas: &[ArmMeta<'a>],
        value_ty: &Term,
        value: &Value,
        harvested_lists: &BTreeMap<Place, HarvestedListPlace<'a>>,
        err_env: &BTreeMap<Place, Option<usize>>,
        place_infos: &mut BTreeMap<Place, ValidatedPlaceInfo>,
        needed_list_ops: &mut BTreeSet<(Place, ListCacheOp)>,
        arm_exported_tys: &mut BTreeMap<usize, BTreeMap<LocalId, Term>>,
    ) -> Result<bool, String> {
        let Some(meta) = arm_metas.get(arm_index) else {
            return Ok(false);
        };
        for binding in bindings {
            let Some(raw_id) = parse_local_id(&binding.name) else {
                return Ok(false);
            };
            let target_id = resolve_remap_id(raw_id, &meta.remaps);
            let Some(place_info) = self.validate_place(
                &binding.place,
                value_ty,
                value,
                harvested_lists,
                err_env,
                place_infos,
                needed_list_ops,
            )?
            else {
                return Ok(false);
            };
            let binding_term = meta
                .binding_tys
                .get(&target_id)
                .or_else(|| meta.binding_tys.get(&raw_id))
                .cloned()
                .unwrap_or_else(|| place_info.term.clone());
            let target_ty = repr(self.graph, &binding_term, self.checked)?;
            if target_ty != place_info.ty && !can_convert_bitslice_ty(&place_info.ty, &target_ty) {
                return Ok(false);
            }
            let Some(receipt_path) =
                place_to_receipt_path(&binding.place, value_ty, err_env, self.graph)
            else {
                return Ok(false);
            };
            let _ = receipt_path;
            arm_exported_tys
                .entry(arm_index)
                .or_default()
                .insert(target_id, binding_term);
        }
        Ok(true)
    }

    fn validate_place<'a>(
        &self,
        place: &Place,
        value_ty: &Term,
        value: &Value,
        harvested_lists: &BTreeMap<Place, HarvestedListPlace<'a>>,
        err_env: &BTreeMap<Place, Option<usize>>,
        place_infos: &mut BTreeMap<Place, ValidatedPlaceInfo>,
        needed_list_ops: &mut BTreeSet<(Place, ListCacheOp)>,
    ) -> Result<Option<ValidatedPlaceInfo>, String> {
        let mut cur_steps = Vec::with_capacity(place.steps.len());
        for step in &place.steps {
            match step {
                PlaceStep::ListElem(idx) => {
                    let parent = Place {
                        steps: cur_steps.clone(),
                    };
                    if !harvested_lists
                        .get(&parent)
                        .is_some_and(|h| h.elems.contains_key(idx))
                    {
                        return Ok(None);
                    }
                    needed_list_ops.insert((parent, ListCacheOp::Elem(*idx)));
                }
                PlaceStep::ListRest(from) => {
                    let parent = Place {
                        steps: cur_steps.clone(),
                    };
                    if !harvested_lists
                        .get(&parent)
                        .is_some_and(|h| h.rests.contains_key(from))
                    {
                        return Ok(None);
                    }
                    needed_list_ops.insert((parent, ListCacheOp::Rest(*from)));
                }
                _ => {}
            }
            if matches!(step, PlaceStep::ListElem(_) | PlaceStep::ListRest(_))
                && !cur_steps.is_empty()
            {
                let parent = Place {
                    steps: cur_steps.clone(),
                };
                if !place_infos.contains_key(&parent) {
                    let Some(parent_info) = self.resolve_place_type(
                        &parent,
                        value_ty,
                        value,
                        harvested_lists,
                        err_env,
                    )?
                    else {
                        return Ok(None);
                    };
                    place_infos.insert(parent, parent_info);
                }
            }
            cur_steps.push(step.clone());
        }
        let Some(info) =
            self.resolve_place_type(place, value_ty, value, harvested_lists, err_env)?
        else {
            return Ok(None);
        };
        place_infos.insert(
            place.clone(),
            ValidatedPlaceInfo {
                term: info.term.clone(),
                ty: info.ty.clone(),
            },
        );
        Ok(Some(info))
    }

    fn resolve_place_type<'a>(
        &self,
        place: &Place,
        root_term: &Term,
        root_val: &Value,
        harvested_lists: &BTreeMap<Place, HarvestedListPlace<'a>>,
        err_env: &BTreeMap<Place, Option<usize>>,
    ) -> Result<Option<ValidatedPlaceInfo>, String> {
        let mut cur_term = root_term.clone();
        let mut cur_ty = root_val.ty.clone();
        let mut cur_place = Place::root();
        for step in &place.steps {
            let next_term_and_ty: Option<(Term, Ty)> = match step {
                PlaceStep::TupleField(idx) => {
                    let fields = tuple_elements_term(self.graph, &cur_term);
                    let Some(field_term) = fields.and_then(|fs| fs.get(*idx)).cloned() else {
                        return Ok(None);
                    };
                    let field_ty = match &cur_ty {
                        Ty::Record(fs) | Ty::PackedRecord(fs) => fs.get(*idx).cloned(),
                        _ => None,
                    };
                    let Some(field_ty) = field_ty else {
                        return Ok(None);
                    };
                    Some((field_term, field_ty))
                }
                PlaceStep::RecordField(name) => {
                    let unwrapped = unwrap_newtype_term(self.graph, &cur_term);
                    let field_info = match unwrapped {
                        Term::Record(fields) => fields
                            .iter()
                            .enumerate()
                            .find(|(_, (n, _))| n == name)
                            .map(|(i, (_, t))| (i, t.clone())),
                        Term::Nominal { .. } => self
                            .graph
                            .nominals
                            .iter()
                            .find(|inst| inst.ty == *unwrapped)
                            .and_then(|inst| match &inst.definition {
                                iir::NominalDefinition::Struct(def) => def
                                    .fields
                                    .iter()
                                    .enumerate()
                                    .find(|(_, (n, _))| n == name)
                                    .map(|(i, (_, t))| (i, t.clone())),
                                _ => None,
                            }),
                        _ => None,
                    };
                    let Some((idx, field_term)) = field_info else {
                        return Ok(None);
                    };
                    let field_ty = match &cur_ty {
                        Ty::Record(fs) | Ty::PackedRecord(fs) => fs.get(idx).cloned(),
                        _ => None,
                    };
                    let Some(field_ty) = field_ty else {
                        return Ok(None);
                    };
                    Some((field_term, field_ty))
                }
                PlaceStep::OptionalInner => {
                    let unwrapped = unwrap_newtype_term(self.graph, &cur_term);
                    let Term::Optional(inner_term) = unwrapped else {
                        return Ok(None);
                    };
                    let Ty::Optional(opt) = &cur_ty else {
                        return Ok(None);
                    };
                    Some(((**inner_term).clone(), opt.payload.clone()))
                }
                PlaceStep::ErrorOkInner => {
                    let unwrapped = unwrap_newtype_term(self.graph, &cur_term);
                    let Term::ErrorResult { result, .. } = unwrapped else {
                        return Ok(None);
                    };
                    let Ty::ErrorResult(layout) = &cur_ty else {
                        return Ok(None);
                    };
                    let Some((_, payload_ty)) = layout.branches.first() else {
                        return Ok(None);
                    };
                    Some(((**result).clone(), payload_ty.clone()))
                }
                PlaceStep::ErrorErrInner => {
                    let Some(Some(err_idx)) = err_env.get(&cur_place) else {
                        return Ok(None);
                    };
                    let Ty::ErrorResult(layout) = &cur_ty else {
                        return Ok(None);
                    };
                    let Some((Some(err_term), err_ty)) = layout.branches.get(*err_idx) else {
                        return Ok(None);
                    };
                    Some((err_term.clone(), err_ty.clone()))
                }
                PlaceStep::VariantTupleField { tag, index } => {
                    let unwrapped = unwrap_newtype_term(self.graph, &cur_term);
                    let Some(inst) = self.graph.nominals.iter().find(|i| i.ty == *unwrapped) else {
                        return Ok(None);
                    };
                    let iir::NominalDefinition::Enum(def) = &inst.definition else {
                        return Ok(None);
                    };
                    let Some(tag_name) = variant_tag_name(tag) else {
                        return Ok(None);
                    };
                    let Some(v_idx) = def.variants.iter().position(|v| v.name == tag_name) else {
                        return Ok(None);
                    };
                    let Some(field_term) = def.variants[v_idx].fields.get(*index).cloned() else {
                        return Ok(None);
                    };
                    let field_ty = repr(self.graph, &field_term, self.checked)?;
                    Some((field_term, field_ty))
                }
                PlaceStep::VariantRecordField { tag, field } => {
                    let unwrapped = unwrap_newtype_term(self.graph, &cur_term);
                    let Some(inst) = self.graph.nominals.iter().find(|i| i.ty == *unwrapped) else {
                        return Ok(None);
                    };
                    let iir::NominalDefinition::Enum(def) = &inst.definition else {
                        return Ok(None);
                    };
                    let Some(tag_name) = variant_tag_name(tag) else {
                        return Ok(None);
                    };
                    let Some(v_idx) = def.variants.iter().position(|v| v.name == tag_name) else {
                        return Ok(None);
                    };
                    let variant = &def.variants[v_idx];
                    let nir::program::VariantShape::Named(names) = &variant.shape else {
                        return Ok(None);
                    };
                    let Some(f_idx) = names.iter().position(|n| n == field) else {
                        return Ok(None);
                    };
                    let Some(field_term) = variant.fields.get(f_idx).cloned() else {
                        return Ok(None);
                    };
                    let field_ty = repr(self.graph, &field_term, self.checked)?;
                    Some((field_term, field_ty))
                }
                PlaceStep::ListElem(idx) => {
                    let Some(call_ref) = harvested_lists
                        .get(&cur_place)
                        .and_then(|h| h.elems.get(idx))
                    else {
                        return Ok(None);
                    };
                    let elem_ty = repr(self.graph, &call_ref.expr.ty, self.checked)?;
                    Some((call_ref.expr.ty.clone(), elem_ty))
                }
                PlaceStep::ListRest(from) => {
                    let Some(call_ref) = harvested_lists
                        .get(&cur_place)
                        .and_then(|h| h.rests.get(from))
                    else {
                        return Ok(None);
                    };
                    let rest_ty = repr(self.graph, &call_ref.expr.ty, self.checked)?;
                    Some((call_ref.expr.ty.clone(), rest_ty))
                }
                PlaceStep::BitSlice { lo, hi, ty: _ } => {
                    let Some(parent_w) = bit_width(&cur_ty) else {
                        return Ok(None);
                    };
                    if lo > hi || *hi > parent_w {
                        return Ok(None);
                    }
                    let slice_w = hi - lo;
                    if slice_w > bits::MAX_BIT_WIDTH {
                        return Ok(None);
                    }
                    let slice_term = Term::Primitive(P::BitVec(slice_w as usize));
                    let slice_ty = Ty::Int(slice_w, false);
                    Some((slice_term, slice_ty))
                }
            };
            let Some((next_term, next_ty)) = next_term_and_ty else {
                return Ok(None);
            };
            cur_term = next_term;
            cur_ty = next_ty;
            cur_place.steps.push(step.clone());
        }
        Ok(Some(ValidatedPlaceInfo {
            term: cur_term,
            ty: cur_ty,
        }))
    }

    fn resolve_error_tag_index(
        &self,
        layout: &ErrorResultLayout,
        tag: &pa::ErrorTag,
    ) -> Option<usize> {
        let mut matched = None;
        for (idx, (label, _)) in layout.branches.iter().enumerate().skip(1) {
            let Some(err_term) = label else {
                continue;
            };
            let direct = err_term.to_string() == tag.0;
            let unwrapped = unwrap_newtype_term(self.graph, err_term).to_string() == tag.0;
            if direct || unwrapped {
                if matched.is_some() {
                    return None;
                }
                matched = Some(idx);
            }
        }
        matched
    }

    fn harvest_pattern<'a>(
        &self,
        pat: &'a Pattern,
        place: &Place,
        cur_ty: &Term,
        list_views: &BTreeMap<Place, ListViewOps>,
        harvested_lists: &mut BTreeMap<Place, HarvestedListPlace<'a>>,
        remaps: &mut BTreeMap<LocalId, LocalId>,
        binding_tys: &mut BTreeMap<LocalId, Term>,
        exported_locals: &mut BTreeSet<LocalId>,
        is_direct_export: bool,
        depth: usize,
    ) -> Result<bool, String> {
        if depth > 128 {
            return Ok(false);
        }
        match pat {
            Pattern::Wild | Pattern::Unit | Pattern::Literal(_) | Pattern::OptionalAbsent => {
                Ok(true)
            }
            Pattern::Binding { binding, inner } => {
                binding_tys
                    .entry(*binding)
                    .or_insert_with(|| cur_ty.clone());
                if is_direct_export {
                    exported_locals.insert(*binding);
                }
                self.harvest_pattern(
                    inner,
                    place,
                    cur_ty,
                    list_views,
                    harvested_lists,
                    remaps,
                    binding_tys,
                    exported_locals,
                    is_direct_export,
                    depth + 1,
                )
            }
            Pattern::Remap { pattern, bindings } => {
                for (from, to) in bindings {
                    remaps.insert(*from, *to);
                }
                let mut inner_exports = BTreeSet::new();
                if !self.harvest_pattern(
                    pattern,
                    place,
                    cur_ty,
                    list_views,
                    harvested_lists,
                    remaps,
                    binding_tys,
                    &mut inner_exports,
                    is_direct_export,
                    depth + 1,
                )? {
                    return Ok(false);
                }
                if is_direct_export {
                    for id in inner_exports {
                        exported_locals.insert(resolve_remap_id(id, remaps));
                    }
                }
                for (from, to) in bindings {
                    if let Some(ty) = binding_tys.get(from).cloned() {
                        binding_tys.entry(*to).or_insert(ty);
                    }
                }
                Ok(true)
            }
            Pattern::OptionalPresent(inner) => {
                let unwrapped = unwrap_newtype_term(self.graph, cur_ty);
                let Term::Optional(inner_ty) = unwrapped else {
                    return Ok(false);
                };
                self.harvest_pattern(
                    inner,
                    &place.optional_inner(),
                    inner_ty,
                    list_views,
                    harvested_lists,
                    remaps,
                    binding_tys,
                    exported_locals,
                    is_direct_export,
                    depth + 1,
                )
            }
            Pattern::ErrorResult { label, inner } => {
                let unwrapped = unwrap_newtype_term(self.graph, cur_ty);
                let Term::ErrorResult { errors, result } = unwrapped else {
                    return Ok(false);
                };
                let (next_place, next_ty) = match label {
                    None => (place.error_ok_inner(), result.as_ref()),
                    Some(err_ty) => (
                        place.error_err_inner(),
                        errors.iter().find(|e| *e == err_ty).unwrap_or(err_ty),
                    ),
                };
                self.harvest_pattern(
                    inner,
                    &next_place,
                    next_ty,
                    list_views,
                    harvested_lists,
                    remaps,
                    binding_tys,
                    exported_locals,
                    is_direct_export,
                    depth + 1,
                )
            }
            Pattern::Tuple(elems) => {
                let Some(elem_tys) = tuple_elements_term(self.graph, cur_ty) else {
                    return Ok(false);
                };
                if elems.len() != elem_tys.len() {
                    return Ok(false);
                }
                for (idx, (sub, elem_ty)) in elems.iter().zip(elem_tys.iter()).enumerate() {
                    if !self.harvest_pattern(
                        sub,
                        &place.tuple_field(idx),
                        elem_ty,
                        list_views,
                        harvested_lists,
                        remaps,
                        binding_tys,
                        exported_locals,
                        is_direct_export,
                        depth + 1,
                    )? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Pattern::AnonymousRecord { names, fields } => {
                let unwrapped = unwrap_newtype_term(self.graph, cur_ty);
                let Term::Record(rec_fields) = unwrapped else {
                    return Ok(false);
                };
                for (name, sub) in names.iter().zip(fields.iter()) {
                    let Some((_, field_ty)) = rec_fields.iter().find(|(n, _)| n == name) else {
                        return Ok(false);
                    };
                    if !self.harvest_pattern(
                        sub,
                        &place.record_field(name.as_str()),
                        field_ty,
                        list_views,
                        harvested_lists,
                        remaps,
                        binding_tys,
                        exported_locals,
                        is_direct_export,
                        depth + 1,
                    )? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Pattern::Record {
                declaration: _,
                fields,
            } => {
                let unwrapped = unwrap_newtype_term(self.graph, cur_ty);
                let Some(inst) = self.graph.nominals.iter().find(|i| i.ty == *unwrapped) else {
                    return Ok(false);
                };
                let iir::NominalDefinition::Struct(def) = &inst.definition else {
                    return Ok(false);
                };
                if def.fields.len() != fields.len() {
                    return Ok(false);
                }
                for ((name, field_ty), sub) in def.fields.iter().zip(fields.iter()) {
                    if !self.harvest_pattern(
                        sub,
                        &place.record_field(name.as_str()),
                        field_ty,
                        list_views,
                        harvested_lists,
                        remaps,
                        binding_tys,
                        exported_locals,
                        is_direct_export,
                        depth + 1,
                    )? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Pattern::Variant {
                declaration: _,
                index,
                fields,
            } => {
                let unwrapped = unwrap_newtype_term(self.graph, cur_ty);
                let Some(inst) = self.graph.nominals.iter().find(|i| i.ty == *unwrapped) else {
                    return Ok(false);
                };
                let iir::NominalDefinition::Enum(def) = &inst.definition else {
                    return Ok(false);
                };
                let Some(variant) = def.variants.get(*index) else {
                    return Ok(false);
                };
                if variant.fields.len() != fields.len() {
                    return Ok(false);
                }
                let tag = pa::VariantTag::named(variant.name.clone());
                for (i, (sub, field_ty)) in fields.iter().zip(variant.fields.iter()).enumerate() {
                    let sub_place = match &variant.shape {
                        nir::program::VariantShape::Unit => return Ok(false),
                        nir::program::VariantShape::Tuple => {
                            place.variant_tuple_field(tag.clone(), i)
                        }
                        nir::program::VariantShape::Named(names) => {
                            let Some(name) = names.get(i) else {
                                return Ok(false);
                            };
                            place.variant_record_field(tag.clone(), name.as_str())
                        }
                    };
                    if !self.harvest_pattern(
                        sub,
                        &sub_place,
                        field_ty,
                        list_views,
                        harvested_lists,
                        remaps,
                        binding_tys,
                        exported_locals,
                        is_direct_export,
                        depth + 1,
                    )? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Pattern::Or(alts) => {
                for (i, alt) in alts.iter().enumerate() {
                    if !self.harvest_pattern(
                        alt,
                        place,
                        cur_ty,
                        list_views,
                        harvested_lists,
                        remaps,
                        binding_tys,
                        exported_locals,
                        is_direct_export && i == 0,
                        depth + 1,
                    )? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Pattern::Observe { binding, inner } => {
                if has_list_view_for_place(place, list_views) {
                    let mut steps = Vec::new();
                    flatten_then_unit_spine(inner, &mut steps);
                    let Some(&(len_cond_expr, len_next)) = steps.first() else {
                        return Ok(false);
                    };
                    let (has_rest, prefix_len, len_expr) = match &len_cond_expr.kind {
                        K::ScalarLessEqual(arity_expr, len_call)
                        | K::IntegerBinary {
                            op: Op::LessEqual,
                            left: arity_expr,
                            right: len_call,
                        } => {
                            let Some(arity) = extract_literal_usize(arity_expr) else {
                                return Ok(false);
                            };
                            (true, arity, len_call.as_ref())
                        }
                        _ if matches!(len_next, Pattern::Literal(_)) => {
                            (false, steps.len() - 1, len_cond_expr)
                        }
                        _ => return Ok(false),
                    };
                    if steps.len() < 1 + prefix_len || steps.len() > 2 + prefix_len {
                        return Ok(false);
                    }
                    let entry = harvested_lists.entry(place.clone()).or_default();
                    entry.len.get_or_insert(ListCallRef {
                        obs_binding: *binding,
                        expr: len_expr,
                    });
                    for i in 0..prefix_len {
                        let (elem_expr, elem_pat) = steps[1 + i];
                        harvested_lists
                            .entry(place.clone())
                            .or_default()
                            .elems
                            .entry(i)
                            .or_insert(ListCallRef {
                                obs_binding: *binding,
                                expr: elem_expr,
                            });
                        if !self.harvest_pattern(
                            elem_pat,
                            &place.list_elem(i),
                            &elem_expr.ty,
                            list_views,
                            harvested_lists,
                            remaps,
                            binding_tys,
                            exported_locals,
                            is_direct_export,
                            depth + 1,
                        )? {
                            return Ok(false);
                        }
                    }
                    if has_rest && steps.len() == 2 + prefix_len {
                        let (rest_expr, rest_pat) = steps[1 + prefix_len];
                        harvested_lists
                            .entry(place.clone())
                            .or_default()
                            .rests
                            .entry(prefix_len)
                            .or_insert(ListCallRef {
                                obs_binding: *binding,
                                expr: rest_expr,
                            });
                        if !self.harvest_pattern(
                            rest_pat,
                            &place.list_rest(prefix_len),
                            &rest_expr.ty,
                            list_views,
                            harvested_lists,
                            remaps,
                            binding_tys,
                            exported_locals,
                            is_direct_export,
                            depth + 1,
                        )? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                } else if matches!(
                    inner.as_ref(),
                    Pattern::Guard { pattern, .. } if matches!(pattern.as_ref(), Pattern::Wild)
                ) {
                    // Range pattern (`Pattern::Observe { inner: Guard(Wild, ..) }`) lowers natively via IntRange/CharRange switches.
                    Ok(true)
                } else {
                    self.harvest_pattern(
                        inner,
                        place,
                        cur_ty,
                        list_views,
                        harvested_lists,
                        remaps,
                        binding_tys,
                        exported_locals,
                        is_direct_export,
                        depth + 1,
                    )
                }
            }
            Pattern::Then { first, value, next } => {
                if !self.harvest_pattern(
                    first,
                    place,
                    cur_ty,
                    list_views,
                    harvested_lists,
                    remaps,
                    binding_tys,
                    exported_locals,
                    is_direct_export,
                    depth + 1,
                )? {
                    return Ok(false);
                }
                self.harvest_pattern(
                    next,
                    place,
                    &value.ty,
                    list_views,
                    harvested_lists,
                    remaps,
                    binding_tys,
                    exported_locals,
                    is_direct_export,
                    depth + 1,
                )
            }
            Pattern::Guard { .. }
            | Pattern::NominalType { .. }
            | Pattern::TypeTuple(_)
            | Pattern::NormalForm { .. }
            | Pattern::CodeEvidence { .. } => Ok(false),
        }
    }

    fn emit_match_dag<'a>(
        &mut self,
        v: DagPlanValidation<'a>,
        value: &Value,
        value_ty: &Term,
        arms: &'a [MatchArm],
        match_ty: &Term,
        depth: usize,
        selected: Option<&effects::Environment>,
    ) -> Result<Value, String> {
        let never = *match_ty == Term::Primitive(P::Never);
        let res_ty = if never {
            Ty::Unit
        } else {
            return_repr(self.graph, match_ty, self.checked)?
        };
        let result = self.slot(res_ty);
        let end = self.fresh();
        let outer = self.locals.clone();
        let lexical_loops = self.loops.clone();

        let scrutinee_slot = self.slot(value.ty.clone());
        self.store(&scrutinee_slot, value)?;

        // Pre-allocate labels for every DAG node and every reachable arm body.
        let node_bbs: Vec<String> = (0..v.plan.dag.nodes.len()).map(|_| self.fresh()).collect();
        let mut arm_body_bbs: BTreeMap<usize, String> = BTreeMap::new();
        for &arm_idx in &v.reachable_arms {
            arm_body_bbs.insert(arm_idx, self.fresh());
        }

        // Pre-allocate arm binding & receipt slots in the function prologue so all DAG paths dominate.
        let mut arm_local_slots: BTreeMap<(usize, LocalId), Slot> = BTreeMap::new();
        let mut arm_receipt_slots: BTreeMap<(usize, LocalId), (Slot, Slot)> = BTreeMap::new();
        for (&arm_idx, id_map) in &v.arm_exported_tys {
            for (local_id, term) in id_map {
                let llvm_ty = repr(self.graph, term, self.checked)?;
                arm_local_slots.insert((arm_idx, *local_id), self.slot(llvm_ty));
                let path_slot = self.slot(Ty::Int(32, false));
                let rank_slot = self.slot(Ty::Int(32, false));
                self.prologue
                    .push_str(&format!("  store i32 -1, ptr {}\n", path_slot.pointer));
                self.prologue
                    .push_str(&format!("  store i32 -1, ptr {}\n", rank_slot.pointer));
                arm_receipt_slots.insert((arm_idx, *local_id), (path_slot, rank_slot));
            }
        }

        // Pre-allocate lazy list-observation cache slots in prologue and reset init flags here at match entry.
        let mut list_caches: BTreeMap<(Place, ListCacheOp), ListCacheSlot> = BTreeMap::new();
        for (place, op) in &v.needed_list_ops {
            let place_ops = &v.harvested_lists[place];
            let call_ref = match op {
                ListCacheOp::Len => place_ops.len.unwrap(),
                ListCacheOp::Elem(idx) => place_ops.elems[idx],
                ListCacheOp::Rest(from) => place_ops.rests[from],
            };
            let parent_ty = if place.steps.is_empty() {
                value.ty.clone()
            } else {
                v.place_infos[place].ty.clone()
            };
            let parent_slot = if place.steps.is_empty() {
                None
            } else {
                Some(self.slot(parent_ty))
            };
            let val_llvm_ty = repr(self.graph, &call_ref.expr.ty, self.checked)?;
            let val_slot = self.slot(val_llvm_ty);
            let init_flag = self.slot(Ty::Bool);
            self.prologue
                .push_str(&format!("  store i1 false, ptr {}\n", init_flag.pointer));
            self.store(
                &init_flag,
                &Value {
                    ty: Ty::Bool,
                    text: "false".into(),
                },
            )?;
            list_caches.insert(
                (place.clone(), *op),
                ListCacheSlot {
                    obs_binding: call_ref.obs_binding,
                    expr_idx: CacheExprSource::new(call_ref.expr),
                    parent_slot,
                    val_slot,
                    init_flag,
                },
            );
        }

        // Enter the decision DAG root.
        self.branch(&node_bbs[v.plan.dag.root.0]);

        for &node_id in &v.reachable_nodes {
            self.label(&node_bbs[node_id.0]);
            self.locals = outer.clone();
            self.loops = lexical_loops.clone();
            let err_env = &v.active_error_branches[&node_id];
            match &v.plan.dag.nodes[node_id.0] {
                DagNode::Fail => {
                    self.emit_trap();
                    self.line("unreachable");
                    self.terminated = true;
                }
                DagNode::Bind {
                    arm_index,
                    bindings,
                    next,
                } => {
                    self.materialize_dag_bindings(
                        *arm_index,
                        bindings,
                        &v,
                        value_ty,
                        &scrutinee_slot,
                        err_env,
                        &list_caches,
                        &arm_local_slots,
                        &arm_receipt_slots,
                        depth,
                    )?;
                    if !self.terminated {
                        self.branch(&node_bbs[next.0]);
                    }
                }
                DagNode::GuardCheck {
                    guard_id,
                    bindings,
                    on_pass,
                    on_fail,
                } => {
                    self.materialize_dag_bindings(
                        *guard_id,
                        bindings,
                        &v,
                        value_ty,
                        &scrutinee_slot,
                        err_env,
                        &list_caches,
                        &arm_local_slots,
                        &arm_receipt_slots,
                        depth,
                    )?;
                    if !self.terminated {
                        let mut live_ids = Vec::new();
                        if let Some(id_map) = v.arm_exported_tys.get(guard_id) {
                            for id in id_map.keys() {
                                if let Some(slot) = arm_local_slots.get(&(*guard_id, *id)) {
                                    self.locals.insert(*id, slot.clone());
                                    if self.binding_needs_live_flag(*id) {
                                        self.set_live(*id, &[], true)?;
                                        live_ids.push(*id);
                                    }
                                }
                            }
                        }
                        let guard_expr = v.arm_metas[*guard_id].outer_guard.unwrap();
                        let cond = self.expr(guard_expr, depth)?;
                        if !self.terminated {
                            if live_ids.is_empty() {
                                self.locals = outer.clone();
                                self.line(format!(
                                    "br i1 {}, label %{}, label %{}",
                                    cond.text, node_bbs[on_pass.0], node_bbs[on_fail.0]
                                ));
                                self.terminated = true;
                            } else {
                                let fail_reset_bb = self.fresh();
                                self.line(format!(
                                    "br i1 {}, label %{}, label %{fail_reset_bb}",
                                    cond.text, node_bbs[on_pass.0]
                                ));
                                self.label(&fail_reset_bb);
                                for id in live_ids {
                                    self.set_live(id, &[], false)?;
                                }
                                self.locals = outer.clone();
                                self.branch(&node_bbs[on_fail.0]);
                            }
                        } else {
                            self.locals = outer.clone();
                        }
                    }
                }
                DagNode::Leaf {
                    arm_index,
                    bindings,
                } => {
                    self.materialize_dag_bindings(
                        *arm_index,
                        bindings,
                        &v,
                        value_ty,
                        &scrutinee_slot,
                        err_env,
                        &list_caches,
                        &arm_local_slots,
                        &arm_receipt_slots,
                        depth,
                    )?;
                    if !self.terminated {
                        self.branch(&arm_body_bbs[arm_index]);
                    }
                }
                DagNode::Switch { place, kind } => {
                    self.emit_dag_switch(
                        place,
                        kind,
                        &v,
                        value_ty,
                        &scrutinee_slot,
                        err_env,
                        &list_caches,
                        &node_bbs,
                        depth,
                    )?;
                }
            }
        }

        // Emit each reachable arm body once (shared target across all DAG paths selecting that arm).
        let mut continues = false;
        for &arm_idx in &v.reachable_arms {
            self.label(&arm_body_bbs[&arm_idx]);
            self.locals = outer.clone();
            self.loops = lexical_loops.clone();
            let mut receipt = patterns::Receipt::new();
            if let Some(id_map) = v.arm_exported_tys.get(&arm_idx) {
                for id in id_map.keys() {
                    if let Some(slot) = arm_local_slots.get(&(arm_idx, *id)) {
                        self.locals.insert(*id, slot.clone());
                        self.set_live(*id, &[], true)?;
                    }
                    if let Some((path_slot, rank_slot)) = arm_receipt_slots.get(&(arm_idx, *id)) {
                        receipt.insert(*id, (self.load(path_slot), self.load(rank_slot)));
                    }
                }
            }
            let arm = &arms[arm_idx];
            self.retain_pattern(arm.body.source, value, &receipt)?;
            let selected_val = self.expr_with_effects(&arm.body, depth, selected)?;
            if !self.terminated {
                if !matches!(arm.body.kind, K::Block { .. } | K::CallPreparation { .. }) {
                    self.cleanup_boundary(arm.body.source, iir::cleanup::Boundary::ScopeEnd)?;
                }
                if never {
                    self.line("unreachable");
                    self.terminated = true;
                } else {
                    self.store(&result, &selected_val)?;
                    self.branch(&end);
                    continues = true;
                }
            }
        }

        self.locals = outer;
        self.loops = lexical_loops;
        if continues {
            self.label(&end);
            Ok(self.load(&result))
        } else {
            Ok(Value::unit())
        }
    }

    fn binding_needs_live_flag(&self, binding: LocalId) -> bool {
        self.cleanup.is_some_and(|schedule| {
            schedule.sites.iter().any(|site| {
                site.actions.iter().any(|action| {
                    matches!(&action.target, iir::cleanup::Target::Local { binding: id, .. } if *id == binding)
                        && cleanup::has_condition(&action.program.steps)
                })
            })
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn materialize_dag_bindings<'a>(
        &mut self,
        arm_index: usize,
        bindings: &[pa::Binding],
        v: &DagPlanValidation<'a>,
        value_ty: &Term,
        scrutinee_slot: &Slot,
        err_env: &BTreeMap<Place, Option<usize>>,
        list_caches: &BTreeMap<(Place, ListCacheOp), ListCacheSlot>,
        arm_local_slots: &BTreeMap<(usize, LocalId), Slot>,
        arm_receipt_slots: &BTreeMap<(usize, LocalId), (Slot, Slot)>,
        depth: usize,
    ) -> Result<(), String> {
        let remaps = &v.arm_metas[arm_index].remaps;
        for (rank_idx, binding) in bindings.iter().enumerate() {
            if self.terminated {
                return Ok(());
            }
            let raw_id = parse_local_id(&binding.name).unwrap();
            let target_id = resolve_remap_id(raw_id, remaps);
            let mut val = self.eval_place(
                &binding.place,
                value_ty,
                scrutinee_slot,
                v,
                err_env,
                list_caches,
                depth,
            )?;
            if self.terminated {
                return Ok(());
            }
            let dest_slot = arm_local_slots[&(arm_index, target_id)].clone();
            if val.ty != dest_slot.ty {
                val = self.convert_bitslice_value(val, dest_slot.ty.clone())?;
            }
            self.store(&dest_slot, &val)?;
            let receipt_path = place_to_receipt_path(&binding.place, value_ty, err_env, self.graph)
                .ok_or("host DAG receipt projection failed")?;
            let path_code = match receipt_path {
                Some(path) => self.receipt_path(&path)?.to_string(),
                None => "-1".into(),
            };
            let (path_slot, rank_slot) = &arm_receipt_slots[&(arm_index, target_id)];
            self.store(
                path_slot,
                &Value {
                    ty: Ty::Int(32, false),
                    text: path_code,
                },
            )?;
            self.store(
                rank_slot,
                &Value {
                    ty: Ty::Int(32, false),
                    text: rank_idx.to_string(),
                },
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_dag_switch<'a>(
        &mut self,
        place: &Place,
        kind: &SwitchKind,
        v: &DagPlanValidation<'a>,
        value_ty: &Term,
        scrutinee_slot: &Slot,
        err_env: &BTreeMap<Place, Option<usize>>,
        list_caches: &BTreeMap<(Place, ListCacheOp), ListCacheSlot>,
        node_bbs: &[String],
        depth: usize,
    ) -> Result<(), String> {
        match kind {
            SwitchKind::ListLen { cases, default } => {
                let len_val = self.eval_cached_list_op(
                    place,
                    ListCacheOp::Len,
                    value_ty,
                    scrutinee_slot,
                    v,
                    err_env,
                    list_caches,
                    depth,
                )?;
                if self.terminated {
                    return Ok(());
                }
                self.emit_list_len_switch(&len_val, cases, *default, node_bbs);
                Ok(())
            }
            _ => {
                let val = self.eval_place(
                    place,
                    value_ty,
                    scrutinee_slot,
                    v,
                    err_env,
                    list_caches,
                    depth,
                )?;
                if self.terminated {
                    return Ok(());
                }
                match kind {
                    SwitchKind::Bool { on_false, on_true } => {
                        self.line(format!(
                            "br i1 {}, label %{}, label %{}",
                            val.text, node_bbs[on_true.0], node_bbs[on_false.0]
                        ));
                        self.terminated = true;
                        Ok(())
                    }
                    SwitchKind::Optional {
                        on_null,
                        on_present,
                    } => {
                        let tag = self.optional_tag(&val)?;
                        self.line(format!(
                            "br i1 {}, label %{}, label %{}",
                            tag.text, node_bbs[on_present.0], node_bbs[on_null.0]
                        ));
                        self.terminated = true;
                        Ok(())
                    }
                    SwitchKind::Error {
                        on_ok,
                        cases,
                        default_err,
                    } => {
                        let Ty::ErrorResult(layout) = val.ty.clone() else {
                            return Err("host DAG error switch requires ErrorResult layout".into());
                        };
                        let tag = self.error_tag(&val)?;
                        let Ty::Int(tag_bits, false) = tag.ty else {
                            return Err("host DAG error tag must be unsigned integer".into());
                        };
                        let default_bb = match default_err {
                            Some(def) => node_bbs[def.0].clone(),
                            None => {
                                let unreach_bb = self.fresh();
                                let mut table = Vec::with_capacity(cases.len() + 1);
                                table.push(format!("i{tag_bits} 0, label %{}", node_bbs[on_ok.0]));
                                for (err_tag, target) in cases {
                                    let idx = self
                                        .resolve_error_tag_index(&layout, err_tag)
                                        .ok_or("host DAG missing error branch index")?;
                                    table.push(format!(
                                        "i{tag_bits} {idx}, label %{}",
                                        node_bbs[target.0]
                                    ));
                                }
                                self.line(format!(
                                    "switch i{tag_bits} {}, label %{unreach_bb} [\n    {}\n  ]",
                                    tag.text,
                                    table.join("\n    ")
                                ));
                                self.label(&unreach_bb);
                                self.line("unreachable");
                                self.terminated = true;
                                return Ok(());
                            }
                        };
                        let mut table = Vec::with_capacity(cases.len() + 1);
                        table.push(format!("i{tag_bits} 0, label %{}", node_bbs[on_ok.0]));
                        for (err_tag, target) in cases {
                            let idx = self
                                .resolve_error_tag_index(&layout, err_tag)
                                .ok_or("host DAG missing error branch index")?;
                            table.push(format!("i{tag_bits} {idx}, label %{}", node_bbs[target.0]));
                        }
                        self.line(format!(
                            "switch i{tag_bits} {}, label %{default_bb} [\n    {}\n  ]",
                            tag.text,
                            table.join("\n    ")
                        ));
                        self.terminated = true;
                        Ok(())
                    }
                    SwitchKind::Enum { cases, default } => {
                        self.emit_enum_switch(&val, cases, *default, node_bbs)
                    }
                    SwitchKind::Int { cases, default } => {
                        self.emit_int_switch(&val, cases, *default, node_bbs)
                    }
                    SwitchKind::Char { cases, default } => {
                        self.emit_char_switch(&val, cases, *default, node_bbs)
                    }
                    SwitchKind::BitSlice {
                        lo,
                        hi,
                        cases,
                        default,
                    } => self.emit_bitslice_switch(&val, *lo, *hi, cases, *default, node_bbs),
                    SwitchKind::ListLen { .. } => unreachable!(),
                }
            }
        }
    }

    fn emit_enum_switch(
        &mut self,
        val: &Value,
        cases: &BTreeMap<VariantTag, NodeId>,
        default: Option<NodeId>,
        node_bbs: &[String],
    ) -> Result<(), String> {
        let Ty::Enum(layout) = val.ty.clone() else {
            return Err("host DAG enum switch requires enum representation".into());
        };
        let variant_targets: Vec<Option<NodeId>> = layout
            .names
            .iter()
            .map(|name| {
                cases
                    .iter()
                    .find(|(tag, _)| variant_tag_name(tag) == Some(name.as_str()))
                    .map(|(_, target)| *target)
                    .or(default)
            })
            .collect();

        let mut distinct_targets = Vec::new();
        for target in variant_targets.iter().flatten() {
            if !distinct_targets.contains(target) {
                distinct_targets.push(*target);
            }
        }
        if distinct_targets.is_empty() {
            self.line("unreachable");
            self.terminated = true;
            return Ok(());
        }
        if distinct_targets.len() == 1 {
            self.branch(&node_bbs[distinct_targets[0].0]);
            return Ok(());
        }

        // Binary split across contiguous variant groups reuses `variant_set_condition` (e.g. range cmp / XOR).
        if distinct_targets.len() == 2
            && !variant_targets.contains(&None)
            && !matches!(layout.repr, enums::EnumRepr::Symbol(_))
        {
            let t_true = distinct_targets[0];
            let t_false = distinct_targets[1];
            let true_indices: Vec<usize> = variant_targets
                .iter()
                .enumerate()
                .filter_map(|(idx, target)| (*target == Some(t_true)).then_some(idx))
                .collect();
            if let Some(cond) = self.variant_set_condition(val, &true_indices)? {
                self.line(format!(
                    "br i1 {}, label %{}, label %{}",
                    cond.text, node_bbs[t_true.0], node_bbs[t_false.0]
                ));
                self.terminated = true;
                return Ok(());
            }
        }

        match &layout.repr {
            enums::EnumRepr::Unit { width } if *width > 0 => {
                let width = *width;
                self.emit_variant_targets_llvm_switch(
                    width,
                    &val.text,
                    (0..layout.names.len()).map(|idx| (idx.to_string(), variant_targets[idx])),
                    node_bbs,
                );
            }
            enums::EnumRepr::TaggedUnion { tag_bits, .. } => {
                let tag_bits = *tag_bits;
                let tag = self.instruction(
                    Ty::Int(tag_bits, false),
                    format!("extractvalue {} {}, 0", val.ty.llvm(), val.text),
                );
                self.emit_variant_targets_llvm_switch(
                    tag_bits,
                    &tag.text,
                    (0..layout.names.len()).map(|idx| (idx.to_string(), variant_targets[idx])),
                    node_bbs,
                );
            }
            enums::EnumRepr::Niche {
                data_variant,
                carrier,
                niche,
                ..
            } => {
                let nir::layout::NicheScalar::Int { bits } = niche.scalar else {
                    // Pointer niche only has 2 states (null vs non-null), already handled by `len == 2`.
                    return self.emit_sequential_enum_groups(
                        val,
                        &variant_targets,
                        &distinct_targets,
                        node_bbs,
                    );
                };
                let leaf = self.extract_niche_leaf(val, carrier, niche)?;
                let mut entries = Vec::new();
                for (idx, target) in variant_targets.iter().enumerate() {
                    if idx == *data_variant {
                        continue;
                    }
                    let ord = if idx < *data_variant { idx } else { idx - 1 };
                    let code = u128::from(niche.start) + ord as u128;
                    if let Some(t) = target {
                        entries.push(format!("i{bits} {code}, label %{}", node_bbs[t.0]));
                    }
                }
                let default_bb = match variant_targets[*data_variant] {
                    Some(t) => node_bbs[t.0].clone(),
                    None => {
                        let unreach = self.fresh();
                        self.line(format!(
                            "switch i{bits} {}, label %{unreach} [\n    {}\n  ]",
                            leaf.text,
                            entries.join("\n    ")
                        ));
                        self.label(&unreach);
                        self.line("unreachable");
                        self.terminated = true;
                        return Ok(());
                    }
                };
                self.line(format!(
                    "switch i{bits} {}, label %{default_bb} [\n    {}\n  ]",
                    leaf.text,
                    entries.join("\n    ")
                ));
                self.terminated = true;
            }
            enums::EnumRepr::FixedBits {
                width,
                codes,
                masks,
                ..
            } if *width <= 128 && fixed_bits_all_exact_masks(*width, masks) => {
                let width = *width;
                self.emit_variant_targets_llvm_switch(
                    width,
                    &val.text,
                    codes
                        .iter()
                        .enumerate()
                        .map(|(idx, code)| (code.clone(), variant_targets[idx])),
                    node_bbs,
                );
            }
            _ => {
                self.emit_sequential_enum_groups(
                    val,
                    &variant_targets,
                    &distinct_targets,
                    node_bbs,
                )?;
            }
        }
        Ok(())
    }

    fn emit_variant_targets_llvm_switch(
        &mut self,
        bits: u32,
        scrut_text: &str,
        entries: impl Iterator<Item = (String, Option<NodeId>)>,
        node_bbs: &[String],
    ) {
        let mut items: Vec<(String, Option<NodeId>)> = entries.collect();
        let mut counts: BTreeMap<NodeId, usize> = BTreeMap::new();
        for (_, target) in &items {
            if let Some(t) = target {
                *counts.entry(*t).or_default() += 1;
            }
        }
        let default_target = if items.iter().any(|(_, t)| t.is_none()) {
            None
        } else {
            counts
                .into_iter()
                .max_by_key(|(_, count)| *count)
                .map(|(t, _)| t)
        };
        let mut cases_text = Vec::new();
        for (code, target) in items.drain(..) {
            if let Some(t) = target {
                if default_target == Some(t) {
                    continue;
                }
                cases_text.push(format!("i{bits} {code}, label %{}", node_bbs[t.0]));
            }
        }
        match default_target {
            Some(def) => {
                self.line(format!(
                    "switch i{bits} {scrut_text}, label %{} [\n    {}\n  ]",
                    node_bbs[def.0],
                    cases_text.join("\n    ")
                ));
                self.terminated = true;
            }
            None => {
                let unreach = self.fresh();
                self.line(format!(
                    "switch i{bits} {scrut_text}, label %{unreach} [\n    {}\n  ]",
                    cases_text.join("\n    ")
                ));
                self.label(&unreach);
                self.line("unreachable");
                self.terminated = true;
            }
        }
    }

    fn emit_sequential_enum_groups(
        &mut self,
        val: &Value,
        variant_targets: &[Option<NodeId>],
        distinct_targets: &[NodeId],
        node_bbs: &[String],
    ) -> Result<(), String> {
        let last_idx = distinct_targets.len() - 1;
        for (pos, &target) in distinct_targets.iter().enumerate() {
            if pos == last_idx && !variant_targets.contains(&None) {
                self.branch(&node_bbs[target.0]);
                return Ok(());
            }
            let group_indices: Vec<usize> = variant_targets
                .iter()
                .enumerate()
                .filter_map(|(idx, t)| (*t == Some(target)).then_some(idx))
                .collect();
            let cond = if let Some(c) = self.variant_set_condition(val, &group_indices)? {
                c
            } else {
                let mut combined: Option<Value> = None;
                for idx in group_indices {
                    let c = self.variant_condition(val, idx)?;
                    combined = Some(match combined {
                        None => c,
                        Some(prev) => {
                            self.instruction(Ty::Bool, format!("or i1 {}, {}", prev.text, c.text))
                        }
                    });
                }
                combined.unwrap_or(Value {
                    ty: Ty::Bool,
                    text: "false".into(),
                })
            };
            let next_bb = self.fresh();
            self.line(format!(
                "br i1 {}, label %{}, label %{next_bb}",
                cond.text, node_bbs[target.0]
            ));
            self.label(&next_bb);
        }
        self.line("unreachable");
        self.terminated = true;
        Ok(())
    }

    fn emit_int_switch(
        &mut self,
        val: &Value,
        cases: &[(IntInterval, NodeId)],
        default: Option<NodeId>,
        node_bbs: &[String],
    ) -> Result<(), String> {
        let Ty::Int(bits, signed) = val.ty else {
            return Err("host DAG int switch requires integer type".into());
        };
        let default_bb = match default {
            Some(def) => node_bbs[def.0].clone(),
            None => {
                let unreach = self.fresh();
                if signed {
                    self.emit_signed_int_cases(val, bits, cases, &unreach, node_bbs)?;
                } else {
                    self.emit_unsigned_int_cases(val, bits, cases, &unreach, node_bbs)?;
                }
                self.label(&unreach);
                self.line("unreachable");
                self.terminated = true;
                return Ok(());
            }
        };
        if signed {
            self.emit_signed_int_cases(val, bits, cases, &default_bb, node_bbs)
        } else {
            self.emit_unsigned_int_cases(val, bits, cases, &default_bb, node_bbs)
        }
    }

    fn emit_signed_int_cases(
        &mut self,
        val: &Value,
        bits: u32,
        cases: &[(IntInterval, NodeId)],
        default_bb: &str,
        node_bbs: &[String],
    ) -> Result<(), String> {
        let clamped: Vec<(i128, i128, NodeId)> = cases
            .iter()
            .filter_map(|&(iv, target)| {
                clamp_signed_interval(iv, bits).map(|(lo, hi)| (lo, hi, target))
            })
            .collect();
        if clamped.is_empty() {
            self.branch(default_bb);
            return Ok(());
        }
        if clamped.iter().all(|(lo, hi, _)| lo == hi) {
            let entries: Vec<String> = clamped
                .iter()
                .map(|(lo, _, target)| format!("i{bits} {lo}, label %{}", node_bbs[target.0]))
                .collect();
            self.line(format!(
                "switch i{bits} {}, label %{default_bb} [\n    {}\n  ]",
                val.text,
                entries.join("\n    ")
            ));
            self.terminated = true;
            return Ok(());
        }
        self.emit_signed_range_tree(val, bits, &clamped, default_bb, node_bbs);
        Ok(())
    }

    fn emit_signed_range_tree(
        &mut self,
        val: &Value,
        bits: u32,
        slice: &[(i128, i128, NodeId)],
        default_bb: &str,
        node_bbs: &[String],
    ) {
        let domain_min = min_signed_i128(bits);
        let domain_max = max_signed_i128(bits);
        if slice.len() >= 6 {
            let mid = slice.len() / 2;
            let pivot = slice[mid].0;
            let left_bb = self.fresh();
            let right_bb = self.fresh();
            let cmp = self.instruction(Ty::Bool, format!("icmp slt i{bits} {}, {pivot}", val.text));
            self.line(format!(
                "br i1 {}, label %{left_bb}, label %{right_bb}",
                cmp.text
            ));
            self.label(&left_bb);
            self.emit_signed_range_tree(val, bits, &slice[..mid], default_bb, node_bbs);
            self.label(&right_bb);
            self.emit_signed_range_tree(val, bits, &slice[mid..], default_bb, node_bbs);
            return;
        }
        for &(lo, hi, target) in slice {
            if lo == domain_min && hi == domain_max {
                self.branch(&node_bbs[target.0]);
                return;
            }
            let cond = if lo == hi {
                self.instruction(Ty::Bool, format!("icmp eq i{bits} {}, {lo}", val.text))
            } else if lo == domain_min {
                self.instruction(Ty::Bool, format!("icmp sle i{bits} {}, {hi}", val.text))
            } else if hi == domain_max {
                self.instruction(Ty::Bool, format!("icmp sge i{bits} {}, {lo}", val.text))
            } else {
                let shifted = self.instruction(
                    Ty::Int(bits, true),
                    format!("sub i{bits} {}, {lo}", val.text),
                );
                let span = (hi as u128).wrapping_sub(lo as u128);
                self.instruction(
                    Ty::Bool,
                    format!("icmp ule i{bits} {}, {span}", shifted.text),
                )
            };
            let next_bb = self.fresh();
            self.line(format!(
                "br i1 {}, label %{}, label %{next_bb}",
                cond.text, node_bbs[target.0]
            ));
            self.label(&next_bb);
        }
        self.branch(default_bb);
    }

    fn emit_unsigned_int_cases(
        &mut self,
        val: &Value,
        bits: u32,
        cases: &[(IntInterval, NodeId)],
        default_bb: &str,
        node_bbs: &[String],
    ) -> Result<(), String> {
        let clamped: Vec<(u128, u128, NodeId)> = cases
            .iter()
            .filter_map(|&(iv, target)| {
                clamp_unsigned_interval(iv, bits).map(|(lo, hi)| (lo, hi, target))
            })
            .collect();
        if clamped.is_empty() {
            self.branch(default_bb);
            return Ok(());
        }
        if clamped.iter().all(|(lo, hi, _)| lo == hi) {
            let entries: Vec<String> = clamped
                .iter()
                .map(|(lo, _, target)| format!("i{bits} {lo}, label %{}", node_bbs[target.0]))
                .collect();
            self.line(format!(
                "switch i{bits} {}, label %{default_bb} [\n    {}\n  ]",
                val.text,
                entries.join("\n    ")
            ));
            self.terminated = true;
            return Ok(());
        }
        self.emit_unsigned_range_tree(val, bits, &clamped, default_bb, node_bbs);
        Ok(())
    }

    fn emit_unsigned_range_tree(
        &mut self,
        val: &Value,
        bits: u32,
        slice: &[(u128, u128, NodeId)],
        default_bb: &str,
        node_bbs: &[String],
    ) {
        let domain_max = max_unsigned_u128(bits);
        if slice.len() >= 6 {
            let mid = slice.len() / 2;
            let pivot = slice[mid].0;
            let left_bb = self.fresh();
            let right_bb = self.fresh();
            let cmp = self.instruction(Ty::Bool, format!("icmp ult i{bits} {}, {pivot}", val.text));
            self.line(format!(
                "br i1 {}, label %{left_bb}, label %{right_bb}",
                cmp.text
            ));
            self.label(&left_bb);
            self.emit_unsigned_range_tree(val, bits, &slice[..mid], default_bb, node_bbs);
            self.label(&right_bb);
            self.emit_unsigned_range_tree(val, bits, &slice[mid..], default_bb, node_bbs);
            return;
        }
        for &(lo, hi, target) in slice {
            if lo == 0 && hi == domain_max {
                self.branch(&node_bbs[target.0]);
                return;
            }
            let cond = if lo == hi {
                self.instruction(Ty::Bool, format!("icmp eq i{bits} {}, {lo}", val.text))
            } else if lo == 0 {
                self.instruction(Ty::Bool, format!("icmp ule i{bits} {}, {hi}", val.text))
            } else if hi == domain_max {
                self.instruction(Ty::Bool, format!("icmp uge i{bits} {}, {lo}", val.text))
            } else {
                let shifted = self.instruction(
                    Ty::Int(bits, false),
                    format!("sub i{bits} {}, {lo}", val.text),
                );
                let span = hi - lo;
                self.instruction(
                    Ty::Bool,
                    format!("icmp ule i{bits} {}, {span}", shifted.text),
                )
            };
            let next_bb = self.fresh();
            self.line(format!(
                "br i1 {}, label %{}, label %{next_bb}",
                cond.text, node_bbs[target.0]
            ));
            self.label(&next_bb);
        }
        self.branch(default_bb);
    }

    fn emit_char_switch(
        &mut self,
        val: &Value,
        cases: &[(pa::CharInterval, NodeId)],
        default: Option<NodeId>,
        node_bbs: &[String],
    ) -> Result<(), String> {
        let slice: Vec<(u128, u128, NodeId)> = cases
            .iter()
            .filter_map(|&(iv, target)| {
                let lo = u128::from(iv.lo as u32);
                let hi = u128::from(iv.hi as u32);
                (lo <= hi).then_some((lo, hi, target))
            })
            .collect();
        if slice.is_empty() {
            match default {
                Some(def) => self.branch(&node_bbs[def.0]),
                None => {
                    self.line("unreachable");
                    self.terminated = true;
                }
            }
            return Ok(());
        }
        let mut emit_or_unreach = false;
        let default_bb = match default {
            Some(def) => node_bbs[def.0].clone(),
            None => {
                emit_or_unreach = true;
                self.fresh()
            }
        };
        if slice.iter().all(|(lo, hi, _)| lo == hi) {
            let entries: Vec<String> = slice
                .iter()
                .map(|(lo, _, target)| format!("i32 {lo}, label %{}", node_bbs[target.0]))
                .collect();
            self.line(format!(
                "switch i32 {}, label %{default_bb} [\n    {}\n  ]",
                val.text,
                entries.join("\n    ")
            ));
            self.terminated = true;
        } else {
            self.emit_unsigned_range_tree(val, 32, &slice, &default_bb, node_bbs);
        }
        if emit_or_unreach {
            self.label(&default_bb);
            self.line("unreachable");
            self.terminated = true;
        }
        Ok(())
    }

    fn emit_list_len_switch(
        &mut self,
        len_val: &Value,
        cases: &[(LenInterval, NodeId)],
        default: Option<NodeId>,
        node_bbs: &[String],
    ) {
        let Ty::Int(bits, false) = len_val.ty else {
            unreachable!()
        };
        let mut exact_points = Vec::new();
        let mut ranges = Vec::new();
        for &(iv, target) in cases {
            if iv.max_inclusive == Some(iv.min) {
                exact_points.push((iv.min, target));
            } else {
                ranges.push((iv, target));
            }
        }
        let ranges_bb = if ranges.is_empty() {
            None
        } else {
            Some(self.fresh())
        };
        let mut unreach_bb = None;
        let fallthrough_bb = if let Some(ref bb) = ranges_bb {
            bb.clone()
        } else if let Some(def) = default {
            node_bbs[def.0].clone()
        } else {
            let bb = self.fresh();
            unreach_bb = Some(bb.clone());
            bb
        };
        if !exact_points.is_empty() {
            let entries: Vec<String> = exact_points
                .iter()
                .map(|(len, target)| format!("i{bits} {len}, label %{}", node_bbs[target.0]))
                .collect();
            self.line(format!(
                "switch i{bits} {}, label %{fallthrough_bb} [\n    {}\n  ]",
                len_val.text,
                entries.join("\n    ")
            ));
            self.terminated = true;
        } else {
            self.branch(&fallthrough_bb);
        }
        if let Some(ref rbb) = ranges_bb {
            self.label(rbb);
            for &(iv, target) in &ranges {
                if iv.min == 0 && iv.max_inclusive.is_none() {
                    self.branch(&node_bbs[target.0]);
                    return;
                }
                let cond = match (iv.min, iv.max_inclusive) {
                    (0, Some(max)) => self.instruction(
                        Ty::Bool,
                        format!("icmp ule i{bits} {}, {max}", len_val.text),
                    ),
                    (min, None) => self.instruction(
                        Ty::Bool,
                        format!("icmp uge i{bits} {}, {min}", len_val.text),
                    ),
                    (min, Some(max)) => {
                        let lo_c = self.instruction(
                            Ty::Bool,
                            format!("icmp uge i{bits} {}, {min}", len_val.text),
                        );
                        let hi_c = self.instruction(
                            Ty::Bool,
                            format!("icmp ule i{bits} {}, {max}", len_val.text),
                        );
                        self.instruction(Ty::Bool, format!("and i1 {}, {}", lo_c.text, hi_c.text))
                    }
                };
                let next_bb = self.fresh();
                self.line(format!(
                    "br i1 {}, label %{}, label %{next_bb}",
                    cond.text, node_bbs[target.0]
                ));
                self.label(&next_bb);
            }
            match default {
                Some(def) => self.branch(&node_bbs[def.0]),
                None => {
                    self.line("unreachable");
                    self.terminated = true;
                }
            }
        } else if let Some(ubb) = unreach_bb {
            self.label(&ubb);
            self.line("unreachable");
            self.terminated = true;
        }
    }

    fn emit_bitslice_switch(
        &mut self,
        parent_val: &Value,
        lo: u32,
        hi: u32,
        cases: &[(BitInterval, NodeId)],
        default: Option<NodeId>,
        node_bbs: &[String],
    ) -> Result<(), String> {
        let width = hi - lo;
        if width == 0 {
            if let Some((_, target)) = cases.first() {
                self.branch(&node_bbs[target.0]);
            } else if let Some(def) = default {
                self.branch(&node_bbs[def.0]);
            } else {
                self.line("unreachable");
                self.terminated = true;
            }
            return Ok(());
        }
        let raw_bits = if parent_val.ty == Ty::Int(bit_width(&parent_val.ty).unwrap(), false) {
            parent_val.clone()
        } else {
            self.to_bits(parent_val.clone())?
        };
        let slice = self.bit_slice(raw_bits, lo as usize, hi as usize)?;
        if cases.len() == 1 && cases[0].0.is_full() {
            self.branch(&node_bbs[cases[0].1.0]);
            return Ok(());
        }
        let mut emit_unreach = false;
        let default_bb = match default {
            Some(def) => node_bbs[def.0].clone(),
            None => {
                emit_unreach = true;
                self.fresh()
            }
        };
        if width <= 64 && cases.iter().all(|(iv, _)| iv.lo == iv.hi) {
            let entries: Vec<String> = cases
                .iter()
                .map(|(iv, target)| {
                    format!(
                        "i{width} {}, label %{}",
                        bitvalue_decimal(&iv.lo),
                        node_bbs[target.0]
                    )
                })
                .collect();
            self.line(format!(
                "switch i{width} {}, label %{default_bb} [\n    {}\n  ]",
                slice.text,
                entries.join("\n    ")
            ));
            self.terminated = true;
        } else {
            self.emit_bit_range_tree(&slice, width, cases, &default_bb, node_bbs);
        }
        if emit_unreach {
            self.label(&default_bb);
            self.line("unreachable");
            self.terminated = true;
        }
        Ok(())
    }

    fn emit_bit_range_tree(
        &mut self,
        slice: &Value,
        width: u32,
        cases: &[(BitInterval, NodeId)],
        default_bb: &str,
        node_bbs: &[String],
    ) {
        if cases.len() >= 6 {
            let mid = cases.len() / 2;
            let pivot = bitvalue_decimal(&cases[mid].0.lo);
            let left_bb = self.fresh();
            let right_bb = self.fresh();
            let cmp = self.instruction(
                Ty::Bool,
                format!("icmp ult i{width} {}, {pivot}", slice.text),
            );
            self.line(format!(
                "br i1 {}, label %{left_bb}, label %{right_bb}",
                cmp.text
            ));
            self.label(&left_bb);
            self.emit_bit_range_tree(slice, width, &cases[..mid], default_bb, node_bbs);
            self.label(&right_bb);
            self.emit_bit_range_tree(slice, width, &cases[mid..], default_bb, node_bbs);
            return;
        }
        for (iv, target) in cases {
            if iv.is_full() {
                self.branch(&node_bbs[target.0]);
                return;
            }
            let cond = if iv.lo == iv.hi {
                let dec = bitvalue_decimal(&iv.lo);
                self.instruction(Ty::Bool, format!("icmp eq i{width} {}, {dec}", slice.text))
            } else if iv.lo.is_zero() {
                let hi_dec = bitvalue_decimal(&iv.hi);
                self.instruction(
                    Ty::Bool,
                    format!("icmp ule i{width} {}, {hi_dec}", slice.text),
                )
            } else {
                let lo_dec = bitvalue_decimal(&iv.lo);
                let hi_dec = bitvalue_decimal(&iv.hi);
                let lo_c = self.instruction(
                    Ty::Bool,
                    format!("icmp uge i{width} {}, {lo_dec}", slice.text),
                );
                let hi_c = self.instruction(
                    Ty::Bool,
                    format!("icmp ule i{width} {}, {hi_dec}", slice.text),
                );
                self.instruction(Ty::Bool, format!("and i1 {}, {}", lo_c.text, hi_c.text))
            };
            let next_bb = self.fresh();
            self.line(format!(
                "br i1 {}, label %{}, label %{next_bb}",
                cond.text, node_bbs[target.0]
            ));
            self.label(&next_bb);
        }
        self.branch(default_bb);
    }

    #[allow(clippy::too_many_arguments)]
    fn eval_place<'a>(
        &mut self,
        place: &Place,
        value_ty: &Term,
        scrutinee_slot: &Slot,
        v: &DagPlanValidation<'a>,
        err_env: &BTreeMap<Place, Option<usize>>,
        list_caches: &BTreeMap<(Place, ListCacheOp), ListCacheSlot>,
        depth: usize,
    ) -> Result<Value, String> {
        let mut cur = self.load(scrutinee_slot);
        let mut cur_term = value_ty.clone();
        let mut cur_place = Place::root();
        if let Some(info) = v.place_infos.get(&Place::root()) {
            cur_term = info.term.clone();
        }
        for step in &place.steps {
            if self.terminated {
                return Ok(Value::unit());
            }
            cur = match step {
                PlaceStep::TupleField(idx) => {
                    let next_term = tuple_elements_term(self.graph, &cur_term)
                        .and_then(|fs| fs.get(*idx).cloned())
                        .ok_or("host DAG tuple projection out of range")?;
                    cur_term = next_term;
                    self.record_field(&cur, *idx)?
                }
                PlaceStep::RecordField(name) => {
                    let (idx, next_term) = resolve_record_field(self.graph, &cur_term, name)
                        .ok_or("host DAG record projection missing")?;
                    cur_term = next_term;
                    self.record_field(&cur, idx)?
                }
                PlaceStep::OptionalInner => {
                    let unwrapped = unwrap_newtype_term(self.graph, &cur_term);
                    let Term::Optional(inner_term) = unwrapped else {
                        return Err("host DAG optional projection requires optional type".into());
                    };
                    cur_term = (**inner_term).clone();
                    self.optional_payload(&cur)?
                }
                PlaceStep::ErrorOkInner => {
                    let unwrapped = unwrap_newtype_term(self.graph, &cur_term);
                    let Term::ErrorResult { result, .. } = unwrapped else {
                        return Err("host DAG ok projection requires error result type".into());
                    };
                    cur_term = (**result).clone();
                    self.error_payload(&cur, 0)?
                }
                PlaceStep::ErrorErrInner => {
                    let Some(Some(err_idx)) = err_env.get(&cur_place) else {
                        return Err("host DAG error payload index is ambiguous".into());
                    };
                    let Ty::ErrorResult(layout) = &cur.ty else {
                        return Err("host DAG error payload requires ErrorResult layout".into());
                    };
                    let next_term = layout
                        .branches
                        .get(*err_idx)
                        .and_then(|(t, _)| t.clone())
                        .ok_or("host DAG error branch type missing")?;
                    cur_term = next_term;
                    self.error_payload(&cur, *err_idx)?
                }
                PlaceStep::VariantTupleField { tag, index } => {
                    let tag_name = variant_tag_name(tag)
                        .ok_or("host DAG variant tuple projection requires named tag")?;
                    let (v_idx, next_term) =
                        resolve_variant_tuple_field(self.graph, &cur_term, tag_name, *index)
                            .ok_or("host DAG variant tuple projection missing")?;
                    cur_term = next_term;
                    self.variant_field(&cur, v_idx, *index)?
                }
                PlaceStep::VariantRecordField { tag, field } => {
                    let tag_name = variant_tag_name(tag)
                        .ok_or("host DAG variant record projection requires named tag")?;
                    let (v_idx, f_idx, next_term) =
                        resolve_variant_record_field(self.graph, &cur_term, tag_name, field)
                            .ok_or("host DAG variant record projection missing")?;
                    cur_term = next_term;
                    self.variant_field(&cur, v_idx, f_idx)?
                }
                PlaceStep::ListElem(idx) => {
                    let op = ListCacheOp::Elem(*idx);
                    let val = self.eval_cached_list_op_from_parent(
                        &cur_place,
                        &cur,
                        op,
                        scrutinee_slot,
                        list_caches,
                        depth,
                    )?;
                    cur_term = v.harvested_lists[&cur_place].elems[idx].expr.ty.clone();
                    val
                }
                PlaceStep::ListRest(from) => {
                    let op = ListCacheOp::Rest(*from);
                    let val = self.eval_cached_list_op_from_parent(
                        &cur_place,
                        &cur,
                        op,
                        scrutinee_slot,
                        list_caches,
                        depth,
                    )?;
                    cur_term = v.harvested_lists[&cur_place].rests[from].expr.ty.clone();
                    val
                }
                PlaceStep::BitSlice { lo, hi, .. } => {
                    let raw_bits = if cur.ty == Ty::Int(bit_width(&cur.ty).unwrap(), false) {
                        cur
                    } else {
                        self.to_bits(cur)?
                    };
                    cur_term = Term::Primitive(P::BitVec((hi - lo) as usize));
                    self.bit_slice(raw_bits, *lo as usize, *hi as usize)?
                }
            };
            cur_place.steps.push(step.clone());
            if let Some(info) = v.place_infos.get(&cur_place) {
                cur_term = info.term.clone();
            }
        }
        Ok(cur)
    }

    #[allow(clippy::too_many_arguments)]
    fn eval_cached_list_op<'a>(
        &mut self,
        place: &Place,
        op: ListCacheOp,
        value_ty: &Term,
        scrutinee_slot: &Slot,
        v: &DagPlanValidation<'a>,
        err_env: &BTreeMap<Place, Option<usize>>,
        list_caches: &BTreeMap<(Place, ListCacheOp), ListCacheSlot>,
        depth: usize,
    ) -> Result<Value, String> {
        let parent_val = if place.steps.is_empty() {
            self.load(scrutinee_slot)
        } else {
            self.eval_place(
                place,
                value_ty,
                scrutinee_slot,
                v,
                err_env,
                list_caches,
                depth,
            )?
        };
        if self.terminated {
            return Ok(Value::unit());
        }
        self.eval_cached_list_op_from_parent(
            place,
            &parent_val,
            op,
            scrutinee_slot,
            list_caches,
            depth,
        )
    }

    fn eval_cached_list_op_from_parent(
        &mut self,
        place: &Place,
        parent_val: &Value,
        op: ListCacheOp,
        scrutinee_slot: &Slot,
        list_caches: &BTreeMap<(Place, ListCacheOp), ListCacheSlot>,
        depth: usize,
    ) -> Result<Value, String> {
        let cache = &list_caches[&(place.clone(), op)];
        let is_init = self.load(&cache.init_flag);
        let compute_bb = self.fresh();
        let done_bb = self.fresh();
        self.line(format!(
            "br i1 {}, label %{done_bb}, label %{compute_bb}",
            is_init.text
        ));
        self.label(&compute_bb);
        let obs_slot = match &cache.parent_slot {
            None => scrutinee_slot.clone(),
            Some(slot) => {
                self.store(slot, parent_val)?;
                slot.clone()
            }
        };
        let previous = self.locals.insert(cache.obs_binding, obs_slot);
        // SAFETY: `cache.expr_idx` points into `&arms` held immutable for the whole match.
        let expr = unsafe { cache.expr_idx.get() };
        let computed = self.expr(expr, depth)?;
        self.locals.remove(&cache.obs_binding);
        if let Some(prev) = previous {
            self.locals.insert(cache.obs_binding, prev);
        }
        if !self.terminated {
            self.store(&cache.val_slot, &computed)?;
            self.store(
                &cache.init_flag,
                &Value {
                    ty: Ty::Bool,
                    text: "true".into(),
                },
            )?;
            self.branch(&done_bb);
        }
        self.label(&done_bb);
        Ok(self.load(&cache.val_slot))
    }

    fn convert_bitslice_value(&mut self, value: Value, target: Ty) -> Result<Value, String> {
        if value.ty == target {
            return Ok(value);
        }
        let Ty::Int(w, _) = value.ty else {
            return self.bit_convert(value, target);
        };
        if target == Ty::Bool && w == 1 {
            return Ok(Value {
                ty: Ty::Bool,
                text: value.text,
            });
        }
        if matches!(target, Ty::Int(w2, _) if w2 == w) {
            return Ok(Value {
                ty: target,
                text: value.text,
            });
        }
        if matches!(
            target,
            Ty::Float16 | Ty::BFloat16 | Ty::Float32 | Ty::Float64
        ) {
            return Ok(self.instruction(
                target.clone(),
                format!("bitcast i{w} {} to {}", value.text, target.llvm()),
            ));
        }
        self.bit_convert(value, target)
    }
}

fn pattern_has_active_then_cleanup(
    pat: &Pattern,
    schedule: &iir::cleanup::FunctionSchedule,
) -> bool {
    let mut stack = vec![pat];
    while let Some(node) = stack.pop() {
        match node {
            Pattern::Then { first, value, next } => {
                let source = value.source;
                let has_failure = schedule.sites.iter().any(|site| {
                    site.source == source
                        && site.boundary == iir::cleanup::Boundary::PatternFailure
                        && site
                            .actions
                            .iter()
                            .any(|action| cleanup::has_invoke(&action.program.steps))
                });
                let has_temp = schedule
                    .sites
                    .iter()
                    .flat_map(|s| &s.actions)
                    .any(|action| {
                        matches!(
                            action.target,
                            iir::cleanup::Target::Temporary { source: origin, .. }
                            if origin == source
                        ) && cleanup::has_invoke(&action.program.steps)
                    });
                if has_failure || has_temp {
                    return true;
                }
                stack.push(first);
                stack.push(next);
            }
            Pattern::OptionalPresent(inner)
            | Pattern::ErrorResult { inner, .. }
            | Pattern::Binding { inner, .. }
            | Pattern::Observe { inner, .. }
            | Pattern::Guard { pattern: inner, .. }
            | Pattern::Remap { pattern: inner, .. } => stack.push(inner),
            Pattern::Tuple(elems)
            | Pattern::AnonymousRecord { fields: elems, .. }
            | Pattern::Record { fields: elems, .. }
            | Pattern::Variant { fields: elems, .. }
            | Pattern::Or(elems)
            | Pattern::TypeTuple(elems)
            | Pattern::NominalType { fields: elems, .. }
            | Pattern::NormalForm {
                arguments: elems, ..
            } => stack.extend(elems),
            Pattern::Wild
            | Pattern::Unit
            | Pattern::Literal(_)
            | Pattern::OptionalAbsent
            | Pattern::CodeEvidence { .. } => {}
        }
    }
    false
}

fn has_list_view_for_place(place: &Place, list_views: &BTreeMap<Place, ListViewOps>) -> bool {
    if list_views.contains_key(place) {
        return true;
    }
    list_views.keys().any(|candidate| {
        if candidate.steps.len() != place.steps.len() {
            return false;
        }
        candidate
            .steps
            .iter()
            .zip(place.steps.iter())
            .all(|(a, b)| match (a, b) {
                (PlaceStep::ListElem(_), PlaceStep::ListElem(_)) => true,
                _ => a == b,
            })
    })
}

fn flatten_then_unit_spine<'a>(pat: &'a Pattern, out: &mut Vec<(&'a Expr, &'a Pattern)>) {
    match pat {
        Pattern::Then { first, value, next } if matches!(value.kind, K::Unit) => {
            flatten_then_unit_spine(first, out);
            flatten_then_unit_spine(next, out);
        }
        Pattern::Then { first, value, next } if matches!(first.as_ref(), Pattern::Wild) => {
            out.push((value.as_ref(), next.as_ref()));
        }
        _ => {}
    }
}

fn extract_literal_usize(expr: &Expr) -> Option<usize> {
    match &expr.kind {
        K::Literal(Literal::UnsignedInteger(u)) => usize::try_from(*u).ok(),
        K::Literal(Literal::SignedInteger(s)) if *s >= 0 => usize::try_from(*s).ok(),
        K::Literal(Literal::Integer(s)) => s.parse::<usize>().ok(),
        _ => None,
    }
}

fn parse_local_id(name: &str) -> Option<LocalId> {
    let rest = name.strip_prefix('_')?;
    let (owner_str, idx_str) = rest.split_once('_')?;
    Some(LocalId {
        owner: owner_str.parse().ok()?,
        index: idx_str.parse().ok()?,
    })
}

fn resolve_remap_id(mut id: LocalId, remaps: &BTreeMap<LocalId, LocalId>) -> LocalId {
    for _ in 0..=remaps.len() {
        let Some(&next) = remaps.get(&id) else {
            break;
        };
        if next == id {
            break;
        }
        id = next;
    }
    id
}

fn unwrap_newtype_term<'a>(graph: &'a iir::Package, mut ty: &'a Term) -> &'a Term {
    for _ in 0..32 {
        let Term::Nominal { .. } = ty else {
            break;
        };
        let Some(inst) = graph.nominals.iter().find(|i| i.ty == *ty) else {
            break;
        };
        let iir::NominalDefinition::Newtype(def) = &inst.definition else {
            break;
        };
        ty = &def.representation;
    }
    ty
}

fn tuple_elements_term<'a>(graph: &'a iir::Package, ty: &'a Term) -> Option<&'a [Term]> {
    let unwrapped = unwrap_newtype_term(graph, ty);
    match unwrapped {
        Term::Tuple(elems) => Some(elems.as_slice()),
        Term::NormalForm { .. } => graph
            .normal_forms
            .iter()
            .find_map(|nf| nf.tuple_elements(unwrapped)),
        _ => None,
    }
}

fn resolve_record_field(graph: &iir::Package, ty: &Term, name: &str) -> Option<(usize, Term)> {
    let unwrapped = unwrap_newtype_term(graph, ty);
    match unwrapped {
        Term::Record(fields) => fields
            .iter()
            .enumerate()
            .find(|(_, (n, _))| n == name)
            .map(|(idx, (_, t))| (idx, t.clone())),
        Term::Nominal { .. } => graph
            .nominals
            .iter()
            .find(|i| i.ty == *unwrapped)
            .and_then(|i| match &i.definition {
                iir::NominalDefinition::Struct(def) => def
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, (n, _))| n == name)
                    .map(|(idx, (_, t))| (idx, t.clone())),
                _ => None,
            }),
        _ => None,
    }
}

fn resolve_variant_tuple_field(
    graph: &iir::Package,
    ty: &Term,
    tag_name: &str,
    index: usize,
) -> Option<(usize, Term)> {
    let unwrapped = unwrap_newtype_term(graph, ty);
    let inst = graph.nominals.iter().find(|i| i.ty == *unwrapped)?;
    let iir::NominalDefinition::Enum(def) = &inst.definition else {
        return None;
    };
    let v_idx = def.variants.iter().position(|v| v.name == tag_name)?;
    let field_ty = def.variants[v_idx].fields.get(index)?.clone();
    Some((v_idx, field_ty))
}

fn resolve_variant_record_field(
    graph: &iir::Package,
    ty: &Term,
    tag_name: &str,
    field_name: &str,
) -> Option<(usize, usize, Term)> {
    let unwrapped = unwrap_newtype_term(graph, ty);
    let inst = graph.nominals.iter().find(|i| i.ty == *unwrapped)?;
    let iir::NominalDefinition::Enum(def) = &inst.definition else {
        return None;
    };
    let v_idx = def.variants.iter().position(|v| v.name == tag_name)?;
    let variant = &def.variants[v_idx];
    let nir::program::VariantShape::Named(names) = &variant.shape else {
        return None;
    };
    let f_idx = names.iter().position(|n| n == field_name)?;
    let field_ty = variant.fields.get(f_idx)?.clone();
    Some((v_idx, f_idx, field_ty))
}

fn place_to_receipt_path(
    place: &Place,
    root_ty: &Term,
    err_env: &BTreeMap<Place, Option<usize>>,
    graph: &iir::Package,
) -> Option<Option<Vec<usize>>> {
    let mut path = Vec::new();
    let mut cur_ty = root_ty.clone();
    let mut cur_place = Place::root();
    for step in &place.steps {
        match step {
            PlaceStep::TupleField(idx) => {
                let next = tuple_elements_term(graph, &cur_ty)?.get(*idx)?.clone();
                path.push(*idx);
                cur_ty = next;
            }
            PlaceStep::RecordField(name) => {
                let (idx, next) = resolve_record_field(graph, &cur_ty, name)?;
                path.push(idx);
                cur_ty = next;
            }
            PlaceStep::OptionalInner => {
                let unwrapped = unwrap_newtype_term(graph, &cur_ty);
                let Term::Optional(inner) = unwrapped else {
                    return None;
                };
                path.push(0);
                cur_ty = (**inner).clone();
            }
            PlaceStep::ErrorOkInner => {
                let unwrapped = unwrap_newtype_term(graph, &cur_ty);
                let Term::ErrorResult { result, .. } = unwrapped else {
                    return None;
                };
                path.push(0);
                cur_ty = (**result).clone();
            }
            PlaceStep::ErrorErrInner => {
                path.push(0);
                let unwrapped = unwrap_newtype_term(graph, &cur_ty);
                let Term::ErrorResult { errors, .. } = unwrapped else {
                    return None;
                };
                if let Some(Some(err_idx)) = err_env.get(&cur_place)
                    && let Some(err_ty) = err_idx.checked_sub(1).and_then(|i| errors.get(i))
                {
                    cur_ty = err_ty.clone();
                } else {
                    // Single non-nested error payload projection needs no inner field lookup;
                    // if a deeper projection follows without an exact branch index, bail out cleanly.
                    if cur_place.steps.len() + 1 < place.steps.len() {
                        return None;
                    }
                    return Some(Some(path));
                }
            }
            PlaceStep::VariantTupleField { tag, index } => {
                let tag_name = variant_tag_name(tag)?;
                let (_, next) = resolve_variant_tuple_field(graph, &cur_ty, tag_name, *index)?;
                path.push(*index);
                cur_ty = next;
            }
            PlaceStep::VariantRecordField { tag, field } => {
                let tag_name = variant_tag_name(tag)?;
                let (_, f_idx, next) =
                    resolve_variant_record_field(graph, &cur_ty, tag_name, field)?;
                path.push(f_idx);
                cur_ty = next;
            }
            PlaceStep::ListElem(_) | PlaceStep::ListRest(_) | PlaceStep::BitSlice { .. } => {
                return Some(None);
            }
        }
        cur_place.steps.push(step.clone());
    }
    Some(Some(path))
}

fn variant_tag_name(tag: &VariantTag) -> Option<&str> {
    match tag {
        VariantTag::Named(name) => Some(name.as_str()),
        VariantTag::Discrete(_) => None,
    }
}

fn can_convert_bitslice_ty(src: &Ty, dst: &Ty) -> bool {
    let Some(w_src) = bit_width(src) else {
        return false;
    };
    match dst {
        Ty::Bool => w_src == 1,
        Ty::Int(w_dst, _) => *w_dst == w_src,
        Ty::Float16 | Ty::BFloat16 => w_src == 16,
        Ty::Float32 => w_src == 32,
        Ty::Float64 => w_src == 64,
        _ => false,
    }
}

fn fixed_bits_all_exact_masks(width: u32, masks: &[String]) -> bool {
    let full = max_unsigned_u128(width).to_string();
    masks.iter().all(|mask| *mask == full)
}

fn bitvalue_decimal(bv: &pa::bitvec::BitValue) -> String {
    let mut u32_words = Vec::with_capacity(bv.words().len() * 2);
    for &w in bv.words() {
        u32_words.push(w as u32);
        u32_words.push((w >> 32) as u32);
    }
    while u32_words.last() == Some(&0) && u32_words.len() > 1 {
        u32_words.pop();
    }
    if u32_words.is_empty() {
        "0".into()
    } else {
        bits::decimal_words(&u32_words)
    }
}

fn min_signed_i128(bits: u32) -> i128 {
    if bits == 128 {
        i128::MIN
    } else {
        -(1i128 << (bits - 1))
    }
}

fn max_signed_i128(bits: u32) -> i128 {
    if bits == 128 {
        i128::MAX
    } else {
        (1i128 << (bits - 1)) - 1
    }
}

fn max_unsigned_u128(bits: u32) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

fn clamp_signed_interval(iv: IntInterval, bits: u32) -> Option<(i128, i128)> {
    let min_f = FiniteInt::min_signed(bits as u8);
    let max_f = FiniteInt::max_signed(bits as u8);
    let lo_f = match iv.lo {
        LowerPoint::NegInf => min_f,
        LowerPoint::Finite(v) => v.max(min_f),
    };
    let hi_f = match iv.hi {
        UpperPoint::PosInf => max_f,
        UpperPoint::Finite(v) => v.min(max_f),
    };
    if lo_f > hi_f {
        return None;
    }
    Some((lo_f.to_i128()?, hi_f.to_i128()?))
}

fn clamp_unsigned_interval(iv: IntInterval, bits: u32) -> Option<(u128, u128)> {
    let min_f = FiniteInt::ZERO;
    let max_f = FiniteInt::max_unsigned(bits as u8);
    let lo_f = match iv.lo {
        LowerPoint::NegInf => min_f,
        LowerPoint::Finite(v) => v.max(min_f),
    };
    let hi_f = match iv.hi {
        UpperPoint::PosInf => max_f,
        UpperPoint::Finite(v) => v.min(max_f),
    };
    if lo_f > hi_f {
        return None;
    }
    Some((lo_f.to_u128()?, hi_f.to_u128()?))
}

fn len_interval_fits(iv: LenInterval, bits: u32) -> bool {
    let max_u128 = max_unsigned_u128(bits);
    let Ok(min_val) = u128::try_from(iv.min) else {
        return false;
    };
    if min_val > max_u128 {
        return false;
    }
    if let Some(max) = iv.max_inclusive {
        let Ok(max_val) = u128::try_from(max) else {
            return false;
        };
        if max_val > max_u128 || min_val > max_val {
            return false;
        }
    }
    true
}
