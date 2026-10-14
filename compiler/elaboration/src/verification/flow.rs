//! Ordered scalar-range analysis of the supported NIR control-flow fragment.
//! This observes checked program facts; it is not a type solver or an effect
//! verifier. Unsupported constructs return an explicit analysis limitation.
mod copies;
mod nominal;
mod paths;
mod patterns;
mod operators;
mod bit_indices;
mod memory_contracts;
use paths::{Component, FieldPath, Projection, record_path};

use super::{
    Comparison as C, Condition, Facts, Interval, Limits, PlaceId, Polynomial, Unknown, ValueId,
};
use comptime_runtime::Integer;
use nir::{
    Term,
    program::{Expr, ExprKind as K, Function, IntegerOp, Literal, LocalId, SourceRange},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

pub struct Report {
    pub points: Vec<Point>,
    pub exit: Facts,
}
pub struct Point {
    pub source: SourceRange,
    pub kind: PointKind,
    pub closure_context: Vec<SourceRange>,
    pub range: Option<Interval>,
    pub facts: Facts,
    /// State before the check publishes any new successful-path facts.
    pub check_input: Option<CheckInput>,
    aggregate_snapshots: BTreeMap<PlaceId, Value>,
    field_paths: BTreeMap<FieldPath, ValueId>,
}
/// Optional proof queries use an explicit caller budget, independently of the
/// mandatory arithmetic analysis. Unknown leaves the executable check intact.
pub struct CheckInput {
    // Ephemeral identity, consumed only against this exact unchanged tree.
    // Never serialized or reused after cloning/moving the owning expression.
    pub(super) site: usize,
    pub condition: Condition,
    pub facts: Facts,
}
impl CheckInput {
    pub fn prove(&self, limits: &mut Limits) -> Result<super::PredicateProof, Unknown> {
        self.facts.prove_condition(&self.condition, true, limits)
    }
}
impl fmt::Display for CheckInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(check-input (condition {}) {})",
            self.condition, self.facts
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointKind {
    Check,
    Postcondition,
    Arithmetic(IntegerOp),
    Negate,
    BitIndex,
    BitWindow,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Error {
    pub source: SourceRange,
    pub reason: Reason,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Reason {
    Unknown(Unknown),
    Unsupported,
    DepthLimit,
    InvalidSignature,
    MissingMachineProof,
    MissingDivisionProof,
    MissingBitIndexProof,
    MissingBitWindowProof,
    MissingPreconditionProof,
    MissingPostconditionProof,
    Phase(&'static str),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(flow-error (source {} {}) (reason ",
            self.source.start, self.source.end
        )?;
        match &self.reason {
            Reason::Unknown(reason) => reason.fmt(f)?,
            Reason::Unsupported => f.write_str("unsupported-construct")?,
            Reason::DepthLimit => f.write_str("depth-limit")?,
            Reason::InvalidSignature => f.write_str("invalid-signature")?,
            Reason::MissingBitWindowProof => write!(f, "{:?}",
                "BitVec slice bounds or constant width are not proven; add an earlier checked assumes or a stronger contract")?,
            Reason::MissingBitIndexProof => write!(f, "{:?}",
                "BitVec index is not proven in bounds; add an earlier checked assumes or a stronger contract")?,
            Reason::MissingDivisionProof => write!(f, "{:?}",
                "integer divisor is not proven nonzero; establish a positive or negative range before division")?,
            Reason::MissingMachineProof => write!(
                f,
                "{:?}",
                "machine arithmetic range is not proven; add an earlier checked assumes or a stronger contract"
            )?,
            Reason::MissingPreconditionProof => write!(
                f,
                "{:?}",
                "program precondition is not proven; establish it before the call with checked assumes"
            )?,
            Reason::MissingPostconditionProof => write!(
                f,
                "{:?}",
                "program postcondition is not proven on this return path"
            )?,
            Reason::Phase(message) => write!(f, "{message:?}")?,
        }
        f.write_str("))")
    }
}
impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(program-flow (points")?;
        for point in &self.points {
            write!(f, " {point}")?;
        }
        write!(f, ") (exit {}))", self.exit)
    }
}
impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(point (source {} {}) (kind ",
            self.source.start, self.source.end
        )?;
        match self.kind {
            PointKind::Check => f.write_str("check")?,
            PointKind::Postcondition => f.write_str("postcondition")?,
            PointKind::Arithmetic(op) => write!(f, "(arithmetic {op})")?,
            PointKind::Negate => f.write_str("negate")?,
            PointKind::BitIndex => f.write_str("bit-index")?,
            PointKind::BitWindow => f.write_str("bit-window")?,
        }
        f.write_str(") (closure-context")?;
        for source in &self.closure_context {
            write!(f, " (source {} {})", source.start, source.end)?;
        }
        f.write_str(") (range ")?;
        match &self.range {
            Some(range) => range.fmt(f)?,
            None => f.write_str("unknown")?,
        }
        write!(f, ") {}", self.facts)?;
        if let Some(input) = &self.check_input {
            write!(f, " {input}")?;
        }
        f.write_str(" (aggregate-snapshots")?;
        for (place, value) in &self.aggregate_snapshots {
            write!(
                f,
                " (aggregate (place {} {}) {value})",
                place.owner, place.local
            )?;
        }
        f.write_str(") (field-paths")?;
        for (path, value) in &self.field_paths {
            write!(f, " ({path} {value})")?;
        }
        f.write_str("))")
    }
}
#[derive(Default)]
struct Value {
    pointer_target: Option<(u64, Vec<nir::program::Place>)>,
    evidence: Option<std::sync::Arc<copies::PreparedCopy>>,
    polynomial: Option<Polynomial>,
    projection: Option<Projection>,
    range: Option<Interval>,
    scalar: Option<Scalar>,
    condition: Option<Condition>,
    fields: Option<Vec<Value>>,
    aggregate_kind: Option<AggregateKind>,
    nominal_type: Option<std::sync::Arc<Term>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum AggregateKind {
    AnonymousRecord,
    Tuple,
    Record(nir::DeclarationId),
    Variant(nir::DeclarationId, usize),
}
impl fmt::Display for AggregateKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AnonymousRecord => f.write_str("(anonymous-record)"),
            Self::Tuple => f.write_str("tuple"),
            Self::Record(id) => write!(f, "(record {id})"),
            Self::Variant(id, index) => write!(f, "(variant {id} {index})"),
        }
    }
}
enum Scalar {
    Local(ValueId),
    Constant(Integer),
}
struct State {
    frame: u64,
    facts: Facts,
    domains: BTreeMap<PlaceId, Option<Interval>>,
    aggregates: BTreeMap<PlaceId, Value>,
    paths: BTreeMap<FieldPath, ValueId>,
    frozen: BTreeMap<PlaceId, Term>,
    // Same-address observations, not ownership roots or disjointness evidence.
    // A pointer binding and its field path survive only until a write barrier.
    reads: BTreeMap<FieldPath, (Term, PlaceId)>,
    // Copy-time pointer identities. Both binding versions must still match.
    read_aliases: BTreeMap<PlaceId, (FieldPath, ValueId, ValueId)>,
}
impl State {
    fn fork(&self, limits: &mut Limits) -> Result<Self, Unknown> {
        limits.step(self.frozen.len())?;
        self.charge_read_paths(limits)?;
        let mut domains = BTreeMap::new();
        for (place, bound) in &self.domains {
            limits.step(1)?;
            domains.insert(*place, bound.as_ref().map(|b| b.copy(limits)).transpose()?);
        }
        Ok(Self {
            frame: self.frame,
            facts: self.facts.fork(limits)?,
            domains,
            aggregates: copy_aggregates(&self.aggregates, limits)?,
            paths: paths::copy_paths(&self.paths, limits)?,
            frozen: self.frozen.clone(),
            reads: self.reads.clone(),
            read_aliases: self.read_aliases.clone(),
        })
    }
    fn charge_read_paths(&self, limits: &mut Limits) -> Result<(), Unknown> {
        for (path, _, _) in self.read_aliases.values() {
            limits.step(path.fields.len().saturating_add(1))?;
        }
        for path in self.reads.keys() {
            limits.step(path.fields.len().saturating_add(1))?;
        }
        Ok(())
    }
    fn forget_reads(&mut self, limits: &mut Limits) -> Result<(), Unknown> {
        limits.step(self.reads.len())?;
        self.reads.clear();
        Ok(())
    }
    fn havoc(&mut self, limits: &mut Limits) -> Result<(), Unknown> {
        self.havoc_with_private_scalars(None, limits)
    }
    fn havoc_with_private_scalars(
        &mut self, addressed: Option<&BTreeSet<LocalId>>, limits: &mut Limits,
    ) -> Result<(), Unknown> {
        self.forget_reads(limits)?;
        if !self.facts.is_reachable() {
            return Ok(());
        }
        self.invalidate_paths(None, &[], limits)?;
        limits.step(self.aggregates.len())?;
        let mut forgotten = Vec::new();
        for (place, value) in &mut self.aggregates {
            if self.frozen.contains_key(place) {
                continue;
            }
            if !value.forget_observations(limits)? {
                forgotten.push(*place);
            }
        }
        for place in forgotten {
            self.aggregates.remove(&place);
        }
        for (place, domain) in &self.domains {
            if self.frozen.contains_key(place) {
                continue;
            }
            limits.step(1)?;
            // Synthetic observations are not lexical locals. Preserve only
            // scalar by-value storage that has never had its address exposed.
            if domain.is_some() && addressed.is_some_and(|addressed|
                u32::try_from(place.owner).ok().is_some_and(|owner|
                    !addressed.contains(&LocalId { owner, index: place.local }))) {
                continue;
            }
            match domain {
                Some(bound) => {
                    self.facts.assign(*place, bound.copy(limits)?)?;
                }
                None => {
                    self.facts.invalidate(*place)?;
                }
            }
        }
        Ok(())
    }
}
/// Conservative entry stability for a repeatedly callable environment. This
/// is a program-state footprint, independent of trait/type proposition solving.
fn mutable_capture_paths(
    body: &Expr,
    captures: &[nir::program::Capture],
    limits: &mut Limits,
) -> Result<BTreeMap<LocalId, BTreeSet<Vec<(nir::program::FieldOwner, usize)>>>, Reason> {
    if captures.is_empty() {
        return Ok(BTreeMap::new());
    }
    fn may_alias(ty: &Term, limits: &mut Limits, depth: usize) -> Result<bool, Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        match ty {
            Term::Primitive(_) | Term::Function { .. } | Term::ComptimeFunction(_) => Ok(false),
            Term::Record(fields) => {
                for (_, ty) in fields {
                    if may_alias(ty, limits, depth + 1)? { return Ok(true); }
                }
                Ok(false)
            }
            Term::Tuple(fields)
            | Term::Closure {
                environment: fields,
                ..
            } => {
                for field in fields {
                    if may_alias(field, limits, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            // Nominal/opaque layouts require a memory preservation contract.
            _ => Ok(true),
        }
    }
    let mut roots = BTreeSet::new();
    let mut aliases = false;
    for capture in captures {
        roots.insert(capture.binding);
        aliases |= may_alias(&capture.ty, limits, 0)?;
    }
    let mut changed: BTreeMap<LocalId, BTreeSet<Vec<(nir::program::FieldOwner, usize)>>> =
        BTreeMap::new();
    let mut unknown_write = false;
    let mut fuel = limits.arithmetic.steps;
    let result = iir::visit::expression(body, &mut fuel, |expression, fuel| {
        match &expression.kind {
            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate {place,..} if roots.contains(&place.root) => {
                *fuel = fuel
                    .checked_sub(place.fields.len())
                    .ok_or("capture path budget exhausted")?;
                changed
                    .entry(place.root)
                    .or_default()
                    .insert(place.fields.clone());
            }
            K::AddressOf(_) | K::WritePointer { .. } | K::BitWritePointer {..} => unknown_write = true,
            K::Call { .. } | K::EffectHandle { .. } | K::EffectRequest { .. } | K::EffectRun { .. } if aliases => unknown_write = true,
            _ => {}
        }
        Ok(())
    });
    limits.arithmetic.steps = fuel;
    result.map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
    if unknown_write {
        Ok(roots
            .into_iter()
            .map(|root| (root, BTreeSet::from([vec![]])))
            .collect())
    } else {
        Ok(changed)
    }
}

fn place(id: LocalId) -> PlaceId {
    PlaceId {
        owner: u64::from(id.owner),
        local: id.index,
    }
}

fn insert_pointer_target(
    targets: &mut Vec<nir::program::Place>,
    target: nir::program::Place,
    fuel: &mut usize,
) -> Result<(), Unknown> {
    let mut index = targets.len();
    for (offset, previous) in targets.iter().enumerate() {
        *fuel = fuel
            .checked_sub(
                previous
                    .fields
                    .len()
                    .saturating_add(target.fields.len())
                    .saturating_add(1),
            )
            .ok_or_else(|| Unknown::from(comptime_runtime::Error::BudgetExhausted))?;
        match (&previous.root, &previous.fields).cmp(&(&target.root, &target.fields)) {
            std::cmp::Ordering::Equal => return Ok(()),
            std::cmp::Ordering::Greater => {
                index = offset;
                break;
            }
            std::cmp::Ordering::Less => {}
        }
    }
    *fuel = fuel
        .checked_sub(targets.len().saturating_add(1))
        .ok_or_else(|| Unknown::from(comptime_runtime::Error::BudgetExhausted))?;
    targets.insert(index, target);
    Ok(())
}

/// Recognize only a syntactically fixed address of owned local storage. Pointer
/// locals, calls, branches and arbitrary pointer computations remain unknown.
fn direct_pointer_place(
    expr: &Expr,
    fuel: &mut usize,
) -> Result<Option<nir::program::Place>, String> {
    syntactic_place(expr, fuel, true)
}
fn field_owner_kind(owner: nir::program::FieldOwner) -> AggregateKind {
    match owner {
        nir::program::FieldOwner::Nominal(id) => AggregateKind::Record(id),
        nir::program::FieldOwner::Anonymous => AggregateKind::AnonymousRecord,
    }
}
fn syntactic_place(
    expr: &Expr,
    fuel: &mut usize,
    mut pointer: bool,
) -> Result<Option<nir::program::Place>, String> {
    let mut current = expr;
    let mut fields = Vec::new();
    for _ in 0..128 {
        *fuel = fuel.checked_sub(1).ok_or("pointer path budget exhausted")?;
        match (&current.kind, pointer) {
            (
                K::PointerField {
                    pointer: base,
                    declaration,
                    index,
                },
                true,
            ) => {
                fields.push((nir::program::FieldOwner::Nominal(*declaration), *index));
                current = base;
            }
            (K::PointerAnonymousField { pointer: base, index, .. }, true) => {
                fields.push((nir::program::FieldOwner::Anonymous, *index));
                current = base;
            }
            (K::AnonymousField { value, index, .. }, false) => {
                fields.push((nir::program::FieldOwner::Anonymous, *index));
                current = value;
            }
            (K::AddressOf(value), true) => {
                pointer = false;
                current = value;
            }
            (
                K::Field {
                    value,
                    declaration,
                    index,
                },
                false,
            ) => {
                fields.push((nir::program::FieldOwner::Nominal(*declaration), *index));
                current = value;
            }
            (K::Local(root), false) => {
                fields.reverse();
                return Ok(Some(nir::program::Place {
                    root: *root,
                    fields,
                }));
            }
            _ => return Ok(None),
        }
    }
    Ok(None)
}
fn domain(ty: &Term, limits: &mut Limits) -> Result<Option<Interval>, Unknown> {
    let Term::Primitive(primitive) = ty else {
        return Ok(None);
    };
    // Logical 0/1 encoding in the program prover, not a runtime ABI/layout.
    if *primitive == nir::Primitive::Bool {
        return Ok(Some(Interval::new(
            Integer::zero(),
            Integer::from_u128(1),
            limits,
        )?));
    }
    primitive
        .fixed_integer_layout()
        .map(|(signed, width)| Interval::machine(width as u16, signed, limits))
        .transpose()
}
fn hull(a: &Interval, b: &Interval, limits: &mut Limits) -> Result<Interval, Unknown> {
    let lower = if limits.compare(a.lower(), b.lower())?.is_lt() {
        a.lower()
    } else {
        b.lower()
    };
    let upper = if limits.compare(a.upper(), b.upper())?.is_gt() {
        a.upper()
    } else {
        b.upper()
    };
    Interval::new(limits.integer(lower)?, limits.integer(upper)?, limits)
}
fn bind(
    state: &mut State,
    local: LocalId,
    ty: &Term,
    bound: Option<Interval>,
    limits: &mut Limits,
) -> Result<(), Unknown> {
    bind_place(state, place(local), ty, bound, limits)
}
fn bind_place(
    state: &mut State,
    place: PlaceId,
    ty: &Term,
    bound: Option<Interval>,
    limits: &mut Limits,
) -> Result<(), Unknown> {
    // Rebinding an existing pointer name invalidates its observation identity,
    // including malformed/reused IR bindings and pattern-delivered parameters.
    if let Ok(owner) = u32::try_from(place.owner) {
        limits.step(state.reads.len())?;
        let root = self::place(LocalId {owner, index: place.local});
        state.reads.retain(|address, _| address.root != root);
    }
    state.invalidate_paths(Some(place), &[], limits)?;
    state.aggregates.remove(&place);
    if matches!(ty, Term::Nominal { .. } | Term::Tuple(_)) {
        state.aggregates.insert(place, Value::unknown(ty, limits)?);
    }
    let domain = domain(ty, limits)?;
    let bound = match (bound, &domain) {
        (Some(bound), _) => Some(bound),
        (None, Some(domain)) => Some(domain.copy(limits)?),
        _ => None,
    };
    state.domains.insert(place, domain);
    match bound {
        Some(bound) => {
            state.facts.assign(place, bound)?;
        }
        None => {
            state.facts.invalidate(place)?;
        }
    }
    Ok(())
}

pub fn analyze(function: &Function, limits: &mut Limits) -> Result<Report, Error> {
    analyze_impl(function, None, limits)
}
pub fn analyze_in_package(
    package: &nir::program::Package,
    function: &Function,
    limits: &mut Limits,
) -> Result<Report, Error> {
    analyze_impl(function, Some(package), limits)
}
fn analyze_impl(
    function: &Function,
    package: Option<&nir::program::Package>,
    limits: &mut Limits,
) -> Result<Report, Error> {
    let source = function.body.source;
    let error = |reason| Error { source, reason };
    let Term::Function { parameters, result } = function.signature.callable_signature() else {
        return Err(error(Reason::InvalidSignature));
    };
    if parameters.len() != function.parameters.len() {
        return Err(error(Reason::InvalidSignature));
    }
    let mut state = State {
        frame: 0,
        facts: Facts::default(),
        domains: BTreeMap::new(),
        aggregates: BTreeMap::new(),
        paths: BTreeMap::new(),
        frozen: BTreeMap::new(),
        reads: BTreeMap::new(), read_aliases: BTreeMap::new(),
    };
    // Type premises determine scalar domains; value premises below determine
    // facts within those domains. Never send program predicates to PTC.
    let mut type_context = crate::ptc::Context::with_scope(
        vec![parameters.iter().map(|parameter| parameter.sort).collect()],
    );
    let mut contract_budget = 4096;
    for requirement in &function.requirements {
        let formula = crate::ptc::Formula::from_predicate(requirement);
        let before = contract_budget;
        let admitted = crate::ptc::within_limits(&formula, 128, &mut contract_budget);
        limits.step(before - contract_budget).map_err(|e| error(Reason::Unknown(e)))?;
        if !admitted { return Err(error(Reason::Unsupported)); }
        type_context.assume(formula);
    }
    for (binding, ty) in function.parameters.iter().zip(parameters) {
        let mut view = None;
        if matches!(ty.ty, Term::Bound { .. } | Term::Rigid { .. } | Term::NormalForm { .. } | Term::AssociatedType { .. } | Term::Hole { .. }) {
            limits.step(4096).map_err(|e| error(Reason::Unknown(e)))?;
            view = type_context.structural_view(&ty.ty, crate::ptc::Budget { steps: 4096, ..Default::default() })
                .ok().flatten().map(|(ty, _)| ty)
                .filter(|ty| matches!(ty, Term::Primitive(_)));
        }
        bind(&mut state, *binding, view.as_ref().unwrap_or(&ty.ty), None, limits)
            .map_err(|e| error(Reason::Unknown(e)))?;
    }
    // Only explicit address formation can expose a by-value local to pointer
    // aliases. Scan the complete checked function, including nested closures,
    // contracts and defaults; future address-taking is conservatively included.
    let mut addressed_locals = BTreeSet::new();
    iir::visit::expressions(function, &mut limits.arithmetic.steps, |expr, fuel| {
        if let K::EffectHandle {handlers,..}=&expr.kind {
            for handler in handlers {for arm in &handler.arms {
                for value in arm.guard.iter().chain(std::iter::once(&arm.body)) {
                    nir::visit::expression(value,fuel,|expr,_| {
                        if let K::Assign {place,..} | K::BitAssign {place,..} | K::BitUpdate {place,..}=&expr.kind {
                            addressed_locals.insert(place.root);
                        }
                        Ok(())
                    })?;
                }
            }}
        }
        if let K::AddressOf(value) = &expr.kind {
            iir::visit::expression(value, fuel, |operand, _| {
                if let K::Local(binding) = operand.kind { addressed_locals.insert(binding); }
                Ok(())
            })?;
        }
        Ok(())
    }).map_err(|_| error(Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into())))?;
    let mut walker = Walker {
        addressed_locals,
        type_context,
        input_frames: vec![parameters.iter().map(|p|p.sort).collect()],
        next_frame: 1,
        limits,
        package,
        points: vec![],
        failure_source: None,
        closure_context: vec![],
        single_pass_exits:BTreeMap::new(),
        next_field: 0,
        assuming_postcondition: false,
        memory_summaries: std::collections::HashMap::new(),
        exit_contract: Some(ExitContract {
            conditions: &function.postconditions,
            binding: function.result_binding,
            ty: if let Term::Computation {result,..}=result.as_ref() {result.as_ref()} else {result.as_ref()},
        }),
    };
    // Publish aggregate field domains before preconditions and entry snapshots.
    // This is an analysis representation; declaration-side types stay intact.
    for (binding, parameter) in function.parameters.iter().zip(parameters) {
        let value = walker.unknown_value(&parameter.ty).map_err(&error)?;
        if value.fields.is_some() || value.nominal_type.is_some() {
            state.aggregates.insert(place(*binding), value);
        }
    }
    for condition in &function.preconditions {
        let checked = walker.eval(condition, &mut state, 0)?;
        let Some(condition) = checked.condition else {
            return Err(error(Reason::Unsupported));
        };
        state
            .facts
            .assume(&condition, true, walker.limits)
            .map_err(|e| error(Reason::Unknown(e)))?;
    }
    for snapshot in &function.entry_snapshots {
        let index = function
            .parameters
            .iter()
            .position(|p| *p == snapshot.parameter)
            .ok_or_else(|| error(Reason::InvalidSignature))?;
        walker
            .materialize_entry_fields(&mut state, snapshot.parameter)
            .map_err(|reason| error(reason))?;
        let value = walker
            .local_value(&state, snapshot.parameter)
            .map_err(|reason| error(reason))?;
        walker
            .bind_related_value(&mut state, snapshot.binding, &parameters[index].ty, value)
            .map_err(|reason| error(reason))?;
        state
            .frozen
            .insert(place(snapshot.binding), parameters[index].ty.clone());
    }
    if function.postconditions.is_empty() {
        walker.eval(&function.body, &mut state, 0)?;
    } else {
        walker
            .tail(&function.body, &mut state, 0)
            .map_err(|reason| Error {
                source: walker.failure_source.unwrap_or(source),
                reason,
            })?;
    }
    Ok(Report {
        points: walker.points,
        exit: state.facts,
    })
}
struct ExitContract<'a> {
    conditions: &'a [Expr],
    binding: Option<LocalId>,
    ty: &'a Term,
}
struct CallMemoryPlan {
    writes: Vec<(nir::program::Place, Term)>,
    returned: Option<Vec<nir::program::Place>>,
}

struct Walker<'a> {
    addressed_locals: BTreeSet<LocalId>,
    type_context: crate::ptc::Context,
    // Type bindings are separate from symbolic program-state frames.
    input_frames: Vec<Vec<nir::Sort>>,
    next_frame: u64,
    memory_summaries:
        std::collections::HashMap<iir::InstanceKey, Option<std::sync::Arc<super::memory::Summary>>>,
    exit_contract: Option<ExitContract<'a>>,
    assuming_postcondition: bool,
    next_field: u32,
    limits: &'a mut Limits,
    package: Option<&'a nir::program::Package>,
    points: Vec<Point>,
    failure_source: Option<SourceRange>,
    closure_context: Vec<SourceRange>,
    single_pass_exits:BTreeMap<LocalId,Vec<State>>,
}
impl Walker<'_> {
    fn unknown_value(&mut self, ty: &Term) -> Result<Value, Reason> {
        self.unknown_value_at(ty, 0)
    }

    fn unknown_value_at(&mut self, ty: &Term, depth: usize) -> Result<Value, Reason> {
        self.limits.step(1)?;
        if depth >= 128 { return Err(Reason::DepthLimit); }
        if let Some(fields) = self.tuple_field_types(ty)? {
            return Ok(Value {
                aggregate_kind: Some(AggregateKind::Tuple),
                fields: Some(fields.iter().map(|ty| self.unknown_value_at(ty, depth + 1))
                    .collect::<Result<_, _>>()?),
                ..Value::default()
            });
        }
        if let Term::Record(fields) = ty {
            return Ok(Value {
                nominal_type: Some(std::sync::Arc::new(ty.clone())),
                aggregate_kind: Some(AggregateKind::AnonymousRecord),
                fields: Some(fields.iter().map(|(_, ty)| self.unknown_value_at(ty, depth + 1))
                    .collect::<Result<_, _>>()?),
                ..Value::default()
            });
        }
        let ty = self.scalar_type_view(ty)?;
        Ok(Value::unknown(&ty, self.limits)?)
    }

    fn tuple_field_types(&mut self, ty: &Term) -> Result<Option<Vec<Term>>, Reason> {
        if !matches!(ty, Term::Tuple(_) | Term::NormalForm { .. } | Term::Bound { .. }
            | Term::Rigid { .. } | Term::AssociatedType { .. } | Term::Hole { .. }) {
            return Ok(None);
        }
        crate::associated::charge_projection_copy(ty, &mut self.limits.arithmetic.steps)
            .map_err(|_| Reason::Unsupported)?;
        if let Term::Tuple(fields) = ty { return Ok(Some(fields.clone())); }
        let Some(package) = self.package else { return Ok(None); };
        let is_tuple = |id| package.normal_forms.iter().any(|head|
            head.declaration == id && head.role == nir::normal_forms::Role::Tuple);
        if let Term::NormalForm { head, .. } = ty {
            if let Some(fields) = package.normal_forms.iter().find(|metadata| metadata.declaration == *head)
                .and_then(|metadata| metadata.tuple_elements(ty)) {
                return Ok(Some(fields.to_vec()));
            }
        }
        let (view, used) = self.type_context.structural_view_with_heads_metered(
            ty, crate::ptc::Budget { steps: self.limits.arithmetic.steps.min(4096), ..Default::default() }, is_tuple,
        );
        self.limits.step(used)?;
        let Some((view, _)) = view.ok().flatten() else { return Ok(None); };
        crate::associated::charge_projection_copy(&view, &mut self.limits.arithmetic.steps)
            .map_err(|_| Reason::Unsupported)?;
        if let Term::Tuple(fields) = view { return Ok(Some(fields)); }
        let Term::NormalForm { head, .. } = &view else { return Ok(None); };
        let Some(fields) = package.normal_forms.iter().find(|metadata| metadata.declaration == *head)
            .and_then(|metadata| metadata.tuple_elements(&view)) else { return Ok(None); };
        Ok(Some(fields.to_vec()))
    }

    fn scalar_type_view(&mut self, ty: &Term) -> Result<Term, Reason> {
        if !matches!(ty,
            Term::Bound {..} | Term::Rigid {..} | Term::NormalForm {..}
            | Term::AssociatedType {..} | Term::Hole {..}) {
            return Ok(ty.clone());
        }
        self.limits.step(4096)?;
        Ok(self.type_context.structural_view(ty, crate::ptc::Budget {steps:4096,..Default::default()})
            .ok().flatten().map(|(view,_)| view)
            .filter(|view| matches!(view, Term::Primitive(_)))
            .unwrap_or_else(|| ty.clone()))
    }
    fn external_read_path(&mut self, pointer: &Expr) -> Result<Option<FieldPath>, Reason> {
        Ok(external_read_path(pointer, self.limits)?)
    }
    fn aliased_read_path(&mut self, pointer: &Expr, state: &State) -> Result<Option<FieldPath>, Reason> {
        let Some(mut path) = self.external_read_path(pointer)? else { return Ok(None); };
        if let Some((original, source_version, alias_version)) = state.read_aliases.get(&path.root) {
            self.limits.step(original.fields.len().saturating_add(path.fields.len()).saturating_add(1))?;
            if state.facts.value(path.root) == Some(*alias_version)
                && state.facts.value(original.root) == Some(*source_version) {
                let mut fields = original.fields.clone();
                fields.extend(path.fields);
                path = FieldPath { root: original.root, fields };
            }
        }
        Ok(Some(path))
    }
    fn observe_external_read(&mut self, pointer: &Expr, ty: &Term, state: &mut State) -> Result<Value, Reason> {
        let Some(address) = self.aliased_read_path(pointer, state)? else {return self.unknown_value(ty);};
        let view = self.scalar_type_view(ty)?;
        if !matches!(view, Term::Primitive(p) if p == nir::Primitive::Bool || p.fixed_integer_layout().is_some()) {
            return self.unknown_value(ty);
        }
        self.limits.step(1)?;
        if let Some((previous, root)) = state.reads.get(&address) {
            if previous == &view {return self.local_value_at(state, *root);}
        }
        let root = self.result_root(&view)?.ok_or(Reason::Unsupported)?;
        let value = self.unknown_value(&view)?;
        bind_value_place(state, root, &view, value, self.limits)?;
        state.reads.insert(address, (view, root));
        self.local_value_at(state, root)
    }

    fn read_pointer_place(
        &mut self,
        state: &mut State,
        target: &nir::program::Place,
    ) -> Result<Value, Reason> {
        let mut current = self.local_value(state, target.root)?;
        for (declaration, index) in &target.fields {
            let ty = current
                .nominal_type
                .clone()
                .ok_or(Reason::InvalidSignature)?;
            current = match declaration {
                nir::program::FieldOwner::Nominal(id) => self.read_record_field(current, &ty, *id, *index, state)?,
                nir::program::FieldOwner::Anonymous => self.read_anonymous_field(current, &ty, *index, state)?,
            };
        }
        Ok(current)
    }
    fn fresh_frame(&mut self) -> Result<u64, Unknown> {
        let frame = self.next_frame;
        self.next_frame = frame.checked_add(1).ok_or(Unknown::VersionExhausted)?;
        Ok(frame)
    }
    fn call_preserves_memory(
        &mut self,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
        bindings: &[LocalId],
    ) -> bool {
        if !self.readonly_call(callee) {
            return false;
        }
        self.defaults_preserve_memory(callee, arguments, bindings, false)
    }
    // Input observations also require the prepared parameter bindings to retain
    // their identities. A memory-readonly default may still rebind a parameter.
    fn defaults_preserve_memory(
        &mut self,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
        bindings: &[LocalId],
        stable_bindings: bool,
    ) -> bool {
        if !arguments.iter().any(|argument| argument.is_default) {
            return true;
        }
        let Some(package) = self.package else {
            return false;
        };
        let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
            return false;
        };
        if super::memory::validate_argument_plan(arguments, bindings, parameters.len()).is_err() {
            return false;
        }
        let allowance = (self.limits.arithmetic.steps / 4).min(20_000);
        let mut fuel = allowance;
        let result = (|| {
            let mut prepared = Vec::new();
            for argument in arguments {
                fuel = fuel.checked_sub(1)?;
                if argument.is_default {
                    if stable_bindings {
                        let mut stable = true;
                        iir::visit::expression(&argument.value, &mut fuel, |expr, fuel| {
                            match &expr.kind {
                                K::Assign {place,..} | K::BitAssign {place,..} | K::BitUpdate {place,..} => {
                                    *fuel = fuel.checked_sub(bindings.len()).ok_or("default frame budget exhausted")?;
                                    if bindings.contains(&place.root) {stable = false;}
                                }
                                K::AddressOf(_) => stable = false,
                                _ => {}
                            }
                            Ok(())
                        }).ok()?;
                        if !stable {return None;}
                    }
                    let summary = super::memory::infer_argument(
                        package,
                        &argument.value,
                        &prepared,
                        &mut fuel,
                    )
                    .ok()?;
                    if summary.accesses.iter().any(|access| access.write) {
                        return None;
                    }
                }
                super::memory::bound_type(&argument.value.ty, &mut fuel).ok()?;
                prepared.push((
                    *bindings.get(argument.parameter)?,
                    argument.value.ty.clone(),
                    argument.parameter,
                ));
            }
            Some(())
        })();
        self.limits.arithmetic.steps -= allowance - fuel;
        result.is_some()
    }

    /// Only completed fixed-point summaries enter the local cache.
    fn call_summary(&mut self, callee: &Expr) -> Option<std::sync::Arc<super::memory::Summary>> {
        let package = self.package?;
        let (declaration, arguments) = match &callee.kind {
            K::Function(declaration) => (*declaration, &[][..]),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.as_slice()),
            _ => return None,
        };
        let allowance = (self.limits.arithmetic.steps / 4).min(20_000);
        let mut fuel = allowance;
        let result = (|| {
            super::memory::bound_type(&callee.ty, &mut fuel).ok()?;
            for argument in arguments {
                super::memory::bound_type(argument, &mut fuel).ok()?;
            }
            let key = iir::InstanceKey {
                declaration,
                arguments: arguments.to_vec(),
            };
            let summary = if let Some(cached) = self.memory_summaries.get(&key) {
                cached.clone()
            } else {
                let summary =
                    super::memory::infer_in_package(package, declaration, arguments, &mut fuel)
                        .ok()
                        .map(std::sync::Arc::new);
                self.memory_summaries.insert(key, summary.clone());
                summary
            }?;
            if summary.signature != *callee.ty.callable_signature() {
                return None;
            }
            Some(summary)
        })();
        self.limits.arithmetic.steps -= allowance - fuel;
        result
    }

    fn readonly_call(&mut self, callee: &Expr) -> bool {
        let Some(summary) = self.call_summary(callee) else {
            return false;
        };
        self.limits.step(summary.accesses.len()).is_ok()
            && summary.accesses.iter().all(|access| !access.write)
    }

    /// Map the complete write set before changing caller state. Any opaque
    /// destination rejects selective invalidation for the entire call.
    fn call_memory_plan(
        &mut self,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
        bindings: &[LocalId],
        evaluated: Option<&BTreeMap<usize, Option<Vec<nir::program::Place>>>>,
    ) -> Option<CallMemoryPlan> {
        type Targets = BTreeMap<usize, Option<Vec<nir::program::Place>>>;
        fn rebase(
            target: &super::memory::ParameterPlace,
            targets: &Targets,
            fuel: &mut usize,
        ) -> Option<Vec<nir::program::Place>> {
            // This analysis has no stored-pointer provenance. Indirect writes
            // must invalidate conservatively rather than masquerade as fields.
            if !target.through.is_empty() { return None; }
            let bases = targets.get(&target.parameter)?.as_ref()?;
            let mut resolved = Vec::new();
            for base in bases {
                *fuel = fuel.checked_sub(
                    base.fields
                        .len()
                        .saturating_add(target.fields.len())
                        .saturating_add(1),
                )?;
                let mut place = base.clone();
                place.fields.extend_from_slice(&target.fields);
                resolved.push(place);
            }
            Some(resolved)
        }
        fn collect(
            accesses: &[super::memory::Access],
            targets: &Targets,
            fuel: &mut usize,
            writes: &mut Vec<(nir::program::Place, Term)>,
        ) -> Option<()> {
            for access in accesses {
                *fuel = fuel.checked_sub(1)?;
                if access.write {
                    for target in rebase(&access.target, targets, fuel)? {
                        super::memory::bound_type(&access.ty, fuel).ok()?;
                        writes.push((target, access.ty.clone()));
                    }
                }
            }
            Some(())
        }
        let summary = self.call_summary(callee)?;
        let package = self.package?;
        let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
            return None;
        };
        super::memory::validate_argument_plan(arguments, bindings, parameters.len()).ok()?;
        let allowance = (self.limits.arithmetic.steps / 4).min(20_000);
        let mut fuel = allowance;
        let result = (|| {
            let mut writes = Vec::new();
            let mut targets = Targets::new();
            let mut prepared = Vec::new();
            for argument in arguments {
                fuel = fuel.checked_sub(1)?;
                let actual = if argument.is_default {
                    // ArgumentSummary describes reads/writes and the result,
                    // not rebinding of the prepared frame for later defaults.
                    let mut stable_frame = true;
                    iir::visit::expression(&argument.value, &mut fuel, |expression, fuel| {
                        match &expression.kind {
                            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate {place,..} => {
                                *fuel = fuel
                                    .checked_sub(prepared.len())
                                    .ok_or("default frame budget exhausted")?;
                                if prepared
                                    .iter()
                                    .any(|(binding, _, _)| *binding == place.root)
                                {
                                    stable_frame = false;
                                }
                            }
                            K::AddressOf(_) => stable_frame = false,
                            _ => {}
                        }
                        Ok(())
                    })
                    .ok()?;
                    if !stable_frame {
                        return None;
                    }
                    let default = super::memory::infer_argument(
                        package,
                        &argument.value,
                        &prepared,
                        &mut fuel,
                    )
                    .ok()?;
                    collect(&default.accesses, &targets, &mut fuel, &mut writes)?;
                    match default.result_targets {
                        Some(relative) => {
                            let mut actual = Vec::new();
                            for target in relative {
                                actual.extend(rebase(&target, &targets, &mut fuel)?);
                            }
                            Some(actual)
                        }
                        None => None,
                    }
                } else if let Some(evaluated) = evaluated {
                    let target = evaluated.get(&argument.parameter)?;
                    if let Some(targets) = target {
                        for target in targets {
                            fuel = fuel.checked_sub(target.fields.len().saturating_add(1))?;
                        }
                        Some(targets.clone())
                    } else {
                        None
                    }
                } else {
                    direct_pointer_place(&argument.value, &mut fuel)
                        .ok()?
                        .map(|target| vec![target])
                };
                targets.insert(argument.parameter, actual);
                if let Some(binding) = bindings.get(argument.parameter) {
                    super::memory::bound_type(&argument.value.ty, &mut fuel).ok()?;
                    prepared.push((*binding, argument.value.ty.clone(), argument.parameter));
                }
            }
            collect(&summary.accesses, &targets, &mut fuel, &mut writes)?;
            let returned = (|| {
                let mut result = Vec::new();
                for relative in summary.result_targets.as_ref()? {
                    for target in rebase(relative, &targets, &mut fuel)? {
                        insert_pointer_target(&mut result, target, &mut fuel).ok()?;
                    }
                }
                if result.is_empty() {
                    None
                } else {
                    Some(result)
                }
            })();
            Some(CallMemoryPlan { writes, returned })
        })();
        self.limits.arithmetic.steps -= allowance - fuel;
        result
    }

    fn invariant_pointer_targets(
        &mut self,
        expr: &Expr,
        state: &State,
        changed: &BTreeSet<(LocalId, Vec<(nir::program::FieldOwner, usize)>)>,
        depth: usize,
    ) -> Result<Option<Vec<nir::program::Place>>, Unknown> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Ok(None);
        }
        match &expr.kind {
            K::AddressOf(_) => Ok(
                direct_pointer_place(expr, &mut self.limits.arithmetic.steps)
                    .map_err(|_| Unknown::from(comptime_runtime::Error::BudgetExhausted))?
                    .map(|target| vec![target]),
            ),
            K::PointerField {
                pointer,
                declaration,
                index,
            } => {
                let Some(targets) =
                    self.invariant_pointer_targets(pointer, state, changed, depth + 1)?
                else {
                    return Ok(None);
                };
                let mut projected = Vec::new();
                for mut target in targets {
                    self.limits.step(1)?;
                    target.fields.push((nir::program::FieldOwner::Nominal(*declaration), *index));
                    insert_pointer_target(
                        &mut projected,
                        target,
                        &mut self.limits.arithmetic.steps,
                    )?;
                }
                Ok(Some(projected))
            }
            K::PointerAnonymousField { pointer, index, .. } => {
                let Some(targets) =
                    self.invariant_pointer_targets(pointer, state, changed, depth + 1)?
                else {
                    return Ok(None);
                };
                let mut projected = Vec::new();
                for mut target in targets {
                    self.limits.step(1)?;
                    target.fields.push((nir::program::FieldOwner::Anonymous, *index));
                    insert_pointer_target(
                        &mut projected,
                        target,
                        &mut self.limits.arithmetic.steps,
                    )?;
                }
                Ok(Some(projected))
            }
            K::Local(_) | K::AnonymousField { .. } | K::Field { .. } => {
                let Some(source) = syntactic_place(expr, &mut self.limits.arithmetic.steps, false)
                    .map_err(|_| Unknown::from(comptime_runtime::Error::BudgetExhausted))?
                else {
                    return Ok(None);
                };
                for (root, fields) in changed {
                    self.limits.step(
                        fields
                            .len()
                            .saturating_add(source.fields.len())
                            .saturating_add(1),
                    )?;
                    if *root == source.root
                        && (fields.starts_with(&source.fields) || source.fields.starts_with(fields))
                    {
                        return Ok(None);
                    }
                }
                let Some(mut value) = state.aggregates.get(&place(source.root)) else {
                    return Ok(None);
                };
                for (declaration, index) in source.fields {
                    self.limits.step(1)?;
                    if value.aggregate_kind != Some(field_owner_kind(declaration)) {
                        return Ok(None);
                    }
                    let Some(field) = value.fields.as_ref().and_then(|fields| fields.get(index))
                    else {
                        return Ok(None);
                    };
                    value = field;
                }
                Ok(value
                    .copy_pointer_target(self.limits)?
                    .filter(|(frame, _)| *frame == state.frame)
                    .map(|(_, targets)| targets))
            }
            K::Block {
                result: Some(result),
                ..
            } => {
                // The enclosing footprint traversal separately observes every
                // statement effect; this query only describes result origins.
                self.invariant_pointer_targets(result, state, changed, depth + 1)
            }
            K::If {
                then_branch,
                else_branch: Some(other),
                ..
            } => {
                let mut targets = Vec::new();
                for branch in [then_branch.as_ref(), other.as_ref()] {
                    let Some(origins) =
                        self.invariant_pointer_targets(branch, state, changed, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    for target in origins {
                        insert_pointer_target(
                            &mut targets,
                            target,
                            &mut self.limits.arithmetic.steps,
                        )?;
                    }
                }
                Ok(Some(targets))
            }
            K::Match { arms, .. } => {
                let mut targets = Vec::new();
                for arm in arms {
                    let Some(origins) =
                        self.invariant_pointer_targets(&arm.body, state, changed, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    for target in origins {
                        insert_pointer_target(
                            &mut targets,
                            target,
                            &mut self.limits.arithmetic.steps,
                        )?;
                    }
                }
                Ok(if targets.is_empty() {
                    None
                } else {
                    Some(targets)
                })
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } => {
                let mut origins = BTreeMap::new();
                for argument in arguments.iter().filter(|argument| !argument.is_default) {
                    let targets =
                        self.invariant_pointer_targets(&argument.value, state, changed, depth + 1)?;
                    origins.insert(argument.parameter, targets);
                }
                Ok(self
                    .call_memory_plan(callee, arguments, parameter_bindings, Some(&origins))
                    .and_then(|plan| plan.returned))
            }
            _ => Ok(None),
        }
    }

    fn forget_loop_writes(&mut self, body: &Expr, state: &mut State) -> Result<(), Reason> {
        let mut changed = BTreeSet::new();
        let mut invalidates_external_reads = false;
        // A later alias write can invalidate an origin used earlier in the
        // scan. Repeat the monotonically growing footprint until stable.
        loop {
            let previous = changed.len();
            let mut unknown_write = false;
            let mut fuel = self.limits.arithmetic.steps;
            let scanned = iir::visit::expression(body, &mut fuel, |expression, fuel| {
                self.limits.arithmetic.steps = *fuel;
                let scanned = (|| -> Result<(), String> {
                    match &expression.kind {
                        K::EffectHandle {..} | K::EffectRequest {..} | K::EffectRun {..} => unknown_write = true,
                        K::Assign { place: target, .. } | K::BitAssign { place: target, .. } | K::BitUpdate {place: target,..} => {
                            self.limits
                                .step(target.fields.len())
                                .map_err(|_| "loop path budget exhausted")?;
                            invalidates_external_reads |= self.addressed_locals.contains(&target.root);
                            if state.domains.contains_key(&place(target.root)) {
                                changed.insert((target.root, target.fields.clone()));
                            }
                        }
                        K::Call {
                            callee,
                            arguments,
                            parameter_bindings,
                        } => {
                            if !self.call_preserves_memory(callee, arguments, parameter_bindings) {
                                invalidates_external_reads = true;
                                let mut origins = BTreeMap::new();
                                for argument in
                                    arguments.iter().filter(|argument| !argument.is_default)
                                {
                                    let targets = self
                                        .invariant_pointer_targets(
                                            &argument.value,
                                            state,
                                            &changed,
                                            0,
                                        )
                                        .map_err(|_| "loop origin budget exhausted")?;
                                    origins.insert(argument.parameter, targets);
                                }
                                if let Some(plan) = self.call_memory_plan(
                                    callee,
                                    arguments,
                                    parameter_bindings,
                                    Some(&origins),
                                ) {
                                    for (target, _) in plan.writes {
                                        if state.domains.contains_key(&place(target.root)) {
                                            changed.insert((target.root, target.fields));
                                        }
                                    }
                                } else {
                                    unknown_write = true;
                                }
                            }
                        }
                        K::WritePointer { pointer, .. } | K::BitWritePointer {pointer,..} => {
                            invalidates_external_reads = true;
                            if let Some(targets) = self
                                .invariant_pointer_targets(pointer, state, &changed, 0)
                                .map_err(|_| "loop origin budget exhausted")?
                            {
                                for target in targets {
                                    if state.domains.contains_key(&place(target.root)) {
                                        changed.insert((target.root, target.fields));
                                    }
                                }
                            } else {
                                unknown_write = true;
                            }
                        }
                        _ => {}
                    }
                    Ok(())
                })();
                *fuel = self.limits.arithmetic.steps;
                scanned
            });
            self.limits.arithmetic.steps = fuel;
            scanned
                .map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
            if unknown_write {
                state.havoc(self.limits)?;
                return Ok(());
            }
            if changed.len() == previous {
                break;
            }
        }
        if invalidates_external_reads {
            state.forget_reads(self.limits)?;
        } else {
            // Memory is unchanged, but a reassigned pointer name no longer
            // denotes its entry address, including at the loop header.
            self.limits.step(changed.len().saturating_add(state.reads.len()))?;
            let roots: BTreeSet<_> = changed.iter().map(|(binding, _)| place(*binding)).collect();
            state.reads.retain(|path, _| !roots.contains(&path.root));
        }
        for (binding, fields) in changed {
            let root = place(binding);
            if state.frozen.contains_key(&root) {
                continue;
            }
            if fields.is_empty() {
                state.invalidate_paths(Some(root), &[], self.limits)?;
                if let Some(value) = state.aggregates.get_mut(&root) {
                    if !value.forget_observations(self.limits)? {
                        state.aggregates.remove(&root);
                    }
                }
                match state.domains.get(&root).and_then(Option::as_ref) {
                    Some(domain) => {
                        state.facts.assign(root, domain.copy(self.limits)?)?;
                    }
                    None => {
                        state.facts.invalidate(root)?;
                    }
                }
            } else {
                let mut ty = state
                    .aggregates
                    .get(&root)
                    .and_then(|value| value.nominal_type.as_ref())
                    .ok_or(Reason::Unsupported)?
                    .as_ref()
                    .clone();
                for (declaration, index) in &fields {
                    ty = self
                        .place_fields(&ty, *declaration)?
                        .get(*index)
                        .cloned()
                        .ok_or(Reason::InvalidSignature)?;
                }
                let unknown = self.unknown_value(&ty)?;
                self.write_field(
                    state,
                    &nir::program::Place {
                        root: binding,
                        fields,
                    },
                    unknown,
                )?;
            }
        }
        Ok(())
    }

    // Tail alternatives are distinct completion paths. Check their guarantees
    // before joining facts, so branch-dependent contracts are not erased by a
    // lossy interval/relational join.
    fn tail(&mut self, expr: &Expr, state: &mut State, depth: usize) -> Result<(), Reason> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        if !state.facts.is_reachable() {
            return Ok(());
        }
        match &expr.kind {
            K::EffectComputation {body,..} => self.tail(body,state,depth+1)?,
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                for statement in statements {
                    self.child(statement, state, depth + 1)?;
                }
                if let Some(result) = result {
                    self.tail(result, state, depth + 1)?;
                } else {
                    self.check_exit(Value::default(), state, depth + 1)?;
                }
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition = self.child(condition, state, depth + 1)?.condition;
                let mut yes = state.fork(self.limits)?;
                let mut no = state.fork(self.limits)?;
                if let Some(condition) = condition {
                    yes.facts.assume(&condition, true, self.limits)?;
                    no.facts.assume(&condition, false, self.limits)?;
                }
                self.tail(then_branch, &mut yes, depth + 1)?;
                if let Some(otherwise) = else_branch {
                    self.tail(otherwise, &mut no, depth + 1)?;
                } else {
                    self.check_exit(Value::default(), &mut no, depth + 1)?;
                }
                *state = yes.merge(no, self.limits)?;
            }
            K::Match { value, arms } => {
                self.match_expression(value, arms, &expr.ty, state, depth + 1, true)?;
            }
            _ => {
                let value = self.child(expr, state, depth + 1)?;
                self.check_exit(value, state, depth + 1)?;
            }
        }
        Ok(())
    }
    fn check_exit(&mut self, value: Value, state: &mut State, depth: usize) -> Result<(), Reason> {
        if !state.facts.is_reachable() {
            return Ok(());
        }
        let Some((conditions, binding, ty)) = self
            .exit_contract
            .as_ref()
            .map(|c| (c.conditions, c.binding, c.ty))
        else {
            return Ok(());
        };
        if conditions.is_empty() {
            return Ok(());
        }
        if let Some(binding) = binding {
            self.bind_related_value(state, binding, ty, value)?;
        }
        for predicate in conditions {
            self.failure_source = Some(predicate.source);
            let checked = self.child(predicate, state, depth)?;
            let condition = checked.condition.ok_or(Reason::MissingPostconditionProof)?;
            let proof = state
                .facts
                .prove_condition(&condition, true, self.limits)
                .map_err(|_| Reason::MissingPostconditionProof)?;
            proof
                .replay(&state.facts, self.limits)
                .map_err(|_| Reason::MissingPostconditionProof)?;
            state.facts.assume(&condition, true, self.limits)?;
            self.record(predicate, state, PointKind::Postcondition, None)?;
        }
        self.failure_source = None;
        Ok(())
    }
    fn check_machine(
        &mut self,
        expr: &Expr,
        state: &State,
        range: Option<&Interval>,
    ) -> Result<(), Reason> {
        if !state.facts.is_reachable() || self.assuming_postcondition {
            return Ok(());
        }
        if let Some(primitive) = nir::program::machine_integer_type(expr) {
            let (signed, width) = primitive.fixed_integer_layout().unwrap();
            let target = Interval::machine(width as u16, signed, self.limits)?;
            if !range
                .map(|range| range.within(&target, self.limits))
                .transpose()?
                .unwrap_or(false)
            {
                return Err(Reason::MissingMachineProof);
            }
        }
        Ok(())
    }

    fn eval(&mut self, expr: &Expr, state: &mut State, depth: usize) -> Result<Value, Error> {
        let result = self.child(expr, state, depth);
        result.map_err(|reason| Error {
            source: self.failure_source.unwrap_or(expr.source),
            reason,
        })
    }
    fn child(&mut self, expr: &Expr, state: &mut State, depth: usize) -> Result<Value, Reason> {
        self.inner(expr, state, depth).map_err(|reason| {
            self.failure_source.get_or_insert(expr.source);
            reason
        })
    }
    fn record(
        &mut self,
        expr: &Expr,
        state: &State,
        kind: PointKind,
        range: Option<&Interval>,
    ) -> Result<(), Unknown> {
        self.limits.step(1)?;
        self.points.push(Point {
            source: expr.source,
            kind,
            closure_context: self.closure_context.clone(),
            range: range.map(|b| b.copy(self.limits)).transpose()?,
            facts: state.facts.fork(self.limits)?,
            check_input: None,
            aggregate_snapshots: copy_aggregates(&state.aggregates, self.limits)?,
            field_paths: paths::copy_paths(&state.paths, self.limits)?,
        });
        Ok(())
    }
    fn inner(&mut self, expr: &Expr, state: &mut State, depth: usize) -> Result<Value, Reason> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        if !state.facts.is_reachable() {
            return Ok(Value::default());
        }
        let depth = depth + 1;
        let mut result = match &expr.kind {
            K::Literal(_) | K::Local(_) | K::Unit | K::TypeValue(_) | K::TypeListValue(_) | K::Function(_) | K::FunctionInstance {..}
            | K::TraitMethod {..} | K::Block {..} | K::CallPreparation {..} | K::Let {..} => self.binding_expression(expr,state,depth)?,
            K::IntegerBinary {..} | K::IntegerNegate(_) => self.integer_expression(expr,state,depth)?,
            K::Not(_) | K::And(..) | K::Or(..)
            | K::ScalarEqual(..) | K::ScalarLess(..) | K::ScalarLessEqual(..)
            | K::ScalarGreater(..) | K::ScalarGreaterEqual(..) => self.logical_expression(expr,state,depth)?,
            _=>self.general_expression(expr,state,depth)?,
        };
        let invalidates_external_reads = match &expr.kind {
            K::Assign {place, ..} | K::BitAssign {place, ..} | K::BitUpdate {place, ..} => {
                self.limits.step(1)?;
                self.addressed_locals.contains(&place.root)
            }
            K::BitWritePointer {..} | K::EffectRequest {..} | K::EffectRun {..} | K::EffectHandle {..} => true,
            _ => false,
        };
        if invalidates_external_reads { state.forget_reads(self.limits)?; }
        if expr.ty == Term::Primitive(nir::Primitive::Bool) && result.condition.is_none() {
            if let Some(Scalar::Local(value)) = result.scalar {
                result.condition = Some(Condition::Compare {
                    value,
                    relation: C::Equal,
                    constant: Integer::from_u128(1),
                });
            }
        }
        // Branch joins and operand effects can retire identities referenced by
        // an evaluated Boolean. Its value range survives, but the old relation
        // must not be assumed against the new memory state.
        if let Some(condition) = &result.condition {
            if !condition.is_current(&state.facts, self.limits, 0)? {
                result.condition = None;
            }
        }
        if let Some(condition) = &result.condition {
            let truth = match state.facts.known_condition(condition, self.limits)? {
                Some(truth) => Some(truth),
                None => match condition.truth_by_enclosure(&state.facts, self.limits, 0) {
                    Ok(truth) => truth,
                    Err(Unknown::StaleValue(_) | Unknown::MissingBound(_)) => None,
                    Err(reason) => return Err(reason.into()),
                },
            };
            if let Some(truth) = truth {
                result.range = Some(Interval::singleton(&Integer::from_u128(u128::from(truth)), self.limits)?);
                result.condition = Some(Condition::Bool(truth));
            }
        }
        Ok(result)
    }
    #[inline(never)]
    fn call_expression(&mut self,expr:&Expr,callee:&Expr,arguments:&[nir::program::CallArgument],
        parameter_bindings:&[nir::program::LocalId],state:&mut State,depth:usize)->Result<Value,Reason> {
        let mut result;
                self.child(callee, state, depth)?;
                let method_summary = if let K::TraitMethod {
                    method_arguments,
                    interface,
                    interface_arguments,
                    member,
                    subject,
                } = &callee.kind
                {
                    let definition = self
                        .package
                        .and_then(|p| p.traits.iter().find(|t| t.declaration == *interface))
                        .ok_or(Reason::InvalidSignature)?;
                    let method = definition
                        .methods
                        .iter()
                        .find(|m| m.declaration == *member)
                        .ok_or(Reason::InvalidSignature)?;
                    let parameters: Vec<_> = definition.parameters_with_self().into_iter()
                        .chain(method.type_parameters.iter().copied()).collect();
                    let arguments: Vec<_> = std::iter::once(subject.clone())
                        .chain(interface_arguments.clone())
                        .chain(method_arguments.clone())
                        .collect();
                    Some(
                        method
                            .program_summary(&parameters, &arguments)
                            .map_err(|_| Reason::InvalidSignature)?,
                    )
                } else {
                    None
                };
                let target = match &callee.kind {
                    K::Function(id)
                    | K::FunctionInstance {
                        declaration: id, ..
                    } => self
                        .package
                        .and_then(|p| p.functions.iter().chain(&p.quote_templates).find(|f| f.declaration == *id)),
                    K::TraitMethod { .. } => method_summary.as_ref(),
                    _ => None,
                };
                let mut preconditions = Vec::new();
                let mut postconditions = Vec::new();
                let result_binding = target.and_then(|f| f.result_binding);
                let snapshots = target
                    .map(|f| f.entry_snapshots.as_slice())
                    .unwrap_or_default();
                let needs_arguments = target
                    .is_some_and(|f| !f.preconditions.is_empty() || !f.entry_snapshots.is_empty());
                if let Some(target) = target {
                    if needs_arguments && parameter_bindings != &target.parameters {
                        return Err(Reason::InvalidSignature);
                    }
                    for condition in &target.preconditions {
                        let condition = if let K::FunctionInstance { arguments, .. } = &callee.kind
                        {
                            if arguments.iter().any(|ty| ty.check_scope(&[]).is_err()) {
                                condition.instantiate_types_in_scope(
                                    &target.type_parameters, arguments, &self.input_frames,
                                )
                            } else {
                                condition.instantiate_types(&target.type_parameters, arguments)
                            }.map_err(|_| Reason::InvalidSignature)?
                        } else {
                            condition.clone()
                        };
                        preconditions.push(condition);
                    }
                    for condition in &target.postconditions {
                        let condition = if let K::FunctionInstance { arguments, .. } = &callee.kind
                        {
                            if arguments.iter().any(|ty| ty.check_scope(&[]).is_err()) {
                                condition.instantiate_types_in_scope(
                                    &target.type_parameters, arguments, &self.input_frames,
                                )
                            } else {
                                condition.instantiate_types(&target.type_parameters, arguments)
                            }.map_err(|_| Reason::InvalidSignature)?
                        } else {
                            condition.clone()
                        };
                        postconditions.push(condition);
                    }
                }
                let mut transfers = Vec::new();
                let mut pointer_postconditions = false;
                for predicate in &postconditions {
                    iir::visit::expression(predicate, &mut self.limits.arithmetic.steps, |expr, _| {
                        pointer_postconditions |= matches!(expr.kind, K::ReadPointer(_));
                        Ok(())
                    }).map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
                }
                let mut stable_postcondition_parameters = pointer_postconditions;
                if stable_postcondition_parameters {
                    if let Some(target) = target {
                        iir::visit::expression(&target.body, &mut self.limits.arithmetic.steps, |expr, fuel| {
                            *fuel = fuel.checked_sub(target.parameters.len()).ok_or("parameter stability budget exhausted")?;
                            match &expr.kind {
                                K::Assign {place,..} | K::BitAssign {place,..} | K::BitUpdate {place,..}
                                    if target.parameters.contains(&place.root) => stable_postcondition_parameters = false,
                                K::AddressOf(_) => stable_postcondition_parameters = false,
                                _ => {}
                            }
                            Ok(())
                        }).map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
                    } else {stable_postcondition_parameters = false;}
                }
                let readonly_postconditions = stable_postcondition_parameters
                    && self.call_preserves_memory(callee, arguments, parameter_bindings);
                let mut evaluated_targets = BTreeMap::new();
                let mut observed_inputs = BTreeMap::new();
                let mut evaluated_addresses = BTreeMap::new();
                let mut defaults = State {
                    frame: self.fresh_frame()?,
                    facts: Facts::default(),
                    domains: BTreeMap::new(),
                    aggregates: BTreeMap::new(),
                    paths: BTreeMap::new(),
                    frozen: BTreeMap::new(),
                    reads: BTreeMap::new(), read_aliases: BTreeMap::new(),
                };
                if !postconditions.is_empty() {
                    transfers = self.copy_frozen_frame(state, &mut defaults)?;
                }
                self.limits.step(arguments.len())?;
                let has_defaults = arguments.iter().any(|argument| argument.is_default);
                let stable_defaults = !has_defaults || ((!preconditions.is_empty() || stable_postcondition_parameters || !state.reads.is_empty())
                    && self.defaults_preserve_memory(callee, arguments, parameter_bindings, true));
                for argument in arguments {
                    if !state.facts.is_reachable() {
                        break;
                    }
                    // Argument evaluation alternates caller/default frames;
                    // neither frame's cached reads can survive effects in the other.
                    defaults.forget_reads(self.limits)?;
                    if argument.is_default && !stable_defaults {state.forget_reads(self.limits)?;}
                    let mut value = if argument.is_default {
                        self.child(&argument.value, &mut defaults, depth)?
                    } else {
                        self.child(&argument.value, state, depth)?
                    };
                    {
                        let target = value
                            .copy_pointer_target(self.limits)?
                            .filter(|(frame, _)| *frame == state.frame)
                            .map(|(_, target)| target);
                        evaluated_targets.insert(argument.parameter, target);
                        if !argument.is_default {
                            if let Some(address) = self.external_read_path(&argument.value)? {
                                if stable_postcondition_parameters {
                                    if let Some(version) = state.facts.value(address.root) {
                                        self.limits.step(address.fields.len().saturating_add(1))?;
                                        evaluated_addresses.insert(argument.parameter, (address.clone(), version));
                                    }
                                }
                                let mut observations = Vec::new();
                                for (path, observation) in &state.reads {
                                    self.limits.step(path.fields.len().saturating_add(1))?;
                                    if path.root == address.root && path.fields.starts_with(&address.fields) {
                                        observations.push((path.fields[address.fields.len()..].to_vec(), path.clone(), observation.clone()));
                                    }
                                }
                                observed_inputs.insert(argument.parameter, observations);
                            }
                        } else if stable_defaults {
                            if let K::Local(address) = argument.value.kind {
                                self.limits.step(parameter_bindings.len())?;
                                if let Some(index) = parameter_bindings.iter().position(|binding| *binding == address) {
                                    if let Some((path, version)) = evaluated_addresses.get(&index) {
                                        self.limits.step(path.fields.len().saturating_add(1))?;
                                        evaluated_addresses.insert(argument.parameter, (path.clone(), *version));
                                    }
                                    if let Some(observations) = observed_inputs.get(&index) {
                                        for (fields, path, _) in observations {
                                            self.limits.step(fields.len().saturating_add(path.fields.len()).saturating_add(1))?;
                                        }
                                        observed_inputs.insert(argument.parameter, observations.clone());
                                    }
                                }
                            }
                        }
                    }
                    if needs_arguments
                        && argument.value.ty == Term::Primitive(nir::Primitive::Bool)
                        && value.range.is_none()
                    {
                        if let Some(condition) = &value.condition {
                            let source = if argument.is_default {
                                &defaults.facts
                            } else {
                                &state.facts
                            };
                            if let Some(truth) =
                                condition.truth_by_enclosure(source, self.limits, 0)?
                            {
                                let bit = Integer::from_u128(u128::from(truth));
                                value.range =
                                    Some(Interval::new(bit.clone(), bit.clone(), self.limits)?);
                                value.scalar = Some(Scalar::Constant(bit));
                            }
                        }
                    }
                    let transfer = if !needs_arguments {
                        None
                    } else {
                        self.prepare_copy(
                            &value,
                            if argument.is_default {
                                &defaults
                            } else {
                                state
                            },
                        )?
                    };
                    if let Some(binding) = parameter_bindings.get(argument.parameter) {
                        bind_value(
                            &mut defaults,
                            *binding,
                            &argument.value.ty,
                            value,
                            self.limits,
                        )?;
                        if let Some(transfer) = transfer {
                            transfers.extend(self.install_copy(
                                &mut defaults,
                                *binding,
                                transfer,
                            )?);
                            self.finish_copy_batch(&mut defaults, &transfers)?;
                        }
                    } else if has_defaults || needs_arguments {
                        return Err(Reason::InvalidSignature);
                    }
                }
                if state.facts.is_reachable() && stable_defaults && (!preconditions.is_empty() || readonly_postconditions) {
                    // Import pointee observations only after every actual argument
                    // has run. Captured targets preserve address evaluation order;
                    // unknown-pointer evidence must still be current in the caller.
                    let mut reads = Vec::new();
                    for predicate in preconditions.iter().chain(postconditions.iter().filter(|_| readonly_postconditions)) {
                        iir::visit::expression(predicate, &mut self.limits.arithmetic.steps, |expr, _| {
                            if let K::ReadPointer(pointer) = &expr.kind {reads.push((pointer.as_ref(), &expr.ty));}
                            Ok(())
                        }).map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
                    }
                    for (pointer, ty) in reads {
                        let Some(path) = self.external_read_path(pointer)? else {continue;};
                        self.limits.step(parameter_bindings.len())?;
                        let Some(parameter) = parameter_bindings.iter().position(|binding| place(*binding) == path.root) else {continue;};
                        if defaults.reads.contains_key(&path) {continue;}
                        let view = self.scalar_type_view(ty)?;
                        if !matches!(view, Term::Primitive(p) if p == nir::Primitive::Bool || p.fixed_integer_layout().is_some()) {continue;}
                        let observed = if let Some(Some(targets)) = evaluated_targets.get(&parameter) {
                            let mut joined: Option<Value> = None;
                            for target in targets {
                                self.limits.step(target.fields.len().saturating_add(path.fields.len()).saturating_add(1))?;
                                let mut target = target.clone();
                                for (owner, index) in &path.fields {
                                    let owner = match owner {
                                        AggregateKind::Record(id) => nir::program::FieldOwner::Nominal(*id),
                                        AggregateKind::AnonymousRecord => nir::program::FieldOwner::Anonymous,
                                        _ => return Err(Reason::InvalidSignature),
                                    };
                                    target.fields.push((owner,*index));
                                }
                                let value = self.read_pointer_place(state, &target)?;
                                joined = Some(match joined {Some(old)=>old.join(&value,self.limits,0)?,None=>value});
                            }
                            joined
                        } else if let Some(observations) = observed_inputs.get(&parameter) {
                            let mut value = None;
                            for (fields, address, observation) in observations {
                                self.limits.step(fields.len().saturating_add(1))?;
                                if fields == &path.fields && observation.0 == view && state.reads.get(address) == Some(observation) {
                                    value = Some(self.local_value_at(state, observation.1)?);
                                    break;
                                }
                            }
                            value
                        } else {None};
                        let Some(value) = observed else {continue;};
                        let root = self.result_root(&view)?.ok_or(Reason::Unsupported)?;
                        let transfer = self.prepare_copy(&value,state)?;
                        bind_value_place(&mut defaults,root,&view,value,self.limits)?;
                        if let Some(transfer) = transfer {
                            transfers.extend(self.install_copy_at(&mut defaults,root,&[],transfer)?);
                        }
                        defaults.reads.insert(path,(view,root));
                    }
                    self.finish_copy_batch(&mut defaults,&transfers)?;
                }
                if state.facts.is_reachable() {
                    for predicate in &preconditions {
                        let checked = self.child(predicate, &mut defaults, depth)?;
                        let condition =
                            checked.condition.ok_or(Reason::MissingPreconditionProof)?;
                        let proof = defaults
                            .facts
                            .prove_condition(&condition, true, self.limits)
                            .map_err(|_| Reason::MissingPreconditionProof)?;
                        proof
                            .replay(&defaults.facts, self.limits)
                            .map_err(|_| Reason::MissingPreconditionProof)?;
                        defaults.facts.assume(&condition, true, self.limits)?;
                    }
                }
                for snapshot in snapshots {
                    let index = parameter_bindings
                        .iter()
                        .position(|p| *p == snapshot.parameter)
                        .ok_or(Reason::InvalidSignature)?;
                    let argument = arguments
                        .iter()
                        .find(|a| a.parameter == index)
                        .ok_or(Reason::InvalidSignature)?;
                    self.materialize_entry_fields(&mut defaults, snapshot.parameter)?;
                    let value = self.local_value(&defaults, snapshot.parameter)?;
                    self.bind_related_value(
                        &mut defaults,
                        snapshot.binding,
                        &argument.value.ty,
                        value,
                    )?;
                    defaults
                        .frozen
                        .insert(place(snapshot.binding), argument.value.ty.clone());
                }
                // Recheck after all argument evaluation, before call havoc retires
                // scalar facts. Only a complete scalar-write footprint guarantees
                // that the call cannot replace these pointer-valued bindings.
                let mut stable_addresses = BTreeMap::new();
                if stable_postcondition_parameters && stable_defaults
                    && self.call_preserves_pointer_bindings(callee)? {
                    for (parameter, (path, version)) in evaluated_addresses {
                        self.limits.step(path.fields.len().saturating_add(1))?;
                        if state.facts.value(path.root) == Some(version) {
                            stable_addresses.insert(parameter, path);
                        }
                    }
                }
                // Include selected defaults and map both writes and returned
                // aliases using argument values captured before later effects.
                let mut returned_pointer = None;
                if state.facts.is_reachable() {
                    if let Some(plan) = self.call_memory_plan(
                        callee,
                        arguments,
                        parameter_bindings,
                        Some(&evaluated_targets),
                    ) {
                        returned_pointer = plan.returned;
                        // Known destinations do not imply disjointness from an
                        // external pointer. Any write invalidates its cached
                        // reads; a proved empty write set preserves them.
                        if !plan.writes.is_empty() {
                            state.forget_reads(self.limits)?;
                        }
                        for (target, ty) in plan.writes {
                            let unknown = self.unknown_value(&ty)?;
                            if target.is_root() {
                                bind_value(state, target.root, &ty, unknown, self.limits)?;
                            } else {
                                self.write_field(state, &target, unknown)?;
                            }
                        }
                    } else if !self.call_preserves_memory(callee, arguments, parameter_bindings) {
                        // Even an opaque callee cannot replace a scalar local
                        // whose address is never exposed. Pointer identities,
                        // pointee reads and addressed storage are still havoced;
                        // this is a storage fact, not a purity assumption.
                        state.havoc_with_private_scalars(Some(&self.addressed_locals), self.limits)?;
                    }
                }
                let delivered=if let Term::Computation {result,..}=&expr.ty {result.as_ref()} else {&expr.ty};
                let result_ty = self.scalar_type_view(delivered)?;
                let memory_result = if stable_postcondition_parameters && stable_defaults && state.facts.is_reachable() {
                    self.apply_memory_postconditions(state, &defaults, &postconditions, snapshots,
                        parameter_bindings, &evaluated_targets, &stable_addresses,
                        if readonly_postconditions {None} else {result_binding}, &result_ty,
                        // Include ordinary argument copies, not only frozen
                        // entry ghosts. Export validates their caller versions
                        // after all argument evaluation and memory invalidation.
                        &transfers, depth)?
                } else {None};
                let has_memory_result = memory_result.is_some();
                result = match memory_result {Some(value) => value, None => self.unknown_value(&result_ty)?};
                if matches!(expr.ty, Term::Pointer(_)) {
                    result.pointer_target = returned_pointer.map(|target| (state.frame, target));
                }
                if let Some(binding) = result_binding.filter(|_| !has_memory_result) {
                    if state.facts.is_reachable()
                        && !postconditions.is_empty()
                        && let Some(root) = self.result_root(&result_ty)?
                    {
                        let mut returned = defaults;
                        bind_value(&mut returned, binding, &result_ty, result, self.limits)?;
                        for predicate in &postconditions {
                            if !exportable_postcondition(predicate, Some(binding), snapshots,
                                (readonly_postconditions && stable_defaults).then_some(&returned.reads), self.limits)? {
                                continue;
                            }
                            // These predicates were proved on every callee return.
                            // The caller observes their mathematical consequence;
                            // it does not re-execute predicate arithmetic here.
                            let old = std::mem::replace(&mut self.assuming_postcondition, true);
                            let checked = self.child(predicate, &mut returned, depth);
                            self.assuming_postcondition = old;
                            if let Some(condition) = checked?.condition {
                                returned.facts.assume(&condition, true, self.limits)?;
                            }
                        }
                        if !returned.facts.is_reachable() {
                            state
                                .facts
                                .assume(&Condition::Bool(false), true, self.limits)?;
                            result = Value::default();
                        } else {
                            result = self.local_value(&returned, binding)?;
                            result.evidence = self.prepare_copy(&result, &returned)?;
                            self.bind_call_result(
                                state,
                                root,
                                &result_ty,
                                result,
                                &returned,
                                // Current caller versions decide which input
                                // relations survive, independently of writes to
                                // other storage or the presence of pointer posts.
                                &transfers,
                                &[],
                            )?;
                            result = self.local_value_at(state, root)?;
                        }
                    }
                }
        Ok(result)
    }
    #[inline(never)]
    fn closure_expression(&mut self,expr:&Expr,state:&mut State,depth:usize)->Result<Value,Reason> {
        let result=Value::default();
        match &expr.kind {
            K::Closure {
                parameters,
                captures,
                body,
            } => {
                let Term::Function {
                    parameters: types, ..
                } = expr.ty.callable_signature()
                else {
                    return Err(Reason::InvalidSignature);
                };
                if parameters.len() != types.len() {
                    return Err(Reason::InvalidSignature);
                }
                let mut invocation = State {
                    frame: self.fresh_frame()?,
                    facts: Facts::default(),
                    domains: BTreeMap::new(),
                    aggregates: BTreeMap::new(),
                    paths: BTreeMap::new(),
                    frozen: BTreeMap::new(),
                    reads: BTreeMap::new(), read_aliases: BTreeMap::new(),
                };
                let mutable_captures = mutable_capture_paths(body, captures, self.limits)?;
                let mut copied_relations = Vec::new();
                for capture in captures {
                    if parameters.contains(&capture.binding)
                        || invocation.domains.contains_key(&place(capture.binding))
                    {
                        return Err(Reason::InvalidSignature);
                    }
                    let captured = if let Some(value) = &capture.value {
                        self.child(value, state, depth)?
                    } else {
                        self.local_value(state, capture.binding)?
                    };
                    if !state.facts.is_reachable() {
                        return Ok(Value::default());
                    }
                    let transfer = self.prepare_copy(&captured, state)?;
                    bind_value(
                        &mut invocation,
                        capture.binding,
                        &capture.ty,
                        captured,
                        self.limits,
                    )?;
                    if let Some(transfer) = transfer {
                        copied_relations.extend(self.install_copy(
                            &mut invocation,
                            capture.binding,
                            transfer,
                        )?);
                    }
                }
                self.finish_copies(&mut invocation, copied_relations)?;
                // Publish copies first, then invalidate the written paths using
                // the same versioning as real assignments. This also removes
                // relations that depended on the old mutable field values.
                for capture in captures {
                    let Some(paths) = mutable_captures.get(&capture.binding) else {
                        continue;
                    };
                    for path in paths {
                        self.limits.step(path.len().saturating_add(1))?;
                        if path.is_empty() {
                            bind_value(
                                &mut invocation,
                                capture.binding,
                                &capture.ty,
                                self.unknown_value(&capture.ty)?,
                                self.limits,
                            )?;
                            break; // The whole capture subsumes every subpath.
                        }
                        let mut ty = capture.ty.clone();
                        for (declaration, index) in path {
                            ty = self
                                .place_fields(&ty, *declaration)?
                                .get(*index)
                                .cloned()
                                .ok_or(Reason::InvalidSignature)?;
                        }
                        let unknown = self.unknown_value(&ty)?;
                        self.write_field(
                            &mut invocation,
                            &nir::program::Place {
                                root: capture.binding,
                                fields: path.clone(),
                            },
                            unknown,
                        )?;
                    }
                }
                for (binding, parameter) in parameters.iter().zip(types) {
                    if invocation.domains.contains_key(&place(*binding)) {
                        return Err(Reason::InvalidSignature);
                    }
                    // Closure signatures introduce a callable binder; their
                    // body annotations stay in the enclosing type-input scope.
                    // Lambda parameters are runtime values, not type binders.
                    let ty = parameter.ty.substitute_partial(&[])
                        .map_err(|_| Reason::InvalidSignature)?;
                    let ty = self.scalar_type_view(&ty)?;
                    bind(&mut invocation, *binding, &ty, None, self.limits)?;
                }
                // The body is validated for a future invocation. Its returns,
                // writes, calls and checks must not change the creator's state.
                self.closure_context.push(expr.source);
                let outer_exit = self.exit_contract.take();
                let checked = self.child(body, &mut invocation, depth);
                self.exit_contract = outer_exit;
                self.closure_context.pop();
                checked?;
            }
            _=>unreachable!(),
        }
        Ok(result)
    }
    #[inline(never)]
    fn branch_expression(&mut self,expr:&Expr,state:&mut State,depth:usize)->Result<Value,Reason> {
        let mut result=Value::default();
        match &expr.kind {
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition = self.child(condition, state, depth)?.condition;
                let mut yes = state.fork(self.limits)?;
                let mut no = state.fork(self.limits)?;
                if let Some(condition) = &condition {
                    yes.facts.assume(condition, true, self.limits)?;
                    no.facts.assume(condition, false, self.limits)?;
                }
                let a = self.child(then_branch, &mut yes, depth)?;
                let b = if let Some(branch) = else_branch {
                    self.child(branch, &mut no, depth)?
                } else {
                    Value::default()
                };
                if let Some(root) = self.result_root(&expr.ty)? {
                    self.bind_result(&mut yes, root, &expr.ty, a)?;
                    self.bind_result(&mut no, root, &expr.ty, b)?;
                    *state = yes.merge(no, self.limits)?;
                    if state.facts.is_reachable() {
                        result = self.local_value_at(state, root)?;
                    }
                } else {
                    result = if !yes.facts.is_reachable() {
                        b
                    } else if !no.facts.is_reachable() {
                        a
                    } else {
                        a.join(&b, self.limits, 0)?
                    };
                    *state = yes.merge(no, self.limits)?;
                }
            }
            _=>unreachable!(),
        }
        Ok(result)
    }
    #[inline(never)]
    fn loop_expression(&mut self,identity:LocalId,body:&Expr,state:&mut State,depth:usize)->Result<(),Reason> {
        let mut tail=body;
        loop {
            self.limits.step(1)?;
            match &tail.kind {
                K::Block {result:Some(result),..} | K::CallPreparation {result:Some(result),..}=>tail=result,
                _=>break,
            }
        }
        let mut single=matches!(tail.kind,K::Break(target) if target==identity);
        if single {
            nir::visit::expression(body,&mut self.limits.arithmetic.steps,|expr,_| {
                if matches!(expr.kind,K::Continue(target) if target==identity) {single=false;}
                Ok(())
            }).map_err(|_|Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
        }
        if single {
            if self.single_pass_exits.insert(identity,vec![]).is_some() {return Err(Reason::InvalidSignature);}
            let mut iteration=state.fork(self.limits)?;
            let checked=self.child(body,&mut iteration,depth);
            let exits=self.single_pass_exits.remove(&identity).unwrap();
            checked?;
            let mut merged:Option<State>=None;
            for exit in exits {
                merged=Some(if let Some(previous)=merged {previous.merge(exit,self.limits)?} else {exit});
            }
            if let Some(exit)=merged {*state=exit;} else {state.facts.assume(&Condition::Bool(false),true,self.limits)?;}
        } else {
            self.forget_loop_writes(body,state)?;
            let mut iteration=state.fork(self.limits)?;
            self.child(body,&mut iteration,depth)?;
            self.forget_loop_writes(body,state)?;
        }
        Ok(())
    }
    #[inline(never)]
    fn binding_expression(&mut self,expr:&Expr,state:&mut State,depth:usize)->Result<Value,Reason> {
        let mut result=Value::default();
        match &expr.kind {
            K::Literal(literal) => {
                let number = match literal {
                    Literal::SignedInteger(n) => Some(Integer::from_i128(*n)),
                    Literal::UnsignedInteger(n) => Some(Integer::from_u128(*n)),
                    Literal::Integer(text) => Some(
                        Integer::parse_literal(text, &mut self.limits.arithmetic)
                            .map_err(Unknown::from)?,
                    ),
                    Literal::Bool(value) => {
                        result.condition = Some(Condition::Bool(*value));
                        let encoded = Integer::from_u128(u128::from(*value));
                        result.range = Some(Interval::new(
                            encoded.clone(),
                            encoded.clone(),
                            self.limits,
                        )?);
                        result.scalar = Some(Scalar::Constant(encoded));
                        None
                    }
                    _ => None,
                };
                if let Some(number) = number {
                    result.range = Some(Interval::singleton(&number, self.limits)?);
                    result.scalar = Some(Scalar::Constant(number));
                }
            }
            K::Local(local) => {
                result = self.local_value(state, *local)?;
            }
            K::Unit
            | K::Function(_)
            | K::FunctionInstance { .. }
            | K::TraitMethod { .. }
            | K::TypeValue(_) | K::TypeListValue(_) => {}
            K::Block { statements, result: tail }
            | K::CallPreparation { statements, result: tail } => {
                for statement in statements {
                    self.child(statement, state, depth)?;
                }
                if let Some(tail) = tail {
                    result = self.child(tail, state, depth)?;
                }
            }
            K::Let { binding, value } => {
                let value_result = self.child(value, state, depth)?;
                if state.facts.is_reachable() {
                    let alias = if matches!(value.ty, Term::Pointer(_)) {
                        self.aliased_read_path(value, state)?.and_then(|path|
                            state.facts.value(path.root).map(|version| (path, version)))
                    } else { None };
                    self.bind_related_value(state, *binding, &value.ty, value_result)?;
                    state.read_aliases.remove(&place(*binding));
                    if let Some((path, source_version)) = alias {
                        if let Some(alias_version) = state.facts.value(place(*binding)) {
                            state.read_aliases.insert(place(*binding), (path, source_version, alias_version));
                        }
                    }
                }
            }
            _=>unreachable!(),
        }
        Ok(result)
    }
    fn completed_effect_result(&mut self,mut value:Value,ty:&Term,state:&mut State)->Result<Value,Reason> {
        if !state.facts.is_reachable() {return Ok(value);}
        let ty=if let Term::Computation {result,..}=ty {result.as_ref()} else {ty};
        // The returned value is already complete. Retain its immutable evidence
        // while invalidating mutable storage reachable by the handled effects.
        value.evidence=self.prepare_copy(&value,state)?;
        state.havoc_with_private_scalars(Some(&self.addressed_locals),self.limits)?;
        if let Some(root)=self.result_root(ty)? {
            self.bind_result(state,root,ty,value)?;
            self.local_value_at(state,root)
        } else {Ok(value)}
    }
    #[inline(never)]
    fn general_expression(&mut self, expr: &Expr, state: &mut State, depth: usize) -> Result<Value, Reason> {
        let mut result = Value::default();
        match &expr.kind {
            K::BitWindow {value, start, end} => {
                self.bit_window(expr, value, start, end, state, depth)?;
            }
            K::BitIndex(value, index) => self.bit_index(expr,value,index,state,depth)?,
            K::Snapshot(value) => { result = self.child(value, state, depth)?; }
            K::BitSlice {value, ..} | K::TakeBits(value) | K::SymbolName(value) | K::EnumName(value) | K::BitConvert(value) => {
                self.child(value, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
            }
            K::BitEqual(left, right) | K::BitConcat(left, right) => {
                self.child(left, state, depth)?;
                self.child(right, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
            }
            K::EffectResume(value) => {
                self.child(value, state, depth)?;
                state.facts.assume(&Condition::Bool(false), true, self.limits)?;
            }
            K::EffectHandle {computation, handlers, ..} => {
                for handler in handlers {self.effect_handler_arms(handler, state, depth)?;}
                // Installation-time providers can mutate captured storage even
                // when the subject never requests an operation. Entry facts
                // therefore cannot flow into that subject unchanged.
                if handlers.iter().any(|handler| handler.abi == nir::effects::Abi::DirectPassing) {
                    state.havoc(self.limits)?;
                }
                let completed=self.child(computation,state,depth)?;
                result=self.completed_effect_result(completed,&expr.ty,state)?;
            }
            K::EffectComputation {body, ..} => {
                result=self.child(body,state,depth)?;
            }
            K::EffectRun {computation, ..} => {
                let completed=self.child(computation,state,depth)?;
                result=self.completed_effect_result(completed,&expr.ty,state)?;
            }
            K::EffectRequest {arguments, ..} => {
                for argument in arguments {self.child(argument,state,depth)?;}
                state.havoc_with_private_scalars(Some(&self.addressed_locals),self.limits)?;
                result = self.unknown_value(&expr.ty)?;
            }
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => {
                self.let_pattern(
                    pattern,
                    value,
                    guard.as_deref(),
                    otherwise.as_deref(),
                    state,
                    depth,
                )?;
            }
            K::Match { value, arms } => {
                result = self.match_expression(value, arms, &expr.ty, state, depth, false)?;
            }
            K::Literal(_) | K::Local(_) | K::Unit | K::TypeValue(_) | K::TypeListValue(_) | K::Function(_) | K::FunctionInstance {..}
            | K::TraitMethod {..} | K::Block {..} | K::CallPreparation {..} | K::Let {..} => unreachable!(),
            K::AddressOf(_) => {
                result.pointer_target =
                    direct_pointer_place(expr, &mut self.limits.arithmetic.steps)
                        .map_err(|_| {
                            Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into())
                        })?
                        .map(|target| (state.frame, vec![target]));
            }
            K::PointerField {
                pointer,
                declaration,
                index,
            } => {
                if let Some((frame, targets)) = self.child(pointer, state, depth)?.pointer_target {
                    let mut projected = Vec::new();
                    for mut target in targets {
                        self.limits.step(1)?;
                        target.fields.push((nir::program::FieldOwner::Nominal(*declaration), *index));
                        insert_pointer_target(
                            &mut projected,
                            target,
                            &mut self.limits.arithmetic.steps,
                        )?;
                    }
                    result.pointer_target = Some((frame, projected));
                }
            }
            K::PointerAnonymousField { pointer, index, .. } => {
                if let Some((frame, targets)) = self.child(pointer, state, depth)?.pointer_target {
                    let mut projected = Vec::new();
                    for mut target in targets {
                        self.limits.step(1)?;
                        target.fields.push((nir::program::FieldOwner::Anonymous, *index));
                        insert_pointer_target(
                            &mut projected,
                            target,
                            &mut self.limits.arithmetic.steps,
                        )?;
                    }
                    result.pointer_target = Some((frame, projected));
                }
            }
            K::ReadPointer(pointer) => {
                let targets = self
                    .child(pointer, state, depth)?
                    .pointer_target
                    .filter(|(frame, _)| *frame == state.frame)
                    .map(|(_, targets)| targets);
                result = if let Some(targets) = targets {
                    let mut joined: Option<Value> = None;
                    for target in targets {
                        let current = self.read_pointer_place(state, &target)?;
                        joined = Some(match joined {
                            Some(previous) => previous.join(&current, self.limits, 0)?,
                            None => current,
                        });
                    }
                    match joined {
                        Some(value) => value,
                        None => self.unknown_value(&expr.ty)?,
                    }
                } else {
                    self.observe_external_read(pointer, &expr.ty, state)?
                };
            }
            K::BitWritePointer {pointer,selections,value} => {
                let Term::Pointer(storage) = &pointer.ty else {return Err(Reason::Unsupported)};
                let targets = self.child(pointer,state,depth)?.pointer_target
                    .filter(|(frame,_)|*frame==state.frame).map(|(_,targets)|targets);
                self.bit_selections(expr,storage,selections,state,depth)?;
                self.child(value,state,depth)?;
                if state.facts.is_reachable() {
                    if let Some(targets) = targets {
                        for target in targets {
                            let unknown = self.unknown_value(storage)?;
                            if target.is_root() {self.bind_related_value(state,target.root,storage,unknown)?;}
                            else {self.write_field(state,&target,unknown)?;}
                        }
                    } else {state.havoc(self.limits)?;}
                }
            }
            K::WritePointer { pointer, value } => {
                let targets = self
                    .child(pointer, state, depth)?
                    .pointer_target
                    .filter(|(frame, _)| *frame == state.frame)
                    .map(|(_, targets)| targets);
                let address = self.aliased_read_path(pointer, state)?
                    .and_then(|path| state.facts.value(path.root).map(|version| (path, version)));
                let mut replacement = self.child(value, state, depth)?;
                if state.facts.is_reachable() {
                    state.forget_reads(self.limits)?;
                    if let Some(targets) = targets {
                        let multiple = targets.len() > 1;
                        let mut updates = Vec::new();
                        for target in targets {
                            let changed = if multiple {
                                // Each candidate may remain untouched. Merge
                                // old/new values, never assign the RHS to all.
                                self.read_pointer_place(state, &target)?.join(
                                    &replacement,
                                    self.limits,
                                    0,
                                )?
                            } else {
                                std::mem::take(&mut replacement)
                            };
                            updates.push((target, changed));
                        }
                        for (target, changed) in updates {
                            if target.is_root() {
                                self.bind_related_value(state, target.root, &value.ty, changed)?;
                            } else {
                                self.write_field(state, &target, changed)?;
                            }
                        }
                    } else {
                        // The address was evaluated before the RHS. Only attach
                        // the new observation to its source path if that binding
                        // still names the captured address after RHS effects.
                        let view = self.scalar_type_view(&value.ty)?;
                        let observed = if let Some((path, version)) = address {
                            if state.facts.value(path.root) == Some(version)
                                && matches!(view, Term::Primitive(p) if p == nir::Primitive::Bool || p.fixed_integer_layout().is_some()) {
                                let transfer = self.prepare_copy(&replacement, state)?;
                                Some((path, transfer))
                            } else {None}
                        } else {None};
                        state.havoc_with_private_scalars(Some(&self.addressed_locals), self.limits)?;
                        if let Some((path, transfer)) = observed {
                            let root = self.result_root(&view)?.ok_or(Reason::Unsupported)?;
                            bind_value_place(state, root, &view, replacement, self.limits)?;
                            if let Some(transfer) = transfer {
                                let copies = self.install_copy_at(state, root, &[], transfer)?;
                                self.finish_copies(state, copies)?;
                            }
                            state.reads.insert(path, (view, root));
                        }
                    }
                }
            }
            K::Assign {
                place: target,
                value,
            } => {
                let value_result = self.child(value, state, depth)?;
                if state.facts.is_reachable() {
                    if target.is_root() {
                        self.bind_related_value(state, target.root, &value.ty, value_result)?;
                    } else {
                        self.write_field(state, target, value_result)?;
                    }
                }
            }
            K::BitAssign {place: target,storage_type,value,..}
            | K::BitUpdate {place: target,storage_type,value,..} => {
                if let K::BitUpdate {selections,..} = &expr.kind {
                    self.bit_selections(expr,storage_type,selections,state,depth)?;
                }
                self.child(value, state, depth)?;
                if state.facts.is_reachable() {
                    // The slice write preserves the storage type, but not its
                    // previous numeric range or relations to other places.
                    let unknown = self.unknown_value(storage_type)?;
                    if target.is_root() {
                        bind_value(state, target.root, storage_type, unknown, self.limits)?;
                    } else {
                        self.write_field(state, target, unknown)?;
                    }
                }
            }
            K::Closure {..} => { result=self.closure_expression(expr,state,depth)?; }
            K::Discard(value) => {
                self.child(value, state, depth)?;
            }
            K::RequirePredicate(predicate) | K::EnsurePredicate(predicate) => {
                if state.facts.is_reachable() {
                    let failure=|| if matches!(expr.kind,K::RequirePredicate(_)) {
                        Reason::MissingPreconditionProof
                    } else {Reason::MissingPostconditionProof};
                    let checked=self.child(predicate,state,depth)?;
                    let condition=checked.condition.ok_or_else(&failure)?;
                    let proof=state.facts.prove_condition(&condition,true,self.limits).map_err(|_|failure())?;
                    proof.replay(&state.facts,self.limits).map_err(|_|failure())?;
                    state.facts.assume(&condition,true,self.limits)?;
                }
            }
            K::CheckPredicate(condition) => {
                let checked = self.child(condition, state, depth)?;
                let check_input = if let Some(condition) = checked.condition {
                    let facts = state.facts.fork(self.limits)?;
                    state.facts.assume(&condition, true, self.limits)?;
                    Some(CheckInput {
                        site: expr as *const Expr as usize,
                        condition,
                        facts,
                    })
                } else {
                    None
                };
                self.record(expr, state, PointKind::Check, None)?;
                self.points.last_mut().unwrap().check_input = check_input;
            }
            K::If {..} => { result=self.branch_expression(expr,state,depth)?; }
            K::Return(value) => {
                if let Some(value) = value {
                    self.tail(value, state, depth)?;
                } else {
                    self.check_exit(Value::default(), state, depth)?;
                }
                state
                    .facts
                    .assume(&Condition::Bool(false), true, self.limits)?;
            }
            K::Loop {identity,body} => { self.loop_expression(*identity,body,state,depth)?; }
            K::Break(identity) | K::Continue(identity) => {
                if matches!(expr.kind,K::Break(_)) && self.single_pass_exits.contains_key(identity) {
                    let exit=state.fork(self.limits)?;
                    self.single_pass_exits.get_mut(identity).unwrap().push(exit);
                }
                state.facts.assume(&Condition::Bool(false),true,self.limits)?;
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } => { result=self.call_expression(expr,callee,arguments,parameter_bindings,state,depth)?; }
            K::Tuple(values)
            | K::AnonymousRecord { fields: values } | K::Record { fields: values, .. }
            | K::Variant { fields: values, .. } => {
                let mut fields = Vec::new();
                for value in values {
                    let mut evaluated = self.child(value, state, depth)?;
                    evaluated.evidence = self.prepare_copy(&evaluated, state)?;
                    fields.push(evaluated.snapshot(self.limits, 0)?);
                }
                result.fields = Some(fields);
                result.nominal_type = if matches!(expr.ty, Term::Nominal { .. } | Term::Record(_)) {
                    Some(std::sync::Arc::new(expr.ty.clone()))
                } else {
                    None
                };
                result.aggregate_kind = Some(match expr.kind {
                    K::Tuple(..) => AggregateKind::Tuple,
                    K::AnonymousRecord { .. } => AggregateKind::AnonymousRecord,
                    K::Record { declaration, .. } => AggregateKind::Record(declaration),
                    K::Variant {
                        declaration, index, ..
                    } => AggregateKind::Variant(declaration, index),
                    _ => unreachable!(),
                });
                result = self.materialize_aggregate(result, &expr.ty, state)?;
            }
            K::AnonymousField { value, index, .. } => {
                let base = self.child(value, state, depth)?;
                result = self.read_anonymous_field(base, &value.ty, *index, state)?;
            }
            K::Field {
                value,
                declaration,
                index,
            } => {
                let base = self.child(value, state, depth)?;
                result = self.read_record_field(base, &value.ty, *declaration, *index, state)?;
            }
            _ => return Err(Reason::Unsupported),
        }
        Ok(result)
    }
}
impl From<Unknown> for Reason {
    fn from(value: Unknown) -> Self {
        Self::Unknown(value)
    }
}

fn compare_values(
    a: Value,
    b: Value,
    relation: C,
    limits: &mut Limits,
) -> Result<Option<Condition>, Unknown> {
    // Resolve comparisons of evaluated intervals before constructing symbolic
    // relations. Known truth needs no additional branch relation or solver work.
    if let (Some(a), Some(b)) = (&a.range, &b.range) {
        let high_low = limits.compare(a.upper(), b.lower())?;
        let low_high = limits.compare(a.lower(), b.upper())?;
        let truth = match relation {
            C::Less if high_low.is_lt() => Some(true),
            C::Less if !low_high.is_lt() => Some(false),
            C::LessEqual if !high_low.is_gt() => Some(true),
            C::LessEqual if low_high.is_gt() => Some(false),
            C::Greater if low_high.is_gt() => Some(true),
            C::Greater if !high_low.is_gt() => Some(false),
            C::GreaterEqual if !low_high.is_lt() => Some(true),
            C::GreaterEqual if high_low.is_lt() => Some(false),
            C::Equal | C::NotEqual if high_low.is_lt() || low_high.is_gt() => Some(relation == C::NotEqual),
            C::Equal | C::NotEqual if high_low.is_eq() && low_high.is_eq() => Some(relation == C::Equal),
            _ => None,
        };
        if let Some(truth) = truth {return Ok(Some(Condition::Bool(truth)));}
    }
    let polynomial = match (a.polynomial(limits)?, b.polynomial(limits)?) {
        (Some(left), Some(right)) => Some(Condition::PolynomialCompare {
            left,
            relation,
            right,
        }),
        _ => None,
    };
    Ok(match (a.scalar, b.scalar) {
        (Some(Scalar::Local(value)), Some(Scalar::Constant(constant))) => {
            Some(Condition::Compare {
                value,
                relation,
                constant,
            })
        }
        (Some(Scalar::Constant(constant)), Some(Scalar::Local(value))) => {
            Some(Condition::Compare {
                value,
                relation: match relation {
                    C::Less => C::Greater,
                    C::LessEqual => C::GreaterEqual,
                    C::Greater => C::Less,
                    C::GreaterEqual => C::LessEqual,
                    other => other,
                },
                constant,
            })
        }
        (Some(Scalar::Constant(a)), Some(Scalar::Constant(b))) => {
            let order = limits.compare(&a, &b)?;
            Some(Condition::Bool(match relation {
                C::Equal => order.is_eq(),
                C::NotEqual => !order.is_eq(),
                C::Less => order.is_lt(),
                C::LessEqual => !order.is_gt(),
                C::Greater => order.is_gt(),
                C::GreaterEqual => !order.is_lt(),
            }))
        }
        _ if polynomial.is_some() => polynomial,
        _ => None,
    })
}

/// Mandatory source gate before interpreting or delivering machine arithmetic.
/// Reuses phase traversal so nested/default operations cannot escape detection.
pub fn validate_machine_operations(package: &nir::program::Package) -> Result<(), Error> {
    let owners = nir::phases::machine_arithmetic_functions(package).map_err(|e| Error {
        source: e.source,
        reason: Reason::Phase(e.message),
    })?;
    for function in package.functions.iter().chain(&package.quote_templates) {
        if !owners.contains(&function.declaration) {
            continue;
        }
        analyze_in_package(package, function, &mut Limits::default())?;
        for (_, default) in &function.defaults {
            let mut function = function.clone();
            function.body = default.clone();
            function.defaults.clear();
            // Defaults execute before call preconditions have been discharged.
            function.preconditions.clear();
            function.postconditions.clear();
            function.result_binding = None;
            function.entry_snapshots.clear();
            analyze_in_package(package, &function, &mut Limits::default())?;
        }
    }
    Ok(())
}

// Aggregate elements snapshot their values. Numeric source versions are not
// retained by copying; pointer values retain storage origins with frame tags.
impl Value {
    fn unknown(ty: &Term, limits: &mut Limits) -> Result<Self, Unknown> {
        Self::unknown_at(ty, limits, 0)
    }
    fn unknown_at(ty: &Term, limits: &mut Limits, depth: usize) -> Result<Self, Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        Ok(Self {
            range: domain(ty, limits)?,
            nominal_type: if matches!(ty, Term::Nominal { .. } | Term::Record(_)) {
                Some(std::sync::Arc::new(ty.clone()))
            } else {
                None
            },
            aggregate_kind: if matches!(ty, Term::Tuple(_)) {
                Some(AggregateKind::Tuple)
            } else if matches!(ty, Term::Record(_)) {
                Some(AggregateKind::AnonymousRecord)
            } else {
                None
            },
            fields: if let Term::Record(fields) = ty {
                Some(fields.iter().map(|(_, ty)| Self::unknown_at(ty, limits, depth + 1)).collect::<Result<_, _>>()?)
            } else if let Term::Tuple(types) = ty {
                Some(
                    types
                        .iter()
                        .map(|t| Self::unknown_at(t, limits, depth + 1))
                        .collect::<Result<_, _>>()?,
                )
            } else {
                None
            },
            ..Self::default()
        })
    }
    fn forget_observations(&mut self, limits: &mut Limits) -> Result<bool, Unknown> {
        limits.step(1)?;
        self.pointer_target = None;
        self.range = None;
        self.scalar = None;
        self.polynomial = None;
        self.condition = None;
        self.projection = None;
        self.evidence = None;
        if self.aggregate_kind == Some(AggregateKind::Tuple) {
            if let Some(fields) = &mut self.fields {
                for field in fields {
                    field.forget_observations(limits)?;
                }
            }
            return Ok(true);
        }
        self.fields = None;
        self.aggregate_kind = None;
        Ok(self.nominal_type.is_some())
    }
    fn clear_evidence(&mut self, limits: &mut Limits) -> Result<(), Unknown> {
        limits.step(1)?;
        self.evidence = None;
        if let Some(fields) = &mut self.fields {
            for field in fields {
                field.clear_evidence(limits)?;
            }
        }
        Ok(())
    }
    fn snapshot(&self, limits: &mut Limits, depth: usize) -> Result<Self, Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        Ok(Self {
            pointer_target: self.copy_pointer_target(limits)?,
            projection: None,
            polynomial: None,
            evidence: self.evidence.clone(),
            range: self.range.as_ref().map(|r| r.copy(limits)).transpose()?,
            scalar: match &self.scalar {
                Some(Scalar::Constant(n)) => Some(Scalar::Constant(limits.integer(n)?)),
                _ => None,
            },
            condition: match self.condition {
                Some(Condition::Bool(b)) => Some(Condition::Bool(b)),
                _ => None,
            },
            aggregate_kind: self.aggregate_kind,
            nominal_type: self.nominal_type.clone(),
            fields: self
                .fields
                .as_ref()
                .map(|fields| {
                    fields
                        .iter()
                        .map(|v| v.snapshot(limits, depth + 1))
                        .collect()
                })
                .transpose()?,
        })
    }
    fn join(&self, other: &Self, limits: &mut Limits, depth: usize) -> Result<Self, Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        Ok(Self {
            pointer_target: {
                let left = self.copy_pointer_target(limits)?;
                let right = other.copy_pointer_target(limits)?;
                match (left, right) {
                    (Some((a, mut targets)), Some((b, other))) if a == b => {
                        for target in other {
                            insert_pointer_target(
                                &mut targets,
                                target,
                                &mut limits.arithmetic.steps,
                            )?;
                        }
                        Some((a, targets))
                    }
                    _ => None,
                }
            },
            projection: None,
            polynomial: None,
            evidence: None,
            range: match (&self.range, &other.range) {
                (Some(a), Some(b)) => Some(hull(a, b, limits)?),
                _ => None,
            },
            scalar: None,
            condition: match (&self.condition, &other.condition) {
                (Some(Condition::Bool(a)), Some(Condition::Bool(b))) if a == b => {
                    Some(Condition::Bool(*a))
                }
                _ => None,
            },
            nominal_type: if self.nominal_type == other.nominal_type {
                self.nominal_type.clone()
            } else {
                None
            },
            aggregate_kind: if self.aggregate_kind == other.aggregate_kind
                && self.nominal_type == other.nominal_type
            {
                self.aggregate_kind
            } else {
                None
            },
            fields: match (&self.fields, &other.fields) {
                (Some(a), Some(b))
                    if a.len() == b.len()
                        && self.aggregate_kind == other.aggregate_kind
                        && self.nominal_type == other.nominal_type =>
                {
                    Some(
                        a.iter()
                            .zip(b)
                            .map(|(a, b)| a.join(b, limits, depth + 1))
                            .collect::<Result<_, _>>()?,
                    )
                }
                _ => None,
            },
        })
    }
}
fn copy_aggregates(
    aggregates: &BTreeMap<PlaceId, Value>,
    limits: &mut Limits,
) -> Result<BTreeMap<PlaceId, Value>, Unknown> {
    let mut result = BTreeMap::new();
    for (place, value) in aggregates {
        limits.step(1)?;
        result.insert(*place, value.snapshot(limits, 0)?);
    }
    Ok(result)
}
fn bind_value(
    state: &mut State,
    local: LocalId,
    ty: &Term,
    value: Value,
    limits: &mut Limits,
) -> Result<(), Unknown> {
    bind_value_place(state, place(local), ty, value, limits)
}
fn bind_value_place(
    state: &mut State,
    root: PlaceId,
    ty: &Term,
    value: Value,
    limits: &mut Limits,
) -> Result<(), Unknown> {
    let mut snapshot = if value.fields.is_some()
        || value.aggregate_kind.is_some()
        || value.nominal_type.is_some()
        || value.pointer_target.is_some()
    {
        Some(value.snapshot(limits, 0)?)
    } else {
        None
    };
    if let Some(snapshot) = &mut snapshot {
        snapshot.clear_evidence(limits)?;
    }
    bind_place(state, root, ty, value.range, limits)?;
    if let Some(snapshot) = snapshot {
        state.aggregates.insert(root, snapshot);
    }
    Ok(())
}
impl State {
    fn join_aggregates(&mut self, other: &Self, limits: &mut Limits) -> Result<(), Unknown> {
        if !self.facts.is_reachable() {
            self.aggregates = copy_aggregates(&other.aggregates, limits)?;
            return Ok(());
        }
        if !other.facts.is_reachable() {
            return Ok(());
        }
        let mut aggregates = BTreeMap::new();
        for (place, left) in &self.aggregates {
            limits.step(1)?;
            if let Some(right) = other.aggregates.get(place) {
                aggregates.insert(*place, left.join(right, limits, 0)?);
            }
        }
        self.aggregates = aggregates;
        Ok(())
    }
}
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(range-value (pointer-target ")?;
        match &self.pointer_target {
            Some((frame, targets)) => {
                write!(f, "(frame {frame}) (targets")?;
                for target in targets {
                    write!(f, " {target}")?;
                }
                f.write_str(")")?;
            }
            None => f.write_str("unknown")?,
        }
        f.write_str(") (range ")?;
        match &self.range {
            Some(r) => r.fmt(f)?,
            None => f.write_str("unknown")?,
        }
        f.write_str(") (origin ")?;
        match &self.scalar {
            Some(Scalar::Local(id)) => id.fmt(f)?,
            Some(Scalar::Constant(n)) => n.fmt(f)?,
            None => f.write_str("unknown")?,
        }
        f.write_str(") (condition ")?;
        match &self.condition {
            Some(q) => q.fmt(f)?,
            None => f.write_str("unknown")?,
        }
        f.write_str(") (aggregate-kind ")?;
        match &self.aggregate_kind {
            Some(kind) => kind.fmt(f)?,
            None => f.write_str("unknown")?,
        }
        f.write_str(") (nominal-type ")?;
        match &self.nominal_type {
            Some(ty) => ty.fmt(f)?,
            None => f.write_str("unknown")?,
        }
        f.write_str(") (polynomial ")?;
        match &self.polynomial {
            Some(p) => p.fmt(f)?,
            None => f.write_str("unknown")?,
        };
        f.write_str(") (projection ")?;
        match &self.projection {
            Some(p) => write!(f, "{p}")?,
            None => f.write_str("none")?,
        };
        f.write_str(") (fields")?;
        match &self.fields {
            Some(fields) => {
                for field in fields {
                    write!(f, " {field}")?;
                }
            }
            None => f.write_str(" unknown")?,
        }
        f.write_str("))")
    }
}

impl Value {
    fn copy_pointer_target(
        &self,
        limits: &mut Limits,
    ) -> Result<Option<(u64, Vec<nir::program::Place>)>, Unknown> {
        if let Some((_, targets)) = &self.pointer_target {
            for target in targets {
                limits.step(target.fields.len().saturating_add(1))?;
            }
        }
        Ok(self.pointer_target.clone())
    }
    fn polynomial(&self, limits: &mut Limits) -> Result<Option<Polynomial>, Unknown> {
        if let Some(p) = &self.polynomial {
            return Ok(Some(p.copy(limits)?));
        }
        match &self.scalar {
            Some(Scalar::Local(id)) if self.range.is_some() => {
                Polynomial::variable(*id, limits).map(Some)
            }
            Some(Scalar::Constant(n)) => Polynomial::constant(limits.integer(n)?, limits).map(Some),
            _ => Ok(None),
        }
    }
}
fn combine_polynomials(
    a: &Value,
    b: &Value,
    op: IntegerOp,
    limits: &mut Limits,
) -> Result<Option<Polynomial>, Unknown> {
    let (Some(a), Some(b)) = (a.polynomial(limits)?, b.polynomial(limits)?) else {
        return Ok(None);
    };
    match match op {
        IntegerOp::Add => a.add(&b, limits),
        IntegerOp::Subtract => a.subtract(&b, limits),
        IntegerOp::Multiply => a.multiply(&b, limits),
        _ => return Ok(None),
    } {
        Ok(p) => Ok(Some(p)),
        Err(Unknown::DegreeLimit | Unknown::TermLimit) => Ok(None),
        Err(e) => Err(e),
    }
}
impl Walker<'_> {
    fn refine_polynomial(&mut self, value: &mut Value, state: &State) -> Result<(), Reason> {
        if !state.facts.is_reachable() {
            return Ok(());
        }
        let Some(p) = &value.polynomial else {
            return Ok(());
        };
        let range = match p.enclosure(&state.facts, self.limits) {
            Ok((range, _)) => range,
            Err(
                Unknown::StaleValue(_)
                | Unknown::MissingBound(_)
                | Unknown::TermLimit
                | Unknown::InsufficientBounds,
            ) => {
                value.polynomial = None;
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        value.range = Some(match value.range.take() {
            None => range,
            Some(old) => Interval::new(
                if self.limits.compare(&old.lower, &range.lower)?.is_gt() {
                    old.lower
                } else {
                    range.lower
                },
                if self.limits.compare(&old.upper, &range.upper)?.is_lt() {
                    old.upper
                } else {
                    range.upper
                },
                self.limits,
            )?,
        });
        Ok(())
    }
}

// Export result and explicit entry-snapshot guarantees. Bare parameter names
// denote exit-state bindings and cannot stand for original caller arguments.
fn external_read_path(mut pointer: &Expr, limits: &mut Limits) -> Result<Option<FieldPath>, Unknown> {
    let mut fields = Vec::new();
    loop {
        limits.step(1)?;
        match &pointer.kind {
            K::Local(address) => {
                fields.reverse();
                return Ok(Some(FieldPath {root: place(*address), fields: record_path(&fields)}));
            }
            K::PointerField {pointer: base, declaration, index} => {
                fields.push((nir::program::FieldOwner::Nominal(*declaration), *index));
                pointer = base;
            }
            K::PointerAnonymousField {pointer: base, index, ..} => {
                fields.push((nir::program::FieldOwner::Anonymous, *index));
                pointer = base;
            }
            _ => return Ok(None),
        }
    }
}

fn exportable_postcondition(
    expr: &Expr,
    binding: Option<LocalId>,
    snapshots: &[nir::program::EntrySnapshot],
    reads: Option<&BTreeMap<FieldPath, (Term, PlaceId)>>,
    limits: &mut Limits,
) -> Result<bool, Unknown> {
    let mut work = vec![expr];
    while let Some(expr) = work.pop() {
        limits.step(1)?;
        match &expr.kind {
            K::ReadPointer(pointer) => {
                let Some(path) = external_read_path(pointer, limits)? else {return Ok(false);};
                if !reads.is_some_and(|reads| reads.contains_key(&path)) {return Ok(false);}
            }
            K::Local(id) if Some(*id) == binding || snapshots.iter().any(|s| s.binding == *id) => {}
            K::Literal(_) | K::Unit => {}
            K::Not(a)
            | K::IntegerNegate(a)
            | K::FloatNegate(a)
            | K::AnonymousField { value: a, .. } | K::Field { value: a, .. }
            | K::BitSlice { value: a, .. }
            | K::Snapshot(a) | K::TakeBits(a) | K::SymbolName(a) | K::EnumName(a)
            | K::BitConvert(a)
            | K::Discard(a) => work.push(a),
            K::IntegerBinary {
                left: a, right: b, ..
            }
            | K::FloatBinary {
                left: a, right: b, ..
            }
            | K::ScalarEqual(a, b)
            | K::ScalarLess(a, b)
            | K::ScalarLessEqual(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b)
            | K::BitEqual(a, b)
            | K::BitConcat(a, b)
            | K::And(a, b)
            | K::Or(a, b) => {
                work.push(a);
                work.push(b);
            }
            K::Tuple(fields) => work.extend(fields),
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                work.push(condition);
                work.push(then_branch);
                if let Some(otherwise) = else_branch {
                    work.push(otherwise);
                }
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                work.extend(statements);
                if let Some(result) = result {
                    work.push(result);
                }
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod interval_observation_tests {
    use super::*;

    #[test]
    fn interval_observations_agree_with_all_concrete_pairs() {
        for lo_a in -2..=2 {
            for hi_a in lo_a..=2 {
                for lo_b in -2..=2 {
                    for hi_b in lo_b..=2 {
                        // Two singletons already use the polynomial path.
                        if lo_a == hi_a && lo_b == hi_b {continue;}
                        for relation in [C::Equal,C::NotEqual,C::Less,C::LessEqual,C::Greater,C::GreaterEqual] {
                            let mut limits=Limits::default();
                            let value=|lo,hi,limits:&mut Limits| Value {
                                range:Some(Interval::new(Integer::from_i128(lo),Integer::from_i128(hi),limits).unwrap()),
                                ..Value::default()
                            };
                            let a=value(lo_a,hi_a,&mut limits);
                            let b=value(lo_b,hi_b,&mut limits);
                            let decision=compare_values(a,b,relation,&mut limits).unwrap();
                            let observations=(lo_a..=hi_a).flat_map(|a|(lo_b..=hi_b).map(move |b|match relation {
                                C::Equal=>a==b,C::NotEqual=>a!=b,C::Less=>a<b,C::LessEqual=>a<=b,
                                C::Greater=>a>b,C::GreaterEqual=>a>=b,
                            })).collect::<Vec<_>>();
                            let expected=observations.iter().all(|truth|*truth==observations[0]).then_some(observations[0]);
                            let actual=match decision {Some(Condition::Bool(truth))=>Some(truth),None=>None,_=>panic!("unexpected symbolic identity")};
                            assert_eq!(actual,expected,"[{lo_a},{hi_a}] {relation:?} [{lo_b},{hi_b}]");
                        }
                    }
                }
            }
        }
    }
}
