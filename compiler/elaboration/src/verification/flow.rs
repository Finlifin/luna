//! Ordered scalar-range analysis of the supported NIR control-flow fragment.
//! This observes checked program facts; it is not a type solver or an effect
//! verifier. Unsupported constructs return an explicit analysis limitation.
mod bit_indices;
mod copies;
mod finite_domains;
mod intrinsics;
mod memory_contracts;
mod nominal;
mod operators;
mod paths;
mod patterns;
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
    IntegerFormation(String),
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
            Reason::MissingBitWindowProof => write!(
                f,
                "{:?}",
                "BitVec slice bounds or constant width are not proven; add an earlier checked assumes or a stronger contract"
            )?,
            Reason::MissingBitIndexProof => write!(
                f,
                "{:?}",
                "BitVec index is not proven in bounds; add an earlier checked assumes or a stronger contract"
            )?,
            Reason::MissingDivisionProof => write!(
                f,
                "{:?}",
                "integer divisor is not proven nonzero; establish a positive or negative range before division"
            )?,
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
            Reason::IntegerFormation(message) => write!(
                f,
                "(integer-formation {})",
                Literal::String(message.clone())
            )?,
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
type ChangedPlaces = BTreeSet<(LocalId, Vec<(nir::program::FieldOwner, usize)>)>;
type InvariantLoads<'a> = Option<(&'a State, &'a ChangedPlaces)>;
type InputPointerPlaces = BTreeMap<(usize, Vec<usize>), Vec<nir::program::Place>>;
#[derive(Clone)]
enum PointerOrigin {
    Local(nir::program::Place),
    External(Projection),
}
impl PointerOrigin {
    fn path_len(&self) -> usize {
        match self {
            Self::Local(place) => place.fields.len(),
            Self::External(p) => p.path.fields.len(),
        }
    }
    fn current(&self, state: &State) -> bool {
        match self {
            Self::Local(_) => true,
            Self::External(p) => state.facts.value(p.path.root) == Some(p.root_version),
        }
    }
    fn project(&mut self, fields: &[(nir::program::FieldOwner, usize)]) {
        match self {
            Self::Local(place) => place.fields.extend_from_slice(fields),
            Self::External(p) => p.path.fields.extend(paths::record_path(fields)),
        }
    }
}
impl fmt::Display for PointerOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local(place) => write!(f, "(local {place})"),
            Self::External(projection) => write!(f, "(external {projection})"),
        }
    }
}
#[derive(Default)]
struct Value {
    // Mathematical Integer can carry scalar facts without a finite interval.
    unbounded_integer: bool,
    pointer_target: Option<(u64, Vec<nir::program::Place>)>,
    // Complete local/external address alternatives, not observations of their
    // contents. Reads consult the current state; copies keep frame/version guards.
    pointer_origins: Option<(u64, Vec<PointerOrigin>)>,
    evidence: Option<std::sync::Arc<copies::PreparedCopy>>,
    polynomial: Option<Polynomial>,
    projection: Option<Projection>,
    immutable_projection: Option<(u64, Projection)>,
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
    EnumTag(nir::DeclarationId),
    Optional(bool),
    OptionalTag,
}
impl fmt::Display for AggregateKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AnonymousRecord => f.write_str("(anonymous-record)"),
            Self::Tuple => f.write_str("tuple"),
            Self::Record(id) => write!(f, "(record {id})"),
            Self::Variant(id, index) => write!(f, "(variant {id} {index})"),
            Self::EnumTag(id) => write!(f, "(enum-tag {id})"),
            Self::Optional(present) => write!(f, "(optional {present})"),
            Self::OptionalTag => f.write_str("(optional-tag)"),
        }
    }
}
enum Scalar {
    Local(ValueId),
    Constant(Integer),
}
struct State {
    target: Option<nir::target::TargetData>,
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
            target: self.target,
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
        &mut self,
        addressed: Option<&BTreeSet<LocalId>>,
        limits: &mut Limits,
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
            if (domain.is_some()
                || self
                    .aggregates
                    .get(place)
                    .is_some_and(|value| value.unbounded_integer))
                && addressed.is_some_and(|addressed| {
                    u32::try_from(place.owner).ok().is_some_and(|owner| {
                        !addressed.contains(&LocalId {
                            owner,
                            index: place.local,
                        })
                    })
                })
            {
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
    immutable: &BTreeSet<LocalId>,
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
            Term::Primitive(_)
            | Term::Function { .. }
            | Term::UnsafeFunction(_)
            | Term::ComptimeFunction(_) => Ok(false),
            Term::Record(fields) => {
                for (_, ty) in fields {
                    if may_alias(ty, limits, depth + 1)? {
                        return Ok(true);
                    }
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
            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. }
                if roots.contains(&place.root) =>
            {
                *fuel = fuel
                    .checked_sub(place.fields.len())
                    .ok_or("capture path budget exhausted")?;
                changed
                    .entry(place.root)
                    .or_default()
                    .insert(place.fields.clone());
            }
            K::AddressOf(_) | K::WritePointer { .. } | K::BitWritePointer { .. } => {
                unknown_write = true
            }
            K::Call { .. }
            | K::EffectHandle { .. }
            | K::EffectRequest { .. }
            | K::EffectRun { .. }
                if aliases =>
            {
                unknown_write = true
            }
            _ => {}
        }
        Ok(())
    });
    limits.arithmetic.steps = fuel;
    result.map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
    if unknown_write {
        for root in roots {
            // A staged input proven neither written nor address-exposed by
            // the enclosing traversal cannot be changed by an opaque callee.
            // Keep direct write paths above even for inconsistent input IR.
            if !immutable.contains(&root) {
                changed.entry(root).or_default().insert(vec![]);
            }
        }
    }
    Ok(changed)
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
            (
                K::PointerAnonymousField {
                    pointer: base,
                    index,
                    ..
                },
                true,
            ) => {
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
    domain_in(ty, None, limits)
}
fn domain_in(
    ty: &Term,
    target: Option<nir::target::TargetData>,
    limits: &mut Limits,
) -> Result<Option<Interval>, Unknown> {
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
    target
        .map_or_else(
            || primitive.fixed_integer_layout(),
            |target| target.integer_layout(*primitive),
        )
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
        let root = self::place(LocalId {
            owner,
            index: place.local,
        });
        state.reads.retain(|address, _| address.root != root);
    }
    state.invalidate_paths(Some(place), &[], limits)?;
    state.aggregates.remove(&place);
    if matches!(
        ty,
        Term::Optional(_)
            | Term::Nominal { .. }
            | Term::Tuple(_)
            | Term::Primitive(nir::Primitive::Integer)
    ) {
        state.aggregates.insert(place, Value::unknown(ty, limits)?);
    }
    let domain = domain_in(ty, state.target, limits)?;
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

/// Recheck program contracts, machine arithmetic and bit endpoints after
/// staging has exposed concrete types and callable targets. The package must
/// include the residual runtime-instance overlay. Bodies without these
/// obligations retain their independent resource-verification path.
pub fn validate_residual_contracts(
    package: &nir::program::Package,
    function: &Function,
    limits: &mut Limits,
) -> Result<(), Error> {
    let mut required = !function.preconditions.is_empty() || !function.postconditions.is_empty();
    let mut invalid_target = false;
    iir::visit::expressions(function, &mut limits.arithmetic.steps, |expr, fuel| {
        required |= nir::program::machine_integer_type_in(expr, package.target).is_some()
            || matches!(
                expr.kind,
                K::RequirePredicate(_)
                    | K::EnsurePredicate(_)
                    | K::BitIndex(..)
                    | K::BitWindow { .. }
                    | K::BitUpdate { .. }
                    | K::BitWritePointer { .. }
            );
        let K::Call { callee, .. } = &expr.kind else {
            return Ok(());
        };
        let (declaration, arguments) = match &callee.kind {
            K::Function(declaration) => (*declaration, &[][..]),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.as_slice()),
            _ => return Ok(()),
        };
        *fuel = fuel
            .checked_sub(
                package
                    .runtime_instances
                    .len()
                    .saturating_add(package.functions.len())
                    .saturating_add(package.quote_templates.len()),
            )
            .ok_or("residual contract lookup budget exhausted")?;
        let Some((target, _)) = package.runtime_body(declaration, arguments) else {
            invalid_target = true;
            return Err("residual contract target is missing or ambiguous".into());
        };
        required |= !target.preconditions.is_empty();
        Ok(())
    })
    .map_err(|_| Error {
        source: function.body.source,
        reason: if invalid_target {
            Reason::InvalidSignature
        } else {
            Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into())
        },
    })?;
    if required {
        analyze_in_package(package, function, limits)?;
    }
    Ok(())
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
        target: package.and_then(|package| package.target),
        frame: 0,
        facts: Facts::default(),
        domains: BTreeMap::new(),
        aggregates: BTreeMap::new(),
        paths: BTreeMap::new(),
        frozen: BTreeMap::new(),
        reads: BTreeMap::new(),
        read_aliases: BTreeMap::new(),
    };
    // Type premises determine scalar domains; value premises below determine
    // facts within those domains. Never send program predicates to PTC.
    let mut type_context = crate::ptc::Context::with_scope(vec![
        parameters.iter().map(|parameter| parameter.sort).collect(),
    ]);
    let mut contract_budget = 4096;
    for requirement in &function.requirements {
        let formula = crate::ptc::Formula::from_predicate(requirement);
        let before = contract_budget;
        let admitted = crate::ptc::within_limits(&formula, 128, &mut contract_budget);
        limits
            .step(before - contract_budget)
            .map_err(|e| error(Reason::Unknown(e)))?;
        if !admitted {
            return Err(error(Reason::Unsupported));
        }
        type_context.assume(formula);
    }
    for (binding, ty) in function.parameters.iter().zip(parameters) {
        let mut view = None;
        if matches!(
            ty.ty,
            Term::Bound { .. }
                | Term::Rigid { .. }
                | Term::NormalForm { .. }
                | Term::AssociatedType { .. }
                | Term::Hole { .. }
        ) {
            let (resolved, used) = type_context.structural_view_metered(
                &ty.ty,
                crate::ptc::Budget {
                    steps: limits.arithmetic.steps.min(4096),
                    ..Default::default()
                },
            );
            limits.step(used).map_err(|e| error(Reason::Unknown(e)))?;
            view = resolved
                .ok()
                .flatten()
                .map(|(ty, _)| ty)
                .filter(|ty| matches!(ty, Term::Primitive(_)));
        }
        bind(
            &mut state,
            *binding,
            view.as_ref().unwrap_or(&ty.ty),
            None,
            limits,
        )
        .map_err(|e| error(Reason::Unknown(e)))?;
    }
    // Only explicit address formation can expose a by-value local to pointer
    // aliases. Scan the complete checked function, including nested closures,
    // contracts and defaults; future address-taking is conservatively included.
    let mut addressed_locals = BTreeSet::new();
    let mut written_locals = BTreeSet::new();
    iir::visit::expressions(function, &mut limits.arithmetic.steps, |expr, fuel| {
        if let K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } =
            &expr.kind
        {
            written_locals.insert(place.root);
        }
        if let K::EffectHandle { handlers, .. } = &expr.kind {
            for handler in handlers {
                for arm in &handler.arms {
                    addressed_locals.extend(patterns::handler_arm_writes(arm, fuel)?);
                }
            }
        }
        if let K::AddressOf(value) = &expr.kind {
            iir::visit::expression(value, fuel, |operand, _| {
                if let K::Local(binding) = operand.kind {
                    addressed_locals.insert(binding);
                }
                Ok(())
            })?;
        }
        Ok(())
    })
    .map_err(|_| {
        error(Reason::Unknown(
            comptime_runtime::Error::BudgetExhausted.into(),
        ))
    })?;
    // Erased inputs may be reconstructed as ordinary locals in runtime
    // instances. Preserve only inputs whose reconstruction neither changes
    // nor escapes; this does not freeze pointees or mutable runtime copies.
    if function.phase == nir::program::FunctionPhase::Runtime {
        for (binding, parameter) in function.parameters.iter().zip(parameters) {
            if parameter.sort != nir::Sort::Runtime
                && !addressed_locals.contains(binding)
                && !written_locals.contains(binding)
            {
                state.frozen.insert(place(*binding), parameter.ty.clone());
            }
        }
    }
    let mut walker = Walker {
        handler_unstable_locals: Some(addressed_locals.union(&written_locals).copied().collect()),
        addressed_locals,
        written_locals,
        type_context,
        input_frames: vec![parameters.iter().map(|p| p.sort).collect()],
        next_frame: 1,
        limits,
        package,
        points: vec![],
        failure_source: None,
        closure_context: vec![],
        single_pass_exits: BTreeMap::new(),
        next_field: 0,
        captured_values: BTreeMap::new(),
        associated_scalars: Vec::new(),
        assuming_postcondition: false,
        memory_summaries: std::collections::HashMap::new(),
        exit_contract: Some(ExitContract {
            conditions: &function.postconditions,
            binding: function.result_binding,
            ty: if let Term::Computation { result, .. } = result.as_ref() {
                result.as_ref()
            } else {
                result.as_ref()
            },
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
    // Handler arms can run at arbitrary request points, so preserve only
    // lexical scalars neither written nor address-exposed anywhere in the body.
    handler_unstable_locals: Option<BTreeSet<LocalId>>,
    written_locals: BTreeSet<LocalId>,
    type_context: crate::ptc::Context,
    // Type bindings are separate from symbolic program-state frames.
    input_frames: Vec<Vec<nir::Sort>>,
    next_frame: u64,
    memory_summaries:
        std::collections::HashMap<iir::InstanceKey, std::sync::Arc<super::memory::Summary>>,
    exit_contract: Option<ExitContract<'a>>,
    assuming_postcondition: bool,
    next_field: u32,
    captured_values: BTreeMap<nir::DeclarationId, (Term, PlaceId)>,
    associated_scalars: Vec<(
        (nir::program::InterfaceInstance, nir::DeclarationId, Term),
        Term,
        PlaceId,
    )>,
    limits: &'a mut Limits,
    package: Option<&'a nir::program::Package>,
    points: Vec<Point>,
    failure_source: Option<SourceRange>,
    closure_context: Vec<SourceRange>,
    single_pass_exits: BTreeMap<LocalId, Vec<State>>,
}
impl Walker<'_> {
    fn target(&self) -> Option<nir::target::TargetData> {
        self.package.and_then(|package| package.target)
    }
    fn integer_layout(&self, primitive: nir::Primitive) -> Option<(bool, u32)> {
        self.target().map_or_else(
            || primitive.fixed_integer_layout(),
            |target| target.integer_layout(primitive),
        )
    }

    fn unknown_value(&mut self, ty: &Term) -> Result<Value, Reason> {
        self.unknown_value_at(ty, 0)
    }

    fn unknown_value_at(&mut self, ty: &Term, depth: usize) -> Result<Value, Reason> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        if let Some(fields) = self.tuple_field_types(ty)? {
            return Ok(Value {
                nominal_type: Some(std::sync::Arc::new(ty.clone())),
                aggregate_kind: Some(AggregateKind::Tuple),
                fields: Some(
                    fields
                        .iter()
                        .map(|ty| self.unknown_value_at(ty, depth + 1))
                        .collect::<Result<_, _>>()?,
                ),
                ..Value::default()
            });
        }
        if let Term::Record(fields) = ty {
            return Ok(Value {
                nominal_type: Some(std::sync::Arc::new(ty.clone())),
                aggregate_kind: Some(AggregateKind::AnonymousRecord),
                fields: Some(
                    fields
                        .iter()
                        .map(|(_, ty)| self.unknown_value_at(ty, depth + 1))
                        .collect::<Result<_, _>>()?,
                ),
                ..Value::default()
            });
        }
        let ty = self.scalar_type_view(ty)?;
        let mut value = Value::unknown(&ty, self.limits)?;
        value.range = domain_in(&ty, self.target(), self.limits)?;
        if let (Term::Nominal { declaration, .. }, Some(package)) = (&ty, self.package) {
            self.limits.step(package.enums.len())?;
            if package.enums.iter().any(|definition| {
                definition.declaration == *declaration && definition.variants.len() == 1
            }) {
                // The declaration determines the sole tag even when its
                // payload is unknown. Leave recursive fields lazy.
                value.aggregate_kind = Some(AggregateKind::Variant(*declaration, 0));
            }
        }
        Ok(value)
    }

    fn tuple_field_types(&mut self, ty: &Term) -> Result<Option<Vec<Term>>, Reason> {
        if !matches!(
            ty,
            Term::Tuple(_)
                | Term::NormalForm { .. }
                | Term::Bound { .. }
                | Term::Rigid { .. }
                | Term::AssociatedType { .. }
                | Term::Hole { .. }
        ) {
            return Ok(None);
        }
        crate::associated::charge_projection_copy(ty, &mut self.limits.arithmetic.steps)
            .map_err(|_| Reason::Unsupported)?;
        if let Term::Tuple(fields) = ty {
            return Ok(Some(fields.clone()));
        }
        let Some(package) = self.package else {
            return Ok(None);
        };
        let is_tuple = |id| {
            package
                .normal_forms
                .iter()
                .any(|head| head.declaration == id && head.role == nir::normal_forms::Role::Tuple)
        };
        if let Term::NormalForm { head, .. } = ty {
            if let Some(fields) = package
                .normal_forms
                .iter()
                .find(|metadata| metadata.declaration == *head)
                .and_then(|metadata| metadata.tuple_elements(ty))
            {
                return Ok(Some(fields.to_vec()));
            }
        }
        let (view, used) = self.type_context.structural_view_with_heads_metered(
            ty,
            crate::ptc::Budget {
                steps: self.limits.arithmetic.steps.min(4096),
                ..Default::default()
            },
            is_tuple,
        );
        self.limits.step(used)?;
        let Some((view, _)) = view.ok().flatten() else {
            return Ok(None);
        };
        crate::associated::charge_projection_copy(&view, &mut self.limits.arithmetic.steps)
            .map_err(|_| Reason::Unsupported)?;
        if let Term::Tuple(fields) = view {
            return Ok(Some(fields));
        }
        let Term::NormalForm { head, .. } = &view else {
            return Ok(None);
        };
        let Some(fields) = package
            .normal_forms
            .iter()
            .find(|metadata| metadata.declaration == *head)
            .and_then(|metadata| metadata.tuple_elements(&view))
        else {
            return Ok(None);
        };
        Ok(Some(fields.to_vec()))
    }

    fn scalar_type_view(&mut self, ty: &Term) -> Result<Term, Reason> {
        if !matches!(
            ty,
            Term::Bound { .. }
                | Term::Rigid { .. }
                | Term::NormalForm { .. }
                | Term::AssociatedType { .. }
                | Term::Hole { .. }
        ) {
            return Ok(ty.clone());
        }
        // A query limit is not its cost. Charge the metered work, including
        // failed lookups, so repeated generic scalar observations do not
        // consume the entire allowance when no structural view is available.
        let (view, used) = self.type_context.structural_view_metered(
            ty,
            crate::ptc::Budget {
                steps: self.limits.arithmetic.steps.min(4096),
                ..Default::default()
            },
        );
        self.limits.step(used)?;
        Ok(view
            .ok()
            .flatten()
            .map(|(view, _)| view)
            .filter(|view| matches!(view, Term::Primitive(_)))
            .unwrap_or_else(|| ty.clone()))
    }
    fn external_read_path(&mut self, pointer: &Expr) -> Result<Option<FieldPath>, Reason> {
        Ok(external_read_path(pointer, self.limits)?)
    }
    fn aliased_read_path(
        &mut self,
        pointer: &Expr,
        state: &State,
    ) -> Result<Option<FieldPath>, Reason> {
        let Some(path) = self.external_read_path(pointer)? else {
            return Ok(None);
        };
        self.canonical_read_path(path, state).map(Some)
    }
    fn canonical_read_path(
        &mut self,
        mut path: FieldPath,
        state: &State,
    ) -> Result<FieldPath, Reason> {
        if let Some((original, source_version, alias_version)) = state.read_aliases.get(&path.root)
        {
            self.limits.step(
                original
                    .fields
                    .len()
                    .saturating_add(path.fields.len())
                    .saturating_add(1),
            )?;
            if state.facts.value(path.root) == Some(*alias_version)
                && state.facts.value(original.root) == Some(*source_version)
            {
                let mut fields = original.fields.clone();
                fields.extend(path.fields);
                path = FieldPath {
                    root: original.root,
                    fields,
                };
            }
        }
        Ok(path)
    }
    fn pointer_value_origins(
        &mut self,
        value: &Value,
        state: &State,
    ) -> Result<Option<Vec<PointerOrigin>>, Reason> {
        if let Some((frame, targets)) = &value.pointer_origins {
            if *frame != state.frame {
                return Ok(None);
            }
            for target in targets {
                self.limits.step(target.path_len().saturating_add(1))?;
                if !target.current(state) {
                    return Ok(None);
                }
            }
            return Ok(Some(targets.clone()));
        }
        if let Some((frame, targets)) = value.copy_pointer_target(self.limits)? {
            return Ok((frame == state.frame)
                .then(|| targets.into_iter().map(PointerOrigin::Local).collect()));
        }
        let Some(projection) = &value.projection else {
            return Ok(None);
        };
        if state.facts.value(projection.path.root) != Some(projection.root_version) {
            return Ok(None);
        }
        self.limits
            .step(projection.path.fields.len().saturating_add(1))?;
        let path = self.canonical_read_path(projection.path.clone(), state)?;
        Ok(state
            .facts
            .value(path.root)
            .map(|root_version| vec![PointerOrigin::External(Projection { root_version, path })]))
    }
    fn pointer_input_origins(
        &mut self,
        value: &Value,
        fields: &[usize],
        state: &State,
    ) -> Result<Option<Vec<PointerOrigin>>, Reason> {
        if fields.is_empty() {
            return self.pointer_value_origins(value, state);
        }
        // Formal nominal inputs may have no materialized payload yet. Obtain
        // their owned layout from declarations, without unfolding type heads.
        let mut expanded = value.snapshot(self.limits, 0)?;
        if let Some(source) = &value.projection {
            self.limits
                .step(source.path.fields.len().saturating_add(1))?;
            expanded.projection = Some(source.clone());
        }
        self.expand_owned_shape(&mut expanded, 0)?;
        let mut value = &expanded;
        let mut projection = None;
        for index in fields {
            self.limits.step(1)?;
            if let Some(source) = &value.projection {
                self.limits
                    .step(source.path.fields.len().saturating_add(1))?;
                projection = Some(source.clone());
            }
            let Some(kind) = value.aggregate_kind else {
                return Ok(None);
            };
            let Some(field) = value.fields.as_ref().and_then(|fields| fields.get(*index)) else {
                return Ok(None);
            };
            if let Some(path) = &mut projection {
                path.path.fields.push((kind, *index));
            }
            value = field;
        }
        // Explicit leaf origins override its container's structural projection.
        if value.pointer_origins.is_some() || value.projection.is_some() || fields.is_empty() {
            return self.pointer_value_origins(value, state);
        }
        self.pointer_value_origins(
            &Value {
                projection,
                ..Value::default()
            },
            state,
        )
    }
    fn pointer_value_path(
        &mut self,
        value: &Value,
        state: &State,
    ) -> Result<Option<FieldPath>, Reason> {
        Ok(self
            .pointer_value_origins(value, state)?
            .filter(|targets| targets.len() == 1)
            .and_then(|mut targets| targets.pop())
            .and_then(|target| match target {
                PointerOrigin::External(p) => Some(p.path),
                PointerOrigin::Local(_) => None,
            }))
    }

    fn observe_external_read(
        &mut self,
        pointer: &Expr,
        ty: &Term,
        state: &mut State,
    ) -> Result<Value, Reason> {
        let Some(address) = self.aliased_read_path(pointer, state)? else {
            return self.unknown_value(ty);
        };
        self.observe_external_path(address, ty, state)
    }
    fn observe_external_path(
        &mut self,
        address: FieldPath,
        ty: &Term,
        state: &mut State,
    ) -> Result<Value, Reason> {
        let view = self.scalar_type_view(ty)?;
        let pointer_slot = matches!(view, Term::Pointer(_));
        if !pointer_slot
            && !matches!(view, Term::Primitive(p) if p == nir::Primitive::Bool || self.integer_layout(p).is_some())
        {
            return self.unknown_value(ty);
        }
        self.limits.step(1)?;
        if let Some((previous, root)) = state.reads.get(&address) {
            if previous == &view {
                return self.local_value_at(state, *root);
            }
        }
        let root = if pointer_slot {
            // Each observed load gets an independent pointer identity. A later
            // store retires the read cache; reloading must allocate a new root.
            let local = self.next_field;
            self.next_field = local.checked_add(1).ok_or(Unknown::VersionExhausted)?;
            PlaceId {
                owner: u64::MAX - 1,
                local,
            }
        } else {
            self.result_root(&view)?.ok_or(Reason::Unsupported)?
        };
        let value = self.unknown_value(&view)?;
        bind_value_place(state, root, &view, value, self.limits)?;
        state.reads.insert(address, (view, root));
        self.local_value_at(state, root)
    }

    fn contract_pointer_origins(
        &mut self,
        pointer: &Expr,
        bindings: &[LocalId],
        arguments: &BTreeMap<(usize, Vec<usize>), Vec<PointerOrigin>>,
        state: &mut State,
        depth: usize,
    ) -> Result<Option<Vec<PointerOrigin>>, Reason> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        if let Some(path) = self.external_read_path(pointer)? {
            self.limits.step(bindings.len())?;
            let Some(parameter) = bindings
                .iter()
                .position(|binding| place(*binding) == path.root)
            else {
                return Ok(None);
            };
            let owned = owned_pointer_input_path(pointer, self.limits)?;
            let Some(inputs) = arguments.get(&(parameter, owned.clone())) else {
                return Ok(None);
            };
            let mut fields = Vec::new();
            for (kind, index) in &path.fields[owned.len()..] {
                let owner = match kind {
                    AggregateKind::Record(id) => nir::program::FieldOwner::Nominal(*id),
                    AggregateKind::AnonymousRecord => nir::program::FieldOwner::Anonymous,
                    _ => return Ok(None),
                };
                fields.push((owner, *index));
            }
            let mut origins = Vec::new();
            for input in inputs {
                self.limits.step(
                    input
                        .path_len()
                        .saturating_add(fields.len())
                        .saturating_add(1),
                )?;
                if !input.current(state) {
                    return Ok(None);
                }
                let mut origin = input.clone();
                origin.project(&fields);
                insert_pointer_origin(&mut origins, origin, self.limits)?;
            }
            return Ok(Some(origins));
        }
        match &pointer.kind {
            K::ReadPointer(base) => {
                let Some(inputs) =
                    self.contract_pointer_origins(base, bindings, arguments, state, depth + 1)?
                else {
                    return Ok(None);
                };
                let mut origins = Vec::new();
                for input in inputs {
                    let value = self.read_pointer_origin(input, &pointer.ty, state)?;
                    let Some(loaded) = self.pointer_value_origins(&value, state)? else {
                        return Ok(None);
                    };
                    for origin in loaded {
                        insert_pointer_origin(&mut origins, origin, self.limits)?;
                    }
                }
                Ok(Some(origins))
            }
            K::PointerField {
                pointer: base,
                declaration,
                index,
            } => {
                let Some(mut origins) =
                    self.contract_pointer_origins(base, bindings, arguments, state, depth + 1)?
                else {
                    return Ok(None);
                };
                for origin in &mut origins {
                    self.limits.step(1)?;
                    origin.project(&[(nir::program::FieldOwner::Nominal(*declaration), *index)]);
                }
                Ok(Some(origins))
            }
            K::PointerAnonymousField {
                pointer: base,
                index,
                ..
            } => {
                let Some(mut origins) =
                    self.contract_pointer_origins(base, bindings, arguments, state, depth + 1)?
                else {
                    return Ok(None);
                };
                for origin in &mut origins {
                    self.limits.step(1)?;
                    origin.project(&[(nir::program::FieldOwner::Anonymous, *index)]);
                }
                Ok(Some(origins))
            }
            _ => Ok(None),
        }
    }

    fn load_pointer_origins(
        &mut self,
        inputs: &[PointerOrigin],
        through: &[Vec<(nir::program::FieldOwner, usize)>],
        loaded_types: &[Term],
        state: &mut State,
    ) -> Result<Option<Vec<PointerOrigin>>, Reason> {
        for input in inputs {
            self.limits.step(input.path_len().saturating_add(1))?;
        }
        let mut current = inputs.to_vec();
        if loaded_types.len() != through.len() {
            return Ok(None);
        }
        for (fields, ty) in through.iter().zip(loaded_types) {
            self.limits.step(fields.len().saturating_add(1))?;
            let mut loaded = Vec::new();
            for mut origin in current {
                self.limits.step(
                    origin
                        .path_len()
                        .saturating_add(fields.len())
                        .saturating_add(1),
                )?;
                if !origin.current(state) {
                    return Ok(None);
                }
                origin.project(fields);
                // The summary supplies the actual loaded pointer type. An
                // external slot load obtains a fresh/reused observation identity,
                // never the address identity of the slot itself.
                let value = self.read_pointer_origin(origin, ty, state)?;
                let Some(targets) = self.pointer_value_origins(&value, state)? else {
                    return Ok(None);
                };
                for target in targets {
                    insert_pointer_origin(&mut loaded, target, self.limits)?;
                }
            }
            if loaded.is_empty() {
                return Ok(None);
            }
            current = loaded;
        }
        Ok(Some(current))
    }

    // Conditional payloads are resolved only from complete local snapshots.
    // An external observation or an unknown tag must not select a convenient
    // payload layout. The caller excludes writes that could replace tags or
    // pointer storage before consulting these snapshots.
    fn conditional_pointer_origins(
        &mut self,
        target: &super::memory::ParameterPlace,
        inputs: &[PointerOrigin],
        state: &State,
    ) -> Result<Option<Vec<PointerOrigin>>, Reason> {
        if target
            .conditions
            .keys()
            .any(|stage| *stage > target.through.len())
        {
            return Ok(None);
        }
        for input in inputs {
            self.limits.step(input.path_len().saturating_add(1))?;
        }
        let mut current = inputs.to_vec();
        for stage in 0..=target.through.len() {
            let mut next = Vec::new();
            for mut origin in current {
                self.limits.step(origin.path_len().saturating_add(1))?;
                if !origin.current(state) {
                    return Ok(None);
                }
                if stage == target.through.len() && !target.conditions.contains_key(&stage) {
                    self.limits.step(target.fields.len())?;
                    origin.project(&target.fields);
                    insert_pointer_origin(&mut next, origin, self.limits)?;
                    continue;
                }
                let PointerOrigin::Local(base) = &origin else {
                    return Ok(None);
                };
                let Some(root) = state.aggregates.get(&place(base.root)) else {
                    return Ok(None);
                };
                let Some(value) = snapshot_fields(root, &base.fields, self.limits)? else {
                    return Ok(None);
                };
                let mut active = true;
                if let Some(guards) = target.conditions.get(&stage) {
                    for (fields, expected) in guards {
                        self.limits.step(1)?;
                        let Some(actual) = snapshot_indices(value, fields, self.limits)? else {
                            return Ok(None);
                        };
                        match actual.aggregate_kind {
                            Some(AggregateKind::Variant(_, tag)) if tag == *expected => {}
                            Some(AggregateKind::Variant(_, _)) => {
                                active = false;
                                break;
                            }
                            _ => return Ok(None),
                        }
                    }
                }
                if !active {
                    continue;
                }
                if stage == target.through.len() {
                    self.limits.step(target.fields.len())?;
                    origin.project(&target.fields);
                    insert_pointer_origin(&mut next, origin, self.limits)?;
                } else {
                    let Some(loaded) = snapshot_fields(value, &target.through[stage], self.limits)?
                    else {
                        return Ok(None);
                    };
                    let Some(origins) = self.pointer_value_origins(loaded, state)? else {
                        return Ok(None);
                    };
                    for origin in origins {
                        insert_pointer_origin(&mut next, origin, self.limits)?;
                    }
                }
            }
            current = next;
        }
        Ok(Some(current))
    }

    fn summary_pointer_origins(
        &mut self,
        accesses: &[super::memory::Access],
        may_run_cleanup: bool,
        preserves_pointer_storage: bool,
        targets: Option<&BTreeSet<super::memory::ParameterPlace>>,
        arguments: &BTreeMap<(usize, Vec<usize>), Vec<PointerOrigin>>,
        state: &mut State,
    ) -> Result<Option<Vec<PointerOrigin>>, Reason> {
        if may_run_cleanup {
            return Ok(None);
        }
        self.limits.step(accesses.len())?;
        let readonly = accesses.iter().all(|access| !access.write);
        let Some(targets) = targets else {
            return Ok(None);
        };
        let mut result = Vec::new();
        for target in targets {
            if !target.conditions.is_empty() && !readonly && !preserves_pointer_storage {
                return Ok(None);
            }
            // Direct input addresses survive value writes. Stored-pointer
            // loads still require a readonly body: entry and exit contents
            // cannot be substituted for an unknown intermediate load.
            if !readonly && !preserves_pointer_storage && !target.through.is_empty() {
                return Ok(None);
            }

            self.limits
                .step(target.input_fields.len().saturating_add(1))?;
            let Some(inputs) = arguments.get(&(target.parameter, target.input_fields.clone()))
            else {
                return Ok(None);
            };
            if !target.conditions.is_empty() {
                let Some(origins) = self.conditional_pointer_origins(target, inputs, state)? else {
                    return Ok(None);
                };
                for origin in origins {
                    insert_pointer_origin(&mut result, origin, self.limits)?;
                }
                continue;
            }
            let mut types = Vec::new();
            for (index, fields) in target.through.iter().enumerate() {
                let mut loaded = None;
                for access in accesses {
                    let cost = access.target.through.iter().fold(
                        access
                            .target
                            .input_fields
                            .len()
                            .saturating_add(access.target.fields.len())
                            .saturating_add(1),
                        |cost, fields| cost.saturating_add(fields.len()).saturating_add(1),
                    );
                    self.limits.step(cost)?;
                    if access.read_value
                        && access.target.conditions.is_empty()
                        && access.target.parameter == target.parameter
                        && access.target.input_fields == target.input_fields
                        && access.target.through.as_slice() == &target.through[..index]
                        && access.target.fields == *fields
                        && matches!(access.ty, Term::Pointer(_))
                    {
                        super::memory::bound_type(&access.ty, &mut self.limits.arithmetic.steps)
                            .map_err(|_| {
                                Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into())
                            })?;
                        loaded = Some(access.ty.clone());
                        break;
                    }
                }
                let Some(ty) = loaded else {
                    return Ok(None);
                };
                types.push(ty);
            }
            let Some(origins) =
                self.load_pointer_origins(inputs, &target.through, &types, state)?
            else {
                return Ok(None);
            };
            for mut origin in origins {
                self.limits.step(
                    origin
                        .path_len()
                        .saturating_add(target.fields.len())
                        .saturating_add(1),
                )?;
                if !origin.current(state) {
                    return Ok(None);
                }
                origin.project(&target.fields);
                insert_pointer_origin(&mut result, origin, self.limits)?;
            }
        }
        Ok((!result.is_empty()).then_some(result))
    }

    fn indirect_scalar_write_plan(
        &mut self,
        callee: &Expr,
        inputs: &BTreeMap<(usize, Vec<usize>), Vec<PointerOrigin>>,
        state: &mut State,
    ) -> Result<Option<CallMemoryPlan>, Reason> {
        if !self.call_preserves_pointer_bindings(callee)? {
            return Ok(None);
        }
        let Some(summary) = self.call_summary(callee) else {
            return Ok(None);
        };
        if summary.may_run_cleanup {
            return Ok(None);
        }
        let mut writes = Vec::new();
        for access in &summary.accesses {
            self.limits.step(1)?;
            if !access.write {
                continue;
            }
            // Scalar-only writes cannot replace the pointer slots used by this
            // path. Resolve all destinations before invalidating caller facts.
            self.limits.step(
                access.target.through.iter().fold(
                    access
                        .target
                        .input_fields
                        .len()
                        .saturating_add(access.target.fields.len())
                        .saturating_add(1),
                    |cost, fields| cost.saturating_add(fields.len()).saturating_add(1),
                ),
            )?;
            let targets = BTreeSet::from([access.target.clone()]);
            let Some(origins) = self.summary_pointer_origins(
                &summary.accesses,
                summary.may_run_cleanup,
                true,
                Some(&targets),
                inputs,
                state,
            )?
            else {
                return Ok(None);
            };
            for origin in origins {
                let PointerOrigin::Local(target) = origin else {
                    return Ok(None);
                };
                super::memory::bound_type(&access.ty, &mut self.limits.arithmetic.steps).map_err(
                    |_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()),
                )?;
                writes.push((target, access.ty.clone()));
            }
        }
        Ok(Some(CallMemoryPlan {
            writes,
            returned: None,
        }))
    }

    fn read_pointer_origin(
        &mut self,
        origin: PointerOrigin,
        ty: &Term,
        state: &mut State,
    ) -> Result<Value, Reason> {
        match origin {
            PointerOrigin::Local(place) => self.read_pointer_place(state, &place),
            PointerOrigin::External(projection) => {
                self.observe_external_path(projection.path, ty, state)
            }
        }
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
                nir::program::FieldOwner::Nominal(id) => {
                    self.read_record_field(current, &ty, *id, *index, state)?
                }
                nir::program::FieldOwner::Anonymous => {
                    self.read_anonymous_field(current, &ty, *index, state)?
                }
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
                                K::Assign { place, .. }
                                | K::BitAssign { place, .. }
                                | K::BitUpdate { place, .. } => {
                                    *fuel = fuel
                                        .checked_sub(bindings.len())
                                        .ok_or("default frame budget exhausted")?;
                                    if bindings.contains(&place.root) {
                                        stable = false;
                                    }
                                }
                                K::AddressOf(_) => stable = false,
                                _ => {}
                            }
                            Ok(())
                        })
                        .ok()?;
                        if !stable {
                            return None;
                        }
                    }
                    let summary = super::memory::infer_argument(
                        package,
                        &argument.value,
                        &prepared,
                        &mut fuel,
                    )
                    .ok()?;
                    if summary.may_run_cleanup || summary.accesses.iter().any(|access| access.write)
                    {
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
                // A failed attempt is not a property of the instance: resource
                // exhaustion must not become permanent negative evidence.
                let summary = std::sync::Arc::new(
                    super::memory::infer_in_package(package, declaration, arguments, &mut fuel)
                        .ok()?,
                );
                self.memory_summaries.insert(key, summary.clone());
                summary
            };
            if summary.may_run_cleanup || summary.signature != *callee.ty.callable_signature() {
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
        input_fields: Option<&InputPointerPlaces>,
        invariant_loads: InvariantLoads<'_>,
        result_field: Option<&[usize]>,
    ) -> Option<CallMemoryPlan> {
        type Targets = BTreeMap<usize, Option<Vec<nir::program::Place>>>;
        fn rebase(
            target: &super::memory::ParameterPlace,
            targets: &Targets,
            input_fields: Option<&InputPointerPlaces>,
            invariant_loads: InvariantLoads<'_>,
            fuel: &mut usize,
        ) -> Option<Vec<nir::program::Place>> {
            let bases = if target.input_fields.is_empty() {
                targets.get(&target.parameter)?.as_ref()?
            } else {
                *fuel = fuel.checked_sub(target.input_fields.len().saturating_add(1))?;
                input_fields?.get(&(target.parameter, target.input_fields.clone()))?
            };
            if !target.conditions.is_empty() {
                let (state, changed) = invariant_loads?;
                return invariant_conditional_targets(target, bases, state, changed, fuel);
            }
            let mut current = Vec::new();
            for base in bases {
                *fuel = fuel.checked_sub(base.fields.len().saturating_add(1))?;
                current.push(base.clone());
            }
            for fields in &target.through {
                let (state, changed) = invariant_loads?;
                let mut loaded = Vec::new();
                for mut slot in current {
                    *fuel = fuel.checked_sub(fields.len().saturating_add(1))?;
                    slot.fields.extend_from_slice(fields);
                    for target in invariant_slot_targets(&slot, state, changed, fuel)? {
                        insert_pointer_target(&mut loaded, target, fuel).ok()?;
                    }
                }
                if loaded.is_empty() {
                    return None;
                }
                current = loaded;
            }
            let mut resolved = Vec::new();
            for base in &current {
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
            input_fields: Option<&InputPointerPlaces>,
            invariant_loads: InvariantLoads<'_>,
            fuel: &mut usize,
            writes: &mut Vec<(nir::program::Place, Term)>,
        ) -> Option<()> {
            for access in accesses {
                *fuel = fuel.checked_sub(1)?;
                if access.write {
                    for target in
                        rebase(&access.target, targets, input_fields, invariant_loads, fuel)?
                    {
                        super::memory::bound_type(&access.ty, fuel).ok()?;
                        writes.push((target, access.ty.clone()));
                    }
                }
            }
            Some(())
        }
        let summary = self.call_summary(callee)?;
        // Loading a slot across a call requires proof that the callee cannot
        // replace pointer storage. Defaults below must be readonly as well.
        let invariant_loads = if invariant_loads.is_some()
            && matches!(self.call_preserves_pointer_bindings(callee), Ok(true))
        {
            invariant_loads
        } else {
            None
        };
        let package = self.package?;
        let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
            return None;
        };
        super::memory::validate_argument_plan(arguments, bindings, parameters.len()).ok()?;
        let allowance = (self.limits.arithmetic.steps / 4).min(20_000);
        let mut fuel = allowance;
        let result = (|| {
            let mut owned_inputs = BTreeMap::new();
            if let Some(inputs) = input_fields {
                for ((parameter, fields), places) in inputs {
                    fuel = fuel.checked_sub(fields.len().saturating_add(1))?;
                    for place in places {
                        fuel = fuel.checked_sub(place.fields.len().saturating_add(1))?;
                    }
                    owned_inputs.insert((*parameter, fields.clone()), places.clone());
                }
            }
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
                            K::Assign { place, .. }
                            | K::BitAssign { place, .. }
                            | K::BitUpdate { place, .. } => {
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
                    if default.may_run_cleanup
                        || (invariant_loads.is_some() && default.accesses.iter().any(|a| a.write))
                    {
                        return None;
                    }
                    collect(
                        &default.accesses,
                        &targets,
                        Some(&owned_inputs),
                        invariant_loads,
                        &mut fuel,
                        &mut writes,
                    )?;
                    // Each prepared aggregate contributes its own pointer
                    // fields to later defaults and the callee write footprint.
                    // Resolve all sources against the earlier prepared inputs.
                    let mut fields = Vec::new();
                    for (path, relative) in &default.pointer_fields {
                        fuel = fuel.checked_sub(path.len().saturating_add(1))?;
                        let resolved = (|| {
                            let mut places = Vec::new();
                            for target in relative.as_ref()? {
                                for place in rebase(
                                    target,
                                    &targets,
                                    Some(&owned_inputs),
                                    invariant_loads,
                                    &mut fuel,
                                )? {
                                    insert_pointer_target(&mut places, place, &mut fuel).ok()?;
                                }
                            }
                            (!places.is_empty()).then_some(places)
                        })();
                        fields.push((path.clone(), resolved));
                    }
                    for (path, places) in fields {
                        let key = (argument.parameter, path);
                        owned_inputs.remove(&key);
                        if let Some(places) = places {
                            owned_inputs.insert(key, places);
                        }
                    }
                    match default.result_targets {
                        Some(relative) => {
                            let mut actual = Vec::new();
                            for target in relative {
                                actual.extend(rebase(
                                    &target,
                                    &targets,
                                    Some(&owned_inputs),
                                    invariant_loads,
                                    &mut fuel,
                                )?);
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
            collect(
                &summary.accesses,
                &targets,
                Some(&owned_inputs),
                invariant_loads,
                &mut fuel,
                &mut writes,
            )?;
            let returned = (|| {
                let mut result = Vec::new();
                let relative_targets = match result_field {
                    Some(path) => summary.pointer_fields.get(path)?.as_ref()?,
                    None => summary.result_targets.as_ref()?,
                };
                for relative in relative_targets {
                    for target in rebase(
                        relative,
                        &targets,
                        Some(&owned_inputs),
                        invariant_loads,
                        &mut fuel,
                    )? {
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
        result.or_else(|| {
            // A writing default may still have a complete direct footprint;
            // preserve that path without importing any stored-pointer loads.
            invariant_loads?;
            self.call_memory_plan(
                callee,
                arguments,
                bindings,
                evaluated,
                input_fields,
                None,
                result_field,
            )
        })
    }

    fn invariant_scalar_write_plan(
        &mut self,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
        state: &State,
        changed: &BTreeSet<(LocalId, Vec<(nir::program::FieldOwner, usize)>)>,
    ) -> Result<Option<CallMemoryPlan>, Unknown> {
        self.limits.step(arguments.len())?;
        if arguments.iter().any(|argument| argument.is_default)
            || !matches!(self.call_preserves_pointer_bindings(callee), Ok(true))
        {
            return Ok(None);
        }
        let Some(summary) = self.call_summary(callee) else {
            return Ok(None);
        };
        if summary.may_run_cleanup {
            return Ok(None);
        }
        let mut writes = Vec::new();
        for access in &summary.accesses {
            self.limits.step(1)?;
            if !access.write {
                continue;
            }
            let target = &access.target;
            self.limits.step(
                arguments
                    .len()
                    .saturating_add(target.input_fields.len())
                    .saturating_add(1),
            )?;
            let Some(argument) = arguments
                .iter()
                .find(|argument| argument.parameter == target.parameter)
            else {
                return Ok(None);
            };
            let Some(mut current) = self.invariant_input_pointer_targets(
                &argument.value,
                &target.input_fields,
                state,
                changed,
                0,
            )?
            else {
                return Ok(None);
            };
            if !target.conditions.is_empty() {
                let Some(resolved) = invariant_conditional_targets(
                    target,
                    &current,
                    state,
                    changed,
                    &mut self.limits.arithmetic.steps,
                ) else {
                    return Ok(None);
                };
                for place in resolved {
                    super::memory::bound_type(&access.ty, &mut self.limits.arithmetic.steps)
                        .map_err(|_| Unknown::from(comptime_runtime::Error::BudgetExhausted))?;
                    writes.push((place, access.ty.clone()));
                }
                continue;
            }
            for fields in &target.through {
                for place in &mut current {
                    self.limits.step(fields.len().saturating_add(1))?;
                    place.fields.extend_from_slice(fields);
                }
                let Some(loaded) = self.invariant_load_pointer_targets(current, state, changed)?
                else {
                    return Ok(None);
                };
                current = loaded;
            }
            if current.is_empty() {
                return Ok(None);
            }
            for mut place in current {
                self.limits.step(target.fields.len().saturating_add(1))?;
                place.fields.extend_from_slice(&target.fields);
                super::memory::bound_type(&access.ty, &mut self.limits.arithmetic.steps)
                    .map_err(|_| Unknown::from(comptime_runtime::Error::BudgetExhausted))?;
                writes.push((place, access.ty.clone()));
            }
        }
        Ok(Some(CallMemoryPlan {
            writes,
            returned: None,
        }))
    }

    fn invariant_input_pointer_targets(
        &mut self,
        expr: &Expr,
        input: &[usize],
        state: &State,
        changed: &BTreeSet<(LocalId, Vec<(nir::program::FieldOwner, usize)>)>,
        depth: usize,
    ) -> Result<Option<Vec<nir::program::Place>>, Unknown> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Ok(None);
        }
        let Some((index, rest)) = input.split_first() else {
            return self.invariant_pointer_targets(expr, state, changed, depth + 1);
        };
        match &expr.kind {
            K::Tuple(fields) | K::AnonymousRecord { fields } | K::Record { fields, .. } => {
                match fields.get(*index) {
                    Some(field) => {
                        self.invariant_input_pointer_targets(field, rest, state, changed, depth + 1)
                    }
                    None => Ok(None),
                }
            }
            K::Block {
                result: Some(result),
                ..
            } => self.invariant_input_pointer_targets(result, input, state, changed, depth + 1),
            K::If {
                then_branch,
                else_branch: Some(other),
                ..
            } => {
                let mut targets = Vec::new();
                for branch in [then_branch.as_ref(), other.as_ref()] {
                    let Some(origins) = self.invariant_input_pointer_targets(
                        branch,
                        input,
                        state,
                        changed,
                        depth + 1,
                    )?
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
                    let Some(origins) = self.invariant_input_pointer_targets(
                        &arm.body,
                        input,
                        state,
                        changed,
                        depth + 1,
                    )?
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
                if !matches!(self.call_preserves_pointer_bindings(callee), Ok(true)) {
                    return Ok(None);
                }
                let mut origins = BTreeMap::new();
                for argument in arguments.iter().filter(|argument| !argument.is_default) {
                    let targets =
                        self.invariant_pointer_targets(&argument.value, state, changed, depth + 1)?;
                    origins.insert(argument.parameter, targets);
                }
                let inputs = self.invariant_input_places(
                    callee,
                    arguments,
                    parameter_bindings,
                    state,
                    changed,
                    depth + 1,
                )?;
                // Use the same preparation and rebase rules as scalar pointer
                // results, but select the requested owned result field. Defaults
                // are callee-frame expressions, not caller-local bindings.
                Ok(self
                    .call_memory_plan(
                        callee,
                        arguments,
                        parameter_bindings,
                        Some(&origins),
                        Some(&inputs),
                        Some((state, changed)),
                        Some(input),
                    )
                    .and_then(|plan| plan.returned))
            }
            K::Local(_) | K::Field { .. } | K::AnonymousField { .. } => {
                let Some(mut source) =
                    syntactic_place(expr, &mut self.limits.arithmetic.steps, false)
                        .map_err(|_| Unknown::from(comptime_runtime::Error::BudgetExhausted))?
                else {
                    if let K::Field { value, index, .. } | K::AnonymousField { value, index, .. } =
                        &expr.kind
                    {
                        self.limits.step(input.len().saturating_add(1))?;
                        let mut path = vec![*index];
                        path.extend_from_slice(input);
                        return self.invariant_input_pointer_targets(
                            value,
                            &path,
                            state,
                            changed,
                            depth + 1,
                        );
                    }
                    return Ok(None);
                };
                let Some(mut value) = state.aggregates.get(&place(source.root)) else {
                    return Ok(None);
                };
                for (owner, index) in &source.fields {
                    self.limits.step(1)?;
                    if !storage_owner_matches(value, *owner) {
                        return Ok(None);
                    }
                    let Some(field) = value.fields.as_ref().and_then(|fields| fields.get(*index))
                    else {
                        return Ok(None);
                    };
                    value = field;
                }
                let mut tuple_projection = false;
                for index in input {
                    self.limits.step(1)?;
                    let owner = match value.aggregate_kind {
                        Some(AggregateKind::Record(declaration)) => {
                            Some(nir::program::FieldOwner::Nominal(declaration))
                        }
                        Some(AggregateKind::AnonymousRecord) => {
                            Some(nir::program::FieldOwner::Anonymous)
                        }
                        Some(AggregateKind::Tuple) => {
                            // Program places have no tuple-element owner. Keep
                            // the enclosing storage prefix for invalidation;
                            // do not invent a record identity for Tuple.
                            tuple_projection = true;
                            None
                        }
                        _ => return Ok(None),
                    };
                    let Some(field) = value.fields.as_ref().and_then(|fields| fields.get(*index))
                    else {
                        return Ok(None);
                    };
                    if !tuple_projection {
                        if let Some(owner) = owner {
                            source.fields.push((owner, *index));
                        }
                    }
                    value = field;
                }
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
                Ok(value
                    .copy_pointer_target(self.limits)?
                    .filter(|(frame, _)| *frame == state.frame)
                    .map(|(_, targets)| targets))
            }
            _ => Ok(None),
        }
    }

    fn invariant_input_places(
        &mut self,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
        bindings: &[LocalId],
        state: &State,
        changed: &BTreeSet<(LocalId, Vec<(nir::program::FieldOwner, usize)>)>,
        depth: usize,
    ) -> Result<InputPointerPlaces, Unknown> {
        let mut inputs = InputPointerPlaces::new();
        let Some(summary) = self.call_summary(callee) else {
            return Ok(inputs);
        };
        // Defaults can consume an owned pointer field that the callee never
        // mentions: its body sees only the prepared default result. Collect
        // dependencies from both summaries before resolving explicit inputs.
        let mut needed = BTreeSet::new();
        let mut collect = |target: &super::memory::ParameterPlace, fuel: &mut usize| {
            *fuel = fuel
                .checked_sub(target.input_fields.len().saturating_add(1))
                .ok_or_else(|| Unknown::from(comptime_runtime::Error::BudgetExhausted))?;
            if !target.input_fields.is_empty() {
                needed.insert((target.parameter, target.input_fields.clone()));
            }
            Ok::<_, Unknown>(())
        };
        for target in summary
            .accesses
            .iter()
            .map(|access| &access.target)
            .chain(summary.result_targets.iter().flatten())
            .chain(summary.pointer_fields.values().flatten().flatten())
        {
            collect(target, &mut self.limits.arithmetic.steps)?;
        }
        if arguments.iter().any(|argument| argument.is_default) {
            let mut prepared = Vec::new();
            for argument in arguments {
                self.limits.step(1)?;
                if argument.is_default
                    && let Some(package) = self.package
                {
                    let allowance = (self.limits.arithmetic.steps / 4).min(20_000);
                    let mut fuel = allowance;
                    let default = super::memory::infer_argument(
                        package,
                        &argument.value,
                        &prepared,
                        &mut fuel,
                    );
                    self.limits.arithmetic.steps -= allowance - fuel;
                    if let Ok(default) = default {
                        for target in default
                            .accesses
                            .iter()
                            .map(|access| &access.target)
                            .chain(default.result_targets.iter().flatten())
                            .chain(default.pointer_fields.values().flatten().flatten())
                        {
                            collect(target, &mut self.limits.arithmetic.steps)?;
                        }
                    }
                }
                let Some(binding) = bindings.get(argument.parameter) else {
                    return Ok(inputs);
                };
                super::memory::bound_type(&argument.value.ty, &mut self.limits.arithmetic.steps)
                    .map_err(|_| Unknown::from(comptime_runtime::Error::BudgetExhausted))?;
                prepared.push((*binding, argument.value.ty.clone(), argument.parameter));
            }
        }
        for (parameter, fields) in needed {
            self.limits.step(arguments.len().saturating_add(1))?;
            let Some(argument) = arguments
                .iter()
                .find(|argument| argument.parameter == parameter && !argument.is_default)
            else {
                continue;
            };
            if let Some(places) = self.invariant_input_pointer_targets(
                &argument.value,
                &fields,
                state,
                changed,
                depth + 1,
            )? {
                inputs.insert((parameter, fields), places);
            }
        }
        Ok(inputs)
    }

    fn invariant_place_pointer_targets(
        &mut self,
        source: &nir::program::Place,
        state: &State,
        changed: &BTreeSet<(LocalId, Vec<(nir::program::FieldOwner, usize)>)>,
    ) -> Result<Option<Vec<nir::program::Place>>, Unknown> {
        Ok(invariant_slot_targets(
            source,
            state,
            changed,
            &mut self.limits.arithmetic.steps,
        ))
    }

    fn invariant_load_pointer_targets(
        &mut self,
        slots: Vec<nir::program::Place>,
        state: &State,
        changed: &BTreeSet<(LocalId, Vec<(nir::program::FieldOwner, usize)>)>,
    ) -> Result<Option<Vec<nir::program::Place>>, Unknown> {
        let mut loaded = Vec::new();
        for slot in slots {
            self.limits.step(slot.fields.len().saturating_add(1))?;
            let Some(targets) = self.invariant_place_pointer_targets(&slot, state, changed)? else {
                return Ok(None);
            };
            for target in targets {
                insert_pointer_target(&mut loaded, target, &mut self.limits.arithmetic.steps)?;
            }
        }
        Ok((!loaded.is_empty()).then_some(loaded))
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
                    target
                        .fields
                        .push((nir::program::FieldOwner::Nominal(*declaration), *index));
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
                    target
                        .fields
                        .push((nir::program::FieldOwner::Anonymous, *index));
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
                    if let K::Field { value, index, .. } | K::AnonymousField { value, index, .. } =
                        &expr.kind
                    {
                        return self.invariant_input_pointer_targets(
                            value,
                            &[*index],
                            state,
                            changed,
                            depth + 1,
                        );
                    }
                    return Ok(None);
                };
                self.invariant_place_pointer_targets(&source, state, changed)
            }
            K::ReadPointer(pointer) => {
                let Some(slots) =
                    self.invariant_pointer_targets(pointer, state, changed, depth + 1)?
                else {
                    return Ok(None);
                };
                self.invariant_load_pointer_targets(slots, state, changed)
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
                let inputs = self.invariant_input_places(
                    callee,
                    arguments,
                    parameter_bindings,
                    state,
                    changed,
                    depth + 1,
                )?;
                if let Some(returned) = self
                    .call_memory_plan(
                        callee,
                        arguments,
                        parameter_bindings,
                        Some(&origins),
                        Some(&inputs),
                        Some((state, changed)),
                        None,
                    )
                    .and_then(|plan| plan.returned)
                {
                    return Ok(Some(returned));
                }
                let Some(summary) = self.call_summary(callee) else {
                    return Ok(None);
                };
                self.limits.step(summary.accesses.len())?;
                if summary.may_run_cleanup
                    || !matches!(self.call_preserves_pointer_bindings(callee), Ok(true))
                {
                    return Ok(None);
                }
                let Some(targets) = &summary.result_targets else {
                    return Ok(None);
                };
                let mut returned = Vec::new();
                for target in targets {
                    if !target.conditions.is_empty() {
                        return Ok(None);
                    }
                    self.limits
                        .step(target.input_fields.len().saturating_add(1))?;
                    let base = if target.input_fields.is_empty() {
                        origins
                            .get(&target.parameter)
                            .and_then(|value| value.as_ref())
                    } else {
                        inputs.get(&(target.parameter, target.input_fields.clone()))
                    };
                    let Some(base) = base else {
                        return Ok(None);
                    };
                    for place in base {
                        self.limits.step(place.fields.len().saturating_add(1))?;
                    }
                    let mut current = base.clone();
                    for fields in &target.through {
                        for place in &mut current {
                            self.limits.step(fields.len().saturating_add(1))?;
                            place.fields.extend_from_slice(fields);
                        }
                        let Some(loaded) =
                            self.invariant_load_pointer_targets(current, state, changed)?
                        else {
                            return Ok(None);
                        };
                        current = loaded;
                    }
                    for mut place in current {
                        self.limits.step(target.fields.len().saturating_add(1))?;
                        place.fields.extend_from_slice(&target.fields);
                        insert_pointer_target(
                            &mut returned,
                            place,
                            &mut self.limits.arithmetic.steps,
                        )?;
                    }
                }
                Ok((!returned.is_empty()).then_some(returned))
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
                        K::EffectHandle { .. } | K::EffectRequest { .. } | K::EffectRun { .. } => {
                            unknown_write = true
                        }
                        K::Assign { place: target, .. }
                        | K::BitAssign { place: target, .. }
                        | K::BitUpdate { place: target, .. } => {
                            self.limits
                                .step(target.fields.len())
                                .map_err(|_| "loop path budget exhausted")?;
                            invalidates_external_reads |=
                                self.addressed_locals.contains(&target.root);
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
                                let inputs = self
                                    .invariant_input_places(
                                        callee,
                                        arguments,
                                        parameter_bindings,
                                        state,
                                        &changed,
                                        0,
                                    )
                                    .map_err(|_| "loop input origin budget exhausted")?;
                                let mut plan = self.call_memory_plan(
                                    callee,
                                    arguments,
                                    parameter_bindings,
                                    Some(&origins),
                                    Some(&inputs),
                                    Some((state, &changed)),
                                    None,
                                );
                                if plan.is_none() {
                                    plan = self
                                        .invariant_scalar_write_plan(
                                            callee, arguments, state, &changed,
                                        )
                                        .map_err(|_| "loop indirect write budget exhausted")?;
                                }
                                if let Some(plan) = plan {
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
                        K::WritePointer { pointer, .. } | K::BitWritePointer { pointer, .. } => {
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
            self.limits
                .step(changed.len().saturating_add(state.reads.len()))?;
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
            K::EffectComputation { body, .. } => self.tail(body, state, depth + 1)?,
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                for statement in statements {
                    self.child(statement, state, depth + 1)?;
                }
                if let Some(result) = result {
                    self.tail(result, state, depth + 1)?;
                } else {
                    self.check_exit(Value::default(), None, state, depth + 1)?;
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
                    self.check_exit(Value::default(), None, &mut no, depth + 1)?;
                }
                *state = yes.merge(no, self.limits)?;
            }
            K::Match { value, arms } => {
                self.match_expression(value, arms, &expr.ty, state, depth + 1, true)?;
            }
            _ => {
                let value = self.child(expr, state, depth + 1)?;
                self.check_exit(value, Some(expr), state, depth + 1)?;
            }
        }
        Ok(())
    }
    fn check_exit(
        &mut self,
        value: Value,
        expression: Option<&Expr>,
        state: &mut State,
        depth: usize,
    ) -> Result<(), Reason> {
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
            // A direct returned pointer path denotes the same storage in the
            // exit contract. Keep version checks so rebinding cannot reuse an
            // obsolete memory observation through the result name.
            let alias = if matches!(ty, Term::Pointer(_)) {
                let path = if let Some(expression) = expression {
                    self.aliased_read_path(expression, state)?
                } else {
                    None
                };
                let path = match path {
                    Some(path) => Some(path),
                    None => self.pointer_value_path(&value, state)?,
                };
                path.and_then(|path| state.facts.value(path.root).map(|version| (path, version)))
            } else {
                None
            };
            self.bind_related_value(state, binding, ty, value)?;
            state.read_aliases.remove(&place(binding));
            if let Some((path, source_version)) = alias {
                if let Some(alias_version) = state.facts.value(place(binding)) {
                    state
                        .read_aliases
                        .insert(place(binding), (path, source_version, alias_version));
                }
            }
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
        if let Some(primitive) = nir::program::machine_integer_type_in(expr, self.target()) {
            let (signed, width) = self.integer_layout(primitive).unwrap();
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
        let result = self.inner(expr, state, depth).and_then(|mut value| {
            if matches!(expr.ty, Term::Pointer(_)) && value.pointer_origins.is_none() {
                value.pointer_origins = self
                    .pointer_value_origins(&value, state)?
                    .map(|origins| (state.frame, origins));
            }
            Ok(value)
        });
        result.map_err(|reason| {
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
            K::Literal(_)
            | K::Local(_)
            | K::Unit
            | K::ComptimeValue(_)
            | K::TypeValue(_)
            | K::TypeListValue(_)
            | K::Function(_)
            | K::FunctionInstance { .. }
            | K::TraitMethod { .. } => self.binding_expression(expr, state, depth)?,
            K::Block { .. } | K::CallPreparation { .. } | K::Let { .. } => {
                self.scope_expression(expr, state, depth)?
            }
            K::IntegerBinary { .. } | K::IntegerNegate(_) => {
                self.integer_expression(expr, state, depth)?
            }
            K::Not(_)
            | K::And(..)
            | K::Or(..)
            | K::TemplateEqual(..)
            | K::ScalarEqual(..)
            | K::ScalarLess(..)
            | K::ScalarLessEqual(..)
            | K::ScalarGreater(..)
            | K::ScalarGreaterEqual(..) => self.logical_expression(expr, state, depth)?,
            K::Snapshot(_)
            | K::EffectResume(_)
            | K::EffectHandle { .. }
            | K::EffectComputation { .. }
            | K::EffectRun { .. }
            | K::EffectRequest { .. }
            | K::LetPattern { .. }
            | K::Match { .. }
            | K::RequirePredicate(_)
            | K::EnsurePredicate(_)
            | K::CheckPredicate(_)
            | K::If { .. }
            | K::Return(_)
            | K::Loop { .. }
            | K::Break(_)
            | K::Continue(_)
            | K::Call { .. } => self.control_expression(expr, state, depth)?,
            _ => self.general_expression(expr, state, depth)?,
        };
        let invalidates_external_reads = match &expr.kind {
            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } => {
                self.limits.step(1)?;
                self.addressed_locals.contains(&place.root)
            }
            K::BitWritePointer { .. }
            | K::EffectRequest { .. }
            | K::EffectRun { .. }
            | K::EffectHandle { .. } => true,
            _ => false,
        };
        if invalidates_external_reads {
            state.forget_reads(self.limits)?;
        }
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
                result.range = Some(Interval::singleton(
                    &Integer::from_u128(u128::from(truth)),
                    self.limits,
                )?);
                result.condition = Some(Condition::Bool(truth));
            }
        }
        Ok(result)
    }
    #[inline(never)]
    fn call_expression(
        &mut self,
        expr: &Expr,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
        parameter_bindings: &[nir::program::LocalId],
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
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
            let parameters: Vec<_> = definition
                .binders_with_self(&mut self.limits.arithmetic.steps)
                .map_err(|_| Reason::InvalidSignature)?
                .into_iter()
                .chain(
                    super::memory::memory_binders(
                        self.package,
                        &method.type_parameters,
                        &mut self.limits.arithmetic.steps,
                    )
                    .map_err(|_| Reason::InvalidSignature)?,
                )
                .collect();
            let arguments: Vec<_> = std::iter::once(subject.clone())
                .chain(interface_arguments.clone())
                .chain(method_arguments.clone())
                .collect();
            Some(
                method
                    .program_summary_parameters(&parameters, &arguments)
                    .map_err(|_| Reason::InvalidSignature)?,
            )
        } else {
            None
        };
        let direct = match &callee.kind {
            K::Function(id) => Some((*id, &[][..])),
            K::FunctionInstance {
                declaration,
                arguments,
            } => Some((*declaration, arguments.as_slice())),
            _ => None,
        };
        let selected_target = match (direct, self.package) {
            (Some((declaration, arguments)), Some(package)) => {
                self.limits.step(
                    package
                        .runtime_instances
                        .len()
                        .saturating_add(package.functions.len())
                        .saturating_add(package.quote_templates.len()),
                )?;
                Some(
                    package
                        .runtime_body(declaration, arguments)
                        .ok_or(Reason::InvalidSignature)?,
                )
            }
            _ => match &callee.kind {
                K::TraitMethod { .. } => method_summary.as_ref().map(|f| (f, &[][..])),
                _ => None,
            },
        };
        let target = selected_target.map(|(function, _)| function);
        // A residual instance already substituted both clause and ordinary
        // staged inputs. Its key identifies it, but must not substitute again.
        let target_arguments = selected_target
            .map(|(_, arguments)| arguments)
            .unwrap_or(&[]);
        let mut preconditions = Vec::new();
        let mut postconditions = Vec::new();
        let result_binding = target.and_then(|f| f.result_binding);
        let snapshots = target
            .map(|f| f.entry_snapshots.as_slice())
            .unwrap_or_default();
        let needs_arguments =
            target.is_some_and(|f| !f.preconditions.is_empty() || !f.entry_snapshots.is_empty());
        if let Some(target) = target {
            let sorts = if !target.type_parameters.is_empty()
                && (!target.preconditions.is_empty() || !target.postconditions.is_empty())
            {
                self.package
                    .ok_or(Reason::InvalidSignature)?
                    .declaration_parameter_sorts(&mut self.limits.arithmetic.steps)
                    .map_err(|_| Reason::InvalidSignature)?
            } else {
                BTreeMap::new()
            };
            let binders = target
                .type_parameters
                .iter()
                .map(|id| nir::DeclarationParameter {
                    declaration: *id,
                    sort: sorts.get(id).copied().unwrap_or(nir::Sort::Type),
                })
                .collect::<Vec<_>>();
            if needs_arguments && parameter_bindings != &target.parameters {
                return Err(Reason::InvalidSignature);
            }
            for condition in &target.preconditions {
                let condition = if !target_arguments.is_empty() || !binders.is_empty() {
                    let arguments = target_arguments;
                    if arguments.iter().any(|ty| ty.check_scope(&[]).is_err()) {
                        condition.instantiate_parameters_in_scope(
                            &binders,
                            arguments,
                            &self.input_frames,
                        )
                    } else {
                        condition.instantiate_parameters(&binders, arguments)
                    }
                    .map_err(|_| Reason::InvalidSignature)?
                } else {
                    condition.clone()
                };
                preconditions.push(condition);
            }
            for condition in &target.postconditions {
                let condition = if !target_arguments.is_empty() || !binders.is_empty() {
                    let arguments = target_arguments;
                    if arguments.iter().any(|ty| ty.check_scope(&[]).is_err()) {
                        condition.instantiate_parameters_in_scope(
                            &binders,
                            arguments,
                            &self.input_frames,
                        )
                    } else {
                        condition.instantiate_parameters(&binders, arguments)
                    }
                    .map_err(|_| Reason::InvalidSignature)?
                } else {
                    condition.clone()
                };
                postconditions.push(condition);
            }
        }
        // Reserve immutable contract observations in the caller before making
        // the invocation frame. A constant first mentioned by a postcondition
        // still needs a caller identity for exporting result/bound relations.
        // This introduces only its unknown domain, not its initializer value.
        let mut associated_inputs = Vec::new();
        for predicate in preconditions.iter().chain(&postconditions) {
            iir::visit::expression(predicate, &mut self.limits.arithmetic.steps, |expr, _| {
                if matches!(expr.kind, K::AssociatedValue { .. }) {
                    associated_inputs.push(expr);
                }
                Ok(())
            })
            .map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
        }
        for input in associated_inputs {
            self.child(input, state, depth)?;
        }
        let mut transfers = Vec::new();
        let mut pointer_postconditions = false;
        for predicate in &postconditions {
            iir::visit::expression(predicate, &mut self.limits.arithmetic.steps, |expr, _| {
                pointer_postconditions |= matches!(expr.kind, K::ReadPointer(_));
                Ok(())
            })
            .map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
        }
        let mut stable_postcondition_parameters = pointer_postconditions;
        if stable_postcondition_parameters {
            if let Some(target) = target {
                iir::visit::expression(
                    &target.body,
                    &mut self.limits.arithmetic.steps,
                    |expr, fuel| {
                        *fuel = fuel
                            .checked_sub(target.parameters.len())
                            .ok_or("parameter stability budget exhausted")?;
                        match &expr.kind {
                            K::Assign { place, .. }
                            | K::BitAssign { place, .. }
                            | K::BitUpdate { place, .. }
                                if target.parameters.contains(&place.root) =>
                            {
                                stable_postcondition_parameters = false
                            }
                            K::AddressOf(_) => stable_postcondition_parameters = false,
                            _ => {}
                        }
                        Ok(())
                    },
                )
                .map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
            } else {
                stable_postcondition_parameters = false;
            }
        }
        let readonly_postconditions = stable_postcondition_parameters
            && self.call_preserves_memory(callee, arguments, parameter_bindings);
        let mut reads = Vec::new();
        for predicate in preconditions
            .iter()
            .chain(postconditions.iter().filter(|_| readonly_postconditions))
        {
            iir::visit::expression(predicate, &mut self.limits.arithmetic.steps, |expr, _| {
                if let K::ReadPointer(pointer) = &expr.kind {
                    reads.push((pointer.as_ref(), &expr.ty));
                }
                Ok(())
            })
            .map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
        }
        // Contract inputs are required even when the body has no usable
        // memory summary (for example, because it stores a new pointer).
        let mut contract_inputs = BTreeSet::new();
        let mut input_pointers: Vec<_> = reads.iter().map(|(pointer, _)| *pointer).collect();
        for argument in arguments.iter().filter(|argument| argument.is_default) {
            iir::visit::expression(
                &argument.value,
                &mut self.limits.arithmetic.steps,
                |expr, _| {
                    if matches!(expr.ty, Term::Pointer(_)) {
                        input_pointers.push(expr);
                    }
                    Ok(())
                },
            )
            .map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
        }
        for pointer in input_pointers {
            let Some(path) = self.external_read_path(pointer)? else {
                continue;
            };
            self.limits.step(parameter_bindings.len())?;
            if let Some(parameter) = parameter_bindings
                .iter()
                .position(|binding| place(*binding) == path.root)
            {
                contract_inputs
                    .insert((parameter, owned_pointer_input_path(pointer, self.limits)?));
            }
        }
        let mut default_summaries = BTreeMap::new();
        self.limits.step(arguments.len())?;
        if let Some(package) = self
            .package
            .filter(|_| arguments.iter().any(|argument| argument.is_default))
        {
            let mut prepared = Vec::new();
            for argument in arguments {
                self.limits.step(1)?;
                if argument.is_default {
                    let allowance = (self.limits.arithmetic.steps / 4).min(20_000);
                    let mut fuel = allowance;
                    let summary = super::memory::infer_argument(
                        package,
                        &argument.value,
                        &prepared,
                        &mut fuel,
                    );
                    self.limits.arithmetic.steps -= allowance - fuel;
                    if let Ok(summary) = summary {
                        if !summary.may_run_cleanup
                            && !summary.accesses.iter().any(|access| access.write)
                        {
                            for target in summary.result_targets.iter().flatten().chain(
                                summary
                                    .pointer_fields
                                    .values()
                                    .filter_map(|targets| targets.as_ref())
                                    .flatten(),
                            ) {
                                self.limits
                                    .step(target.input_fields.len().saturating_add(1))?;
                                contract_inputs
                                    .insert((target.parameter, target.input_fields.clone()));
                            }
                            default_summaries.insert(argument.parameter, summary);
                        }
                    }
                }
                let Some(binding) = parameter_bindings.get(argument.parameter) else {
                    return Err(Reason::InvalidSignature);
                };
                super::memory::bound_type(&argument.value.ty, &mut self.limits.arithmetic.steps)
                    .map_err(|_| {
                        Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into())
                    })?;
                prepared.push((*binding, argument.value.ty.clone(), argument.parameter));
            }
        }
        let mut evaluated_targets = BTreeMap::new();
        let mut observed_inputs = BTreeMap::new();
        let mut evaluated_addresses = BTreeMap::new();
        let mut returned_addresses = BTreeMap::new();
        let mut returned_places = BTreeMap::new();
        let mut defaults = State {
            target: self.target(),
            frame: self.fresh_frame()?,
            facts: Facts::default(),
            domains: BTreeMap::new(),
            aggregates: BTreeMap::new(),
            paths: BTreeMap::new(),
            frozen: BTreeMap::new(),
            reads: BTreeMap::new(),
            read_aliases: BTreeMap::new(),
        };
        if !postconditions.is_empty() {
            transfers = self.copy_frozen_frame(state, &mut defaults)?;
        }
        // Declaration captures are immutable across the call. Transport their
        // symbolic facts alongside evaluated arguments, using the same copy
        // evidence so relations such as value <= maximum survive the frame.
        self.limits
            .step(self.captured_values.len() + self.associated_scalars.len())?;
        let captures = self
            .captured_values
            .values()
            .cloned()
            .chain(
                self.associated_scalars
                    .iter()
                    .map(|(_, ty, root)| (ty.clone(), *root)),
            )
            .collect::<Vec<_>>();
        for (ty, root) in captures {
            if state.facts.value(root).is_some() {
                let value = self.local_value_at(state, root)?;
                let transfer = self.prepare_copy(&value, state)?;
                bind_value_place(&mut defaults, root, &ty, value, self.limits)?;
                if let Some(transfer) = transfer {
                    transfers.extend(self.install_copy_at(&mut defaults, root, &[], transfer)?);
                }
            }
        }
        self.limits.step(arguments.len())?;
        let has_defaults = arguments.iter().any(|argument| argument.is_default);
        let stable_defaults = !has_defaults
            || ((!preconditions.is_empty()
                || stable_postcondition_parameters
                || !state.reads.is_empty())
                && self.defaults_preserve_memory(callee, arguments, parameter_bindings, true));
        for argument in arguments {
            if !state.facts.is_reachable() {
                break;
            }
            // Argument evaluation alternates caller/default frames;
            // neither frame's cached reads can survive effects in the other.
            defaults.forget_reads(self.limits)?;
            if argument.is_default && !stable_defaults {
                state.forget_reads(self.limits)?;
            }
            let mut value = if argument.is_default {
                self.child(&argument.value, &mut defaults, depth)?
            } else {
                self.child(&argument.value, state, depth)?
            };
            if argument.is_default
                && stable_defaults
                && matches!(argument.value.ty, Term::Pointer(_))
            {
                let origins = if let Some(summary) = default_summaries.get(&argument.parameter) {
                    self.summary_pointer_origins(
                        &summary.accesses,
                        summary.may_run_cleanup,
                        false,
                        summary.result_targets.as_ref(),
                        &returned_addresses,
                        state,
                    )?
                } else {
                    self.contract_pointer_origins(
                        &argument.value,
                        parameter_bindings,
                        &returned_addresses,
                        state,
                        0,
                    )?
                };
                if let Some(origins) = origins {
                    // Fixed loads in a readonly default use caller storage.
                    // The ordinary default-frame evaluation above still checks
                    // the expression; only explicit pointer provenance crosses.
                    value.set_pointer_origins(state.frame, origins, self.limits)?;
                }
            }
            if argument.is_default && stable_defaults {
                if let Some(summary) = default_summaries.get(&argument.parameter) {
                    for (fields, targets) in &summary.pointer_fields {
                        self.limits.step(fields.len().saturating_add(1))?;
                        if let Some(origins) = self.summary_pointer_origins(
                            &summary.accesses,
                            summary.may_run_cleanup,
                            false,
                            targets.as_ref(),
                            &returned_addresses,
                            state,
                        )? {
                            // Keep aggregate field provenance beside the prepared
                            // values: an opaque nominal result need not materialize
                            // caller storage into the private default frame.
                            returned_addresses
                                .insert((argument.parameter, fields.clone()), origins);
                        }
                    }
                }
            }
            if !argument.is_default || stable_defaults {
                for (parameter, fields) in &contract_inputs {
                    self.limits.step(fields.len().saturating_add(1))?;
                    if *parameter == argument.parameter {
                        let origins = if argument.is_default {
                            // Prepared values may carry caller-owned provenance,
                            // but their scalar projections belong to the default
                            // frame. Never reinterpret those local IDs in caller
                            // storage when explicit provenance is unavailable.
                            let mut leaf = Some(&value);
                            for index in fields {
                                leaf = leaf.and_then(|value| value.fields.as_ref()?.get(*index));
                            }
                            match leaf {
                                Some(leaf) => leaf
                                    .copy_pointer_origins(self.limits)?
                                    .filter(|(frame, _)| *frame == state.frame)
                                    .map(|(_, origins)| origins),
                                None => None,
                            }
                        } else {
                            self.pointer_input_origins(&value, fields, state)?
                        };
                        if let Some(origins) = origins {
                            returned_addresses.insert((*parameter, fields.clone()), origins);
                        }
                    }
                }
            }
            if !argument.is_default {
                if let Some(summary) = self.call_summary(callee) {
                    {
                        for target in summary
                            .accesses
                            .iter()
                            .map(|access| &access.target)
                            .chain(summary.result_targets.iter().flatten())
                            .chain(
                                summary
                                    .pointer_fields
                                    .values()
                                    .filter_map(|targets| targets.as_ref())
                                    .flatten(),
                            )
                        {
                            self.limits.step(1)?;
                            if target.parameter != argument.parameter {
                                continue;
                            }
                            self.limits
                                .step(target.input_fields.len().saturating_add(1))?;
                            let mut leaf = Some(&value);
                            for index in &target.input_fields {
                                self.limits.step(1)?;
                                leaf = leaf.and_then(|value| value.fields.as_ref()?.get(*index));
                            }
                            if let Some(leaf) = leaf {
                                if let Some((_, places)) = leaf
                                    .copy_pointer_target(self.limits)?
                                    .filter(|(frame, _)| *frame == state.frame)
                                {
                                    returned_places.insert(
                                        (argument.parameter, target.input_fields.clone()),
                                        places,
                                    );
                                }
                            }
                            if let Some(paths) =
                                self.pointer_input_origins(&value, &target.input_fields, state)?
                            {
                                returned_addresses.insert(
                                    (argument.parameter, target.input_fields.clone()),
                                    paths,
                                );
                            }
                        }
                    }
                }
            }
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
                                evaluated_addresses
                                    .insert(argument.parameter, (address.clone(), version));
                            }
                        }
                        let mut observations = Vec::new();
                        for (path, observation) in &state.reads {
                            self.limits.step(path.fields.len().saturating_add(1))?;
                            if path.root == address.root && path.fields.starts_with(&address.fields)
                            {
                                observations.push((
                                    path.fields[address.fields.len()..].to_vec(),
                                    path.clone(),
                                    observation.clone(),
                                ));
                            }
                        }
                        observed_inputs.insert(argument.parameter, observations);
                    }
                } else if stable_defaults {
                    if let K::Local(address) = argument.value.kind {
                        self.limits.step(parameter_bindings.len())?;
                        if let Some(index) = parameter_bindings
                            .iter()
                            .position(|binding| *binding == address)
                        {
                            if let Some((path, version)) = evaluated_addresses.get(&index) {
                                self.limits.step(path.fields.len().saturating_add(1))?;
                                evaluated_addresses
                                    .insert(argument.parameter, (path.clone(), *version));
                            }
                            if let Some(observations) = observed_inputs.get(&index) {
                                for (fields, path, _) in observations {
                                    self.limits.step(
                                        fields
                                            .len()
                                            .saturating_add(path.fields.len())
                                            .saturating_add(1),
                                    )?;
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
                    if let Some(truth) = condition.truth_by_enclosure(source, self.limits, 0)? {
                        let bit = Integer::from_u128(u128::from(truth));
                        value.range = Some(Interval::new(bit.clone(), bit.clone(), self.limits)?);
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
                    transfers.extend(self.install_copy(&mut defaults, *binding, transfer)?);
                    self.finish_copy_batch(&mut defaults, &transfers)?;
                }
            } else if has_defaults || needs_arguments {
                return Err(Reason::InvalidSignature);
            }
        }
        if state.facts.is_reachable()
            && stable_defaults
            && (!preconditions.is_empty() || readonly_postconditions)
        {
            // Import pointee observations only after every actual argument
            // has run. Captured targets preserve address evaluation order;
            // unknown-pointer evidence must still be current in the caller.
            let mut ordered = Vec::new();
            for (pointer, ty) in reads {
                if let Some(load_depth) = fixed_pointer_load_depth(pointer, self.limits)? {
                    ordered.push((load_depth, pointer, ty));
                }
            }
            self.limits.step(
                ordered
                    .len()
                    .saturating_mul(ordered.len().max(1).ilog2() as usize + 1),
            )?;
            ordered.sort_by_key(|(depth, _, _)| *depth);
            for (load_depth, pointer, ty) in ordered {
                if load_depth > 0 || matches!(ty, Term::Pointer(_)) {
                    let Some(origins) = self.contract_pointer_origins(
                        pointer,
                        parameter_bindings,
                        &returned_addresses,
                        state,
                        0,
                    )?
                    else {
                        continue;
                    };
                    let mut observed: Option<Value> = None;
                    for origin in origins {
                        let value = self.read_pointer_origin(origin, ty, state)?;
                        observed = Some(match observed {
                            Some(previous) => previous.join(&value, self.limits, 0)?,
                            None => value,
                        });
                    }
                    let Some(value) = observed else {
                        continue;
                    };
                    let path = if let Some(path) = self.external_read_path(pointer)? {
                        path
                    } else {
                        // Inner pointer observations have already been installed.
                        // Evaluate only the structurally fixed pointer chain in
                        // the private contract frame, never a source call.
                        let pointer_value = self.child(pointer, &mut defaults, depth)?;
                        let Some(path) = self.pointer_value_path(&pointer_value, &defaults)? else {
                            continue;
                        };
                        path
                    };
                    if matches!(ty, Term::Pointer(_)) {
                        // Keep caller origins outside the callee frame. This
                        // ghost identifies the corresponding loaded pointer.
                        self.observe_external_path(path, ty, &mut defaults)?;
                    } else {
                        let view = self.scalar_type_view(ty)?;
                        let Some(root) = self.result_root(&view)? else {
                            continue;
                        };
                        let transfer = self.prepare_copy(&value, state)?;
                        bind_value_place(&mut defaults, root, &view, value, self.limits)?;
                        if let Some(transfer) = transfer {
                            transfers.extend(self.install_copy_at(
                                &mut defaults,
                                root,
                                &[],
                                transfer,
                            )?);
                        }
                        defaults.reads.insert(path, (view, root));
                    }
                    continue;
                }
                let Some(path) = self.external_read_path(pointer)? else {
                    continue;
                };
                self.limits.step(parameter_bindings.len())?;
                let Some(parameter) = parameter_bindings
                    .iter()
                    .position(|binding| place(*binding) == path.root)
                else {
                    continue;
                };
                if defaults.reads.contains_key(&path) {
                    continue;
                }
                let view = self.scalar_type_view(ty)?;
                if !matches!(view, Term::Primitive(p) if p == nir::Primitive::Bool || self.integer_layout(p).is_some())
                {
                    continue;
                }
                let input_fields = owned_pointer_input_path(pointer, self.limits)?;
                let pointee_fields = &path.fields[input_fields.len()..];
                let local_targets = if input_fields.is_empty() {
                    evaluated_targets
                        .get(&parameter)
                        .and_then(|targets| targets.as_ref())
                } else {
                    returned_places.get(&(parameter, input_fields.clone()))
                };
                let observed = if let Some(targets) = local_targets {
                    let mut joined: Option<Value> = None;
                    for target in targets {
                        self.limits.step(
                            target
                                .fields
                                .len()
                                .saturating_add(pointee_fields.len())
                                .saturating_add(1),
                        )?;
                        let mut target = target.clone();
                        for (owner, index) in pointee_fields {
                            let owner = match owner {
                                AggregateKind::Record(id) => nir::program::FieldOwner::Nominal(*id),
                                AggregateKind::AnonymousRecord => {
                                    nir::program::FieldOwner::Anonymous
                                }
                                _ => return Err(Reason::InvalidSignature),
                            };
                            target.fields.push((owner, *index));
                        }
                        let value = self.read_pointer_place(state, &target)?;
                        joined = Some(match joined {
                            Some(old) => old.join(&value, self.limits, 0)?,
                            None => value,
                        });
                    }
                    joined
                } else if let Some(addresses) = returned_addresses.get(&(parameter, input_fields)) {
                    let mut joined: Option<Value> = None;
                    {
                        for address in addresses {
                            self.limits.step(
                                address
                                    .path_len()
                                    .saturating_add(pointee_fields.len())
                                    .saturating_add(1),
                            )?;
                            if !address.current(state) {
                                joined = None;
                                break;
                            }
                            let mut address = address.clone();
                            let mut fields = Vec::new();
                            for (kind, index) in pointee_fields {
                                let owner = match kind {
                                    AggregateKind::Record(id) => {
                                        nir::program::FieldOwner::Nominal(*id)
                                    }
                                    AggregateKind::AnonymousRecord => {
                                        nir::program::FieldOwner::Anonymous
                                    }
                                    _ => return Err(Reason::InvalidSignature),
                                };
                                fields.push((owner, *index));
                            }
                            address.project(&fields);
                            let current = self.read_pointer_origin(address, ty, state)?;
                            joined = Some(match joined {
                                Some(previous) => previous.join(&current, self.limits, 0)?,
                                None => current,
                            });
                        }
                    }
                    joined
                } else if let Some(observations) = observed_inputs.get(&parameter) {
                    let mut value = None;
                    for (fields, address, observation) in observations {
                        self.limits.step(fields.len().saturating_add(1))?;
                        if fields == &path.fields
                            && observation.0 == view
                            && state.reads.get(address) == Some(observation)
                        {
                            value = Some(self.local_value_at(state, observation.1)?);
                            break;
                        }
                    }
                    value
                } else {
                    None
                };
                let Some(value) = observed else {
                    continue;
                };
                let root = self.result_root(&view)?.ok_or(Reason::Unsupported)?;
                let transfer = self.prepare_copy(&value, state)?;
                bind_value_place(&mut defaults, root, &view, value, self.limits)?;
                if let Some(transfer) = transfer {
                    transfers.extend(self.install_copy_at(&mut defaults, root, &[], transfer)?);
                }
                defaults.reads.insert(path, (view, root));
            }
            self.finish_copy_batch(&mut defaults, &transfers)?;
        }
        if state.facts.is_reachable() {
            for predicate in &preconditions {
                let checked = self.child(predicate, &mut defaults, depth)?;
                let condition = checked.condition.ok_or(Reason::MissingPreconditionProof)?;
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
            self.bind_related_value(&mut defaults, snapshot.binding, &argument.value.ty, value)?;
            defaults
                .frozen
                .insert(place(snapshot.binding), argument.value.ty.clone());
        }
        // Recheck after all argument evaluation, before call havoc retires
        // scalar facts. Only a complete scalar-write footprint guarantees
        // that the call cannot replace these pointer-valued bindings.
        let mut stable_addresses = BTreeMap::new();
        if stable_postcondition_parameters
            && stable_defaults
            && self.call_preserves_pointer_bindings(callee)?
        {
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
            let mut plan = self.call_memory_plan(
                callee,
                arguments,
                parameter_bindings,
                Some(&evaluated_targets),
                Some(&returned_places),
                None,
                None,
            );
            if plan.is_none() && stable_defaults {
                plan = self.indirect_scalar_write_plan(callee, &returned_addresses, state)?;
            }
            if let Some(plan) = plan {
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
        let delivered = if let Term::Computation { result, .. } = &expr.ty {
            result.as_ref()
        } else {
            &expr.ty
        };
        let result_ty = self.scalar_type_view(delivered)?;
        let memory_result =
            if stable_postcondition_parameters && stable_defaults && state.facts.is_reachable() {
                self.apply_memory_postconditions(
                    state,
                    &defaults,
                    &postconditions,
                    snapshots,
                    parameter_bindings,
                    &evaluated_targets,
                    &stable_addresses,
                    if readonly_postconditions {
                        None
                    } else {
                        result_binding
                    },
                    &result_ty,
                    // Include ordinary argument copies, not only frozen
                    // entry ghosts. Export validates their caller versions
                    // after all argument evaluation and memory invalidation.
                    &transfers,
                    depth,
                )?
            } else {
                None
            };
        let has_memory_result = memory_result.is_some();
        result = match memory_result {
            Some(value) => value,
            None => self.unknown_value(&result_ty)?,
        };
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
                    if !exportable_postcondition(
                        predicate,
                        Some(binding),
                        snapshots,
                        (readonly_postconditions && stable_defaults).then_some(&returned.reads),
                        self.limits,
                    )? {
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
        if matches!(expr.ty, Term::Pointer(_)) && result.pointer_target.is_none() {
            if let Some(summary) = self.call_summary(callee) {
                let stable_storage = self.call_preserves_pointer_bindings(callee)?;
                if let Some(origins) = self.summary_pointer_origins(
                    &summary.accesses,
                    summary.may_run_cleanup,
                    stable_storage,
                    summary.result_targets.as_ref(),
                    &returned_addresses,
                    state,
                )? {
                    result.set_pointer_origins(state.frame, origins, self.limits)?;
                }
            }
        }
        if stable_defaults && state.facts.is_reachable() {
            if let Some(summary) = self.call_summary(callee) {
                if !summary.pointer_fields.is_empty() && !summary.may_run_cleanup {
                    let stable_storage = self.call_preserves_pointer_bindings(callee)?;
                    self.expand_owned_shape(&mut result, 0)?;
                    for (fields, targets) in &summary.pointer_fields {
                        self.limits.step(fields.len().saturating_add(1))?;
                        let Some(origins) = self.summary_pointer_origins(
                            &summary.accesses,
                            summary.may_run_cleanup,
                            stable_storage,
                            targets.as_ref(),
                            &returned_addresses,
                            state,
                        )?
                        else {
                            continue;
                        };
                        let mut leaf = Some(&mut result);
                        for index in fields {
                            leaf = leaf.and_then(|value| value.fields.as_mut()?.get_mut(*index));
                        }
                        if let Some(leaf) = leaf {
                            // Preserve field-specific caller provenance without
                            // identifying the returned aggregate with an input.
                            leaf.set_pointer_origins(state.frame, origins, self.limits)?;
                        }
                    }
                }
            }
        }
        Ok(result)
    }
    #[inline(never)]
    fn closure_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let result = Value::default();
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
                    target: self.target(),
                    frame: self.fresh_frame()?,
                    facts: Facts::default(),
                    domains: BTreeMap::new(),
                    aggregates: BTreeMap::new(),
                    paths: BTreeMap::new(),
                    frozen: BTreeMap::new(),
                    reads: BTreeMap::new(),
                    read_aliases: BTreeMap::new(),
                };
                self.limits.step(captures.len())?;
                let immutable_captures = captures
                    .iter()
                    .filter(|capture| {
                        capture.value.is_none()
                            && state.frozen.contains_key(&place(capture.binding))
                    })
                    .map(|capture| capture.binding)
                    .collect::<BTreeSet<_>>();
                let mutable_captures =
                    mutable_capture_paths(body, captures, &immutable_captures, self.limits)?;
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
                for capture in captures {
                    if immutable_captures.contains(&capture.binding)
                        && !mutable_captures.contains_key(&capture.binding)
                    {
                        invocation
                            .frozen
                            .insert(place(capture.binding), capture.ty.clone());
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
                    let ty = parameter
                        .ty
                        .substitute_partial(&[])
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
            _ => unreachable!(),
        }
        Ok(result)
    }
    fn known_branch(
        &mut self,
        condition: Option<&Condition>,
        state: &State,
    ) -> Result<Option<bool>, Reason> {
        let Some(condition) = condition else {
            return Ok(None);
        };
        if let Some(truth) = state.facts.known_condition(condition, self.limits)? {
            return Ok(Some(truth));
        }
        match condition.truth_by_enclosure(&state.facts, self.limits, 0) {
            Ok(truth) => Ok(truth),
            Err(Unknown::MissingBound(_)) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    #[inline(never)]
    fn branch_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let mut result = Value::default();
        match &expr.kind {
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition = self.child(condition, state, depth)?.condition;
                // The condition has already executed. A proved branch needs
                // neither a duplicate state nor an impossible-branch merge.
                if let Some(truth) = self.known_branch(condition.as_ref(), state)? {
                    state
                        .facts
                        .assume(condition.as_ref().unwrap(), truth, self.limits)?;
                    return if truth {
                        self.child(then_branch, state, depth)
                    } else if let Some(branch) = else_branch {
                        self.child(branch, state, depth)
                    } else {
                        Ok(Value::default())
                    };
                }
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
            _ => unreachable!(),
        }
        Ok(result)
    }
    #[inline(never)]
    fn loop_expression(
        &mut self,
        identity: LocalId,
        body: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<(), Reason> {
        let mut tail = body;
        loop {
            self.limits.step(1)?;
            match &tail.kind {
                K::Block {
                    result: Some(result),
                    ..
                }
                | K::CallPreparation {
                    result: Some(result),
                    ..
                } => tail = result,
                _ => break,
            }
        }
        let mut single = matches!(tail.kind,K::Break(target) if target==identity);
        if single {
            nir::visit::expression(body, &mut self.limits.arithmetic.steps, |expr, _| {
                if matches!(expr.kind,K::Continue(target) if target==identity) {
                    single = false;
                }
                Ok(())
            })
            .map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
        }
        if single {
            if self.single_pass_exits.insert(identity, vec![]).is_some() {
                return Err(Reason::InvalidSignature);
            }
            let mut iteration = state.fork(self.limits)?;
            let checked = self.child(body, &mut iteration, depth);
            let exits = self.single_pass_exits.remove(&identity).unwrap();
            checked?;
            let mut merged: Option<State> = None;
            for exit in exits {
                merged = Some(if let Some(previous) = merged {
                    previous.merge(exit, self.limits)?
                } else {
                    exit
                });
            }
            if let Some(exit) = merged {
                *state = exit;
            } else {
                state
                    .facts
                    .assume(&Condition::Bool(false), true, self.limits)?;
            }
        } else {
            self.forget_loop_writes(body, state)?;
            let mut iteration = state.fork(self.limits)?;
            self.child(body, &mut iteration, depth)?;
            self.forget_loop_writes(body, state)?;
        }
        Ok(())
    }
    fn integer_spelling(&mut self, text: &str) -> Result<Integer, Reason> {
        // NIR constants and patterns can carry a materialized sign even though
        // lexical integer tokens are unsigned. Both consumers share the same
        // parsing and arithmetic budget, including the cost of negation.
        let (negative, digits) = text
            .strip_prefix('-')
            .map_or((false, text), |digits| (true, digits));
        let value =
            Integer::parse_literal(digits, &mut self.limits.arithmetic).map_err(Unknown::from)?;
        if negative {
            Ok(value
                .negate(&mut self.limits.arithmetic)
                .map_err(Unknown::from)?)
        } else {
            Ok(value)
        }
    }

    #[inline(never)]
    fn binding_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let mut result = Value::default();
        match &expr.kind {
            K::ComptimeValue(Term::Rigid {
                declaration,
                sort: nir::Sort::ComptimeValue,
            }) => {
                self.limits.step(1)?;
                let view = self.scalar_type_view(&expr.ty)?;
                if !matches!(view, Term::Primitive(p) if self.integer_layout(p).is_some() || matches!(p, nir::Primitive::Bool | nir::Primitive::Integer))
                    && !matches!(
                        view,
                        Term::Tuple(_) | Term::Record(_) | Term::Nominal { .. }
                    )
                    && self.tuple_field_types(&view)?.is_none()
                {
                    return Ok(result);
                }
                let root = if let Some((domain, root)) = self.captured_values.get(declaration) {
                    if domain != &view {
                        return Err(Reason::InvalidSignature);
                    }
                    *root
                } else {
                    let Some(package) = self.package else {
                        return Ok(result);
                    };
                    let domains = package
                        .declaration_parameter_domains(&mut self.limits.arithmetic.steps)
                        .map_err(|_| Reason::InvalidSignature)?;
                    let domain = domains.get(declaration).ok_or(Reason::InvalidSignature)?;
                    if self.scalar_type_view(domain)? != view {
                        return Err(Reason::InvalidSignature);
                    }
                    let root = self.result_root(&view)?.ok_or(Reason::Unsupported)?;
                    self.captured_values
                        .insert(*declaration, (view.clone(), root));
                    root
                };
                if state.facts.value(root).is_none() {
                    let unknown = self.unknown_value(&view)?;
                    bind_value_place(state, root, &view, unknown, self.limits)?;
                }
                // This root describes the immutable staged input, not a
                // runtime reconstruction or memory reachable through pointers.
                // Opaque runtime calls cannot change its field observations.
                state.frozen.insert(root, view.clone());
                return self.local_value_at(state, root);
            }
            K::ComptimeValue(
                value @ (Term::TupleValue(_) | Term::RecordValue(_) | Term::NominalValue { .. }),
            ) => {
                return self.completed_aggregate_value(value, &expr.ty, expr.source, state, depth);
            }
            K::ComptimeValue(Term::Bool(value))
                if expr.ty == Term::Primitive(nir::Primitive::Bool) =>
            {
                // Completed immutable Boolean data uses the same program fact
                // as a literal; symbolic captures remain unknown below.
                let literal = Expr {
                    source: expr.source,
                    ty: expr.ty.clone(),
                    kind: K::Literal(Literal::Bool(*value)),
                };
                return self.binding_expression(&literal, state, depth);
            }
            K::ComptimeValue(Term::Integer(value)) => {
                // Capture identity is not a type proposition. Only a completed
                // integer with checked domain evidence supplies a program fact.
                let number = self.limits.integer(value)?;
                let Term::Primitive(primitive) = &expr.ty else {
                    return Err(Reason::InvalidSignature);
                };
                if let Some((signed, width)) = self.integer_layout(*primitive) {
                    if signed {
                        number.to_signed(width).map_err(Unknown::from)?;
                    } else {
                        number.to_unsigned(width).map_err(Unknown::from)?;
                    }
                } else if *primitive != nir::Primitive::Integer {
                    return Err(Reason::InvalidSignature);
                }
                result.range = Some(Interval::singleton(&number, self.limits)?);
                result.scalar = Some(Scalar::Constant(number));
            }
            K::Literal(literal) => {
                let number = match literal {
                    Literal::SignedInteger(n) => Some(Integer::from_i128(*n)),
                    Literal::UnsignedInteger(n) => Some(Integer::from_u128(*n)),
                    Literal::Integer(text) => Some(self.integer_spelling(text)?),
                    Literal::PendingInteger { spelling, negative } => {
                        if let Term::Primitive(domain) = expr.ty {
                            let target = self.target();
                            let formed = nir::integer_formation::form(
                                spelling,
                                *negative,
                                domain,
                                target,
                                &mut self.limits.arithmetic,
                            )
                            .map_err(|cause| {
                                Reason::IntegerFormation(
                                    nir::integer_formation::FormationError {
                                        source: expr.source,
                                        target: expr.ty.clone(),
                                        layout: target,
                                        cause,
                                    }
                                    .to_string(),
                                )
                            })?;
                            Some(match formed {
                                Literal::SignedInteger(value) => Integer::from_i128(value),
                                Literal::UnsignedInteger(value) => Integer::from_u128(value),
                                Literal::Integer(text) => self.integer_spelling(&text)?,
                                _ => return Err(Reason::InvalidSignature),
                            })
                        } else {
                            // The caller has not selected a concrete domain;
                            // do not turn a pending proxy into value evidence.
                            None
                        }
                    }
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
            | K::ComptimeValue(_)
            | K::TypeValue(_)
            | K::TypeListValue(_) => {}
            _ => unreachable!(),
        }
        Ok(result)
    }
    // Recursive scopes must not reserve all completed-value decoding scratch
    // space at every nesting level in unoptimized compiler builds.
    #[inline(never)]
    fn scope_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let mut result = Value::default();
        match &expr.kind {
            K::Block {
                statements,
                result: tail,
            }
            | K::CallPreparation {
                statements,
                result: tail,
            } => {
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
                        let path = match self.aliased_read_path(value, state)? {
                            Some(path) => Some(path),
                            None => self.pointer_value_path(&value_result, state)?,
                        };
                        path.and_then(|path| {
                            state.facts.value(path.root).map(|version| (path, version))
                        })
                    } else {
                        None
                    };
                    self.bind_related_value(state, *binding, &value.ty, value_result)?;
                    state.read_aliases.remove(&place(*binding));
                    if let Some((path, source_version)) = alias {
                        if let Some(alias_version) = state.facts.value(place(*binding)) {
                            state
                                .read_aliases
                                .insert(place(*binding), (path, source_version, alias_version));
                        }
                    }
                }
            }
            _ => unreachable!(),
        }
        Ok(result)
    }
    fn completed_aggregate_value(
        &mut self,
        value: &Term,
        ty: &Term,
        source: SourceRange,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        let (kind, values, types): (AggregateKind, Vec<&Term>, Vec<Term>) = match value {
            Term::TupleValue(values) => {
                let Some(types) = self.tuple_field_types(ty)? else {
                    return self.unknown_value(ty);
                };
                self.limits.step(values.len())?;
                (AggregateKind::Tuple, values.iter().collect(), types)
            }
            Term::RecordValue(values) => {
                let Term::Record(types) = ty else {
                    return self.unknown_value(ty);
                };
                self.limits.step(values.len().saturating_add(types.len()))?;
                if values.len() != types.len()
                    || values
                        .iter()
                        .zip(types)
                        .any(|((name, _), (expected, _))| name != expected)
                {
                    return Err(Reason::InvalidSignature);
                }
                crate::associated::charge_projection_copy(ty, &mut self.limits.arithmetic.steps)
                    .map_err(|_| {
                        Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into())
                    })?;
                (
                    AggregateKind::AnonymousRecord,
                    values.iter().map(|(_, v)| v).collect(),
                    types.iter().map(|(_, t)| t.clone()).collect(),
                )
            }
            Term::NominalValue {
                ty: actual,
                variant,
                fields,
            } => {
                super::memory::bound_type(actual, &mut self.limits.arithmetic.steps).map_err(
                    |_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()),
                )?;
                if actual.as_ref() != ty {
                    return self.unknown_value(ty);
                }
                let Term::Nominal { declaration, .. } = ty else {
                    return self.unknown_value(ty);
                };
                let variant = variant.map(|index| index as usize);
                let types = self.nominal_fields(ty, *declaration, variant)?;
                self.limits.step(fields.len())?;
                (
                    variant.map_or(AggregateKind::Record(*declaration), |index| {
                        AggregateKind::Variant(*declaration, index)
                    }),
                    fields.iter().collect(),
                    types,
                )
            }
            _ => return Err(Reason::InvalidSignature),
        };
        if values.len() != types.len() {
            return Err(Reason::InvalidSignature);
        }
        let mut fields = Vec::new();
        for (value, domain) in values.into_iter().zip(types) {
            // Types and type lists may be legitimate fields of a comptime
            // aggregate. They supply no scalar program fact; their formation
            // remains the staging/type layer's responsibility.
            if value.sort() != nir::Sort::ComptimeValue
                || domain == Term::Primitive(nir::Primitive::Type)
            {
                super::memory::bound_type(value, &mut self.limits.arithmetic.steps).map_err(
                    |_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()),
                )?;
                fields.push(self.unknown_value(&domain)?);
                continue;
            }
            nir::comptime_values::validate(value, &domain, &mut self.limits.arithmetic.steps)
                .map_err(|error| match error {
                    nir::comptime_values::Error::Budget => {
                        Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into())
                    }
                    _ => Reason::InvalidSignature,
                })?;
            let expression = Expr {
                source,
                ty: domain,
                kind: K::ComptimeValue(value.clone()),
            };
            fields.push(self.binding_expression(&expression, state, depth + 1)?);
        }
        super::memory::bound_type(ty, &mut self.limits.arithmetic.steps)
            .map_err(|_| Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()))?;
        Ok(Value {
            aggregate_kind: Some(kind),
            nominal_type: Some(std::sync::Arc::new(ty.clone())),
            fields: Some(fields),
            ..Value::default()
        })
    }

    fn completed_effect_result(
        &mut self,
        mut value: Value,
        ty: &Term,
        state: &mut State,
    ) -> Result<Value, Reason> {
        if !state.facts.is_reachable() {
            return Ok(value);
        }
        let ty = if let Term::Computation { result, .. } = ty {
            result.as_ref()
        } else {
            ty
        };
        // The returned value is already complete. Retain its immutable evidence
        // while invalidating mutable storage reachable by the handled effects.
        value.evidence = self.prepare_copy(&value, state)?;
        state.havoc_with_private_scalars(Some(&self.addressed_locals), self.limits)?;
        if let Some(root) = self.result_root(ty)? {
            self.bind_result(state, root, ty, value)?;
            self.local_value_at(state, root)
        } else {
            Ok(value)
        }
    }
    fn normal_input_value(
        &mut self,
        input: &nir::program::NormalFormArgument,
        domain: Option<&Term>,
        state: &mut State,
        depth: usize,
    ) -> Result<(Value, Term), Reason> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        if let Some(expression) = input.expression() {
            return Ok((
                self.child(expression, state, depth + 1)?,
                expression.ty.clone(),
            ));
        }
        if !state.facts.is_reachable() {
            return Ok((
                Value::default(),
                domain
                    .cloned()
                    .unwrap_or(Term::Primitive(nir::Primitive::Never)),
            ));
        }
        use nir::program::NormalFormArgument as A;
        match input {
            A::Integer {
                source,
                spelling,
                negative,
            } if domain == Some(&Term::Primitive(nir::Primitive::Integer)) => {
                let ty = Term::Primitive(nir::Primitive::Integer);
                let literal = Expr {
                    source: *source,
                    ty: ty.clone(),
                    kind: K::Literal(Literal::Integer(spelling.clone())),
                };
                let expression = if *negative {
                    Expr {
                        source: *source,
                        ty: ty.clone(),
                        kind: K::IntegerNegate(Box::new(literal)),
                    }
                } else {
                    literal
                };
                Ok((self.child(&expression, state, depth + 1)?, ty))
            }
            A::Integer {
                source,
                spelling,
                negative,
            } if matches!(domain, Some(Term::Primitive(primitive)) if self.integer_layout(*primitive).is_some()) =>
            {
                let ty = domain.unwrap();
                let Term::Primitive(primitive) = ty else {
                    unreachable!()
                };
                let (signed, bits) = self.integer_layout(*primitive).unwrap();
                let mut value = self.integer_spelling(spelling)?;
                if *negative {
                    value = value
                        .negate(&mut self.limits.arithmetic)
                        .map_err(Unknown::from)?;
                }
                // These intrinsic conversions preserve the mathematical value
                // only within the target domain. Do not model them as wrapping.
                let literal = if signed {
                    Literal::SignedInteger(value.to_signed(bits).map_err(Unknown::from)?)
                } else {
                    Literal::UnsignedInteger(value.to_unsigned(bits).map_err(Unknown::from)?)
                };
                let expression = Expr {
                    source: *source,
                    ty: ty.clone(),
                    kind: K::Literal(literal),
                };
                Ok((self.child(&expression, state, depth + 1)?, ty.clone()))
            }
            A::Integer {
                spelling, negative, ..
            } if matches!(domain, Some(Term::Primitive(nir::Primitive::BitVec(_)))) => {
                let ty = domain.unwrap();
                let Term::Primitive(nir::Primitive::BitVec(width)) = ty else {
                    unreachable!()
                };
                let mut value = self.integer_spelling(spelling)?;
                if *negative {
                    value = value
                        .negate(&mut self.limits.arithmetic)
                        .map_err(Unknown::from)?;
                }
                value
                    .to_bitvec(*width, &mut self.limits.arithmetic)
                    .map_err(|failure| {
                        Unknown::from(match failure {
                            comptime_runtime::bitvec::Error::Resource(resource) => resource,
                            _ => comptime_runtime::Error::OutOfRange,
                        })
                    })?;
                // Exact bit construction grants no storage/unsafe permission
                // and no integer-program fact about later bit operations.
                Ok((self.unknown_value(ty)?, ty.clone()))
            }
            A::Block {
                statements, result, ..
            } => {
                for statement in statements {
                    self.child(statement, state, depth + 1)?;
                }
                match result {
                    Some(result) => self.normal_input_value(result, domain, state, depth + 1),
                    None => Ok((Value::default(), Term::Primitive(nir::Primitive::Void))),
                }
            }
            A::If {
                condition,
                then_input,
                else_input,
                ..
            } => {
                let ty = domain.ok_or(Reason::Unsupported)?;
                let condition = self.child(condition, state, depth + 1)?.condition;
                let mut yes = state.fork(self.limits)?;
                let mut no = state.fork(self.limits)?;
                if let Some(condition) = condition {
                    yes.facts.assume(&condition, true, self.limits)?;
                    no.facts.assume(&condition, false, self.limits)?;
                }
                let (a, _) = self.normal_input_value(then_input, domain, &mut yes, depth + 1)?;
                let (b, _) = self.normal_input_value(else_input, domain, &mut no, depth + 1)?;
                let result = if let Some(root) = self.result_root(ty)? {
                    self.bind_result(&mut yes, root, ty, a)?;
                    self.bind_result(&mut no, root, ty, b)?;
                    *state = yes.merge(no, self.limits)?;
                    if state.facts.is_reachable() {
                        self.local_value_at(state, root)?
                    } else {
                        Value::default()
                    }
                } else {
                    let result = if !yes.facts.is_reachable() {
                        b
                    } else if !no.facts.is_reachable() {
                        a
                    } else {
                        a.join(&b, self.limits, 0)?
                    };
                    *state = yes.merge(no, self.limits)?;
                    result
                };
                Ok((result, ty.clone()))
            }
            A::Match { value, arms, .. } => {
                let ty = domain.ok_or(Reason::Unsupported)?;
                Ok((
                    self.normal_match_input(value, arms, ty, state, depth + 1)?,
                    ty.clone(),
                ))
            }
            A::Unit { .. }
                if matches!(
                    domain,
                    Some(Term::Primitive(nir::Primitive::Void | nir::Primitive::Type))
                ) =>
            {
                Ok((Value::default(), domain.unwrap().clone()))
            }
            A::Tuple { items, .. } => {
                let ty = domain.ok_or(Reason::Unsupported)?;
                if *ty == Term::Primitive(nir::Primitive::Type) {
                    for item in items {
                        if !state.facts.is_reachable() {
                            break;
                        }
                        self.normal_input_value(item, Some(ty), state, depth + 1)?;
                    }
                    return Ok((Value::default(), ty.clone()));
                }
                let domains = self.tuple_field_types(ty)?.ok_or(Reason::Unsupported)?;
                if domains.len() != items.len() {
                    return Err(Reason::Unsupported);
                }
                let mut fields = Vec::new();
                for (item, domain) in items.iter().zip(&domains) {
                    if !state.facts.is_reachable() {
                        return Ok((Value::default(), ty.clone()));
                    }
                    let (mut value, _) =
                        self.normal_input_value(item, Some(domain), state, depth + 1)?;
                    if !state.facts.is_reachable() {
                        return Ok((Value::default(), ty.clone()));
                    }
                    value.evidence = self.prepare_copy(&value, state)?;
                    fields.push(value.snapshot(self.limits, 0)?);
                }
                let value = Value {
                    nominal_type: Some(std::sync::Arc::new(ty.clone())),
                    fields: Some(fields),
                    aggregate_kind: Some(AggregateKind::Tuple),
                    ..Value::default()
                };
                Ok((self.materialize_aggregate(value, ty, state)?, ty.clone()))
            }
            nir::program::NormalFormArgument::TypeList { items, .. } => {
                for item in items {
                    if !state.facts.is_reachable() {
                        break;
                    }
                    self.normal_input_value(item, None, state, depth + 1)?;
                }
                // The collector stores Type descriptions, not program scalar
                // evidence. Keep its declared list domain and operand effects.
                Ok((Value::default(), domain.ok_or(Reason::Unsupported)?.clone()))
            }
            _ => Err(Reason::Unsupported),
        }
    }

    #[inline(never)]
    fn control_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let mut result = Value::default();
        match &expr.kind {
            K::EffectResume(value) => {
                self.child(value, state, depth)?;
                state
                    .facts
                    .assume(&Condition::Bool(false), true, self.limits)?;
            }
            K::EffectHandle {
                computation,
                handlers,
                ..
            } => {
                for handler in handlers {
                    self.effect_handler_arms(handler, state, depth)?;
                }
                // Installation-time providers can mutate captured storage even
                // when the subject never requests an operation. Entry facts
                // therefore cannot flow into that subject unchanged. Stable
                // by-value scalars remain valid: the full-function scan rules
                // out both writes and address exposure for those bindings.
                if handlers
                    .iter()
                    .any(|handler| handler.abi == nir::effects::Abi::DirectPassing)
                {
                    state.havoc_with_private_scalars(
                        self.handler_unstable_locals.as_ref(),
                        self.limits,
                    )?;
                }
                let completed = self.child(computation, state, depth)?;
                result = self.completed_effect_result(completed, &expr.ty, state)?;
            }
            K::EffectComputation { body, .. } => {
                result = self.child(body, state, depth)?;
            }
            K::EffectRun { computation, .. } => {
                let completed = self.child(computation, state, depth)?;
                result = self.completed_effect_result(completed, &expr.ty, state)?;
            }
            K::EffectRequest { arguments, .. } => {
                for argument in arguments {
                    self.child(argument, state, depth)?;
                }
                state.havoc_with_private_scalars(Some(&self.addressed_locals), self.limits)?;
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
            K::RequirePredicate(predicate) | K::EnsurePredicate(predicate) => {
                if state.facts.is_reachable() {
                    let failure = || {
                        if matches!(expr.kind, K::RequirePredicate(_)) {
                            Reason::MissingPreconditionProof
                        } else {
                            Reason::MissingPostconditionProof
                        }
                    };
                    // Predicate-local pattern bindings and constructor ghosts
                    // cannot escape a void proof obligation. Keep caller roots
                    // and project surviving observations when leaving its scope.
                    self.limits.step(state.domains.len())?;
                    let existing = state.domains.keys().copied().collect();
                    let checked = self.child(predicate, state, depth)?;
                    let condition = checked.condition.ok_or_else(&failure)?;
                    let proof = state
                        .facts
                        .prove_condition(&condition, true, self.limits)
                        .map_err(|_| failure())?;
                    proof
                        .replay(&state.facts, self.limits)
                        .map_err(|_| failure())?;
                    state.facts.assume(&condition, true, self.limits)?;
                    state.keep(&existing, self.limits)?;
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
            K::If { .. } => {
                result = self.branch_expression(expr, state, depth)?;
            }
            K::Return(value) => {
                if let Some(value) = value {
                    self.tail(value, state, depth)?;
                } else {
                    self.check_exit(Value::default(), None, state, depth)?;
                }
                state
                    .facts
                    .assume(&Condition::Bool(false), true, self.limits)?;
            }
            K::Loop { identity, body } => {
                self.loop_expression(*identity, body, state, depth)?;
            }
            K::Break(identity) | K::Continue(identity) => {
                if matches!(expr.kind, K::Break(_)) && self.single_pass_exits.contains_key(identity)
                {
                    let exit = state.fork(self.limits)?;
                    self.single_pass_exits.get_mut(identity).unwrap().push(exit);
                }
                state
                    .facts
                    .assume(&Condition::Bool(false), true, self.limits)?;
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } => {
                result = self.call_expression(
                    expr,
                    callee,
                    arguments,
                    parameter_bindings,
                    state,
                    depth,
                )?;
            }
            K::Snapshot(value) => {
                result = self.child(value, state, depth)?;
            }
            _ => unreachable!(),
        }
        Ok(result)
    }

    #[inline(never)]
    fn general_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let mut result = Value::default();
        match &expr.kind {
            K::Intrinsic { inputs, .. } => {
                result = self.intrinsic_value(expr, inputs, state, depth)?;
            }
            K::BitWindow { value, start, end } => {
                self.bit_window(expr, value, start, end, state, depth)?;
            }
            K::BitIndex(value, index) => self.bit_index(expr, value, index, state, depth)?,
            // IEEE arithmetic supplies no exact integer/polynomial evidence.
            // Still visit operands in source order: their calls, writes and
            // machine arithmetic remain verification obligations.
            K::FloatNegate(value)
            | K::BitSlice { value, .. }
            | K::TakeBits(value)
            | K::TextQuery { subject: value, .. }
            | K::IntegerRepresentation(value)
            | K::SymbolName(value)
            | K::EnumName(value)
            | K::BitConvert(value) => {
                self.child(value, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
            }
            K::FloatBinary { left, right, .. }
            | K::BitEqual(left, right)
            | K::BitConcat(left, right) => {
                self.child(left, state, depth)?;
                self.child(right, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
            }
            K::TypeListIndex(values, index) if matches!(expr.ty, Term::Primitive(primitive) if primitive != nir::Primitive::Object) =>
            {
                // Primitive elements carry no mutable storage aliases. Keep
                // their domain bounds, without assuming a particular element
                // value or transferring facts from another list observation.
                // Operands execute in order; checked indexing remains in the VM.
                // Never transports no element: returning from either operand
                // prevents the later operand and indexing from executing.
                // Objects and structured domains need their own alias transfer.
                self.child(values, state, depth)?;
                self.child(index, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
            }
            K::StructFieldTypes(subject)
            | K::StructFieldNames(subject)
            | K::BuildStruct(subject)
            | K::BuildEnum(subject)
            | K::ObjectQuery { subject, .. }
            | K::ObjectEdit { subject, .. }
            | K::ComptimeFunctionInstance { subject, .. }
            | K::EnumStructure {
                subject: subject, ..
            } => {
                self.child(subject, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
            }
            K::Literal(_)
            | K::Local(_)
            | K::Unit
            | K::ComptimeValue(_)
            | K::TypeValue(_)
            | K::TypeListValue(_)
            | K::Function(_)
            | K::FunctionInstance { .. }
            | K::TraitMethod { .. }
            | K::Block { .. }
            | K::CallPreparation { .. }
            | K::Let { .. } => unreachable!(),
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
                let input = self.child(pointer, state, depth)?;
                let origins = self.pointer_value_origins(&input, state)?;
                if let Some((frame, targets)) = input.pointer_target {
                    let mut projected = Vec::new();
                    for mut target in targets {
                        self.limits.step(1)?;
                        target
                            .fields
                            .push((nir::program::FieldOwner::Nominal(*declaration), *index));
                        insert_pointer_target(
                            &mut projected,
                            target,
                            &mut self.limits.arithmetic.steps,
                        )?;
                    }
                    result.pointer_target = Some((frame, projected));
                } else if let Some(mut targets) = origins {
                    for target in &mut targets {
                        self.limits.step(1)?;
                        target
                            .project(&[(nir::program::FieldOwner::Nominal(*declaration), *index)]);
                    }
                    result.pointer_origins = Some((state.frame, targets));
                }
            }
            K::PointerAnonymousField { pointer, index, .. } => {
                let input = self.child(pointer, state, depth)?;
                let origins = self.pointer_value_origins(&input, state)?;
                if let Some((frame, targets)) = input.pointer_target {
                    let mut projected = Vec::new();
                    for mut target in targets {
                        self.limits.step(1)?;
                        target
                            .fields
                            .push((nir::program::FieldOwner::Anonymous, *index));
                        insert_pointer_target(
                            &mut projected,
                            target,
                            &mut self.limits.arithmetic.steps,
                        )?;
                    }
                    result.pointer_target = Some((frame, projected));
                } else if let Some(mut targets) = origins {
                    for target in &mut targets {
                        self.limits.step(1)?;
                        target.project(&[(nir::program::FieldOwner::Anonymous, *index)]);
                    }
                    result.pointer_origins = Some((state.frame, targets));
                }
            }
            K::ReadPointer(pointer) => {
                let pointer_value = self.child(pointer, state, depth)?;
                let origins = self.pointer_value_origins(&pointer_value, state)?;
                let targets = pointer_value
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
                } else if let Some(addresses) = origins {
                    let mut joined: Option<Value> = None;
                    for address in addresses {
                        let current = self.read_pointer_origin(address, &expr.ty, state)?;
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
            K::BitWritePointer {
                pointer,
                selections,
                value,
            } => {
                let Term::Pointer(storage) = &pointer.ty else {
                    return Err(Reason::Unsupported);
                };
                let targets = self
                    .child(pointer, state, depth)?
                    .pointer_target
                    .filter(|(frame, _)| *frame == state.frame)
                    .map(|(_, targets)| targets);
                self.bit_selections(expr, storage, selections, state, depth)?;
                self.child(value, state, depth)?;
                if state.facts.is_reachable() {
                    if let Some(targets) = targets {
                        for target in targets {
                            let unknown = self.unknown_value(storage)?;
                            if target.is_root() {
                                self.bind_related_value(state, target.root, storage, unknown)?;
                            } else {
                                self.write_field(state, &target, unknown)?;
                            }
                        }
                    } else {
                        state.havoc(self.limits)?;
                    }
                }
            }
            K::WritePointer { pointer, value } => {
                let targets = self
                    .child(pointer, state, depth)?
                    .pointer_target
                    .filter(|(frame, _)| *frame == state.frame)
                    .map(|(_, targets)| targets);
                let address = self
                    .aliased_read_path(pointer, state)?
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
                                && matches!(view, Term::Primitive(p) if p == nir::Primitive::Bool || self.integer_layout(p).is_some())
                            {
                                let transfer = self.prepare_copy(&replacement, state)?;
                                Some((path, transfer))
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        state.havoc_with_private_scalars(
                            Some(&self.addressed_locals),
                            self.limits,
                        )?;
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
            K::BitAssign {
                place: target,
                storage_type,
                value,
                ..
            }
            | K::BitUpdate {
                place: target,
                storage_type,
                value,
                ..
            } => {
                if let K::BitUpdate { selections, .. } = &expr.kind {
                    self.bit_selections(expr, storage_type, selections, state, depth)?;
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
            K::Closure { .. } => {
                result = self.closure_expression(expr, state, depth)?;
            }
            K::UnsafeScope(value) => {
                result = self.child(value, state, depth)?;
            }
            K::Discard(value) => {
                self.child(value, state, depth)?;
            }
            K::NormalFormType {
                head,
                arguments,
                plan,
            } => {
                let metadata = self.package.and_then(|package| {
                    package
                        .normal_forms
                        .iter()
                        .find(|item| item.declaration == *head)
                });
                if let Some(plan) = plan {
                    self.limits
                        .step(arguments.len().saturating_add(plan.preparations.len()))?;
                    if !plan.valid(arguments.len()) {
                        return Err(Reason::Unsupported);
                    }
                    let mut preparations = plan.preparations.iter().peekable();
                    for (position, slot) in plan.order.iter().copied().enumerate() {
                        while preparations.peek().is_some_and(|p| p.before == position) {
                            self.child(&preparations.next().unwrap().expression, state, depth)?;
                        }
                        if !state.facts.is_reachable() {
                            break;
                        }
                        let domain = metadata
                            .and_then(|head| head.parameters.get(slot))
                            .map(|parameter| &parameter.ty);
                        let (value, ty) =
                            self.normal_input_value(&arguments[slot], domain, state, depth)?;
                        if state.facts.is_reachable() {
                            self.bind_related_value(state, plan.bindings[slot], &ty, value)?;
                        }
                    }
                    for preparation in preparations {
                        self.child(&preparation.expression, state, depth)?;
                    }
                } else {
                    for (slot, argument) in arguments.iter().enumerate() {
                        if !state.facts.is_reachable() {
                            break;
                        }
                        let domain = metadata
                            .and_then(|head| head.parameters.get(slot))
                            .map(|parameter| &parameter.ty);
                        self.normal_input_value(argument, domain, state, depth)?;
                    }
                }
            }
            K::NominalType {
                preparations,
                arguments,
                order,
                bindings,
                ..
            }
            | K::CheckTypeAlias {
                preparations,
                arguments,
                order,
                bindings,
                ..
            } => {
                self.limits
                    .step(arguments.len().saturating_add(preparations.len()))?;
                if expr.kind.nominal_local_bindings().is_none() {
                    return Err(Reason::Unsupported);
                }
                if bindings
                    .as_ref()
                    .is_some_and(|bindings| bindings.len() != arguments.len())
                {
                    return Err(Reason::Unsupported);
                }
                let mut seen = vec![false; arguments.len()];
                if order
                    .as_ref()
                    .is_some_and(|order| order.len() != arguments.len())
                {
                    return Err(Reason::Unsupported);
                }
                for position in 0..arguments.len() {
                    let slot = order.as_ref().map_or(position, |order| order[position]);
                    if slot >= arguments.len() || seen[slot] {
                        return Err(Reason::Unsupported);
                    }
                    seen[slot] = true;
                }
                let mut preparations = preparations.iter().peekable();
                for position in 0..arguments.len() {
                    while preparations.peek().is_some_and(|p| p.before == position) {
                        self.child(&preparations.next().unwrap().expression, state, depth)?;
                    }
                    if !state.facts.is_reachable() {
                        break;
                    }
                    let slot = order.as_ref().map_or(position, |order| order[position]);
                    let (value, ty) =
                        self.normal_input_value(&arguments[slot], None, state, depth)?;
                    if state.facts.is_reachable() {
                        if let Some(bindings) = bindings {
                            self.bind_related_value(state, bindings[slot], &ty, value)?;
                        }
                    }
                }
                for preparation in preparations {
                    self.child(&preparation.expression, state, depth)?;
                }
                if let K::CheckTypeAlias {
                    body: Some(body), ..
                } = &expr.kind
                {
                    self.child(body, state, depth)?;
                }
            }
            K::TypeList(arguments)
            | K::TupleType(arguments)
            | K::FunctionType {
                parameters: arguments,
                ..
            } => {
                // Formation predicates belong to the type checker/invocation,
                // but computing their operands is ordinary program evaluation.
                // Preserve operand order, effects and early exits here without
                // importing type propositions into the program facts.
                for argument in arguments {
                    self.child(argument, state, depth)?;
                }
            }
            K::OptionalType(value) | K::PointerType(value) | K::BitVecType(value) => {
                self.child(value, state, depth)?;
            }
            K::FunctionResultType {
                signature,
                result: output,
            } => {
                self.child(signature, state, depth)?;
                self.child(output, state, depth)?;
            }
            K::ClosureType {
                environment,
                signature,
            } => {
                for field in environment {
                    self.child(field, state, depth)?;
                }
                self.child(signature, state, depth)?;
            }
            K::TypeEqual(left, right) => {
                self.child(left, state, depth)?;
                self.child(right, state, depth)?;
                // This layer tracks the evaluation, not type-theory equality.
                // Its Bool result remains unknown without program evidence.
            }
            K::OptionalAbsent | K::OptionalPresent(_) => {
                let Term::Optional(payload_type) = &expr.ty else {
                    return Err(Reason::InvalidSignature);
                };
                let mut fields = Vec::new();
                if let K::OptionalPresent(payload) = &expr.kind {
                    if payload.ty != **payload_type {
                        return Err(Reason::InvalidSignature);
                    }
                    let mut field = self.child(payload, state, depth)?;
                    if matches!(payload.ty, Term::Pointer(_)) && field.pointer_target.is_none() {
                        field.pointer_origins = self
                            .pointer_value_origins(&field, state)?
                            .map(|targets| (state.frame, targets));
                    }
                    field.evidence = self.prepare_copy(&field, state)?;
                    fields.push(field.snapshot(self.limits, 0)?);
                }
                result.fields = Some(fields);
                result.nominal_type = Some(std::sync::Arc::new(expr.ty.clone()));
                result.aggregate_kind = Some(AggregateKind::Optional(matches!(
                    expr.kind,
                    K::OptionalPresent(_)
                )));
                result = self.materialize_aggregate(result, &expr.ty, state)?;
            }
            K::Object { fields, .. } => {
                // Metadata retains no scalar proof here. Its producers still
                // run in order and must satisfy their own program contracts.
                for field in fields {
                    self.child(field, state, depth)?;
                }
            }
            K::Tuple(values)
            | K::AnonymousRecord { fields: values }
            | K::Record { fields: values, .. }
            | K::Variant { fields: values, .. } => {
                let mut fields = Vec::new();
                for value in values {
                    let mut evaluated = self.child(value, state, depth)?;
                    if matches!(value.ty, Term::Pointer(_)) && evaluated.pointer_target.is_none() {
                        evaluated.pointer_origins = self
                            .pointer_value_origins(&evaluated, state)?
                            .map(|targets| (state.frame, targets));
                    }
                    evaluated.evidence = self.prepare_copy(&evaluated, state)?;
                    fields.push(evaluated.snapshot(self.limits, 0)?);
                }
                result.fields = Some(fields);
                result.nominal_type = if matches!(expr.kind, K::Tuple(_))
                    || matches!(expr.ty, Term::Nominal { .. } | Term::Record(_))
                {
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
            K::AssociatedValue {
                interface,
                member,
                subject,
            } => {
                // Selection and required initialization belong to elaboration
                // and comptime invocation. Until completed, the program layer
                // knows only the checked value domain, not a predicate or the
                // implementation body's result.
                result = self.unknown_value(&expr.ty)?;
                let view = self.scalar_type_view(&expr.ty)?;
                self.limits.step(self.associated_scalars.len())?;
                let existing = self.associated_scalars.iter().find(
                    |((stored_interface, stored_member, stored_subject), _, _)| {
                        stored_interface == interface
                            && stored_member == member
                            && stored_subject == subject
                    },
                );
                let root = if let Some((_, domain, root)) = existing {
                    if domain != &view {
                        return Err(Reason::InvalidSignature);
                    }
                    Some(*root)
                } else if let Some(root) = self.result_root(&view)? {
                    for ty in std::iter::once(subject)
                        .chain(&interface.arguments)
                        .chain(std::iter::once(&view))
                    {
                        nir::type_computations::charge_term(ty, &mut self.limits.arithmetic.steps)
                            .map_err(|_| {
                                Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into())
                            })?;
                    }
                    self.associated_scalars.push((
                        (interface.clone(), *member, subject.clone()),
                        view.clone(),
                        root,
                    ));
                    Some(root)
                } else {
                    None
                };
                if let Some(root) = root {
                    // Associated constants retain identity across observations
                    // and call frames, without assuming an initializer result.
                    if state.facts.value(root).is_none() {
                        bind_value_place(state, root, &view, result, self.limits)?;
                    }
                    // This is an immutable declaration value, not caller
                    // storage that an opaque call may overwrite.
                    state.frozen.insert(root, view.clone());
                    result = self.local_value_at(state, root)?;
                }
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
            C::Equal | C::NotEqual if high_low.is_lt() || low_high.is_gt() => {
                Some(relation == C::NotEqual)
            }
            C::Equal | C::NotEqual if high_low.is_eq() && low_high.is_eq() => {
                Some(relation == C::Equal)
            }
            _ => None,
        };
        if let Some(truth) = truth {
            return Ok(Some(Condition::Bool(truth)));
        }
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
        let mut limits = Limits::default();
        let instances = finite_domains::integer_instances(function, &mut limits)?;
        for instance in instances
            .as_deref()
            .unwrap_or(std::slice::from_ref(function))
        {
            // An unspecified target is not the compiler host. A generic
            // platform case is portable only if its program obligations hold
            // at EVERY layout admitted by TargetData. These are private proof
            // copies; neither the source package target nor its literals change.
            let mut platform = false;
            if instances.is_some() && package.target.is_none() {
                nir::visit::expressions(instance, &mut limits.arithmetic.steps, |expr, _| {
                    platform |= matches!(
                        expr.ty,
                        Term::Primitive(nir::Primitive::Usize | nir::Primitive::Isize)
                    );
                    Ok(())
                })
                .map_err(|_| Error {
                    source: instance.body.source,
                    reason: Reason::Unsupported,
                })?;
            }
            if platform {
                let mut proof_package = package.clone();
                for width in (8..=128).step_by(8) {
                    proof_package.target = Some(nir::target::TargetData::new(width).unwrap());
                    let mut proof = instance.clone();
                    finite_domains::complete_literals(
                        std::slice::from_mut(&mut proof),
                        proof_package.target,
                        &mut limits,
                    )?;
                    validate_machine_function(&proof_package, &proof)?;
                }
            } else {
                let mut proof = instance.clone();
                if instances.is_some() {
                    finite_domains::complete_literals(
                        std::slice::from_mut(&mut proof),
                        package.target,
                        &mut limits,
                    )?;
                }
                validate_machine_function(package, &proof)?;
            }
        }
    }
    Ok(())
}

// Each universally checked body has the same bounded analysis budget as a
// monomorphic declaration. Type/layout enumeration is separately finite.
fn validate_machine_function(
    package: &nir::program::Package,
    function: &Function,
) -> Result<(), Error> {
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
            unbounded_integer: *ty == Term::Primitive(nir::Primitive::Integer),
            range: domain(ty, limits)?,
            nominal_type: if matches!(
                ty,
                Term::Optional(_) | Term::Nominal { .. } | Term::Record(_) | Term::Tuple(_)
            ) {
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
                Some(
                    fields
                        .iter()
                        .map(|(_, ty)| Self::unknown_at(ty, limits, depth + 1))
                        .collect::<Result<_, _>>()?,
                )
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
        self.pointer_origins = None;
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
        Ok(self.nominal_type.is_some() || self.unbounded_integer)
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
    fn copy_pointer_origins(
        &self,
        limits: &mut Limits,
    ) -> Result<Option<(u64, Vec<PointerOrigin>)>, Unknown> {
        if let Some((frame, targets)) = &self.pointer_origins {
            for target in targets {
                limits.step(target.path_len().saturating_add(1))?;
            }
            Ok(Some((*frame, targets.clone())))
        } else {
            Ok(self.copy_pointer_target(limits)?.map(|(frame, targets)| {
                (
                    frame,
                    targets.into_iter().map(PointerOrigin::Local).collect(),
                )
            }))
        }
    }
    fn snapshot(&self, limits: &mut Limits, depth: usize) -> Result<Self, Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        Ok(Self {
            unbounded_integer: self.unbounded_integer,
            pointer_target: self.copy_pointer_target(limits)?,
            pointer_origins: self.copy_pointer_origins(limits)?,
            projection: None,
            immutable_projection: None,
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
        for alias in [&self.immutable_projection, &other.immutable_projection] {
            if let Some((_, projection)) = alias {
                limits.step(projection.path.fields.len().saturating_add(1))?;
            }
        }
        Ok(Self {
            unbounded_integer: self.unbounded_integer && other.unbounded_integer,
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
            pointer_origins: match (
                self.copy_pointer_origins(limits)?,
                other.copy_pointer_origins(limits)?,
            ) {
                (Some((a, mut targets)), Some((b, others))) if a == b => {
                    for target in others {
                        insert_pointer_origin(&mut targets, target, limits)?;
                    }
                    Some((a, targets))
                }
                _ => None,
            },
            projection: None,
            immutable_projection: if self.immutable_projection == other.immutable_projection {
                self.immutable_projection.clone()
            } else {
                None
            },
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
fn insert_pointer_origin(
    targets: &mut Vec<PointerOrigin>,
    target: PointerOrigin,
    limits: &mut Limits,
) -> Result<(), Unknown> {
    for previous in targets.iter() {
        limits.step(previous.path_len().min(target.path_len()).saturating_add(1))?;
        let same = match (previous, &target) {
            (PointerOrigin::Local(a), PointerOrigin::Local(b)) => a == b,
            (PointerOrigin::External(a), PointerOrigin::External(b)) => {
                a.root_version == b.root_version && a.path == b.path
            }
            _ => false,
        };
        if same {
            return Ok(());
        }
    }
    targets.push(target);
    Ok(())
}
fn copy_aggregates(
    aggregates: &BTreeMap<PlaceId, Value>,
    limits: &mut Limits,
) -> Result<BTreeMap<PlaceId, Value>, Unknown> {
    let mut result = BTreeMap::new();
    for (place, value) in aggregates {
        limits.step(1)?;
        let mut snapshot = value.snapshot(limits, 0)?;
        if let Some((frame, projection)) = &value.immutable_projection {
            limits.step(projection.path.fields.len().saturating_add(1))?;
            snapshot.immutable_projection = Some((*frame, projection.clone()));
        }
        result.insert(*place, snapshot);
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
        || value.pointer_origins.is_some()
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
        write!(
            f,
            "(range-value (unbounded-integer {}) (pointer-target ",
            self.unbounded_integer
        )?;
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
        f.write_str(") (pointer-origins ")?;
        match &self.pointer_origins {
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
        f.write_str(") (immutable-projection ")?;
        match &self.immutable_projection {
            Some((frame, projection)) => write!(f, "(frame {frame}) {projection}")?,
            None => f.write_str("none")?,
        }
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
    fn set_pointer_origins(
        &mut self,
        frame: u64,
        origins: Vec<PointerOrigin>,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        let mut local = Vec::new();
        let mut all_local = !origins.is_empty();
        for origin in &origins {
            limits.step(origin.path_len().saturating_add(1))?;
            match origin {
                PointerOrigin::Local(place) => local.push(place.clone()),
                PointerOrigin::External(_) => all_local = false,
            }
        }
        // Selective writes require the complete set, never a local subset of
        // a mixed result. Reads retain the full provenance in either case.
        self.pointer_target = all_local.then_some((frame, local));
        self.pointer_origins = Some((frame, origins));
        Ok(())
    }

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
            Some(Scalar::Local(id)) if self.range.is_some() || self.unbounded_integer => {
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
    fn refine_polynomial(
        &mut self,
        expr: &Expr,
        value: &mut Value,
        state: &State,
    ) -> Result<(), Reason> {
        if !state.facts.is_reachable() {
            return Ok(());
        }
        let Some(p) = &value.polynomial else {
            return Ok(());
        };
        let enclosed =
            if let Some(primitive) = nir::program::machine_integer_type_in(expr, self.target()) {
                let (signed, width) = self.integer_layout(primitive).unwrap();
                let target = Interval::machine(width as u16, signed, self.limits)?;
                p.enclosure_for_target(&target, &state.facts, self.limits)
            } else {
                p.enclosure(&state.facts, self.limits)
            };
        let range = match enclosed {
            Ok((range, _)) => range,
            Err(Unknown::MissingBound(_)) if state.facts.relation_is_current(p, self.limits)? => {
                // No finite enclosure is not loss of the expression identity.
                // It remains useful for exact program-contract substitution.
                return Ok(());
            }
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
fn fixed_pointer_load_depth(
    mut pointer: &Expr,
    limits: &mut Limits,
) -> Result<Option<usize>, Unknown> {
    let mut loads = 0;
    for _ in 0..128 {
        limits.step(1)?;
        pointer = match &pointer.kind {
            K::Local(_) => return Ok(Some(loads)),
            K::Field { value, .. } | K::AnonymousField { value, .. } => value,
            K::PointerField { pointer, .. } | K::PointerAnonymousField { pointer, .. } => pointer,
            K::ReadPointer(pointer) => {
                loads += 1;
                pointer
            }
            _ => return Ok(None),
        };
    }
    Err(Unknown::ConditionDepthLimit)
}

fn owned_pointer_input_path(
    mut pointer: &Expr,
    limits: &mut Limits,
) -> Result<Vec<usize>, Unknown> {
    let mut fields = Vec::new();
    loop {
        limits.step(1)?;
        match &pointer.kind {
            K::PointerField { pointer: base, .. }
            | K::PointerAnonymousField { pointer: base, .. } => pointer = base,
            K::Field { value, index, .. } | K::AnonymousField { value, index, .. } => {
                fields.push(*index);
                pointer = value;
            }
            _ => {
                fields.reverse();
                return Ok(fields);
            }
        }
    }
}

fn external_read_path(
    mut pointer: &Expr,
    limits: &mut Limits,
) -> Result<Option<FieldPath>, Unknown> {
    let mut fields = Vec::new();
    loop {
        limits.step(1)?;
        match &pointer.kind {
            K::Local(address) => {
                fields.reverse();
                return Ok(Some(FieldPath {
                    root: place(*address),
                    fields: record_path(&fields),
                }));
            }
            K::PointerField {
                pointer: base,
                declaration,
                index,
            }
            | K::Field {
                value: base,
                declaration,
                index,
            } => {
                fields.push((nir::program::FieldOwner::Nominal(*declaration), *index));
                pointer = base;
            }
            K::PointerAnonymousField {
                pointer: base,
                index,
                ..
            }
            | K::AnonymousField {
                value: base, index, ..
            } => {
                fields.push((nir::program::FieldOwner::Anonymous, *index));
                pointer = base;
            }
            _ => return Ok(None),
        }
    }
}

// Resolve only already imported observations. In particular, traversing a
// pointer load must not invent a new ghost or evaluate a source expression.
fn observed_pointer_path(
    pointer: &Expr,
    reads: &BTreeMap<FieldPath, (Term, PlaceId)>,
    limits: &mut Limits,
    depth: usize,
) -> Result<Option<FieldPath>, Unknown> {
    limits.step(1)?;
    if depth >= 128 {
        return Err(Unknown::ConditionDepthLimit);
    }
    if let Some(path) = external_read_path(pointer, limits)? {
        return Ok(Some(path));
    }
    let (base, field) = match &pointer.kind {
        K::ReadPointer(base) => {
            let Some(path) = observed_pointer_path(base, reads, limits, depth + 1)? else {
                return Ok(None);
            };
            return Ok(reads.get(&path).and_then(|(ty, root)| {
                matches!(ty, Term::Pointer(_)).then(|| FieldPath {
                    root: *root,
                    fields: Vec::new(),
                })
            }));
        }
        K::PointerField {
            pointer,
            declaration,
            index,
        } => (pointer, (AggregateKind::Record(*declaration), *index)),
        K::PointerAnonymousField { pointer, index, .. } => {
            (pointer, (AggregateKind::AnonymousRecord, *index))
        }
        _ => return Ok(None),
    };
    let Some(mut path) = observed_pointer_path(base, reads, limits, depth + 1)? else {
        return Ok(None);
    };
    path.fields.push(field);
    Ok(Some(path))
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
                let Some(reads) = reads else {
                    return Ok(false);
                };
                let Some(path) = observed_pointer_path(pointer, reads, limits, 0)? else {
                    return Ok(false);
                };
                if !reads.contains_key(&path) {
                    return Ok(false);
                }
            }
            K::Local(id) if Some(*id) == binding || snapshots.iter().any(|s| s.binding == *id) => {}
            K::Literal(_) | K::OptionalAbsent | K::Unit => {}
            // The selected associated constant has the same identity in the
            // callee's result contract and the caller's observation frame.
            // Its initializer is not executed by proof export.
            K::AssociatedValue { .. } => {}
            K::ComptimeValue(value) => {
                if nir::comptime_values::validate(value, &expr.ty, &mut limits.arithmetic.steps)
                    .is_err()
                {
                    return Ok(false);
                }
            }
            K::Not(a)
            | K::IntegerNegate(a)
            | K::FloatNegate(a)
            | K::AnonymousField { value: a, .. }
            | K::Field { value: a, .. }
            | K::BitSlice { value: a, .. }
            | K::Snapshot(a)
            | K::TakeBits(a)
            | K::TextQuery { subject: a, .. }
            | K::IntegerRepresentation(a)
            | K::SymbolName(a)
            | K::EnumName(a)
            | K::BitConvert(a)
            | K::UnsafeScope(a)
            | K::Discard(a) => work.push(a),
            K::IntegerBinary {
                left: a, right: b, ..
            }
            | K::FloatBinary {
                left: a, right: b, ..
            }
            | K::ScalarEqual(a, b)
            | K::TemplateEqual(a, b)
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
                        if lo_a == hi_a && lo_b == hi_b {
                            continue;
                        }
                        for relation in [
                            C::Equal,
                            C::NotEqual,
                            C::Less,
                            C::LessEqual,
                            C::Greater,
                            C::GreaterEqual,
                        ] {
                            let mut limits = Limits::default();
                            let value = |lo, hi, limits: &mut Limits| Value {
                                range: Some(
                                    Interval::new(
                                        Integer::from_i128(lo),
                                        Integer::from_i128(hi),
                                        limits,
                                    )
                                    .unwrap(),
                                ),
                                ..Value::default()
                            };
                            let a = value(lo_a, hi_a, &mut limits);
                            let b = value(lo_b, hi_b, &mut limits);
                            let decision = compare_values(a, b, relation, &mut limits).unwrap();
                            let observations = (lo_a..=hi_a)
                                .flat_map(|a| {
                                    (lo_b..=hi_b).map(move |b| match relation {
                                        C::Equal => a == b,
                                        C::NotEqual => a != b,
                                        C::Less => a < b,
                                        C::LessEqual => a <= b,
                                        C::Greater => a > b,
                                        C::GreaterEqual => a >= b,
                                    })
                                })
                                .collect::<Vec<_>>();
                            let expected = observations
                                .iter()
                                .all(|truth| *truth == observations[0])
                                .then_some(observations[0]);
                            let actual = match decision {
                                Some(Condition::Bool(truth)) => Some(truth),
                                None => None,
                                _ => panic!("unexpected symbolic identity"),
                            };
                            assert_eq!(
                                actual, expected,
                                "[{lo_a},{hi_a}] {relation:?} [{lo_b},{hi_b}]"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod memory_summary_cache_tests {
    use super::*;

    #[test]
    fn budget_failure_can_retry_and_completed_summary_is_reused() {
        let declaration = nir::DeclarationId {
            package: 0,
            local: 1,
        };
        let unit = || Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: Term::Primitive(nir::Primitive::Void),
            kind: K::Unit,
        };
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(unit().ty),
        };
        let package = nir::program::Package {
            functions: vec![Function {
                phase: nir::program::FunctionPhase::Runtime,
                inline: false,
                pure: false,
                is_unsafe: false,
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![],
                declaration,
                name: "work".into(),
                signature: signature.clone(),
                parameters: vec![],
                body: Expr {
                    kind: K::Block {
                        statements: (0..100).map(|_| unit()).collect(),
                        result: Some(Box::new(unit())),
                    },
                    ..unit()
                },
                defaults: vec![],
            }],
            ..Default::default()
        };
        let callee = Expr {
            ty: signature,
            kind: K::Function(declaration),
            ..unit()
        };
        let mut limits = Limits::default();
        limits.arithmetic.steps = 200;
        let mut walker = Walker {
            addressed_locals: BTreeSet::new(),
            handler_unstable_locals: None,
            written_locals: BTreeSet::new(),
            type_context: crate::ptc::Context::with_scope(vec![]),
            input_frames: vec![],
            next_frame: 1,
            memory_summaries: Default::default(),
            exit_contract: None,
            assuming_postcondition: false,
            next_field: 0,
            captured_values: BTreeMap::new(),
            associated_scalars: Vec::new(),
            limits: &mut limits,
            package: Some(&package),
            points: vec![],
            failure_source: None,
            closure_context: vec![],
            single_pass_exits: BTreeMap::new(),
        };
        assert!(walker.call_summary(&callee).is_none());
        assert!(walker.limits.arithmetic.steps < 200);
        assert!(walker.memory_summaries.is_empty());

        // Retry the same instance with enough resources; partial fixed-point
        // state from the failed attempt must not stand in for its full body.
        walker.limits.arithmetic.steps = 100_000;
        let summary = walker.call_summary(&callee).expect("retry succeeds");
        assert!(summary.accesses.is_empty());
        assert_eq!(walker.memory_summaries.len(), 1);
        walker.limits.arithmetic.steps = 200;
        let cached = walker.call_summary(&callee).expect("completed cache hit");
        assert!(std::sync::Arc::ptr_eq(&summary, &cached));

        let incompatible = Expr {
            ty: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(nir::Primitive::Bool)),
            },
            ..callee.clone()
        };
        assert!(walker.call_summary(&incompatible).is_none());
        assert!(walker.call_summary(&callee).is_some());
    }
}

fn invariant_slot_targets(
    source: &nir::program::Place,
    state: &State,
    changed: &ChangedPlaces,
    fuel: &mut usize,
) -> Option<Vec<nir::program::Place>> {
    invariant_storage_unchanged(source, changed, true, fuel)?;
    let mut value = state.aggregates.get(&place(source.root))?;
    for (owner, index) in &source.fields {
        *fuel = fuel.checked_sub(1)?;
        if !storage_owner_matches(value, *owner) {
            return None;
        }
        value = value.fields.as_ref()?.get(*index)?;
    }
    let (frame, targets) = value.pointer_target.as_ref()?;
    if *frame != state.frame || targets.is_empty() {
        return None;
    }
    for target in targets {
        *fuel = fuel.checked_sub(target.fields.len().saturating_add(1))?;
    }
    Some(targets.clone())
}

// These projections consume only stored facts; they never synthesize a tag or
// reinterpret one variant's physical payload as another variant's value.
fn snapshot_fields<'a>(
    mut value: &'a Value,
    fields: &[(nir::program::FieldOwner, usize)],
    limits: &mut Limits,
) -> Result<Option<&'a Value>, Reason> {
    for (owner, index) in fields {
        limits.step(1)?;
        if !snapshot_owner_matches(value, *owner) {
            return Ok(None);
        }
        let Some(next) = value.fields.as_ref().and_then(|fields| fields.get(*index)) else {
            return Ok(None);
        };
        value = next;
    }
    Ok(Some(value))
}

fn snapshot_indices<'a>(
    mut value: &'a Value,
    fields: &[usize],
    limits: &mut Limits,
) -> Result<Option<&'a Value>, Reason> {
    for index in fields {
        limits.step(1)?;
        let Some(next) = value.fields.as_ref().and_then(|fields| fields.get(*index)) else {
            return Ok(None);
        };
        value = next;
    }
    Ok(Some(value))
}

#[cfg(test)]
mod conditional_origin_tests {
    use super::*;

    #[test]
    fn conditional_results_follow_current_tags_and_forget_invalidated_snapshots() {
        let mut limits = Limits::default();
        let mut walker = Walker {
            addressed_locals: BTreeSet::new(),
            handler_unstable_locals: None,
            written_locals: BTreeSet::new(),
            type_context: crate::ptc::Context::with_scope(vec![]),
            input_frames: vec![],
            next_frame: 1,
            memory_summaries: Default::default(),
            exit_contract: None,
            assuming_postcondition: false,
            next_field: 0,
            captured_values: BTreeMap::new(),
            associated_scalars: vec![],
            limits: &mut limits,
            package: None,
            points: vec![],
            failure_source: None,
            closure_context: vec![],
            single_pass_exits: BTreeMap::new(),
        };
        let declaration = nir::DeclarationId {
            package: 0,
            local: 1,
        };
        let root = LocalId { owner: 0, index: 1 };
        let output = nir::program::Place {
            root: LocalId { owner: 0, index: 2 },
            fields: vec![],
        };
        let input = PointerOrigin::Local(nir::program::Place {
            root,
            fields: vec![],
        });
        let target = super::super::memory::ParameterPlace {
            parameter: 0,
            input_fields: vec![],
            through: vec![vec![(nir::program::FieldOwner::Nominal(declaration), 0)]],
            fields: vec![],
            conditions: BTreeMap::from([(0, vec![(vec![], 1)])]),
        };
        let mut state = State {
            target: None,
            frame: 0,
            facts: Facts::default(),
            domains: BTreeMap::new(),
            aggregates: BTreeMap::new(),
            paths: BTreeMap::new(),
            frozen: BTreeMap::new(),
            reads: BTreeMap::new(),
            read_aliases: BTreeMap::new(),
        };
        let snapshot = |tag, frame| Value {
            aggregate_kind: Some(AggregateKind::Variant(declaration, tag)),
            fields: Some(vec![Value {
                pointer_target: Some((frame, vec![output.clone()])),
                ..Value::default()
            }]),
            ..Value::default()
        };
        state
            .aggregates
            .insert(place(root), snapshot(1, state.frame));
        let arguments = BTreeMap::from([((0, vec![]), vec![input.clone()])]);
        let targets = BTreeSet::from([target.clone()]);
        let resolve = |walker: &mut Walker<'_>, state: &mut State| {
            walker.summary_pointer_origins(&[], false, false, Some(&targets), &arguments, state)
        };
        let found = resolve(&mut walker, &mut state).unwrap().unwrap();
        assert!(matches!(found.as_slice(), [PointerOrigin::Local(actual)] if *actual == output));
        state
            .aggregates
            .insert(place(root), snapshot(0, state.frame));
        assert!(resolve(&mut walker, &mut state).unwrap().is_none());
        state.aggregates.insert(place(root), Value::default());
        assert!(resolve(&mut walker, &mut state).unwrap().is_none());
        state
            .aggregates
            .insert(place(root), snapshot(1, state.frame + 1));
        assert!(resolve(&mut walker, &mut state).unwrap().is_none());
        state.aggregates.clear();
        assert!(resolve(&mut walker, &mut state).unwrap().is_none());
        state
            .aggregates
            .insert(place(root), snapshot(1, state.frame));
        let write = super::super::memory::Access {
            target: target.clone(),
            write: true,
            read_value: false,
            ty: Term::Primitive(nir::Primitive::Bool),
        };
        assert!(
            walker
                .summary_pointer_origins(
                    &[write.clone()],
                    false,
                    false,
                    Some(&targets),
                    &arguments,
                    &mut state
                )
                .unwrap()
                .is_none()
        );
        assert!(
            walker
                .summary_pointer_origins(&[], true, true, Some(&targets), &arguments, &mut state)
                .unwrap()
                .is_none()
        );
        let preserved = walker
            .summary_pointer_origins(
                &[write],
                false,
                true,
                Some(&targets),
                &arguments,
                &mut state,
            )
            .unwrap()
            .unwrap();
        assert!(
            matches!(preserved.as_slice(), [PointerOrigin::Local(actual)] if *actual == output)
        );
        // The second guard belongs to the loaded root, not the initial enum.
        let final_place = nir::program::Place {
            root: LocalId { owner: 0, index: 3 },
            fields: vec![],
        };
        let mut nested = target.clone();
        nested.through.push(target.through[0].clone());
        nested.conditions.insert(1, vec![(vec![], 0)]);
        state.aggregates.insert(
            place(output.root),
            Value {
                aggregate_kind: Some(AggregateKind::Variant(declaration, 0)),
                fields: Some(vec![Value {
                    pointer_target: Some((state.frame, vec![final_place.clone()])),
                    ..Value::default()
                }]),
                ..Value::default()
            },
        );
        let found = walker
            .conditional_pointer_origins(&nested, &[input.clone()], &state)
            .unwrap()
            .unwrap();
        assert!(
            matches!(found.as_slice(), [PointerOrigin::Local(actual)] if *actual == final_place)
        );
        state
            .aggregates
            .get_mut(&place(output.root))
            .unwrap()
            .aggregate_kind = Some(AggregateKind::Variant(declaration, 1));
        assert!(
            walker
                .conditional_pointer_origins(&nested, &[input.clone()], &state)
                .unwrap()
                .unwrap()
                .is_empty()
        );
        let base = nir::program::Place {
            root,
            fields: vec![],
        };
        let mut fuel = 100_000;
        let stable = invariant_conditional_targets(
            &target,
            &[base.clone()],
            &state,
            &BTreeSet::new(),
            &mut fuel,
        )
        .unwrap();
        assert_eq!(stable, vec![output.clone()]);
        // Writes through the result affect values, not the source slot/tag.
        let destination_write = BTreeSet::from([(output.root, vec![])]);
        assert!(
            invariant_conditional_targets(
                &target,
                &[base.clone()],
                &state,
                &destination_write,
                &mut fuel
            )
            .is_some()
        );
        let owner_write = BTreeSet::from([(root, vec![])]);
        assert!(
            invariant_conditional_targets(
                &target,
                &[base.clone()],
                &state,
                &owner_write,
                &mut fuel
            )
            .is_none()
        );
        // In the two-load path the same location is an intermediate slot,
        // so changing it invalidates the address computation.
        assert!(
            invariant_conditional_targets(
                &nested,
                &[base.clone()],
                &state,
                &destination_write,
                &mut fuel
            )
            .is_none()
        );
        assert!(
            invariant_conditional_targets(&target, &[base], &state, &BTreeSet::new(), &mut 0)
                .is_none()
        );
        // A write to another payload field changes neither the tag nor
        // the pointer slot used to find the destination.
        state
            .aggregates
            .get_mut(&place(root))
            .unwrap()
            .fields
            .as_mut()
            .unwrap()
            .push(Value::default());
        let sibling = BTreeSet::from([(
            root,
            vec![(nir::program::FieldOwner::Nominal(declaration), 1)],
        )]);
        assert!(
            invariant_conditional_targets(
                &target,
                &[nir::program::Place {
                    root,
                    fields: vec![]
                }],
                &state,
                &sibling,
                &mut fuel
            )
            .is_some()
        );
        let slot_write = BTreeSet::from([(root, target.through[0].clone())]);
        assert!(
            invariant_conditional_targets(
                &target,
                &[nir::program::Place {
                    root,
                    fields: vec![]
                }],
                &state,
                &slot_write,
                &mut fuel
            )
            .is_none()
        );
        let mut invalid = target;
        invalid.conditions.insert(2, vec![(vec![], 1)]);
        assert!(
            walker
                .conditional_pointer_origins(&invalid, &[input], &state)
                .unwrap()
                .is_none()
        );
        walker.limits.arithmetic.steps = 0;
        assert!(resolve(&mut walker, &mut state).is_err());
    }
}

// A conditional loop path may use only tags and pointer slots that survive
// every possible loop write. The final destination itself need not be stable.
fn invariant_conditional_targets(
    target: &super::memory::ParameterPlace,
    bases: &[nir::program::Place],
    state: &State,
    changed: &ChangedPlaces,
    fuel: &mut usize,
) -> Option<Vec<nir::program::Place>> {
    *fuel = fuel.checked_sub(target.conditions.len().saturating_add(1))?;
    if target
        .conditions
        .keys()
        .any(|stage| *stage > target.through.len())
    {
        return None;
    }
    for base in bases {
        *fuel = fuel.checked_sub(base.fields.len().saturating_add(1))?;
    }
    let mut current = bases.to_vec();
    for stage in 0..=target.through.len() {
        let mut next = Vec::new();
        for mut base in current {
            *fuel = fuel.checked_sub(1)?;
            // With no remaining load or guard, this is an address, not a
            // dependence on the destination's stored value.
            if stage == target.through.len() && !target.conditions.contains_key(&stage) {
                *fuel = fuel.checked_sub(target.fields.len())?;
                base.fields.extend_from_slice(&target.fields);
                insert_pointer_target(&mut next, base, fuel).ok()?;
                continue;
            }
            let mut value = state.aggregates.get(&place(base.root))?;
            for (owner, index) in &base.fields {
                *fuel = fuel.checked_sub(1)?;
                if !snapshot_owner_matches(value, *owner) {
                    return None;
                }
                value = value.fields.as_ref()?.get(*index)?;
            }
            let mut active = true;
            if let Some(guards) = target.conditions.get(&stage) {
                for (fields, expected) in guards {
                    *fuel = fuel.checked_sub(fields.len().saturating_add(1))?;
                    *fuel = fuel.checked_sub(base.fields.len().saturating_add(1))?;
                    let mut container = base.clone();
                    let mut guarded = value;
                    for index in fields {
                        let owner = match guarded.aggregate_kind? {
                            AggregateKind::Record(id) | AggregateKind::Variant(id, _) => {
                                nir::program::FieldOwner::Nominal(id)
                            }
                            AggregateKind::AnonymousRecord | AggregateKind::Tuple => {
                                nir::program::FieldOwner::Anonymous
                            }
                            AggregateKind::EnumTag(_)
                            | AggregateKind::Optional(_)
                            | AggregateKind::OptionalTag => return None,
                        };
                        container.fields.push((owner, *index));
                        guarded = guarded.fields.as_ref()?.get(*index)?;
                    }
                    // A payload-field write cannot change this container's
                    // tag. Replacing the container or an ancestor can.
                    invariant_storage_unchanged(&container, changed, false, fuel)?;
                    match guarded.aggregate_kind {
                        Some(AggregateKind::Variant(_, tag)) if tag == *expected => {}
                        Some(AggregateKind::Variant(_, _)) => {
                            active = false;
                            break;
                        }
                        _ => return None,
                    }
                }
            }
            if !active {
                continue;
            }
            if stage == target.through.len() {
                *fuel = fuel.checked_sub(target.fields.len())?;
                base.fields.extend_from_slice(&target.fields);
                insert_pointer_target(&mut next, base, fuel).ok()?;
            } else {
                *fuel = fuel.checked_sub(
                    base.fields
                        .len()
                        .saturating_add(target.through[stage].len())
                        .saturating_add(1),
                )?;
                let mut slot = base.clone();
                slot.fields.extend_from_slice(&target.through[stage]);
                invariant_storage_unchanged(&slot, changed, true, fuel)?;
                for (owner, index) in &target.through[stage] {
                    *fuel = fuel.checked_sub(1)?;
                    if !snapshot_owner_matches(value, *owner) {
                        return None;
                    }
                    value = value.fields.as_ref()?.get(*index)?;
                }
                let (frame, targets) = value.pointer_target.as_ref()?;
                if *frame != state.frame || targets.is_empty() {
                    return None;
                }
                for target in targets {
                    *fuel = fuel.checked_sub(target.fields.len().saturating_add(1))?;
                    insert_pointer_target(&mut next, target.clone(), fuel).ok()?;
                }
            }
        }
        current = next;
    }
    Some(current)
}

fn snapshot_owner_matches(value: &Value, owner: nir::program::FieldOwner) -> bool {
    match (owner, value.aggregate_kind) {
        (nir::program::FieldOwner::Anonymous, Some(AggregateKind::Tuple)) => true,
        (nir::program::FieldOwner::Nominal(expected), Some(AggregateKind::Variant(actual, _))) => {
            expected == actual
        }
        _ => value.aggregate_kind == Some(field_owner_kind(owner)),
    }
}

fn invariant_storage_unchanged(
    source: &nir::program::Place,
    changed: &ChangedPlaces,
    descendants_matter: bool,
    fuel: &mut usize,
) -> Option<()> {
    for (root, fields) in changed {
        *fuel = fuel.checked_sub(
            fields
                .len()
                .saturating_add(source.fields.len())
                .saturating_add(1),
        )?;
        if *root == source.root
            && (source.fields.starts_with(fields)
                || (descendants_matter && fields.starts_with(&source.fields)))
        {
            return None;
        }
    }
    Some(())
}

#[cfg(test)]
mod tuple_storage_tests {
    use super::*;
    #[test]
    fn tuple_pointer_reads_writes_keep_paths_and_opaque_identity() {
        let head = nir::DeclarationId {
            package: 0,
            local: 99,
        };
        let package = nir::program::Package {
            normal_forms: vec![nir::normal_forms::Head {
                role: nir::normal_forms::Role::Tuple,
                declaration: head,
                parameters: vec![],
                requirements: vec![],
            }],
            ..Default::default()
        };
        let byte = Term::Primitive(nir::Primitive::U8);
        let fields = vec![
            byte.clone(),
            Term::Tuple(vec![Term::Pointer(Box::new(byte.clone()))]),
        ];
        for ty in [
            Term::Tuple(fields.clone()),
            Term::NormalForm {
                head,
                sort: nir::Sort::Type,
                arguments: vec![Term::TypeListValue(fields.clone())],
            },
        ] {
            let mut limits = Limits::default();
            let mut walker = Walker {
                addressed_locals: BTreeSet::new(),
                handler_unstable_locals: None,
                written_locals: BTreeSet::new(),
                type_context: crate::ptc::Context::with_scope(vec![]),
                input_frames: vec![],
                next_frame: 1,
                memory_summaries: Default::default(),
                exit_contract: None,
                assuming_postcondition: false,
                next_field: 0,
                captured_values: BTreeMap::new(),
                associated_scalars: vec![],
                limits: &mut limits,
                package: Some(&package),
                points: vec![],
                failure_source: None,
                closure_context: vec![],
                single_pass_exits: BTreeMap::new(),
            };
            let opaque = Term::NormalForm {
                head: nir::DeclarationId {
                    package: 0,
                    local: 100,
                },
                sort: nir::Sort::Type,
                arguments: vec![Term::TypeListValue(fields.clone())],
            };
            assert!(
                walker
                    .place_fields(&opaque, nir::program::FieldOwner::Anonymous)
                    .is_err()
            );
            let mut state = State {
                target: None,
                frame: 0,
                facts: Facts::default(),
                domains: BTreeMap::new(),
                aggregates: BTreeMap::new(),
                paths: BTreeMap::new(),
                frozen: BTreeMap::new(),
                reads: BTreeMap::new(),
                read_aliases: BTreeMap::new(),
            };
            let closed_fields = vec![byte.clone(), Term::Primitive(nir::Primitive::Bool)];
            let closed_ty = if matches!(ty, Term::NormalForm { .. }) {
                Term::NormalForm {
                    head,
                    sort: nir::Sort::Type,
                    arguments: vec![Term::TypeListValue(closed_fields)],
                }
            } else {
                Term::Tuple(closed_fields)
            };
            let closed =
                Term::TupleValue(vec![Term::Integer(Integer::from_u128(7)), Term::Bool(true)]);
            let source = SourceRange { start: 0, end: 1 };
            let completed = walker
                .completed_aggregate_value(&closed, &closed_ty, source, &mut state, 0)
                .unwrap();
            assert_eq!(completed.nominal_type.as_deref(), Some(&closed_ty));
            let values = completed.fields.unwrap();
            assert_eq!(
                values[0].range.as_ref().unwrap().upper(),
                &Integer::from_u128(7)
            );
            assert!(matches!(values[1].condition, Some(Condition::Bool(true))));
            for malformed in [
                Term::TupleValue(vec![]),
                Term::TupleValue(vec![
                    Term::Integer(Integer::from_u128(256)),
                    Term::Bool(true),
                ]),
                Term::TupleValue(vec![Term::Bool(true), Term::Bool(true)]),
            ] {
                assert!(
                    walker
                        .completed_aggregate_value(&malformed, &closed_ty, source, &mut state, 0)
                        .is_err()
                );
            }
            assert!(
                walker
                    .completed_aggregate_value(&closed, &closed_ty, source, &mut state, 128)
                    .is_err()
            );
            let mixed_ty = Term::Tuple(vec![byte.clone(), Term::Primitive(nir::Primitive::Type)]);
            let mixed = Term::TupleValue(vec![
                Term::Integer(Integer::from_u128(11)),
                Term::Primitive(nir::Primitive::Bool),
            ]);
            let mixed_value = walker
                .completed_aggregate_value(&mixed, &mixed_ty, source, &mut state, 0)
                .unwrap();
            assert_eq!(
                mixed_value.fields.as_ref().unwrap()[0]
                    .range
                    .as_ref()
                    .unwrap()
                    .upper(),
                &Integer::from_u128(11)
            );
            assert!(mixed_value.fields.as_ref().unwrap()[1].range.is_none());
            let record_ty = Term::Record(vec![
                ("count".into(), byte.clone()),
                ("flag".into(), Term::Primitive(nir::Primitive::Bool)),
            ]);
            let record = Term::RecordValue(vec![
                ("count".into(), Term::Integer(Integer::from_u128(13))),
                ("flag".into(), Term::Bool(true)),
            ]);
            let record_value = walker
                .completed_aggregate_value(&record, &record_ty, source, &mut state, 0)
                .unwrap();
            assert!(matches!(
                record_value.aggregate_kind,
                Some(AggregateKind::AnonymousRecord)
            ));
            assert_eq!(
                record_value.fields.as_ref().unwrap()[0]
                    .range
                    .as_ref()
                    .unwrap()
                    .upper(),
                &Integer::from_u128(13)
            );
            for malformed in [
                Term::RecordValue(vec![]),
                Term::RecordValue(vec![
                    ("wrong".into(), Term::Integer(Integer::from_u128(13))),
                    ("flag".into(), Term::Bool(true)),
                ]),
                Term::RecordValue(vec![
                    ("flag".into(), Term::Bool(true)),
                    ("count".into(), Term::Integer(Integer::from_u128(13))),
                ]),
            ] {
                assert!(
                    walker
                        .completed_aggregate_value(&malformed, &record_ty, source, &mut state, 0)
                        .is_err()
                );
            }
            let root = LocalId { owner: 0, index: 0 };
            let scalar = nir::program::Place {
                root,
                fields: vec![(nir::program::FieldOwner::Anonymous, 0)],
            };
            let slot = nir::program::Place {
                root,
                fields: vec![
                    (nir::program::FieldOwner::Anonymous, 1),
                    (nir::program::FieldOwner::Anonymous, 0),
                ],
            };
            let destination = nir::program::Place {
                root: LocalId { owner: 0, index: 2 },
                fields: vec![],
            };
            let mut value = walker.unknown_value(&ty).unwrap();
            value.fields.as_mut().unwrap()[0].range = Some(
                Interval::new(Integer::from_u128(7), Integer::from_u128(7), walker.limits).unwrap(),
            );
            state.aggregates.insert(place(root), value);
            state.facts.invalidate(place(root)).unwrap();
            {
                let mut aliases = state.fork(walker.limits).unwrap();
                aliases.frozen.insert(place(root), ty.clone());
                let alias = LocalId {
                    owner: 0,
                    index: 90,
                };
                let unknown = walker.unknown_value(&ty).unwrap();
                bind_value(&mut aliases, alias, &ty, unknown, walker.limits).unwrap();
                let projection = Projection {
                    root_version: aliases.facts.value(place(root)).unwrap(),
                    path: FieldPath {
                        root: place(root),
                        fields: vec![],
                    },
                };
                let stored = aliases.aggregates.get_mut(&place(alias)).unwrap();
                stored.immutable_projection = Some((aliases.frame, projection));
                assert!(stored.to_string().contains("(immutable-projection (frame"));
                assert!(
                    stored
                        .snapshot(walker.limits, 0)
                        .unwrap()
                        .immutable_projection
                        .is_none()
                );
                let mut branch = aliases.fork(walker.limits).unwrap();
                assert_eq!(
                    walker
                        .local_value(&branch, alias)
                        .unwrap()
                        .projection
                        .unwrap()
                        .path
                        .root,
                    place(root)
                );
                branch.frame += 1;
                assert_eq!(
                    walker
                        .local_value(&branch, alias)
                        .unwrap()
                        .projection
                        .unwrap()
                        .path
                        .root,
                    place(alias)
                );
                aliases.facts.invalidate(place(root)).unwrap();
                assert_eq!(
                    walker
                        .local_value(&aliases, alias)
                        .unwrap()
                        .projection
                        .unwrap()
                        .path
                        .root,
                    place(alias)
                );
            }
            let saved = walker.read_pointer_place(&mut state, &scalar).unwrap();
            let mut replacement = walker.unknown_value(&byte).unwrap();
            replacement.range = Some(
                Interval::new(Integer::from_u128(9), Integer::from_u128(9), walker.limits).unwrap(),
            );
            walker
                .write_field(&mut state, &scalar, replacement)
                .unwrap();
            assert_eq!(saved.range.unwrap().upper(), &Integer::from_u128(7));
            assert_eq!(
                walker
                    .read_pointer_place(&mut state, &scalar)
                    .unwrap()
                    .range
                    .unwrap()
                    .upper(),
                &Integer::from_u128(9)
            );
            walker
                .write_field(
                    &mut state,
                    &slot,
                    Value {
                        pointer_target: Some((0, vec![destination.clone()])),
                        ..Value::default()
                    },
                )
                .unwrap();
            let loaded = walker.read_pointer_place(&mut state, &slot).unwrap();
            assert_eq!(loaded.pointer_target.unwrap().1, vec![destination.clone()]);
            assert_eq!(
                invariant_slot_targets(&slot, &state, &BTreeSet::new(), &mut 10000).unwrap(),
                vec![destination]
            );
            assert_eq!(
                state.aggregates[&place(root)].nominal_type.as_deref(),
                Some(&ty)
            );
            let invalid = nir::program::Place {
                root,
                fields: vec![(nir::program::FieldOwner::Anonymous, 2)],
            };
            assert!(
                walker
                    .write_field(&mut state, &invalid, Value::default())
                    .is_err()
            );
            assert_eq!(
                walker
                    .read_pointer_place(&mut state, &scalar)
                    .unwrap()
                    .range
                    .unwrap()
                    .upper(),
                &Integer::from_u128(9)
            );
        }
    }
}

// Unguarded physical paths do not gain access to variant payloads merely
// because a snapshot happens to carry a particular constructor.
fn storage_owner_matches(value: &Value, owner: nir::program::FieldOwner) -> bool {
    matches!(
        (owner, value.aggregate_kind),
        (
            nir::program::FieldOwner::Anonymous,
            Some(AggregateKind::Tuple)
        )
    ) || value.aggregate_kind == Some(field_owner_kind(owner))
}
