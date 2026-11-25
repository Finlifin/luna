//! Conservative certification of IIR function and callsite runtime behavior
//! for LLVM 23 optimization attributes.
//!
//! Soundness invariants enforced by this pass:
//! 1. `P4` loops or recursive SCCs never receive `terminates = true` (`willreturn`).
//! 2. Any unelided predicate check, precondition, postcondition, trapping
//!    integer operation, dynamic bit-window/update guard, pattern fallback, or
//!    effect control operation forces `no_trap_or_unwind = false` and forbids
//!    `memory(none)`, `memory(argmem: read)`, `nounwind`, and `speculatable`.
//! 3. `no_free` (`nofree`) and `no_sync` (`nosync`) are tracked independently.
//! 4. Certification is best-effort and bounded: budget exhaustion leaves
//!    uncertified functions as `None` (conservative P0).
use iir::{BehaviorAttributes, CallsiteBehavior, MemoryEffect, PointerParamAttrs};
use nir::program::{Expr, ExprKind as K, IntegerOp, Literal, LocalId};
use nir::{DeclarationId, Primitive as P, Sort, Term};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PtrSource {
    Param(usize),
    LocalAlloc(LocalId),
    ConstGlobal,
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PtrOrigin {
    sources: BTreeSet<PtrSource>,
}

impl PtrOrigin {
    fn param(slot: usize) -> Self {
        let mut sources = BTreeSet::new();
        sources.insert(PtrSource::Param(slot));
        Self { sources }
    }

    fn local_alloc(id: LocalId) -> Self {
        let mut sources = BTreeSet::new();
        sources.insert(PtrSource::LocalAlloc(id));
        Self { sources }
    }

    fn const_global() -> Self {
        let mut sources = BTreeSet::new();
        sources.insert(PtrSource::ConstGlobal);
        Self { sources }
    }

    fn unknown() -> Self {
        let mut sources = BTreeSet::new();
        sources.insert(PtrSource::Unknown);
        Self { sources }
    }

    fn join(&mut self, other: &Self) {
        self.sources.extend(other.sources.iter().copied());
    }

    fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

#[derive(Clone, Copy, Debug)]
enum LocalTarget<'a> {
    Function(usize),
    Closure(&'a Expr),
}

#[derive(Clone, Debug)]
struct BodySummary {
    attrs: BehaviorAttributes,
    callsites: Vec<CallsiteBehavior>,
}

pub(crate) fn integer_op_may_trap(op: IntegerOp) -> bool {
    matches!(
        op,
        IntegerOp::Add
            | IntegerOp::Subtract
            | IntegerOp::Multiply
            | IntegerOp::DivideTrunc
            | IntegerOp::RemainderTrunc
    )
}

struct GraphIndex<'a> {
    graph: &'a iir::Package,
    schedule: Option<&'a iir::cleanup::Schedule>,
    // Lookup-only tables keyed by `Term` (`Term: Eq + Hash`, not `Ord`); never iterated.
    by_key: HashMap<(DeclarationId, &'a [Term]), usize>,
    dispatch: HashMap<
        (
            DeclarationId,
            &'a [Term],
            &'a Term,
            DeclarationId,
            &'a [Term],
        ),
        usize,
    >,
}

impl<'a> GraphIndex<'a> {
    fn new(graph: &'a iir::Package, schedule: Option<&'a iir::cleanup::Schedule>) -> Self {
        let mut by_key = HashMap::with_capacity(graph.functions.len());
        for (idx, instance) in graph.functions.iter().enumerate() {
            by_key.insert(
                (instance.key.declaration, instance.key.arguments.as_ref()),
                idx,
            );
        }
        let mut dispatch = HashMap::new();
        for inst in &graph.dispatch {
            for method in &inst.methods {
                if let Some(&idx) = by_key.get(&(method.body, method.arguments.as_slice())) {
                    dispatch.insert(
                        (
                            inst.interface,
                            inst.interface_arguments.as_slice(),
                            &inst.subject,
                            method.member,
                            method.method_arguments.as_slice(),
                        ),
                        idx,
                    );
                }
            }
        }
        Self {
            graph,
            schedule,
            by_key,
            dispatch,
        }
    }

    fn resolve_direct(&self, expr: &Expr) -> Option<usize> {
        match &expr.kind {
            K::Function(decl) => self.by_key.get(&(*decl, &[][..])).copied(),
            K::FunctionInstance {
                declaration,
                arguments,
            } => self
                .by_key
                .get(&(*declaration, arguments.as_slice()))
                .copied(),
            K::TraitMethod {
                method_arguments,
                interface_arguments,
                interface,
                member,
                subject,
            } => self
                .dispatch
                .get(&(
                    *interface,
                    interface_arguments.as_slice(),
                    subject,
                    *member,
                    method_arguments.as_slice(),
                ))
                .copied(),
            _ => None,
        }
    }
}

fn spend(fuel: &mut usize, amount: usize) -> Option<()> {
    *fuel = fuel.checked_sub(amount)?;
    Some(())
}

/// Certify runtime `BehaviorAttributes` and `CallsiteBehavior`s on `graph`.
/// Leaves any unanalyzed instances untouched (`None` = conservative P0) if `fuel` runs out.
pub fn certify_iir_package(
    graph: &mut iir::Package,
    schedule: Option<&iir::cleanup::Schedule>,
    fuel: &mut usize,
) {
    let n = graph.functions.len();
    if n == 0 || *fuel == 0 {
        return;
    }

    let index = GraphIndex::new(graph, schedule);
    let Some(edges) = collect_call_edges(&index, fuel) else {
        return;
    };
    let sccs = tarjan_sccs(n, &edges);

    let mut certified: Vec<Option<BodySummary>> = vec![None; n];
    for scc in sccs {
        if *fuel == 0 {
            break;
        }
        let cyclic = scc.len() > 1 || scc.first().is_some_and(|&idx| edges[idx].contains(&idx));
        if !cyclic {
            let idx = scc[0];
            if let Some(summary) = analyze_instance(&index, idx, &certified, false, fuel) {
                certified[idx] = Some(summary);
            }
            continue;
        }
        // Recursive SCC: compute the greatest fixed point assuming internal cycle
        // callees start optimistically (except `terminates = false`, per Red Line 1)
        // and shrink monotonically until convergence. Any member that fails analysis or
        // exhausts budget leaves the whole SCC uncertified (`None` = conservative P0).
        let mut iter_fuel = (*fuel).min(65_536);
        let initial_iter_fuel = iter_fuel;
        let max_rounds = scc.len().saturating_add(6);
        let mut seeded = true;
        for &idx in &scc {
            if let Some(seed) = optimistic_scc_seed(&index, idx, &mut iter_fuel) {
                certified[idx] = Some(seed);
            } else {
                seeded = false;
                break;
            }
        }
        if !seeded {
            for &member in &scc {
                certified[member] = None;
            }
            *fuel = fuel.saturating_sub(initial_iter_fuel.saturating_sub(iter_fuel));
            continue;
        }

        let mut changed = true;
        let mut converged = false;
        let mut rounds = 0usize;
        while changed && rounds < max_rounds && iter_fuel > 0 {
            changed = false;
            rounds += 1;
            let mut failed = false;
            for &idx in &scc {
                let Some(mut summary) =
                    analyze_instance(&index, idx, &certified, true, &mut iter_fuel)
                else {
                    failed = true;
                    break;
                };
                summary.attrs.terminates = false;
                if certified[idx]
                    .as_ref()
                    .is_none_or(|prev| prev.attrs != summary.attrs)
                {
                    changed = true;
                }
                certified[idx] = Some(summary);
            }
            if failed {
                for &member in &scc {
                    certified[member] = None;
                }
                converged = true;
                break;
            }
            if !changed {
                converged = true;
            }
        }
        if !converged {
            for &member in &scc {
                certified[member] = None;
            }
        }
        *fuel = fuel.saturating_sub(initial_iter_fuel.saturating_sub(iter_fuel));
        // Final soundness guard: cyclic SCCs can never promise termination.
        for &idx in &scc {
            if let Some(summary) = &mut certified[idx] {
                summary.attrs.terminates = false;
                // Re-apply Red Line 2 after fixed point in case an SCC member has traps.
                if !summary.attrs.no_trap_or_unwind {
                    summary.attrs.memory = MemoryEffect::Unknown;
                }
            }
        }
    }

    for (instance, summary) in graph.functions.iter_mut().zip(certified) {
        if let Some(summary) = summary {
            instance.behavior = Some(summary.attrs);
            instance.callsite_behaviors = summary.callsites;
        }
    }
}

fn collect_call_edges(index: &GraphIndex<'_>, fuel: &mut usize) -> Option<Vec<BTreeSet<usize>>> {
    let n = index.graph.functions.len();
    let mut edges = vec![BTreeSet::new(); n];
    for (idx, instance) in index.graph.functions.iter().enumerate() {
        spend(fuel, 1)?;
        let caller_edges = &mut edges[idx];
        iir::visit::expressions(&instance.function, fuel, |expr, _| {
            if let Some(target) = index.resolve_direct(expr) {
                caller_edges.insert(target);
            }
            Ok(())
        })
        .ok()?;
        if let Some(schedule) = index.schedule {
            collect_schedule_edges(index, &instance.key, schedule, caller_edges, fuel)?;
        }
    }
    Some(edges)
}

fn collect_schedule_edges(
    index: &GraphIndex<'_>,
    key: &iir::InstanceKey,
    schedule: &iir::cleanup::Schedule,
    edges: &mut BTreeSet<usize>,
    fuel: &mut usize,
) -> Option<()> {
    for scope in schedule
        .functions
        .iter()
        .filter(|scope| &scope.instance == key)
    {
        spend(fuel, 1)?;
        for site in &scope.sites {
            for action in &site.actions {
                collect_cleanup_steps(index, &action.program.steps, edges, fuel)?;
            }
        }
        for discard in &scope.discards {
            collect_cleanup_steps(index, &discard.program.steps, edges, fuel)?;
        }
    }
    Some(())
}

fn collect_cleanup_steps(
    index: &GraphIndex<'_>,
    steps: &[iir::cleanup::Step],
    edges: &mut BTreeSet<usize>,
    fuel: &mut usize,
) -> Option<()> {
    use iir::cleanup::Step;
    for step in steps {
        spend(fuel, 1)?;
        match step {
            Step::Release { .. } | Step::DropTraitObject { .. } => {}
            Step::IfConstructed { steps, .. } => {
                collect_cleanup_steps(index, steps, edges, fuel)?;
            }
            Step::Switch { branches, .. } => {
                for branch in branches {
                    collect_cleanup_steps(index, branch, edges, fuel)?;
                }
            }
            Step::Select { cases } => {
                for case in cases {
                    collect_cleanup_steps(index, &case.steps, edges, fuel)?;
                }
            }
            Step::Invoke { method, .. } => {
                if let Some(&target) = index
                    .by_key
                    .get(&(method.declaration, method.arguments.as_ref()))
                {
                    edges.insert(target);
                }
            }
        }
    }
    Some(())
}

/// Tarjan strongly connected components in topological callee-before-caller order.
/// Uses an explicit heap-allocated DFS stack to prevent call-stack overflow on
/// large or deeply coupled call graphs.
fn tarjan_sccs(n: usize, edges: &[BTreeSet<usize>]) -> Vec<Vec<usize>> {
    struct Frame<'a> {
        v: usize,
        iter: std::collections::btree_set::Iter<'a, usize>,
        child: Option<usize>,
    }

    let mut index = 0usize;
    let mut indices = vec![None; n];
    let mut lowlink = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut sccs = Vec::new();
    let mut dfs: Vec<Frame<'_>> = Vec::new();

    for root in 0..n {
        if indices[root].is_some() {
            continue;
        }
        indices[root] = Some(index);
        lowlink[root] = index;
        index += 1;
        stack.push(root);
        on_stack[root] = true;
        dfs.push(Frame {
            v: root,
            iter: edges[root].iter(),
            child: None,
        });

        while let Some(frame) = dfs.last_mut() {
            if let Some(w) = frame.child.take() {
                lowlink[frame.v] = lowlink[frame.v].min(lowlink[w]);
            }
            if let Some(&w) = frame.iter.next() {
                if indices[w].is_none() {
                    frame.child = Some(w);
                    indices[w] = Some(index);
                    lowlink[w] = index;
                    index += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    dfs.push(Frame {
                        v: w,
                        iter: edges[w].iter(),
                        child: None,
                    });
                } else if on_stack[w]
                    && let Some(w_index) = indices[w]
                {
                    lowlink[frame.v] = lowlink[frame.v].min(w_index);
                }
            } else {
                let v = frame.v;
                dfs.pop();
                if Some(lowlink[v]) == indices[v] {
                    let mut scc = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        scc.push(w);
                        if w == v {
                            break;
                        }
                    }
                    sccs.push(scc);
                }
            }
        }
    }

    sccs
}

struct Analyzer<'a, 'b> {
    index: &'a GraphIndex<'a>,
    certified: &'a [Option<BodySummary>],
    in_cycle: bool,
    fuel: &'b mut usize,
    is_ptr_slot: Vec<bool>,
    written: Vec<bool>,
    escaped: Vec<bool>,
    local_origins: BTreeMap<LocalId, PtrOrigin>,
    local_targets: BTreeMap<LocalId, Vec<LocalTarget<'a>>>,
    memory: MemoryEffect,
    terminates: bool,
    no_trap_or_unwind: bool,
    no_free: bool,
    no_sync: bool,
    callsites: Vec<CallsiteBehavior>,
}

fn is_pointer_type<'a>(
    graph: &'a iir::Package,
    mut ty: &'a Term,
    fuel: &mut usize,
) -> Option<bool> {
    for _ in 0..32 {
        spend(fuel, 1)?;
        match ty {
            Term::Pointer(_) => return Some(true),
            Term::Nominal { .. } => {
                let nominal = graph.nominals.iter().find(|n| &n.ty == ty)?;
                match &nominal.definition {
                    iir::NominalDefinition::Newtype(def) => {
                        ty = &def.representation;
                    }
                    iir::NominalDefinition::Struct(_) | iir::NominalDefinition::Enum(_) => {
                        return Some(false);
                    }
                }
            }
            _ => return Some(false),
        }
    }
    None
}

fn type_may_contain_pointer(graph: &iir::Package, ty: &Term, fuel: &mut usize) -> bool {
    let mut pending = vec![ty];
    let mut visited = HashSet::new();
    while let Some(cur) = pending.pop() {
        if spend(fuel, 1).is_none() || visited.len() >= 128 {
            return true;
        }
        if !visited.insert(cur) {
            continue;
        }
        match cur {
            Term::Pointer(_)
            | Term::TraitObject { .. }
            | Term::Function { .. }
            | Term::UnsafeFunction(_)
            | Term::PureFunction(_)
            | Term::Closure { .. } => return true,
            Term::Primitive(_) => {}
            Term::Tuple(items) => pending.extend(items.iter()),
            Term::Record(fields) => pending.extend(fields.iter().map(|(_, f)| f)),
            Term::Optional(inner) => pending.push(inner),
            Term::ErrorResult { errors, result } => {
                pending.extend(errors.iter());
                pending.push(result);
            }
            Term::Nominal { .. } => {
                let Some(nominal) = graph.nominals.iter().find(|n| &n.ty == cur) else {
                    return true;
                };
                match &nominal.definition {
                    iir::NominalDefinition::Newtype(def) => pending.push(&def.representation),
                    iir::NominalDefinition::Struct(def) => {
                        pending.extend(def.fields.iter().map(|(_, f)| f));
                    }
                    iir::NominalDefinition::Enum(def) => {
                        for variant in &def.variants {
                            pending.extend(variant.fields.iter());
                        }
                    }
                }
            }
            _ => return true,
        }
    }
    false
}

struct SlotInfo<'a> {
    is_ptr_slot: Vec<bool>,
    local_origins: BTreeMap<LocalId, PtrOrigin>,
    effect_row: &'a [nir::effects::Operation],
    logical_result: &'a Term,
}

fn collect_slot_info<'a>(
    index: &'a GraphIndex<'_>,
    idx: usize,
    fuel: &mut usize,
) -> Option<SlotInfo<'a>> {
    let func = &index.graph.functions[idx].function;
    if func.phase != nir::program::FunctionPhase::Runtime {
        return None;
    }
    let Term::Function { parameters, result } = func.signature.callable_signature() else {
        return None;
    };
    if parameters.len() != func.parameters.len() {
        return None;
    }

    let (effect_row, logical_result) = match &**result {
        Term::Computation { effects, result } => (effects.as_slice(), result.as_ref()),
        other => (&[][..], other),
    };
    let slot_count = parameters.len() + func.env_parameters.len();
    let mut is_ptr_slot = Vec::with_capacity(slot_count);
    let mut local_origins = BTreeMap::new();

    for (slot, (param_ty, binding)) in parameters.iter().zip(&func.parameters).enumerate() {
        let is_ptr = param_ty.sort == Sort::Runtime
            && is_pointer_type(index.graph, &param_ty.ty, fuel).unwrap_or(false);
        is_ptr_slot.push(is_ptr);
        if is_ptr || type_may_contain_pointer(index.graph, &param_ty.ty, fuel) {
            local_origins.insert(*binding, PtrOrigin::param(slot));
        }
    }
    for (offset, env) in func.env_parameters.iter().enumerate() {
        let slot = parameters.len() + offset;
        let is_ptr = is_pointer_type(index.graph, &env.ty, fuel).unwrap_or(false);
        is_ptr_slot.push(is_ptr);
        if is_ptr || type_may_contain_pointer(index.graph, &env.ty, fuel) {
            local_origins.insert(env.binding, PtrOrigin::param(slot));
        }
    }
    for snap in &func.entry_snapshots {
        if let Some(origin) = local_origins.get(&snap.parameter).cloned() {
            local_origins.insert(snap.binding, origin);
        }
    }

    Some(SlotInfo {
        is_ptr_slot,
        local_origins,
        effect_row,
        logical_result,
    })
}

fn optimistic_scc_seed(
    index: &GraphIndex<'_>,
    idx: usize,
    fuel: &mut usize,
) -> Option<BodySummary> {
    let slots = collect_slot_info(index, idx, fuel)?;
    let pointer_params = slots
        .is_ptr_slot
        .into_iter()
        .map(|is_ptr| {
            if is_ptr {
                PointerParamAttrs {
                    readonly: true,
                    noalias: false,
                    captures_none: true,
                    nonnull: true,
                }
            } else {
                PointerParamAttrs::default()
            }
        })
        .collect();
    Some(BodySummary {
        attrs: BehaviorAttributes {
            memory: MemoryEffect::None,
            terminates: false,
            no_trap_or_unwind: true,
            no_free: true,
            no_sync: true,
            pointer_params,
        },
        callsites: Vec::new(),
    })
}

fn analyze_instance(
    index: &GraphIndex<'_>,
    idx: usize,
    certified: &[Option<BodySummary>],
    in_cycle: bool,
    fuel: &mut usize,
) -> Option<BodySummary> {
    let instance = &index.graph.functions[idx];
    let func = &instance.function;
    let slots = collect_slot_info(index, idx, fuel)?;
    let slot_count = slots.is_ptr_slot.len();
    let effect_row = slots.effect_row;
    let logical_result = slots.logical_result;

    let mut analyzer = Analyzer {
        index,
        certified,
        in_cycle,
        fuel,
        is_ptr_slot: slots.is_ptr_slot,
        written: vec![false; slot_count],
        escaped: vec![false; slot_count],
        local_origins: slots.local_origins,
        local_targets: BTreeMap::new(),
        memory: MemoryEffect::None,
        terminates: !in_cycle,
        no_trap_or_unwind: true,
        no_free: true,
        no_sync: true,
        callsites: Vec::new(),
    };

    // Soundness Red Lines 2 & 4: preconditions, postconditions, effects, or Never exits
    // may transfer control, trap, or diverge.
    if !func.preconditions.is_empty()
        || !func.postconditions.is_empty()
        || !effect_row.is_empty()
        || *logical_result == Term::Primitive(P::Never)
    {
        analyzer.no_trap_or_unwind = false;
        analyzer.memory = MemoryEffect::Unknown;
        if *logical_result == Term::Primitive(P::Never) {
            analyzer.terminates = false;
        }
    }

    if let Some(schedule) = index.schedule {
        analyzer.check_cleanup_schedule(&instance.key, schedule)?;
    }

    let ret_origin = analyzer.visit_expr(&func.body, 0)?;
    if !ret_origin.is_empty() {
        analyzer.mark_escaped(&ret_origin);
    }

    let attrs = analyzer.finalize();
    Some(BodySummary {
        attrs,
        callsites: analyzer.callsites,
    })
}

impl<'a, 'b> Analyzer<'a, 'b> {
    fn fail_all_conservative(&mut self) {
        self.memory = MemoryEffect::Unknown;
        self.terminates = false;
        self.no_trap_or_unwind = false;
        self.no_free = false;
        self.no_sync = false;
        for slot in 0..self.written.len() {
            self.written[slot] = true;
            self.escaped[slot] = true;
        }
    }

    fn mark_trap(&mut self) {
        self.no_trap_or_unwind = false;
        self.memory = MemoryEffect::Unknown;
    }

    fn mark_volatile(&mut self, origin: Option<&PtrOrigin>) {
        self.memory = MemoryEffect::Unknown;
        self.no_sync = false;
        if let Some(origin) = origin {
            for &src in &origin.sources {
                if let PtrSource::Param(slot) = src
                    && slot < self.written.len()
                {
                    self.written[slot] = true;
                }
            }
        }
    }

    fn pointer_access_is_volatile(&mut self, pointer: &Expr, pointee_ty: &Term) -> bool {
        if self.index.graph.type_is_volatile(pointee_ty, self.fuel) {
            return true;
        }
        let mut cursor = pointer;
        let mut remaining = 64usize;
        while remaining > 0 {
            remaining -= 1;
            match &cursor.kind {
                K::PointerField {
                    pointer: inner,
                    declaration,
                    index,
                } => {
                    if self
                        .index
                        .graph
                        .volatile_storage
                        .is_field_volatile(*declaration, *index)
                    {
                        return true;
                    }
                    cursor = inner;
                }
                K::PointerAnonymousField { pointer: inner, .. } => cursor = inner,
                _ => break,
            }
        }
        false
    }

    fn place_is_volatile(&self, place: &nir::program::Place) -> bool {
        place.fields.iter().any(|(owner, index)| {
            matches!(
                owner,
                nir::program::FieldOwner::Nominal(declaration)
                    if self
                        .index
                        .graph
                        .volatile_storage
                        .is_field_volatile(*declaration, *index)
            )
        })
    }

    fn mark_escaped(&mut self, origin: &PtrOrigin) {
        for &src in &origin.sources {
            match src {
                PtrSource::Param(slot) if slot < self.escaped.len() => {
                    self.escaped[slot] = true;
                }
                _ => {}
            }
        }
    }

    fn mark_written(&mut self, origin: &PtrOrigin) {
        let mut saw_param = false;
        let mut saw_unknown = false;
        let mut saw_other = false;
        for &src in &origin.sources {
            match src {
                PtrSource::Param(slot) if slot < self.written.len() => {
                    self.written[slot] = true;
                    saw_param = true;
                }
                PtrSource::LocalAlloc(_) => {}
                PtrSource::ConstGlobal => saw_other = true,
                PtrSource::Unknown | PtrSource::Param(_) => saw_unknown = true,
            }
        }
        if saw_unknown || origin.is_empty() {
            self.written.fill(true);
            self.memory = MemoryEffect::Unknown;
        } else if saw_other {
            self.memory = MemoryEffect::Unknown;
        } else if saw_param {
            self.memory = self.memory.join(MemoryEffect::ArgMemReadWrite);
        }
    }

    fn mark_read(&mut self, origin: &PtrOrigin) {
        let mut saw_param = false;
        let mut saw_const = false;
        let mut saw_unknown = false;
        for &src in &origin.sources {
            match src {
                PtrSource::Param(slot) if slot < self.written.len() => saw_param = true,
                PtrSource::LocalAlloc(_) => {}
                PtrSource::ConstGlobal => saw_const = true,
                PtrSource::Unknown | PtrSource::Param(_) => saw_unknown = true,
            }
        }
        if saw_unknown || origin.is_empty() {
            self.memory = MemoryEffect::Unknown;
        } else if saw_const {
            self.memory = self.memory.join(MemoryEffect::ReadOnly);
        } else if saw_param {
            self.memory = self.memory.join(MemoryEffect::ArgMemRead);
        }
    }

    fn check_cleanup_schedule(
        &mut self,
        key: &iir::InstanceKey,
        schedule: &'a iir::cleanup::Schedule,
    ) -> Option<()> {
        for scope in schedule.functions.iter().filter(|s| &s.instance == key) {
            spend(self.fuel, 1)?;
            if !scope.pattern_orders.is_empty() {
                // `patterns.rs:541,574` emits runtime guards for residual binding receipts.
                self.mark_trap();
            }
            for site in &scope.sites {
                for action in &site.actions {
                    self.check_cleanup_program(&action.program)?;
                }
            }
            for discard in &scope.discards {
                self.check_cleanup_program(&discard.program)?;
            }
        }
        Some(())
    }

    fn check_cleanup_program(&mut self, program: &'a iir::cleanup::Program) -> Option<()> {
        if !program.guards.is_empty() {
            self.mark_trap();
        }
        self.check_cleanup_steps(&program.steps)
    }

    fn check_cleanup_steps(&mut self, steps: &'a [iir::cleanup::Step]) -> Option<()> {
        use iir::cleanup::Step;
        for step in steps {
            spend(self.fuel, 1)?;
            match step {
                Step::Release { .. } => {}
                Step::DropTraitObject { .. } => {
                    self.mark_trap();
                    self.fail_all_conservative();
                }
                Step::Select { .. } => {
                    // Runtime tag selection in `host/llvm/cleanup.rs` may guard unique arms.
                    self.mark_trap();
                }
                Step::IfConstructed { steps, .. } => {
                    self.mark_trap();
                    self.check_cleanup_steps(steps)?;
                }
                Step::Switch { branches, .. } => {
                    self.mark_trap();
                    for branch in branches {
                        self.check_cleanup_steps(branch)?;
                    }
                }
                Step::Invoke {
                    subject,
                    method,
                    env_arguments,
                    ..
                } => {
                    let mut env_origins = Vec::with_capacity(env_arguments.len());
                    for arg in env_arguments {
                        env_origins.push(self.visit_expr(arg, 0)?);
                    }
                    let target = self
                        .index
                        .by_key
                        .get(&(method.declaration, method.arguments.as_ref()))
                        .copied();
                    if let Some(target_idx) = target {
                        // The cleanup receiver is a slot in this frame. A pointer
                        // stored inside it can still derive from any incoming
                        // parameter. Receiver attributes cover the slot itself.
                        let may_publish_or_write =
                            self.certified[target_idx].as_ref().is_none_or(|body| {
                                matches!(
                                    body.attrs.memory,
                                    MemoryEffect::Unknown | MemoryEffect::ArgMemReadWrite
                                )
                            });
                        if may_publish_or_write
                            && type_may_contain_pointer(self.index.graph, subject, self.fuel)
                        {
                            self.written.fill(true);
                            self.escaped.fill(true);
                        }
                        self.apply_known_call(target_idx, &[PtrOrigin::unknown()], &env_origins);
                    } else {
                        self.fail_all_conservative();
                    }
                }
            }
        }
        Some(())
    }

    fn address_origin(&mut self, expr: &'a Expr, depth: usize) -> Option<PtrOrigin> {
        spend(self.fuel, 1)?;
        if depth > 256 {
            return None;
        }
        match &expr.kind {
            K::Local(id) => Some(PtrOrigin::local_alloc(*id)),
            K::Field { value, .. } | K::AnonymousField { value, .. } => {
                self.address_origin(value, depth + 1)
            }
            K::ReadPointer(pointer) => self.visit_expr(pointer, depth + 1),
            _ => Some(PtrOrigin::unknown()),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr, depth: usize) -> Option<PtrOrigin> {
        spend(self.fuel, 1)?;
        if depth > 512 {
            return None;
        }
        let next = depth + 1;
        match &expr.kind {
            K::Literal(
                Literal::Bool(_)
                | Literal::Char(_)
                | Literal::SignedInteger(_)
                | Literal::UnsignedInteger(_)
                | Literal::Float16(_)
                | Literal::BFloat16(_)
                | Literal::Float32(_)
                | Literal::Float64(_)
                | Literal::BitVec { .. }
                | Literal::String(_)
                | Literal::Symbol(_),
            )
            | K::Unit
            | K::Function(_)
            | K::FunctionInstance { .. }
            | K::TraitMethod { .. } => Some(PtrOrigin::default()),

            K::ConstAddress { .. } => Some(PtrOrigin::const_global()),

            K::Local(id) => {
                let origin = self.local_origins.get(id).cloned().unwrap_or_default();
                if self.index.graph.type_is_volatile(&expr.ty, self.fuel) {
                    self.mark_volatile(Some(&origin));
                }
                Some(origin)
            }

            K::AddressOf(inner) => self.address_origin(inner, next),

            K::PointerField { pointer, .. }
            | K::PointerAnonymousField { pointer, .. }
            | K::NewtypeConvert(pointer)
            | K::UnsafeScope(pointer) => self.visit_expr(pointer, next),

            K::Field {
                value: pointer,
                declaration,
                index,
            } => {
                let field_volatile = self
                    .index
                    .graph
                    .volatile_storage
                    .is_field_volatile(*declaration, *index)
                    || self.index.graph.type_is_volatile(&expr.ty, self.fuel);
                if field_volatile {
                    let origin = self.address_origin(pointer, next)?;
                    self.mark_volatile(Some(&origin));
                }
                self.visit_expr(pointer, next)
            }

            K::AnonymousField { value: pointer, .. } => self.visit_expr(pointer, next),

            K::ReadPointer(pointer) => {
                let origin = self.visit_expr(pointer, next)?;
                if self.pointer_access_is_volatile(pointer, &expr.ty) {
                    self.mark_volatile(Some(&origin));
                } else {
                    self.mark_read(&origin);
                }
                if type_may_contain_pointer(self.index.graph, &expr.ty, self.fuel) {
                    let mut result = PtrOrigin::unknown();
                    for &src in &origin.sources {
                        if let PtrSource::LocalAlloc(local_id) = src
                            && let Some(stored) = self.local_origins.get(&local_id)
                        {
                            result.join(stored);
                        }
                    }
                    Some(result)
                } else {
                    Some(PtrOrigin::default())
                }
            }

            K::WritePointer { pointer, value } => {
                let ptr_origin = self.visit_expr(pointer, next)?;
                let val_origin = self.visit_expr(value, next)?;
                self.mark_written(&ptr_origin);
                if self.pointer_access_is_volatile(pointer, &value.ty) {
                    self.mark_volatile(Some(&ptr_origin));
                }
                self.mark_escaped(&val_origin);
                Some(PtrOrigin::default())
            }

            K::Tuple(fields)
            | K::AnonymousRecord { fields }
            | K::Record { fields, .. }
            | K::Variant { fields, .. } => {
                let mut combined = PtrOrigin::default();
                for field in fields {
                    combined.join(&self.visit_expr(field, next)?);
                }
                Some(combined)
            }

            K::Closure {
                parameters: _,
                captures,
                body: _,
            } => {
                for capture in captures {
                    if let Some(init) = &capture.value {
                        let o = self.visit_expr(init, next)?;
                        self.mark_escaped(&o);
                    } else if let Some(o) = self.local_origins.get(&capture.binding).cloned() {
                        self.mark_escaped(&o);
                    }
                }
                Some(PtrOrigin::default())
            }

            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                for stmt in statements {
                    let _ = self.visit_expr(stmt, next)?;
                }
                if let Some(res) = result {
                    self.visit_expr(res, next)
                } else {
                    Some(PtrOrigin::default())
                }
            }

            K::Let { binding, value } => {
                let mut targets = Vec::new();
                self.resolve_local_targets(value, &mut targets);
                if !targets.is_empty() {
                    self.local_targets.insert(*binding, targets);
                } else {
                    self.local_targets.remove(binding);
                }
                let origin = self.visit_expr(value, next)?;
                if origin.is_empty() {
                    self.local_origins.remove(binding);
                } else {
                    self.local_origins.insert(*binding, origin);
                }
                Some(PtrOrigin::default())
            }

            K::Assign { place, value } => {
                let origin = self.visit_expr(value, next)?;
                if self.place_is_volatile(place)
                    || self.index.graph.type_is_volatile(&value.ty, self.fuel)
                {
                    let root_origin = self.local_origins.get(&place.root).cloned();
                    self.mark_volatile(root_origin.as_ref());
                }
                if !origin.is_empty() {
                    if place.fields.is_empty() {
                        self.local_origins
                            .entry(place.root)
                            .or_default()
                            .join(&origin);
                    } else {
                        self.mark_escaped(&origin);
                    }
                }
                self.local_targets.remove(&place.root);
                Some(PtrOrigin::default())
            }

            K::Discard(inner) => {
                let _ = self.visit_expr(inner, next)?;
                Some(PtrOrigin::default())
            }

            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let _ = self.visit_expr(condition, next)?;
                let mut combined = self.visit_expr(then_branch, next)?;
                if let Some(else_br) = else_branch {
                    combined.join(&self.visit_expr(else_br, next)?);
                }
                Some(combined)
            }

            K::Return(value) => {
                if let Some(inner) = value {
                    let origin = self.visit_expr(inner, next)?;
                    self.mark_escaped(&origin);
                }
                Some(PtrOrigin::default())
            }

            K::Loop { body, .. } => {
                // Red Line 1: Loops never allow `terminates = true` / `willreturn`.
                self.terminates = false;
                let _ = self.visit_expr(body, next)?;
                Some(PtrOrigin::default())
            }

            K::Break(_) | K::Continue(_) => Some(PtrOrigin::default()),

            K::Not(a)
            | K::FloatNegate(a)
            | K::TryIntegerConvert(a)
            | K::IntegerRepresentation(a)
            | K::TakeBits(a)
            | K::BitSlice { value: a, .. }
            | K::BitConvert(a)
            | K::SymbolName(a)
            | K::EnumName(a) => {
                let _ = self.visit_expr(a, next)?;
                Some(PtrOrigin::default())
            }

            K::And(a, b)
            | K::Or(a, b)
            | K::FloatBinary {
                left: a, right: b, ..
            }
            | K::BitEqual(a, b)
            | K::BitConcat(a, b) => {
                let _ = self.visit_expr(a, next)?;
                let _ = self.visit_expr(b, next)?;
                Some(PtrOrigin::default())
            }

            K::ScalarEqual(a, b)
            | K::ScalarLess(a, b)
            | K::ScalarLessEqual(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b)
            | K::TemplateEqual(a, b) => {
                let _ = self.visit_expr(a, next)?;
                let _ = self.visit_expr(b, next)?;
                if matches!(a.ty, Term::Primitive(P::Str | P::Symbol))
                    || matches!(b.ty, Term::Primitive(P::Str | P::Symbol))
                {
                    self.memory = self.memory.join(MemoryEffect::ReadOnly);
                }
                Some(PtrOrigin::default())
            }

            K::IntegerBinary { op, left, right } => {
                let _ = self.visit_expr(left, next)?;
                let _ = self.visit_expr(right, next)?;
                if integer_op_may_trap(*op) {
                    // Overflow / div-by-zero checks emit `self.guard`.
                    self.mark_trap();
                }
                Some(PtrOrigin::default())
            }

            K::IntegerNegate(inner) => {
                let _ = self.visit_expr(inner, next)?;
                // `0 - x` emits overflow guard on `INT_MIN`.
                self.mark_trap();
                Some(PtrOrigin::default())
            }

            K::CheckPredicate(cond) | K::RequirePredicate(cond) | K::EnsurePredicate(cond) => {
                let _ = self.visit_expr(cond, next)?;
                // A failing runtime check transfers control or traps.
                self.mark_trap();
                Some(PtrOrigin::default())
            }

            K::BitIndex(value, idx) => {
                let _ = self.visit_expr(value, next)?;
                let _ = self.visit_expr(idx, next)?;
                self.mark_trap();
                Some(PtrOrigin::default())
            }

            K::BitWindow { value, start, end } => {
                let _ = self.visit_expr(value, next)?;
                let _ = self.visit_expr(start, next)?;
                let _ = self.visit_expr(end, next)?;
                self.mark_trap();
                Some(PtrOrigin::default())
            }

            K::BitUpdate {
                value, selections, ..
            } => {
                let _ = self.visit_expr(value, next)?;
                for sel in selections {
                    match sel {
                        nir::program::BitSelection::Static { .. } => {}
                        nir::program::BitSelection::Index(idx) => {
                            let _ = self.visit_expr(idx, next)?;
                        }
                        nir::program::BitSelection::Window { start, end, .. } => {
                            let _ = self.visit_expr(start, next)?;
                            let _ = self.visit_expr(end, next)?;
                        }
                    }
                }
                self.mark_trap();
                Some(PtrOrigin::default())
            }

            K::BitWritePointer {
                pointer,
                selections,
                value,
            } => {
                let p_origin = self.visit_expr(pointer, next)?;
                for sel in selections {
                    match sel {
                        nir::program::BitSelection::Static { .. } => {}
                        nir::program::BitSelection::Index(idx) => {
                            let _ = self.visit_expr(idx, next)?;
                        }
                        nir::program::BitSelection::Window { start, end, .. } => {
                            let _ = self.visit_expr(start, next)?;
                            let _ = self.visit_expr(end, next)?;
                        }
                    }
                }
                let v_origin = self.visit_expr(value, next)?;
                self.mark_written(&p_origin);
                self.mark_escaped(&v_origin);
                self.mark_trap();
                Some(PtrOrigin::default())
            }

            K::TextQuery { subject, query } => {
                let _ = self.visit_expr(subject, next)?;
                if !query.is_length() {
                    // `byte_at` dereferences backing character pointer from `{ ptr, i64 }`.
                    self.memory = self.memory.join(MemoryEffect::ReadOnly);
                }
                Some(PtrOrigin::default())
            }

            K::Intrinsic { inputs, .. } => {
                for input in inputs {
                    let o = self.visit_expr(input, next)?;
                    self.mark_escaped(&o);
                }
                self.apply_intrinsic(expr);
                if type_may_contain_pointer(self.index.graph, &expr.ty, self.fuel) {
                    Some(PtrOrigin::unknown())
                } else {
                    Some(PtrOrigin::default())
                }
            }

            K::DynCast(cast) => {
                let val_origin = self.visit_expr(&cast.value, next)?;
                self.mark_escaped(&val_origin);
                for env_arg in &cast.env_arguments {
                    let o = self.visit_expr(env_arg, next)?;
                    self.mark_escaped(&o);
                }
                if !cast.by_pointer {
                    self.fail_all_conservative();
                }
                Some(PtrOrigin::unknown())
            }

            K::DynCall(call) => {
                let recv_origin = self.visit_expr(&call.receiver, next)?;
                self.mark_escaped(&recv_origin);
                for arg in &call.arguments {
                    let o = self.visit_expr(&arg.value, next)?;
                    self.mark_escaped(&o);
                }
                self.fail_all_conservative();
                if type_may_contain_pointer(self.index.graph, &expr.ty, self.fuel) {
                    Some(PtrOrigin::unknown())
                } else {
                    Some(PtrOrigin::default())
                }
            }

            K::Call {
                callee,
                arguments,
                parameter_bindings: _,
                env_arguments,
            } => {
                let _ = self.visit_expr(callee, next)?;
                let mut arg_origins = Vec::new();
                for arg in arguments {
                    let o = self.visit_expr(&arg.value, next)?;
                    if arg.parameter >= arg_origins.len() {
                        arg_origins.resize(arg.parameter + 1, PtrOrigin::default());
                    }
                    arg_origins[arg.parameter].join(&o);
                }
                let mut env_origins = Vec::with_capacity(env_arguments.len());
                for env_arg in env_arguments {
                    env_origins.push(self.visit_expr(env_arg, next)?);
                }

                if let Some(target_idx) = self.index.resolve_direct(callee) {
                    self.apply_known_call(target_idx, &arg_origins, &env_origins);
                } else if let Some(callsite_attrs) =
                    self.resolve_indirect_call(callee, &arg_origins, &env_origins)
                {
                    if !callsite_attrs.is_conservative() {
                        self.callsites.push(CallsiteBehavior {
                            source: expr.source,
                            behavior: callsite_attrs,
                        });
                    }
                } else {
                    for o in arg_origins.iter().chain(&env_origins) {
                        self.mark_escaped(o);
                    }
                    self.fail_all_conservative();
                }

                if type_may_contain_pointer(self.index.graph, &expr.ty, self.fuel) {
                    Some(PtrOrigin::unknown())
                } else {
                    Some(PtrOrigin::default())
                }
            }

            _ => {
                // Any unwhitelisted expression kind (`Match`, `LetPattern`, `Effect*`, etc.)
                // falls back to strictly conservative P0 behavior.
                self.fail_all_conservative();
                Some(PtrOrigin::unknown())
            }
        }
    }

    fn resolve_local_targets(&self, expr: &'a Expr, out: &mut Vec<LocalTarget<'a>>) {
        match &expr.kind {
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. } => {
                if let Some(idx) = self.index.resolve_direct(expr) {
                    out.push(LocalTarget::Function(idx));
                }
            }
            K::Closure { .. } => out.push(LocalTarget::Closure(expr)),
            K::Local(id) => {
                if let Some(targets) = self.local_targets.get(id) {
                    out.extend(targets.iter().copied());
                }
            }
            _ => {}
        }
    }

    fn resolve_indirect_call(
        &mut self,
        callee: &Expr,
        arg_origins: &[PtrOrigin],
        env_origins: &[PtrOrigin],
    ) -> Option<BehaviorAttributes> {
        let K::Local(id) = &callee.kind else {
            return None;
        };
        let targets = self.local_targets.get(id)?.clone();
        if targets.is_empty() {
            return None;
        }
        let mut joined: Option<BehaviorAttributes> = None;
        for target in targets {
            let behavior = match target {
                LocalTarget::Function(idx) => {
                    if let Some(summary) = &self.certified[idx] {
                        summary.attrs.clone()
                    } else {
                        return None;
                    }
                }
                LocalTarget::Closure(node) => self.analyze_closure_target(node)?,
            };
            joined = Some(match joined {
                Some(prev) => prev.join(&behavior),
                None => behavior,
            });
        }
        let behavior = joined?;
        self.apply_call_behavior(&behavior, arg_origins, env_origins);
        Some(behavior)
    }

    fn analyze_closure_target(&mut self, node: &'a Expr) -> Option<BehaviorAttributes> {
        let K::Closure {
            parameters,
            captures,
            body,
        } = &node.kind
        else {
            return None;
        };
        let env_types: Option<&[Term]> = match &node.ty {
            Term::Closure { environment, .. } => Some(environment.as_slice()),
            _ => None,
        };
        let mut capture_set = BTreeSet::new();
        let mut has_non_unit_capture = false;
        for (idx, c) in captures.iter().enumerate() {
            capture_set.insert(c.binding);
            let cap_ty = c
                .value
                .as_ref()
                .map(|v| &v.ty)
                .or_else(|| env_types.and_then(|env| env.get(idx)));
            let Some(ty) = cap_ty else {
                return None;
            };
            if *ty != Term::Primitive(P::Void) {
                has_non_unit_capture = true;
            }
            if type_may_contain_pointer(self.index.graph, ty, self.fuel) {
                return None;
            }
        }
        let mut sub = Analyzer {
            index: self.index,
            certified: self.certified,
            in_cycle: self.in_cycle,
            fuel: self.fuel,
            is_ptr_slot: vec![false; parameters.len()],
            written: vec![false; parameters.len()],
            escaped: vec![false; parameters.len()],
            local_origins: BTreeMap::new(),
            local_targets: BTreeMap::new(),
            memory: if has_non_unit_capture {
                MemoryEffect::ArgMemRead
            } else {
                MemoryEffect::None
            },
            terminates: !self.in_cycle,
            no_trap_or_unwind: true,
            no_free: true,
            no_sync: true,
            callsites: Vec::new(),
        };
        let mut capture_mutated = false;
        let _ = nir::visit::expression(body, sub.fuel, |expr, _| {
            if let K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } =
                &expr.kind
                && capture_set.contains(&place.root)
            {
                capture_mutated = true;
            }
            Ok(())
        });
        if capture_mutated {
            sub.memory = MemoryEffect::Unknown;
        }
        if sub.visit_expr(body, 0).is_none() {
            return None;
        }
        Some(sub.finalize())
    }

    fn apply_known_call(
        &mut self,
        target_idx: usize,
        arg_origins: &[PtrOrigin],
        env_origins: &[PtrOrigin],
    ) {
        let Some(callee_summary) = &self.certified[target_idx] else {
            for o in arg_origins.iter().chain(env_origins) {
                self.mark_escaped(o);
            }
            self.fail_all_conservative();
            return;
        };
        let callee_attrs = callee_summary.attrs.clone();
        self.apply_call_behavior(&callee_attrs, arg_origins, env_origins);
    }

    fn apply_call_behavior(
        &mut self,
        callee: &BehaviorAttributes,
        arg_origins: &[PtrOrigin],
        env_origins: &[PtrOrigin],
    ) {
        self.terminates &= callee.terminates;
        self.no_trap_or_unwind &= callee.no_trap_or_unwind;
        self.no_free &= callee.no_free;
        self.no_sync &= callee.no_sync;
        if !callee.no_trap_or_unwind {
            self.memory = MemoryEffect::Unknown;
        }

        // Propagate callee memory effects back to caller arguments and global effect.
        match callee.memory {
            MemoryEffect::None => {}
            MemoryEffect::ReadOnly => self.memory = self.memory.join(MemoryEffect::ReadOnly),
            MemoryEffect::Unknown => {
                self.memory = MemoryEffect::Unknown;
                for (slot, origin) in arg_origins.iter().chain(env_origins).enumerate() {
                    if origin.is_empty() {
                        continue;
                    }
                    let param_readonly =
                        callee.pointer_params.get(slot).is_some_and(|p| p.readonly);
                    if !param_readonly {
                        self.mark_written(origin);
                    } else {
                        self.mark_read(origin);
                    }
                }
            }
            MemoryEffect::ArgMemRead | MemoryEffect::ArgMemReadWrite => {
                let writing = callee.memory == MemoryEffect::ArgMemReadWrite;
                for (slot, origin) in arg_origins.iter().chain(env_origins).enumerate() {
                    if origin.is_empty() {
                        continue;
                    }
                    let param_readonly =
                        callee.pointer_params.get(slot).is_some_and(|p| p.readonly);
                    if writing && !param_readonly {
                        self.mark_written(origin);
                    } else {
                        self.mark_read(origin);
                    }
                }
            }
        }

        for (slot, origin) in arg_origins.iter().chain(env_origins).enumerate() {
            if origin.is_empty() {
                continue;
            }
            let param_captures_none = callee
                .pointer_params
                .get(slot)
                .is_some_and(|p| p.captures_none);
            if !param_captures_none {
                self.mark_escaped(origin);
            }
        }
    }

    fn apply_intrinsic(&mut self, expr: &Expr) {
        use nir::intrinsics::Operation as Op;
        let Ok(checked) = nir::intrinsics::checked(expr) else {
            self.fail_all_conservative();
            return;
        };
        match checked.operation() {
            Op::FusedMultiplyAdd
            | Op::SquareRoot
            | Op::CopySign
            | Op::Floor
            | Op::Ceiling
            | Op::Truncate
            | Op::RoundEven
            | Op::RoundAway
            | Op::AbsoluteValue
            | Op::Sine
            | Op::Cosine
            | Op::Exponential
            | Op::Exponential2
            | Op::NaturalLog
            | Op::Log2
            | Op::Log10
            | Op::Power
            | Op::IntegerPower
            | Op::ScaleByPowerOfTwo
            | Op::Minimum
            | Op::Maximum
            | Op::MinimumNumber
            | Op::MaximumNumber
            | Op::ByteSwap
            | Op::BitReverse
            | Op::BitAnd
            | Op::BitOr
            | Op::BitXor
            | Op::PopulationCount
            | Op::LeadingZeros
            | Op::TrailingZeros
            | Op::FunnelShiftLeft
            | Op::FunnelShiftRight
            | Op::IsNan
            | Op::IsInfinite
            | Op::IsFinite
            | Op::IsNormal
            | Op::IsSubnormal
            | Op::SignBit
            | Op::SaturatingAdd
            | Op::SaturatingSubtract
            | Op::IntegerMinimum
            | Op::IntegerMaximum
            | Op::WrappingAbsoluteValue => {}
            Op::MemRawRead | Op::MemCompareBytes | Op::MemBorrowStr => {
                self.memory = self.memory.join(MemoryEffect::ReadOnly);
            }
            Op::MemRawWrite | Op::MemCopy | Op::MemCopyNonOverlapping | Op::MemSetBytes => {
                self.memory = MemoryEffect::Unknown;
                self.written.fill(true);
            }
            Op::LinuxDealloc => {
                self.memory = MemoryEffect::Unknown;
                self.written.fill(true);
                self.no_free = false;
                self.no_sync = false;
                self.no_trap_or_unwind = false;
            }
            Op::LinuxReadByte | Op::LinuxReadRaw => {
                self.memory = MemoryEffect::Unknown;
                self.written.fill(true);
                self.no_sync = false;
                self.no_trap_or_unwind = false;
            }
            Op::LinuxWrite
            | Op::LinuxWriteByte
            | Op::LinuxOpen
            | Op::LinuxClose
            | Op::LinuxUnlink
            | Op::LinuxGetPid
            | Op::LinuxAlloc
            | Op::LinuxWriteRaw => {
                self.memory = MemoryEffect::Unknown;
                self.no_sync = false;
                self.no_trap_or_unwind = false;
            }
        }
    }

    fn finalize(&self) -> BehaviorAttributes {
        // Soundness Red Line 2: an unexcluded trap or failure transfer
        // forbids `memory(none)`, `memory(argmem: read)`, `nounwind`, and `speculatable`.
        let memory = if !self.no_trap_or_unwind {
            MemoryEffect::Unknown
        } else {
            self.memory
        };
        let ptr_param_count = self.is_ptr_slot.iter().filter(|is| **is).count();
        let single_argmem_ptr = ptr_param_count == 1
            && matches!(
                memory,
                MemoryEffect::ArgMemRead | MemoryEffect::ArgMemReadWrite
            );
        let pointer_params = self
            .is_ptr_slot
            .iter()
            .enumerate()
            .map(|(slot, &is_ptr)| {
                if !is_ptr {
                    PointerParamAttrs::default()
                } else {
                    PointerParamAttrs {
                        readonly: !self.written[slot] && !self.escaped[slot],
                        noalias: single_argmem_ptr && !self.escaped[slot],
                        captures_none: !self.escaped[slot],
                        nonnull: true,
                    }
                }
            })
            .collect();
        BehaviorAttributes {
            memory,
            terminates: self.terminates && self.no_trap_or_unwind,
            no_trap_or_unwind: self.no_trap_or_unwind,
            no_free: self.no_free,
            no_sync: self.no_sync,
            pointer_params,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::{Function, FunctionDeclaration, FunctionPhase, SourceRange};

    fn src() -> SourceRange {
        SourceRange { start: 1, end: 2 }
    }

    fn decl(local: u32) -> DeclarationId {
        DeclarationId { package: 1, local }
    }

    fn lid(index: u32) -> LocalId {
        LocalId { owner: 1, index }
    }

    fn expr(ty: Term, kind: K) -> Expr {
        Expr {
            source: src(),
            ty,
            kind,
        }
    }

    fn runtime_fn(
        local: u32,
        name: &str,
        params: Vec<(LocalId, Term)>,
        result: Term,
        body: Expr,
    ) -> iir::FunctionInstance {
        let sig = Term::Function {
            parameters: params
                .iter()
                .map(|(_, ty)| nir::Parameter {
                    sort: Sort::Runtime,
                    ty: ty.clone(),
                })
                .collect(),
            result: Box::new(result),
        };
        let key = iir::InstanceKey {
            declaration: decl(local),
            arguments: vec![].into(),
        };
        iir::FunctionInstance::new(
            key,
            Function {
                interface: FunctionDeclaration {
                    source: src(),
                    declaration: decl(local),
                    name: name.into(),
                    phase: FunctionPhase::Runtime,
                    inline: false,
                    pure: true,
                    is_unsafe: false,
                    preconditions: vec![],
                    postconditions: vec![],
                    result_binding: None,
                    result_type_binder: None,
                    result_guarantees: vec![],
                    entry_snapshots: vec![],
                    type_parameters: vec![],
                    requirements: vec![],
                    capabilities: vec![],
                    signature: sig,
                    parameters: params.into_iter().map(|(id, _)| id).collect(),
                    env_parameters: vec![],
                    defaults: vec![],
                },
                body,
                runtime_deliveries: Default::default(),
            },
        )
    }

    #[test]
    fn certifies_p5_vs_p4_loop_vs_trapping_and_pointer_roles() {
        let u32_ty = Term::Primitive(P::U32);
        let bool_ty = Term::Primitive(P::Bool);
        let ptr_u32_ty = Term::Pointer(Box::new(u32_ty.clone()));
        let p0 = lid(0);
        let p1 = lid(1);

        // 0: P5 pure total comparison
        let p5 = runtime_fn(
            1,
            "is_zero",
            vec![(p0, u32_ty.clone())],
            bool_ty.clone(),
            expr(
                bool_ty.clone(),
                K::IntegerBinary {
                    op: IntegerOp::Equal,
                    left: Box::new(expr(u32_ty.clone(), K::Local(p0))),
                    right: Box::new(expr(
                        u32_ty.clone(),
                        K::Literal(Literal::UnsignedInteger(0)),
                    )),
                },
            ),
        );

        // 1: P4 non-trapping loop (Red Line 1: no terminates)
        let loop_id = lid(10);
        let p4_loop = runtime_fn(
            2,
            "spin_forever",
            vec![],
            bool_ty.clone(),
            expr(
                bool_ty.clone(),
                K::Block {
                    statements: vec![expr(
                        Term::Primitive(P::Void),
                        K::Loop {
                            identity: loop_id,
                            body: Box::new(expr(Term::Primitive(P::Void), K::Unit)),
                        },
                    )],
                    result: Some(Box::new(expr(
                        bool_ty.clone(),
                        K::Literal(Literal::Bool(true)),
                    ))),
                },
            ),
        );

        // 2: Checked integer add (Red Line 2: trap or failure transfer excludes memory(none) / nounwind)
        let trapping_add = runtime_fn(
            3,
            "add_checked",
            vec![(p0, u32_ty.clone()), (p1, u32_ty.clone())],
            u32_ty.clone(),
            expr(
                u32_ty.clone(),
                K::IntegerBinary {
                    op: IntegerOp::Add,
                    left: Box::new(expr(u32_ty.clone(), K::Local(p0))),
                    right: Box::new(expr(u32_ty.clone(), K::Local(p1))),
                },
            ),
        );

        // 3: Read-only pointer parameter
        let read_ptr = runtime_fn(
            4,
            "read_u32",
            vec![(p0, ptr_u32_ty.clone())],
            u32_ty.clone(),
            expr(
                u32_ty.clone(),
                K::ReadPointer(Box::new(expr(ptr_u32_ty.clone(), K::Local(p0)))),
            ),
        );

        // 4: Mutable pointer parameter
        let write_ptr = runtime_fn(
            5,
            "write_u32",
            vec![(p0, ptr_u32_ty.clone())],
            Term::Primitive(P::Void),
            expr(
                Term::Primitive(P::Void),
                K::WritePointer {
                    pointer: Box::new(expr(ptr_u32_ty.clone(), K::Local(p0))),
                    value: Box::new(expr(
                        u32_ty.clone(),
                        K::Literal(Literal::UnsignedInteger(42)),
                    )),
                },
            ),
        );

        // 5: Escaping pointer parameter (returned)
        let escape_ptr = runtime_fn(
            6,
            "id_ptr",
            vec![(p0, ptr_u32_ty.clone())],
            ptr_u32_ty.clone(),
            expr(ptr_u32_ty.clone(), K::Local(p0)),
        );

        let mut pkg = iir::Package {
            functions: vec![p5, p4_loop, trapping_add, read_ptr, write_ptr, escape_ptr],
            ..Default::default()
        };
        let mut fuel = 100_000;
        certify_iir_package(&mut pkg, None, &mut fuel);

        let b0 = pkg.functions[0].behavior.as_ref().unwrap();
        assert_eq!(b0.memory, MemoryEffect::None);
        assert!(b0.terminates && b0.no_trap_or_unwind && b0.no_free && b0.no_sync);

        let b1 = pkg.functions[1].behavior.as_ref().unwrap();
        assert_eq!(b1.memory, MemoryEffect::None);
        assert!(!b1.terminates, "P4 loop must NOT set terminates");
        assert!(b1.no_trap_or_unwind && b1.no_free && b1.no_sync);

        let b2 = pkg.functions[2].behavior.as_ref().unwrap();
        assert!(!b2.no_trap_or_unwind && !b2.terminates);
        assert_eq!(b2.memory, MemoryEffect::Unknown);

        let b3 = pkg.functions[3].behavior.as_ref().unwrap();
        assert_eq!(b3.memory, MemoryEffect::ArgMemRead);
        assert!(b3.pointer_params[0].readonly);
        assert!(b3.pointer_params[0].captures_none);
        assert!(b3.pointer_params[0].noalias);

        let b4 = pkg.functions[4].behavior.as_ref().unwrap();
        assert_eq!(b4.memory, MemoryEffect::ArgMemReadWrite);
        assert!(!b4.pointer_params[0].readonly);
        assert!(b4.pointer_params[0].captures_none);
        assert!(b4.pointer_params[0].noalias);

        let b5 = pkg.functions[5].behavior.as_ref().unwrap();
        assert!(!b5.pointer_params[0].captures_none);
        assert!(!b5.pointer_params[0].readonly);
        assert!(!b5.pointer_params[0].noalias);
    }

    #[test]
    fn precondition_and_zero_fuel_fail_closed() {
        let bool_ty = Term::Primitive(P::Bool);
        let mut f = runtime_fn(
            1,
            "guarded",
            vec![],
            bool_ty.clone(),
            expr(bool_ty.clone(), K::Literal(Literal::Bool(true))),
        );
        f.function
            .interface
            .preconditions
            .push(expr(bool_ty.clone(), K::Literal(Literal::Bool(true))));
        let mut pkg = iir::Package {
            functions: vec![f],
            ..Default::default()
        };
        certify_iir_package(&mut pkg, None, &mut 10_000);
        let b = pkg.functions[0].behavior.as_ref().unwrap();
        assert!(!b.no_trap_or_unwind);
        assert!(!b.terminates);
        assert_eq!(b.memory, MemoryEffect::Unknown);

        let f2 = runtime_fn(
            2,
            "pure_unvisited",
            vec![],
            bool_ty.clone(),
            expr(bool_ty, K::Literal(Literal::Bool(true))),
        );
        let mut pkg2 = iir::Package {
            functions: vec![f2],
            ..Default::default()
        };
        certify_iir_package(&mut pkg2, None, &mut 0);
        assert!(pkg2.functions[0].behavior.is_none());
    }

    #[test]
    fn recursive_scc_forbids_terminates_and_indirect_call_records_callsite() {
        let u32_ty = Term::Primitive(P::U32);
        let bool_ty = Term::Primitive(P::Bool);
        let p0 = lid(0);
        let fn_sig = Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Runtime,
                ty: u32_ty.clone(),
            }],
            result: Box::new(bool_ty.clone()),
        };

        // 0: Recursive function `rec(n)` calling `rec(n)`
        let rec_fn = runtime_fn(
            1,
            "rec",
            vec![(p0, u32_ty.clone())],
            bool_ty.clone(),
            expr(
                bool_ty.clone(),
                K::Call {
                    callee: Box::new(expr(fn_sig.clone(), K::Function(decl(1)))),
                    arguments: vec![nir::program::CallArgument {
                        passing: nir::program::ArgumentPassing::Value,
                        parameter: 0,
                        value: expr(u32_ty.clone(), K::Local(p0)),
                        is_default: false,
                    }],
                    parameter_bindings: vec![p0],
                    env_arguments: vec![],
                },
            ),
        );

        // 1: Pure leaf `leaf(x) -> x == 0`
        let leaf_fn = runtime_fn(
            2,
            "leaf",
            vec![(p0, u32_ty.clone())],
            bool_ty.clone(),
            expr(
                bool_ty.clone(),
                K::IntegerBinary {
                    op: IntegerOp::Equal,
                    left: Box::new(expr(u32_ty.clone(), K::Local(p0))),
                    right: Box::new(expr(
                        u32_ty.clone(),
                        K::Literal(Literal::UnsignedInteger(0)),
                    )),
                },
            ),
        );

        // 2: Indirect call via local `let cb = leaf; cb(n)`
        let cb_id = lid(20);
        let indirect_caller = runtime_fn(
            3,
            "indirect_caller",
            vec![(p0, u32_ty.clone())],
            bool_ty.clone(),
            expr(
                bool_ty,
                K::Block {
                    statements: vec![expr(
                        Term::Primitive(P::Void),
                        K::Let {
                            binding: cb_id,
                            value: Box::new(expr(fn_sig.clone(), K::Function(decl(2)))),
                        },
                    )],
                    result: Some(Box::new(expr(
                        Term::Primitive(P::Bool),
                        K::Call {
                            callee: Box::new(expr(fn_sig, K::Local(cb_id))),
                            arguments: vec![nir::program::CallArgument {
                                passing: nir::program::ArgumentPassing::Value,
                                parameter: 0,
                                value: expr(u32_ty, K::Local(p0)),
                                is_default: false,
                            }],
                            parameter_bindings: vec![p0],
                            env_arguments: vec![],
                        },
                    ))),
                },
            ),
        );

        let mut pkg = iir::Package {
            functions: vec![rec_fn, leaf_fn, indirect_caller],
            ..Default::default()
        };
        certify_iir_package(&mut pkg, None, &mut 100_000);

        let b_rec = pkg.functions[0].behavior.as_ref().unwrap();
        assert!(!b_rec.terminates, "recursive SCC must NOT set terminates");
        assert!(b_rec.no_trap_or_unwind);
        assert_eq!(b_rec.memory, MemoryEffect::None);

        let b_indirect = pkg.functions[2].behavior.as_ref().unwrap();
        assert_eq!(b_indirect.memory, MemoryEffect::None);
        assert!(b_indirect.terminates && b_indirect.no_trap_or_unwind);
        assert_eq!(pkg.functions[2].callsite_behaviors.len(), 1);
        assert_eq!(
            pkg.functions[2].callsite_behaviors[0].behavior.memory,
            MemoryEffect::None
        );
    }

    #[test]
    fn reloaded_pointer_and_unknown_memory_callee_clear_param_readonly() {
        let u32_ty = Term::Primitive(P::U32);
        let ptr_u32_ty = Term::Pointer(Box::new(u32_ty.clone()));
        let ptr_ptr_u32_ty = Term::Pointer(Box::new(ptr_u32_ty.clone()));
        let p0 = lid(0);
        let p1 = lid(1);
        let slot_id = lid(2);
        let ref_id = lid(3);
        let q_id = lid(4);

        // (a) Write through pointer reloaded via ReadPointer on a local holding a param:
        // let slot = p; let r = slot.ref; let q = r.*; q.* = 99;
        let reload_write = runtime_fn(
            1,
            "reload_write",
            vec![(p0, ptr_u32_ty.clone())],
            Term::Primitive(P::Void),
            expr(
                Term::Primitive(P::Void),
                K::Block {
                    statements: vec![
                        expr(
                            Term::Primitive(P::Void),
                            K::Let {
                                binding: slot_id,
                                value: Box::new(expr(ptr_u32_ty.clone(), K::Local(p0))),
                            },
                        ),
                        expr(
                            Term::Primitive(P::Void),
                            K::Let {
                                binding: ref_id,
                                value: Box::new(expr(
                                    ptr_ptr_u32_ty.clone(),
                                    K::AddressOf(Box::new(expr(
                                        ptr_u32_ty.clone(),
                                        K::Local(slot_id),
                                    ))),
                                )),
                            },
                        ),
                        expr(
                            Term::Primitive(P::Void),
                            K::Let {
                                binding: q_id,
                                value: Box::new(expr(
                                    ptr_u32_ty.clone(),
                                    K::ReadPointer(Box::new(expr(
                                        ptr_ptr_u32_ty.clone(),
                                        K::Local(ref_id),
                                    ))),
                                )),
                            },
                        ),
                        expr(
                            Term::Primitive(P::Void),
                            K::WritePointer {
                                pointer: Box::new(expr(ptr_u32_ty.clone(), K::Local(q_id))),
                                value: Box::new(expr(
                                    u32_ty.clone(),
                                    K::Literal(Literal::UnsignedInteger(99)),
                                )),
                            },
                        ),
                    ],
                    result: Some(Box::new(expr(Term::Primitive(P::Void), K::Unit))),
                },
            ),
        );

        // (b) Returning a pointer reloaded via ReadPointer on a local holding a param
        // must clear captures_none, readonly, and noalias.
        let reload_escape = runtime_fn(
            2,
            "reload_escape",
            vec![(p0, ptr_u32_ty.clone())],
            ptr_u32_ty.clone(),
            expr(
                ptr_u32_ty.clone(),
                K::Block {
                    statements: vec![
                        expr(
                            Term::Primitive(P::Void),
                            K::Let {
                                binding: slot_id,
                                value: Box::new(expr(ptr_u32_ty.clone(), K::Local(p0))),
                            },
                        ),
                        expr(
                            Term::Primitive(P::Void),
                            K::Let {
                                binding: ref_id,
                                value: Box::new(expr(
                                    ptr_ptr_u32_ty.clone(),
                                    K::AddressOf(Box::new(expr(
                                        ptr_u32_ty.clone(),
                                        K::Local(slot_id),
                                    ))),
                                )),
                            },
                        ),
                    ],
                    result: Some(Box::new(expr(
                        ptr_u32_ty.clone(),
                        K::ReadPointer(Box::new(expr(ptr_ptr_u32_ty, K::Local(ref_id)))),
                    ))),
                },
            ),
        );

        // (c) Callee writes param AND traps (MemoryEffect::Unknown); caller passing p0
        // must also clear readonly on p0.
        let callee_sig = Term::Function {
            parameters: vec![
                nir::Parameter {
                    sort: Sort::Runtime,
                    ty: ptr_u32_ty.clone(),
                },
                nir::Parameter {
                    sort: Sort::Runtime,
                    ty: u32_ty.clone(),
                },
            ],
            result: Box::new(Term::Primitive(P::Void)),
        };
        let trapping_writer = runtime_fn(
            3,
            "trapping_writer",
            vec![(p0, ptr_u32_ty.clone()), (p1, u32_ty.clone())],
            Term::Primitive(P::Void),
            expr(
                Term::Primitive(P::Void),
                K::WritePointer {
                    pointer: Box::new(expr(ptr_u32_ty.clone(), K::Local(p0))),
                    value: Box::new(expr(
                        u32_ty.clone(),
                        K::IntegerBinary {
                            op: IntegerOp::Add,
                            left: Box::new(expr(u32_ty.clone(), K::Local(p1))),
                            right: Box::new(expr(
                                u32_ty.clone(),
                                K::Literal(Literal::UnsignedInteger(1)),
                            )),
                        },
                    )),
                },
            ),
        );
        let caller_of_trapping_writer = runtime_fn(
            4,
            "caller_of_trapping_writer",
            vec![(p0, ptr_u32_ty.clone()), (p1, u32_ty.clone())],
            Term::Primitive(P::Void),
            expr(
                Term::Primitive(P::Void),
                K::Call {
                    callee: Box::new(expr(callee_sig, K::Function(decl(3)))),
                    arguments: vec![
                        nir::program::CallArgument {
                            passing: nir::program::ArgumentPassing::Value,
                            parameter: 0,
                            value: expr(ptr_u32_ty, K::Local(p0)),
                            is_default: false,
                        },
                        nir::program::CallArgument {
                            passing: nir::program::ArgumentPassing::Value,
                            parameter: 1,
                            value: expr(u32_ty, K::Local(p1)),
                            is_default: false,
                        },
                    ],
                    parameter_bindings: vec![p0, p1],
                    env_arguments: vec![],
                },
            ),
        );

        let mut pkg = iir::Package {
            functions: vec![
                reload_write,
                reload_escape,
                trapping_writer,
                caller_of_trapping_writer,
            ],
            ..Default::default()
        };
        certify_iir_package(&mut pkg, None, &mut 100_000);

        let b_write = pkg.functions[0].behavior.as_ref().unwrap();
        assert_eq!(b_write.memory, MemoryEffect::Unknown);
        assert!(
            !b_write.pointer_params[0].readonly,
            "writing through reloaded pointer must clear param readonly"
        );

        let b_escape = pkg.functions[1].behavior.as_ref().unwrap();
        assert!(!b_escape.pointer_params[0].captures_none);
        assert!(!b_escape.pointer_params[0].readonly);
        assert!(!b_escape.pointer_params[0].noalias);

        let b_callee = pkg.functions[2].behavior.as_ref().unwrap();
        assert_eq!(b_callee.memory, MemoryEffect::Unknown);
        assert!(!b_callee.pointer_params[0].readonly);

        let b_caller = pkg.functions[3].behavior.as_ref().unwrap();
        assert_eq!(b_caller.memory, MemoryEffect::Unknown);
        assert!(
            !b_caller.pointer_params[0].readonly,
            "passing param to MemoryEffect::Unknown non-readonly param must clear caller readonly"
        );
    }

    #[test]
    fn iterative_tarjan_sccs_handles_deep_20000_node_chain_and_cycles_without_stack_overflow() {
        let chain_len = 20_000usize;
        let mut edges = vec![BTreeSet::new(); chain_len];
        for i in 0..chain_len - 1 {
            edges[i].insert(i + 1);
        }
        let sccs = tarjan_sccs(chain_len, &edges);
        assert_eq!(sccs.len(), chain_len);
        for (pos, scc) in sccs.iter().enumerate() {
            assert_eq!(scc.as_slice(), &[chain_len - 1 - pos]);
        }

        // Add a backedge from the tail to node 10_000, creating a 10,000-node cycle
        // reachable from a 10,000-node prefix chain; verify callee-before-caller order.
        edges[chain_len - 1].insert(10_000);
        let sccs = tarjan_sccs(chain_len, &edges);
        assert_eq!(sccs.len(), 10_001);
        assert_eq!(sccs[0].len(), 10_000);
        let cycle_members: BTreeSet<usize> = sccs[0].iter().copied().collect();
        assert_eq!(cycle_members, (10_000..chain_len).collect());
        for pos in 1..sccs.len() {
            assert_eq!(sccs[pos].as_slice(), &[10_000 - pos]);
        }
    }
}
