//! Ordered owned-position move analysis. This pass reports cleanup obligations;
//! it neither executes Drop nor proves Copy representation/alias legality.
//! Unsupported operations fail explicitly instead of silently passing.
use super::pattern_facts::{constant_bool, irrefutable_shape, known_pattern, outer_guards};
use super::{
    PlaceId,
    ownership::{Availability, CleanupTree, Place, Shape, State},
};
use nir::{
    Term,
    program::{
        Expr, ExprKind as K, Function, FunctionPhase, LocalId, Package, Pattern, SourceRange,
    },
};
use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet},
    fmt,
};
mod borrowed_code;
use borrowed_code::borrowed_memory_inputs;
pub use borrowed_code::copy_borrowed_code;
mod cleanup_memory;
pub use cleanup_memory::BorrowedRoot;
mod effects;
mod origins;
mod patterns;
pub use effects::effect_result_slots;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Move,
    Copy,
}
#[derive(Debug)]
pub struct Error {
    pub source: SourceRange,
    pub reason: String,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(move-error (source {}) (reason {}))",
            self.source,
            Term::String(self.reason.clone().into())
        )
    }
}
impl std::error::Error for Error {}
#[derive(Debug)]
pub struct Cleanup {
    pub source: SourceRange,
    pub place: Place,
    pub availability: Availability,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemporaryBoundary {
    FullExpression,
    Condition,
    Match,
    ReturnValue,
    FunctionResult,
    DefaultArgument,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TemporaryScope {
    pub source: SourceRange,
    pub boundary: TemporaryBoundary,
}
impl fmt::Display for TemporaryScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.boundary {
            TemporaryBoundary::FullExpression => "full-expression",
            TemporaryBoundary::Condition => "condition",
            TemporaryBoundary::Match => "match",
            TemporaryBoundary::ReturnValue => "return-value",
            TemporaryBoundary::FunctionResult => "function-result",
            TemporaryBoundary::DefaultArgument => "default-argument",
        };
        write!(
            f,
            "(temporary-scope (kind {kind}) (source {}))",
            self.source
        )
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct TemporaryCleanup {
    pub scope: TemporaryScope,
    pub source: SourceRange,
    pub ty: Term,
    pub fields: Vec<usize>,
}
/// One completed component, before ownership reaches the enclosing operation.
#[derive(Clone, Debug)]
pub struct PendingValue {
    pub sequence: u64,
    pub source: SourceRange,
    pub scope: TemporaryScope,
    pub ty: Term,
    pub slot: usize,
    /// Lexical anchors at acquisition; a future cleanup planner must interleave
    /// local and pending responsibilities rather than concatenate report lists.
    pub locals_before: Vec<PlaceId>,
}
#[derive(Clone, Debug)]
struct PendingFrame {
    conditions: BTreeMap<u64, ValueConditions>,
    origins: BTreeMap<u64, ValueOrigins>,
    source: SourceRange,
    values: Vec<PendingValue>,
}
#[derive(Debug)]
pub struct IncompleteCleanup {
    pub exit: SourceRange,
    pub construction: SourceRange,
    /// Completed components only, in reverse acquisition order. No Drop of the
    /// unfinished enclosing object is implied.
    pub values: Vec<PendingValue>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupReason {
    /// Abandon this resource scope through an effect handler outside it.
    /// The selected installer uses its return/loop plan instead; resume never
    /// runs this checkpoint. Intermediate handler scopes may share an invocation.
    EffectEscape,
    /// Installer return after handler and interrupted invocations have unwound.
    EffectReturn,
    EffectBreak(LocalId),
    EffectContinue(LocalId),
    HandlerEnd,
    EffectResume,
    Return,
    FunctionEnd,
    PredicateFailure,
    Break,
    Continue,
    ScopeEnd,
    Replacement,
    TemporaryEnd,
}
impl fmt::Display for CleanupReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Self::EffectBreak(id) | Self::EffectContinue(id) = self {
            let kind = if matches!(self, Self::EffectBreak(_)) {
                "effect-break"
            } else {
                "effect-continue"
            };
            return write!(f, "({kind} (loop {} {}))", id.owner, id.index);
        }
        f.write_str(match self {
            Self::EffectEscape => "effect-escape",
            Self::EffectReturn => "effect-return",
            Self::EffectBreak(_) | Self::EffectContinue(_) => unreachable!(),
            Self::HandlerEnd => "handler-end",
            Self::EffectResume => "effect-resume",
            Self::Return => "return",
            Self::FunctionEnd => "function-end",
            Self::PredicateFailure => "predicate-failure",
            Self::Break => "break",
            Self::Continue => "continue",
            Self::ScopeEnd => "scope-end",
            Self::Replacement => "replacement",
            Self::TemporaryEnd => "temporary-end",
        })
    }
}
/// A known enum case at a relative position within an owned cleanup tree.
/// An absent entry leaves enum cleanup dependent on its runtime discriminant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantSelection {
    pub fields: Vec<usize>,
    pub index: usize,
}
impl fmt::Display for VariantSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(variant-selection (fields")?;
        for index in &self.fields {
            write!(f, " {index}")?;
        }
        write!(f, ") (index {}))", self.index)
    }
}
// Callable evidence paths qualify enum payloads with [VARIANT_PATH, tag].
// Ownership paths remain ordinary field paths after a successful tag guard.
// No allocated field layout can have usize::MAX entries.
pub(super) const VARIANT_PATH: usize = usize::MAX;

fn display_callable_path(path: &[usize], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let mut components = path.iter();
    while let Some(index) = components.next() {
        if *index == VARIANT_PATH {
            match components.next() {
                Some(variant) => write!(f, " (variant {variant})")?,
                None => f.write_str(" (invalid-variant)")?,
            }
        } else {
            write!(f, " {index}")?;
        }
    }
    Ok(())
}

struct CallablePlace<'a>(&'a Place);
impl fmt::Display for CallablePlace<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(owned-place (owner {}) (local {}) (fields",
            self.0.root.owner, self.0.root.local
        )?;
        display_callable_path(&self.0.fields, f)?;
        f.write_str("))")
    }
}

fn select_variant_callables<T: Clone>(
    callables: &BTreeMap<Vec<usize>, T>,
    path: &[usize],
    variant: usize,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<BTreeMap<Vec<usize>, T>, Error> {
    let mut selected = BTreeMap::new();
    for (leaf, callable) in callables {
        *fuel = fuel
            .checked_sub(leaf.len().saturating_add(1))
            .ok_or_else(|| error(source, "variant callable selection budget exhausted"))?;
        if let Some(relative) = leaf.strip_prefix(path) {
            if relative.get(..2) == Some(&[VARIANT_PATH, variant]) {
                let mut projected = path.to_vec();
                projected.extend_from_slice(&relative[2..]);
                selected.insert(projected, callable.clone());
            }
        } else {
            selected.insert(leaf.clone(), callable.clone());
        }
    }
    Ok(selected)
}

type EnumDomains = BTreeMap<Vec<usize>, BTreeSet<usize>>;

fn enum_path_excluded(path: &[usize], domains: &EnumDomains) -> bool {
    let mut index = 0;
    while index + 1 < path.len() {
        if path[index] == VARIANT_PATH {
            if domains
                .get(&path[..index])
                .is_some_and(|tags| !tags.contains(&path[index + 1]))
            {
                return true;
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    false
}

fn copy_enum_domains(
    domains: &EnumDomains,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<EnumDomains, Error> {
    for (path, tags) in domains {
        *fuel = fuel
            .checked_sub(path.len().saturating_add(tags.len()).saturating_add(1))
            .ok_or_else(|| error(source, "enum domain export budget exhausted"))?;
    }
    Ok(domains.clone())
}

fn join_enum_domains(
    left: &EnumDomains,
    right: &EnumDomains,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<EnumDomains, Error> {
    let mut result = EnumDomains::new();
    for (path, tags) in left.iter().chain(right) {
        *fuel = fuel
            .checked_sub(path.len().saturating_add(tags.len()).saturating_add(1))
            .ok_or_else(|| error(source, "enum domain join budget exhausted"))?;
        if result.contains_key(path) {
            continue;
        }
        match (left.get(path), right.get(path)) {
            (Some(a), Some(b)) => {
                *fuel = fuel
                    .checked_sub(b.len())
                    .ok_or_else(|| error(source, "enum domain join budget exhausted"))?;
                result.insert(path.clone(), a.union(b).copied().collect());
            }
            (Some(a), None) if enum_path_excluded(path, right) => {
                result.insert(path.clone(), a.clone());
            }
            (None, Some(b)) if enum_path_excluded(path, left) => {
                result.insert(path.clone(), b.clone());
            }
            _ => (),
        }
    }
    Ok(result)
}

fn join_guarded_callables<A: Borrow<Callable>, B: Borrow<Callable>>(
    left: &BTreeMap<Vec<usize>, A>,
    right: &BTreeMap<Vec<usize>, B>,
    left_tags: &EnumDomains,
    right_tags: &EnumDomains,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<BTreeMap<Vec<usize>, Callable>, Error> {
    let mut result = BTreeMap::new();
    for path in left.keys().chain(right.keys()) {
        *fuel = fuel
            .checked_sub(path.len().saturating_add(1))
            .ok_or_else(|| error(source, "guarded callable join budget exhausted"))?;
        if result.contains_key(path) {
            continue;
        }
        let value = match (left.get(path), right.get(path)) {
            (Some(a), Some(b)) => {
                merge_callable_provenance(a.borrow(), b.borrow(), fuel, source, 0)?
            }
            (Some(a), None) if enum_path_excluded(path, right_tags) => Some(a.borrow().clone()),
            (None, Some(b)) if enum_path_excluded(path, left_tags) => Some(b.borrow().clone()),
            _ => None,
        };
        if let Some(value) = value {
            result.insert(path.clone(), value);
        }
    }
    Ok(result)
}

fn known_pattern_with_domains(
    pattern: &Pattern,
    value: &Expr,
    domains: &EnumDomains,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Option<bool>, Error> {
    let known = known_pattern(pattern, value, 0);
    if known.is_some() || domains.is_empty() {
        return Ok(known);
    }
    let mut pattern = pattern;
    while let Pattern::Binding { inner, .. } | Pattern::Remap { pattern: inner, .. } = pattern {
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(source, "error pattern domain budget exhausted"))?;
        pattern = inner;
    }
    match pattern {
        Pattern::ErrorResult { label, inner } => {
            let Term::ErrorResult { errors, .. } = &value.ty else {
                return Ok(None);
            };
            super::memory::bound_type(&value.ty, fuel).map_err(|e| error(source, e))?;
            let tag = if let Some(label) = label {
                let mut found = None;
                for (index, candidate) in errors.iter().enumerate() {
                    super::memory::bound_type(label, fuel).map_err(|e| error(source, e))?;
                    super::memory::bound_type(candidate, fuel).map_err(|e| error(source, e))?;
                    if label == candidate {
                        found = Some(index + 1);
                        break;
                    }
                }
                let Some(tag) = found else {
                    return Ok(Some(false));
                };
                tag
            } else {
                0
            };
            let Some(tags) = domains.get(&vec![]) else {
                return Ok(None);
            };
            if !tags.contains(&tag) {
                return Ok(Some(false));
            }
            let child = pattern_from_enum_domains(
                inner,
                &mut vec![VARIANT_PATH, tag, 0],
                domains,
                fuel,
                source,
                1,
            )?;
            return Ok(if tags.len() == 1 || child == Some(false) {
                child
            } else {
                None
            });
        }
        _ => (),
    }
    pattern_from_enum_domains(pattern, &mut vec![], domains, fuel, source, 0)
}

fn pattern_from_enum_domains(
    pattern: &Pattern,
    path: &mut Vec<usize>,
    domains: &EnumDomains,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<Option<bool>, Error> {
    if depth >= 128 {
        return Err(error(source, "enum pattern depth exhausted"));
    }
    *fuel = fuel
        .checked_sub(path.len().saturating_add(1))
        .ok_or_else(|| error(source, "enum pattern budget exhausted"))?;
    let (fields, tag_known) = match pattern {
        Pattern::Wild | Pattern::Unit => return Ok(Some(true)),
        Pattern::Binding { inner, .. } | Pattern::Remap { pattern: inner, .. } => {
            return pattern_from_enum_domains(inner, path, domains, fuel, source, depth + 1);
        }
        Pattern::Guard { pattern: inner, .. } | Pattern::Then { first: inner, .. } => {
            return Ok(
                if pattern_from_enum_domains(inner, path, domains, fuel, source, depth + 1)?
                    == Some(false)
                {
                    Some(false)
                } else {
                    None
                },
            );
        }
        Pattern::Or(alternatives) => {
            let mut unknown = false;
            for alternative in alternatives {
                match pattern_from_enum_domains(
                    alternative,
                    path,
                    domains,
                    fuel,
                    source,
                    depth + 1,
                )? {
                    Some(true) => return Ok(Some(true)),
                    None => unknown = true,
                    _ => (),
                }
            }
            return Ok(if unknown { None } else { Some(false) });
        }
        Pattern::Variant { index, fields, .. } => {
            let known = domains
                .get(path)
                .map(|tags| tags.len() == 1 && tags.contains(index))
                .unwrap_or(false);
            if domains.get(path).is_some_and(|tags| !tags.contains(index)) {
                return Ok(Some(false));
            }
            path.extend([VARIANT_PATH, *index]);
            (fields, known)
        }
        Pattern::Tuple(fields)
        | Pattern::Record { fields, .. }
        | Pattern::AnonymousRecord { fields, .. } => (fields, true),
        _ => return Ok(None),
    };
    let mut result = if tag_known { Some(true) } else { None };
    let mut unknown_before = false;
    for (index, field) in fields.iter().enumerate() {
        path.push(index);
        let child = pattern_from_enum_domains(field, path, domains, fuel, source, depth + 1)?;
        path.pop();
        if child == Some(false) {
            result = if unknown_before { None } else { Some(false) };
            break;
        }
        if child.is_none() {
            result = None;
            unknown_before |= super::pattern_facts::pattern_observes(field, depth + 1);
        }
    }
    if matches!(pattern, Pattern::Variant { .. }) {
        path.truncate(path.len() - 2);
    }
    Ok(result)
}

fn group_owned_callables<'a>(
    fields: &'a BTreeMap<Place, Callable>,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<BTreeMap<PlaceId, BTreeMap<Vec<usize>, &'a Callable>>, Error> {
    let mut grouped: BTreeMap<PlaceId, BTreeMap<Vec<usize>, &Callable>> = BTreeMap::new();
    for (place, value) in fields {
        *fuel = fuel
            .checked_sub(place.fields.len().saturating_add(1))
            .ok_or_else(|| error(source, "callable field grouping budget exhausted"))?;
        grouped
            .entry(place.root)
            .or_default()
            .insert(place.fields.clone(), value);
    }
    Ok(grouped)
}

type PointerTargetGuards = BTreeMap<Place, Vec<(Vec<usize>, usize)>>;
type ValuePointerGuards = BTreeMap<Vec<usize>, PointerTargetGuards>;

struct PatternPreparation<'a> {
    origins: &'a ValueOrigins,
    pointer_guards: ValuePointerGuards,
    selected_pointer_variants: Vec<VariantSelection>,
    domains: EnumDomains,
    callables: BTreeMap<Vec<usize>, Callable>,
    transfers: Vec<Vec<usize>>,
    moved_bindings: Vec<(LocalId, Vec<usize>)>,
    variants: Vec<VariantSelection>,
    /// Inner bindings of an as-pattern observe a retained whole owner.
    /// Each extracted value therefore needs its own checked Copy delivery.
    copy_bindings: bool,
    binding_remaps: Vec<BTreeMap<LocalId, LocalId>>,
}
#[derive(Debug)]
pub enum CleanupAction {
    /// A validated plan for this exact candidate, not an additional action.
    Planned {
        candidate: Box<CleanupAction>,
        plan: Box<super::drop::CleanupPlan>,
    },
    /// Alternative residual layouts before attaching executable proof plans.
    Alternatives(Vec<CleanupAction>),
    Temporary {
        identity: u64,
        temporary: TemporaryCleanup,
        presence: Availability,
        remaining: CleanupTree,
        variants: Vec<VariantSelection>,
    },
    Owned {
        ty: Term,
        remaining: CleanupTree,
    },
    Prepared {
        construction: SourceRange,
        value: PendingValue,
    },
}
impl fmt::Display for CleanupAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Alternatives(cases) => {
                f.write_str("(cleanup-alternatives")?;
                for case in cases {
                    write!(f, " {case}")?;
                }
                f.write_str(")")
            }
            Self::Planned { candidate, plan } => write!(f, "(planned-cleanup {candidate} {plan})"),
            Self::Temporary {
                identity,
                temporary,
                presence,
                remaining,
                variants,
            } => {
                write!(
                    f,
                    "(temporary-cleanup (identity {identity}) (source {}) {} (type {}) (presence {}) (fields",
                    temporary.source,
                    temporary.scope,
                    temporary.ty,
                    match presence {
                        Availability::Present => "present",
                        Availability::Conditional => "conditional",
                        Availability::Empty => "empty",
                    }
                )?;
                for index in &temporary.fields {
                    write!(f, " {index}")?;
                }
                f.write_str(") (variants")?;
                for variant in variants {
                    write!(f, " {variant}")?;
                }
                write!(f, ") {remaining})")
            }
            Self::Owned { ty, remaining } => write!(f, "(owned-cleanup (type {ty}) {remaining})"),
            Self::Prepared {
                construction,
                value,
            } => write!(
                f,
                "(prepared-cleanup (construction {construction}) (source {}) (slot {}) (type {}) {})",
                value.source, value.slot, value.ty, value.scope
            ),
        }
    }
}
/// Ordered structural candidates, before applying user Drop contracts. A user
/// destructor runs before children and may change the remaining field state.
/// Temporary presence is path-sensitive, but conditional execution still needs
/// runtime/path evidence. The selected enum layouts accompany decomposed
/// subjects; destructor access/environment/post-Drop state requires additional
/// contracts before these candidates can become executable cleanup.
#[derive(Debug)]
pub struct CleanupSite {
    pub interface_precondition: bool,
    pub interface_postcondition: bool,
    pub result_origin: Option<SourceRange>,
    pub default_origin: Option<(SourceRange, usize)>,
    pub source: SourceRange,
    pub reason: CleanupReason,
    pub actions: Vec<CleanupAction>,
}
impl fmt::Display for CleanupSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(cleanup-site (source {}) (reason {})",
            self.source, self.reason
        )?;
        if let Some((construction, parameter)) = self.default_origin {
            write!(
                f,
                " (default-origin (construction {construction}) (parameter {parameter}))"
            )?;
        }
        if self.interface_precondition {
            f.write_str(" (interface-precondition true)")?;
        }
        if self.interface_postcondition {
            f.write_str(" (interface-postcondition true)")?;
        }
        if let Some(origin) = self.result_origin {
            write!(f, " (result-origin {origin})")?;
        }
        f.write_str(" (actions")?;
        for action in &self.actions {
            write!(f, " {action}")?;
        }
        f.write_str("))")
    }
}
#[derive(Debug)]
pub struct TemporaryOwner {
    pub scope: TemporaryScope,
    pub source: SourceRange,
    pub ty: Term,
    /// Relative ownership shape in an isolated synthetic root, not a source local.
    pub remaining: State,
}
#[derive(Debug)]
pub struct ClosureReport {
    pub definition: Expr,
    pub invoked: bool,
    pub source: SourceRange,
    pub captures: Vec<PlaceId>,
    pub invocation: Report,
}
#[derive(Debug)]
pub struct LoopReport {
    pub source: SourceRange,
    pub header: State,
    pub breaks: Vec<State>,
}
#[derive(Debug)]
pub struct CallPreparation {
    pub source: SourceRange,
    pub parameters: State,
}
#[derive(Debug)]
pub struct MatchArmReport {
    pub pattern: Pattern,
    pub guards: Vec<Expr>,
    pub exit: Option<State>,
}
#[derive(Debug)]
pub struct MatchReport {
    pub source: SourceRange,
    pub arms: Vec<MatchArmReport>,
}
#[derive(Debug)]
pub struct Decomposition {
    pub source: SourceRange,
    pub ty: Term,
    pub fields: Vec<usize>,
    pub variant: Option<usize>,
}
#[derive(Debug)]
pub struct LetReport {
    pub source: SourceRange,
    pub pattern: Pattern,
    pub guards: Vec<Expr>,
    pub success: bool,
    pub failure: bool,
}
/// Default-local obligations live in a separate parameter environment. Its
/// prepared parameters alias pending caller arguments, rather than new owners.
#[derive(Debug)]
pub struct DefaultEvaluation {
    pub construction: SourceRange,
    pub source: SourceRange,
    pub parameter: usize,
    pub evaluation: Report,
}
impl fmt::Display for DefaultEvaluation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(default-evaluation (source {}) (construction {}) (parameter {}) {})",
            self.source, self.construction, self.parameter, self.evaluation
        )
    }
}
/// Caller-established enum tags, conditional on qualified enclosing variant paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumParameter {
    pub parameter: usize,
    pub domains: BTreeMap<Vec<usize>, BTreeSet<usize>>,
}
impl fmt::Display for EnumParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(enum-parameter (parameter {}) (domains", self.parameter)?;
        for (path, tags) in &self.domains {
            f.write_str(" (domain (path")?;
            display_callable_path(path, f)?;
            f.write_str(") (tags")?;
            for tag in tags {
                write!(f, " {tag}")?;
            }
            f.write_str("))")?;
        }
        f.write_str("))")
    }
}
// Empty prepared code or closed-summary pointer origins at a variant-qualified
// leaf prove that combination of enclosing tags absent. Propagate an empty
// nested domain to its parent.
fn exclude_absent_code_leaf(
    package: Option<&Package>,
    subject: &Term,
    fields: &[usize],
    domains: &mut EnumDomains,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<bool, Error> {
    *fuel = fuel
        .checked_sub(fields.len() + 1)
        .ok_or_else(|| error(source, "returned capture absence budget exhausted"))?;
    if enum_path_excluded(fields, domains) {
        return Ok(true);
    }
    let mut path = fields.to_vec();
    loop {
        *fuel = fuel
            .checked_sub(path.len() + 1)
            .ok_or_else(|| error(source, "returned capture absence budget exhausted"))?;
        let Some(index) = path.iter().rposition(|field| *field == VARIANT_PATH) else {
            return Ok(false);
        };
        let Some(variant) = path.get(index + 1).copied() else {
            return Ok(false);
        };
        path.truncate(index);
        let ty = owned_field_type(package, subject, &path, fuel, source)?;
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Ok(false);
        };
        let Some(definition) = package.and_then(|package| {
            package
                .enums
                .iter()
                .find(|item| item.declaration == declaration)
        }) else {
            return Ok(false);
        };
        let variants = callable_variant_layout(definition, &arguments, fuel, source)?;
        *fuel = fuel
            .checked_sub(variants.len() + path.len() + 1)
            .ok_or_else(|| error(source, "returned capture absence budget exhausted"))?;
        if variant >= variants.len() {
            return Err(error(source, "absent code variant out of bounds"));
        }
        let tags = domains
            .entry(path.clone())
            .or_insert_with(|| (0..variants.len()).collect());
        tags.remove(&variant);
        if !tags.is_empty() {
            break;
        }
        domains.remove(&path);
    }
    let mut excluded = vec![];
    for path in domains.keys() {
        *fuel = fuel
            .checked_sub(path.len() + 1)
            .ok_or_else(|| error(source, "returned capture absence budget exhausted"))?;
        if enum_path_excluded(path, domains) {
            excluded.push(path.clone());
        }
    }
    for path in excluded {
        domains.remove(&path);
    }
    Ok(true)
}

fn validate_enum_domains(
    package: Option<&Package>,
    subject: &Term,
    domains: &EnumDomains,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<(), Error> {
    if domains.is_empty() {
        return Err(error(source, "empty enum domain premise"));
    }
    for (path, tags) in domains {
        if tags.is_empty() {
            return Err(error(source, "empty enum tag domain"));
        }
        super::memory::bound_type(subject, fuel).map_err(|e| error(source, e))?;
        let ty = owned_field_type(package, subject, path, fuel, source)?;
        let variants = super::memory::owned_variants(package, &ty, fuel)
            .map_err(|reason| error(source, reason))?;
        if tags.iter().any(|tag| *tag >= variants.len()) {
            return Err(error(source, "enum premise tag out of bounds"));
        }
        if enum_path_excluded(path, domains) {
            return Err(error(
                source,
                "enum premise has contradictory enclosing tags",
            ));
        }
    }
    Ok(())
}

fn copy_captured_enums(
    captures: &BTreeMap<PlaceId, EnumDomains>,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<BTreeMap<PlaceId, EnumDomains>, Error> {
    let mut result = BTreeMap::new();
    for (root, domains) in captures {
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(source, "captured enum export budget exhausted"))?;
        result.insert(*root, copy_enum_domains(domains, fuel, source)?);
    }
    Ok(result)
}
fn display_captured_enums(
    captures: &BTreeMap<PlaceId, EnumDomains>,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    for (root, domains) in captures {
        write!(
            f,
            " (capture (owner {}) (local {}) (domains",
            root.owner, root.local
        )?;
        for (path, tags) in domains {
            f.write_str(" (domain (path")?;
            display_callable_path(path, f)?;
            f.write_str(") (tags")?;
            for tag in tags {
                write!(f, " {tag}")?;
            }
            f.write_str("))")?;
        }
        f.write_str("))")?;
    }
    Ok(())
}
fn record_enum_parameter(
    inputs: &mut Vec<EnumParameter>,
    parameter: usize,
    domains: &EnumDomains,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<(), Error> {
    if !domains.is_empty() {
        inputs.push(EnumParameter {
            parameter,
            domains: copy_enum_domains(domains, fuel, source)?,
        });
    }
    Ok(())
}

#[derive(Debug)]
pub struct NamedCall {
    pub source: SourceRange,
    pub callee: Expr,
    pub callables: Vec<CallableParameter>,
    pub enums: Vec<EnumParameter>,
    pub effects: Vec<EffectResultInput>,
    pub borrowed_code: Vec<BorrowedCode>,
}
impl fmt::Display for NamedCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(named-call (source {}) (callee {}) (callables",
            self.source, self.callee
        )?;
        for input in &self.callables {
            write!(f, " {input}")?;
        }
        f.write_str(") (enums")?;
        for input in &self.enums {
            write!(f, " {input}")?;
        }
        f.write_str(") (effects")?;
        for input in &self.effects {
            write!(f, " {input}")?;
        }
        f.write_str(") (borrowed-code")?;
        for input in &self.borrowed_code {
            write!(f, " {input}")?;
        }
        f.write_str("))")
    }
}

/// Finite named targets whose memory contracts were all checked at one call.
#[derive(Debug)]
pub struct NamedCandidateCall {
    pub source: SourceRange,
    pub candidates: Vec<Expr>,
}
impl fmt::Display for NamedCandidateCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(named-candidate-call (source {}) (candidates",
            self.source
        )?;
        for candidate in &self.candidates {
            write!(f, " {candidate}")?;
        }
        f.write_str("))")
    }
}

/// Code provenance and conditional captured tags, separate from executable
/// capture initializers. Ownership availability is imported independently at use.
#[derive(Clone, Debug, PartialEq)]
pub struct CallableEvidence {
    /// Additional callable candidates, each retaining its own capture evidence.
    /// A set containing closures uses a closure representative; named-only sets
    /// continue to use `alternatives`.
    pub choices: Vec<CallableEvidence>,
    pub enum_captures: BTreeMap<PlaceId, BTreeMap<Vec<usize>, BTreeSet<usize>>>,
    pub definition: Expr,
    pub alternatives: Vec<Expr>,
    /// Captured binding plus relative owned field path; an empty path is the whole binding.
    pub captures: BTreeMap<Place, CallableEvidence>,
}
impl fmt::Display for CallableEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(callable-evidence (definition {}) (alternatives",
            self.definition
        )?;
        for alternative in &self.alternatives {
            write!(f, " {alternative}")?;
        }
        f.write_str(") (choices")?;
        for choice in &self.choices {
            write!(f, " {choice}")?;
        }
        f.write_str(") (captures")?;
        for (binding, evidence) in &self.captures {
            write!(f, " (capture {} {evidence})", CallablePlace(binding))?;
        }
        f.write_str(") (enum-captures")?;
        display_captured_enums(&self.enum_captures, f)?;
        f.write_str("))")
    }
}

/// A conditional input to function-body analysis. The caller must establish
/// that this callable definition belongs to the delivered parameter; equal callable
/// types alone do not discharge this premise.
#[derive(Clone, Debug, PartialEq)]
pub struct CallableParameter {
    pub choices: Vec<CallableEvidence>,
    pub enum_captures: BTreeMap<PlaceId, BTreeMap<Vec<usize>, BTreeSet<usize>>>,
    pub parameter: usize,
    /// Relative evidence path; empty means the argument itself. Enum payloads
    /// use [usize::MAX, variant, field, ...], requiring a matching tag guard.
    pub fields: Vec<usize>,
    pub definition: Expr,
    /// Additional targets for named-only sets. Sets involving closures use
    /// `choices`, which retains separate evidence for each environment.
    pub alternatives: Vec<Expr>,
    /// Captured binding plus relative owned field path; an empty path is the whole binding.
    pub captures: BTreeMap<Place, CallableEvidence>,
}
impl fmt::Display for CallableParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(callable-parameter (parameter {}) (definition {}) (alternatives",
            self.parameter, self.definition
        )?;
        for alternative in &self.alternatives {
            write!(f, " {alternative}")?;
        }
        f.write_str(") (fields")?;
        display_callable_path(&self.fields, f)?;
        f.write_str(") (choices")?;
        for choice in &self.choices {
            write!(f, " {choice}")?;
        }
        f.write_str(") (captures")?;
        for (binding, evidence) in &self.captures {
            write!(f, " (capture {} {evidence})", CallablePlace(binding))?;
        }
        f.write_str(") (enum-captures")?;
        display_captured_enums(&self.enum_captures, f)?;
        f.write_str("))")
    }
}

/// Result evidence travels with a lexical handler return, independently of
/// the interrupted operation's ordinary result. Shared snapshots avoid copying
/// closure trees at each enclosing installation or suspension checkpoint.
#[derive(Debug, PartialEq)]
pub struct HandlerReturnValue {
    pub source: SourceRange,
    /// Installer-relative targets; request-parameter virtual roots require a separate substitution.
    pub origins: BTreeMap<Vec<usize>, BTreeSet<Place>>,
    pub pointer_guards: ValuePointerGuards,
    pub callable: Option<CallableEvidence>,
    pub fields: BTreeMap<Vec<usize>, CallableEvidence>,
    pub variants: BTreeMap<Vec<usize>, BTreeSet<usize>>,
}
impl fmt::Display for HandlerReturnValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(handler-return (source {}) (callable", self.source)?;
        if let Some(value) = &self.callable {
            write!(f, " {value}")?;
        }
        f.write_str(") (fields")?;
        for (path, value) in &self.fields {
            f.write_str(" (field ")?;
            display_callable_path(path, f)?;
            write!(f, " {value})")?;
        }
        f.write_str(") (variants")?;
        for (path, tags) in &self.variants {
            f.write_str(" (domain ")?;
            display_callable_path(path, f)?;
            f.write_str(" (tags")?;
            for tag in tags {
                write!(f, " {tag}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(") (origins")?;
        for (path, targets) in &self.origins {
            f.write_str(" (leaf ")?;
            display_callable_path(path, f)?;
            f.write_str(" (targets")?;
            for target in targets {
                write!(f, " {target}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(") (pointer-guards")?;
        for (path, targets) in &self.pointer_guards {
            f.write_str(" (leaf (path")?;
            display_callable_path(path, f)?;
            f.write_str(") (targets")?;
            for (target, conditions) in targets {
                write!(f, " (target {target} (conditions")?;
                for (path, tag) in conditions {
                    f.write_str(" (guard (path")?;
                    display_callable_path(path, f)?;
                    write!(f, ") (tag {tag}))")?;
                }
                f.write_str("))")?;
            }
            f.write_str("))")?;
        }
        f.write_str("))")
    }
}

#[derive(Debug)]
pub struct HandlerReport {
    pub installation: SourceRange,
    pub handler: usize,
    pub arm: usize,
    pub definition: nir::program::MatchArm,
    pub result: Term,
    pub invocation: Report,
}
impl fmt::Display for HandlerReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(handler-resource-report (installation {}) (handler {}) (arm {}) (result {}) {})",
            self.installation, self.handler, self.arm, self.result, self.invocation
        )
    }
}
/// Conditional validity of a pointer stored in an owned parameter field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BorrowedField {
    pub parameter: usize,
    pub fields: Vec<usize>,
    pub pointee: Term,
}

// Invocation-local evidence for a read-only borrowed storage root. It is not a
// property of the pointer type and must never survive a potentially writing call.
#[derive(Clone, Debug, PartialEq)]
pub struct BorrowedCode {
    absent: bool,
    invocation: bool,
    parameter: usize,
    input_fields: Vec<usize>,
    pointee: Term,
    fields: BTreeMap<Vec<usize>, Callable>,
    domains: EnumDomains,
}
impl std::fmt::Display for BorrowedField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(borrowed-field (parameter {}) (fields", self.parameter)?;
        for field in &self.fields {
            write!(f, " {field}")?;
        }
        write!(f, ") (pointee {}))", self.pointee)
    }
}
#[derive(Debug, Default)]
pub struct Report {
    /// Checked affine deliveries, executed after evaluating the exact source
    /// place. Reading a projection's base is not a separate transfer.
    pub transfers: Vec<(SourceRange, Place)>,
    /// Checked representation deliveries preserve the source ownership tree.
    pub copies: Vec<(SourceRange, Place)>,
    /// Canonical pattern bindings and their checked possible subject positions.
    pub pattern_transfers: Vec<(SourceRange, LocalId, Vec<usize>)>,
    pub pattern_orders: Vec<(SourceRange, Vec<Vec<LocalId>>)>,
    /// Ordered structural candidates. Other lists describe analysis evidence;
    /// they must not be appended as additional executable cleanup actions.
    pub cleanup_sites: Vec<CleanupSite>,
    /// Conditional entry premises; not inferred from the parameter type alone.
    pub callable_parameters: Vec<CallableParameter>,
    pub enum_parameters: Vec<EnumParameter>,
    pub effect_parameters: Vec<EffectResultInput>,
    /// Conditional initialized pointees; publication must check actual call targets.
    pub borrowed_parameters: Vec<(usize, Term)>,
    pub borrowed_fields: Vec<BorrowedField>,
    pub borrowed_code: Vec<BorrowedCode>,
    /// Symbolic origins of nonowned roots, retained for cleanup contract export.
    pub borrowed_roots: BTreeMap<PlaceId, BorrowedRoot>,
    /// Implicit accesses of executed closures, resolved in this caller's storage.
    pub cleanup_call_accesses: Vec<(super::memory::Access, BTreeSet<Place>)>,
    pub incomplete: Vec<IncompleteCleanup>,
    pub cleanups: Vec<Cleanup>,
    /// Successful returns only; closure calls join these continuation states.
    pub exits: Vec<State>,
    /// Known result definition per successful exit; None is unknown/non-callable.
    pub callable_exits: Vec<Option<Expr>>,
    /// Expression-only candidate view per successful exit. Unknown when nested
    /// captures need richer evidence; the singleton view also excludes alternatives.
    pub callable_candidate_exits: Vec<Option<Vec<Expr>>>,
    /// Authoritative return provenance, including nested callable captures.
    /// The expression-only views above are unknown if this cannot be encoded.
    pub callable_evidence_exits: Vec<Option<CallableEvidence>>,
    /// Known callable leaves delivered on each normal exit, relative to the result.
    pub callable_field_exits: Vec<BTreeMap<Vec<usize>, CallableEvidence>>,
    /// Conditional enum tag domains for the delivered value at each normal exit.
    pub enum_variant_exits: Vec<BTreeMap<Vec<usize>, BTreeSet<usize>>>,
    /// Callable identities remaining in persistent capture slots at each
    /// successful exit; aligned with `exits`, never a substitute for its state.
    pub captured_callable_exits: Vec<BTreeMap<Place, CallableEvidence>>,
    pub captured_enum_exits: Vec<BTreeMap<PlaceId, BTreeMap<Vec<usize>, BTreeSet<usize>>>>,
    pub failed_capture_enum_exits: Vec<BTreeMap<PlaceId, EnumDomains>>,
    pub effect_capture_enum_exits: Vec<BTreeMap<PlaceId, EnumDomains>>,
    /// Pointer-slot origins retained by capture storage, aligned with each exit
    /// kind. Missing slots are not proof of pointer freedom; consult the exit
    /// state and layout. Targets remain in this invocation's coordinate system.
    pub captured_pointer_exits: Vec<BTreeMap<Place, BTreeSet<Place>>>,
    pub captured_pointer_guard_exits: Vec<BTreeMap<Place, PointerTargetGuards>>,
    pub failed_capture_pointer_exits: Vec<BTreeMap<Place, BTreeSet<Place>>>,
    pub failed_capture_pointer_guard_exits: Vec<BTreeMap<Place, PointerTargetGuards>>,
    pub effect_capture_pointer_exits: Vec<BTreeMap<Place, BTreeSet<Place>>>,
    pub effect_capture_pointer_guard_exits: Vec<BTreeMap<Place, PointerTargetGuards>>,
    /// Work charged by the outer analysis entry point, including nested summaries.
    pub entry_steps_used: usize,
    pub failed_exits: Vec<State>,
    /// Source and suspended state on full-frame effect escape. Kept separate
    /// from normal returns and program rejection, including capture writeback.
    pub effect_exits: Vec<(SourceRange, State)>,
    /// Returns targeting the lexical installer rather than operation completion.
    pub lexical_returns: Vec<std::sync::Arc<HandlerReturnValue>>,
    /// Lexical target and edge kind: true is break, false is continue.
    pub lexical_loop_exits: BTreeSet<(LocalId, bool)>,
    /// Component-level descriptions, including fields retained by one parent
    /// candidate after pattern decomposition. Not an independent schedule.
    pub temporaries: Vec<TemporaryCleanup>,
    pub temporary_owners: Vec<TemporaryOwner>,
    pub handlers: Vec<HandlerReport>,
    pub closures: Vec<ClosureReport>,
    pub loops: Vec<LoopReport>,
    pub calls: Vec<CallPreparation>,
    /// Unresolved callable obligations in construction-only body analysis.
    /// These are never proof of a checked invocation or executable cleanup.
    pub deferred_calls: Vec<SourceRange>,
    pub named_calls: Vec<NamedCall>,
    pub named_candidate_calls: Vec<NamedCandidateCall>,
    /// Owned callee values that have no source-level local binding.
    pub callee_temporaries: Vec<(PlaceId, SourceRange)>,
    pub defaults: Vec<DefaultEvaluation>,
    pub matches: Vec<MatchReport>,
    pub decompositions: Vec<Decomposition>,
    pub bindings: Vec<LetReport>,
}
impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(move-report (transfers")?;
        for (source, place) in &self.transfers {
            write!(f, " (transfer (source {source}) {place})")?;
        }
        f.write_str(") (pattern-orders")?;
        for (source, orders) in &self.pattern_orders {
            write!(f, " (pattern-order (source {source})")?;
            for order in orders {
                f.write_str(" (bindings")?;
                for binding in order {
                    write!(f, " {binding}")?;
                }
                f.write_str(")")?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (pattern-transfers")?;
        for (source, binding, path) in &self.pattern_transfers {
            write!(
                f,
                " (pattern-transfer (source {source}) (binding {binding}) (fields"
            )?;
            for field in path {
                write!(f, " {field}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(") (copies")?;
        for (source, place) in &self.copies {
            write!(f, " (representation-copy (source {source}) {place})")?;
        }
        f.write_str(") (borrowed-parameters")?;
        for (slot, ty) in &self.borrowed_parameters {
            write!(f, " (borrowed-parameter (slot {slot}) (pointee {ty}))")?;
        }
        f.write_str(") (borrowed-fields")?;
        for field in &self.borrowed_fields {
            write!(f, " {field}")?;
        }
        f.write_str(") (borrowed-code")?;
        for input in &self.borrowed_code {
            write!(f, " {input}")?;
        }
        f.write_str(") (borrowed-roots")?;
        for (root, origin) in &self.borrowed_roots {
            write!(
                f,
                " (borrowed-root (owner {}) (local {}) {origin})",
                root.owner, root.local
            )?;
        }
        f.write_str(") (cleanup-call-accesses")?;
        for (access, targets) in &self.cleanup_call_accesses {
            write!(f, " (cleanup-call-access {access} (targets")?;
            for target in targets {
                write!(f, " {target}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(") (effect-parameters")?;
        for input in &self.effect_parameters {
            write!(f, " {input}")?;
        }
        f.write_str(") (cleanups")?;
        for c in &self.cleanups {
            write!(
                f,
                " (cleanup (source {}) {} {})",
                c.source,
                c.place,
                match c.availability {
                    Availability::Present => "present",
                    Availability::Conditional => "conditional",
                    Availability::Empty => "empty",
                }
            )?;
        }
        f.write_str(") (callable-parameters")?;
        for parameter in &self.callable_parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (enum-parameters")?;
        for parameter in &self.enum_parameters {
            write!(f, " {parameter}")?;
        }
        write!(f, ") (entry-steps-used {}) (exits", self.entry_steps_used)?;
        for exit in &self.exits {
            write!(f, " {exit}")?;
        }
        f.write_str(") (callable-exits")?;
        for value in &self.callable_exits {
            if let Some(value) = value {
                write!(f, " (known {value})")?;
            } else {
                f.write_str(" (unknown)")?;
            }
        }
        f.write_str(") (callable-candidate-exits")?;
        for candidates in &self.callable_candidate_exits {
            if let Some(candidates) = candidates {
                f.write_str(" (candidates")?;
                for candidate in candidates {
                    write!(f, " {candidate}")?;
                }
                f.write_str(")")?;
            } else {
                f.write_str(" (unknown)")?;
            }
        }
        f.write_str(") (callable-evidence-exits")?;
        for evidence in &self.callable_evidence_exits {
            if let Some(evidence) = evidence {
                write!(f, " {evidence}")?;
            } else {
                f.write_str(" (unknown)")?;
            }
        }
        f.write_str(") (callable-field-exits")?;
        for leaves in &self.callable_field_exits {
            f.write_str(" (fields")?;
            for (path, evidence) in leaves {
                f.write_str(" (leaf (path")?;
                display_callable_path(path, f)?;
                write!(f, ") {evidence})")?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (enum-variant-exits")?;
        for domains in &self.enum_variant_exits {
            f.write_str(" (domains")?;
            for (path, tags) in domains {
                f.write_str(" (domain (path")?;
                display_callable_path(path, f)?;
                f.write_str(") (tags")?;
                for tag in tags {
                    write!(f, " {tag}")?;
                }
                f.write_str("))")?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (captured-callable-exits")?;
        for captures in &self.captured_callable_exits {
            f.write_str(" (captures")?;
            for (binding, evidence) in captures {
                write!(f, " (capture {} {evidence})", CallablePlace(binding))?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (captured-enum-exits")?;
        for captures in &self.captured_enum_exits {
            f.write_str(" (captures")?;
            display_captured_enums(captures, f)?;
            f.write_str(")")?;
        }
        for (name, exits) in [
            ("failed-capture-enum-exits", &self.failed_capture_enum_exits),
            ("effect-capture-enum-exits", &self.effect_capture_enum_exits),
        ] {
            write!(f, ") ({name}")?;
            for captures in exits {
                f.write_str(" (captures")?;
                display_captured_enums(captures, f)?;
                f.write_str(")")?;
            }
        }
        for (name, exits) in [
            ("captured-pointer-exits", &self.captured_pointer_exits),
            (
                "failed-capture-pointer-exits",
                &self.failed_capture_pointer_exits,
            ),
            (
                "effect-capture-pointer-exits",
                &self.effect_capture_pointer_exits,
            ),
        ] {
            write!(f, ") ({name}")?;
            for captures in exits {
                f.write_str(" (captures")?;
                for (slot, targets) in captures {
                    write!(f, " (slot {slot} (targets")?;
                    for target in targets {
                        write!(f, " {target}")?;
                    }
                    f.write_str("))")?;
                }
                f.write_str(")")?;
            }
        }
        for (name, exits) in [
            (
                "captured-pointer-guard-exits",
                &self.captured_pointer_guard_exits,
            ),
            (
                "failed-capture-pointer-guard-exits",
                &self.failed_capture_pointer_guard_exits,
            ),
            (
                "effect-capture-pointer-guard-exits",
                &self.effect_capture_pointer_guard_exits,
            ),
        ] {
            write!(f, ") ({name}")?;
            for captures in exits {
                f.write_str(" (captures")?;
                for (slot, entries) in captures {
                    write!(f, " (slot {slot}")?;
                    for (target, conditions) in entries {
                        write!(f, " (target {target}")?;
                        for (path, tag) in conditions {
                            f.write_str(" (condition (fields")?;
                            for field in path {
                                write!(f, " {field}")?;
                            }
                            write!(f, ") (variant {tag}))")?;
                        }
                        f.write_str(")")?;
                    }
                    f.write_str(")")?;
                }
                f.write_str(")")?;
            }
        }
        f.write_str(") (failed-exits")?;
        for exit in &self.failed_exits {
            write!(f, " {exit}")?;
        }
        f.write_str(") (effect-exits")?;
        for (id, is_break) in &self.lexical_loop_exits {
            let kind = if *is_break {
                "handler-break"
            } else {
                "handler-continue"
            };
            write!(f, " ({kind} (loop {} {}))", id.owner, id.index)?;
        }
        for value in &self.lexical_returns {
            write!(f, " {value}")?;
        }
        for (source, state) in &self.effect_exits {
            write!(f, " (effect-exit (source {source}) {state})")?;
        }
        f.write_str(") (temporary-cleanups")?;
        for temporary in &self.temporaries {
            write!(
                f,
                " (temporary (source {}) (type {}) {} (fields",
                temporary.source, temporary.ty, temporary.scope
            )?;
            for index in &temporary.fields {
                write!(f, " {index}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(") (handlers")?;
        for handler in &self.handlers {
            write!(f, " {handler}")?;
        }
        f.write_str(") (closures")?;
        for closure in &self.closures {
            write!(f, " (closure (source {}) (captures", closure.source)?;
            for capture in &closure.captures {
                write!(f, " (local {} {})", capture.owner, capture.local)?;
            }
            write!(
                f,
                ") (invoked {}) (definition {}) (invocation {}))",
                closure.invoked, closure.definition, closure.invocation
            )?;
        }
        f.write_str(") (callee-temporaries")?;
        for (root, source) in &self.callee_temporaries {
            write!(
                f,
                " (callee-temporary (owner {}) (local {}) (source {}))",
                root.owner, root.local, source
            )?;
        }
        f.write_str(") (named-calls")?;
        for call in &self.named_calls {
            write!(f, " {call}")?;
        }
        f.write_str(") (named-candidate-calls")?;
        for call in &self.named_candidate_calls {
            write!(f, " {call}")?;
        }
        f.write_str(") (deferred-calls")?;
        for source in &self.deferred_calls {
            write!(f, " {source}")?;
        }
        f.write_str(") (loops")?;
        for item in &self.loops {
            write!(
                f,
                " (loop (source {}) (header {}) (breaks",
                item.source, item.header
            )?;
            for exit in &item.breaks {
                write!(f, " {exit}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(") (call-preparations")?;
        for call in &self.calls {
            write!(
                f,
                " (call (source {}) (parameters {}))",
                call.source, call.parameters
            )?;
        }
        f.write_str(") (default-evaluations")?;
        for default in &self.defaults {
            write!(f, " {default}")?;
        }
        f.write_str(") (matches")?;
        for item in &self.matches {
            write!(f, " (match (source {}) (arms", item.source)?;
            for arm in &item.arms {
                write!(f, " (arm (pattern {}) (guards", arm.pattern)?;
                for guard in &arm.guards {
                    write!(f, " {guard}")?;
                }
                f.write_str(") (exit")?;
                if let Some(exit) = &arm.exit {
                    write!(f, " {exit}")?;
                }
                f.write_str("))")?;
            }
            f.write_str("))")?;
        }
        f.write_str(") (decompositions")?;
        for item in &self.decompositions {
            write!(
                f,
                " (decomposition (source {}) (type {}) (fields",
                item.source, item.ty
            )?;
            for field in &item.fields {
                write!(f, " {field}")?;
            }
            f.write_str(") (variant")?;
            if let Some(variant) = item.variant {
                write!(f, " {variant}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(") (pattern-bindings")?;
        for binding in &self.bindings {
            write!(
                f,
                " (binding (source {}) (pattern {}) (guards",
                binding.source, binding.pattern
            )?;
            for guard in &binding.guards {
                write!(f, " {guard}")?;
            }
            write!(
                f,
                ") (success {}) (failure {}))",
                binding.success, binding.failure
            )?;
        }
        f.write_str(") (temporary-owners")?;
        for owner in &self.temporary_owners {
            write!(
                f,
                " (owner (source {}) (type {}) {} (remaining {}))",
                owner.source, owner.ty, owner.scope, owner.remaining
            )?;
        }
        f.write_str(") (cleanup-sites")?;
        for site in &self.cleanup_sites {
            write!(f, " {site}")?;
        }
        f.write_str(") (incomplete-cleanups")?;
        for item in &self.incomplete {
            write!(
                f,
                " (incomplete (exit {}) (construction {}) (values",
                item.exit, item.construction
            )?;
            for value in &item.values {
                write!(
                    f,
                    " (value (source {}) {} (type {}) (slot {}) (locals-before",
                    value.source, value.scope, value.ty, value.slot
                )?;
                for local in &value.locals_before {
                    write!(f, " (local {} {})", local.owner, local.local)?;
                }
                f.write_str("))")?;
            }
            f.write_str("))")?;
        }
        f.write_str("))")
    }
}
impl Report {
    fn append(&mut self, mut other: Report) {
        // append is only for work in the same invocation. Nested function and
        // closure reports retain their own origin namespace instead.
        self.borrowed_roots.append(&mut other.borrowed_roots);
        self.cleanup_call_accesses
            .append(&mut other.cleanup_call_accesses);
        self.transfers.append(&mut other.transfers);
        self.copies.append(&mut other.copies);
        self.pattern_transfers.append(&mut other.pattern_transfers);
        self.pattern_orders.append(&mut other.pattern_orders);
        self.cleanup_sites.append(&mut other.cleanup_sites);
        self.incomplete.append(&mut other.incomplete);
        self.cleanups.append(&mut other.cleanups);
        self.exits.append(&mut other.exits);
        self.callable_exits.append(&mut other.callable_exits);
        self.callable_candidate_exits
            .append(&mut other.callable_candidate_exits);
        self.callable_evidence_exits
            .append(&mut other.callable_evidence_exits);
        self.callable_field_exits
            .append(&mut other.callable_field_exits);
        self.enum_variant_exits
            .append(&mut other.enum_variant_exits);
        self.captured_callable_exits
            .append(&mut other.captured_callable_exits);
        self.captured_enum_exits
            .append(&mut other.captured_enum_exits);
        self.captured_pointer_exits
            .append(&mut other.captured_pointer_exits);
        self.captured_pointer_guard_exits
            .append(&mut other.captured_pointer_guard_exits);
        self.failed_capture_enum_exits
            .append(&mut other.failed_capture_enum_exits);
        self.effect_capture_enum_exits
            .append(&mut other.effect_capture_enum_exits);
        self.failed_capture_pointer_exits
            .append(&mut other.failed_capture_pointer_exits);
        self.failed_capture_pointer_guard_exits
            .append(&mut other.failed_capture_pointer_guard_exits);
        self.effect_capture_pointer_exits
            .append(&mut other.effect_capture_pointer_exits);
        self.effect_capture_pointer_guard_exits
            .append(&mut other.effect_capture_pointer_guard_exits);
        self.failed_exits.append(&mut other.failed_exits);
        self.effect_exits.append(&mut other.effect_exits);
        self.lexical_returns.append(&mut other.lexical_returns);
        self.lexical_loop_exits
            .append(&mut other.lexical_loop_exits);
        self.temporaries.append(&mut other.temporaries);
        self.temporary_owners.append(&mut other.temporary_owners);
        self.closures.append(&mut other.closures);
        self.handlers.append(&mut other.handlers);
        self.loops.append(&mut other.loops);
        self.calls.append(&mut other.calls);
        self.deferred_calls.append(&mut other.deferred_calls);
        self.named_calls.append(&mut other.named_calls);
        self.named_candidate_calls
            .append(&mut other.named_candidate_calls);
        self.callee_temporaries
            .append(&mut other.callee_temporaries);
        self.defaults.append(&mut other.defaults);
        self.matches.append(&mut other.matches);
        self.decompositions.append(&mut other.decompositions);
        self.bindings.append(&mut other.bindings);
    }
}
#[derive(Clone)]
struct LoopFrame {
    temporary_floor: u64,
    pending_depth: usize,
    identity: LocalId,
    roots: BTreeSet<PlaceId>,
    breaks: Vec<Flow>,
    continues: Vec<Flow>,
}
#[derive(Clone, Debug, PartialEq)]
struct Callable {
    choices: Vec<Callable>,
    enum_captures: BTreeMap<PlaceId, EnumDomains>,
    definition: Expr,
    // Additional plain named targets, never alternative captured environments.
    alternatives: Vec<Expr>,
    generation: u64,
    // Definition provenance is immutable; live capture availability comes from
    // the nested owned positions imported at each invocation, never this tree.
    captures: BTreeMap<Place, Callable>,
}
fn bound_callable_copy(
    value: &Callable,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<(), Error> {
    if depth >= 128 {
        return Err(error(source, "callable copy depth exhausted"));
    }
    for definition in std::iter::once(&value.definition).chain(&value.alternatives) {
        nir::visit::expression(definition, fuel, |expr, fuel| {
            super::memory::bound_type(&expr.ty, fuel)?;
            if let K::FunctionInstance { arguments, .. } = &expr.kind {
                for argument in arguments {
                    super::memory::bound_type(argument, fuel)?;
                }
            }
            Ok(())
        })
        .map_err(|e| error(source, e))?;
    }
    for domains in value.enum_captures.values() {
        for (path, tags) in domains {
            *fuel = fuel
                .checked_sub(path.len().saturating_add(tags.len()).saturating_add(1))
                .ok_or_else(|| error(source, "callable copy budget exhausted"))?;
        }
    }
    for (place, capture) in &value.captures {
        *fuel = fuel
            .checked_sub(place.fields.len().saturating_add(1))
            .ok_or_else(|| error(source, "callable copy budget exhausted"))?;
        bound_callable_copy(capture, fuel, source, depth + 1)?;
    }
    for choice in &value.choices {
        bound_callable_copy(choice, fuel, source, depth + 1)?;
    }
    Ok(())
}
/// Values already prepared at their source evaluation points. Parameter slots
/// are positional so different closure definitions can bind the same delivered
/// arguments without evaluating source expressions again.
#[derive(Clone)]
struct ClosureArguments {
    pointers: MemoryTargets,
    callables: BTreeMap<usize, Callable>,
    fields: BTreeMap<usize, BTreeMap<Vec<usize>, Callable>>,
    domains: BTreeMap<usize, EnumDomains>,
}
#[derive(Clone, Debug, PartialEq)]
struct ResidualLayout {
    remaining: State,
    variants: Vec<VariantSelection>,
}
#[derive(Clone, Debug, Default, PartialEq)]
struct ValueConditions {
    domains: EnumDomains,
    pointers: ValuePointerGuards,
}
#[derive(Clone, Debug, PartialEq)]
struct ActiveTemporary {
    conditions: ValueConditions,
    alternatives: Vec<ResidualLayout>,
    origins: ValueOrigins,
    variants: Vec<VariantSelection>,
    scope_id: u64,
    description: TemporaryCleanup,
    remaining: State,
    presence: Availability,
    locals_before: Vec<PlaceId>,
}
// A reserved, uninhabited target preserves uncertainty beside known alternatives.
// Source locals use u32 owners; synthetic storage uses MAX through MAX-4.
// MAX-6 similarly marks a target invalidated by retirement. Neither root is
// ever declared, so neither can justify an access or a borrow premise.
// Keeping it prevents an unknown branch from erasing a known escaping address.
fn unknown_pointer_target() -> Place {
    Place {
        root: PlaceId {
            owner: u64::MAX - 5,
            local: 0,
        },
        fields: vec![],
    }
}
fn invalidated_pointer_target() -> Place {
    Place {
        root: PlaceId {
            owner: u64::MAX - 6,
            local: 0,
        },
        fields: vec![],
    }
}
// Known pointer leaves relative to the root of a value being delivered.
// Missing leaves are unknown, not proof that the value has no dependencies.
type ValueOrigins = BTreeMap<Vec<usize>, BTreeSet<Place>>;
// Symbolic owned input paths; unlike ValueOrigins these omit pointer wrappers.
type MemoryTargets = BTreeMap<usize, ValueOrigins>;

#[derive(Clone, Default)]
struct Flow {
    cleanup_before: BTreeMap<PlaceId, BTreeSet<PlaceId>>,
    borrowed_scope: u64,
    // Synthetic nonowned targets whose actual caller origins passed the scalar
    // handler contract at this invocation. Construction never supplies these.
    effect_borrows: BTreeSet<PlaceId>,
    // Pointer capture slots checked across the full lexical installer.
    stable_handler_slots: BTreeSet<PlaceId>,
    // Complete enclosing body scan for frames without a function exit contract.
    // Shared by branches, never inferred from the path currently being checked.
    pointer_slot_mutations: Option<std::sync::Arc<BTreeSet<Place>>>,
    // Optional necessary conditions for individual physical-slot candidates.
    // Missing metadata never authorizes excluding a target.
    pointer_guards: BTreeMap<Place, PointerTargetGuards>,
    state: State,
    roots: BTreeSet<PlaceId>,
    order: Vec<PlaceId>,
    types: BTreeMap<PlaceId, Term>,
    temporaries: BTreeMap<u64, ActiveTemporary>,
    callables: BTreeMap<PlaceId, Callable>,
    callable_fields: BTreeMap<Place, Callable>,
    value_callable: Option<Callable>,
    // Construction transfers pointer-bearing capture paths with the closure.
    // Consumed by evaluate_value_origins immediately after a closure expression.
    constructed_capture_origins: ValueOrigins,
    // Callable leaves of the value currently being delivered, relative to its root.
    value_callables: BTreeMap<Vec<usize>, Callable>,
    value_variants: EnumDomains,
    value_pointer_guards: ValuePointerGuards,
    enum_domains: BTreeMap<PlaceId, EnumDomains>,
    effect_mutable_roots: BTreeSet<PlaceId>,
    pointers: BTreeMap<Place, BTreeSet<Place>>,
    memory_exposed: bool,
    live: bool,
}
impl Flow {
    fn callable_at(&self, place: &Place) -> Option<&Callable> {
        if place.fields.is_empty() {
            self.callables.get(&place.root)
        } else {
            self.callable_fields.get(place)
        }
    }
    fn callable_at_mut(&mut self, place: &Place) -> Option<&mut Callable> {
        if place.fields.is_empty() {
            self.callables.get_mut(&place.root)
        } else {
            self.callable_fields.get_mut(place)
        }
    }
    fn set_callable_at(&mut self, place: &Place, callable: Callable) {
        if place.fields.is_empty() {
            self.callables.insert(place.root, callable);
        } else {
            self.callable_fields.insert(place.clone(), callable);
        }
    }
}
fn root(local: LocalId) -> PlaceId {
    PlaceId {
        owner: u64::from(local.owner),
        local: local.index,
    }
}
fn error(source: SourceRange, reason: impl Into<String>) -> Error {
    Error {
        source,
        reason: reason.into(),
    }
}
/// An immutable-world binding arena. Acquisition is separate from planning;
/// `plan` must only use choices previously captured by `acquire`.
pub trait CleanupPolicy {
    fn acquire(&mut self, ty: &Term, fuel: &mut usize) -> Result<(), String>;
    /// No user cleanup in the complete representation, independently of Copy.
    /// Missing evidence must not manufacture duplicability or trivial cleanup.
    fn is_trivial(&mut self, _ty: &Term, _fuel: &mut usize) -> Result<bool, String> {
        Ok(false)
    }
    /// No writes, reads outside the owned receiver, or checked-predicate
    /// rejection from cleanup in the complete representation. Receiver-field
    /// availability remains an obligation of the actual resource plan. Unlike
    /// is_trivial, this may include user destructors, which must still run.
    /// This is not a termination or Copy certificate.
    fn is_memory_inert(&mut self, ty: &Term, fuel: &mut usize) -> Result<bool, String> {
        self.is_trivial(ty, fuel)
    }
    /// Complete receiver-relative read footprint, with no writes or predicate
    /// rejection. None retains the unmodeled-cleanup obligation. Returned paths
    /// still need mapping to the current value's pointer origins.
    fn cleanup_reads(
        &mut self,
        _ty: &Term,
        _fuel: &mut usize,
    ) -> Result<Option<Vec<super::memory::Access>>, String> {
        Ok(None)
    }
    /// Embedding preserves payloads, but replaces the enclosing result owner.
    /// A custom responsibility on that owner needs an explicit transfer rule.
    fn check_error_embedding(
        &mut self,
        source: &Term,
        target: &Term,
        _fuel: &mut usize,
    ) -> Result<(), String> {
        if source == target {
            Ok(())
        } else {
            Err("error embedding requires Drop responsibility transfer evidence".into())
        }
    }
    /// Conservative rejection evidence for implicit retirement in this fixed
    /// selection world. This does not authorize access or execute cleanup.
    fn may_reject(&mut self, _ty: &Term, _fuel: &mut usize) -> Result<bool, String> {
        Ok(true)
    }
    fn plan(
        &mut self,
        ty: &Term,
        state: &State,
        place: &Place,
        variants: &[VariantSelection],
        fuel: &mut usize,
    ) -> Result<super::drop::CleanupPlan, String>;
    /// Domain keys use physical fields when enclosing tags are selected, or
    /// `[usize::MAX, tag, field, ...]` qualifiers for conditional payloads.
    /// Implementations may conservatively ignore domains, but must not interpret
    /// a conditional domain as an unconditional physical-field constraint.
    fn plan_with_domains(
        &mut self,
        ty: &Term,
        state: &State,
        place: &Place,
        variants: &[VariantSelection],
        domains: &BTreeMap<Vec<usize>, BTreeSet<usize>>,
        pointers: Option<super::drop::PointerMemory<'_>>,
        fuel: &mut usize,
    ) -> Result<super::drop::CleanupPlan, String> {
        let _ = domains;
        self.plan_with_memory(ty, state, place, variants, pointers, fuel)
    }
    fn plan_with_memory(
        &mut self,
        ty: &Term,
        state: &State,
        place: &Place,
        variants: &[VariantSelection],
        _pointers: Option<super::drop::PointerMemory<'_>>,
        fuel: &mut usize,
    ) -> Result<super::drop::CleanupPlan, String> {
        self.plan(ty, state, place, variants, fuel)
    }
}

/// Opaque completion evidence issued by checked handler installations. Its
/// applicability is conditional on the corresponding operation slot and call.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectResultInput {
    slot: usize,
    operation: nir::effects::Operation,
    abi: nir::effects::Abi,
    signature: Term,
    values: Vec<std::sync::Arc<HandlerReturnValue>>,
    /// Every normal completion transfers one of these complete input operands.
    forwarded_parameters: Option<BTreeSet<usize>>,
    copy_parameters: BTreeSet<usize>,
    owned_inputs: bool,
    parameter_accesses: BTreeSet<super::memory::EffectParameterAccess>,
}

/// Copy a bounded set of shared handler-result snapshots for plan publication.
pub fn copy_effect_inputs(
    inputs: &[EffectResultInput],
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Vec<EffectResultInput>, Error> {
    for input in inputs {
        input.charge(fuel, source)?;
    }
    Ok(inputs.to_vec())
}

impl EffectResultInput {
    fn memory_input(
        &self,
        fuel: &mut usize,
        source: SourceRange,
    ) -> Result<super::memory::EffectInput, Error> {
        self.charge(fuel, source)?;
        Ok(super::memory::EffectInput {
            operation: self.operation.clone(),
            abi: self.abi,
            signature: self.signature.clone(),
            forwarded_parameters: self.forwarded_parameters.clone(),
            copy_parameters: self.copy_parameters.clone(),
            owned_inputs: self.owned_inputs,
            parameter_accesses: self.parameter_accesses.clone(),
        })
    }
    fn charge(&self, fuel: &mut usize, source: SourceRange) -> Result<(), Error> {
        *fuel = fuel
            .checked_sub(
                self.values
                    .len()
                    .saturating_add(1)
                    .saturating_add(self.forwarded_parameters.as_ref().map_or(0, BTreeSet::len))
                    .saturating_add(self.copy_parameters.len()),
            )
            .ok_or_else(|| error(source, "effect result premise budget exhausted"))?;
        for access in &self.parameter_accesses {
            *fuel = fuel
                .checked_sub(access.input_fields.len().saturating_add(1))
                .ok_or_else(|| error(source, "effect parameter access copy budget exhausted"))?;
        }
        crate::associated::charge_projection_copy(&self.signature, fuel)
            .map_err(|e| error(source, e))?;
        for argument in &self.operation.arguments {
            crate::associated::charge_projection_copy(argument, fuel)
                .map_err(|e| error(source, e))?;
        }
        Ok(())
    }
}
impl fmt::Display for EffectResultInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(effect-result-input (slot {}) {} (abi {}) (signature {}) (completions",
            self.slot, self.operation, self.abi, self.signature
        )?;
        for value in &self.values {
            write!(f, " {value}")?;
        }
        f.write_str(") (forwarded-parameters")?;
        if let Some(parameters) = &self.forwarded_parameters {
            for parameter in parameters {
                write!(f, " {parameter}")?;
            }
        }
        f.write_str(") (copy-inputs")?;
        for parameter in &self.copy_parameters {
            write!(f, " {parameter}")?;
        }
        write!(
            f,
            ") (owned-inputs {}) (parameter-accesses",
            self.owned_inputs
        )?;
        for access in &self.parameter_accesses {
            write!(f, " {access}")?;
        }
        f.write_str("))")
    }
}

struct Analyzer<'a, F, G> {
    // Captured in this analysis's immutable cleanup world; never grants Copy.
    trivial_cleanup_types: Vec<Term>,
    cleanup: Option<&'a mut dyn CleanupPolicy>,
    package: Option<&'a Package>,
    exit_contract: Option<&'a Function>,
    handler_result: Option<Term>,
    effect_return_enabled: bool,
    effect_return_values: Vec<std::sync::Arc<HandlerReturnValue>>,
    effect_loop_targets: BTreeSet<(LocalId, bool)>,
    effect_memory_inputs: Vec<super::memory::EffectInput>,
    effect_result_inputs: Vec<EffectResultInput>,
    handler_completions: Vec<std::sync::Arc<HandlerReturnValue>>,
    rejection: super::rejection::Cache<'a>,
    delivery: F,
    shape: G,
    fuel: usize,
    report: Report,
    candidate_observations: Option<Report>,
    observation_region: bool,
    observation_locals: BTreeSet<PlaceId>,
    method_callees: super::memory::MethodCallees,
    persistent_roots: BTreeSet<PlaceId>,
    captured_roots: BTreeSet<PlaceId>,
    defer_unknown_calls: bool,
    return_stack: Vec<(nir::DeclarationId, Vec<Term>)>,
    next_generation: u64,
    next_borrowed_pointer: u32,
    next_borrowed_scope: u64,
    borrowed_pointees: BTreeMap<(u64, Place, Vec<(Vec<usize>, usize)>), (PlaceId, Term)>,
    temporary_scope: TemporaryScope,
    temporary_scope_id: u64,
    next_sequence: u64,
    loops: Vec<LoopFrame>,
    pending: Vec<PendingFrame>,
}
impl<'a, F: FnMut(&Term) -> Result<Delivery, String>, G: FnMut(&Term) -> Result<Shape, String>>
    Analyzer<'a, F, G>
{
    fn call_may_reject(&mut self, callee: &Expr) -> Result<bool, &'static str> {
        let policy = &mut self.cleanup;
        self.rejection
            .may_reject_with_cleanup(
                callee,
                &mut self.fuel,
                &mut |ty, fuel| match policy.as_mut() {
                    Some(policy) => policy.may_reject(ty, fuel),
                    None => Ok(true),
                },
            )
    }
    fn acquire_cleanup(&mut self, ty: &Term, source: SourceRange) -> Result<(), Error> {
        if let Some(policy) = self.cleanup.as_mut() {
            policy
                .acquire(ty, &mut self.fuel)
                .map_err(|e| error(source, e))?;
            if self.observation_region {
                self.check_observation_cleanup(ty, source)?;
            }
        } else if self.observation_region
            && matches!(
                ty,
                Term::Nominal { .. } | Term::Record(_) | Term::Tuple(_) | Term::NormalForm { .. }
            )
        {
            return Err(error(
                source,
                "observation local cleanup requires captured selection evidence",
            ));
        }
        Ok(())
    }
    fn check_observation_cleanup(&mut self, ty: &Term, source: SourceRange) -> Result<(), Error> {
        let pointer_bearing =
            !super::memory::pointer_input_layout(self.package, ty, &mut self.fuel)
                .map_err(|e| error(source, e))?
                .is_empty();
        let policy = self.cleanup.as_mut().ok_or_else(|| {
            error(
                source,
                "observation cleanup requires captured selection evidence",
            )
        })?;
        if policy
            .may_reject(ty, &mut self.fuel)
            .map_err(|e| error(source, e))?
        {
            return Err(error(
                source,
                "observation cleanup needs failure-path resource transfer",
            ));
        }
        if pointer_bearing
            && policy
                .cleanup_reads(ty, &mut self.fuel)
                .map_err(|e| error(source, e))?
                .is_none()
        {
            return Err(error(
                source,
                "observation pointer-bearing cleanup requires a complete read-only footprint",
            ));
        }
        Ok(())
    }
    fn check_contract_predicate(
        &mut self,
        condition: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<(), Error> {
        if !flow.live {
            return Ok(());
        }
        if !nir::program::checkable_predicate(condition) {
            return Err(error(condition.source, "invalid runtime predicate check"));
        }
        self.observe_guard(condition, flow, depth + 1)?;
        match constant_bool(condition, 0) {
            Some(true) => (),
            Some(false) => self.exit(flow, condition.source, CleanupReason::PredicateFailure)?,
            None => {
                let mut failed = flow.clone();
                self.exit(
                    &mut failed,
                    condition.source,
                    CleanupReason::PredicateFailure,
                )?;
            }
        }
        Ok(())
    }
    fn stored_pointer_possible(
        &mut self,
        flow: &Flow,
        slot: &Place,
        source: SourceRange,
    ) -> Result<bool, Error> {
        // A consumed capture has no live pointer slot on this branch. Missing
        // provenance there must not poison the other branch's guarded cleanup.
        // Failed projections stay conservative (sum payloads can be opaque to
        // the ownership shape); only established absence excludes a slot.
        for depth in 0..=slot.fields.len() {
            self.fuel = self
                .fuel
                .checked_sub(depth + 1)
                .ok_or_else(|| error(source, "pointer presence budget exhausted"))?;
            let prefix = Place {
                root: slot.root,
                fields: slot.fields[..depth].to_vec(),
            };
            if matches!(flow.state.may_be_present(&prefix), Ok(false)) {
                return Ok(false);
            }
        }
        let Some(ty) = flow.types.get(&slot.root) else {
            return Ok(true);
        };
        let unknown = EnumDomains::new();
        origins::pointer_possible(
            self.package,
            ty,
            &slot.fields,
            flow.enum_domains.get(&slot.root).unwrap_or(&unknown),
            &mut vec![],
            &mut self.fuel,
            source,
            0,
        )
    }
    fn join_pointer_origins(
        &mut self,
        left: &mut Flow,
        right: &Flow,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.fuel = self
            .fuel
            .checked_sub(left.pointers.len().saturating_add(right.pointers.len()))
            .ok_or_else(|| error(source, "pointer origin join budget exhausted"))?;
        for (slot, targets) in left.pointers.iter_mut() {
            if !right.pointers.contains_key(slot)
                && self.stored_pointer_possible(right, slot, source)?
            {
                targets.insert(unknown_pointer_target());
            }
        }
        for (slot, targets) in &right.pointers {
            self.pointer_work(targets, source)?;
            if let Some(previous) = left.pointers.get_mut(slot) {
                previous.extend(targets.iter().cloned());
            } else {
                let mut targets = targets.clone();
                if self.stored_pointer_possible(left, slot, source)? {
                    targets.insert(unknown_pointer_target());
                }
                self.fuel = self
                    .fuel
                    .checked_sub(slot.fields.len().saturating_add(1))
                    .ok_or_else(|| error(source, "pointer origin join budget exhausted"))?;
                left.pointers.insert(slot.clone(), targets);
            }
        }
        Ok(())
    }
    fn join_value_origins(
        &mut self,
        ty: &Term,
        left: &mut ValueOrigins,
        left_tags: &EnumDomains,
        right: &ValueOrigins,
        right_tags: &EnumDomains,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.fuel = self
            .fuel
            .checked_sub(left.len().saturating_add(right.len()))
            .ok_or_else(|| error(source, "value pointer origin join budget exhausted"))?;
        for (path, targets) in left.iter_mut() {
            if !right.contains_key(path)
                && origins::pointer_possible(
                    self.package,
                    ty,
                    path,
                    right_tags,
                    &mut vec![],
                    &mut self.fuel,
                    source,
                    0,
                )?
            {
                targets.insert(unknown_pointer_target());
            }
        }
        for (path, targets) in right {
            self.pointer_work(targets, source)?;
            if let Some(previous) = left.get_mut(path) {
                previous.extend(targets.iter().cloned());
            } else {
                let mut targets = targets.clone();
                if origins::pointer_possible(
                    self.package,
                    ty,
                    path,
                    left_tags,
                    &mut vec![],
                    &mut self.fuel,
                    source,
                    0,
                )? {
                    targets.insert(unknown_pointer_target());
                }
                left.insert(path.clone(), targets);
            }
        }
        Ok(())
    }
    fn check_scope_targets(
        &mut self,
        targets: &BTreeSet<Place>,
        retiring: &[PlaceId],
        source: SourceRange,
    ) -> Result<(), Error> {
        self.pointer_work(targets, source)?;
        for target in targets {
            self.fuel = self
                .fuel
                .checked_sub(retiring.len().saturating_add(1))
                .ok_or_else(|| error(source, "pointer escape check budget exhausted"))?;
            if target.root == invalidated_pointer_target().root || retiring.contains(&target.root) {
                return Err(error(
                    source,
                    "pointer target unavailable outside its local scope (escaping provenance)",
                ));
            }
        }
        Ok(())
    }
    fn check_scope_origins(
        &mut self,
        origins: &ValueOrigins,
        retiring: &[PlaceId],
        source: SourceRange,
    ) -> Result<(), Error> {
        for (path, targets) in origins {
            self.fuel = self
                .fuel
                .checked_sub(path.len().saturating_add(1))
                .ok_or_else(|| error(source, "pointer escape check budget exhausted"))?;
            self.check_scope_targets(targets, retiring, source)?;
        }
        Ok(())
    }
    fn check_return_origins(
        &mut self,
        flow: &Flow,
        origins: &ValueOrigins,
        source: SourceRange,
    ) -> Result<(), Error> {
        if !flow.live {
            return Ok(());
        }
        self.fuel = self
            .fuel
            .checked_sub(flow.order.len())
            .ok_or_else(|| error(source, "pointer escape check budget exhausted"))?;
        let retiring: Vec<_> = flow
            .order
            .iter()
            .filter(|id| !self.persistent_roots.contains(id))
            .copied()
            .collect();
        self.check_scope_origins(origins, &retiring, source)
    }
    fn check_function_result(
        &mut self,
        flow: &mut Flow,
        origins: ValueOrigins,
        source: SourceRange,
        depth: usize,
    ) -> Result<(), Error> {
        let Some(function) = self.exit_contract else {
            return Ok(());
        };
        if !flow.live {
            return Ok(());
        }
        if function.postconditions.is_empty() {
            return self.check_return_origins(flow, &origins, source);
        }
        let Term::Function { result, .. } = function.signature.callable_signature() else {
            return Err(error(source, "missing result signature"));
        };
        let result = if let Term::Computation { result, .. } = result.as_ref() {
            result
        } else {
            result
        };
        for targets in origins.values() {
            self.pointer_work(targets, source)?;
        }
        let delivered_origins = origins.clone();
        let first_site = self.report.cleanup_sites.len();
        if let Some(binding) = function.result_binding {
            let conditions = self.value_conditions(flow, source)?;
            self.declare(flow, binding, result, source)?;
            self.install_value_origins(
                flow,
                &Place {
                    root: root(binding),
                    fields: vec![],
                },
                origins,
                source,
            )?;
            let destination = Place {
                root: root(binding),
                fields: vec![],
            };
            self.install_enum_domains(flow, &destination, conditions.domains, source)?;
            self.install_pointer_guards(flow, &destination, conditions.pointers, source)?;
            for condition in &function.postconditions {
                self.check_contract_predicate(condition, flow, depth + 1)?;
            }
            if flow.live {
                // Only after all checks succeed does the result leave this frame.
                flow.state
                    .take(&Place {
                        root: root(binding),
                        fields: vec![],
                    })
                    .map_err(|e| error(source, format!("result transfer: {e:?}")))?;
                self.remove_local(flow, root(binding), source)?;
            }
        } else {
            self.pending_frame(source, |this| {
                // An unnamed owned result still exists during the postchecks.
                let value = Expr {
                    source,
                    ty: (**result).clone(),
                    kind: K::Unit,
                };
                this.remember_pending_origins(&value, 0, flow, origins)?;
                for condition in &function.postconditions {
                    this.check_contract_predicate(condition, flow, depth + 1)?;
                }
                Ok(())
            })?;
        }
        for site in &mut self.report.cleanup_sites[first_site..] {
            site.result_origin = Some(source);
        }
        self.check_return_origins(flow, &delivered_origins, source)
    }
    fn sequence(&mut self, source: SourceRange) -> Result<u64, Error> {
        let sequence = self.next_sequence;
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| error(source, "cleanup sequence exhausted"))?;
        Ok(sequence)
    }
    fn value_conditions(
        &mut self,
        flow: &Flow,
        source: SourceRange,
    ) -> Result<ValueConditions, Error> {
        Ok(ValueConditions {
            domains: copy_enum_domains(&flow.value_variants, &mut self.fuel, source)?,
            pointers: self.copy_value_pointer_guards(&flow.value_pointer_guards, source)?,
        })
    }
    fn retain_temporary(
        &mut self,
        description: TemporaryCleanup,
        state: State,
        origins: ValueOrigins,
        conditions: ValueConditions,
        flow: &mut Flow,
    ) -> Result<(), Error> {
        self.acquire_cleanup(&description.ty, description.source)?;
        let id = self.sequence(description.source)?;
        flow.temporaries.insert(
            id,
            ActiveTemporary {
                conditions,
                alternatives: vec![],
                origins,
                variants: vec![],
                scope_id: self.temporary_scope_id,
                description,
                remaining: state,
                presence: Availability::Present,
                locals_before: flow.order.clone(),
            },
        );
        Ok(())
    }
    fn temporary_component(
        &mut self,
        ty: &Term,
        source: SourceRange,
        fields: &[usize],
        origins: ValueOrigins,
        flow: &mut Flow,
    ) -> Result<(), Error> {
        if *ty == Term::Primitive(nir::Primitive::Void) {
            return Ok(());
        }
        let description = TemporaryCleanup {
            scope: self.temporary_scope,
            source,
            ty: ty.clone(),
            fields: fields.to_vec(),
        };
        let mut state = State::default();
        flow.memory_exposed |= matches!(ty, Term::Pointer(_));
        let shape = (self.shape)(ty).map_err(|e| error(source, e))?;
        state
            .declare(
                PlaceId {
                    owner: u64::MAX,
                    local: 0,
                },
                &shape,
                true,
                &mut self.fuel,
            )
            .map_err(|e| error(source, format!("temporary shape: {e:?}")))?;
        self.report.temporaries.push(description.clone());
        let conditions = self.value_conditions(flow, source)?;
        self.retain_temporary(description, state, origins, conditions, flow)
    }
    fn temporary_action(
        &mut self,
        identity: u64,
        temporary: &ActiveTemporary,
        flow: &Flow,
        retired: &[Place],
        source: SourceRange,
    ) -> Result<CleanupAction, Error> {
        if temporary.alternatives.is_empty() {
            return self.temporary_layout_action(
                identity,
                temporary,
                &temporary.remaining,
                &temporary.variants,
                flow,
                retired,
                source,
            );
        }
        let mut actions = vec![];
        actions.push(self.temporary_layout_action(
            identity,
            temporary,
            &temporary.remaining,
            &temporary.variants,
            flow,
            retired,
            source,
        )?);
        for layout in &temporary.alternatives {
            actions.push(self.temporary_layout_action(
                identity,
                temporary,
                &layout.remaining,
                &layout.variants,
                flow,
                retired,
                source,
            )?);
        }
        if self.cleanup.is_none() {
            return Ok(CleanupAction::Alternatives(actions));
        }
        let mut candidates = vec![];
        let mut plans = vec![];
        for action in actions {
            let CleanupAction::Planned { candidate, plan } = action else {
                return Err(error(
                    source,
                    "residual layout lacks a checked cleanup plan",
                ));
            };
            candidates.push(*candidate);
            plans.push(*plan);
        }
        let plan = super::drop::CleanupPlan {
            receiver: plans[0].receiver.clone(),
            initial: None,
            variants: vec![],
            steps: vec![super::drop::CleanupStep::SelectVariantPlan { plans }],
        };
        // The executable target is one retained subject. Every layout's
        // initial tree and exact tag premises remain in the nested plans.
        Ok(CleanupAction::Planned {
            candidate: Box::new(CleanupAction::Alternatives(candidates)),
            plan: Box::new(plan),
        })
    }
    fn temporary_layout_action(
        &mut self,
        identity: u64,
        temporary: &ActiveTemporary,
        state: &State,
        variants: &[VariantSelection],
        flow: &Flow,
        retired: &[Place],
        source: SourceRange,
    ) -> Result<CleanupAction, Error> {
        let remaining = state
            .cleanup_tree(
                &Place {
                    root: PlaceId {
                        owner: u64::MAX,
                        local: 0,
                    },
                    fields: vec![],
                },
                &mut self.fuel,
            )
            .map_err(|e| error(source, format!("temporary cleanup: {e:?}")))?
            .ok_or_else(|| error(source, "temporary has no remaining owner"))?;
        let candidate = CleanupAction::Temporary {
            identity,
            temporary: temporary.description.clone(),
            presence: temporary.presence,
            variants: variants.to_vec(),
            remaining,
        };
        // Presence is external to the per-instance remaining-state plan.
        // Executable lowering must guard conditional instances by retained storage.
        self.plan_retained_action(
            candidate,
            &temporary.description.ty,
            state,
            variants,
            &temporary.origins,
            &temporary.conditions,
            flow,
            retired,
            source,
        )
    }
    fn plan_retained_action(
        &mut self,
        candidate: CleanupAction,
        ty: &Term,
        remaining: &State,
        variants: &[VariantSelection],
        origins: &ValueOrigins,
        conditions: &ValueConditions,
        flow: &Flow,
        retired: &[Place],
        source: SourceRange,
    ) -> Result<CleanupAction, Error> {
        let place = Place {
            root: PlaceId {
                owner: u64::MAX,
                local: 0,
            },
            fields: vec![],
        };
        if self.cleanup.is_none() {
            return Ok(candidate);
        }
        // Retained ownership is relative to the canonical temporary root;
        // pointee validity comes from the current surrounding frame.
        let mut state = flow
            .state
            .join(&flow.state, &mut self.fuel)
            .map_err(|e| error(source, format!("temporary memory snapshot: {e:?}")))?;
        state
            .import_position(place.root, remaining, &place, &mut self.fuel)
            .map_err(|e| error(source, format!("temporary memory import: {e:?}")))?;
        let mut types = BTreeMap::new();
        for (root, ty) in &flow.types {
            super::memory::bound_type(ty, &mut self.fuel).map_err(|e| error(source, e))?;
            types.insert(*root, ty.clone());
        }
        types.insert(place.root, ty.clone());
        let mut slots = BTreeMap::new();
        for (slot, targets) in &flow.pointers {
            let cost = targets
                .iter()
                .fold(slot.fields.len().saturating_add(1), |cost, target| {
                    cost.saturating_add(target.fields.len()).saturating_add(1)
                });
            self.fuel = self
                .fuel
                .checked_sub(cost)
                .ok_or_else(|| error(source, "temporary pointer snapshot budget exhausted"))?;
            slots.insert(slot.clone(), targets.clone());
        }
        for (fields, targets) in origins {
            let cost = targets
                .iter()
                .fold(fields.len().saturating_add(1), |cost, target| {
                    cost.saturating_add(target.fields.len()).saturating_add(1)
                });
            self.fuel = self
                .fuel
                .checked_sub(cost)
                .ok_or_else(|| error(source, "temporary pointer origin budget exhausted"))?;
            slots.insert(
                Place {
                    root: place.root,
                    fields: fields.clone(),
                },
                targets.clone(),
            );
        }
        let mut enum_domains = BTreeMap::new();
        for (root, domains) in &flow.enum_domains {
            enum_domains.insert(*root, copy_enum_domains(domains, &mut self.fuel, source)?);
        }
        enum_domains.insert(
            place.root,
            copy_enum_domains(&conditions.domains, &mut self.fuel, source)?,
        );
        let mut pointer_conditions = BTreeMap::new();
        for (slot, entries) in &flow.pointer_guards {
            for (target, guards) in entries {
                self.fuel = self
                    .fuel
                    .checked_sub(slot.fields.len() + target.fields.len() + 1)
                    .ok_or_else(|| {
                        error(source, "temporary condition snapshot budget exhausted")
                    })?;
                for (path, _) in guards {
                    self.fuel = self.fuel.checked_sub(path.len() + 1).ok_or_else(|| {
                        error(source, "temporary condition snapshot budget exhausted")
                    })?;
                }
            }
            pointer_conditions.insert(slot.clone(), entries.clone());
        }
        for (fields, entries) in &conditions.pointers {
            for (target, guards) in entries {
                self.fuel = self
                    .fuel
                    .checked_sub(fields.len() + target.fields.len() + 1)
                    .ok_or_else(|| error(source, "retained pointer condition budget exhausted"))?;
                for (path, _) in guards {
                    self.fuel = self.fuel.checked_sub(path.len() + 1).ok_or_else(|| {
                        error(source, "retained pointer condition budget exhausted")
                    })?;
                }
            }
            pointer_conditions.insert(
                Place {
                    root: place.root,
                    fields: fields.clone(),
                },
                entries.clone(),
            );
        }
        let mut snapshot = Flow {
            state,
            types,
            pointers: slots,
            enum_domains,
            pointer_guards: pointer_conditions,
            borrowed_scope: flow.borrowed_scope,
            ..Flow::default()
        };
        self.prepare_cleanup_origins(&mut snapshot, place.root, ty, source)?;
        let plan = self
            .cleanup
            .as_mut()
            .expect("cleanup policy was checked")
            .plan_with_memory(
                ty,
                &snapshot.state,
                &place,
                variants,
                Some(super::drop::PointerMemory {
                    types: &snapshot.types,
                    slots: &snapshot.pointers,
                    enum_domains: &snapshot.enum_domains,
                    pointer_conditions: &snapshot.pointer_guards,
                    retired,
                }),
                &mut self.fuel,
            )
            .map_err(|e| error(source, e))?;
        Ok(CleanupAction::Planned {
            candidate: Box::new(candidate),
            plan: Box::new(plan),
        })
    }
    fn finish_temporaries(
        &mut self,
        flow: &mut Flow,
        source: SourceRange,
        scope_id: u64,
    ) -> Result<(), Error> {
        let ids: Vec<_> = flow
            .temporaries
            .iter()
            .rev()
            .filter(|(_, t)| t.scope_id == scope_id)
            .map(|(id, _)| *id)
            .collect();
        let mut actions = vec![];
        for id in &ids {
            actions.push(self.temporary_action(*id, &flow.temporaries[id], flow, &[], source)?);
        }
        for id in ids {
            flow.temporaries.remove(&id);
        }
        if !actions.is_empty() {
            self.report.cleanup_sites.push(CleanupSite {
                default_origin: None,
                result_origin: None,
                interface_precondition: false,
                interface_postcondition: false,
                source,
                reason: CleanupReason::TemporaryEnd,
                actions,
            });
        }
        Ok(())
    }
    fn owned_action(
        &mut self,
        state: &State,
        place: Place,
        ty: &Term,
        pointers: super::drop::PointerMemory<'_>,
        variants: &[VariantSelection],
        domains: &EnumDomains,
        source: SourceRange,
    ) -> Result<Option<CleanupAction>, Error> {
        let Some(remaining) = state
            .cleanup_tree(&place, &mut self.fuel)
            .map_err(|e| error(source, format!("cleanup hierarchy: {e:?}")))?
        else {
            return Ok(None);
        };
        let candidate = CleanupAction::Owned {
            ty: ty.clone(),
            remaining,
        };
        let Some(policy) = self.cleanup.as_mut() else {
            return Ok(Some(candidate));
        };
        let plan = policy
            .plan_with_domains(
                ty,
                state,
                &place,
                variants,
                domains,
                Some(pointers),
                &mut self.fuel,
            )
            .map_err(|e| error(source, e))?;
        Ok(Some(CleanupAction::Planned {
            candidate: Box::new(candidate),
            plan: Box::new(plan),
        }))
    }

    // A nested tag is unconditional only when every enclosing variant is
    // known. Callable evidence uses qualified paths; cleanup uses physical fields.
    fn cleanup_variants(
        &mut self,
        flow: &Flow,
        place: &Place,
        source: SourceRange,
    ) -> Result<(Vec<VariantSelection>, EnumDomains), Error> {
        let domains = self.stored_enum_domains(flow, place, source)?;
        let mut selections = vec![];
        let mut allowed = EnumDomains::new();
        for (path, tags) in &domains {
            self.fuel = self
                .fuel
                .checked_sub(path.len() + tags.len() + 1)
                .ok_or_else(|| error(source, "cleanup variant budget exhausted"))?;
            if tags.is_empty() {
                continue;
            }
            let mut fields = vec![];
            let mut index = 0;
            let mut established = true;
            while index < path.len() {
                if path[index] == VARIANT_PATH {
                    let Some(tag) = path.get(index + 1) else {
                        established = false;
                        break;
                    };
                    if !domains
                        .get(&path[..index])
                        .is_some_and(|tags| tags.len() == 1 && tags.contains(tag))
                    {
                        established = false;
                        break;
                    }
                    index += 2;
                } else {
                    fields.push(path[index]);
                    index += 1;
                }
            }
            if established {
                if tags.len() == 1 {
                    selections.push(VariantSelection {
                        fields,
                        index: *tags.first().unwrap(),
                    });
                } else {
                    allowed.insert(fields, tags.clone());
                }
            } else {
                // Retain the enclosing-tag conditions instead of treating this
                // inner domain as unconditional physical-field evidence.
                allowed.insert(path.clone(), tags.clone());
            }
        }
        Ok((selections, allowed))
    }

    fn local_action(
        &mut self,
        flow: &mut Flow,
        id: PlaceId,
        retired: &[Place],
        source: SourceRange,
    ) -> Result<Option<CleanupAction>, Error> {
        let ty = flow
            .types
            .get(&id)
            .ok_or_else(|| error(source, "missing cleanup local type"))?
            .clone();
        self.prepare_cleanup_origins(flow, id, &ty, source)?;
        let mut possible_retired = retired.to_vec();
        if let Some(before) = flow.cleanup_before.get(&id) {
            self.fuel = self
                .fuel
                .checked_sub(before.len().saturating_add(1))
                .ok_or_else(|| error(source, "conditional cleanup dependency budget exhausted"))?;
            possible_retired.extend(before.iter().map(|id| Place {
                root: *id,
                fields: vec![],
            }));
        }
        let (variants, domains) = self.cleanup_variants(
            flow,
            &Place {
                root: id,
                fields: vec![],
            },
            source,
        )?;
        self.owned_action(
            &flow.state,
            Place {
                root: id,
                fields: vec![],
            },
            &ty,
            super::drop::PointerMemory {
                types: &flow.types,
                slots: &flow.pointers,
                enum_domains: &flow.enum_domains,
                pointer_conditions: &flow.pointer_guards,
                retired: &possible_retired,
            },
            &variants,
            &domains,
            source,
        )
    }
    fn cleanup_exit(
        &mut self,
        flow: &mut Flow,
        locals: Vec<PlaceId>,
        pending_depth: usize,
        temporary_floor: u64,
        source: SourceRange,
        reason: CleanupReason,
    ) -> Result<(), Error> {
        let mut events: Vec<_> = self.pending[pending_depth..]
            .iter()
            .flat_map(|frame| {
                frame.values.iter().map(|value| {
                    (
                        value.sequence,
                        value.locals_before.clone(),
                        Some(CleanupAction::Prepared {
                            construction: frame.source,
                            value: value.clone(),
                        }),
                    )
                })
            })
            .collect();
        let temp_ids: Vec<_> = flow
            .temporaries
            .range(temporary_floor..)
            .map(|(id, _)| *id)
            .collect();
        for id in &temp_ids {
            let temporary = &flow.temporaries[id];
            events.push((*id, temporary.locals_before.clone(), None));
        }
        events.sort_by_key(|(id, _, _)| std::cmp::Reverse(*id));
        let mut locals = locals;
        let mut actions = vec![];
        let mut retired = vec![];
        for (identity, anchors, action) in events {
            while locals.last().is_some_and(|id| !anchors.contains(id)) {
                let id = locals.pop().unwrap();
                if let Some(local) = self.local_action(flow, id, &retired, source)? {
                    actions.push(local);
                }
                retired.push(Place {
                    root: id,
                    fields: vec![],
                });
            }
            let mut action = match action {
                Some(action) => action,
                None => self.temporary_action(
                    identity,
                    &flow.temporaries[&identity],
                    flow,
                    &retired,
                    source,
                )?,
            };
            if self.cleanup.is_some() {
                if let CleanupAction::Prepared { value, .. } = &action {
                    let ty = value.ty.clone();
                    let shape = (self.shape)(&ty).map_err(|e| error(source, e))?;
                    let place = Place {
                        root: PlaceId {
                            owner: u64::MAX,
                            local: 0,
                        },
                        fields: vec![],
                    };
                    let mut state = State::default();
                    state
                        .declare(place.root, &shape, true, &mut self.fuel)
                        .map_err(|e| error(source, format!("prepared cleanup shape: {e:?}")))?;
                    let origins = self
                        .pending
                        .iter()
                        .find_map(|frame| frame.origins.get(&value.sequence))
                        .cloned()
                        .unwrap_or_default();
                    let conditions = self
                        .pending
                        .iter()
                        .find_map(|frame| frame.conditions.get(&value.sequence))
                        .cloned()
                        .unwrap_or_default();
                    action = self.plan_retained_action(
                        action,
                        &ty,
                        &state,
                        &[],
                        &origins,
                        &conditions,
                        flow,
                        &retired,
                        source,
                    )?;
                }
            }
            actions.push(action);
        }
        for id in locals.into_iter().rev() {
            if let Some(action) = self.local_action(flow, id, &retired, source)? {
                actions.push(action);
            }
            retired.push(Place {
                root: id,
                fields: vec![],
            });
        }
        for id in temp_ids {
            flow.temporaries.remove(&id);
        }
        if !actions.is_empty() {
            self.report.cleanup_sites.push(CleanupSite {
                default_origin: None,
                result_origin: None,
                interface_precondition: false,
                interface_postcondition: false,
                source,
                reason,
                actions,
            });
        }
        Ok(())
    }
    fn pending_frame<T>(
        &mut self,
        source: SourceRange,
        run: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        self.pending.push(PendingFrame {
            conditions: BTreeMap::new(),
            origins: BTreeMap::new(),
            source,
            values: vec![],
        });
        let result = run(self);
        self.pending.pop().expect("active preparation frame");
        result
    }
    fn remember_pending_origins(
        &mut self,
        expr: &Expr,
        slot: usize,
        flow: &Flow,
        origins: ValueOrigins,
    ) -> Result<(), Error> {
        let ty = completion_type(&expr.ty);
        if flow.live && *ty != Term::Primitive(nir::Primitive::Void) {
            self.acquire_cleanup(ty, expr.source)?;
            for (path, targets) in &origins {
                self.fuel = self
                    .fuel
                    .checked_sub(path.len().saturating_add(1))
                    .ok_or_else(|| {
                        error(expr.source, "prepared pointer origin budget exhausted")
                    })?;
                self.pointer_work(targets, expr.source)?;
            }
            let sequence = self.sequence(expr.source)?;
            let conditions = self.value_conditions(flow, expr.source)?;
            self.pending
                .last_mut()
                .expect("active preparation frame")
                .conditions
                .insert(sequence, conditions);
            self.pending
                .last_mut()
                .expect("active preparation frame")
                .origins
                .insert(sequence, origins);
            self.pending
                .last_mut()
                .expect("active preparation frame")
                .values
                .push(PendingValue {
                    sequence,
                    source: expr.source,
                    scope: self.temporary_scope,
                    ty: ty.clone(),
                    slot,
                    locals_before: flow.order.clone(),
                });
        }
        Ok(())
    }
    fn incomplete_exit(&mut self, source: SourceRange, depth: usize) {
        // Observation only: conditional exits must not consume sibling paths'
        // preparation state. Loop edges preserve frames outside their loop.
        for frame in self.pending[depth..].iter().rev() {
            if !frame.values.is_empty() {
                self.report.incomplete.push(IncompleteCleanup {
                    exit: source,
                    construction: frame.source,
                    values: frame.values.iter().rev().cloned().collect(),
                });
            }
        }
    }
    fn check_interface_preparation(
        &mut self,
        callee: &Expr,
        flow: &Flow,
        source: SourceRange,
    ) -> Result<(), Error> {
        if !flow.live {
            return Ok(());
        }
        let K::TraitMethod {
            interface, member, ..
        } = &callee.kind
        else {
            return Ok(());
        };
        let package = self
            .package
            .ok_or_else(|| error(source, "interface preparation requires a package"))?;
        let contract = package
            .traits
            .iter()
            .find(|t| t.declaration == *interface)
            .and_then(|t| t.methods.iter().find(|m| m.declaration == *member))
            .ok_or_else(|| error(source, "interface preparation lacks its method contract"))?;
        if contract
            .program
            .preconditions
            .iter()
            .any(|condition| constant_bool(condition, 0) != Some(true))
        {
            let mut failed = flow.clone();
            let first = self.report.cleanup_sites.len();
            self.exit(&mut failed, source, CleanupReason::PredicateFailure)?;
            for site in &mut self.report.cleanup_sites[first..] {
                site.interface_precondition = true;
            }
        }
        Ok(())
    }
    /// On interface rejection the input frame has transferred, but the produced
    /// result has not. Analyze this alternative without consuming the success
    /// path; outer pending constructions and caller locals still unwind normally.
    fn check_interface_result(
        &mut self,
        callee: &Expr,
        result: &Expr,
        flow: &Flow,
    ) -> Result<(), Error> {
        if !flow.live {
            return Ok(());
        }
        let K::TraitMethod {
            interface, member, ..
        } = &callee.kind
        else {
            return Ok(());
        };
        let package = self
            .package
            .ok_or_else(|| error(result.source, "interface result requires a package"))?;
        let contract = package
            .traits
            .iter()
            .find(|t| t.declaration == *interface)
            .and_then(|t| t.methods.iter().find(|m| m.declaration == *member))
            .ok_or_else(|| error(result.source, "interface result lacks its method contract"))?;
        if contract.program.postconditions.is_empty() {
            return Ok(());
        }
        let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
            return Err(error(result.source, "interface result lacks a signature"));
        };
        let slot = parameters.len();
        let mut failed = flow.clone();
        let first = self.report.cleanup_sites.len();
        self.pending_frame(result.source, |this| {
            this.remember_pending_origins(result, slot, &failed, BTreeMap::new())?;
            this.exit(&mut failed, result.source, CleanupReason::PredicateFailure)
        })?;
        if self.report.cleanup_sites.len() == first {
            self.report.cleanup_sites.push(CleanupSite {
                interface_precondition: false,
                interface_postcondition: true,
                result_origin: None,
                default_origin: None,
                source: result.source,
                reason: CleanupReason::PredicateFailure,
                actions: vec![],
            });
        }
        for site in &mut self.report.cleanup_sites[first..] {
            site.interface_postcondition = true;
        }
        Ok(())
    }
    fn prepare_values<'b>(
        &mut self,
        source: SourceRange,
        values: impl IntoIterator<Item = (usize, &'b Expr)>,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<(), Error> {
        self.pending_frame(source, |this| {
            for (slot, value) in values {
                let origins = this.evaluate_value_origins(value, flow, depth + 1)?;
                if !flow.live {
                    break;
                }
                this.remember_pending_origins(value, slot, flow, origins)?;
            }
            Ok(())
        })
    }
    fn in_temporary_scope<T>(
        &mut self,
        source: SourceRange,
        boundary: TemporaryBoundary,
        flow: &mut Flow,
        run: impl FnOnce(&mut Self, &mut Flow) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let id = self.sequence(source)?;
        let previous_id = std::mem::replace(&mut self.temporary_scope_id, id);
        let previous = std::mem::replace(
            &mut self.temporary_scope,
            TemporaryScope { source, boundary },
        );
        let result = run(self, flow);
        let cleanup = if result.is_ok() && flow.live {
            self.finish_temporaries(flow, source, id)
        } else {
            Ok(())
        };
        self.temporary_scope = previous;
        self.temporary_scope_id = previous_id;
        result.and_then(|value| {
            cleanup?;
            Ok(value)
        })
    }
    fn full_expression(&mut self, expr: &Expr, flow: &mut Flow, depth: usize) -> Result<(), Error> {
        self.in_temporary_scope(
            expr.source,
            TemporaryBoundary::FullExpression,
            flow,
            |this, flow| {
                let origins = this.evaluate_value_origins(expr, flow, depth)?;
                if flow.live {
                    this.temporary(expr, origins, flow)?;
                }
                Ok(())
            },
        )
    }
    fn fresh_borrowed_scope(&mut self, source: SourceRange) -> Result<u64, Error> {
        let scope = self.next_borrowed_scope;
        self.next_borrowed_scope = scope
            .checked_add(1)
            .ok_or_else(|| error(source, "borrowed context identity exhausted"))?;
        Ok(scope)
    }
    fn complete_borrowed_premises(
        &mut self,
        flow: &mut Flow,
        source: SourceRange,
    ) -> Result<(), Error> {
        // These are conditional validity premises, not local acquisitions.
        // Every caller must discharge the complete body-derived footprint.
        for ((scope, slot, guards), (pointee, subject)) in &self.borrowed_pointees {
            for (path, _) in guards {
                self.fuel = self
                    .fuel
                    .checked_sub(path.len() + 1)
                    .ok_or_else(|| error(source, "borrowed condition budget exhausted"))?;
            }
            self.fuel = self
                .fuel
                .checked_sub(slot.fields.len().saturating_add(1))
                .ok_or_else(|| error(source, "borrowed premise join budget exhausted"))?;
            if *scope != flow.borrowed_scope {
                continue;
            }
            if let Some(actual) = flow.types.get(pointee) {
                if actual != subject {
                    return Err(error(source, "borrowed pointer premise changed type"));
                }
            } else {
                super::memory::bound_type(subject, &mut self.fuel).map_err(|e| error(source, e))?;
                let shape = (self.shape)(subject).map_err(|e| error(source, e))?;
                flow.state
                    .declare(*pointee, &shape, true, &mut self.fuel)
                    .map_err(|e| error(source, format!("borrowed premise join shape: {e:?}")))?;
                flow.types.insert(*pointee, subject.clone());
            }
            if !guards.is_empty() {
                continue;
            }
            flow.pointers.entry(slot.clone()).or_insert_with(|| {
                BTreeSet::from([Place {
                    root: *pointee,
                    fields: vec![],
                }])
            });
        }
        Ok(())
    }
    fn join_flow(
        &mut self,
        left: &mut Flow,
        right: &Flow,
        source: SourceRange,
    ) -> Result<(), Error> {
        if left.borrowed_scope != right.borrowed_scope || left.order != right.order {
            return Err(error(source, "ownership scope declaration order mismatch"));
        }
        self.complete_borrowed_premises(left, source)?;
        self.fuel = self
            .fuel
            .checked_sub(self.borrowed_pointees.len())
            .ok_or_else(|| error(source, "borrowed premise join budget exhausted"))?;
        let needs_completion =
            self.borrowed_pointees
                .iter()
                .any(|((scope, slot, guards), (pointee, _))| {
                    *scope == right.borrowed_scope
                        && (!right.types.contains_key(pointee)
                            || (guards.is_empty() && !right.pointers.contains_key(slot)))
                });
        let completed_right;
        let right = if needs_completion {
            for ty in right.types.values() {
                super::memory::bound_type(ty, &mut self.fuel).map_err(|e| error(source, e))?;
            }
            for targets in right.pointers.values() {
                self.pointer_work(targets, source)?;
            }
            let mut completed = right.clone();
            self.complete_borrowed_premises(&mut completed, source)?;
            completed_right = completed;
            &completed_right
        } else {
            right
        };
        if left.types != right.types {
            return Err(error(source, "ownership scope declaration order mismatch"));
        }
        self.fuel = self
            .fuel
            .checked_sub(left.effect_borrows.len())
            .ok_or_else(|| error(source, "effect borrow join budget exhausted"))?;
        left.effect_borrows
            .retain(|root| right.effect_borrows.contains(root));
        self.fuel = self
            .fuel
            .checked_sub(left.stable_handler_slots.len())
            .ok_or_else(|| error(source, "handler slot join budget exhausted"))?;
        left.stable_handler_slots
            .retain(|root| right.stable_handler_slots.contains(root));
        // Pointer joins need each branch's original construction state.
        self.join_pointer_origins(left, right, source)?;
        left.state = left
            .state
            .join(&right.state, &mut self.fuel)
            .map_err(|e| error(source, format!("ownership join: {e:?}")))?;
        left.value_callable = match (&left.value_callable, &right.value_callable) {
            (Some(a), Some(b))
                if same_callable_definition(&a.definition, &b.definition)
                    && a.choices == b.choices
                    && a.alternatives == b.alternatives
                    && a.captures == b.captures
                    && a.enum_captures == b.enum_captures =>
            {
                Some(a.clone())
            }
            (Some(a), Some(b)) => merge_callable_provenance(a, b, &mut self.fuel, source, 0)?,
            _ => None,
        };
        left.value_callables = join_guarded_callables(
            &left.value_callables,
            &right.value_callables,
            &left.value_variants,
            &right.value_variants,
            &mut self.fuel,
            source,
        )?;
        left.value_variants = join_enum_domains(
            &left.value_variants,
            &right.value_variants,
            &mut self.fuel,
            source,
        )?;
        left.memory_exposed |= right.memory_exposed;
        for (id, before) in &right.cleanup_before {
            self.fuel = self
                .fuel
                .checked_sub(before.len().saturating_add(1))
                .ok_or_else(|| error(source, "conditional cleanup order budget exhausted"))?;
            left.cleanup_before.entry(*id).or_default().extend(before);
        }
        let mut callables = BTreeMap::new();
        for (id, value) in &left.callables {
            if let Some(other) = right.callables.get(id) {
                let merged = if value == other {
                    Some(value.clone())
                } else {
                    merge_callable_provenance(value, other, &mut self.fuel, source, 0)?
                };
                if let Some(merged) = merged {
                    callables.insert(*id, merged);
                }
            }
        }
        left.callables = callables;
        let mut fields = BTreeMap::new();
        let mut domains = BTreeMap::new();
        let unknown = EnumDomains::new();
        let empty_fields = BTreeMap::new();
        let left_fields = group_owned_callables(&left.callable_fields, &mut self.fuel, source)?;
        let right_fields = group_owned_callables(&right.callable_fields, &mut self.fuel, source)?;
        for root in &left.roots {
            let a = left_fields.get(root).unwrap_or(&empty_fields);
            let b = right_fields.get(root).unwrap_or(&empty_fields);
            let at = left.enum_domains.get(root).unwrap_or(&unknown);
            let bt = right.enum_domains.get(root).unwrap_or(&unknown);
            for (path, callable) in join_guarded_callables(a, b, at, bt, &mut self.fuel, source)? {
                fields.insert(
                    Place {
                        root: *root,
                        fields: path,
                    },
                    callable,
                );
            }
            let joined = join_enum_domains(at, bt, &mut self.fuel, source)?;
            if !joined.is_empty() {
                domains.insert(*root, joined);
            }
        }
        for (slot, guards) in &mut left.pointer_guards {
            let mut retained = BTreeMap::new();
            for (target, conditions) in guards.iter() {
                self.charge_pointer_conditions(conditions, source)?;
                if right
                    .pointer_guards
                    .get(slot)
                    .and_then(|entries| entries.get(target))
                    == Some(conditions)
                {
                    retained.insert(target.clone(), conditions.clone());
                }
            }
            *guards = retained;
        }
        left.pointer_guards.retain(|_, guards| !guards.is_empty());
        left.callable_fields = fields;
        left.enum_domains = domains;
        for (id, temporary) in &mut left.temporaries {
            match right.temporaries.get(id) {
                Some(other) => {
                    if temporary.scope_id != other.scope_id
                        || temporary.description != other.description
                        || temporary.remaining != other.remaining
                        || temporary.alternatives != other.alternatives
                        || temporary.locals_before != other.locals_before
                        || temporary.variants != other.variants
                        || temporary.origins != other.origins
                        || temporary.conditions != other.conditions
                    {
                        return Err(error(source, "temporary identity has inconsistent state"));
                    }
                    if temporary.presence != other.presence {
                        temporary.presence = Availability::Conditional;
                    }
                }
                None => temporary.presence = Availability::Conditional,
            }
        }
        for (id, temporary) in &right.temporaries {
            if !left.temporaries.contains_key(id) {
                let mut temporary = temporary.clone();
                temporary.presence = Availability::Conditional;
                left.temporaries.insert(*id, temporary);
            }
        }
        Ok(())
    }
    fn loop_edge(
        &mut self,
        identity: LocalId,
        is_break: bool,
        flow: &mut Flow,
        source: SourceRange,
    ) -> Result<(), Error> {
        let index = self
            .loops
            .iter()
            .rposition(|frame| frame.identity == identity);
        let Some(index) = index else {
            if self.handler_result.is_none() {
                return Err(error(source, "ownership loop target is not active"));
            }
            self.report.lexical_loop_exits.insert((identity, is_break));
            // The handler owns only its private arm storage. The lexical
            // target's loop boundary is discharged at the suspended installer.
            self.exit(
                flow,
                source,
                if is_break {
                    CleanupReason::Break
                } else {
                    CleanupReason::Continue
                },
            )?;
            return Ok(());
        };
        self.incomplete_exit(source, self.loops[index].pending_depth);
        let locals: Vec<_> = flow
            .order
            .iter()
            .rev()
            .filter(|id| !self.loops[index].roots.contains(id))
            .copied()
            .collect();
        self.cleanup_exit(
            flow,
            locals.iter().rev().copied().collect(),
            self.loops[index].pending_depth,
            self.loops[index].temporary_floor,
            source,
            if is_break {
                CleanupReason::Break
            } else {
                CleanupReason::Continue
            },
        )?;
        for local in locals {
            self.remove_local(flow, local, source)?;
        }
        let edge = flow.clone();
        if is_break {
            self.loops[index].breaks.push(edge);
        } else {
            self.loops[index].continues.push(edge);
        }
        flow.live = false;
        Ok(())
    }
    fn take_nested_report(&mut self, source: SourceRange) -> Result<Report, Error> {
        let borrowed_roots =
            cleanup_memory::copy_roots(&self.report.borrowed_roots, &mut self.fuel)
                .map_err(|reason| error(source, reason))?;
        Ok(std::mem::replace(
            &mut self.report,
            Report {
                borrowed_roots,
                ..Report::default()
            },
        ))
    }

    fn loop_body(
        &mut self,
        identity: LocalId,
        body: &Expr,
        flow: &mut Flow,
        source: SourceRange,
        depth: usize,
    ) -> Result<(), Error> {
        let entry = flow.clone();
        let mut header = entry.clone();
        let enclosing_loops = self.loops.clone();
        loop {
            if self.fuel == 0 || depth > 128 {
                return Err(error(source, "ownership loop fixed point budget exhausted"));
            }
            self.fuel -= 1;
            // Speculative rounds must not publish cleanup or nonlocal edges.
            self.loops = enclosing_loops.clone();
            self.loops.push(LoopFrame {
                temporary_floor: self.next_sequence,
                pending_depth: self.pending.len(),
                identity,
                roots: header.roots.clone(),
                breaks: vec![],
                continues: vec![],
            });
            let outer_report = self.take_nested_report(source)?;
            let mut current = header.clone();
            let result = self.in_temporary_scope(
                body.source,
                TemporaryBoundary::FullExpression,
                &mut current,
                |this, current| {
                    let origins = this.scope_origins(body, current, depth + 1)?;
                    if current.live {
                        this.temporary(body, origins, current)?;
                    }
                    Ok(())
                },
            );
            let mut iteration = std::mem::replace(&mut self.report, outer_report);
            let mut frame = self.loops.pop().expect("active loop frame");
            result?;
            if current.live {
                frame.continues.push(current);
            }
            let mut next = entry.clone();
            for back in &frame.continues {
                self.join_flow(&mut next, back, source)?;
            }
            if next.state != header.state
                || next.memory_exposed != header.memory_exposed
                || next.pointers != header.pointers
                || next.enum_domains != header.enum_domains
                || next.callable_fields != header.callable_fields
                || next.callables != header.callables
                || next.temporaries != header.temporaries
                || next.cleanup_before != header.cleanup_before
            {
                // Origin identities are immutable metadata, not effects. A
                // backedge may retain a pointee created in this round, so keep
                // its origin while discarding speculative execution records.
                self.report
                    .borrowed_roots
                    .append(&mut iteration.borrowed_roots);
                header = next;
                continue;
            }
            iteration.loops.push(LoopReport {
                source,
                header: header.state,
                breaks: frame.breaks.iter().map(|f| f.state.clone()).collect(),
            });
            self.report.append(iteration);
            let mut breaks = frame.breaks.into_iter();
            if let Some(mut exit) = breaks.next() {
                for other in breaks {
                    self.join_flow(&mut exit, &other, source)?;
                }
                *flow = exit;
            } else {
                flow.live = false;
            }
            return Ok(());
        }
    }
    fn store_callable(
        &mut self,
        flow: &mut Flow,
        id: PlaceId,
        mut callable: Callable,
        source: SourceRange,
    ) -> Result<(), Error> {
        let generation = self.next_generation;
        self.next_generation = generation
            .checked_add(1)
            .ok_or_else(|| error(source, "closure generation exhausted"))?;
        callable.generation = if matches!(
            callable.definition.kind,
            K::Function(_) | K::FunctionInstance { .. }
        ) {
            0
        } else {
            generation
        };
        flow.callables.insert(id, callable);
        Ok(())
    }
    fn forget_callable_fields(
        &mut self,
        flow: &mut Flow,
        destination: &Place,
        source: SourceRange,
    ) -> Result<(), Error> {
        let mut remove = vec![];
        for place in flow.callable_fields.keys() {
            self.fuel = self
                .fuel
                .checked_sub(place.fields.len().saturating_add(1))
                .ok_or_else(|| error(source, "callable field budget exhausted"))?;
            if place.root == destination.root && place.fields.starts_with(&destination.fields) {
                remove.push(place.clone());
            }
        }
        for place in remove {
            flow.callable_fields.remove(&place);
        }
        if let Some(domains) = flow.enum_domains.get_mut(&destination.root) {
            domains.retain(|path, _| !path.starts_with(&destination.fields));
            if domains.is_empty() {
                flow.enum_domains.remove(&destination.root);
            }
        }
        Ok(())
    }
    fn stored_enum_domains(
        &mut self,
        flow: &Flow,
        place: &Place,
        source: SourceRange,
    ) -> Result<EnumDomains, Error> {
        let mut domains = EnumDomains::new();
        if let Some(stored) = flow.enum_domains.get(&place.root) {
            for (path, tags) in stored {
                self.fuel = self
                    .fuel
                    .checked_sub(path.len().saturating_add(tags.len()).saturating_add(1))
                    .ok_or_else(|| error(source, "enum domain delivery budget exhausted"))?;
                if let Some(relative) = path.strip_prefix(place.fields.as_slice()) {
                    domains.insert(relative.to_vec(), tags.clone());
                }
            }
        }
        Ok(domains)
    }
    fn stored_code(
        &mut self,
        flow: &Flow,
        place: &Place,
        source: SourceRange,
    ) -> Result<BTreeMap<Vec<usize>, Callable>, Error> {
        let mut code = BTreeMap::new();
        if place.fields.is_empty() {
            if let Some(callable) = flow.callables.get(&place.root) {
                bound_callable_copy(callable, &mut self.fuel, source, 0)?;
                code.insert(vec![], callable.clone());
            }
        }
        for (slot, callable) in &flow.callable_fields {
            self.fuel = self
                .fuel
                .checked_sub(slot.fields.len().saturating_add(1))
                .ok_or_else(|| error(source, "code pointer delivery budget exhausted"))?;
            if slot.root == place.root {
                if let Some(relative) = slot.fields.strip_prefix(place.fields.as_slice()) {
                    bound_callable_copy(callable, &mut self.fuel, source, 0)?;
                    code.insert(relative.to_vec(), callable.clone());
                }
            }
        }
        Ok(code)
    }
    fn install_enum_domains(
        &mut self,
        flow: &mut Flow,
        destination: &Place,
        domains: EnumDomains,
        source: SourceRange,
    ) -> Result<(), Error> {
        if flow.effect_mutable_roots.contains(&destination.root) {
            flow.enum_domains.remove(&destination.root);
            return Ok(());
        }
        let stored = flow.enum_domains.entry(destination.root).or_default();
        stored.retain(|path, _| !path.starts_with(&destination.fields));
        for (path, tags) in domains {
            self.fuel = self
                .fuel
                .checked_sub(
                    path.len()
                        .saturating_add(destination.fields.len())
                        .saturating_add(tags.len())
                        .saturating_add(1),
                )
                .ok_or_else(|| error(source, "enum domain storage budget exhausted"))?;
            let mut nested = destination.fields.clone();
            nested.extend(path);
            stored.insert(nested, tags);
        }
        if stored.is_empty() {
            flow.enum_domains.remove(&destination.root);
        }
        Ok(())
    }
    fn install_callable_fields(
        &mut self,
        flow: &mut Flow,
        destination: &Place,
        leaves: BTreeMap<Vec<usize>, Callable>,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.forget_callable_fields(flow, destination, source)?;
        for (path, mut callable) in leaves {
            self.fuel = self
                .fuel
                .checked_sub(
                    path.len()
                        .saturating_add(destination.fields.len())
                        .saturating_add(1),
                )
                .ok_or_else(|| error(source, "callable field budget exhausted"))?;
            let mut place = destination.clone();
            place.fields.extend(path);
            if place.fields.is_empty() {
                continue;
            }
            if !matches!(
                callable.definition.kind,
                K::Function(_) | K::FunctionInstance { .. }
            ) {
                callable.generation = self.next_generation;
                self.next_generation = self
                    .next_generation
                    .checked_add(1)
                    .ok_or_else(|| error(source, "closure generation exhausted"))?;
            }
            flow.callable_fields.insert(place, callable);
        }
        Ok(())
    }
    fn field_type(
        &mut self,
        ty: &Term,
        path: &[usize],
        source: SourceRange,
    ) -> Result<Term, Error> {
        owned_field_type(self.package, ty, path, &mut self.fuel, source)
    }
    fn project_temporary(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        let mut base = expr;
        let mut path = vec![];
        while let K::AnonymousField { value, index, .. } | K::Field { value, index, .. } =
            &base.kind
        {
            if self.fuel == 0 || path.len() >= 128 {
                return Err(error(expr.source, "temporary projection budget exhausted"));
            }
            self.fuel -= 1;
            path.push(*index);
            base = value;
        }
        path.reverse();
        if path.is_empty() {
            return Err(error(expr.source, "temporary projection has no field path"));
        }
        let field_type = self.field_type(&base.ty, &path, expr.source)?;
        if field_type != expr.ty {
            return Err(error(
                expr.source,
                "temporary projection field type mismatch",
            ));
        }
        let origins = self.evaluate_value_origins(base, flow, depth + 1)?;
        let parent_conditions = self.value_conditions(flow, expr.source)?;
        if !flow.live {
            return Ok(BTreeMap::new());
        }
        let mut selected_callables = BTreeMap::new();
        for (leaf, callable) in std::mem::take(&mut flow.value_callables) {
            self.fuel = self
                .fuel
                .checked_sub(leaf.len().saturating_add(1))
                .ok_or_else(|| error(expr.source, "callable projection budget exhausted"))?;
            if leaf.starts_with(&path) {
                selected_callables.insert(leaf[path.len()..].to_vec(), callable);
            }
        }
        let mut selected_guards = ValuePointerGuards::new();
        for (slot, entries) in std::mem::take(&mut flow.value_pointer_guards) {
            self.fuel = self
                .fuel
                .checked_sub(slot.len().saturating_add(1))
                .ok_or_else(|| {
                    error(
                        expr.source,
                        "temporary pointer guard projection budget exhausted",
                    )
                })?;
            let Some(relative) = slot.strip_prefix(path.as_slice()) else {
                continue;
            };
            let mut projected = PointerTargetGuards::new();
            for (target, conditions) in entries {
                self.charge_pointer_conditions(&conditions, expr.source)?;
                let conditions: Vec<_> = conditions
                    .into_iter()
                    .filter_map(|(condition, tag)| {
                        condition
                            .strip_prefix(path.as_slice())
                            .map(|rest| (rest.to_vec(), tag))
                    })
                    .collect();
                if !conditions.is_empty() {
                    projected.insert(target, conditions);
                }
            }
            if !projected.is_empty() {
                selected_guards.insert(relative.to_vec(), projected);
            }
        }
        flow.value_pointer_guards = selected_guards;
        flow.value_callable = selected_callables.get(&vec![]).cloned();
        flow.value_callables = selected_callables;
        flow.value_variants = std::mem::take(&mut flow.value_variants)
            .into_iter()
            .filter_map(|(leaf, tags)| {
                leaf.strip_prefix(path.as_slice())
                    .map(|rest| (rest.to_vec(), tags))
            })
            .collect();
        let root = PlaceId {
            owner: u64::MAX,
            local: 0,
        };
        let shape = (self.shape)(&base.ty).map_err(|e| error(expr.source, e))?;
        let mut state = State::default();
        state
            .declare(root, &shape, true, &mut self.fuel)
            .map_err(|e| error(expr.source, format!("temporary shape: {e:?}")))?;
        let selected = Place {
            root,
            fields: path.clone(),
        };
        state
            .require_present(&selected)
            .map_err(|e| error(expr.source, format!("temporary projection: {e:?}")))?;
        if (self.delivery)(&expr.ty).map_err(|e| error(expr.source, e))? == Delivery::Move {
            state
                .take(&selected)
                .map_err(|e| error(expr.source, format!("temporary field move: {e:?}")))?;
        }
        // Preserve the parent state as well as leaf responsibilities. A user
        // destructor can inspect still-present siblings before automatic fields.
        for (place, _) in state.remaining().into_iter().rev() {
            let ty = self.field_type(&base.ty, &place.fields, expr.source)?;
            self.report.temporaries.push(TemporaryCleanup {
                scope: self.temporary_scope,
                source: base.source,
                ty,
                fields: place.fields,
            });
        }
        self.retain_temporary(
            TemporaryCleanup {
                scope: self.temporary_scope,
                source: base.source,
                ty: base.ty.clone(),
                fields: vec![],
            },
            state.clone(),
            origins.clone(),
            parent_conditions,
            flow,
        )?;
        self.report.temporary_owners.push(TemporaryOwner {
            scope: self.temporary_scope,
            source: base.source,
            ty: base.ty.clone(),
            remaining: state,
        });
        let mut selected_origins = BTreeMap::new();
        for (leaf, targets) in origins {
            self.fuel = self
                .fuel
                .checked_sub(leaf.len().saturating_add(1))
                .ok_or_else(|| error(expr.source, "projection origin budget exhausted"))?;
            if leaf.starts_with(&path) {
                selected_origins.insert(leaf[path.len()..].to_vec(), targets);
            }
        }
        Ok(selected_origins)
    }
    fn temporary(
        &mut self,
        expr: &Expr,
        origins: ValueOrigins,
        flow: &mut Flow,
    ) -> Result<(), Error> {
        self.temporary_component(completion_type(&expr.ty), expr.source, &[], origins, flow)
    }
    fn declare(
        &mut self,
        flow: &mut Flow,
        id: LocalId,
        ty: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.acquire_cleanup(ty, source)?;
        let shape = (self.shape)(ty).map_err(|e| error(source, e))?;
        flow.state
            .declare(root(id), &shape, true, &mut self.fuel)
            .map_err(|e| error(source, format!("cannot establish local: {e:?}")))?;
        flow.roots.insert(root(id));
        flow.order.push(root(id));
        flow.types.insert(root(id), ty.clone());
        Ok(())
    }
    fn evaluate_default(
        &mut self,
        argument: &nir::program::CallArgument,
        frame: &mut Flow,
        caller: &mut Flow,
        depth: usize,
    ) -> Result<(), Error> {
        let construction = self
            .pending
            .last()
            .ok_or_else(|| {
                error(
                    argument.value.source,
                    "default lacks its construction frame",
                )
            })?
            .source;
        let prepared = frame.roots.clone();
        let outer_report = std::mem::take(&mut self.report);
        let outer_pending = std::mem::take(&mut self.pending);
        let outer_loops = std::mem::take(&mut self.loops);
        let outer_contract = self.exit_contract.take();
        let outer_handler = self.handler_result.take();
        let outer_effect_return = std::mem::replace(&mut self.effect_return_enabled, false);
        let outer_return_values = std::mem::take(&mut self.effect_return_values);
        let outer_effect_loops = std::mem::take(&mut self.effect_loop_targets);
        let outer_effect_memory = std::mem::take(&mut self.effect_memory_inputs);
        let outer_effect_results = std::mem::take(&mut self.effect_result_inputs);
        let outer_completions = std::mem::take(&mut self.handler_completions);
        let outer_persistent = std::mem::replace(&mut self.persistent_roots, prepared.clone());
        let result = self.in_temporary_scope(
            argument.value.source,
            TemporaryBoundary::DefaultArgument,
            frame,
            |this, frame| this.scope(&argument.value, frame, depth + 1),
        );
        let evaluation = std::mem::replace(&mut self.report, outer_report);
        self.pending = outer_pending;
        self.loops = outer_loops;
        self.exit_contract = outer_contract;
        self.handler_result = outer_handler;
        self.effect_return_enabled = outer_effect_return;
        self.effect_return_values = outer_return_values;
        self.effect_loop_targets = outer_effect_loops;
        self.effect_memory_inputs = outer_effect_memory;
        self.effect_result_inputs = outer_effect_results;
        self.handler_completions = outer_completions;
        self.persistent_roots = outer_persistent;
        result?;
        if !evaluation.effect_exits.is_empty() {
            return Err(error(
                argument.value.source,
                "effectful default requires caller escape routing",
            ));
        }
        if !evaluation.exits.is_empty() || (!frame.live && evaluation.failed_exits.is_empty()) {
            return Err(error(
                argument.value.source,
                "default argument nonlocal exit requires control transfer semantics",
            ));
        }
        for failed in &evaluation.failed_exits {
            for binding in &prepared {
                failed
                    .require_present(&Place {
                        root: *binding,
                        fields: vec![],
                    })
                    .map_err(|_| {
                        error(
                            argument.value.source,
                            "default failure consumed a prepared parameter",
                        )
                    })?;
            }
            let mut failed_caller = caller.clone();
            let first_site = self.report.cleanup_sites.len();
            self.exit(
                &mut failed_caller,
                argument.value.source,
                CleanupReason::PredicateFailure,
            )?;
            for site in &mut self.report.cleanup_sites[first_site..] {
                site.default_origin = Some((construction, argument.parameter));
            }
        }
        self.report.defaults.push(DefaultEvaluation {
            construction,
            source: argument.value.source,
            parameter: argument.parameter,
            evaluation,
        });
        if !frame.live {
            caller.live = false;
        }
        Ok(())
    }
    fn prepare_defaults(
        &mut self,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
        bindings: &[LocalId],
        flow: &mut Flow,
        source: SourceRange,
        depth: usize,
    ) -> Result<(), Error> {
        let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
            return Err(error(source, "default call lacks a callable signature"));
        };
        if bindings.len() != parameters.len()
            || arguments.len() != parameters.len()
            || bindings.iter().collect::<BTreeSet<_>>().len() != bindings.len()
        {
            return Err(error(source, "invalid default parameter frame"));
        }
        let mut slots = BTreeSet::new();
        let mut last_default = None;
        for argument in arguments {
            if argument.parameter >= bindings.len() || !slots.insert(argument.parameter) {
                return Err(error(source, "invalid default argument slot"));
            }
            if argument.is_default {
                if last_default.is_some_and(|last| last >= argument.parameter) {
                    return Err(error(
                        source,
                        "default arguments are not in declaration order",
                    ));
                }
                last_default = Some(argument.parameter);
            } else if last_default.is_some() {
                return Err(error(source, "explicit argument follows a default"));
            }
        }
        let mut frame = Flow {
            borrowed_scope: self.fresh_borrowed_scope(source)?,
            live: true,
            ..Default::default()
        };
        self.pending_frame(source, |this| {
            for argument in arguments {
                let mut origins = BTreeMap::new();
                let callable = callable_source(
                    &argument.value,
                    if argument.is_default { &frame } else { flow },
                    &mut this.fuel,
                )?;
                if argument.is_default {
                    this.evaluate_default(argument, &mut frame, flow, depth + 1)?;
                    if !flow.live {
                        return Ok(());
                    }
                } else {
                    origins = this.evaluate_value_origins(&argument.value, flow, depth + 1)?;
                    if !flow.live {
                        return Ok(());
                    }
                }
                let callable = (if argument.is_default {
                    frame.value_callable.clone()
                } else {
                    flow.value_callable.clone()
                })
                .or(callable);
                let fields = if argument.is_default {
                    frame.value_callables.clone()
                } else {
                    flow.value_callables.clone()
                };
                let domains = copy_enum_domains(
                    if argument.is_default {
                        &frame.value_variants
                    } else {
                        &flow.value_variants
                    },
                    &mut this.fuel,
                    source,
                )?;
                this.remember_pending_origins(&argument.value, argument.parameter, flow, origins)?;
                this.declare(
                    &mut frame,
                    bindings[argument.parameter],
                    &parameters[argument.parameter].ty,
                    source,
                )?;
                if let Some(callable) = callable {
                    this.store_callable(
                        &mut frame,
                        root(bindings[argument.parameter]),
                        callable,
                        source,
                    )?;
                }
                this.install_callable_fields(
                    &mut frame,
                    &Place {
                        root: root(bindings[argument.parameter]),
                        fields: vec![],
                    },
                    fields,
                    source,
                )?;
                this.install_enum_domains(
                    &mut frame,
                    &Place {
                        root: root(bindings[argument.parameter]),
                        fields: vec![],
                    },
                    domains,
                    source,
                )?;
            }
            this.check_interface_preparation(callee, flow, source)
        })?;
        if !flow.live {
            return Ok(());
        }
        for binding in bindings {
            let parameter = Place {
                root: root(*binding),
                fields: vec![],
            };
            frame.state.require_present(&parameter).map_err(|_| error(source, format!("default preparation consumed a parameter still required by the call: {parameter}")))?;
        }
        if frame.memory_exposed || flow.memory_exposed {
            return Err(error(
                source,
                "call with pointer dependencies requires a memory contract",
            ));
        }
        // The complete frame is transferred to the callee, not cleaned as a
        // temporary caller scope. No runtime representation copy is implied.
        let mut callables = vec![];
        let mut enums = vec![];
        for (slot, binding) in bindings.iter().enumerate() {
            if let Some(domains) = frame.enum_domains.get(&root(*binding)) {
                record_enum_parameter(&mut enums, slot, domains, &mut self.fuel, source)?;
            }
            if let Some(callable) = frame.callables.get(&root(*binding)) {
                callables.push(export_callable_parameter(slot, callable, &mut self.fuel)?);
            }
            for (place, value) in &frame.callable_fields {
                if place.root == root(*binding) {
                    let mut input = export_callable_parameter(slot, value, &mut self.fuel)?;
                    input.fields = place.fields.clone();
                    callables.push(input);
                }
            }
        }
        flow.value_callable =
            self.record_named_call(callee, callables, enums, &[], source, flow)?;
        self.report.calls.push(CallPreparation {
            source,
            parameters: frame.state,
        });
        Ok(())
    }
    // Reconstruct code provenance only where Copy preserves all captured
    // positions and captured code cannot change after its construction snapshot.
    // Runtime values and pointer origins never come from these descriptors.
    fn returned_code_candidate(
        &mut self,
        code: &super::memory::PreparedCode,
        ty: &Term,
        pointer_contract: bool,
        absent_pointer_paths: &[Vec<usize>],
        source: SourceRange,
        depth: usize,
    ) -> Result<Option<Callable>, Error> {
        use super::memory::{CallablePlace, CallableResult, PreparedCode};
        if depth >= 64 || self.fuel == 0 {
            return Err(error(source, "returned code evidence budget exhausted"));
        }
        self.fuel -= 1;
        super::memory::bound_type(ty, &mut self.fuel).map_err(|e| error(source, e))?;
        match code {
            PreparedCode::Named(key) => {
                for argument in &key.arguments {
                    super::memory::bound_type(argument, &mut self.fuel)
                        .map_err(|e| error(source, e))?;
                }
                let definition = Expr {
                    source,
                    ty: ty.clone(),
                    kind: if key.arguments.is_empty() {
                        K::Function(key.declaration)
                    } else {
                        K::FunctionInstance {
                            declaration: key.declaration,
                            arguments: key.arguments.clone(),
                        }
                    },
                };
                validate_callable_input(self.package, &definition, &mut self.fuel, depth)?;
                callable_source(&definition, &Flow::default(), &mut self.fuel)
            }
            PreparedCode::Closure {
                definition,
                captures: known,
                prepared_captures,
            } => {
                if definition.ty != *ty {
                    return Ok(None);
                }
                nir::visit::expression(definition, &mut self.fuel, |expr, fuel| {
                    super::memory::bound_type(&expr.ty, fuel)
                })
                .map_err(|e| error(source, e))?;
                let K::Closure { captures, body, .. } = &definition.kind else {
                    return Ok(None);
                };
                let mut protected = vec![];
                for capture in captures {
                    for fields in
                        callable_value_paths(self.package, &capture.ty, &mut self.fuel, source, 0)?
                    {
                        protected.push(Place {
                            root: root(capture.binding),
                            fields,
                        });
                    }
                }
                let mut changed_code = false;
                let mut exposed_code = false;
                nir::visit::expression(body, &mut self.fuel, |expr, fuel| {
                    let (changed, address) = match &expr.kind {
                        K::Assign { place, .. }
                        | K::BitAssign { place, .. }
                        | K::BitUpdate { place, .. } => (
                            Some(Place {
                                root: root(place.root),
                                fields: place.fields.iter().map(|(_, index)| *index).collect(),
                            }),
                            false,
                        ),
                        K::AddressOf(value) => (expression_place(value), true),
                        _ => (None, false),
                    };
                    if let Some(changed) = changed {
                        for place in &protected {
                            *fuel = fuel
                                .checked_sub(place.fields.len() + changed.fields.len() + 1)
                                .ok_or("returned capture mutation budget exhausted")?;
                            if place.root == changed.root
                                && (place.fields.starts_with(&changed.fields)
                                    || changed.fields.starts_with(&place.fields))
                            {
                                changed_code = true;
                                exposed_code |= address;
                            }
                        }
                    }
                    Ok(())
                })
                .map_err(|e| error(source, e))?;
                if exposed_code {
                    return Ok(None);
                }
                let invariant = if changed_code {
                    let Some(package) = self.package else {
                        return Ok(None);
                    };
                    match super::memory::returned_capture_invariant(
                        package,
                        definition,
                        known,
                        prepared_captures,
                        &mut self.fuel,
                    ) {
                        Ok(value) => Some(value),
                        Err(_) => return Ok(None),
                    }
                } else {
                    None
                };
                let empty_known = BTreeMap::new();
                let known = if invariant.is_some() {
                    &empty_known
                } else {
                    known.as_ref()
                };
                let prepared_captures = invariant.as_ref().unwrap_or(prepared_captures.as_ref());
                let mut evidence = BTreeMap::new();
                let mut enum_captures: BTreeMap<PlaceId, EnumDomains> = BTreeMap::new();
                for (capture_index, capture) in captures.iter().enumerate() {
                    for path in absent_pointer_paths {
                        self.fuel = self.fuel.checked_sub(path.len() + 1).ok_or_else(|| {
                            error(source, "returned pointer absence budget exhausted")
                        })?;
                        if path.first() == Some(&capture_index)
                            && !exclude_absent_code_leaf(
                                self.package,
                                &capture.ty,
                                &path[1..],
                                enum_captures.entry(root(capture.binding)).or_default(),
                                &mut self.fuel,
                                source,
                            )?
                        {
                            return Ok(None);
                        }
                    }
                    if !recoverable_return_capture_layout(
                        self.package,
                        &capture.ty,
                        pointer_contract,
                        &mut self.fuel,
                        source,
                        0,
                    )? || (self.delivery)(&capture.ty).map_err(|e| error(source, e))?
                        != Delivery::Copy
                    {
                        return Ok(None);
                    }
                    for fields in
                        callable_value_paths(self.package, &capture.ty, &mut self.fuel, source, 0)?
                    {
                        let place = CallablePlace {
                            binding: capture.binding,
                            fields: fields.clone(),
                        };
                        let named;
                        let candidates = match (known.get(&place), prepared_captures.get(&place)) {
                            (None, Some(values)) => values.as_slice(),
                            (Some(keys), None) => {
                                let keys = match keys {
                                    CallableResult::Known(key) => std::slice::from_ref(key),
                                    CallableResult::Candidates(keys) => keys.as_slice(),
                                    _ => return Ok(None),
                                };
                                named = keys
                                    .iter()
                                    .map(|key| {
                                        self.fuel = self.fuel.checked_sub(1).ok_or_else(|| {
                                            error(
                                                source,
                                                "returned capture candidates budget exhausted",
                                            )
                                        })?;
                                        for argument in &key.arguments {
                                            super::memory::bound_type(argument, &mut self.fuel)
                                                .map_err(|e| error(source, e))?;
                                        }
                                        Ok(PreparedCode::Named(key.clone()))
                                    })
                                    .collect::<Result<Vec<_>, Error>>()?;
                                named.as_slice()
                            }
                            _ => return Ok(None),
                        };
                        let expected = owned_field_type(
                            self.package,
                            &capture.ty,
                            &fields,
                            &mut self.fuel,
                            source,
                        )?;
                        if candidates.is_empty() {
                            let domains = enum_captures.entry(root(capture.binding)).or_default();
                            if !exclude_absent_code_leaf(
                                self.package,
                                &capture.ty,
                                &fields,
                                domains,
                                &mut self.fuel,
                                source,
                            )? {
                                return Ok(None);
                            }
                            continue;
                        }
                        let mut nested_absence = vec![];
                        for path in absent_pointer_paths {
                            self.fuel = self
                                .fuel
                                .checked_sub(path.len() + fields.len() + 1)
                                .ok_or_else(|| {
                                    error(source, "nested pointer absence budget exhausted")
                                })?;
                            if path.first() == Some(&capture_index)
                                && path[1..].starts_with(&fields)
                            {
                                nested_absence.push(path[1 + fields.len()..].to_vec());
                            }
                        }
                        let mut joined = None;
                        for candidate in candidates {
                            let Some(value) = self.returned_code_candidate(
                                candidate,
                                &expected,
                                pointer_contract,
                                &nested_absence,
                                source,
                                depth + 1,
                            )?
                            else {
                                return Ok(None);
                            };
                            joined = if let Some(previous) = joined {
                                merge_callable_provenance(
                                    &previous,
                                    &value,
                                    &mut self.fuel,
                                    source,
                                    0,
                                )?
                            } else {
                                Some(value)
                            };
                            if joined.is_none() {
                                return Ok(None);
                            }
                        }
                        let Some(value) = joined else {
                            return Ok(None);
                        };
                        evidence.insert(
                            Place {
                                root: root(capture.binding),
                                fields,
                            },
                            value,
                        );
                    }
                }
                Ok(Some(Callable {
                    definition: definition.as_ref().clone(),
                    captures: evidence,
                    choices: vec![],
                    enum_captures,
                    alternatives: vec![],
                    generation: 0,
                }))
            }
        }
    }

    fn named_callable_result(
        &mut self,
        callee: &Expr,
        inputs: &[CallableParameter],
        enum_inputs: &[EnumParameter],
        borrowed_code: &[BorrowedCode],
        source: SourceRange,
        may_complete: &mut bool,
        result_fields: &mut BTreeMap<Vec<usize>, Callable>,
        result_variants: &mut EnumDomains,
    ) -> Result<Option<Callable>, Error> {
        let Term::Function { result, .. } = callee.ty.callable_signature() else {
            return Ok(None);
        };
        let result = completion_type(result);
        if !contains_return_evidence(self.package, result, &mut self.fuel, source, 0)? {
            return Ok(None);
        }
        let (declaration, arguments) = match &callee.kind {
            K::Function(id) => (*id, vec![]),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.clone()),
            _ => return Ok(None),
        };
        let key = (declaration, arguments.clone());
        if self.return_stack.contains(&key) {
            return Ok(None);
        }
        if self.return_stack.len() >= 128 || self.fuel == 0 {
            return Err(error(source, "callable result analysis budget exhausted"));
        }
        self.fuel -= 1;
        let Some(package) = self.package else {
            return Ok(None);
        };
        let (original, body_arguments) = package
            .runtime_body(declaration, &arguments)
            .ok_or_else(|| error(source, "callable result function is missing"))?;
        if original.phase != FunctionPhase::Runtime
            || original.type_parameters.len() != body_arguments.len()
        {
            return Err(error(
                source,
                "callable result requires a concrete runtime function",
            ));
        }
        let binders =
            super::memory::memory_binders(Some(package), &original.type_parameters, &mut self.fuel)
                .map_err(|e| error(source, e))?;
        let mut function = original.clone();
        let instantiate = |expr: &Expr| {
            expr.instantiate_parameters(&binders, body_arguments)
                .map_err(|e| error(source, format!("callable result instantiation: {e}")))
        };
        function.signature = original
            .signature
            .instantiate_parameters(&binders, body_arguments)
            .map_err(|e| error(source, format!("callable result signature: {e}")))?;
        if function.signature.callable_signature() != callee.ty.callable_signature() {
            return Err(error(source, "callable result signature mismatch"));
        }
        function.body = instantiate(&original.body)?;
        function.preconditions = original
            .preconditions
            .iter()
            .map(instantiate)
            .collect::<Result<_, _>>()?;
        function.postconditions = original
            .postconditions
            .iter()
            .map(instantiate)
            .collect::<Result<_, _>>()?;
        function.defaults = original
            .defaults
            .iter()
            .map(|(slot, value)| Ok((*slot, instantiate(value)?)))
            .collect::<Result<_, Error>>()?;
        // Requirements/capabilities remain the materialization/publication
        // boundary's obligations. This summary never unfolds a normal-form head.
        function.type_parameters.clear();
        let mut borrows = vec![];
        let mut fields = vec![];
        if let Term::Function { parameters, .. } = function.signature.callable_signature() {
            for (slot, parameter) in parameters.iter().enumerate() {
                if parameter.sort == nir::Sort::Type
                    && parameter.ty == Term::Primitive(nir::Primitive::Type)
                {
                    continue;
                }

                for leaf in
                    super::memory::pointer_input_layout(self.package, &parameter.ty, &mut self.fuel)
                        .map_err(|reason| error(source, reason))?
                {
                    let path = leaf.input_fields;
                    let pointee = leaf.pointee;
                    if !path.is_empty() {
                        fields.push(BorrowedField {
                            parameter: slot,
                            fields: path,
                            pointee,
                        });
                    }
                }
                if let Some((_, subject)) = super::memory::pointer_representation(
                    self.package,
                    &parameter.ty,
                    &mut self.fuel,
                )
                .map_err(|reason| error(source, reason))?
                {
                    borrows.push((slot, subject));
                }
            }
        }
        if !borrows.is_empty() || !fields.is_empty() {
            let footprint = self
                .memory_summary_with_borrowed(
                    callee,
                    inputs,
                    if borrowed_code.iter().any(|input| input.invocation) {
                        borrowed_code
                    } else {
                        &[]
                    },
                    source,
                )
                .map_err(|reason| error(source, reason))?;
            if footprint.signature != function.signature {
                return Err(error(source, "callable result borrowed signature mismatch"));
            }
        }
        let mut stack = self.return_stack.clone();
        stack.push(key);
        let effect_inputs = self.effect_inputs_for(&callee.ty, source)?;
        let report = analyze_with_shapes_and_borrowed_code(
            Some(package),
            &function,
            &mut self.delivery as &mut dyn FnMut(&Term) -> Result<Delivery, String>,
            &mut self.shape as &mut dyn FnMut(&Term) -> Result<Shape, String>,
            self.fuel,
            None,
            &borrows,
            inputs,
            enum_inputs,
            stack,
            effect_inputs,
            &fields,
            borrowed_code,
            &self.trivial_cleanup_types,
        )?;
        self.fuel = self
            .fuel
            .checked_sub(report.entry_steps_used)
            .ok_or_else(|| error(source, "callable result analysis budget exhausted"))?;
        if !report.deferred_calls.is_empty() {
            return Ok(None);
        }
        // A fully analyzed callee can have only rejection/effect-escape exits.
        // Missing callable evidence on that path is not an unknown callable:
        // there is no delivered value and no continuation to invoke it.
        *may_complete = !report.exits.is_empty();
        (*result_fields, *result_variants) =
            callable_aggregate_return_summary(&report, &mut self.fuel, source)?;
        callable_return_summary(&report, &mut self.fuel, source)
    }

    fn record_named_call(
        &mut self,
        callee: &Expr,
        callables: Vec<CallableParameter>,
        enums: Vec<EnumParameter>,
        borrowed_code: &[BorrowedCode],
        source: SourceRange,
        flow: &mut Flow,
    ) -> Result<Option<Callable>, Error> {
        // This child describes the selected implementation's resource context,
        // not a replacement of the interface call. The original call retains
        // its entry/result checks and prepared ownership boundaries in `expr`.
        // A generic trait call has no selected implementation body yet. Its
        // interface still owns the call contract and transfer checks; do not
        // invent child-body or returned-callable evidence here. Concrete
        // materialization must select and verify the actual implementation.
        if matches!(callee.kind, K::TraitMethod { .. })
            && self
                .package
                .is_some_and(|package| package.runtime_instances.is_empty())
            && !self.storage_inputs_are_closed(&callee.ty, source)?
        {
            flow.value_callables.clear();
            flow.value_variants.clear();
            return Ok(None);
        }
        let selected;
        let callee = if matches!(callee.kind, K::TraitMethod { .. }) {
            let package = self
                .package
                .ok_or_else(|| error(source, "method invocation requires a package"))?;
            selected = self
                .method_callees
                .select_behavior(package, callee, &mut self.fuel)
                .map_err(|reason| error(source, reason))?;
            &selected
        } else {
            callee
        };
        flow.value_callables.clear();
        flow.value_variants.clear();
        if !matches!(callee.kind, K::Function(_) | K::FunctionInstance { .. }) {
            return Ok(None);
        }
        let Term::Function {
            parameters,
            result: result_ty,
            ..
        } = callee.ty.callable_signature()
        else {
            return Ok(None);
        };
        let effects = self.effect_inputs_for(&callee.ty, source)?;
        let effect_slots = if let Some(package) = self.package {
            effect_result_slots(package, &callee.ty, &mut self.fuel, source)?
        } else {
            vec![]
        };
        if effect_slots
            .iter()
            .any(|slot| !effects.iter().any(|input| input.slot == *slot))
        {
            self.report.deferred_calls.push(source);
            return Ok(None);
        }
        let result_ty = completion_type(result_ty);
        let mut required = vec![];
        for (slot, parameter) in parameters.iter().enumerate() {
            for fields in
                callable_value_paths(self.package, &parameter.ty, &mut self.fuel, source, 0)?
            {
                required.push((slot, fields));
            }
        }
        // Passing owned code does not invoke it. Analyze the callee with the
        // available premises; an actual indirect call still requires identity.
        // Pure transport must preserve enum/cleanup facts even when affine
        // captures prevent reconstructing a callable candidate.
        if required.iter().any(|(slot, fields)| {
            !enums
                .iter()
                .any(|input| input.parameter == *slot && enum_path_excluded(fields, &input.domains))
                && !callables
                    .iter()
                    .any(|input| input.parameter == *slot && input.fields == *fields)
        }) {
            match self.memory_summary_with_borrowed(
                callee,
                &callables,
                if borrowed_code.iter().any(|input| input.invocation) {
                    borrowed_code
                } else {
                    &[]
                },
                source,
            ) {
                Ok(_) => (),
                Err(reason)
                    if reason.contains("budget exhausted")
                        || reason.contains("depth exhausted") =>
                {
                    return Err(error(source, reason));
                }
                Err(_) => {
                    self.report.deferred_calls.push(source);
                    return Ok(None);
                }
            }
        }
        let mut may_complete = true;
        let mut result = self.named_callable_result(
            callee,
            &callables,
            &enums,
            borrowed_code,
            source,
            &mut may_complete,
            &mut flow.value_callables,
            &mut flow.value_variants,
        )?;
        if !may_complete {
            // No returned callable does not remove the caller's unwind work.
            // Record terminal paths before suppressing normal continuation;
            // the usual post-call checkpoints only run for a live flow.
            if self
                .call_may_reject(callee)
                .map_err(|reason| error(source, reason))?
            {
                let mut failed = flow.clone();
                self.exit(&mut failed, source, CleanupReason::PredicateFailure)?;
            }
            if matches!(callee.ty.callable_signature(), Term::Function { result, .. }
                if matches!(result.as_ref(), Term::Computation { effects, .. } if !effects.is_empty()))
            {
                self.effect_escape_checkpoint(flow, source)?;
            }
            flow.live = false;
        }
        if may_complete {
            let paths = callable_value_paths(self.package, result_ty, &mut self.fuel, source, 0)?;
            if paths.iter().any(|path| {
                if path.is_empty() {
                    result.is_none()
                } else {
                    !flow.value_callables.contains_key(path)
                }
            }) {
                // A closed package fixed point can recover recursive result
                // leaves that this finite contextual walk cannot unfold.
                let summary = match self.memory_summary_with_borrowed(
                    callee,
                    &callables,
                    if borrowed_code.iter().any(|input| input.invocation) {
                        borrowed_code
                    } else {
                        &[]
                    },
                    source,
                ) {
                    Ok(summary) => Some(summary),
                    Err(reason)
                        if reason.contains("budget exhausted")
                            || reason.contains("depth exhausted") =>
                    {
                        return Err(error(source, reason));
                    }
                    Err(_) => None,
                };
                if let Some(summary) = summary {
                    // Cleanup needs result tag facts even when affine captures
                    // prevent recovering an invocable code candidate. Keep this
                    // evidence independent from Copy-based code reconstruction.
                    let layout = super::memory::pointer_input_layout(
                        self.package,
                        result_ty,
                        &mut self.fuel,
                    )
                    .map_err(|reason| error(source, reason))?;
                    let mut domains =
                        copy_enum_domains(&flow.value_variants, &mut self.fuel, source)?;
                    let mut consistent = true;
                    for leaf in layout {
                        if !leaf.guards.is_empty()
                            && summary
                                .pointer_fields
                                .get(&leaf.input_fields)
                                .and_then(Option::as_ref)
                                .is_some_and(BTreeSet::is_empty)
                        {
                            consistent &= exclude_absent_code_leaf(
                                self.package,
                                result_ty,
                                &leaf.input_fields,
                                &mut domains,
                                &mut self.fuel,
                                source,
                            )?;
                            if !consistent {
                                break;
                            }
                        }
                    }
                    if consistent {
                        flow.value_variants = domains;
                    }
                    for (path, evidence) in &summary.code_fields {
                        if if path.is_empty() {
                            result.is_some()
                        } else {
                            flow.value_callables.contains_key(path)
                        } {
                            continue;
                        }
                        let super::memory::CodeResult::Known(values) = evidence else {
                            continue;
                        };
                        let ty = owned_field_type(
                            self.package,
                            result_ty,
                            path,
                            &mut self.fuel,
                            source,
                        )?;
                        // Recover pointer layouts only with complete origins for
                        // every possible leaf. Ownership requires Copy and stable
                        // code; this flag never grants pointee validity by itself.
                        let layout =
                            super::memory::pointer_input_layout(self.package, &ty, &mut self.fuel)
                                .map_err(|reason| error(source, reason))?;
                        let mut pointer_contract = true;
                        let mut absent_pointer_paths = vec![];
                        for leaf in &layout {
                            self.fuel = self
                                .fuel
                                .checked_sub(path.len() + leaf.input_fields.len() + 1)
                                .ok_or_else(|| {
                                    error(source, "returned pointer contract budget exhausted")
                                })?;
                            let mut full = path.clone();
                            full.extend(&leaf.input_fields);
                            match summary.pointer_fields.get(&full).and_then(Option::as_ref) {
                                Some(targets) if !targets.is_empty() => (),
                                Some(_) if !leaf.guards.is_empty() => {
                                    absent_pointer_paths.push(leaf.input_fields.clone())
                                }
                                _ => pointer_contract = false,
                            }
                        }
                        let mut joined = None;
                        let mut complete = !values.is_empty();
                        for code in values {
                            let Some(value) = self.returned_code_candidate(
                                code,
                                &ty,
                                pointer_contract,
                                &absent_pointer_paths,
                                source,
                                0,
                            )?
                            else {
                                complete = false;
                                break;
                            };
                            joined = if let Some(previous) = joined {
                                merge_callable_provenance(
                                    &previous,
                                    &value,
                                    &mut self.fuel,
                                    source,
                                    0,
                                )?
                            } else {
                                Some(value)
                            };
                            if joined.is_none() {
                                complete = false;
                                break;
                            }
                        }
                        if complete {
                            if let Some(value) = joined {
                                if path.is_empty() {
                                    result = Some(value);
                                } else {
                                    flow.value_callables.insert(path.clone(), value);
                                }
                            }
                        }
                    }
                    let mut actuals: BTreeMap<
                        usize,
                        BTreeMap<Vec<usize>, super::memory::CallableResult>,
                    > = BTreeMap::new();
                    for input in &callables {
                        if input.choices.is_empty() {
                            if let Some(keys) = named_memory_definitions(
                                &input.definition,
                                &input.alternatives,
                                &mut self.fuel,
                                source,
                            )? {
                                actuals
                                    .entry(input.parameter)
                                    .or_default()
                                    .insert(input.fields.clone(), keys);
                            }
                        }
                    }
                    for (path, keys) in summary.callable_fields {
                        if if path.is_empty() {
                            result.is_some()
                        } else {
                            flow.value_callables.contains_key(&path)
                        } {
                            continue;
                        }
                        let keys = keys
                            .substitute_inputs(&actuals, &mut self.fuel)
                            .map_err(|reason| error(source, reason))?;
                        let definitions = match keys {
                            super::memory::CallableResult::Known(key) => vec![key],
                            super::memory::CallableResult::Candidates(keys) => keys,
                            _ => continue,
                        };
                        let ty = owned_field_type(
                            self.package,
                            result_ty,
                            &path,
                            &mut self.fuel,
                            source,
                        )?;
                        let mut joined = None;
                        for key in definitions {
                            super::memory::bound_type(&ty, &mut self.fuel)
                                .map_err(|reason| error(source, reason))?;
                            let definition = Expr {
                                source,
                                ty: ty.clone(),
                                kind: if key.arguments.is_empty() {
                                    K::Function(key.declaration)
                                } else {
                                    K::FunctionInstance {
                                        declaration: key.declaration,
                                        arguments: key.arguments,
                                    }
                                },
                            };
                            let value =
                                callable_source(&definition, &Flow::default(), &mut self.fuel)?
                                    .ok_or_else(|| {
                                        error(source, "invalid callable result identity")
                                    })?;
                            joined = if let Some(previous) = joined {
                                merge_named_callables(&previous, &value, &mut self.fuel, source)?
                            } else {
                                Some(value)
                            };
                        }
                        if let Some(value) = joined {
                            if path.is_empty() {
                                result = Some(value);
                            } else {
                                flow.value_callables.insert(path, value);
                            }
                        }
                    }
                }
            }
        }
        let invocation_borrowed = if borrowed_code.iter().any(|input| input.invocation) {
            let mut inputs = copy_borrowed_code(borrowed_code, &mut self.fuel, source)?;
            inputs.retain(|input| input.invocation);
            inputs
        } else {
            vec![]
        };
        if !required.is_empty() || !effect_slots.is_empty() || !invocation_borrowed.is_empty() {
            self.report.named_calls.push(NamedCall {
                borrowed_code: invocation_borrowed,
                source,
                callee: callee.clone(),
                callables,
                enums,
                effects,
            });
        }
        Ok(result)
    }

    fn call_temporary(
        &mut self,
        expr: &Expr,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
        parameter_bindings: &[LocalId],
        flow: &mut Flow,
        depth: usize,
    ) -> Result<Option<BTreeSet<Place>>, Error> {
        self.call_temporary_with_origins(expr, callee, arguments, parameter_bindings, flow, depth)
            .map(|(pointer, _, _)| pointer)
    }

    fn call_temporary_with_origins(
        &mut self,
        expr: &Expr,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
        parameter_bindings: &[LocalId],
        flow: &mut Flow,
        depth: usize,
    ) -> Result<(Option<BTreeSet<Place>>, ValueOrigins, ValuePointerGuards), Error> {
        // Owned projections invoke code in place, just like local callees.
        // A closure writes back into this exact environment; named code only
        // observes its slot. Neither route creates an owned callee temporary.
        if let Some(place) = expression_place(callee) {
            if let Some(callable) = callable_source(callee, flow, &mut self.fuel)? {
                if matches!(callable.definition.kind, K::Closure { .. }) {
                    bound_callable_copy(&callable, &mut self.fuel, callee.source, 0)?;
                    return self.call_closure_with_origins(
                        &place,
                        &callable,
                        arguments,
                        flow,
                        expr.source,
                        depth + 1,
                    );
                }
                if callable.choices.is_empty()
                    && matches!(
                        callable.definition.kind,
                        K::Function(_) | K::FunctionInstance { .. }
                    )
                {
                    bound_callable_copy(&callable, &mut self.fuel, callee.source, 0)?;
                    self.observe_named_callee(callee, flow)?;
                    if !callable.alternatives.is_empty() {
                        return self.call_named_candidates_with_origins(
                            expr,
                            &callable,
                            arguments,
                            parameter_bindings,
                            flow,
                            depth + 1,
                        );
                    }
                    let resolved = self.indirect_named_call(expr, flow).ok_or_else(|| {
                        error(callee.source, "observed field callee lost its identity")
                    })?;
                    if super::memory::pointer_representation(self.package, &expr.ty, &mut self.fuel)
                        .map_err(|reason| error(expr.source, reason))?
                        .is_some()
                    {
                        let pointer = self.evaluate_pointer_value(&resolved, flow, depth + 1)?;
                        return Ok((pointer, BTreeMap::new(), BTreeMap::new()));
                    }
                    let origins = self.evaluate_value_origins(&resolved, flow, depth + 1)?;
                    return Ok((
                        None,
                        origins,
                        std::mem::take(&mut flow.value_pointer_guards),
                    ));
                }
            }
        }
        let origins = self.evaluate_value_origins(callee, flow, depth + 1)?;
        if !flow.live {
            return Ok((None, BTreeMap::new(), BTreeMap::new()));
        }
        let guards = std::mem::take(&mut flow.value_pointer_guards);
        let callable = flow.value_callable.take().ok_or_else(|| {
            error(
                callee.source,
                "temporary callee requires known callable provenance",
            )
        })?;
        let identity = self.sequence(expr.source)?;
        let id = PlaceId {
            owner: u64::MAX - 3,
            local: u32::try_from(identity)
                .map_err(|_| error(expr.source, "temporary callee identity exhausted"))?,
        };
        let anchors = flow.order.clone();
        self.acquire_cleanup(&callee.ty, callee.source)?;
        let shape = (self.shape)(&callee.ty).map_err(|e| error(callee.source, e))?;
        flow.state
            .declare(id, &shape, true, &mut self.fuel)
            .map_err(|e| error(callee.source, format!("temporary callee shape: {e:?}")))?;
        flow.roots.insert(id);
        flow.order.push(id);
        flow.types.insert(id, callee.ty.clone());
        self.install_value_origins(
            flow,
            &Place {
                root: id,
                fields: vec![],
            },
            origins.clone(),
            callee.source,
        )?;
        self.install_pointer_guards(
            flow,
            &Place {
                root: id,
                fields: vec![],
            },
            guards,
            callee.source,
        )?;
        self.store_callable(flow, id, callable, callee.source)?;
        let callable = flow.callables[&id].clone();
        self.report.callee_temporaries.push((id, callee.source));
        let mut pointer_result = None;
        let mut result_origins = BTreeMap::new();
        let mut result_guards = BTreeMap::new();
        if !callable.alternatives.is_empty() {
            (pointer_result, result_origins, result_guards) = self
                .call_named_candidates_with_origins(
                    expr,
                    &callable,
                    arguments,
                    parameter_bindings,
                    flow,
                    depth + 1,
                )?;
        } else if matches!(callable.definition.kind, K::Closure { .. }) {
            (pointer_result, result_origins, result_guards) = self.call_closure_with_origins(
                &Place {
                    root: id,
                    fields: vec![],
                },
                &callable,
                arguments,
                flow,
                expr.source,
                depth + 1,
            )?;
        } else {
            let mut target = callable.definition;
            target.source = callee.source;
            let declaration = match target.kind {
                K::Function(id)
                | K::FunctionInstance {
                    declaration: id, ..
                } => id,
                _ => return Err(error(callee.source, "unknown temporary callee kind")),
            };
            let bindings = if parameter_bindings.is_empty() {
                self.package
                    .and_then(|p| p.functions.iter().find(|f| f.declaration == declaration))
                    .map(|f| f.parameters.clone())
                    .unwrap_or_default()
            } else {
                parameter_bindings.to_vec()
            };
            let resolved = Expr {
                source: expr.source,
                ty: expr.ty.clone(),
                kind: K::Call {
                    callee: Box::new(target),
                    arguments: arguments.to_vec(),
                    parameter_bindings: bindings,
                },
            };
            if super::memory::pointer_representation(self.package, &expr.ty, &mut self.fuel)
                .map_err(|reason| error(expr.source, reason))?
                .is_some()
            {
                pointer_result = self.evaluate_pointer_value(&resolved, flow, depth + 1)?;
            } else {
                result_origins = self.evaluate_value_origins(&resolved, flow, depth + 1)?;
                result_guards = std::mem::take(&mut flow.value_pointer_guards);
            }
        }
        let result = flow.value_callable.clone();
        if flow.live {
            // A successful call does not end its enclosing full expression.
            // Retain the receiver's remaining responsibility in the same
            // canonical root representation used by every temporary action.
            let mut remaining = State::default();
            remaining
                .import_position(
                    PlaceId {
                        owner: u64::MAX,
                        local: 0,
                    },
                    &flow.state,
                    &Place {
                        root: id,
                        fields: vec![],
                    },
                    &mut self.fuel,
                )
                .map_err(|e| error(expr.source, format!("temporary callee transfer: {e:?}")))?;
            let callee_place = Place {
                root: id,
                fields: vec![],
            };
            let conditions = ValueConditions {
                domains: self.stored_enum_domains(flow, &callee_place, expr.source)?,
                pointers: self.stored_pointer_guards(&callee_place, flow, expr.source)?,
            };
            self.remove_local(flow, id, expr.source)?;
            flow.temporaries.insert(
                identity,
                ActiveTemporary {
                    conditions,
                    alternatives: vec![],
                    origins,
                    variants: vec![],
                    scope_id: self.temporary_scope_id,
                    description: TemporaryCleanup {
                        scope: self.temporary_scope,
                        source: callee.source,
                        ty: callee.ty.clone(),
                        fields: vec![],
                    },
                    remaining,
                    presence: Availability::Present,
                    locals_before: anchors,
                },
            );
        }
        flow.value_callable = result;
        Ok((pointer_result, result_origins, result_guards))
    }
    fn call_closure(
        &mut self,
        binding: PlaceId,
        callable: &Callable,
        arguments: &[nir::program::CallArgument],
        flow: &mut Flow,
        source: SourceRange,
        depth: usize,
    ) -> Result<Option<BTreeSet<Place>>, Error> {
        self.call_closure_with_origins(
            &Place {
                root: binding,
                fields: vec![],
            },
            callable,
            arguments,
            flow,
            source,
            depth,
        )
        .map(|(pointer, _, _)| pointer)
    }
    fn call_closure_with_origins(
        &mut self,
        binding: &Place,
        callable: &Callable,
        arguments: &[nir::program::CallArgument],
        flow: &mut Flow,
        source: SourceRange,
        depth: usize,
    ) -> Result<(Option<BTreeSet<Place>>, ValueOrigins, ValuePointerGuards), Error> {
        if self.fuel == 0 || depth > 128 {
            return Err(error(
                source,
                "closure invocation analysis budget exhausted",
            ));
        }
        self.fuel = self
            .fuel
            .checked_sub(binding.fields.len() + 1)
            .ok_or_else(|| error(source, "closure receiver path budget exhausted"))?;
        let Term::Function {
            parameters, result, ..
        } = callable.definition.ty.callable_signature()
        else {
            return Err(error(source, "closure invocation lacks a signature"));
        };
        // Ownership-only analysis has no package layouts or callable bodies.
        // Preserve that API; pointer-dependent calls still need memory premises.
        let result_paths = if self.package.is_some()
            && matches!(
                result.as_ref(),
                Term::Tuple(_)
                    | Term::Record(_)
                    | Term::Nominal { .. }
                    | Term::NormalForm { .. }
                    | Term::Closure { .. }
                    | Term::ErrorResult { .. }
            )
            && self.storage_inputs_are_closed(result, source)?
            && super::memory::pointer_representation(self.package, result, &mut self.fuel)
                .map_err(|reason| error(source, reason))?
                .is_none()
        {
            super::memory::pointer_input_layout(self.package, result, &mut self.fuel)
                .map_err(|reason| error(source, reason))?
                .into_iter()
                .map(|leaf| (leaf.input_fields.clone(), leaf))
                .collect()
        } else {
            BTreeMap::new()
        };
        let mut result_origins: ValueOrigins = result_paths
            .values()
            .map(|leaf| (leaf.storage_fields.clone(), BTreeSet::new()))
            .collect();
        let mut result_guards = ValuePointerGuards::new();
        let mut memory = !result_paths.is_empty()
            || flow.memory_exposed
            || super::memory::pointer_representation(self.package, result, &mut self.fuel)
                .map_err(|reason| error(source, reason))?
                .is_some()
            || matches!(&callable.definition.kind,K::Closure {captures,..}
                if captures.iter().any(|capture|matches!(capture.ty,Term::Pointer(_))));
        for parameter in parameters {
            memory |=
                super::memory::pointer_representation(self.package, &parameter.ty, &mut self.fuel)
                    .map_err(|reason| error(source, reason))?
                    .is_some();
        }
        if let K::Closure { captures, .. } = &callable.definition.kind {
            for capture in captures {
                if self.package.is_some() && self.storage_inputs_are_closed(&capture.ty, source)? {
                    memory |= !super::memory::pointer_input_layout(
                        self.package,
                        &capture.ty,
                        &mut self.fuel,
                    )
                    .map_err(|reason| error(source, reason))?
                    .is_empty();
                }
            }
        }
        let Some(prepared) =
            self.prepare_closure_arguments(callable, arguments, flow, source, depth)?
        else {
            return Ok((None, BTreeMap::new(), BTreeMap::new()));
        };
        memory |= prepared.pointers.values().any(|fields| !fields.is_empty());
        // Arguments can mutate state. Do not resurrect a callable moved while
        // evaluating an argument, or confuse a replacement with this receiver.
        if flow.callable_at(binding) != Some(callable) {
            return Err(error(
                source,
                "closure receiver changed while evaluating arguments",
            ));
        }
        let mut continuation: Option<Flow> = None;
        let mut failure: Option<Flow> = None;
        let mut result_targets = Some(BTreeSet::new());
        for candidate in std::iter::once(callable).chain(&callable.choices) {
            self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or_else(|| error(source, "closure candidate invocation budget exhausted"))?;
            let mut candidate = candidate.clone();
            candidate.choices.clear();
            candidate.generation = callable.generation;
            let mut branch = flow.clone();
            branch.set_callable_at(binding, candidate.clone());
            if memory {
                self.fuel = self
                    .fuel
                    .checked_sub(prepared.pointers.len())
                    .ok_or_else(|| error(source, "closure pointer input budget exhausted"))?;
                for fields in prepared.pointers.values() {
                    for (path, targets) in fields {
                        self.fuel = self
                            .fuel
                            .checked_sub(path.len() + 1)
                            .ok_or_else(|| error(source, "closure input path budget exhausted"))?;
                        self.pointer_work(targets, source)?;
                    }
                }
                let mut pointer_inputs = prepared.pointers.clone();
                if let K::Closure { captures, .. } = &candidate.definition.kind {
                    let mut slot = parameters.len();
                    for (index, capture) in captures.iter().enumerate() {
                        let domains = copy_enum_domains(
                            candidate
                                .enum_captures
                                .get(&root(capture.binding))
                                .unwrap_or(&EnumDomains::new()),
                            &mut self.fuel,
                            source,
                        )?;
                        let inputs = self.capture_input_targets(
                            &branch,
                            binding,
                            index,
                            &capture.ty,
                            &domains,
                            true,
                            source,
                        )?;
                        if !inputs.is_empty() {
                            pointer_inputs.insert(slot, inputs);
                            slot += 1;
                        }
                    }
                }
                let inputs = prepared_memory_inputs(&prepared, &mut self.fuel, source)?;
                let code_inputs =
                    prepared_memory_code_inputs(self.package, &prepared, &mut self.fuel, source)?;
                let footprint = if matches!(candidate.definition.kind, K::Closure { .. }) {
                    let mut callees = BTreeMap::new();
                    if let K::Closure { captures, .. } = &candidate.definition.kind {
                        for capture in captures {
                            for (place, value) in &candidate.captures {
                                self.fuel =
                                    self.fuel.checked_sub(place.fields.len() + 1).ok_or_else(
                                        || error(source, "captured memory field budget exhausted"),
                                    )?;
                                if place.root == root(capture.binding) {
                                    if let Some(keys) =
                                        named_memory_callees(value, &mut self.fuel, source)?
                                    {
                                        callees.insert(
                                            super::memory::CallablePlace {
                                                binding: capture.binding,
                                                fields: place.fields.clone(),
                                            },
                                            keys,
                                        );
                                    }
                                }
                            }
                        }
                    }
                    let definitions =
                        memory_code_definitions(self.package, &candidate, &mut self.fuel, source)?;
                    let prepared_captures = definitions
                        .as_ref()
                        .and_then(|values| values.first())
                        .and_then(|value| match value {
                            super::memory::CodeDefinition::Closure {
                                prepared_captures, ..
                            } => Some(prepared_captures.as_ref()),
                            _ => None,
                        });
                    super::memory::infer_closure_with_effect_inputs(
                        self.package
                            .ok_or_else(|| error(source, "closure memory requires a package"))?,
                        &candidate.definition,
                        &callees,
                        prepared_captures.unwrap_or(&BTreeMap::new()),
                        &inputs,
                        &code_inputs,
                        &self.effect_memory_inputs,
                        &mut self.fuel,
                    )
                    .map(|conditional| conditional.memory)
                } else {
                    self.memory_summary_with_prepared_code(
                        &candidate.definition,
                        &inputs,
                        &code_inputs,
                    )
                    .map(|summary| super::memory::ArgumentSummary {
                        input_pointer_exits: BTreeMap::new(),
                        input_code_exits: BTreeMap::new(),
                        code_fields: summary.code_fields,
                        accesses: summary.accesses,
                        may_reject: summary.may_reject,
                        may_run_cleanup: summary.may_run_cleanup,
                        result_targets: summary.result_targets,
                        pointer_fields: summary.pointer_fields,
                    })
                };
                let footprint = match footprint {
                    Ok(footprint) => Some(footprint),
                    // A construction-time invocation has no actual callback
                    // arguments yet. Keep its obligation, then recheck with the
                    // prepared evidence on invocation. Never publish a successful
                    // footprint or pointer provenance for this unknown case.
                    Err(reason)
                        if self.defer_unknown_calls
                            && reason == "dynamic call requires a richer memory contract" =>
                    {
                        self.report.deferred_calls.push(source);
                        result_targets = None;
                        for targets in result_origins.values_mut() {
                            targets.insert(unknown_pointer_target());
                        }
                        None
                    }
                    Err(reason) => return Err(error(source, reason)),
                };
                if let Some(footprint) = footprint {
                    for (path, leaf) in &result_paths {
                        let storage = &leaf.storage_fields;
                        self.fuel = self
                            .fuel
                            .checked_sub(path.len() + storage.len() + 1)
                            .ok_or_else(|| {
                                error(source, "closure result provenance budget exhausted")
                            })?;
                        let mut actual = BTreeSet::new();
                        if let Some(Some(targets)) = footprint.pointer_fields.get(path) {
                            for target in targets {
                                if let Some(mapped) = self.map_memory_target(
                                    target,
                                    &pointer_inputs,
                                    &mut branch,
                                    source,
                                )? {
                                    actual.extend(mapped);
                                } else {
                                    actual.insert(unknown_pointer_target());
                                }
                            }
                        } else {
                            actual.insert(unknown_pointer_target());
                        }
                        self.record_result_pointer_guards(
                            &mut result_guards,
                            leaf,
                            &actual,
                            source,
                        )?;
                        result_origins
                            .get_mut(storage)
                            .expect("validated result path")
                            .extend(actual);
                    }
                    self.memory_accesses(
                        &footprint.accesses,
                        &pointer_inputs,
                        &mut branch,
                        source,
                    )?;
                    let mut previous = result_targets
                        .take()
                        .unwrap_or_else(|| BTreeSet::from([unknown_pointer_target()]));
                    if let Some(targets) = footprint.result_targets {
                        for target in targets {
                            if let Some(mapped) = self.map_memory_target(
                                &target,
                                &pointer_inputs,
                                &mut branch,
                                source,
                            )? {
                                previous.extend(mapped);
                            } else {
                                previous.insert(unknown_pointer_target());
                            }
                        }
                    } else {
                        previous.insert(unknown_pointer_target());
                    }
                    result_targets = Some(previous);
                }
            }
            let failed = if matches!(candidate.definition.kind, K::Closure { .. }) {
                self.call_prepared_closure(
                    binding,
                    &candidate,
                    prepared.clone(),
                    &mut branch,
                    source,
                    depth,
                )?
            } else {
                self.call_prepared_named_candidate(
                    &candidate.definition,
                    &prepared,
                    &mut branch,
                    source,
                )?
            };
            if let Some(failed) = failed {
                if let Some(previous) = &mut failure {
                    self.join_flow(previous, &failed, source)?;
                } else {
                    failure = Some(failed);
                }
            }
            if branch.live {
                if let Some(previous) = &mut continuation {
                    self.join_flow(previous, &branch, source)?;
                } else {
                    continuation = Some(branch);
                }
            }
        }
        if let Some(mut failed) = failure {
            self.exit(&mut failed, source, CleanupReason::PredicateFailure)?;
        }
        if let Some(continuation) = continuation {
            *flow = continuation;
        } else {
            flow.live = false;
        }
        result_guards.retain(|_, entries| {
            entries.retain(|_, conditions| !conditions.is_empty());
            !entries.is_empty()
        });
        Ok((
            if memory { result_targets } else { None },
            result_origins,
            result_guards,
        ))
    }
    // Mixed sets use the same completed argument frame as closure candidates.
    // Any pointer footprint has already been checked against actual targets.
    fn call_prepared_named_candidate(
        &mut self,
        callee: &Expr,
        prepared: &ClosureArguments,
        flow: &mut Flow,
        source: SourceRange,
    ) -> Result<Option<Flow>, Error> {
        let mut callables = vec![];
        let mut enums = vec![];
        for (&parameter, callable) in &prepared.callables {
            callables.push(export_callable_parameter(
                parameter,
                callable,
                &mut self.fuel,
            )?);
        }
        for (&parameter, fields) in &prepared.fields {
            for (path, callable) in fields {
                if !path.is_empty() {
                    let mut input = export_callable_parameter(parameter, callable, &mut self.fuel)?;
                    input.fields = path.clone();
                    callables.push(input);
                }
            }
        }
        for (&parameter, domains) in &prepared.domains {
            record_enum_parameter(&mut enums, parameter, domains, &mut self.fuel, source)?;
        }
        flow.value_callable =
            self.record_named_call(callee, callables, enums, &[], source, flow)?;
        if self
            .call_may_reject(callee)
            .map_err(|reason| error(source, reason))?
        {
            Ok(Some(flow.clone()))
        } else {
            Ok(None)
        }
    }
    fn prepare_closure_arguments(
        &mut self,
        callable: &Callable,
        arguments: &[nir::program::CallArgument],
        flow: &mut Flow,
        source: SourceRange,
        depth: usize,
    ) -> Result<Option<ClosureArguments>, Error> {
        let definition = &callable.definition;
        let K::Closure { parameters, .. } = &definition.kind else {
            return Err(error(source, "invalid closure provenance"));
        };
        let Term::Function {
            parameters: types, ..
        } = definition.ty.callable_signature()
        else {
            return Err(error(source, "invalid closure signature"));
        };
        if arguments.iter().any(|a| a.is_default) {
            return Err(error(
                source,
                "closure defaults require parameter-frame transfer",
            ));
        }
        if parameters.len() != arguments.len() || types.len() != parameters.len() {
            return Err(error(source, "closure argument count mismatch"));
        }
        let mut seen = BTreeSet::new();
        for argument in arguments {
            if argument.parameter >= parameters.len() || !seen.insert(argument.parameter) {
                return Err(error(source, "invalid closure argument plan"));
            }
        }
        // Preserve each argument's provenance at its own evaluation point.
        // Later arguments may replace the source local without changing the
        // already prepared callable value delivered to this parameter.
        let mut argument_pointers = BTreeMap::new();
        let mut argument_callables = BTreeMap::new();
        let mut argument_fields = BTreeMap::new();
        let mut argument_domains = BTreeMap::new();
        self.pending_frame(source, |this| {
            for argument in arguments {
                let callable = callable_source(&argument.value, flow, &mut this.fuel)?;
                let origins = this.evaluate_value_origins(&argument.value, flow, depth + 1)?;
                let mut symbolic = this.memory_input_targets(
                    &argument.value.ty,
                    &origins,
                    &flow.value_variants,
                    source,
                )?;
                // Guard projection only refines existing pointer targets.
                // Whole generic/ownership-only arguments have no such targets;
                // asking for their layout here would prematurely require a
                // concrete instantiation without adding any memory evidence.
                let layout = if symbolic.values().any(|targets| !targets.is_empty()) {
                    super::memory::pointer_input_layout(
                        this.package,
                        &argument.value.ty,
                        &mut this.fuel,
                    )
                    .map_err(|reason| error(source, reason))?
                } else {
                    vec![]
                };
                for leaf in &layout {
                    if let Some(actual) = symbolic.get_mut(&leaf.input_fields) {
                        if !actual.is_empty() {
                            *actual = this.guarded_request_targets(
                                &argument.value,
                                leaf,
                                actual,
                                flow,
                                true,
                            )?;
                        }
                    }
                }
                argument_pointers.insert(argument.parameter, symbolic);
                let callable = flow.value_callable.clone().or(callable);
                if !flow.live {
                    break;
                }
                this.remember_pending_origins(&argument.value, argument.parameter, flow, origins)?;
                if let Some(callable) = callable {
                    argument_callables.insert(argument.parameter, callable);
                }
                argument_fields.insert(argument.parameter, flow.value_callables.clone());
                argument_domains.insert(
                    argument.parameter,
                    copy_enum_domains(&flow.value_variants, &mut this.fuel, source)?,
                );
            }
            Ok(())
        })?;
        if !flow.live {
            return Ok(None);
        }
        Ok(Some(ClosureArguments {
            pointers: argument_pointers,
            callables: argument_callables,
            fields: argument_fields,
            domains: argument_domains,
        }))
    }
    fn declare_closure_borrow(
        &mut self,
        flow: &mut Flow,
        slot: usize,
        binding: LocalId,
        subject: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        let ty = flow
            .types
            .get(&root(binding))
            .ok_or_else(|| error(source, "closure borrowed binding is missing"))?;
        let Some((pointer_depth, actual)) =
            super::memory::pointer_representation(self.package, ty, &mut self.fuel)
                .map_err(|reason| error(source, reason))?
        else {
            return Err(error(
                source,
                "closure borrowed binding lacks pointer representation",
            ));
        };
        if actual != *subject {
            return Err(error(source, "closure borrowed pointee type mismatch"));
        }
        // Conditional borrowed premises never become owned cleanup roots.
        // Actual pointees must separately satisfy the invocation footprint.
        let pointee = PlaceId {
            owner: u64::MAX - 1,
            local: u32::try_from(slot)
                .map_err(|_| error(source, "too many closure pointer parameters"))?,
        };
        let shape = (self.shape)(subject).map_err(|reason| error(source, reason))?;
        flow.state
            .declare(pointee, &shape, true, &mut self.fuel)
            .map_err(|reason| {
                error(
                    source,
                    format!("closure borrowed parameter shape: {reason:?}"),
                )
            })?;
        flow.types.insert(pointee, subject.clone());
        flow.pointers.insert(
            Place {
                root: root(binding),
                fields: vec![0; pointer_depth],
            },
            BTreeSet::from([Place {
                root: pointee,
                fields: vec![],
            }]),
        );
        flow.memory_exposed = true;
        Ok(())
    }
    fn restore_capture_pointer_origins<'b>(
        &mut self,
        binding: &Place,
        captures: &[nir::program::Capture],
        report: &Report,
        snapshots: &[BTreeMap<Place, BTreeSet<Place>>],
        guard_snapshots: &[BTreeMap<Place, PointerTargetGuards>],
        exit_states: impl Iterator<Item = &'b State> + Clone,
        inputs: &MemoryTargets,
        flow: &mut Flow,
        source: SourceRange,
    ) -> Result<(), Error> {
        let Some(package) = self.package else {
            return Ok(());
        };
        if snapshots.len() != guard_snapshots.len() {
            return Err(error(
                source,
                "capture guard snapshots disagree with pointer exits",
            ));
        }
        if snapshots.is_empty() {
            return Ok(());
        }
        let mut restored: BTreeMap<Place, BTreeSet<Place>> = BTreeMap::new();
        let mut counts: BTreeMap<Place, usize> = BTreeMap::new();
        let mut restored_guards = BTreeMap::<Place, PointerTargetGuards>::new();
        for (snapshot, guards) in snapshots.iter().zip(guard_snapshots) {
            self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or_else(|| error(source, "capture pointer writeback budget exhausted"))?;
            for (slot, targets) in snapshot {
                self.fuel = self
                    .fuel
                    .checked_sub(captures.len() + slot.fields.len() + binding.fields.len() + 1)
                    .ok_or_else(|| error(source, "capture pointer writeback budget exhausted"))?;
                let (index, capture) = captures
                    .iter()
                    .enumerate()
                    .find(|(_, capture)| root(capture.binding) == slot.root)
                    .ok_or_else(|| error(source, "pointer exit names an unknown capture"))?;
                let destination = Place {
                    root: binding.root,
                    fields: binding
                        .fields
                        .iter()
                        .copied()
                        .chain(std::iter::once(index))
                        .chain(slot.fields.iter().copied())
                        .collect(),
                };
                if !self.captured_slot_present(
                    &flow.state,
                    &Place {
                        root: binding.root,
                        fields: binding
                            .fields
                            .iter()
                            .copied()
                            .chain(std::iter::once(index))
                            .collect(),
                    },
                    &capture.ty,
                    &slot.fields,
                    source,
                )? {
                    continue;
                }
                let layout =
                    super::memory::pointer_input_layout(self.package, &capture.ty, &mut self.fuel)
                        .map_err(|e| error(source, e))?;
                let subjects: Vec<_> = layout
                    .into_iter()
                    .filter(|leaf| leaf.storage_fields == slot.fields)
                    .map(|leaf| leaf.pointee)
                    .collect();
                if subjects.is_empty() {
                    return Err(error(
                        source,
                        "pointer exit names a non-pointer capture slot",
                    ));
                }
                *counts.entry(destination.clone()).or_default() += 1;
                let mapped = restored.entry(destination.clone()).or_default();
                let mapped_guards = restored_guards.entry(destination).or_default();
                self.pointer_work(targets, source)?;
                for target in targets {
                    if *target == unknown_pointer_target()
                        || *target == invalidated_pointer_target()
                    {
                        mapped.insert(target.clone());
                        self.merge_pointer_conditions(
                            mapped_guards,
                            &BTreeSet::from([target.clone()]),
                            &[],
                            source,
                        )?;
                        continue;
                    }
                    let (origin, ty) =
                        cleanup_memory::project_origin(report, package, target, &mut self.fuel, 0)
                            .map_err(|reason| error(source, reason))?
                            .ok_or_else(|| {
                                error(
                                    source,
                                    "captured pointer cannot retain invocation-local storage",
                                )
                            })?;
                    if !subjects.contains(&ty) {
                        return Err(error(
                            source,
                            "captured pointer writeback pointee type mismatch",
                        ));
                    }
                    let mut conditions = vec![];
                    if let Some(original) = guards.get(slot).and_then(|entries| entries.get(target))
                    {
                        self.charge_pointer_conditions(original, source)?;
                        for (path, tag) in original {
                            self.fuel = self
                                .fuel
                                .checked_sub(binding.fields.len() + 1)
                                .ok_or_else(|| {
                                    error(source, "capture guard receiver path budget exhausted")
                                })?;
                            let mut rebased = binding.fields.clone();
                            rebased.push(index);
                            rebased.extend(path);
                            conditions.push((rebased, *tag));
                        }
                    }
                    let targets = self
                        .map_memory_target(&origin, inputs, flow, source)?
                        .unwrap_or_else(|| BTreeSet::from([unknown_pointer_target()]));
                    self.merge_pointer_conditions(mapped_guards, &targets, &conditions, source)?;
                    mapped.extend(targets);
                }
            }
        }
        for (slot, targets) in &mut restored {
            self.fuel = self
                .fuel
                .checked_sub((slot.fields.len() + 1).saturating_mul(snapshots.len()))
                .ok_or_else(|| error(source, "capture exit availability budget exhausted"))?;
            let original = Place {
                root: root(captures[slot.fields[binding.fields.len()]].binding),
                fields: slot.fields[binding.fields.len() + 1..].to_vec(),
            };
            let mut needed = 0;
            for state in exit_states.clone() {
                if self.captured_slot_present(
                    state,
                    &Place {
                        root: original.root,
                        fields: vec![],
                    },
                    &captures[slot.fields[binding.fields.len()]].ty,
                    &original.fields,
                    source,
                )? {
                    needed += 1;
                }
            }
            if counts.get(slot).copied().unwrap_or(0) != needed {
                targets.insert(unknown_pointer_target());
                // Missing exit evidence introduces an unconditional unknown,
                // even if another exit mapped a conditional source to it.
                if let Some(entries) = restored_guards.get_mut(slot) {
                    entries.remove(&unknown_pointer_target());
                }
            }
        }
        // Retired capture slots lose their old provenance. Target availability
        // remains a separate access obligation; rebasing never revives storage.
        self.fuel = self
            .fuel
            .checked_sub(flow.pointers.len())
            .ok_or_else(|| error(source, "capture pointer replacement budget exhausted"))?;
        flow.pointers.retain(|slot, _| {
            slot.root != binding.root || !slot.fields.starts_with(&binding.fields)
        });
        self.fuel = self
            .fuel
            .checked_sub(flow.pointer_guards.len())
            .ok_or_else(|| error(source, "capture guard invalidation budget exhausted"))?;
        flow.pointer_guards.retain(|slot, _| {
            slot.root != binding.root || !slot.fields.starts_with(&binding.fields)
        });
        flow.pointers.extend(restored);
        restored_guards.retain(|_, entries| {
            entries.retain(|_, conditions| !conditions.is_empty());
            !entries.is_empty()
        });
        flow.pointer_guards.extend(restored_guards);
        Ok(())
    }

    fn call_prepared_closure(
        &mut self,
        binding: &Place,
        callable: &Callable,
        prepared: ClosureArguments,
        flow: &mut Flow,
        source: SourceRange,
        depth: usize,
    ) -> Result<Option<Flow>, Error> {
        let definition = &callable.definition;
        let K::Closure {
            parameters,
            captures,
            body,
        } = &definition.kind
        else {
            return Err(error(source, "invalid prepared closure provenance"));
        };
        let Term::Function {
            parameters: types,
            result: closure_result,
        } = definition.ty.callable_signature()
        else {
            return Err(error(source, "invalid prepared closure signature"));
        };
        let ClosureArguments {
            pointers: argument_pointers,
            callables: mut argument_callables,
            fields: mut argument_fields,
            domains: mut argument_domains,
        } = prepared;
        let mut inner = Flow {
            pointer_slot_mutations: Some(self.pointer_slot_mutations(definition)?),
            borrowed_scope: self.fresh_borrowed_scope(source)?,
            live: true,
            ..Default::default()
        };
        let mut capture_roots = BTreeSet::new();
        let mut borrowed_roots = BTreeMap::new();
        for fields in argument_pointers.values() {
            for (path, targets) in fields {
                self.fuel = self.fuel.checked_sub(path.len() + 1).ok_or_else(|| {
                    error(source, "closure cleanup input snapshot budget exhausted")
                })?;
                self.pointer_work(targets, source)?;
            }
        }
        let mut cleanup_targets = argument_pointers.clone();
        for (index, capture) in captures.iter().enumerate() {
            self.fuel = self
                .fuel
                .checked_sub(binding.fields.len() + 1)
                .ok_or_else(|| error(source, "closure capture path budget exhausted"))?;
            let id = root(capture.binding);
            inner
                .state
                .import_position(
                    id,
                    &flow.state,
                    &Place {
                        root: binding.root,
                        fields: binding
                            .fields
                            .iter()
                            .copied()
                            .chain(std::iter::once(index))
                            .collect(),
                    },
                    &mut self.fuel,
                )
                .map_err(|e| error(source, format!("closure environment import: {e:?}")))?;
            inner.roots.insert(id);
            inner.order.push(id);
            inner.types.insert(id, capture.ty.clone());
            if let Some((pointer_depth, subject)) =
                super::memory::pointer_representation(self.package, &capture.ty, &mut self.fuel)
                    .map_err(|reason| error(source, reason))?
            {
                self.declare_closure_borrow(
                    &mut inner,
                    parameters.len() + index,
                    capture.binding,
                    &subject,
                    source,
                )?;
                let slot = parameters.len() + index;
                borrowed_roots.insert(
                    PlaceId {
                        owner: u64::MAX - 1,
                        local: u32::try_from(slot)
                            .map_err(|_| error(source, "too many closure pointer parameters"))?,
                    },
                    BorrowedRoot::Parameter {
                        parameter: slot,
                        input_fields: vec![],
                        pointee: subject.clone(),
                    },
                );
                if let Some(targets) = flow.pointers.get(&Place {
                    root: binding.root,
                    fields: binding
                        .fields
                        .iter()
                        .copied()
                        .chain(std::iter::once(index))
                        .chain(std::iter::repeat_n(0, pointer_depth))
                        .collect(),
                }) {
                    self.pointer_work(targets, source)?;
                    cleanup_targets.insert(slot, BTreeMap::from([(vec![], targets.clone())]));
                }
                if !self.effect_memory_inputs.is_empty() {
                    let targets = flow
                        .pointers
                        .get(&Place {
                            root: binding.root,
                            fields: binding
                                .fields
                                .iter()
                                .copied()
                                .chain(std::iter::once(index))
                                .chain(std::iter::repeat_n(0, pointer_depth))
                                .collect(),
                        })
                        .ok_or_else(|| {
                            error(source, "captured pointer provenance is not established")
                        })?;
                    self.authorize_effect_borrow(
                        flow,
                        targets,
                        &mut inner,
                        capture.binding,
                        source,
                    )?;
                }
            }
            if self.package.is_some() && self.storage_inputs_are_closed(&capture.ty, source)? {
                let domains = copy_enum_domains(
                    callable
                        .enum_captures
                        .get(&root(capture.binding))
                        .unwrap_or(&EnumDomains::new()),
                    &mut self.fuel,
                    source,
                )?;
                let mut actual = self.capture_input_targets(
                    flow,
                    binding,
                    index,
                    &capture.ty,
                    &domains,
                    false,
                    source,
                )?;
                actual.remove(&vec![]);
                if !actual.is_empty() {
                    let slot = parameters.len() + index;
                    let fields = self.declare_input_field_borrows(
                        &mut inner,
                        capture.binding,
                        &capture.ty,
                        &actual,
                        source,
                    )?;
                    for (path, targets) in fields {
                        for target in targets {
                            let pointee = inner.types.get(&target.root).ok_or_else(|| {
                                error(source, "closure captured field type missing")
                            })?;
                            super::memory::bound_type(pointee, &mut self.fuel)
                                .map_err(|reason| error(source, reason))?;
                            borrowed_roots.insert(
                                target.root,
                                BorrowedRoot::Parameter {
                                    parameter: slot,
                                    input_fields: path.clone(),
                                    pointee: pointee.clone(),
                                },
                            );
                        }
                    }
                    cleanup_targets.insert(slot, actual);
                }
            }
            if let Some(captured) = callable.captures.get(&Place {
                root: id,
                fields: vec![],
            }) {
                inner.callables.insert(id, captured.clone());
            }
            for (place, evidence) in &callable.captures {
                if place.root == id && !place.fields.is_empty() {
                    inner
                        .callable_fields
                        .insert(place.clone(), evidence.clone());
                }
            }
            if let Some(domains) = callable.enum_captures.get(&id) {
                let domains = copy_enum_domains(domains, &mut self.fuel, source)?;
                self.install_enum_domains(
                    &mut inner,
                    &Place {
                        root: id,
                        fields: vec![],
                    },
                    domains,
                    source,
                )?;
            }
            capture_roots.insert(id);
        }
        for (index, (parameter, ty)) in parameters.iter().zip(types).enumerate() {
            self.declare(&mut inner, *parameter, &ty.ty, source)?;
            if let Some((_, subject)) =
                super::memory::pointer_representation(self.package, &ty.ty, &mut self.fuel)
                    .map_err(|reason| error(source, reason))?
            {
                self.declare_closure_borrow(&mut inner, index, *parameter, &subject, source)?;
                borrowed_roots.insert(
                    PlaceId {
                        owner: u64::MAX - 1,
                        local: u32::try_from(index)
                            .map_err(|_| error(source, "too many closure pointer parameters"))?,
                    },
                    BorrowedRoot::Parameter {
                        parameter: index,
                        input_fields: vec![],
                        pointee: subject.clone(),
                    },
                );
                if !self.effect_memory_inputs.is_empty() {
                    let targets = argument_pointers
                        .get(&index)
                        .and_then(|fields| fields.get(&vec![]))
                        .ok_or_else(|| {
                            error(
                                source,
                                "closure pointer argument provenance is not established",
                            )
                        })?;
                    self.authorize_effect_borrow(flow, targets, &mut inner, *parameter, source)?;
                }
            }
            if let Some(actual) = argument_pointers.get(&index) {
                let fields = self
                    .declare_input_field_borrows(&mut inner, *parameter, &ty.ty, actual, source)?;
                for (path, targets) in fields {
                    for target in targets {
                        let pointee = inner
                            .types
                            .get(&target.root)
                            .ok_or_else(|| error(source, "closure borrowed field type missing"))?;
                        super::memory::bound_type(pointee, &mut self.fuel)
                            .map_err(|e| error(source, e))?;
                        borrowed_roots.insert(
                            target.root,
                            BorrowedRoot::Parameter {
                                parameter: index,
                                input_fields: path.clone(),
                                pointee: pointee.clone(),
                            },
                        );
                    }
                }
            }
            if let Some(callable) = argument_callables.remove(&index) {
                self.store_callable(&mut inner, root(*parameter), callable, source)?;
            }
            if let Some(fields) = argument_fields.remove(&index) {
                self.install_callable_fields(
                    &mut inner,
                    &Place {
                        root: root(*parameter),
                        fields: vec![],
                    },
                    fields,
                    source,
                )?;
            }
            if let Some(domains) = argument_domains.remove(&index) {
                self.install_enum_domains(
                    &mut inner,
                    &Place {
                        root: root(*parameter),
                        fields: vec![],
                    },
                    domains,
                    source,
                )?;
            }
        }
        let effect_inputs = self.effect_inputs_for(&callable.definition.ty, source)?;
        let memory_inputs = self.copy_effect_memory_inputs(source)?;
        let outer_report = std::mem::take(&mut self.report);
        self.report.borrowed_roots = borrowed_roots;
        let outer_captured_callables = std::mem::replace(
            &mut self.captured_roots,
            captures.iter().map(|c| root(c.binding)).collect(),
        );
        let outer_persistent = std::mem::replace(&mut self.persistent_roots, capture_roots.clone());
        let outer_pending = std::mem::take(&mut self.pending);
        let outer_loops = std::mem::take(&mut self.loops);
        let outer_contract = self.exit_contract.take();
        let outer_handler = self.handler_result.take();
        let outer_effect_return = std::mem::replace(&mut self.effect_return_enabled, false);
        let outer_return_values = std::mem::take(&mut self.effect_return_values);
        let outer_effect_loops = std::mem::take(&mut self.effect_loop_targets);
        let outer_effect_memory = std::mem::take(&mut self.effect_memory_inputs);
        let outer_effect_results = std::mem::take(&mut self.effect_result_inputs);
        let outer_completions = std::mem::take(&mut self.handler_completions);
        self.effect_memory_inputs = memory_inputs;
        self.effect_result_inputs = effect_inputs;
        self.report.effect_parameters = self.copy_effect_result_inputs(source)?;
        let result = self.in_temporary_scope(
            body.source,
            TemporaryBoundary::FunctionResult,
            &mut inner,
            |this, inner| this.expr(body, inner, depth + 1),
        );
        self.loops = outer_loops;
        self.exit_contract = outer_contract;
        self.handler_result = outer_handler;
        self.effect_return_enabled = outer_effect_return;
        self.effect_return_values = outer_return_values;
        self.effect_loop_targets = outer_effect_loops;
        self.effect_memory_inputs = outer_effect_memory;
        self.effect_result_inputs = outer_effect_results;
        self.handler_completions = outer_completions;
        let result = result.and_then(|()| {
            if inner.live {
                self.exit(&mut inner, body.source, CleanupReason::FunctionEnd)
            } else {
                Ok(())
            }
        });
        self.pending = outer_pending;
        let invocation = std::mem::replace(&mut self.report, outer_report);
        self.persistent_roots = outer_persistent;
        self.captured_roots = outer_captured_callables;
        result?;
        if invocation.captured_pointer_guard_exits.len() != invocation.exits.len()
            || invocation.failed_capture_pointer_guard_exits.len() != invocation.failed_exits.len()
            || invocation.effect_capture_pointer_guard_exits.len() != invocation.effect_exits.len()
            || invocation.captured_pointer_exits.len() != invocation.exits.len()
            || invocation.failed_capture_pointer_exits.len() != invocation.failed_exits.len()
            || invocation.effect_capture_pointer_exits.len() != invocation.effect_exits.len()
            || invocation.failed_capture_enum_exits.len() != invocation.failed_exits.len()
            || invocation.effect_capture_enum_exits.len() != invocation.effect_exits.len()
        {
            return Err(error(
                source,
                "closure pointer snapshots disagree with exit states",
            ));
        }
        if self.cleanup.is_some() {
            let package = self
                .package
                .ok_or_else(|| error(source, "closure cleanup footprint needs a package"))?;
            let accesses = invocation
                .cleanup_memory_accesses(package, &mut self.fuel)
                .map_err(|e| error(source, e))?;
            self.memory_accesses(&accesses, &cleanup_targets, flow, source)?;
            for access in accesses {
                let targets = self
                    .map_memory_target(&access.target, &cleanup_targets, flow, source)?
                    .ok_or_else(|| error(source, "closure cleanup target is not established"))?;
                self.report.cleanup_call_accesses.push((access, targets));
            }
        }
        // Failed checks unwind this invocation; they are never evidence for a
        // normal continuation. Reflect consumed captures before reporting the
        // caller's outstanding obligations. The call boundary has one runtime
        // failure continuation: join only captured ownership across its exits,
        // retaining conditional tokens instead of publishing competing plans
        // for the same caller owner. Invocation-local exits remain separate.
        let mut failed_captures: Option<State> = None;
        for failed in &invocation.failed_exits {
            let captured = failed
                .select_roots(&capture_roots, &mut self.fuel)
                .map_err(|e| error(source, format!("closure failure state: {e:?}")))?;
            failed_captures = Some(match failed_captures {
                None => captured,
                Some(previous) => previous
                    .join(&captured, &mut self.fuel)
                    .map_err(|e| error(source, format!("closure failure join: {e:?}")))?,
            });
        }
        let mut failure = None;
        if let Some(captured) = failed_captures {
            let mut failed_flow = flow.clone();
            for (index, capture) in captures.iter().enumerate() {
                self.fuel = self
                    .fuel
                    .checked_sub(binding.fields.len() + 1)
                    .ok_or_else(|| error(source, "closure capture path budget exhausted"))?;
                failed_flow
                    .state
                    .update_position(
                        &Place {
                            root: binding.root,
                            fields: binding
                                .fields
                                .iter()
                                .copied()
                                .chain(std::iter::once(index))
                                .collect(),
                        },
                        &captured,
                        root(capture.binding),
                        &mut self.fuel,
                    )
                    .map_err(|e| error(source, format!("closure failure writeback: {e:?}")))?;
            }
            self.restore_capture_pointer_origins(
                binding,
                captures,
                &invocation,
                &invocation.failed_capture_pointer_exits,
                &invocation.failed_capture_pointer_guard_exits,
                invocation.failed_exits.iter(),
                &cleanup_targets,
                &mut failed_flow,
                source,
            )?;
            self.restore_exit_capture_domains(
                &mut failed_flow,
                binding,
                captures,
                &invocation.failed_capture_enum_exits,
                source,
            )?;
            failure = Some(failed_flow);
        }
        let mut escaped_captures: Option<State> = None;
        // Locally installed handlers cannot abandon the closure's caller.
        // Only effects exported by this callable's row can cross that boundary.
        let outward_effects = matches!(closure_result.as_ref(), Term::Computation {effects,..} if !effects.is_empty());
        for (_, escaped) in invocation.effect_exits.iter().filter(|_| outward_effects) {
            let captured = escaped
                .select_roots(&capture_roots, &mut self.fuel)
                .map_err(|e| error(source, format!("closure effect escape state: {e:?}")))?;
            escaped_captures = Some(match escaped_captures {
                None => captured,
                Some(previous) => previous
                    .join(&captured, &mut self.fuel)
                    .map_err(|e| error(source, format!("closure effect escape join: {e:?}")))?,
            });
        }
        if let Some(captured) = escaped_captures {
            let mut escaped_flow = flow.clone();
            for (index, capture) in captures.iter().enumerate() {
                self.fuel = self
                    .fuel
                    .checked_sub(binding.fields.len() + 1)
                    .ok_or_else(|| error(source, "closure capture path budget exhausted"))?;
                escaped_flow
                    .state
                    .update_position(
                        &Place {
                            root: binding.root,
                            fields: binding
                                .fields
                                .iter()
                                .copied()
                                .chain(std::iter::once(index))
                                .collect(),
                        },
                        &captured,
                        root(capture.binding),
                        &mut self.fuel,
                    )
                    .map_err(|e| {
                        error(source, format!("closure effect escape writeback: {e:?}"))
                    })?;
            }
            self.restore_capture_pointer_origins(
                binding,
                captures,
                &invocation,
                &invocation.effect_capture_pointer_exits,
                &invocation.effect_capture_pointer_guard_exits,
                invocation.effect_exits.iter().map(|(_, state)| state),
                &cleanup_targets,
                &mut escaped_flow,
                source,
            )?;
            self.restore_exit_capture_domains(
                &mut escaped_flow,
                binding,
                captures,
                &invocation.effect_capture_enum_exits,
                source,
            )?;
            self.effect_escape_checkpoint(&escaped_flow, source)?;
        }
        let mut exits = invocation.exits.iter();
        let Some(first) = exits.next() else {
            self.report.closures.push(ClosureReport {
                invoked: true,
                definition: definition.clone(),
                source,
                captures: captures.iter().map(|c| root(c.binding)).collect(),
                invocation,
            });
            flow.live = false;
            return Ok(failure);
        };
        let mut merged = first
            .select_roots(&capture_roots, &mut self.fuel)
            .map_err(|e| error(source, format!("closure exit: {e:?}")))?;
        for exit in exits {
            let next = exit
                .select_roots(&capture_roots, &mut self.fuel)
                .map_err(|e| error(source, format!("closure exit: {e:?}")))?;
            merged = merged
                .join(&next, &mut self.fuel)
                .map_err(|e| error(source, format!("closure exit join: {e:?}")))?;
        }
        for (index, capture) in captures.iter().enumerate() {
            self.fuel = self
                .fuel
                .checked_sub(binding.fields.len() + 1)
                .ok_or_else(|| error(source, "closure capture path budget exhausted"))?;
            flow.state
                .update_position(
                    &Place {
                        root: binding.root,
                        fields: binding
                            .fields
                            .iter()
                            .copied()
                            .chain(std::iter::once(index))
                            .collect(),
                    },
                    &merged,
                    root(capture.binding),
                    &mut self.fuel,
                )
                .map_err(|e| error(source, format!("closure environment writeback: {e:?}")))?;
        }
        self.restore_capture_pointer_origins(
            binding,
            captures,
            &invocation,
            &invocation.captured_pointer_exits,
            &invocation.captured_pointer_guard_exits,
            invocation.exits.iter(),
            &cleanup_targets,
            flow,
            source,
        )?;
        // A call mutates capture cells, not the enclosing code identity. Retain
        // a target only when every normal exit provides compatible evidence.
        if invocation.captured_callable_exits.len() != invocation.captured_enum_exits.len() {
            return Err(error(source, "captured callable and enum exits disagree"));
        }
        let mut updated: Option<(BTreeMap<Place, Callable>, BTreeMap<PlaceId, EnumDomains>)> = None;
        for (exit, domains) in invocation
            .captured_callable_exits
            .iter()
            .zip(&invocation.captured_enum_exits)
        {
            let mut values = BTreeMap::new();
            for (place, evidence) in exit {
                values.insert(
                    place.clone(),
                    restore_callable_evidence(evidence, &mut self.fuel, 0)?,
                );
            }
            updated = Some(if let Some((previous, tags)) = updated {
                join_capture_evidence(
                    &previous,
                    &values,
                    &tags,
                    domains,
                    &mut self.fuel,
                    source,
                    0,
                )?
            } else {
                (
                    values,
                    copy_captured_enums(domains, &mut self.fuel, source)?,
                )
            });
        }
        let (updated_captures, updated_domains) = updated.unwrap_or_default();
        self.install_capture_domains(flow, binding, captures, &updated_domains, source)?;
        if let Some(receiver) = flow.callable_at_mut(binding) {
            (receiver.captures, receiver.enum_captures) = (updated_captures, updated_domains);
        }
        flow.value_callable = callable_return_summary(&invocation, &mut self.fuel, source)?;
        (flow.value_callables, flow.value_variants) =
            callable_aggregate_return_summary(&invocation, &mut self.fuel, source)?;
        self.report.closures.push(ClosureReport {
            invoked: true,
            definition: definition.clone(),
            source,
            captures: captures.iter().map(|c| root(c.binding)).collect(),
            invocation,
        });
        Ok(failure)
    }
    fn let_pattern(
        &mut self,
        pattern: &Pattern,
        value: &Expr,
        guard: Option<&Expr>,
        otherwise: Option<&Expr>,
        flow: &mut Flow,
        source: SourceRange,
        depth: usize,
    ) -> Result<(), Error> {
        let (pattern, mut guards) = outer_guards(pattern);
        guards.extend(guard);
        let ignored_place = matches!(pattern, Pattern::Wild)
            .then(|| expression_place(value))
            .flatten();
        let origins = if let Some(place) = &ignored_place {
            flow.state
                .require_place(place)
                .map_err(|_| error(source, "wildcard refers to an unknown owned position"))?;
            BTreeMap::new()
        } else {
            let origins = self.evaluate_value_origins(value, flow, depth + 1)?;
            if !flow.live {
                return Ok(());
            }
            origins
        };
        let pointer_guards = if ignored_place.is_some() {
            BTreeMap::new()
        } else {
            self.copy_value_pointer_guards(&flow.value_pointer_guards, source)?
        };
        let known = known_pattern_with_domains(
            pattern,
            value,
            &flow.value_variants,
            &mut self.fuel,
            source,
        )?
        .or_else(|| irrefutable_shape(pattern, 0).then_some(true));
        let mut success = None;
        let mut guard_known = Some(true);
        if known != Some(false) {
            let mut candidate = flow.clone();
            let outer_report = self.take_nested_report(source)?;
            let outer_observations = self.candidate_observations.replace(Report::default());
            if ignored_place.is_none() {
                self.bind_pattern_with_pointer_guards(
                    pattern,
                    &value.ty,
                    &origins,
                    &pointer_guards,
                    &mut candidate,
                    source,
                    &mut vec![],
                    depth + 1,
                )?;
            }
            for guard in &guards {
                self.observe_candidate(guard, &candidate, depth + 1)?;
                match constant_bool(guard, 0) {
                    Some(false) => {
                        guard_known = Some(false);
                        break;
                    }
                    None => guard_known = None,
                    _ => (),
                }
            }
            let prepared = std::mem::replace(&mut self.report, outer_report);
            let observed =
                std::mem::replace(&mut self.candidate_observations, outer_observations).unwrap();
            self.report.append(observed);
            if guard_known != Some(false) {
                self.report.append(prepared);
                success = Some(candidate);
            }
        }
        // Absence of else on typed NIR carries the frontend's irrefutability
        // check. A known failing pattern still cannot bypass that contract.
        let failure = known == Some(false)
            || guard_known == Some(false)
            || otherwise.is_some() && (known != Some(true) || guard_known != Some(true));
        if failure {
            let otherwise =
                otherwise.ok_or_else(|| error(source, "failing pattern binding lacks else"))?;
            let mut failed = flow.clone();
            if ignored_place.is_none() {
                self.temporary(value, origins.clone(), &mut failed)?;
            }
            self.scope(otherwise, &mut failed, depth + 1)?;
            if failed.live {
                return Err(error(
                    source,
                    "let-else failure branch does not establish divergence",
                ));
            }
        }
        self.report.bindings.push(LetReport {
            source,
            pattern: pattern.clone(),
            guards: guards.iter().map(|g| (*g).clone()).collect(),
            success: success.is_some(),
            failure,
        });
        if let Some(success) = success {
            *flow = success;
        } else {
            flow.live = false;
        }
        Ok(())
    }
    fn observe_guard(&mut self, expr: &Expr, flow: &Flow, depth: usize) -> Result<(), Error> {
        self.observe_guard_result(expr, flow, depth).map(|_| ())
    }
    // Template dispatch observes an existing environment-free identity. It
    // neither transfers the callable nor requires a user Copy implementation.
    fn observe_template_identity(&mut self, expr: &Expr, flow: &Flow) -> Result<(), Error> {
        self.fuel = self.fuel.checked_sub(1).ok_or_else(|| {
            error(
                expr.source,
                "template identity observation budget exhausted",
            )
        })?;
        if !matches!(&expr.ty, Term::Function { parameters, .. }
            if parameters.iter().any(|p| p.sort != nir::Sort::Runtime))
        {
            return Err(error(
                expr.source,
                "template comparison requires a runtime template identity",
            ));
        }
        match expr.kind {
            K::Local(binding) => flow
                .state
                .require_present(&Place {
                    root: root(binding),
                    fields: vec![],
                })
                .map_err(|_| {
                    error(
                        expr.source,
                        "template comparison observes an unavailable identity",
                    )
                }),
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. } => Ok(()),
            _ => Err(error(
                expr.source,
                "template comparison requires a stored identity",
            )),
        }
    }
    /// Validate a child that the enclosing ordinary expression will analyze
    /// again. Only that ordinary traversal owns its runtime cleanup records.
    fn check_guard_observation(
        &mut self,
        expr: &Expr,
        flow: &Flow,
        depth: usize,
    ) -> Result<Option<Callable>, Error> {
        let report = self.take_nested_report(expr.source)?;
        let observations = self.candidate_observations.take();
        let result = self.observe_guard_result(expr, flow, depth);
        self.report = report;
        self.candidate_observations = observations;
        result
    }
    fn observe_candidate(
        &mut self,
        expr: &Expr,
        flow: &Flow,
        depth: usize,
    ) -> Result<Option<Callable>, Error> {
        if self.candidate_observations.is_none() {
            return self.observe_guard_result(expr, flow, depth);
        }
        let prepared = self.take_nested_report(expr.source)?;
        let result = self.observe_guard_result(expr, flow, depth);
        let observed = std::mem::replace(&mut self.report, prepared);
        self.candidate_observations
            .as_mut()
            .unwrap()
            .append(observed);
        result
    }
    fn scalar_packet_shape(
        &mut self,
        ty: &Term,
        source: SourceRange,
        depth: usize,
    ) -> Result<bool, Error> {
        if depth >= 128 {
            return Err(error(source, "scalar packet observation depth exhausted"));
        }
        self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or_else(|| error(source, "scalar packet observation budget exhausted"))?;
        if let Some(fields) = nir::normal_forms::tuple_fields(self.package, ty) {
            for field in fields {
                if !self.scalar_packet_shape(field, source, depth + 1)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        if let Term::Record(fields) = ty {
            for (_, field) in fields {
                if !self.scalar_packet_shape(field, source, depth + 1)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        if let Term::ErrorResult { errors, result } = ty {
            for field in std::iter::once(result.as_ref()).chain(errors) {
                if !self.scalar_packet_shape(field, source, depth + 1)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        if let Term::Nominal { declaration, .. } = ty {
            let Some(package) = self.package else {
                return Ok(false);
            };
            let variants = package
                .enums
                .iter()
                .find(|item| item.declaration == *declaration)
                .map(|item| item.variants.len());
            for index in 0..variants.unwrap_or(1) {
                let Some(fields) = nir::normal_forms::nominal_value_fields_budgeted(
                    package,
                    ty,
                    variants.map(|_| index as u32),
                    &mut self.fuel,
                ) else {
                    return Ok(false);
                };
                for field in fields {
                    if !self.scalar_packet_shape(&field, source, depth + 1)? {
                        return Ok(false);
                    }
                }
            }
            return Ok(true);
        }
        Ok(nir::program::scalar_observation_shape(ty))
    }
    fn observe_associated_value(&mut self, ty: &Term, source: SourceRange) -> Result<(), Error> {
        // Selection/initialization is comptime work. An unresolved identity is
        // not permission to invent owned resources, pointer provenance or a
        // callable environment. Scalar Copy results need no such runtime state.
        nir::type_computations::charge_term(ty, &mut self.fuel)
            .map_err(|_| error(source, "associated value observation budget exhausted"))?;
        if !nir::program::scalar_observation_shape_in_package(self.package, ty)
            || (self.delivery)(ty).map_err(|reason| error(source, reason))? != Delivery::Copy
        {
            return Err(error(
                source,
                "associated value requires materialized resource evidence",
            ));
        }
        Ok(())
    }

    fn observation_value_type(&mut self, ty: &Term, source: SourceRange) -> Result<bool, Error> {
        if let Term::Nominal { declaration, .. } = ty {
            if let Some(package) = self.package {
                if package.internal_return_packets.contains(declaration) {
                    if let Some(packet) = package
                        .enums
                        .iter()
                        .find(|packet| packet.declaration == *declaration)
                    {
                        for index in 0..packet.variants.len() {
                            let fields = nir::normal_forms::nominal_value_fields_budgeted(
                                package,
                                ty,
                                Some(index as u32),
                                &mut self.fuel,
                            )
                            .ok_or_else(|| {
                                error(
                                    source,
                                    "scalar packet layout is unresolved or budget exhausted",
                                )
                            })?;
                            for field in fields {
                                if !self.scalar_packet_shape(&field, source, 0)?
                                    || (self.delivery)(&field)
                                        .map_err(|reason| error(source, reason))?
                                        != Delivery::Copy
                                {
                                    return Ok(false);
                                }
                            }
                        }
                        return Ok(true);
                    }
                }
            }
        }
        if matches!(
            ty,
            Term::Nominal { .. } | Term::Record(_) | Term::ErrorResult { .. }
        ) || nir::normal_forms::tuple_fields(self.package, ty).is_some()
        {
            // Copy permits candidate-local delivery; the recursive shape
            // check separately excludes pointers and owned/captured state
            // that would need provenance or failure-path transfer.
            return Ok((self.delivery)(ty).map_err(|reason| error(source, reason))?
                == Delivery::Copy
                && self.scalar_packet_shape(ty, source, 0)?);
        }
        Ok(
            nir::program::scalar_observation_shape_in_package(self.package, ty)
                || matches!(ty, Term::Function { .. } | Term::UnsafeFunction(_))
                    && (self.delivery)(ty).map_err(|e| error(source, e))? == Delivery::Copy,
        )
    }
    // Validate evaluation effects without requiring indirect slot contents to
    // have been materialized already. The subsequent ordinary call walk checks
    // every dereference on a resource snapshot, including pointer availability.
    fn check_pointer_observation(
        &mut self,
        expr: &Expr,
        flow: &Flow,
        depth: usize,
    ) -> Result<(), Error> {
        if depth >= 128 || self.fuel == 0 {
            return Err(error(expr.source, "pointer observation budget exhausted"));
        }
        self.fuel -= 1;
        match &expr.kind {
            K::ReadPointer(pointer) => {
                if !super::memory::readable_representation(self.package, &expr.ty, &mut self.fuel)
                    .map_err(|e| error(expr.source, e))?
                    || (self.delivery)(&expr.ty).map_err(|e| error(expr.source, e))?
                        != Delivery::Copy
                {
                    return Err(error(
                        expr.source,
                        "guard pointee observation requires Copy evidence",
                    ));
                }
                self.check_pointer_observation(pointer, flow, depth + 1)
            }
            K::PointerField { pointer, .. } | K::PointerAnonymousField { pointer, .. } => {
                self.check_pointer_observation(pointer, flow, depth + 1)
            }
            K::If {
                condition,
                then_branch,
                else_branch: Some(other),
            } => {
                self.observe_guard(condition, flow, depth + 1)?;
                match constant_bool(condition, 0) {
                    Some(true) => self.check_pointer_observation(then_branch, flow, depth + 1),
                    Some(false) => self.check_pointer_observation(other, flow, depth + 1),
                    None => {
                        self.check_pointer_observation(then_branch, flow, depth + 1)?;
                        self.check_pointer_observation(other, flow, depth + 1)
                    }
                }
            }
            _ => self.observe_guard(expr, flow, depth + 1),
        }
    }
    fn install_observation_input_borrows(
        &mut self,
        frame: &mut Flow,
        binding: LocalId,
        parameter: usize,
        ty: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        // This isolated frame proves default evaluation under its parameter
        // premises. Ordinary call preparation separately maps every access to
        // the actual arguments; these roots never supply caller validity or
        // assert that two input pointers cannot alias.
        let layout = super::memory::pointer_input_layout(self.package, ty, &mut self.fuel)
            .map_err(|reason| error(source, reason))?;
        for leaf in layout {
            let local = self.next_borrowed_pointer;
            self.next_borrowed_pointer = local
                .checked_add(1)
                .ok_or_else(|| error(source, "observation input identity exhausted"))?;
            let id = PlaceId {
                owner: u64::MAX - 4,
                local,
            };
            let shape = (self.shape)(&leaf.pointee).map_err(|reason| error(source, reason))?;
            frame
                .state
                .declare(id, &shape, true, &mut self.fuel)
                .map_err(|reason| error(source, format!("observation input shape: {reason:?}")))?;
            super::memory::bound_type(&leaf.pointee, &mut self.fuel)
                .map_err(|reason| error(source, reason))?;
            frame.types.insert(id, leaf.pointee.clone());
            self.report.borrowed_roots.insert(
                id,
                BorrowedRoot::Parameter {
                    parameter,
                    input_fields: leaf.input_fields,
                    pointee: leaf.pointee,
                },
            );
            self.fuel = self
                .fuel
                .checked_sub(leaf.storage_fields.len().saturating_add(1))
                .ok_or_else(|| error(source, "observation input path budget exhausted"))?;
            let slot = Place {
                root: root(binding),
                fields: leaf.storage_fields,
            };
            let target = Place {
                root: id,
                fields: vec![],
            };
            frame
                .pointers
                .entry(slot.clone())
                .or_default()
                .insert(target.clone());
            if !leaf.guards.is_empty() {
                self.charge_pointer_conditions(&leaf.guards, source)?;
                frame
                    .pointer_guards
                    .entry(slot)
                    .or_default()
                    .insert(target, leaf.guards);
            }
            frame.memory_exposed = true;
        }
        Ok(())
    }
    fn check_guard_arguments(
        &mut self,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
        bindings: &[LocalId],
        flow: &Flow,
        source: SourceRange,
        depth: usize,
        region_owned: bool,
    ) -> Result<(Vec<CallableParameter>, Vec<EnumParameter>), Error> {
        let defaults = arguments.iter().any(|argument| argument.is_default);
        let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
            return Err(error(source, "guard call lacks a signature"));
        };
        self.fuel = self
            .fuel
            .checked_sub(arguments.len().saturating_add(bindings.len()))
            .ok_or_else(|| error(source, "guard argument budget exhausted"))?;
        if defaults
            && (bindings.len() != parameters.len()
                || arguments.len() != parameters.len()
                || bindings.iter().collect::<BTreeSet<_>>().len() != bindings.len())
        {
            return Err(error(source, "invalid guard default parameter frame"));
        }
        let report = self.take_nested_report(source)?;
        let observations = self.candidate_observations.take();
        let checked = (|| {
            let mut frame = Flow {
                borrowed_scope: self.fresh_borrowed_scope(source)?,
                live: true,
                ..Default::default()
            };
            let mut seen = BTreeSet::new();
            let mut last_default = None;
            let mut inputs = vec![];
            let mut enums = vec![];
            for argument in arguments {
                if argument.parameter >= parameters.len() || !seen.insert(argument.parameter) {
                    return Err(error(source, "invalid guard argument slot"));
                }
                if argument.is_default {
                    if last_default.is_some_and(|last| last >= argument.parameter) {
                        return Err(error(source, "guard defaults are not in declaration order"));
                    }
                    last_default = Some(argument.parameter);
                } else if last_default.is_some() {
                    return Err(error(source, "guard explicit argument follows a default"));
                }
                let input = if argument.is_default { &frame } else { flow };
                let (code, fields, variants) = if region_owned {
                    self.fuel = self
                        .fuel
                        .checked_sub(
                            input
                                .roots
                                .len()
                                .saturating_add(self.observation_locals.len()),
                        )
                        .ok_or_else(|| error(source, "guard argument scope budget exhausted"))?;
                    let private = if argument.is_default {
                        input.roots.clone()
                    } else {
                        self.observation_locals.clone()
                    };
                    let observed =
                        self.observe_region_argument(&argument.value, input, depth + 1, &private)?;
                    (
                        observed.value_callable,
                        observed.value_callables,
                        observed.value_variants,
                    )
                } else if matches!(argument.value.ty, Term::Pointer(_)) {
                    // Purity is checked here; the ordinary call on the
                    // observation snapshot checks actual provenance and may
                    // establish the contents of a conditional borrowed slot.
                    self.check_pointer_observation(&argument.value, input, depth + 1)?;
                    (None, BTreeMap::new(), BTreeMap::new())
                } else {
                    if !self.observation_value_type(&argument.value.ty, source)? {
                        return Err(error(
                            source,
                            "guard arguments need scalar or Copy code inputs; other calls need failure-path resource transfer",
                        ));
                    }
                    (
                        self.observe_guard_result(&argument.value, input, depth + 1)?,
                        BTreeMap::new(),
                        BTreeMap::new(),
                    )
                };
                record_enum_parameter(
                    &mut enums,
                    argument.parameter,
                    &variants,
                    &mut self.fuel,
                    source,
                )?;
                for (path, code) in &fields {
                    if path.is_empty() {
                        continue;
                    }
                    let mut input =
                        export_callable_parameter(argument.parameter, code, &mut self.fuel)?;
                    self.fuel = self
                        .fuel
                        .checked_sub(path.len())
                        .ok_or_else(|| error(source, "guard code field budget exhausted"))?;
                    input.fields = path.clone();
                    inputs.push(input);
                }
                if let Some(code) = &code {
                    inputs.push(export_callable_parameter(
                        argument.parameter,
                        code,
                        &mut self.fuel,
                    )?);
                }
                if defaults {
                    self.declare(
                        &mut frame,
                        bindings[argument.parameter],
                        &parameters[argument.parameter].ty,
                        source,
                    )?;
                    if let Some(code) = code {
                        self.store_callable(
                            &mut frame,
                            root(bindings[argument.parameter]),
                            code,
                            source,
                        )?;
                    }
                    let destination = Place {
                        root: root(bindings[argument.parameter]),
                        fields: vec![],
                    };
                    self.install_callable_fields(&mut frame, &destination, fields, source)?;
                    self.install_enum_domains(&mut frame, &destination, variants, source)?;
                    self.install_observation_input_borrows(
                        &mut frame,
                        bindings[argument.parameter],
                        argument.parameter,
                        &parameters[argument.parameter].ty,
                        source,
                    )?;
                }
            }
            Ok((inputs, enums))
        })();
        self.report = report;
        self.candidate_observations = observations;
        checked
    }
    fn observe_pointer_flow(
        &mut self,
        expr: &Expr,
        flow: &Flow,
        depth: usize,
    ) -> Result<(Flow, ValueOrigins), Error> {
        // Only the ordinary read walk publishes runtime cleanup records.
        // Validation is speculative and cannot grant observation permissions.
        let report = self.take_nested_report(expr.source)?;
        let observations = self.candidate_observations.take();
        let checked = self.check_pointer_observation(expr, flow, depth + 1);
        self.report = report;
        self.candidate_observations = observations;
        checked?;
        self.observe_checked_value(expr, flow, depth + 1, &BTreeSet::new(), true)
    }
    fn observe_checked_expression(
        &mut self,
        expr: &Expr,
        flow: &Flow,
        depth: usize,
    ) -> Result<Option<Callable>, Error> {
        self.observe_checked_flow(expr, flow, depth, &BTreeSet::new())
            .map(|observed| observed.value_callable)
    }
    fn observe_checked_flow(
        &mut self,
        expr: &Expr,
        flow: &Flow,
        depth: usize,
        private: &BTreeSet<PlaceId>,
    ) -> Result<Flow, Error> {
        self.observe_checked_value(expr, flow, depth, private, false)
            .map(|(flow, _)| flow)
    }
    fn observe_checked_value(
        &mut self,
        expr: &Expr,
        flow: &Flow,
        depth: usize,
        private: &BTreeSet<PlaceId>,
        retain_origins: bool,
    ) -> Result<(Flow, ValueOrigins), Error> {
        self.fuel = self
            .fuel
            .checked_sub(flow.types.len().saturating_add(1))
            .ok_or_else(|| error(expr.source, "observation scope budget exhausted"))?;
        let mut observed = flow.clone();
        let mut origins = BTreeMap::new();
        self.in_temporary_scope(
            expr.source,
            TemporaryBoundary::FullExpression,
            &mut observed,
            |this, observed| {
                if retain_origins {
                    origins = this.evaluate_value_origins(expr, observed, depth + 1)?;
                    Ok(())
                } else {
                    this.expr(expr, observed, depth + 1)
                }
            },
        )?;
        let mut comparison_exclusions = private.clone();
        for root in observed
            .types
            .keys()
            .filter(|root| !flow.types.contains_key(root))
        {
            self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or_else(|| error(expr.source, "observation origin budget exhausted"))?;
            if matches!(
                self.report.borrowed_roots.get(root),
                Some(BorrowedRoot::Indirect { .. } | BorrowedRoot::GuardedIndirect { .. })
            ) {
                comparison_exclusions.insert(*root);
            }
        }
        let pointers_preserved = origins::observation_preserves_pointers(
            flow,
            &observed,
            &self.report,
            &mut self.fuel,
            expr.source,
        )?;
        if !observed.live
            || observed
                .state
                .excluding_roots(&comparison_exclusions, &mut self.fuel)
                .map_err(|e| error(expr.source, format!("observation state: {e:?}")))?
                != flow
                    .state
                    .excluding_roots(&comparison_exclusions, &mut self.fuel)
                    .map_err(|e| error(expr.source, format!("observation state: {e:?}")))?
            || observed.roots != flow.roots
            || observed.order != flow.order
            || !pointers_preserved
            || observed.temporaries != flow.temporaries
        {
            return Err(error(
                expr.source,
                "guard expression changes caller resources and needs failure-path resource transfer",
            ));
        }
        Ok((observed, origins))
    }
    fn observe_guard_result(
        &mut self,
        expr: &Expr,
        flow: &Flow,
        depth: usize,
    ) -> Result<Option<Callable>, Error> {
        if self.fuel == 0 || depth > 128 {
            return Err(error(expr.source, "guard observation budget exhausted"));
        }
        self.fuel -= 1;
        let source = expr.source;
        if let K::Block {
            statements,
            result: Some(result),
        } = &expr.kind
        {
            if statements.is_empty() && matches!(expr.ty, Term::Pointer(_)) {
                return self.observe_guard_result(result, flow, depth + 1);
            }
        }
        // A compound condition is one full expression. Discover owned
        // observations before decomposing boolean operators: checking each
        // projection independently would retire its temporary before later
        // operands execute. Closure bodies belong to separate invocations.
        let mut owned_observation = false;
        if self.package.is_some() && !matches!(expr.kind, K::Block { .. }) {
            nir::visit::expression_pruned(expr, &mut self.fuel, |node, _| {
                if owned_observation || matches!(node.kind, K::Closure { .. }) {
                    return Ok(false);
                }
                owned_observation = matches!(node.kind, K::Tuple(_) | K::AnonymousRecord { .. } | K::Record { .. } | K::Variant { .. })
                    || matches!(&node.kind, K::Call { arguments, .. }
                    if arguments.iter().any(|argument| matches!(argument.value.ty,
                        Term::Nominal { .. } | Term::Record(_) | Term::Tuple(_) | Term::NormalForm { .. })))
                    || matches!(node.kind, K::Field { .. } | K::AnonymousField { .. })
                        && expression_place(node).is_none()
                    || matches!(node.kind, K::NewtypeConvert(_))
                    || matches!(&node.kind, K::EnumName(value)
                        if expression_place(value).is_none());
                Ok(!owned_observation)
            }).map_err(|reason| error(source, reason))?;
        }
        let pointer_match =
            matches!(expr.kind, K::Match { .. }) && matches!(expr.ty, Term::Pointer(_));
        if (matches!(expr.kind, K::Block { .. }) || pointer_match || owned_observation)
            && self.package.is_some()
        {
            let locals =
                super::observation_region::check(expr, self.package.unwrap(), &mut self.fuel)
                    .map_err(|reason| error(source, reason))?;
            if locals
                .iter()
                .any(|local| flow.roots.contains(&root(*local)))
            {
                return Err(error(source, "observation region reuses a caller binding"));
            }
            // A pointer result carries no owned cleanup obligation. Its
            // observation snapshot still checks resource preservation; ordinary
            // argument evaluation checks provenance and block-local escapes.
            if !matches!(expr.ty, Term::Pointer(_))
                && !self.observation_value_type(&expr.ty, source)?
            {
                return Err(error(
                    source,
                    "observation region result needs failure-path resource transfer",
                ));
            }
            let previous = std::mem::replace(&mut self.observation_region, true);
            let previous_locals = std::mem::replace(
                &mut self.observation_locals,
                locals.into_iter().map(root).collect(),
            );
            let result = self.observe_checked_expression(expr, flow, depth + 1);
            self.observation_locals = previous_locals;
            self.observation_region = previous;
            return result;
        }
        let mut result_code = None;
        if let Some(message) = nir::program::pointer_operation_error_in(
            expr,
            self.package.and_then(|package| package.target),
        ) {
            return Err(error(source, message));
        }
        match &expr.kind {
            K::UnsafeScope(body) => {
                if body.ty != expr.ty {
                    return Err(error(source, "unsafe observation result type mismatch"));
                }
                result_code = self.observe_guard_result(body, flow, depth + 1)?;
            }
            K::Intrinsic { inputs, .. } => {
                nir::intrinsics::checked(expr).map_err(|reason| error(source, reason))?;
                for input in inputs {
                    self.observe_guard(input, flow, depth + 1)?;
                }
            }
            K::AssociatedValue { .. } => self.observe_associated_value(&expr.ty, source)?,
            K::ComptimeValue(_) | K::Literal(_) | K::Unit => (),
            K::RequirePredicate(condition)
            | K::EnsurePredicate(condition)
            | K::CheckPredicate(condition)
                if constant_bool(condition, 0) == Some(true) =>
            {
                self.observe_guard(condition, flow, depth + 1)?
            }
            K::Function(_) | K::FunctionInstance { .. }
                if self.observation_value_type(&expr.ty, source)? =>
            {
                result_code = callable_source(expr, flow, &mut self.fuel)?;
            }
            K::Closure { captures, .. }
                if captures.is_empty() && self.observation_value_type(&expr.ty, source)? =>
            {
                result_code = callable_source(expr, flow, &mut self.fuel)?;
            }
            K::Local(_) | K::AnonymousField { .. } | K::Field { .. } => {
                let place = expression_place(expr).ok_or_else(|| {
                    error(
                        source,
                        "guard temporary projection needs observation semantics",
                    )
                })?;
                flow.state.require_present(&place).map_err(|_| {
                    error(source, format!("guard observes unavailable value {place}"))
                })?;
                if (self.delivery)(&expr.ty).map_err(|e| error(source, e))? != Delivery::Copy {
                    return Err(error(
                        source,
                        "guard consumption requires tentative binding and failure-path transfer",
                    ));
                }
                if matches!(expr.ty, Term::Function { .. } | Term::UnsafeFunction(_)) {
                    result_code = callable_source(expr, flow, &mut self.fuel)?;
                }
            }
            K::AddressOf(_) => {
                for target in self.pointer_target(expr, flow, depth + 1)? {
                    flow.state.require_place(&target).map_err(|reason| {
                        error(
                            source,
                            format!("guard address target unavailable: {reason:?}"),
                        )
                    })?;
                }
            }
            K::PointerAnonymousField { pointer, .. } | K::PointerField { pointer, .. } => {
                self.observe_guard(pointer, flow, depth + 1)?;
                for target in self.observed_pointer_targets(expr, flow, depth + 1)? {
                    flow.state.require_place(&target).map_err(|e| {
                        error(source, format!("guard field target unavailable: {e:?}"))
                    })?;
                }
            }
            K::ReadPointer(_) => {
                return self
                    .observe_pointer_flow(expr, flow, depth + 1)
                    .map(|(observed, _)| observed.value_callable);
            }
            K::EnumName(value) => {
                if let Some(place) = expression_place(value) {
                    flow.state.require_present(&place).map_err(|reason| {
                        error(
                            source,
                            format!("enum name requires an available value: {reason:?}"),
                        )
                    })?;
                } else {
                    self.observe_guard(value, flow, depth + 1)?;
                }
            }
            K::NewtypeConvert(value) => {
                if !self.observation_value_type(&expr.ty, source)? {
                    return Err(error(
                        source,
                        "guard conversion result needs failure-path resource transfer",
                    ));
                }
                // Validate observation effects before ordinary delivery on a
                // snapshot. Only that delivery records copies and temporaries.
                self.check_guard_observation(value, flow, depth + 1)?;
                result_code = self.observe_checked_expression(expr, flow, depth + 1)?;
            }
            K::Not(value)
            | K::IntegerNegate(value)
            | K::FloatNegate(value)
            | K::BitConvert(value)
            | K::Snapshot(value)
            | K::TakeBits(value)
            | K::SymbolName(value)
            | K::BitSlice { value, .. } => self.observe_guard(value, flow, depth + 1)?,
            K::BitWindow { value, start, end } => {
                self.observe_guard(value, flow, depth + 1)?;
                self.observe_guard(start, flow, depth + 1)?;
                self.observe_guard(end, flow, depth + 1)?;
            }
            K::TemplateEqual(left, right) => {
                self.observe_template_identity(left, flow)?;
                self.observe_template_identity(right, flow)?;
            }
            K::IntegerBinary { left, right, .. }
            | K::FloatBinary { left, right, .. }
            | K::ScalarEqual(left, right)
            | K::ScalarLess(left, right)
            | K::ScalarLessEqual(left, right)
            | K::ScalarGreater(left, right)
            | K::ScalarGreaterEqual(left, right)
            | K::BitEqual(left, right)
            | K::BitConcat(left, right)
            | K::BitIndex(left, right) => {
                self.observe_guard(left, flow, depth + 1)?;
                self.observe_guard(right, flow, depth + 1)?;
            }
            K::And(left, right) | K::Or(left, right) => {
                self.observe_guard(left, flow, depth + 1)?;
                let skip = constant_bool(left, 0).map(|value| {
                    if matches!(expr.kind, K::And(..)) {
                        !value
                    } else {
                        value
                    }
                });
                if skip != Some(true) {
                    self.observe_guard(right, flow, depth + 1)?;
                }
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.observe_guard(condition, flow, depth + 1)?;
                let known = constant_bool(condition, 0);
                if known != Some(false) {
                    result_code = self.observe_guard_result(then_branch, flow, depth + 1)?;
                }
                if known != Some(true) {
                    if let Some(other) = else_branch {
                        let other = self.observe_guard_result(other, flow, depth + 1)?;
                        result_code = if known == Some(false) {
                            other
                        } else {
                            match (result_code, other) {
                                (Some(left), Some(right)) => merge_callable_provenance(
                                    &left,
                                    &right,
                                    &mut self.fuel,
                                    source,
                                    depth + 1,
                                )?,
                                _ => None,
                            }
                        };
                    }
                }
            }
            K::Match { value, arms } => {
                if !self.observation_value_type(&value.ty, source)?
                    || !self.observation_value_type(&expr.ty, source)?
                {
                    return Err(error(
                        source,
                        "observed match needs scalar or Copy code values; other matches need failure-path resource transfer",
                    ));
                }
                // Validate all reachable arm computations in their provisional
                // binding environments. The ordinary match traversal below owns
                // the executable records, including failed-candidate cleanup.
                let report = self.take_nested_report(source)?;
                let observations = self.candidate_observations.take();
                let checked = (|| -> Result<(), Error> {
                    let code = self.observe_guard_result(value, flow, depth + 1)?;
                    self.fuel = self
                        .fuel
                        .checked_sub(flow.types.len().saturating_add(1))
                        .ok_or_else(|| error(source, "observed match scope budget exhausted"))?;
                    let mut base = flow.clone();
                    base.value_callable = None;
                    base.value_callables.clear();
                    base.value_variants.clear();
                    if let Some(code) = code {
                        base.value_callables.insert(vec![], code);
                    }
                    for arm in arms {
                        let (pattern, mut guards) = outer_guards(&arm.pattern);
                        guards.extend(arm.guard.as_ref());
                        let known = known_pattern_with_domains(
                            pattern,
                            value,
                            &base.value_variants,
                            &mut self.fuel,
                            source,
                        )?;
                        if known == Some(false) {
                            continue;
                        }
                        self.fuel = self
                            .fuel
                            .checked_sub(base.types.len().saturating_add(1))
                            .ok_or_else(|| error(source, "observed match arm budget exhausted"))?;
                        let mut branch = base.clone();
                        self.bind_pattern(
                            pattern,
                            &value.ty,
                            &BTreeMap::new(),
                            &mut branch,
                            arm.body.source,
                            &mut vec![],
                            depth + 1,
                        )?;
                        let mut guard_known = Some(true);
                        for guard in guards {
                            self.observe_guard(guard, &branch, depth + 1)?;
                            match constant_bool(guard, 0) {
                                Some(false) => {
                                    guard_known = Some(false);
                                    break;
                                }
                                None => guard_known = None,
                                _ => {}
                            }
                        }
                        if guard_known != Some(false) {
                            self.observe_guard(&arm.body, &branch, depth + 1)?;
                        }
                        if known == Some(true) && guard_known == Some(true) {
                            break;
                        }
                    }
                    Ok(())
                })();
                self.report = report;
                self.candidate_observations = observations;
                checked?;
                result_code = self.observe_checked_expression(expr, flow, depth + 1)?;
            }
            K::Block { statements, result } => {
                self.fuel = self
                    .fuel
                    .checked_sub(flow.types.len().saturating_add(1))
                    .ok_or_else(|| error(source, "candidate block scope budget exhausted"))?;
                let mut scope = flow.clone();
                let scalar = |ty: &Term| {
                    matches!(ty, Term::Primitive(_)) && nir::program::intrinsic_pointer_value(ty)
                        || matches!(ty, Term::Primitive(nir::Primitive::Void))
                };
                for statement in statements {
                    let statement = match &statement.kind {
                        K::Discard(value) => value.as_ref(),
                        _ => statement,
                    };
                    match &statement.kind {
                        K::Let { binding, value }
                            if self.observation_value_type(&value.ty, statement.source)? =>
                        {
                            let code = self.observe_guard_result(value, &scope, depth + 1)?;
                            self.declare(&mut scope, *binding, &value.ty, statement.source)?;
                            if let Some(code) = code {
                                scope.callables.insert(root(*binding), code);
                            }
                        }
                        K::LetPattern {
                            value,
                            otherwise: None,
                            ..
                        } if self.observation_value_type(&value.ty, statement.source)? => {
                            // Typed let without else carries irrefutability.
                            // Pattern computations use the same observation
                            // checks as match; emit its binding plan only once.
                            self.check_guard_observation(value, &scope, depth + 1)?;
                            self.expr(statement, &mut scope, depth + 1)?;
                        }
                        K::Assign { place, value }
                            if place.fields.is_empty()
                                && self.observation_value_type(&value.ty, statement.source)?
                                && scope.roots.contains(&root(place.root))
                                && !flow.roots.contains(&root(place.root)) =>
                        {
                            self.check_guard_observation(value, &scope, depth + 1)?;
                            self.expr(statement, &mut scope, depth + 1)?;
                        }
                        K::RequirePredicate(condition)
                        | K::EnsurePredicate(condition)
                        | K::CheckPredicate(condition)
                            if constant_bool(condition, 0) == Some(true) =>
                        {
                            self.observe_guard(condition, &scope, depth + 1)?
                        }
                        _ if scalar(&statement.ty) || matches!(statement.kind, K::Unit) => {
                            self.observe_guard(statement, &scope, depth + 1)?
                        }
                        _ => {
                            return Err(error(
                                statement.source,
                                "candidate block changes external resources or owns a non-scalar; failure-path resource transfer is required",
                            ));
                        }
                    }
                }
                if let Some(result) = result {
                    result_code = self.observe_guard_result(result, &scope, depth + 1)?;
                }
                let locals: Vec<_> = scope
                    .order
                    .iter()
                    .rev()
                    .filter(|id| !flow.roots.contains(id))
                    .copied()
                    .collect();
                for local in locals {
                    self.retire(&mut scope, local, source)?;
                }
            }
            K::Call { .. } => {
                result_code = self.observe_guard_call(expr, flow, depth + 1, false)?;
            }
            _ => {
                return Err(error(
                    source,
                    "effectful guard needs failure-path resource transfer",
                ));
            }
        }
        Ok(result_code)
    }
    fn observe_region_argument(
        &mut self,
        expr: &Expr,
        flow: &Flow,
        depth: usize,
        private: &BTreeSet<PlaceId>,
    ) -> Result<Flow, Error> {
        if depth > 128 {
            return Err(error(expr.source, "guard argument depth exhausted"));
        }
        if !self.observation_value_type(&expr.ty, expr.source)? {
            if !matches!(
                expr.ty,
                Term::Pointer(_)
                    | Term::Nominal { .. }
                    | Term::Record(_)
                    | Term::Tuple(_)
                    | Term::NormalForm { .. }
                    | Term::ErrorResult { .. }
            ) {
                return Err(error(
                    expr.source,
                    "guard arguments need scalar, Copy code, or private owned inputs",
                ));
            }
            if !super::memory::copy_readable_representation(self.package, &expr.ty, &mut self.fuel)
                .map_err(|reason| error(expr.source, reason))?
            {
                return Err(error(
                    expr.source,
                    "guard argument lacks a readable owned representation",
                ));
            }
            self.check_observation_cleanup(&expr.ty, expr.source)?;
        }
        let package = self
            .package
            .ok_or_else(|| error(expr.source, "guard arguments require package evidence"))?;
        let locals = super::observation_region::check(expr, package, &mut self.fuel)
            .map_err(|reason| error(expr.source, reason))?;
        if locals
            .iter()
            .any(|local| flow.roots.contains(&root(*local)))
        {
            return Err(error(expr.source, "guard arguments reuse a caller binding"));
        }
        self.fuel = self
            .fuel
            .checked_sub(private.len().saturating_add(locals.len()))
            .ok_or_else(|| error(expr.source, "guard argument scope budget exhausted"))?;
        let mut nested = private.clone();
        nested.extend(locals.into_iter().map(root));
        let previous_locals = std::mem::replace(&mut self.observation_locals, nested);
        let previous = std::mem::replace(&mut self.observation_region, true);
        let result = self.observe_checked_flow(expr, flow, depth + 1, private);
        self.observation_region = previous;
        self.observation_locals = previous_locals;
        result
    }
    fn observe_guard_call(
        &mut self,
        expr: &Expr,
        flow: &Flow,
        depth: usize,
        region_owned: bool,
    ) -> Result<Option<Callable>, Error> {
        let source = expr.source;
        if depth > 128 || self.fuel == 0 {
            return Err(error(source, "guard call observation budget exhausted"));
        }
        self.fuel -= 1;
        let K::Call {
            callee,
            arguments,
            parameter_bindings,
        } = &expr.kind
        else {
            return Err(error(source, "guard call checker requires a call"));
        };
        // A checked region may transfer its private owners. Defaults observe
        // only the prepared frame; the outer observation still protects every
        // caller resource. Owned results require captured cleanup evidence.
        if !self.observation_value_type(&expr.ty, source)? && !matches!(expr.ty, Term::Pointer(_)) {
            if !region_owned
                || !matches!(
                    expr.ty,
                    Term::Nominal { .. }
                        | Term::Record(_)
                        | Term::Tuple(_)
                        | Term::NormalForm { .. }
                        | Term::ErrorResult { .. }
                )
            {
                return Err(error(
                    source,
                    "guard call result needs failure-path resource transfer",
                ));
            }
            if !super::memory::copy_readable_representation(self.package, &expr.ty, &mut self.fuel)
                .map_err(|reason| error(source, reason))?
            {
                return Err(error(
                    source,
                    "guard result lacks a readable owned representation",
                ));
            }
            self.check_observation_cleanup(&expr.ty, source)?;
        }
        let (inputs, enums) = self.check_guard_arguments(
            callee,
            arguments,
            parameter_bindings,
            flow,
            source,
            depth + 1,
            region_owned,
        )?;
        let known;
        let selected;
        let targets: Vec<&Expr> =
            if matches!(callee.kind, K::Function(_) | K::FunctionInstance { .. }) {
                vec![callee]
            } else if matches!(callee.kind, K::TraitMethod { .. }) {
                let package = self
                    .package
                    .ok_or_else(|| error(source, "guard method observation requires a package"))?;
                // Selection retains the interface checks: method_callee refuses
                // contracts that have not yet been composed into its footprint.
                selected = self
                    .method_callees
                    .select(package, callee, &mut self.fuel)
                    .map_err(|reason| error(source, reason))?;
                vec![&selected]
            } else {
                if !self.observation_value_type(&callee.ty, source)? {
                    return Err(error(
                        source,
                        "guard callee needs Copy code observation semantics",
                    ));
                }
                known = self
                    .check_guard_observation(callee, flow, depth + 1)?
                    .ok_or_else(|| {
                        error(
                            source,
                            "unknown guard callee needs failure-path resource transfer",
                        )
                    })?;
                if !known.choices.is_empty() || !known.captures.is_empty() {
                    return Err(error(
                        source,
                        "guard captured code needs failure-path resource transfer",
                    ));
                }
                self.fuel = self
                    .fuel
                    .checked_sub(known.alternatives.len() + 1)
                    .ok_or_else(|| error(source, "guard candidate budget exhausted"))?;
                std::iter::once(&known.definition)
                    .chain(&known.alternatives)
                    .collect()
            };
        let mut observes_pointers = matches!(expr.ty, Term::Pointer(_));
        for argument in arguments {
            observes_pointers |= !super::memory::pointer_input_layout(
                self.package,
                &argument.value.ty,
                &mut self.fuel,
            )
            .map_err(|reason| error(source, reason))?
            .is_empty();
        }
        for target in targets {
            if !matches!(target.kind, K::Function(_) | K::FunctionInstance { .. }) {
                return Err(error(
                    source,
                    "guard code target needs an established observation contract",
                ));
            }
            let mut checked_summary = None;
            if observes_pointers {
                let summary = self
                    .memory_summary_with_enum_context(target, &inputs, &[], &enums, source, true)
                    .map_err(|reason| {
                        error(
                            source,
                            format!(
                                "guard pointer observation requires a memory summary: {reason}"
                            ),
                        )
                    })?;
                if summary.accesses.iter().any(|access| access.write) || summary.may_run_cleanup {
                    return Err(error(
                        source,
                        "guard pointer observation cannot write caller memory or run unverified cleanup",
                    ));
                }
                checked_summary = Some(summary);
            }
            let mut may_reject = self
                .call_may_reject(target)
                .map_err(|reason| error(source, reason))?;
            if may_reject {
                // Contextual code identities can refine unknown explicit
                // calls. Memory summaries retain unverified retirement.
                // Reuse only within this observation, with the same code,
                // argument frame, tag premises and cleanup world. This is not
                // a cache across mutations or candidate transactions.
                if checked_summary.is_none() {
                    checked_summary = self
                        .memory_summary_with_enum_context(
                            target,
                            &inputs,
                            &[],
                            &enums,
                            source,
                            true,
                        )
                        .ok();
                }
                if let Some(summary) = checked_summary {
                    may_reject = summary.may_reject
                        || summary.may_run_cleanup
                        || summary.accesses.iter().any(|access| access.write);
                }
            }
            if may_reject {
                return Err(error(
                    source,
                    "rejecting guard call needs failure-path resource transfer",
                ));
            }
        }
        // Keep ordinary call contracts and graph acquisition. The
        // observation precheck above forbids consuming caller state;
        // returned code metadata belongs to this observed computation.
        if region_owned {
            self.fuel = self
                .fuel
                .checked_sub(self.observation_locals.len())
                .ok_or_else(|| error(source, "observation scope budget exhausted"))?;
            let private = self.observation_locals.clone();
            self.observe_checked_flow(expr, flow, depth + 1, &private)
                .map(|observed| observed.value_callable)
        } else {
            self.observe_checked_expression(expr, flow, depth + 1)
        }
    }
    fn match_arms(
        &mut self,
        value: &Expr,
        arms: &[nir::program::MatchArm],
        flow: &mut Flow,
        source: SourceRange,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        let origins = self.evaluate_value_origins(value, flow, depth + 1)?;
        let pointer_guards = self.copy_value_pointer_guards(&flow.value_pointer_guards, source)?;
        if !flow.live {
            return Ok(BTreeMap::new());
        }
        let base = flow.clone();
        let mut continuation: Option<Flow> = None;
        let mut result_origins: Option<ValueOrigins> = None;
        let mut report = MatchReport {
            source,
            arms: vec![],
        };
        for arm in arms {
            let (pattern, mut guards) = outer_guards(&arm.pattern);
            guards.extend(arm.guard.as_ref());
            let known = known_pattern_with_domains(
                pattern,
                value,
                &base.value_variants,
                &mut self.fuel,
                source,
            )?;
            if known == Some(false) {
                continue;
            }
            let mut branch = base.clone();
            let outer_report = self.take_nested_report(arm.body.source)?;
            // These bindings are provisional aliases for read-only guard
            // observation. Their cleanup/decomposition records commit only
            // along the successful candidate path.
            let outer_observations = self.candidate_observations.replace(Report::default());
            self.bind_pattern_with_pointer_guards(
                pattern,
                &value.ty,
                &origins,
                &pointer_guards,
                &mut branch,
                arm.body.source,
                &mut vec![],
                depth + 1,
            )?;
            let mut guard_known = Some(true);
            for guard in &guards {
                self.observe_candidate(guard, &branch, depth + 1)?;
                match constant_bool(guard, 0) {
                    Some(false) => {
                        guard_known = Some(false);
                        break;
                    }
                    None => guard_known = None,
                    _ => (),
                }
            }
            let prepared = std::mem::replace(&mut self.report, outer_report);
            // Candidate computations ran even when its guard is false. Keep
            // their calls/temporary cleanup separately from successful binds.
            let observed =
                std::mem::replace(&mut self.candidate_observations, outer_observations).unwrap();
            self.report.append(observed);
            if guard_known == Some(false) {
                continue;
            }
            self.report.append(prepared);
            let arm_origins = self.evaluate_value_origins(&arm.body, &mut branch, depth + 1)?;
            if branch.live {
                let locals: Vec<_> = branch
                    .order
                    .iter()
                    .rev()
                    .filter(|id| !base.roots.contains(id))
                    .copied()
                    .collect();
                self.check_scope_origins(&arm_origins, &locals, arm.body.source)?;
                for local in locals {
                    self.retire(&mut branch, local, arm.body.source)?;
                }
                report.arms.push(MatchArmReport {
                    pattern: pattern.clone(),
                    guards: guards.iter().map(|g| (*g).clone()).collect(),
                    exit: Some(branch.state.clone()),
                });
                let result_guards = if let Some(merged) = &result_origins {
                    let previous = continuation.as_ref().expect("live result continuation");
                    self.join_result_pointer_guards(
                        &previous.value_pointer_guards,
                        merged,
                        &branch.value_pointer_guards,
                        &arm_origins,
                        source,
                        Some((
                            &arm.body.ty,
                            &previous.value_variants,
                            &branch.value_variants,
                        )),
                    )?
                } else {
                    self.copy_value_pointer_guards(&branch.value_pointer_guards, source)?
                };
                if let Some(merged) = &mut result_origins {
                    let previous = continuation.as_ref().expect("live result continuation");
                    self.join_value_origins(
                        &arm.body.ty,
                        merged,
                        &previous.value_variants,
                        &arm_origins,
                        &branch.value_variants,
                        source,
                    )?;
                } else {
                    result_origins = Some(arm_origins);
                }
                if let Some(merged) = &mut continuation {
                    self.join_flow(merged, &branch, source)?;
                } else {
                    continuation = Some(branch);
                }
                continuation
                    .as_mut()
                    .expect("live continuation")
                    .value_pointer_guards = result_guards;
            } else {
                report.arms.push(MatchArmReport {
                    pattern: pattern.clone(),
                    guards: guards.iter().map(|g| (*g).clone()).collect(),
                    exit: None,
                });
            }
            if known == Some(true) && guard_known == Some(true) {
                break;
            }
        }
        self.report.matches.push(report);
        if let Some(continuation) = continuation {
            *flow = continuation;
        } else {
            flow.live = false;
        }
        Ok(result_origins.unwrap_or_default())
    }
    fn decomposition(
        &mut self,
        ty: &Term,
        source: SourceRange,
        fields: &[usize],
        variant: Option<usize>,
        empty: bool,
    ) -> Result<(), Error> {
        self.report.decompositions.push(Decomposition {
            source,
            ty: ty.clone(),
            fields: fields.to_vec(),
            variant,
        });
        if empty {
            self.report.temporaries.push(TemporaryCleanup {
                scope: self.temporary_scope,
                source,
                ty: ty.clone(),
                fields: fields.to_vec(),
            });
        }
        Ok(())
    }
    fn bind_pattern(
        &mut self,
        pattern: &Pattern,
        ty: &Term,
        origins: &ValueOrigins,
        flow: &mut Flow,
        source: SourceRange,
        path: &mut Vec<usize>,
        depth: usize,
    ) -> Result<(), Error> {
        self.bind_pattern_with_pointer_guards(
            pattern,
            ty,
            origins,
            &BTreeMap::new(),
            flow,
            source,
            path,
            depth,
        )
    }
    fn bind_pattern_with_pointer_guards(
        &mut self,
        pattern: &Pattern,
        ty: &Term,
        origins: &ValueOrigins,
        pointer_guards: &ValuePointerGuards,
        flow: &mut Flow,
        source: SourceRange,
        path: &mut Vec<usize>,
        depth: usize,
    ) -> Result<(), Error> {
        let subject_conditions = ValueConditions {
            domains: copy_enum_domains(&flow.value_variants, &mut self.fuel, source)?,
            pointers: self.copy_value_pointer_guards(pointer_guards, source)?,
        };
        // The delivered subject precedes bindings acquired from it. This
        // anchor matters on return from an arm: bindings retire before the
        // retained subject, just as on ordinary match completion.
        self.acquire_cleanup(ty, source)?;
        let identity = self.sequence(source)?;
        let locals_before = flow.order.clone();
        // Copy-only patterns retain the subject and use the compact observation
        // path. Affine cases join independently checked successful transfers.
        let candidates = if !patterns::has_alternatives(pattern, &mut self.fuel, source, depth)?
            || self.observation_pattern(pattern, ty, source, depth, true)?
        {
            vec![std::borrow::Cow::Borrowed(pattern)]
        } else {
            patterns::alternatives(pattern, &mut self.fuel, source, depth)?
                .into_iter()
                .map(std::borrow::Cow::Owned)
                .collect()
        };
        let conditional = candidates.len() > 1;
        if conditional {
            patterns::binding_map(pattern, &[], &mut self.fuel, source)?;
        }
        let root = PlaceId {
            owner: u64::MAX,
            local: 0,
        };
        let mut merged: Option<Flow> = None;
        let mut layouts: Vec<ResidualLayout> = vec![];
        let mut orders: Vec<Vec<PlaceId>> = vec![];
        for (index, candidate) in candidates.iter().enumerate() {
            let mut preparation = PatternPreparation {
                origins,
                pointer_guards: self.copy_value_pointer_guards(pointer_guards, source)?,
                selected_pointer_variants: vec![],
                domains: flow.value_variants.clone(),
                callables: flow.value_callables.clone(),
                transfers: vec![],
                moved_bindings: vec![],
                variants: vec![],
                copy_bindings: false,
                binding_remaps: vec![],
            };
            let mut candidate_flow = if index + 1 == candidates.len() {
                std::mem::take(flow)
            } else {
                self.fuel = self
                    .fuel
                    .checked_sub(flow.types.len().saturating_add(1))
                    .ok_or_else(|| error(source, "pattern candidate flow budget exhausted"))?;
                flow.clone()
            };
            let shape = self.bind_components(
                candidate,
                ty,
                &mut candidate_flow,
                source,
                path,
                &mut preparation,
                depth,
            )?;
            let mut state = State::default();
            state
                .declare(root, &shape, true, &mut self.fuel)
                .map_err(|e| error(source, format!("pattern subject shape: {e:?}")))?;
            for fields in preparation.transfers {
                state
                    .take(&Place { root, fields })
                    .map_err(|e| error(source, format!("pattern subject transfer: {e:?}")))?;
            }
            if conditional {
                for (binding, fields) in preparation.moved_bindings {
                    self.report
                        .pattern_transfers
                        .push((source, binding, fields));
                }
            }
            let order = candidate_flow.order[locals_before.len()..].to_vec();
            if !orders.contains(&order) {
                orders.push(order);
            }
            if let Some(previous) = &merged {
                let mut expected = previous.order[locals_before.len()..].to_vec();
                let mut actual = candidate_flow.order[locals_before.len()..].to_vec();
                expected.sort();
                actual.sort();
                if expected != actual {
                    return Err(error(source, "pattern candidates declare different owners"));
                }
                // Canonicalize analysis bookkeeping only. Observable cleanup
                // uses the successful candidate's separately retained order.
                candidate_flow.order = previous.order.clone();
            }
            if let Some(previous) = &mut merged {
                self.join_flow(previous, &candidate_flow, source)?;
            } else {
                merged = Some(candidate_flow);
            }
            preparation.variants.sort_by(|a, b| a.fields.cmp(&b.fields));
            if let Some(previous) = layouts
                .iter_mut()
                .find(|layout| layout.variants == preparation.variants)
            {
                previous.remaining =
                    previous
                        .remaining
                        .join(&state, &mut self.fuel)
                        .map_err(|e| {
                            error(source, format!("pattern residual ownership join: {e:?}"))
                        })?;
            } else {
                for previous in &layouts {
                    self.fuel = self
                        .fuel
                        .checked_sub(
                            previous
                                .variants
                                .len()
                                .saturating_mul(preparation.variants.len())
                                .saturating_add(1),
                        )
                        .ok_or_else(|| {
                            error(source, "pattern variant discrimination budget exhausted")
                        })?;
                    if !previous.variants.iter().any(|a| {
                        preparation
                            .variants
                            .iter()
                            .any(|b| a.fields == b.fields && a.index != b.index)
                    }) {
                        return Err(error(
                            source,
                            "pattern residual layouts lack disjoint variant evidence",
                        ));
                    }
                }
                layouts.push(ResidualLayout {
                    remaining: state,
                    variants: preparation.variants,
                });
            }
        }
        let Some(candidate_flow) = merged else {
            return Err(error(
                source,
                "pattern has no successful ownership candidate",
            ));
        };
        let ResidualLayout {
            remaining: state,
            variants,
        } = layouts.remove(0);
        *flow = candidate_flow;
        if orders.len() > 1 {
            let published = patterns::binding_orders(pattern, &mut self.fuel, source, depth)?;
            for order in &published {
                self.fuel = self
                    .fuel
                    .checked_sub(order.len().saturating_mul(order.len()).saturating_add(1))
                    .ok_or_else(|| error(source, "pattern cleanup permutation budget exhausted"))?;
                for (index, id) in order.iter().enumerate() {
                    flow.cleanup_before
                        .entry(self::root(*id))
                        .or_default()
                        .extend(order[index + 1..].iter().copied().map(self::root));
                }
            }
            self.report.pattern_orders.push((source, published));
        }
        if *ty == Term::Primitive(nir::Primitive::Void) {
            return Ok(());
        }
        if state
            .cleanup_tree(
                &Place {
                    root,
                    fields: vec![],
                },
                &mut self.fuel,
            )
            .map_err(|e| error(source, format!("pattern subject cleanup: {e:?}")))?
            .is_some()
        {
            flow.temporaries.insert(
                identity,
                ActiveTemporary {
                    conditions: subject_conditions,
                    alternatives: layouts,
                    origins: origins.clone(),
                    variants,
                    scope_id: self.temporary_scope_id,
                    description: TemporaryCleanup {
                        scope: self.temporary_scope,
                        source,
                        ty: ty.clone(),
                        fields: vec![],
                    },
                    remaining: state,
                    presence: Availability::Present,
                    locals_before,
                },
            );
        }
        Ok(())
    }
    /// A structural test may inspect an affine value without acquiring its
    /// fields. Keep this distinct from decomposition and from patterns which
    /// run code: a failed test must leave ownership available to the next arm.
    /// With `allow_copy_bindings`, every exported value must have Copy evidence;
    /// no field is thereby transferred out of the observed subject.
    fn observation_pattern(
        &mut self,
        pattern: &Pattern,
        ty: &Term,
        source: SourceRange,
        depth: usize,
        allow_copy_bindings: bool,
    ) -> Result<bool, Error> {
        if depth > 128 || self.fuel == 0 {
            return Err(error(source, "pattern observation budget exhausted"));
        }
        self.fuel -= 1;
        let (patterns, types) = match pattern {
            Pattern::Wild | Pattern::Literal(_) => return Ok(true),
            Pattern::Unit => return Ok(*ty == Term::Primitive(nir::Primitive::Void)),
            Pattern::Observe { inner, .. } | Pattern::Guard { pattern: inner, .. }
                if allow_copy_bindings =>
            {
                // Selecting a candidate cannot transfer any part of this
                // packet. bind_components still checks the condition with
                // observe_pattern_value, including its failure-path effects.
                if !self.observation_value_type(ty, source)? {
                    return Ok(false);
                }
                return self.observation_pattern(inner, ty, source, depth + 1, true);
            }
            Pattern::Binding { inner, .. } if allow_copy_bindings => {
                if (self.delivery)(ty).map_err(|e| error(source, e))? != Delivery::Copy {
                    return Ok(false);
                }
                return self.observation_pattern(inner, ty, source, depth + 1, true);
            }
            Pattern::Or(alternatives) => {
                for alternative in alternatives {
                    if !self.observation_pattern(
                        alternative,
                        ty,
                        source,
                        depth + 1,
                        allow_copy_bindings,
                    )? {
                        return Ok(false);
                    }
                }
                return Ok(true);
            }
            Pattern::Remap { pattern, bindings } if allow_copy_bindings || bindings.is_empty() => {
                if allow_copy_bindings {
                    patterns::binding_map(pattern, bindings, &mut self.fuel, source)?;
                }
                return self.observation_pattern(
                    pattern,
                    ty,
                    source,
                    depth + 1,
                    allow_copy_bindings,
                );
            }
            Pattern::ErrorResult { .. } => {
                let (_, patterns, types) =
                    super::memory::owned_variant_pattern(self.package, pattern, ty, &mut self.fuel)
                        .map_err(|reason| error(source, reason))?;
                for (pattern, ty) in patterns.iter().zip(types.iter()) {
                    if !self.observation_pattern(
                        pattern,
                        ty,
                        source,
                        depth + 1,
                        allow_copy_bindings,
                    )? {
                        return Ok(false);
                    }
                }
                return Ok(true);
            }
            Pattern::Tuple(patterns) => {
                let Some(types) = nir::normal_forms::tuple_fields(self.package, ty) else {
                    return Err(error(source, "tuple observation type mismatch"));
                };
                (patterns, types.to_vec())
            }
            Pattern::AnonymousRecord { names, fields } => {
                let Term::Record(types) = ty else {
                    return Err(error(source, "record observation type mismatch"));
                };
                if names.len() != types.len()
                    || names
                        .iter()
                        .zip(types)
                        .any(|(name, (expected, _))| name != expected)
                {
                    return Err(error(source, "record observation labels mismatch"));
                }
                (fields, types.iter().map(|(_, ty)| ty.clone()).collect())
            }
            Pattern::Record {
                declaration,
                fields,
            }
            | Pattern::Variant {
                declaration,
                fields,
                ..
            } => {
                let Term::Nominal {
                    declaration: actual,
                    arguments,
                } = ty
                else {
                    return Err(error(source, "nominal observation type mismatch"));
                };
                if actual != declaration {
                    return Err(error(source, "nominal observation declaration mismatch"));
                }
                let package = self.package.ok_or_else(|| {
                    error(source, "nominal observation requires package metadata")
                })?;
                let types = if let Pattern::Variant { index, .. } = pattern {
                    let definition = package
                        .enums
                        .iter()
                        .find(|item| item.declaration == *declaration)
                        .ok_or_else(|| {
                            error(source, "enum observation requires declaration metadata")
                        })?;
                    definition
                        .instantiate_variants(arguments)
                        .map_err(|failure| error(source, format!("enum observation: {failure}")))?
                        .get(*index)
                        .ok_or_else(|| error(source, "enum observation variant out of bounds"))?
                        .fields
                        .clone()
                } else {
                    let definition = package
                        .structs
                        .iter()
                        .find(|item| item.declaration == *declaration)
                        .ok_or_else(|| {
                            error(source, "record observation requires declaration metadata")
                        })?;
                    definition
                        .instantiate_fields(arguments)
                        .map_err(|failure| error(source, format!("record observation: {failure}")))?
                        .into_iter()
                        .map(|(_, ty)| ty)
                        .collect()
                };
                (fields, types)
            }
            // Bindings transfer values. Guard/Then run expressions and need
            // their own failure-path analysis, even when they export no names.
            _ => return Ok(false),
        };
        if patterns.len() != types.len() {
            return Err(error(source, "pattern observation field count mismatch"));
        }
        for (pattern, ty) in patterns.iter().zip(&types) {
            if !self.observation_pattern(pattern, ty, source, depth + 1, allow_copy_bindings)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn observe_pattern_result(
        &mut self,
        value: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<(Option<Callable>, ValueOrigins), Error> {
        if matches!(value.ty, Term::Pointer(_)) {
            // The borrowed roots discovered by the checked read belong to this
            // candidate. Keep them with the computed binder instead of resolving
            // its result again against the pre-observation snapshot.
            let prepared = if self.candidate_observations.is_some() {
                Some(self.take_nested_report(value.source)?)
            } else {
                None
            };
            let result = self.observe_pointer_flow(value, flow, depth + 1);
            if let Some(prepared) = prepared {
                let observed = std::mem::replace(&mut self.report, prepared);
                self.report.borrowed_roots.extend(
                    cleanup_memory::copy_roots(&observed.borrowed_roots, &mut self.fuel)
                        .map_err(|reason| error(value.source, reason))?,
                );
                self.candidate_observations
                    .as_mut()
                    .unwrap()
                    .append(observed);
            }
            let (observed, origins) = result?;
            let callable = observed.value_callable.clone();
            *flow = observed;
            return Ok((callable, origins));
        }
        let callable = self.observe_candidate(value, flow, depth + 1)?;
        Ok((callable, BTreeMap::new()))
    }

    /// Conditions have already passed candidate observation, so they cannot
    /// alter the caller's pointer slots. Keep general pointer_target conservative
    /// for callers that have not established this observation boundary.
    fn observed_pointer_targets(
        &mut self,
        value: &Expr,
        flow: &Flow,
        depth: usize,
    ) -> Result<BTreeSet<Place>, Error> {
        if depth >= 128 || self.fuel == 0 {
            return Err(error(
                value.source,
                "observed pointer target budget exhausted",
            ));
        }
        self.fuel -= 1;
        match &value.kind {
            K::PointerField { pointer, index, .. }
            | K::PointerAnonymousField { pointer, index, .. } => {
                let targets = self.observed_pointer_targets(pointer, flow, depth + 1)?;
                self.pointer_work(&targets, value.source)?;
                Ok(targets
                    .into_iter()
                    .map(|mut target| {
                        target.fields.push(*index);
                        target
                    })
                    .collect())
            }
            K::ReadPointer(pointer) => {
                let slots = self.observed_pointer_targets(pointer, flow, depth + 1)?;
                let mut targets = BTreeSet::new();
                for slot in slots {
                    flow.state.require_present(&slot).map_err(|reason| {
                        error(
                            value.source,
                            format!("observed pointer slot unavailable: {reason:?}"),
                        )
                    })?;
                    let contents = flow.pointers.get(&slot).ok_or_else(|| {
                        error(
                            value.source,
                            "observed pointer slot contents provenance is not established",
                        )
                    })?;
                    self.pointer_work(contents, value.source)?;
                    targets.extend(contents.iter().cloned());
                }
                Ok(targets)
            }
            K::If {
                condition,
                then_branch,
                else_branch: Some(other),
            } => match constant_bool(condition, 0) {
                Some(true) => self.observed_pointer_targets(then_branch, flow, depth + 1),
                Some(false) => self.observed_pointer_targets(other, flow, depth + 1),
                None => {
                    let mut targets =
                        self.observed_pointer_targets(then_branch, flow, depth + 1)?;
                    let right = self.observed_pointer_targets(other, flow, depth + 1)?;
                    self.pointer_work(&right, value.source)?;
                    targets.extend(right);
                    Ok(targets)
                }
            },
            K::Block {
                statements,
                result: Some(result),
            } if statements.is_empty() => self.observed_pointer_targets(result, flow, depth + 1),
            _ => self.pointer_target(value, flow, depth + 1),
        }
    }
    fn observe_pattern_value(
        &mut self,
        value: &Expr,
        flow: &mut Flow,
        preparation: &PatternPreparation<'_>,
        depth: usize,
    ) -> Result<(Option<Callable>, ValueOrigins), Error> {
        if preparation.binding_remaps.is_empty() {
            return self.observe_pattern_result(value, flow, depth + 1);
        }
        // Executable candidate expressions precede their enclosing Remap;
        // analysis has already published canonical binding identities.
        nir::visit::expression(value, &mut self.fuel, |_, _| Ok(()))
            .map_err(|reason| error(value.source, reason))?;
        let mut value = value.clone();
        nir::visit::expression_mut(&mut value, &mut self.fuel, |expr, fuel| {
            if let K::Local(binding) = &mut expr.kind {
                for mapping in preparation.binding_remaps.iter().rev() {
                    *fuel = fuel
                        .checked_sub(1)
                        .ok_or("pattern expression remap budget exhausted")?;
                    *binding = mapping.get(binding).copied().unwrap_or(*binding);
                }
            }
            Ok(())
        })
        .map_err(|reason| error(value.source, reason))?;
        self.observe_pattern_result(&value, flow, depth + 1)
    }
    fn bind_components(
        &mut self,
        pattern: &Pattern,
        ty: &Term,
        flow: &mut Flow,
        source: SourceRange,
        path: &mut Vec<usize>,
        preparation: &mut PatternPreparation<'_>,
        depth: usize,
    ) -> Result<Shape, Error> {
        if self.fuel == 0 || depth > 128 {
            return Err(error(source, "pattern move budget exhausted"));
        }
        self.fuel -= 1;
        match pattern {
            Pattern::Observe { binding, inner } => {
                if flow.roots.contains(&root(*binding)) {
                    return Err(error(
                        source,
                        "observation subject reuses an active binding",
                    ));
                }
                // Establish provenance just as for a subject binding, but do
                // not commit its ownership transfer: the match retains it.
                let transfers = preparation.transfers.len();
                let moved = preparation.moved_bindings.len();
                let copied = std::mem::replace(&mut preparation.copy_bindings, false);
                let shape = self.bind_components(
                    &Pattern::Binding {
                        binding: *binding,
                        inner: Box::new(Pattern::Wild),
                    },
                    ty,
                    flow,
                    source,
                    path,
                    preparation,
                    depth + 1,
                );
                preparation.transfers.truncate(transfers);
                preparation.moved_bindings.truncate(moved);
                preparation.copy_bindings = copied;
                let shape = shape?;
                preparation.copy_bindings = true;
                let checked =
                    self.bind_components(inner, ty, flow, source, path, preparation, depth + 1);
                preparation.copy_bindings = copied;
                checked?;
                for (place, targets) in &flow.pointers {
                    self.pointer_work(targets, source)?;
                    if place.root != root(*binding)
                        && targets.iter().any(|target| target.root == root(*binding))
                    {
                        return Err(error(
                            source,
                            "pattern observation subject address escapes its scope",
                        ));
                    }
                }
                let cleanups = self.report.cleanups.len();
                self.remove_local(flow, root(*binding), source)?;
                // Removing a nonowning alias is not a runtime cleanup action.
                self.report.cleanups.truncate(cleanups);
                if !copied && *ty != Term::Primitive(nir::Primitive::Void) {
                    self.report.temporaries.push(TemporaryCleanup {
                        scope: self.temporary_scope,
                        source,
                        ty: ty.clone(),
                        fields: path.clone(),
                    });
                }
                return Ok(shape);
            }

            Pattern::Guard { pattern, condition } => {
                let shape =
                    self.bind_components(pattern, ty, flow, source, path, preparation, depth + 1)?;
                self.observe_pattern_value(condition, flow, preparation, depth + 1)?;
                return Ok(shape);
            }
            Pattern::Then { first, value, next } => {
                let shape =
                    self.bind_components(first, ty, flow, source, path, preparation, depth + 1)?;
                let pointer = matches!(value.ty, Term::Pointer(_))
                    && (self.delivery)(&value.ty).map_err(|reason| error(value.source, reason))?
                        == Delivery::Copy;
                if !pointer && !self.observation_value_type(&value.ty, value.source)? {
                    return Err(error(
                        value.source,
                        "computed pattern value needs independent resource and failure-path transfer",
                    ));
                }
                let (callable, origins) =
                    self.observe_pattern_value(value, flow, preparation, depth + 1)?;
                let mut callables = BTreeMap::new();
                if let Some(callable) = callable {
                    callables.insert(vec![], callable);
                }
                // Copy observations have no residual owner. Pointer results
                // retain their targets; binders never subtract subject fields.
                let mut computed = PatternPreparation {
                    origins: &origins,
                    pointer_guards: BTreeMap::new(),
                    selected_pointer_variants: vec![],
                    domains: BTreeMap::new(),
                    callables,
                    transfers: vec![],
                    moved_bindings: vec![],
                    variants: vec![],
                    copy_bindings: true,
                    binding_remaps: preparation.binding_remaps.clone(),
                };
                self.bind_components(
                    next,
                    &value.ty,
                    flow,
                    source,
                    &mut vec![],
                    &mut computed,
                    depth + 1,
                )?;
                return Ok(shape);
            }
            Pattern::Or(alternatives) if preparation.copy_bindings => {
                let mut merged = None;
                let original_order = flow.order.len();
                for alternative in alternatives {
                    let known = pattern_from_enum_domains(
                        alternative,
                        path,
                        &preparation.domains,
                        &mut self.fuel,
                        source,
                        depth + 1,
                    )?;
                    if known == Some(false) {
                        continue;
                    }
                    self.fuel = self
                        .fuel
                        .checked_sub(flow.types.len().saturating_add(1))
                        .ok_or_else(|| {
                            error(source, "copied pattern candidate budget exhausted")
                        })?;
                    let mut candidate = flow.clone();
                    self.bind_components(
                        alternative,
                        ty,
                        &mut candidate,
                        source,
                        path,
                        preparation,
                        depth + 1,
                    )?;
                    // Newly declared bindings are all Copy, without cleanup
                    // effects or candidate-local address acquisition. Their
                    // declaration order may differ with the extraction paths.
                    candidate.order[original_order..].sort();
                    if let Some(previous) = &mut merged {
                        self.join_flow(previous, &candidate, source)?;
                    } else {
                        merged = Some(candidate);
                    }
                    if known == Some(true) {
                        break;
                    }
                }
                if let Some(merged) = merged {
                    *flow = merged;
                }
                return (self.shape)(ty).map_err(|e| error(source, e));
            }
            Pattern::Remap { pattern, bindings } => {
                let mapping = patterns::binding_map(pattern, bindings, &mut self.fuel, source)?;
                preparation.binding_remaps.push(mapping);
                let result =
                    self.bind_components(pattern, ty, flow, source, path, preparation, depth + 1);
                preparation.binding_remaps.pop();
                return result;
            }
            Pattern::Or(_) => {
                if self.observation_pattern(pattern, ty, source, depth, true)? {
                    // All exported values are independent copies. Candidate
                    // layouts may differ while the subject stays intact.
                    preparation.copy_bindings = true;
                    let result = self.bind_components(
                        pattern,
                        ty,
                        flow,
                        source,
                        path,
                        preparation,
                        depth + 1,
                    );
                    preparation.copy_bindings = false;
                    result?;
                    return self.bind_components(
                        &Pattern::Wild,
                        ty,
                        flow,
                        source,
                        path,
                        preparation,
                        depth + 1,
                    );
                }
                let (observation, transfer) =
                    patterns::common_plan(pattern, &mut self.fuel, source, depth)?;
                // Validate every candidate's structural observations before
                // using its common successful transfer. Runtime matching still
                // uses the original pattern and commits its canonical binders.
                if !self.observation_pattern(&observation, ty, source, depth, false)? {
                    return Err(error(
                        source,
                        "or-pattern observation requires candidate evaluation",
                    ));
                }
                return self.bind_components(
                    &transfer,
                    ty,
                    flow,
                    source,
                    path,
                    preparation,
                    depth + 1,
                );
            }
            Pattern::Wild | Pattern::Literal(_) => {
                if !preparation.copy_bindings && *ty != Term::Primitive(nir::Primitive::Void) {
                    self.report.temporaries.push(TemporaryCleanup {
                        scope: self.temporary_scope,
                        source,
                        ty: ty.clone(),
                        fields: path.clone(),
                    });
                }
                return (self.shape)(ty).map_err(|e| error(source, e));
            }
            Pattern::Unit if *ty == Term::Primitive(nir::Primitive::Void) => {
                if !preparation.copy_bindings {
                    preparation.transfers.push(path.clone());
                }
                return Ok(Shape::Value);
            }
            Pattern::Binding { binding, inner }
                if self.observation_pattern(inner, ty, source, depth, false)? =>
            {
                let mut binding = *binding;
                for mapping in preparation.binding_remaps.iter().rev() {
                    self.fuel = self
                        .fuel
                        .checked_sub(1)
                        .ok_or_else(|| error(source, "copied pattern remap budget exhausted"))?;
                    binding = mapping.get(&binding).copied().unwrap_or(binding);
                }
                // A field binding follows ordinary Copy/move delivery. A
                // copied field remains initialized in the residual parent,
                // including for a parent destructor which reads it. At the
                // root, the already delivered subject can transfer directly.
                let copied = (preparation.copy_bindings || !path.is_empty())
                    && (self.delivery)(ty).map_err(|e| error(source, e))? == Delivery::Copy;
                if preparation.copy_bindings && !copied {
                    return Err(error(
                        source,
                        "overlapping pattern binding requires Copy for the extracted value",
                    ));
                }
                self.declare(flow, binding, ty, source)?;
                // Distinct branches reuse physical payload positions. A
                // pointer belonging only to another branch is not evidence
                // for a scalar (or differently shaped) selected payload.
                let pointer_slots = if !preparation.selected_pointer_variants.is_empty()
                    && self.storage_inputs_are_closed(ty, source)?
                {
                    Some(
                        super::memory::pointer_input_layout(self.package, ty, &mut self.fuel)
                            .map_err(|reason| error(source, reason))?
                            .into_iter()
                            .map(|leaf| leaf.storage_fields)
                            .collect::<BTreeSet<_>>(),
                    )
                } else {
                    None
                };
                let mut origins = BTreeMap::new();
                let mut bound_guards = ValuePointerGuards::new();
                for (leaf, targets) in preparation.origins {
                    self.fuel = self
                        .fuel
                        .checked_sub(leaf.len().saturating_add(1))
                        .ok_or_else(|| error(source, "pattern origin budget exhausted"))?;
                    if leaf.starts_with(path) {
                        if pointer_slots
                            .as_ref()
                            .is_some_and(|slots| !slots.contains(&leaf[path.len()..]))
                        {
                            continue;
                        }
                        self.pointer_work(targets, source)?;
                        let mut selected = BTreeSet::new();
                        let mut selected_guards = PointerTargetGuards::new();
                        for target in targets {
                            let guards = preparation
                                .pointer_guards
                                .get(leaf)
                                .and_then(|entries| entries.get(target));
                            let mut excluded = false;
                            if let Some(guards) = guards {
                                for (guard, tag) in guards {
                                    for variant in &preparation.selected_pointer_variants {
                                        self.fuel = self
                                            .fuel
                                            .checked_sub(guard.len() + variant.fields.len() + 1)
                                            .ok_or_else(|| {
                                                error(
                                                    source,
                                                    "pattern pointer guard budget exhausted",
                                                )
                                            })?;
                                        if guard == &variant.fields && *tag != variant.index {
                                            excluded = true;
                                        }
                                    }
                                }
                            }
                            if excluded {
                                continue;
                            }
                            selected.insert(target.clone());
                            if let Some(guards) = guards {
                                let mut projected = vec![];
                                for (guard, tag) in guards {
                                    self.fuel = self.fuel.checked_sub(guard.len() + 1).ok_or_else(
                                        || {
                                            error(
                                                source,
                                                "pattern pointer guard projection budget exhausted",
                                            )
                                        },
                                    )?;
                                    if let Some(relative) = guard.strip_prefix(path.as_slice()) {
                                        projected.push((relative.to_vec(), *tag));
                                    }
                                }
                                if !projected.is_empty() {
                                    selected_guards.insert(target.clone(), projected);
                                }
                            }
                        }
                        if selected.is_empty() && !targets.is_empty() {
                            return Err(error(
                                source,
                                "selected pattern payload has no established pointer source",
                            ));
                        }
                        let relative = leaf[path.len()..].to_vec();
                        if !selected_guards.is_empty() {
                            bound_guards.insert(relative.clone(), selected_guards);
                        }
                        origins.insert(relative, selected);
                    }
                }
                self.install_value_origins(
                    flow,
                    &Place {
                        root: root(binding),
                        fields: vec![],
                    },
                    origins,
                    source,
                )?;
                for (fields, guards) in bound_guards {
                    flow.pointer_guards.insert(
                        Place {
                            root: root(binding),
                            fields,
                        },
                        guards,
                    );
                }
                let mut leaves = BTreeMap::new();
                for (leaf, callable) in &preparation.callables {
                    self.fuel = self
                        .fuel
                        .checked_sub(leaf.len().saturating_add(1))
                        .ok_or_else(|| error(source, "pattern callable budget exhausted"))?;
                    if leaf.starts_with(path) {
                        leaves.insert(leaf[path.len()..].to_vec(), callable.clone());
                    }
                }
                self.install_callable_fields(
                    flow,
                    &Place {
                        root: root(binding),
                        fields: vec![],
                    },
                    leaves,
                    source,
                )?;
                let domains = preparation
                    .domains
                    .iter()
                    .filter_map(|(leaf, tags)| {
                        leaf.strip_prefix(path.as_slice())
                            .map(|rest| (rest.to_vec(), tags.clone()))
                    })
                    .collect();
                self.install_enum_domains(
                    flow,
                    &Place {
                        root: root(binding),
                        fields: vec![],
                    },
                    domains,
                    source,
                )?;
                if let Some(callable) = preparation.callables.get(path).cloned() {
                    self.store_callable(flow, root(binding), callable, source)?;
                }
                if !copied {
                    preparation.transfers.push(path.clone());
                    preparation.moved_bindings.push((binding, path.clone()));
                }
                return (self.shape)(ty).map_err(|e| error(source, e));
            }
            Pattern::Tuple(patterns) => {
                let Some(types) = nir::normal_forms::tuple_fields(self.package, ty) else {
                    return Err(error(source, "tuple pattern ownership type mismatch"));
                };
                if !preparation.copy_bindings {
                    self.decomposition(ty, source, path, None, patterns.is_empty())?;
                }
                return self.bind_fields(patterns, types, flow, source, path, preparation, depth);
            }
            Pattern::AnonymousRecord { names, fields } => {
                let Term::Record(types) = ty else {
                    return Err(error(
                        source,
                        "anonymous record pattern ownership type mismatch",
                    ));
                };
                if names.len() != fields.len()
                    || names.len() != types.len()
                    || names
                        .iter()
                        .zip(types)
                        .any(|(name, (expected, _))| name != expected)
                {
                    return Err(error(
                        source,
                        "anonymous record pattern ownership labels mismatch",
                    ));
                }
                let types = types.iter().map(|(_, ty)| ty.clone()).collect::<Vec<_>>();
                if !preparation.copy_bindings {
                    self.decomposition(ty, source, path, None, fields.is_empty())?;
                }
                return self.bind_fields(fields, &types, flow, source, path, preparation, depth);
            }
            Pattern::Record {
                declaration,
                fields,
            } => {
                let Term::Nominal {
                    declaration: actual,
                    arguments,
                } = ty
                else {
                    return Err(error(source, "record pattern ownership type mismatch"));
                };
                if actual != declaration {
                    return Err(error(
                        source,
                        "record pattern ownership declaration mismatch",
                    ));
                }
                let definition = self
                    .package
                    .and_then(|p| p.structs.iter().find(|s| s.declaration == *declaration))
                    .ok_or_else(|| {
                        error(source, "record pattern ownership requires package metadata")
                    })?;
                let fields_types: Vec<_> = definition
                    .instantiate_fields(arguments)
                    .map_err(|e| error(source, format!("record pattern ownership: {e:?}")))?
                    .into_iter()
                    .map(|(_, ty)| ty)
                    .collect();
                if !preparation.copy_bindings {
                    self.decomposition(ty, source, path, None, fields.is_empty())?;
                }
                return self.bind_fields(
                    fields,
                    &fields_types,
                    flow,
                    source,
                    path,
                    preparation,
                    depth,
                );
            }
            Pattern::Variant { .. } | Pattern::ErrorResult { .. } => {
                let (tag, fields, types) =
                    super::memory::owned_variant_pattern(self.package, pattern, ty, &mut self.fuel)
                        .map_err(|reason| error(source, reason))?;
                let index = &tag;
                if !preparation.copy_bindings {
                    self.decomposition(ty, source, path, Some(*index), fields.is_empty())?;
                    preparation.variants.push(VariantSelection {
                        fields: path.clone(),
                        index: *index,
                    });
                }
                // A tag guard authorizes only that variant's payload evidence.
                // Restore the qualified map afterwards for sibling patterns.
                let selected = select_variant_callables(
                    &preparation.callables,
                    path,
                    *index,
                    &mut self.fuel,
                    source,
                )?;
                let selected_domains = select_variant_callables(
                    &preparation.domains,
                    path,
                    *index,
                    &mut self.fuel,
                    source,
                )?;
                let qualified_domains =
                    std::mem::replace(&mut preparation.domains, selected_domains);
                let qualified = std::mem::replace(&mut preparation.callables, selected);
                preparation
                    .selected_pointer_variants
                    .push(VariantSelection {
                        fields: path.clone(),
                        index: *index,
                    });
                let result =
                    self.bind_fields(fields, &types, flow, source, path, preparation, depth);
                preparation.selected_pointer_variants.pop();
                preparation.callables = qualified;
                preparation.domains = qualified_domains;
                return result;
            }
            Pattern::Binding { binding, inner } => {
                // The whole will be delivered once. Additional constituent
                // bindings may only copy; they must not decompose the retained
                // owner or subtract fields from its cleanup responsibility.
                let previous = std::mem::replace(&mut preparation.copy_bindings, true);
                let observed =
                    self.bind_components(inner, ty, flow, source, path, preparation, depth + 1);
                preparation.copy_bindings = previous;
                observed?;
                return self.bind_components(
                    &Pattern::Binding {
                        binding: *binding,
                        inner: Box::new(Pattern::Wild),
                    },
                    ty,
                    flow,
                    source,
                    path,
                    preparation,
                    depth + 1,
                );
            }
            _ => {
                return Err(error(
                    source,
                    "pattern requires additional transfer and failure semantics",
                ));
            }
        }
    }
    fn bind_fields(
        &mut self,
        patterns: &[Pattern],
        types: &[Term],
        flow: &mut Flow,
        source: SourceRange,
        path: &mut Vec<usize>,
        preparation: &mut PatternPreparation<'_>,
        depth: usize,
    ) -> Result<Shape, Error> {
        if patterns.len() != types.len() {
            return Err(error(source, "pattern ownership field count mismatch"));
        }
        let mut shapes = vec![];
        for (index, (pattern, ty)) in patterns.iter().zip(types).enumerate() {
            path.push(index);
            shapes.push(self.bind_components(
                pattern,
                ty,
                flow,
                source,
                path,
                preparation,
                depth + 1,
            )?);
            path.pop();
        }
        Ok(Shape::Fields(shapes))
    }
    fn retire(&mut self, flow: &mut Flow, id: PlaceId, source: SourceRange) -> Result<(), Error> {
        if let Some(action) = self.local_action(flow, id, &[], source)? {
            self.report.cleanup_sites.push(CleanupSite {
                default_origin: None,
                result_origin: None,
                interface_precondition: false,
                interface_postcondition: false,
                source,
                reason: CleanupReason::ScopeEnd,
                actions: vec![action],
            });
        }
        self.remove_local(flow, id, source)
    }
    fn remove_local(
        &mut self,
        flow: &mut Flow,
        id: PlaceId,
        source: SourceRange,
    ) -> Result<(), Error> {
        for (place, availability) in flow
            .state
            .retire(id)
            .map_err(|e| error(source, format!("cleanup: {e:?}")))?
            .into_iter()
            .rev()
        {
            self.report.cleanups.push(Cleanup {
                source,
                place,
                availability,
            });
        }
        flow.roots.remove(&id);
        flow.types.remove(&id);
        flow.cleanup_before.remove(&id);
        flow.order.retain(|root| *root != id);
        flow.callables.remove(&id);
        flow.callable_fields.retain(|place, _| place.root != id);
        flow.enum_domains.remove(&id);
        self.fuel = self
            .fuel
            .checked_sub(flow.pointer_guards.len())
            .ok_or_else(|| error(source, "retired pointer guard budget exhausted"))?;
        flow.pointer_guards.retain(|slot, _| slot.root != id);
        // Retired local identities may be reused by a later loop iteration.
        // Neither an old pointer nor an alias to its slot revives with it.
        flow.pointers.retain(|binding, _| binding.root != id);
        for targets in flow.pointers.values_mut() {
            self.pointer_work(targets, source)?;
            let before = targets.len();
            targets.retain(|target| target.root != id);
            if targets.len() != before {
                targets.insert(invalidated_pointer_target());
            }
        }
        Ok(())
    }
    fn captured_slot_present(
        &mut self,
        state: &State,
        base: &Place,
        ty: &Term,
        fields: &[usize],
        source: SourceRange,
    ) -> Result<bool, Error> {
        self.fuel = self
            .fuel
            .checked_sub(base.fields.len() + fields.len() + 1)
            .ok_or_else(|| error(source, "capture presence budget exhausted"))?;
        let slot = Place {
            root: base.root,
            fields: base.fields.iter().chain(fields).copied().collect(),
        };
        if let Ok(present) = state.may_be_present(&slot) {
            return Ok(present);
        }
        let layout = super::memory::pointer_input_layout(self.package, ty, &mut self.fuel)
            .map_err(|e| error(source, e))?;
        for leaf in layout {
            if leaf.storage_fields != fields {
                continue;
            }
            for (guard, _) in leaf.guards {
                self.fuel = self
                    .fuel
                    .checked_sub(base.fields.len() + guard.len() + 1)
                    .ok_or_else(|| error(source, "capture presence budget exhausted"))?;
                let owner = Place {
                    root: base.root,
                    fields: base.fields.iter().chain(&guard).copied().collect(),
                };
                if state.field_count(&owner).ok() == Some(None)
                    && state.may_be_present(&owner).unwrap_or(false)
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn capture_enum_snapshot(
        &mut self,
        flow: &Flow,
        source: SourceRange,
    ) -> Result<BTreeMap<PlaceId, EnumDomains>, Error> {
        let mut domains = BTreeMap::new();
        for root in &self.captured_roots {
            self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or_else(|| error(source, "capture enum snapshot budget exhausted"))?;
            if let Some(tags) = flow.enum_domains.get(root) {
                domains.insert(*root, copy_enum_domains(tags, &mut self.fuel, source)?);
            }
        }
        Ok(domains)
    }
    fn capture_storage_domains(
        &mut self,
        captures: &[nir::program::Capture],
        domains: &BTreeMap<PlaceId, EnumDomains>,
        source: SourceRange,
    ) -> Result<EnumDomains, Error> {
        let mut physical = EnumDomains::new();
        for (index, capture) in captures.iter().enumerate() {
            self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or_else(|| error(source, "capture enum writeback budget exhausted"))?;
            if let Some(tags) = domains.get(&root(capture.binding)) {
                for (path, tags) in tags {
                    self.fuel = self
                        .fuel
                        .checked_sub(path.len() + tags.len() + 1)
                        .ok_or_else(|| error(source, "capture enum writeback budget exhausted"))?;
                    physical.insert(
                        std::iter::once(index).chain(path.iter().copied()).collect(),
                        tags.clone(),
                    );
                }
            }
        }
        Ok(physical)
    }
    fn install_capture_domains(
        &mut self,
        flow: &mut Flow,
        binding: &Place,
        captures: &[nir::program::Capture],
        domains: &BTreeMap<PlaceId, EnumDomains>,
        source: SourceRange,
    ) -> Result<(), Error> {
        let physical = self.capture_storage_domains(captures, domains, source)?;
        self.install_enum_domains(flow, binding, physical, source)
    }
    fn restore_exit_capture_domains(
        &mut self,
        flow: &mut Flow,
        binding: &Place,
        captures: &[nir::program::Capture],
        exits: &[BTreeMap<PlaceId, EnumDomains>],
        source: SourceRange,
    ) -> Result<(), Error> {
        let mut merged = None;
        for domains in exits {
            merged = Some(if let Some(previous) = merged {
                join_capture_evidence(
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                    &previous,
                    domains,
                    &mut self.fuel,
                    source,
                    0,
                )?
                .1
            } else {
                copy_captured_enums(domains, &mut self.fuel, source)?
            });
        }
        let domains = merged.unwrap_or_default();
        self.install_capture_domains(flow, binding, captures, &domains, source)?;
        if let Some(receiver) = flow.callable_at_mut(binding) {
            receiver.enum_captures = domains;
        }
        Ok(())
    }

    fn capture_pointer_guard_snapshot(
        &mut self,
        flow: &Flow,
        pointers: &BTreeMap<Place, BTreeSet<Place>>,
        source: SourceRange,
    ) -> Result<BTreeMap<Place, PointerTargetGuards>, Error> {
        let mut snapshot = BTreeMap::new();
        for (slot, targets) in pointers {
            self.pointer_work(targets, source)?;
            self.fuel = self
                .fuel
                .checked_sub(slot.fields.len().saturating_add(1))
                .ok_or_else(|| error(source, "capture guard snapshot budget exhausted"))?;
            if let Some(entries) = flow.pointer_guards.get(slot) {
                let mut retained = PointerTargetGuards::new();
                for target in targets {
                    if let Some(conditions) = entries.get(target) {
                        self.charge_pointer_conditions(conditions, source)?;
                        retained.insert(target.clone(), conditions.clone());
                    }
                }
                if !retained.is_empty() {
                    snapshot.insert(slot.clone(), retained);
                }
            }
        }
        Ok(snapshot)
    }
    fn capture_pointer_snapshot(
        &mut self,
        flow: &Flow,
        source: SourceRange,
    ) -> Result<BTreeMap<Place, BTreeSet<Place>>, Error> {
        let mut snapshot = BTreeMap::new();
        if self.captured_roots.is_empty() {
            return Ok(snapshot);
        }
        for (slot, targets) in &flow.pointers {
            self.fuel = self
                .fuel
                .checked_sub(slot.fields.len() + 1)
                .ok_or_else(|| error(source, "capture pointer snapshot budget exhausted"))?;
            if self.captured_roots.contains(&slot.root)
                && self.captured_slot_present(
                    &flow.state,
                    &Place {
                        root: slot.root,
                        fields: vec![],
                    },
                    flow.types
                        .get(&slot.root)
                        .ok_or_else(|| error(source, "capture type missing"))?,
                    &slot.fields,
                    source,
                )?
            {
                self.pointer_work(targets, source)?;
                snapshot.insert(slot.clone(), targets.clone());
            }
        }
        Ok(snapshot)
    }

    fn exit(
        &mut self,
        flow: &mut Flow,
        source: SourceRange,
        reason: CleanupReason,
    ) -> Result<(), Error> {
        let callable_evidence = if reason == CleanupReason::PredicateFailure {
            None
        } else {
            flow.value_callable
                .as_ref()
                .map(|callable| export_callable_evidence(callable, &mut self.fuel, 0))
                .transpose()?
        };
        let callable_fields = if reason == CleanupReason::PredicateFailure {
            BTreeMap::new()
        } else {
            flow.value_callables
                .iter()
                .filter(|(path, _)| !path.is_empty())
                .map(|(path, value)| {
                    Ok((
                        path.clone(),
                        export_callable_evidence(value, &mut self.fuel, 0)?,
                    ))
                })
                .collect::<Result<_, Error>>()?
        };
        let variant_domains = if reason == CleanupReason::PredicateFailure {
            EnumDomains::new()
        } else {
            copy_enum_domains(&flow.value_variants, &mut self.fuel, source)?
        };
        let callable_candidates = if let Some(evidence) = &callable_evidence {
            legacy_callable_definitions(evidence, &mut self.fuel, 0)?
        } else {
            None
        };
        let callable_result = callable_candidates
            .as_ref()
            .filter(|values| values.len() == 1)
            .map(|values| values[0].clone());
        let locals = flow
            .order
            .iter()
            .filter(|id| !self.persistent_roots.contains(id))
            .copied()
            .collect();
        self.cleanup_exit(flow, locals, 0, 0, source, reason)?;
        self.incomplete_exit(source, 0);
        let capture_pointers = self.capture_pointer_snapshot(flow, source)?;
        let capture_guards =
            self.capture_pointer_guard_snapshot(flow, &capture_pointers, source)?;
        let capture_domains = self.capture_enum_snapshot(flow, source)?;
        if reason == CleanupReason::PredicateFailure {
            self.report
                .failed_capture_pointer_guard_exits
                .push(capture_guards);
            self.report.failed_capture_enum_exits.push(capture_domains);
            self.report
                .failed_capture_pointer_exits
                .push(capture_pointers);
            self.report.failed_exits.push(flow.state.clone());
        } else {
            self.report
                .captured_pointer_guard_exits
                .push(capture_guards);
            self.report.captured_pointer_exits.push(capture_pointers);
            self.report.exits.push(flow.state.clone());
            self.report.callable_exits.push(callable_result);
            self.report
                .callable_candidate_exits
                .push(callable_candidates);
            self.report.callable_evidence_exits.push(callable_evidence);
            self.report.callable_field_exits.push(callable_fields);
            self.report.enum_variant_exits.push(variant_domains);
            let captures = self
                .captured_roots
                .iter()
                .filter_map(|binding| {
                    flow.callables.get(binding).map(|callable| {
                        (
                            Place {
                                root: *binding,
                                fields: vec![],
                            },
                            callable,
                        )
                    })
                })
                .chain(
                    flow.callable_fields
                        .iter()
                        .filter(|(place, _)| self.captured_roots.contains(&place.root))
                        .map(|(place, callable)| (place.clone(), callable)),
                )
                .map(|(binding, callable)| {
                    Ok((
                        binding,
                        export_callable_evidence(callable, &mut self.fuel, 0)?,
                    ))
                })
                .collect::<Result<_, Error>>()?;
            self.report.captured_callable_exits.push(captures);
            self.report.captured_enum_exits.push(capture_domains);
        }
        let mut remaining: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (place, availability) in flow.state.remaining() {
            remaining
                .entry(place.root)
                .or_default()
                .push((place, availability));
        }
        for root in flow.order.iter().rev() {
            if self.persistent_roots.contains(root) {
                continue;
            }
            for (place, availability) in
                remaining.remove(root).unwrap_or_default().into_iter().rev()
            {
                self.report.cleanups.push(Cleanup {
                    source,
                    place,
                    availability,
                });
            }
        }
        flow.live = false;
        Ok(())
    }
    /// Bind a prepared input without ending the enclosing expression's temporaries.
    fn prepared_binding(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<(), Error> {
        if !flow.live {
            return Ok(());
        }
        if self.fuel == 0 || depth > 128 {
            return Err(error(expr.source, "call preparation budget exhausted"));
        }
        self.fuel -= 1;
        if let K::LetPattern {
            pattern: pattern @ Pattern::Tuple(_),
            value,
            guard: None,
            otherwise: None,
        } = &expr.kind
        {
            return self.let_pattern(pattern, value, None, None, flow, expr.source, depth + 1);
        }
        let K::Let { binding, value } = &expr.kind else {
            return Err(error(
                expr.source,
                "call preparation requires binding statements",
            ));
        };
        let callable = callable_source(value, flow, &mut self.fuel)?;
        let origins = self.evaluate_value_origins(value, flow, depth + 1)?;
        let pointer_guards =
            self.copy_value_pointer_guards(&flow.value_pointer_guards, expr.source)?;
        let callable = flow.value_callable.clone().or(callable);
        if flow.live {
            let leaves = flow.value_callables.clone();
            self.declare(flow, *binding, &value.ty, expr.source)?;
            self.install_callable_fields(
                flow,
                &Place {
                    root: root(*binding),
                    fields: vec![],
                },
                leaves,
                expr.source,
            )?;
            self.install_enum_domains(
                flow,
                &Place {
                    root: root(*binding),
                    fields: vec![],
                },
                flow.value_variants.clone(),
                expr.source,
            )?;
            self.install_value_origins(
                flow,
                &Place {
                    root: root(*binding),
                    fields: vec![],
                },
                origins,
                expr.source,
            )?;
            for (fields, guards) in pointer_guards {
                flow.pointer_guards.insert(
                    Place {
                        root: root(*binding),
                        fields,
                    },
                    guards,
                );
            }
            if let Some(callable) = callable {
                self.store_callable(flow, root(*binding), callable, expr.source)?;
            }
        }
        Ok(())
    }
    fn scope_origins(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        let outer = flow.roots.clone();
        let origins = self.evaluate_value_origins(expr, flow, depth)?;
        if flow.live {
            let locals: Vec<_> = flow
                .order
                .iter()
                .rev()
                .filter(|id| !outer.contains(id))
                .copied()
                .collect();
            self.check_scope_origins(&origins, &locals, expr.source)?;
            for id in locals {
                self.retire(flow, id, expr.source)?;
            }
        }
        Ok(origins)
    }
    fn scope(&mut self, expr: &Expr, flow: &mut Flow, depth: usize) -> Result<(), Error> {
        let outer = flow.roots.clone();
        self.expr(expr, flow, depth)?;
        if flow.live {
            let locals: Vec<_> = flow
                .order
                .iter()
                .rev()
                .filter(|id| !outer.contains(id))
                .copied()
                .collect();
            for id in locals {
                self.retire(flow, id, expr.source)?;
            }
        }
        Ok(())
    }
    fn fork(
        &mut self,
        left: &Expr,
        right: Option<&Expr>,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<(), Error> {
        let mut other = flow.clone();
        self.scope(left, flow, depth + 1)?;
        if let Some(right) = right {
            self.scope(right, &mut other, depth + 1)?;
        }
        match (flow.live, other.live) {
            (true, true) => {
                self.join_flow(flow, &other, left.source)?;
            }
            (false, true) => *flow = other,
            _ => (),
        }
        Ok(())
    }
    /// Invoking known code observes its owned slot. This grants no permission
    /// to deliver/copy the function value elsewhere, and checks availability
    /// before any argument can move or replace the selected slot.
    fn observe_named_callee(&mut self, callee: &Expr, flow: &mut Flow) -> Result<(), Error> {
        self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or_else(|| error(callee.source, "callee observation budget exhausted"))?;
        let place = expression_place(callee).ok_or_else(|| {
            error(
                callee.source,
                "named callee observation requires an owned position",
            )
        })?;
        self.fuel = self
            .fuel
            .checked_sub(place.fields.len())
            .ok_or_else(|| error(callee.source, "callee field observation budget exhausted"))?;
        flow.state
            .require_present(&place)
            .map_err(|reason| error(callee.source, format!("callee observation: {reason:?}")))?;
        flow.value_callable = None;
        flow.value_callables.clear();
        flow.value_variants.clear();
        Ok(())
    }

    /// Resolve the body selected by an owned callable before argument evaluation.
    /// The caller must still observe the original position once; this is provenance,
    /// not permission to bypass availability or to unfold a type-level head.
    fn indirect_named_call(&self, expr: &Expr, flow: &Flow) -> Option<Expr> {
        let K::Call {
            callee,
            arguments,
            parameter_bindings,
        } = &expr.kind
        else {
            return None;
        };
        let place = expression_place(callee)?;
        let callable = if place.fields.is_empty() {
            flow.callables.get(&place.root)?
        } else {
            flow.callable_fields.get(&place)?
        };
        if !callable.alternatives.is_empty() {
            return None;
        }
        let mut target = callable.definition.clone();
        let declaration = match target.kind {
            K::Function(id)
            | K::FunctionInstance {
                declaration: id, ..
            } => id,
            _ => return None,
        };
        target.source = callee.source;
        let bindings = if parameter_bindings.is_empty() {
            self.package
                .and_then(|p| p.functions.iter().find(|f| f.declaration == declaration))
                .map(|f| f.parameters.clone())
                .unwrap_or_default()
        } else {
            parameter_bindings.clone()
        };
        Some(Expr {
            source: expr.source,
            ty: expr.ty.clone(),
            kind: K::Call {
                callee: Box::new(target),
                arguments: arguments.clone(),
                parameter_bindings: bindings,
            },
        })
    }
    fn call_named_candidates(
        &mut self,
        expr: &Expr,
        callable: &Callable,
        arguments: &[nir::program::CallArgument],
        bindings: &[LocalId],
        flow: &mut Flow,
        depth: usize,
    ) -> Result<Option<BTreeSet<Place>>, Error> {
        self.call_named_candidates_with_origins(expr, callable, arguments, bindings, flow, depth)
            .map(|(pointer, _, _)| pointer)
    }

    fn call_named_candidates_with_origins(
        &mut self,
        expr: &Expr,
        callable: &Callable,
        arguments: &[nir::program::CallArgument],
        bindings: &[LocalId],
        flow: &mut Flow,
        depth: usize,
    ) -> Result<(Option<BTreeSet<Place>>, ValueOrigins, ValuePointerGuards), Error> {
        if arguments.iter().any(|a| a.is_default) {
            return Err(error(
                expr.source,
                "multi-target defaults require a shared parameter contract",
            ));
        }
        let declaration = match callable.definition.kind {
            K::Function(id)
            | K::FunctionInstance {
                declaration: id, ..
            } => id,
            _ => return Err(error(expr.source, "named callable definition is missing")),
        };
        let recovered;
        let bindings = if bindings.is_empty() {
            recovered = self
                .package
                .and_then(|p| p.functions.iter().find(|f| f.declaration == declaration))
                .ok_or_else(|| error(expr.source, "callable candidate body is missing"))?
                .parameters
                .clone();
            recovered.as_slice()
        } else {
            bindings
        };
        let mut target = callable.definition.clone();
        target.source = expr.source;
        let mut prepared_callables = vec![];
        let mut prepared_enums = vec![];
        let mut prepared_borrowed = vec![];
        let (result, summary, origins, guards) = self.memory_call(
            &target,
            arguments,
            bindings,
            &callable.alternatives,
            &mut prepared_callables,
            &mut prepared_enums,
            &mut prepared_borrowed,
            flow,
            expr.source,
            depth + 1,
        )?;
        let Some(summary) = summary else {
            return Ok((result, origins, guards));
        };
        flow.value_callable = if flow.live
            && matches!(expr.ty, Term::Function { .. } | Term::UnsafeFunction(_))
        {
            let definitions = match &summary.callable_result {
                super::memory::CallableResult::Known(key) => std::slice::from_ref(key),
                super::memory::CallableResult::Candidates(keys) => keys.as_slice(),
                _ => &[],
            };
            let mut result: Option<Callable> = None;
            for key in definitions {
                super::memory::bound_type(&expr.ty, &mut self.fuel)
                    .map_err(|e| error(expr.source, e))?;
                for ty in &key.arguments {
                    super::memory::bound_type(ty, &mut self.fuel)
                        .map_err(|e| error(expr.source, e))?;
                }
                let definition = Expr {
                    source: expr.source,
                    ty: expr.ty.clone(),
                    kind: if key.arguments.is_empty() {
                        K::Function(key.declaration)
                    } else {
                        K::FunctionInstance {
                            declaration: key.declaration,
                            arguments: key.arguments.clone(),
                        }
                    },
                };
                let next = callable_source(&definition, &Flow::default(), &mut self.fuel)?.unwrap();
                result = if let Some(previous) = result {
                    merge_named_callables(&previous, &next, &mut self.fuel, expr.source)?
                } else {
                    Some(next)
                };
            }
            result
        } else {
            None
        };
        let scalar_result = flow.value_callable.clone();
        let mut contextual_scalar: Option<Callable> = None;
        let mut scalar_complete = true;
        let mut result_fields: Option<(BTreeMap<Vec<usize>, Callable>, EnumDomains)> = None;
        if flow.live {
            for candidate in std::iter::once(&callable.definition).chain(&callable.alternatives) {
                // Each body gets the snapshots from the one preparation above.
                let candidate_scalar = self.record_named_call(
                    candidate,
                    prepared_callables.clone(),
                    prepared_enums.clone(),
                    &prepared_borrowed,
                    expr.source,
                    flow,
                )?;
                if scalar_complete {
                    contextual_scalar = match (contextual_scalar.take(), candidate_scalar) {
                        (None, Some(value)) => Some(value),
                        (Some(previous), Some(value)) => {
                            if same_callable_definition(&previous.definition, &value.definition)
                                && previous.choices == value.choices
                                && previous.alternatives == value.alternatives
                                && previous.captures == value.captures
                                && previous.enum_captures == value.enum_captures
                            {
                                Some(previous)
                            } else {
                                merge_callable_provenance(
                                    &previous,
                                    &value,
                                    &mut self.fuel,
                                    expr.source,
                                    0,
                                )?
                            }
                        }
                        _ => None,
                    };
                    scalar_complete = contextual_scalar.is_some();
                }
                let fields = std::mem::take(&mut flow.value_callables);
                let domains = std::mem::take(&mut flow.value_variants);
                result_fields = Some(if let Some((previous, previous_domains)) = result_fields {
                    let merged = join_guarded_callables(
                        &previous,
                        &fields,
                        &previous_domains,
                        &domains,
                        &mut self.fuel,
                        expr.source,
                    )?;
                    (
                        merged,
                        join_enum_domains(
                            &previous_domains,
                            &domains,
                            &mut self.fuel,
                            expr.source,
                        )?,
                    )
                } else {
                    (fields, domains)
                });
            }
        }
        // A context-sensitive result is valid only if every candidate supplied
        // compatible evidence. Otherwise retain independently proved worklist
        // evidence (which can cover recursive returns this local walk cannot).
        flow.value_callable = if scalar_complete {
            contextual_scalar.or(scalar_result)
        } else {
            scalar_result
        };
        (flow.value_callables, flow.value_variants) = result_fields.unwrap_or_default();
        self.refine_absent_result_payloads(&expr.ty, &origins, &guards, flow, expr.source)?;
        let mut candidates = vec![];
        for candidate in std::iter::once(&callable.definition).chain(&callable.alternatives) {
            super::memory::bound_type(&candidate.ty, &mut self.fuel)
                .map_err(|e| error(expr.source, e))?;
            candidates.push(candidate.clone());
        }
        self.report.named_candidate_calls.push(NamedCandidateCall {
            source: expr.source,
            candidates,
        });
        Ok((result, origins, guards))
    }
    fn memory_candidate_summary_with_enums(
        &mut self,
        callee: &Expr,
        alternatives: &[Expr],
        inputs: &[CallableParameter],
        borrowed: &[BorrowedCode],
        enums: &[EnumParameter],
        source: SourceRange,
    ) -> Result<super::memory::Summary, Error> {
        let mut combined: Option<super::memory::Summary> = None;
        for target in std::iter::once(callee).chain(alternatives) {
            let summary = self
                .memory_summary_with_enum_context(target, inputs, borrowed, enums, source, false)
                .map_err(|reason| {
                    error(
                        source,
                        format!("every callable candidate requires a memory contract: {reason}"),
                    )
                })?;
            if &summary.signature != callee.ty.callable_signature() {
                return Err(error(source, "callable candidate signature mismatch"));
            }
            if let Some(combined) = &mut combined {
                self.fuel = self
                    .fuel
                    .checked_sub(summary.accesses.len() + 1)
                    .ok_or_else(|| error(source, "callable candidate effects budget exhausted"))?;
                super::memory::join_code_fields(
                    &mut combined.code_fields,
                    &summary.code_fields,
                    &mut self.fuel,
                )
                .map_err(|reason| error(source, reason))?;
                combined.accesses.extend(summary.accesses);
                combined.may_reject |= summary.may_reject;
                // Cleanup is a union over all possible targets, just like
                // explicit accesses. Candidate order cannot erase it.
                combined.may_run_cleanup |= summary.may_run_cleanup;
                for (path, key) in &mut combined.callable_fields {
                    self.fuel = self
                        .fuel
                        .checked_sub(path.len() + 1)
                        .ok_or_else(|| error(source, "callable result merge budget exhausted"))?;
                    key.join(
                        summary
                            .callable_fields
                            .get(path)
                            .unwrap_or(&super::memory::CallableResult::Unknown),
                        &mut self.fuel,
                    )
                    .map_err(|reason| error(source, reason))?;
                }
                for (path, _) in summary.callable_fields {
                    self.fuel = self
                        .fuel
                        .checked_sub(path.len() + 1)
                        .ok_or_else(|| error(source, "callable result merge budget exhausted"))?;
                    combined
                        .callable_fields
                        .entry(path)
                        .or_insert(super::memory::CallableResult::Unknown);
                }
                combined
                    .callable_result
                    .join(&summary.callable_result, &mut self.fuel)
                    .map_err(|e| error(source, e))?;
                let mut next_fields = summary.pointer_fields;
                for (path, targets) in &mut combined.pointer_fields {
                    self.fuel = self
                        .fuel
                        .checked_sub(path.len() + 1)
                        .ok_or_else(|| error(source, "pointer result merge budget exhausted"))?;
                    let other = next_fields.remove(path).unwrap_or(None);
                    *targets = match (targets.take(), other) {
                        (Some(mut left), Some(right)) => {
                            for target in &right {
                                self.fuel = self
                                    .fuel
                                    .checked_sub(
                                        target.input_fields.len() + target.fields.len() + 1,
                                    )
                                    .ok_or_else(|| {
                                        error(source, "pointer result merge budget exhausted")
                                    })?;
                                for path in &target.through {
                                    self.fuel =
                                        self.fuel.checked_sub(path.len() + 1).ok_or_else(|| {
                                            error(source, "pointer result merge budget exhausted")
                                        })?;
                                }
                            }
                            left.extend(right);
                            Some(left)
                        }
                        _ => None,
                    };
                }
                for (path, _) in next_fields {
                    self.fuel = self
                        .fuel
                        .checked_sub(path.len() + 1)
                        .ok_or_else(|| error(source, "pointer result merge budget exhausted"))?;
                    combined.pointer_fields.insert(path, None);
                }
                combined.result_targets =
                    match (combined.result_targets.take(), summary.result_targets) {
                        (Some(mut left), Some(right)) => {
                            for place in &right {
                                self.fuel = self
                                    .fuel
                                    .checked_sub(place.fields.len() + 1)
                                    .ok_or_else(|| {
                                        error(source, "callable candidate results budget exhausted")
                                    })?;
                            }
                            left.extend(right);
                            Some(left)
                        }
                        _ => None,
                    };
            } else {
                combined = Some(summary);
            }
        }
        combined.ok_or_else(|| error(source, "empty callable candidates"))
    }
    fn memory_summary_with_inputs(
        &mut self,
        callee: &Expr,
        inputs: &[CallableParameter],
        source: SourceRange,
    ) -> Result<super::memory::Summary, String> {
        self.memory_summary_with_borrowed(callee, inputs, &[], source)
    }
    fn memory_summary_with_borrowed(
        &mut self,
        callee: &Expr,
        inputs: &[CallableParameter],
        borrowed: &[BorrowedCode],
        source: SourceRange,
    ) -> Result<super::memory::Summary, String> {
        self.memory_summary_with_enum_context(callee, inputs, borrowed, &[], source, false)
    }
    // Ordinary summaries may precede destructor selection. Observation calls
    // supply cleanup and tag premises from their independently checked frame.
    fn memory_summary_with_enum_context(
        &mut self,
        callee: &Expr,
        inputs: &[CallableParameter],
        borrowed: &[BorrowedCode],
        enums: &[EnumParameter],
        source: SourceRange,
        include_cleanup: bool,
    ) -> Result<super::memory::Summary, String> {
        let (declaration, arguments) = match &callee.kind {
            K::Function(id) => (*id, &[][..]),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.as_slice()),
            _ => return Err("memory callee must identify a function".into()),
        };
        infer_callable_memory_with_borrowed(
            self.package.ok_or("memory summary requires a package")?,
            declaration,
            arguments,
            inputs,
            enums,
            &self.effect_memory_inputs,
            borrowed,
            self.cleanup
                .as_mut()
                .map(|cleanup| &mut **cleanup as &mut dyn CleanupPolicy),
            include_cleanup,
            &mut self.fuel,
            source,
        )
    }
    fn memory_summary_with_code(
        &mut self,
        callee: &Expr,
        inputs: &BTreeMap<super::memory::CallableInput, super::memory::CallableResult>,
    ) -> Result<super::memory::Summary, String> {
        self.memory_summary_with_prepared_code(callee, inputs, &BTreeMap::new())
    }
    fn memory_summary_with_prepared_code(
        &mut self,
        callee: &Expr,
        inputs: &BTreeMap<super::memory::CallableInput, super::memory::CallableResult>,
        code_inputs: &super::memory::CodeInputs<'_>,
    ) -> Result<super::memory::Summary, String> {
        let (id, arguments) = match &callee.kind {
            K::Function(id) => (*id, &[][..]),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.as_slice()),
            _ => return Err("memory callee must identify a function".into()),
        };
        super::memory::infer_with_effect_inputs(
            self.package.ok_or("memory summary requires a package")?,
            id,
            arguments,
            inputs,
            code_inputs,
            &self.effect_memory_inputs,
            &mut self.fuel,
        )
    }
    /// Conditional borrowed-body analysis only. Concrete callers must establish
    /// every indirect footprint against their own stored pointer provenance.
    /// These roots are validity premises, never owned values or no-alias facts.
    fn borrowed_pointer_contents(
        &mut self,
        flow: &mut Flow,
        slot: &Place,
        source: SourceRange,
    ) -> Result<(), Error> {
        if flow.pointers.contains_key(slot)
            || !matches!(slot.root.owner, x if x == u64::MAX - 1 || x == u64::MAX - 2 || x == u64::MAX - 4)
        {
            return Ok(());
        }
        flow.state
            .require_present(slot)
            .map_err(|e| error(source, format!("borrowed pointer slot unavailable: {e:?}")))?;
        let root_ty = flow
            .types
            .get(&slot.root)
            .ok_or_else(|| error(source, "borrowed pointer root type missing"))?
            .clone();
        let Term::Pointer(subject) = self.field_type(&root_ty, &slot.fields, source)? else {
            return Err(error(source, "borrowed pointer slot has non-pointer type"));
        };
        self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or_else(|| error(source, "borrowed pointer premise budget exhausted"))?;
        let key = (flow.borrowed_scope, slot.clone(), vec![]);
        let pointee = if let Some((id, previous)) = self.borrowed_pointees.get(&key) {
            if previous != subject.as_ref() {
                return Err(error(source, "borrowed pointer premise changed type"));
            }
            *id
        } else {
            let local = self.next_borrowed_pointer;
            self.next_borrowed_pointer = local
                .checked_add(1)
                .ok_or_else(|| error(source, "borrowed pointer premise identity exhausted"))?;
            let id = PlaceId {
                owner: u64::MAX - 4,
                local,
            };
            self.borrowed_pointees.insert(key, (id, (*subject).clone()));
            id
        };
        if self.report.borrowed_roots.contains_key(&slot.root) {
            super::memory::bound_type(&subject, &mut self.fuel).map_err(|e| error(source, e))?;
            self.report.borrowed_roots.insert(
                pointee,
                BorrowedRoot::Indirect {
                    slot: slot.clone(),
                    pointee: (*subject).clone(),
                },
            );
        }
        if !flow.types.contains_key(&pointee) {
            let shape = (self.shape)(&subject).map_err(|e| error(source, e))?;
            flow.state
                .declare(pointee, &shape, true, &mut self.fuel)
                .map_err(|e| error(source, format!("borrowed pointer shape: {e:?}")))?;
            flow.types.insert(pointee, *subject);
        }
        flow.pointer_guards.remove(slot);
        flow.pointers.insert(
            slot.clone(),
            BTreeSet::from([Place {
                root: pointee,
                fields: vec![],
            }]),
        );
        Ok(())
    }
    /// Separate validity premises for distinct enum payloads sharing one slot.
    /// Stored candidates retain their guards in flow.pointer_guards.
    fn borrowed_guarded_contents(
        &mut self,
        flow: &mut Flow,
        base: &Place,
        fields: &[(nir::program::FieldOwner, usize)],
        guards: &[(Vec<usize>, usize)],
        subject: &Term,
        source: SourceRange,
    ) -> Result<Option<BTreeSet<Place>>, Error> {
        if !matches!(base.root.owner, x if x == u64::MAX - 1 || x == u64::MAX - 2 || x == u64::MAX - 4)
        {
            return Ok(None);
        }
        let mut slot = base.clone();
        slot.fields.extend(fields.iter().map(|(_, index)| *index));
        let mut absolute = Vec::new();
        for (path, tag) in guards {
            self.charge_pointer_conditions(guards, source)?;
            let mut prefix = base.fields.clone();
            prefix.extend(path);
            absolute.push((prefix, *tag));
        }
        super::memory::bound_type(subject, &mut self.fuel).map_err(|e| error(source, e))?;
        let key = (flow.borrowed_scope, slot.clone(), absolute.clone());
        let pointee = if let Some((id, previous)) = self.borrowed_pointees.get(&key) {
            if previous != subject {
                return Err(error(source, "conditional borrowed premise changed type"));
            }
            *id
        } else {
            let local = self.next_borrowed_pointer;
            self.next_borrowed_pointer = local
                .checked_add(1)
                .ok_or_else(|| error(source, "borrowed identity exhausted"))?;
            let id = PlaceId {
                owner: u64::MAX - 4,
                local,
            };
            self.borrowed_pointees.insert(key, (id, subject.clone()));
            id
        };
        if !flow.types.contains_key(&pointee) {
            let shape = (self.shape)(subject).map_err(|e| error(source, e))?;
            flow.state
                .declare(pointee, &shape, true, &mut self.fuel)
                .map_err(|e| error(source, format!("conditional borrowed shape: {e:?}")))?;
            flow.types.insert(pointee, subject.clone());
        }
        if self.report.borrowed_roots.contains_key(&base.root) {
            self.report.borrowed_roots.insert(
                pointee,
                BorrowedRoot::GuardedIndirect {
                    base: base.clone(),
                    fields: fields.to_vec(),
                    guards: guards.to_vec(),
                    pointee: subject.clone(),
                },
            );
        }
        let target = Place {
            root: pointee,
            fields: vec![],
        };
        // Keep the condition with the stored candidate so subsequent cleanup
        // snapshots can resolve the same load without treating variant payloads
        // that overlap physically as unconditional aliases.
        flow.pointers
            .entry(slot.clone())
            .or_default()
            .insert(target.clone());
        flow.pointer_guards
            .entry(slot)
            .or_default()
            .insert(target.clone(), absolute);
        Ok(Some(BTreeSet::from([target])))
    }
    fn declare_input_field_borrows(
        &mut self,
        flow: &mut Flow,
        binding: LocalId,
        ty: &Term,
        actual: &ValueOrigins,
        source: SourceRange,
    ) -> Result<ValueOrigins, Error> {
        let mut targets = BTreeMap::new();
        if actual.is_empty() {
            return Ok(targets);
        }
        let paths = super::memory::pointer_input_layout(self.package, ty, &mut self.fuel)
            .map_err(|reason| error(source, reason))?;
        for leaf in paths {
            let input = leaf.input_fields;
            let storage = leaf.storage_fields.clone();
            if input.is_empty() || !actual.contains_key(&input) {
                continue;
            }
            let subject = leaf.pointee;
            let local = self.next_borrowed_pointer;
            self.next_borrowed_pointer = local
                .checked_add(1)
                .ok_or_else(|| error(source, "borrowed input field identity exhausted"))?;
            let id = PlaceId {
                owner: u64::MAX - 4,
                local,
            };
            let shape = (self.shape)(&subject).map_err(|reason| error(source, reason))?;
            flow.state
                .declare(id, &shape, true, &mut self.fuel)
                .map_err(|reason| {
                    error(source, format!("borrowed input field shape: {reason:?}"))
                })?;
            flow.types.insert(id, subject);
            let borrowed = BTreeSet::from([Place {
                root: id,
                fields: vec![],
            }]);
            flow.pointers
                .entry(Place {
                    root: root(binding),
                    fields: storage,
                })
                .or_default()
                .extend(borrowed.iter().cloned());
            if !leaf.guards.is_empty() {
                self.charge_pointer_conditions(&leaf.guards, source)?;
                flow.pointer_guards
                    .entry(Place {
                        root: root(binding),
                        fields: leaf.storage_fields.clone(),
                    })
                    .or_default()
                    .insert(borrowed.iter().next().unwrap().clone(), leaf.guards);
            }
            targets.insert(input, borrowed);
            flow.memory_exposed = true;
        }
        Ok(targets)
    }
    fn capture_input_targets(
        &mut self,
        flow: &Flow,
        binding: &Place,
        index: usize,
        ty: &Term,
        domains: &EnumDomains,
        require_present: bool,
        source: SourceRange,
    ) -> Result<ValueOrigins, Error> {
        let layout = super::memory::pointer_input_layout(self.package, ty, &mut self.fuel)
            .map_err(|reason| error(source, reason))?;
        let mut inputs = ValueOrigins::new();
        for leaf in layout {
            self.fuel = self
                .fuel
                .checked_sub(
                    leaf.input_fields
                        .len()
                        .saturating_add(leaf.storage_fields.len())
                        .saturating_add(binding.fields.len())
                        .saturating_add(1),
                )
                .ok_or_else(|| error(source, "capture input binding budget exhausted"))?;
            if enum_path_excluded(&leaf.input_fields, domains) {
                inputs.insert(leaf.input_fields, BTreeSet::new());
                continue;
            }
            let place = Place {
                root: binding.root,
                fields: binding
                    .fields
                    .iter()
                    .copied()
                    .chain(std::iter::once(index))
                    .chain(leaf.storage_fields.iter().copied())
                    .collect(),
            };
            if require_present {
                // Payload fields are opaque until the matching tag is selected.
                // A complete capture proves their storage is present conditionally.
                let owner = if leaf.guards.is_empty() {
                    place.clone()
                } else {
                    Place {
                        root: binding.root,
                        fields: binding
                            .fields
                            .iter()
                            .copied()
                            .chain(std::iter::once(index))
                            .collect(),
                    }
                };
                flow.state
                    .require_present(&owner)
                    .map_err(|e| error(source, format!("pointer capture unavailable: {e:?}")))?;
            }
            if let Some(targets) = flow.pointers.get(&place) {
                self.pointer_work(targets, source)?;
                let mut selected = BTreeSet::new();
                for target in targets {
                    let conditions = flow
                        .pointer_guards
                        .get(&place)
                        .and_then(|entries| entries.get(target));
                    let mut excluded = false;
                    if let Some(conditions) = conditions {
                        for (path, tag) in conditions {
                            self.charge_pointer_conditions(&leaf.guards, source)?;
                            for (guard, required) in &leaf.guards {
                                if path.starts_with(&binding.fields)
                                    && path.get(binding.fields.len()) == Some(&index)
                                    && path[binding.fields.len() + 1..] == guard[..]
                                    && tag != required
                                {
                                    excluded = true;
                                }
                            }
                        }
                    }
                    if !excluded {
                        selected.insert(target.clone());
                    }
                }
                if selected.is_empty() && !targets.is_empty() {
                    return Err(error(
                        source,
                        "captured payload has no compatible pointer origin",
                    ));
                }
                inputs.insert(leaf.input_fields, selected);
            } else if require_present {
                return Err(error(
                    source,
                    "captured pointer provenance is not established",
                ));
            }
        }
        Ok(inputs)
    }

    fn memory_input_targets(
        &mut self,
        ty: &Term,
        origins: &ValueOrigins,
        domains: &EnumDomains,
        source: SourceRange,
    ) -> Result<ValueOrigins, Error> {
        if origins.is_empty() && domains.is_empty() {
            return Ok(BTreeMap::new());
        }
        let layout = super::memory::pointer_input_layout(self.package, ty, &mut self.fuel)
            .map_err(|reason| error(source, reason))?;
        origins::bind_input_origins(&layout, origins, domains, &mut self.fuel, source)
    }
    fn guarded_memory_projection(
        &mut self,
        base: &Place,
        fields: &[(nir::program::FieldOwner, usize)],
        guards: &[(Vec<usize>, usize)],
        flow: &Flow,
        source: SourceRange,
    ) -> Result<Option<(Place, Term)>, Error> {
        let root = flow
            .types
            .get(&base.root)
            .ok_or_else(|| error(source, "conditional memory root type missing"))?;
        let ty = self.field_type(root, &base.fields, source)?;
        let mut traversal =
            super::memory::StorageTraversal::new(self.package, &ty, &[], &mut self.fuel)
                .map_err(|reason| error(source, reason))?;
        traversal
            .then_guarded(self.package, fields, guards, &mut self.fuel)
            .map_err(|reason| error(source, reason))?;
        let domains = self.stored_enum_domains(flow, base, source)?;
        let activity = super::memory::storage_guard_activity(
            guards,
            &mut self.fuel,
            |path, tag| match domains.get(path) {
                Some(tags) if !tags.contains(&tag) => super::memory::GuardActivity::Inactive,
                Some(tags) if tags.len() == 1 => super::memory::GuardActivity::Active,
                _ => super::memory::GuardActivity::Conditional,
            },
        )
        .map_err(|reason| error(source, reason))?;
        if activity == super::memory::GuardActivity::Inactive {
            return Ok(None);
        }
        // Enums are opaque ownership nodes until a pattern opens their payload.
        // A typed guarded projection is readable when its enclosing enum is whole;
        // do not invent an unconditional physical payload node in ownership state.
        if let Some((container, _)) = guards.first() {
            let mut owner = base.clone();
            owner.fields.extend(container);
            flow.state.require_present(&owner).map_err(|reason| {
                error(
                    source,
                    format!("conditional memory owner unavailable: {reason:?}"),
                )
            })?;
        }
        let mut place = base.clone();
        place.fields.extend(fields.iter().map(|(_, index)| *index));
        Ok(Some((place, traversal.target.ty)))
    }
    fn map_memory_target(
        &mut self,
        target: &super::memory::ParameterPlace,
        arguments: &MemoryTargets,
        flow: &mut Flow,
        source: SourceRange,
    ) -> Result<Option<BTreeSet<Place>>, Error> {
        let Some(candidates) = arguments
            .get(&target.parameter)
            .and_then(|fields| fields.get(&target.input_fields))
        else {
            return Ok(None);
        };
        self.pointer_work(candidates, source)?;
        if target
            .conditions
            .keys()
            .any(|stage| *stage > target.through.len())
        {
            return Err(error(source, "memory guard stage out of bounds"));
        }
        for guards in target.conditions.values() {
            self.charge_pointer_conditions(guards, source)?;
        }
        let mut targets = candidates.clone();
        for (stage, fields) in target.through.iter().enumerate() {
            let guards = target
                .conditions
                .get(&stage)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let mut loaded = BTreeSet::new();
            for mut place in targets {
                self.fuel = self
                    .fuel
                    .checked_sub(fields.len().saturating_add(1))
                    .ok_or_else(|| error(source, "pointer provenance budget exhausted"))?;
                let base = place.clone();
                let expected = if guards.is_empty() {
                    place.fields.extend(fields.iter().map(|(_, index)| *index));
                    None
                } else {
                    let Some((projected, ty)) =
                        self.guarded_memory_projection(&base, fields, guards, flow, source)?
                    else {
                        continue;
                    };
                    place = projected;
                    let Term::Pointer(subject) = ty else {
                        return Err(error(source, "conditional memory load is not a pointer"));
                    };
                    Some(subject)
                };
                if guards.is_empty() {
                    flow.state.require_present(&place).map_err(|reason| {
                        error(
                            source,
                            format!("memory pointer slot unavailable: {reason:?}"),
                        )
                    })?;
                }
                let borrowed;
                let pointees = if let Some(subject) = &expected {
                    borrowed = self
                        .borrowed_guarded_contents(flow, &base, fields, guards, subject, source)?;
                    borrowed.as_ref().or_else(|| flow.pointers.get(&place))
                } else {
                    self.borrowed_pointer_contents(flow, &place, source)?;
                    flow.pointers.get(&place)
                };
                let Some(pointees) = pointees else {
                    return Ok(None);
                };
                self.pointer_work(pointees, source)?;
                for pointee in pointees {
                    if let Some(conditions) = flow
                        .pointer_guards
                        .get(&place)
                        .and_then(|entries| entries.get(pointee))
                    {
                        self.charge_pointer_conditions(conditions, source)?;
                        let mut excluded = false;
                        for (path, tag) in guards {
                            let mut absolute = base.fields.clone();
                            absolute.extend(path);
                            if conditions
                                .iter()
                                .any(|(other, actual)| *other == absolute && actual != tag)
                            {
                                excluded = true;
                                break;
                            }
                        }
                        if excluded {
                            continue;
                        }
                    }
                    if let Some(expected) = &expected {
                        let actual = flow.types.get(&pointee.root).ok_or_else(|| {
                            error(source, "conditional pointer target type missing")
                        })?;
                        if self.field_type(actual, &pointee.fields, source)? != **expected {
                            return Err(error(source, "conditional pointer target type mismatch"));
                        }
                    }
                    loaded.insert(pointee.clone());
                }
            }
            targets = loaded;
        }
        let mut result = BTreeSet::new();
        for mut place in targets {
            self.fuel = self
                .fuel
                .checked_sub(target.fields.len())
                .ok_or_else(|| error(source, "pointer provenance budget exhausted"))?;
            let guards = target
                .conditions
                .get(&target.through.len())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if guards.is_empty() {
                place
                    .fields
                    .extend(target.fields.iter().map(|(_, index)| *index));
            } else {
                let Some((projected, _)) =
                    self.guarded_memory_projection(&place, &target.fields, guards, flow, source)?
                else {
                    continue;
                };
                place = projected;
            }
            result.insert(place);
        }
        Ok(Some(result))
    }
    fn memory_accesses(
        &mut self,
        accesses: &[super::memory::Access],
        targets: &MemoryTargets,
        flow: &mut Flow,
        source: SourceRange,
    ) -> Result<(), Error> {
        for access in accesses {
            let targets = self
                .map_memory_target(&access.target, targets, flow, source)?
                .ok_or_else(|| error(source, "memory call target is not established"))?;
            for target in targets {
                let guarded = access
                    .target
                    .conditions
                    .get(&access.target.through.len())
                    .is_some_and(|guards| !guards.is_empty());
                let checked = if guarded {
                    // map_memory_target checked the complete containing enum.
                    Ok(())
                } else if access.read_value {
                    flow.state.require_present(&target)
                } else {
                    flow.state.require_place(&target)
                };
                checked.map_err(|reason| {
                    error(
                        source,
                        format!("memory call target unavailable: {reason:?}"),
                    )
                })?;
                if access.write {
                    self.forget_callable_fields(flow, &target, source)?;
                    if target.fields.is_empty() {
                        flow.callables.remove(&target.root);
                    }
                }
            }
        }
        Ok(())
    }
    fn type_cleanup_is_trivial(&mut self, ty: &Term, source: SourceRange) -> Result<bool, Error> {
        super::memory::bound_type(ty, &mut self.fuel).map_err(|e| error(source, e))?;
        for known in &self.trivial_cleanup_types {
            super::memory::bound_type(known, &mut self.fuel).map_err(|e| error(source, e))?;
            if known == ty {
                return Ok(true);
            }
        }
        let trivial = if let Some(cleanup) = self.cleanup.as_deref_mut() {
            cleanup
                .is_trivial(ty, &mut self.fuel)
                .map_err(|e| error(source, e))?
        } else {
            false
        };
        if trivial {
            self.trivial_cleanup_types.push(ty.clone());
        }
        Ok(trivial)
    }

    /// Refine the package-wide cleanup flag using current Copy/cleanup evidence.
    /// Every possible owned value must be trivially cleaned. Nested calls use
    /// independent no-cleanup summaries or join the concrete-instance worklist.
    /// Revisiting an instance proves neither termination nor runtime purity.
    fn callee_cleanup_is_trivial(
        &mut self,
        callee: &Expr,
        source: SourceRange,
    ) -> Result<bool, Error> {
        let Some(package) = self.package else {
            return Ok(false);
        };
        nir::visit::expression(callee, &mut self.fuel, |_, _| Ok(()))
            .map_err(|e| error(source, e))?;
        let mut pending = vec![callee.clone()];
        let mut seen = std::collections::HashSet::new();
        while let Some(callee) = pending.pop() {
            self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or_else(|| error(source, "cleanup call graph budget exhausted"))?;
            let (declaration, arguments) = match &callee.kind {
                K::Function(id) => (*id, &[][..]),
                K::FunctionInstance {
                    declaration,
                    arguments,
                } => (*declaration, arguments.as_slice()),
                _ => return Ok(false),
            };
            for argument in arguments {
                super::memory::bound_type(argument, &mut self.fuel)
                    .map_err(|e| error(source, e))?;
            }
            if !seen.insert((declaration, arguments.to_vec())) {
                continue;
            }
            let Some((function, arguments)) = package.runtime_body(declaration, arguments) else {
                return Ok(false);
            };
            let binders = super::memory::memory_binders(
                Some(package),
                &function.type_parameters,
                &mut self.fuel,
            )
            .map_err(|e| error(source, e))?;
            let signature = function
                .signature
                .instantiate_parameters(&binders, arguments)
                .map_err(|e| error(source, e.to_string()))?;
            super::memory::bound_type(&signature, &mut self.fuel).map_err(|e| error(source, e))?;
            let Term::Function { parameters, .. } = signature.callable_signature() else {
                return Ok(false);
            };
            let mut types = parameters.iter().map(|p| p.ty.clone()).collect::<Vec<_>>();
            let mut calls = vec![];
            let mut effects = false;
            for body in std::iter::once(&function.body)
                .chain(&function.preconditions)
                .chain(&function.postconditions)
            {
                nir::visit::expression(body, &mut self.fuel, |_, _| Ok(()))
                    .map_err(|e| error(source, e))?;
                let body = body
                    .instantiate_parameters(&binders, arguments)
                    .map_err(|e| error(source, e.to_string()))?;
                // Defaults are evaluated as part of calls in this body. Include
                // their cleanup too, while leaving deferred closure bodies out.
                nir::visit::expression_in_closure_environment(
                    &body,
                    &mut self.fuel,
                    |node, fuel| {
                        super::memory::bound_type(&node.ty, fuel)?;
                        types.push(node.ty.clone());
                        if let K::Call { callee, .. } = &node.kind {
                            nir::visit::expression(callee, fuel, |_, _| Ok(()))?;
                            calls.push((**callee).clone());
                        }
                        effects |= matches!(
                            node.kind,
                            K::EffectRequest { .. }
                                | K::EffectHandle { .. }
                                | K::EffectRun { .. }
                                | K::EffectComputation { .. }
                        );
                        Ok(())
                    },
                )
                .map_err(|e| error(source, e))?;
            }
            if effects {
                return Ok(false);
            }
            for ty in types {
                if !matches!(
                    ty,
                    Term::Primitive(_)
                        | Term::Pointer(_)
                        | Term::Function { .. }
                        | Term::UnsafeFunction(_)
                ) && !matches!((self.delivery)(&ty), Ok(Delivery::Copy))
                {
                    if !self.type_cleanup_is_trivial(&ty, source)? {
                        return Ok(false);
                    }
                }
            }
            for call in calls {
                match self.memory_summary_with_inputs(&call, &[], source) {
                    Ok(summary) if !summary.may_run_cleanup => {}
                    Ok(_) => pending.push(call),
                    Err(_) => return Ok(false),
                }
            }
        }
        Ok(true)
    }

    fn borrowed_code_inputs(
        &mut self,
        signature: &Term,
        targets: &BTreeMap<usize, BTreeMap<Vec<usize>, BTreeSet<Place>>>,
        flow: &Flow,
        enums: &[EnumParameter],
        source: SourceRange,
    ) -> Result<Vec<BorrowedCode>, Error> {
        let Term::Function { parameters, .. } = signature.callable_signature() else {
            return Err(error(source, "borrowed code requires a function signature"));
        };
        let mut inputs = vec![];
        for (parameter, argument) in parameters.iter().enumerate() {
            let actuals = targets.get(&parameter);
            let domains = enums
                .iter()
                .find(|input| input.parameter == parameter)
                .map(|input| &input.domains);
            if actuals.is_none() && domains.is_none() {
                continue;
            }
            let leaves = if let Some((_, pointee)) =
                super::memory::pointer_representation(self.package, &argument.ty, &mut self.fuel)
                    .map_err(|e| error(source, e))?
            {
                vec![(vec![], pointee)]
            } else {
                super::memory::pointer_input_layout(self.package, &argument.ty, &mut self.fuel)
                    .map_err(|e| error(source, e))?
                    .into_iter()
                    .map(|leaf| (leaf.input_fields, leaf.pointee))
                    .collect()
            };
            for (input_fields, pointee) in leaves {
                if callable_value_paths(self.package, &pointee, &mut self.fuel, source, 0)?
                    .is_empty()
                {
                    continue;
                }
                if domains.is_some_and(|domains| enum_path_excluded(&input_fields, domains)) {
                    inputs.push(BorrowedCode {
                        absent: true,
                        invocation: false,
                        parameter,
                        input_fields,
                        pointee,
                        fields: BTreeMap::new(),
                        domains: BTreeMap::new(),
                    });
                    continue;
                }
                let Some(places) = actuals.and_then(|actuals| actuals.get(&input_fields)) else {
                    continue;
                };
                self.pointer_work(places, source)?;
                let mut merged = None;
                let mut complete = true;
                for place in places {
                    let Some(root_ty) = flow.types.get(&place.root) else {
                        complete = false;
                        break;
                    };
                    if self.field_type(root_ty, &place.fields, source)? != pointee {
                        return Err(error(source, "borrowed code storage type mismatch"));
                    }
                    flow.state.require_present(place).map_err(|e| {
                        error(source, format!("borrowed code storage unavailable: {e:?}"))
                    })?;
                    let domains = self.stored_enum_domains(flow, place, source)?;
                    let fields = self.stored_code(flow, place, source)?;
                    merged = Some(if let Some((previous_fields, previous_domains)) = merged {
                        let fields = join_guarded_callables(
                            &previous_fields,
                            &fields,
                            &previous_domains,
                            &domains,
                            &mut self.fuel,
                            source,
                        )?;
                        let domains =
                            join_enum_domains(&previous_domains, &domains, &mut self.fuel, source)?;
                        (fields, domains)
                    } else {
                        (fields, domains)
                    });
                }
                if complete {
                    if let Some((fields, domains)) = merged {
                        inputs.push(BorrowedCode {
                            absent: false,
                            invocation: false,
                            parameter,
                            input_fields,
                            pointee,
                            fields,
                            domains,
                        });
                    }
                }
            }
        }
        Ok(inputs)
    }

    fn memory_call(
        &mut self,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
        bindings: &[LocalId],
        alternatives: &[Expr],
        prepared_callables: &mut Vec<CallableParameter>,
        prepared_enums: &mut Vec<EnumParameter>,
        prepared_borrowed: &mut Vec<BorrowedCode>,
        flow: &mut Flow,
        source: SourceRange,
        depth: usize,
    ) -> Result<
        (
            Option<BTreeSet<Place>>,
            Option<super::memory::Summary>,
            ValueOrigins,
            ValuePointerGuards,
        ),
        Error,
    > {
        let selected;
        let callee = if matches!(callee.kind, K::TraitMethod { .. }) {
            let package = self
                .package
                .ok_or_else(|| error(source, "method memory analysis requires a package"))?;
            selected = self
                .method_callees
                .select(package, callee, &mut self.fuel)
                .map_err(|reason| error(source, reason))?;
            &selected
        } else {
            callee
        };
        let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
            return Err(error(source, "memory call lacks a signature"));
        };
        super::memory::validate_argument_plan(arguments, bindings, parameters.len())
            .map_err(|message| error(source, message))?;
        if bindings.len() != parameters.len()
            || bindings.iter().collect::<BTreeSet<_>>().len() != bindings.len()
        {
            return Err(error(
                source,
                "memory call requires a complete unique parameter binding frame",
            ));
        }
        self.expr(callee, flow, depth + 1)?;
        let mut targets = BTreeMap::new();
        let mut prepared = Vec::new();
        let mut frame = Flow {
            borrowed_scope: self.fresh_borrowed_scope(source)?,
            live: true,
            ..Default::default()
        };
        let mut frame_targets = BTreeMap::new();
        self.pending_frame(source, |this| {
            for argument in arguments {
                let mut default_targets = None;
                let mut origins = BTreeMap::new();
                if argument.is_default {
                    let package = this.package.ok_or_else(|| {
                        error(source, "default memory analysis requires a package")
                    })?;
                    let argument_summary = super::memory::infer_argument(
                        package,
                        &argument.value,
                        &prepared,
                        &mut this.fuel,
                    )
                    .map_err(|message| error(argument.value.source, message))?;
                    this.memory_accesses(
                        &argument_summary.accesses,
                        &targets,
                        flow,
                        argument.value.source,
                    )?;
                    // Produce the same isolated cleanup report as ordinary
                    // defaults. The footprint above checks external accesses;
                    // this frame owns only prepared parameters and default locals.
                    this.evaluate_default(argument, &mut frame, flow, depth + 1)?;
                    if !flow.live {
                        return Ok(());
                    }
                    if !argument_summary.pointer_fields.is_empty() {
                        let storage_paths = super::memory::conditional_pointer_storage_paths(
                            this.package,
                            &argument.value.ty,
                            &mut this.fuel,
                        )
                        .map_err(|reason| error(argument.value.source, reason))?;
                        let mut actual_fields = BTreeMap::new();
                        for (path, alternatives) in &argument_summary.pointer_fields {
                            this.fuel = this.fuel.checked_sub(path.len() + 1).ok_or_else(|| {
                                error(source, "default result field budget exhausted")
                            })?;
                            let storage = storage_paths.get(path).ok_or_else(|| {
                                error(source, "default result field layout mismatch")
                            })?;
                            let mut actual = BTreeSet::new();
                            if let Some(alternatives) = alternatives {
                                for target in alternatives {
                                    if let Some(mapped) = this.map_memory_target(
                                        target,
                                        &targets,
                                        flow,
                                        argument.value.source,
                                    )? {
                                        actual.extend(mapped);
                                    } else {
                                        actual.insert(unknown_pointer_target());
                                    }
                                }
                            } else {
                                actual.insert(unknown_pointer_target());
                            }
                            origins
                                .entry(storage.clone())
                                .or_insert_with(BTreeSet::new)
                                .extend(actual.iter().cloned());
                            actual_fields.insert(path.clone(), actual);
                        }
                        targets.insert(argument.parameter, actual_fields);
                    }
                    if let Some(alternatives) = argument_summary.result_targets {
                        let mut actual = BTreeSet::new();
                        let mut local = BTreeSet::new();
                        let mut known = true;
                        for target in alternatives {
                            match (
                                this.map_memory_target(
                                    &target,
                                    &targets,
                                    flow,
                                    argument.value.source,
                                )?,
                                this.map_memory_target(
                                    &target,
                                    &frame_targets,
                                    &mut frame,
                                    argument.value.source,
                                )?,
                            ) {
                                (Some(mapped), Some(borrowed)) => {
                                    actual.extend(mapped);
                                    local.extend(borrowed);
                                }
                                _ => known = false,
                            }
                        }
                        if !known {
                            actual.insert(unknown_pointer_target());
                            local.insert(unknown_pointer_target());
                        }
                        let pointer_depth = super::memory::pointer_representation(
                            this.package,
                            &argument.value.ty,
                            &mut this.fuel,
                        )
                        .map_err(|reason| error(source, reason))?
                        .ok_or_else(|| {
                            error(source, "default pointer result lacks representation")
                        })?
                        .0;
                        origins.insert(vec![0; pointer_depth], actual.clone());
                        targets.insert(argument.parameter, BTreeMap::from([(vec![], actual)]));
                        default_targets = Some(local);
                    }
                } else {
                    // Snapshot in source order, before later argument effects.
                    origins = this.evaluate_value_origins(&argument.value, flow, depth + 1)?;
                    let mut symbolic = this.memory_input_targets(
                        &argument.value.ty,
                        &origins,
                        &flow.value_variants,
                        source,
                    )?;
                    let layout = super::memory::pointer_input_layout(
                        this.package,
                        &argument.value.ty,
                        &mut this.fuel,
                    )
                    .map_err(|reason| error(source, reason))?;
                    for leaf in &layout {
                        if let Some(actual) = symbolic.get_mut(&leaf.input_fields) {
                            if !actual.is_empty() {
                                *actual = this.guarded_request_targets(
                                    &argument.value,
                                    leaf,
                                    actual,
                                    flow,
                                    true,
                                )?;
                            }
                        }
                    }
                    targets.insert(argument.parameter, symbolic);
                    if !flow.live {
                        return Ok(());
                    }
                }
                let delivered = if argument.is_default { &frame } else { &*flow };
                let callable = delivered.value_callable.clone();
                let fields = delivered.value_callables.clone();
                let domains = copy_enum_domains(&delivered.value_variants, &mut this.fuel, source)?;
                record_enum_parameter(
                    prepared_enums,
                    argument.parameter,
                    &domains,
                    &mut this.fuel,
                    source,
                )?;
                if let Some(value) = &callable {
                    prepared_callables.push(export_callable_parameter(
                        argument.parameter,
                        value,
                        &mut this.fuel,
                    )?);
                }
                for (path, value) in &fields {
                    if !path.is_empty() {
                        let mut input =
                            export_callable_parameter(argument.parameter, value, &mut this.fuel)?;
                        input.fields = path.clone();
                        prepared_callables.push(input);
                    }
                }
                this.remember_pending_origins(&argument.value, argument.parameter, flow, origins)?;
                let binding = bindings[argument.parameter];
                let ty = &parameters[argument.parameter].ty;
                this.declare(&mut frame, binding, ty, source)?;
                if let Some(value) = callable {
                    this.store_callable(&mut frame, root(binding), value, source)?;
                }
                this.install_callable_fields(
                    &mut frame,
                    &Place {
                        root: root(binding),
                        fields: vec![],
                    },
                    fields,
                    source,
                )?;
                this.install_enum_domains(
                    &mut frame,
                    &Place {
                        root: root(binding),
                        fields: vec![],
                    },
                    domains,
                    source,
                )?;
                if let Some((pointer_depth, pointee)) =
                    super::memory::pointer_representation(this.package, ty, &mut this.fuel)
                        .map_err(|reason| error(source, reason))?
                {
                    let borrowed = if argument.is_default {
                        default_targets
                    } else if targets
                        .get(&argument.parameter)
                        .is_some_and(|fields| fields.contains_key(&vec![]))
                    {
                        // Separate borrowed roots avoid collisions between caller
                        // locals and callee parameter IDs, including recursion.
                        let id = PlaceId {
                            owner: u64::MAX - 2,
                            local: u32::try_from(argument.parameter)
                                .map_err(|_| error(source, "too many borrowed argument slots"))?,
                        };
                        let shape = (this.shape)(&pointee).map_err(|e| error(source, e))?;
                        frame
                            .state
                            .declare(id, &shape, true, &mut this.fuel)
                            .map_err(|e| {
                                error(source, format!("borrowed argument shape: {e:?}"))
                            })?;
                        frame.types.insert(id, pointee.clone());
                        Some(BTreeSet::from([Place {
                            root: id,
                            fields: vec![],
                        }]))
                    } else {
                        None
                    };
                    if let Some(borrowed) = borrowed {
                        frame_targets.insert(
                            argument.parameter,
                            BTreeMap::from([(vec![], borrowed.clone())]),
                        );
                        frame.pointers.insert(
                            Place {
                                root: root(binding),
                                fields: vec![0; pointer_depth],
                            },
                            borrowed,
                        );
                    }
                    frame.memory_exposed = true;
                }
                if let Some(actual) = targets.get(&argument.parameter) {
                    let fields =
                        this.declare_input_field_borrows(&mut frame, binding, ty, actual, source)?;
                    if !fields.is_empty() {
                        frame_targets
                            .entry(argument.parameter)
                            .or_default()
                            .extend(fields);
                    }
                }
                prepared.push((binding, ty.clone(), argument.parameter));
            }
            Ok(())
        })?;
        if flow.live {
            for binding in bindings {
                frame
                    .state
                    .require_present(&Place {
                        root: root(*binding),
                        fields: vec![],
                    })
                    .map_err(|_| error(source, "memory default consumed a prepared parameter"))?;
            }
        }
        if !flow.live {
            return Ok((None, None, BTreeMap::new(), BTreeMap::new()));
        }
        prepared_borrowed.clear();
        let mut summary_result = self.memory_candidate_summary_with_enums(
            callee,
            alternatives,
            prepared_callables,
            &[],
            prepared_enums,
            source,
        );
        if summary_result.as_ref().is_err_and(|failure| {
            failure
                .reason
                .contains("dynamic call requires a richer memory contract")
        }) {
            let mut borrowed =
                self.borrowed_code_inputs(&callee.ty, &targets, flow, prepared_enums, source)?;
            if !borrowed.is_empty() {
                for input in &mut borrowed {
                    input.invocation = true;
                }
                summary_result = self.memory_candidate_summary_with_enums(
                    callee,
                    alternatives,
                    prepared_callables,
                    &borrowed,
                    prepared_enums,
                    source,
                );
                if summary_result.is_ok() {
                    *prepared_borrowed = borrowed;
                }
            }
        }
        let summary = match summary_result {
            Ok(summary) => summary,
            Err(failure)
                if self.defer_unknown_calls
                    && failure.reason
                        == "every callable candidate requires a memory contract: dynamic call requires a richer memory contract" =>
            {
                // Construction-time checks have no concrete callback arguments.
                // Preserve the obligation for invocation without inventing an
                // access summary or a returned pointer target.
                self.report.deferred_calls.push(source);
                return Ok((None, None, BTreeMap::new(), BTreeMap::new()));
            }
            Err(failure) => return Err(failure),
        };
        if &summary.signature != callee.ty.callable_signature() {
            return Err(error(
                source,
                "memory callee signature disagrees with its body",
            ));
        }
        // Capture after every argument/default has run. Do not seed a body
        // that can mutate storage or run unmodeled implicit cleanup: parameter
        // aliases would otherwise make independent borrowed roots unsound.
        let returns_code = match summary.signature.callable_signature() {
            Term::Function { result, .. } => contains_callable_value(
                self.package,
                completion_type(result),
                &mut self.fuel,
                source,
                0,
            )?,
            _ => false,
        };
        let mut cleanup_is_trivial = !summary.may_run_cleanup;
        if returns_code
            && !cleanup_is_trivial
            && summary.accesses.iter().all(|access| !access.write)
        {
            cleanup_is_trivial = true;
            for candidate in std::iter::once(callee).chain(alternatives) {
                if !self.callee_cleanup_is_trivial(candidate, source)? {
                    cleanup_is_trivial = false;
                    break;
                }
            }
        }
        if prepared_borrowed.is_empty()
            && returns_code
            && cleanup_is_trivial
            && summary.accesses.iter().all(|access| !access.write)
        {
            *prepared_borrowed = self.borrowed_code_inputs(
                &summary.signature,
                &targets,
                flow,
                prepared_enums,
                source,
            )?;
        }
        if flow.live {
            self.memory_accesses(&summary.accesses, &targets, flow, source)?;
            if summary.may_reject {
                let mut failed = flow.clone();
                self.exit(&mut failed, source, CleanupReason::PredicateFailure)?;
            }
        }
        if !flow.live {
            return Ok((None, Some(summary), BTreeMap::new(), BTreeMap::new()));
        }
        let mut origins = BTreeMap::new();
        let Term::Function { result, .. } = summary.signature.callable_signature() else {
            return Err(error(source, "memory result lacks a signature"));
        };
        let storage_paths = if summary.pointer_fields.is_empty() {
            BTreeMap::new()
        } else {
            super::memory::pointer_input_layout(
                self.package,
                completion_type(result),
                &mut self.fuel,
            )
            .map_err(|reason| error(source, reason))?
            .into_iter()
            .map(|leaf| (leaf.input_fields.clone(), leaf))
            .collect()
        };
        let mut result_guards = ValuePointerGuards::new();
        for (path, results) in &summary.pointer_fields {
            self.fuel = self
                .fuel
                .checked_sub(path.len() + 1)
                .ok_or_else(|| error(source, "memory result field budget exhausted"))?;
            let leaf = storage_paths
                .get(path)
                .ok_or_else(|| error(source, "memory result field layout mismatch"))?;
            let mut actual = BTreeSet::new();
            if let Some(results) = results {
                for result in results {
                    if let Some(targets) = self.map_memory_target(result, &targets, flow, source)? {
                        actual.extend(targets);
                    } else {
                        actual.insert(unknown_pointer_target());
                    }
                }
            } else {
                actual.insert(unknown_pointer_target());
            }
            self.record_result_pointer_guards(&mut result_guards, leaf, &actual, source)?;
            origins
                .entry(leaf.storage_fields.clone())
                .or_insert_with(BTreeSet::new)
                .extend(actual);
        }
        result_guards.retain(|_, entries| {
            entries.retain(|_, conditions| !conditions.is_empty());
            !entries.is_empty()
        });
        let Some(results) = &summary.result_targets else {
            return Ok((None, Some(summary), origins, result_guards));
        };
        let mut all = BTreeSet::new();
        for target in results {
            if let Some(actual) = self.map_memory_target(target, &targets, flow, source)? {
                all.extend(actual);
            } else {
                all.insert(unknown_pointer_target());
            }
        }
        Ok((Some(all), Some(summary), origins, result_guards))
    }
    fn pointer_work(
        &mut self,
        targets: &BTreeSet<Place>,
        source: SourceRange,
    ) -> Result<(), Error> {
        for target in targets {
            let Some(remaining) = self.fuel.checked_sub(target.fields.len().saturating_add(1))
            else {
                self.fuel = 0;
                return Err(error(source, "pointer provenance budget exhausted"));
            };
            self.fuel = remaining;
        }
        Ok(())
    }
    fn pointer_target(
        &mut self,
        expr: &Expr,
        flow: &Flow,
        depth: usize,
    ) -> Result<BTreeSet<Place>, Error> {
        if self.fuel == 0 || depth > 128 {
            return Err(error(expr.source, "pointer provenance budget exhausted"));
        }
        self.fuel -= 1;
        match &expr.kind {
            K::AddressOf(place) => expression_place(place)
                .map(|place| BTreeSet::from([place]))
                .ok_or_else(|| error(expr.source, "pointer address requires a known local place")),
            K::Local(_) | K::AnonymousField { .. } | K::Field { .. } => {
                let slot = expression_place(expr)
                    .ok_or_else(|| error(expr.source, "pointer value requires a local place"))?;
                let targets = flow.pointers.get(&slot).ok_or_else(|| {
                    error(expr.source, "pointer target provenance is not established")
                })?;
                self.pointer_work(targets, expr.source)?;
                Ok(targets.clone())
            }
            K::ReadPointer(pointer) => {
                let slots = self.pointer_target(pointer, flow, depth + 1)?;
                let mut targets = BTreeSet::new();
                for slot in slots {
                    flow.state.require_present(&slot).map_err(|e| {
                        error(expr.source, format!("pointer slot unavailable: {e:?}"))
                    })?;
                    let values = flow.pointers.get(&slot).ok_or_else(|| {
                        error(
                            expr.source,
                            "pointer slot contents provenance is not established",
                        )
                    })?;
                    self.pointer_work(values, expr.source)?;
                    targets.extend(values.iter().cloned());
                }
                Ok(targets)
            }
            K::PointerAnonymousField { pointer, index, .. }
            | K::PointerField { pointer, index, .. } => {
                let targets = self.pointer_target(pointer, flow, depth + 1)?;
                self.pointer_work(&targets, expr.source)?;
                Ok(targets
                    .into_iter()
                    .map(|mut target| {
                        target.fields.push(*index);
                        target
                    })
                    .collect())
            }
            K::Block {
                statements,
                result: Some(result),
            } if statements.is_empty() => self.pointer_target(result, flow, depth + 1),
            K::If {
                condition,
                then_branch,
                else_branch: Some(other),
            } if matches!(
                condition.kind,
                K::Local(_) | K::Literal(nir::program::Literal::Bool(_))
            ) =>
            {
                match constant_bool(condition, 0) {
                    Some(true) => self.pointer_target(then_branch, flow, depth + 1),
                    Some(false) => self.pointer_target(other, flow, depth + 1),
                    None => {
                        let mut targets = self.pointer_target(then_branch, flow, depth + 1)?;
                        targets.extend(self.pointer_target(other, flow, depth + 1)?);
                        Ok(targets)
                    }
                }
            }
            _ => Err(error(
                expr.source,
                "pointer target requires a memory contract",
            )),
        }
    }
    fn charge_pointer_conditions(
        &mut self,
        guards: &[(Vec<usize>, usize)],
        source: SourceRange,
    ) -> Result<(), Error> {
        for (path, _) in guards {
            self.fuel = self
                .fuel
                .checked_sub(path.len() + 1)
                .ok_or_else(|| error(source, "pointer condition budget exhausted"))?;
        }
        Ok(())
    }
    fn merge_pointer_conditions(
        &mut self,
        guarded: &mut PointerTargetGuards,
        actual: &BTreeSet<Place>,
        conditions: &[(Vec<usize>, usize)],
        source: SourceRange,
    ) -> Result<(), Error> {
        self.pointer_work(actual, source)?;
        for target in actual {
            self.charge_pointer_conditions(conditions, source)?;
            match guarded.entry(target.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(conditions.to_vec());
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    // One actual target can occur under multiple return tags.
                    // Retain only conditions necessary for every occurrence.
                    let mut common = vec![];
                    for condition in entry.get() {
                        self.charge_pointer_conditions(conditions, source)?;
                        if conditions.contains(condition) {
                            common.push(condition.clone());
                        }
                    }
                    entry.insert(common);
                }
            }
        }
        Ok(())
    }
    /// A completely mapped, empty result leaf cannot contain a live pointer.
    /// Recover the excluded payload tag so later by-value calls do not turn
    /// established absence back into unknown pointer provenance.
    fn refine_absent_result_payloads(
        &mut self,
        ty: &Term,
        origins: &ValueOrigins,
        qualifiers: &ValuePointerGuards,
        flow: &mut Flow,
        source: SourceRange,
    ) -> Result<(), Error> {
        let layout =
            super::memory::pointer_input_layout(self.package, completion_type(ty), &mut self.fuel)
                .map_err(|reason| error(source, reason))?;
        for leaf in layout {
            let Some(targets) = origins.get(&leaf.storage_fields) else {
                continue;
            };
            let mut absent = true;
            for target in targets {
                let Some(conditions) = qualifiers
                    .get(&leaf.storage_fields)
                    .and_then(|entries| entries.get(target))
                else {
                    absent = false;
                    break;
                };
                self.charge_pointer_conditions(conditions, source)?;
                self.fuel = self
                    .fuel
                    .checked_sub(
                        conditions
                            .len()
                            .saturating_mul(leaf.guards.len())
                            .saturating_add(target.fields.len()),
                    )
                    .ok_or_else(|| error(source, "absent result correlation budget exhausted"))?;
                let excluded = conditions.iter().any(|(path, tag)| {
                    leaf.guards
                        .iter()
                        .any(|(selected, actual)| selected == path && actual != tag)
                });
                if !excluded {
                    absent = false;
                    break;
                }
            }
            if !absent {
                continue;
            }
            let Some(marker) = leaf
                .input_fields
                .iter()
                .rposition(|field| *field == VARIANT_PATH)
            else {
                continue;
            };
            let path = &leaf.input_fields[..marker];
            let tag = leaf.input_fields[marker + 1];
            let owner = self.field_type(completion_type(ty), path, source)?;
            let variants = super::memory::owned_variants(self.package, &owner, &mut self.fuel)
                .map_err(|reason| error(source, reason))?;
            self.fuel = self
                .fuel
                .checked_sub(variants.len() + path.len() + 1)
                .ok_or_else(|| error(source, "absent result domain budget exhausted"))?;
            let mut allowed = flow
                .value_variants
                .get(path)
                .cloned()
                .unwrap_or_else(|| (0..variants.len()).collect());
            allowed.remove(&tag);
            if !allowed.is_empty() {
                flow.value_variants.insert(path.to_vec(), allowed);
            }
        }
        Ok(())
    }
    fn record_result_pointer_guards(
        &mut self,
        guards: &mut ValuePointerGuards,
        leaf: &super::memory::PointerInputLeaf,
        actual: &BTreeSet<Place>,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.merge_pointer_conditions(
            guards.entry(leaf.storage_fields.clone()).or_default(),
            actual,
            &leaf.guards,
            source,
        )
    }
    fn join_result_pointer_guards(
        &mut self,
        left: &ValuePointerGuards,
        left_origins: &ValueOrigins,
        right: &ValuePointerGuards,
        right_origins: &ValueOrigins,
        source: SourceRange,
        domains: Option<(&Term, &EnumDomains, &EnumDomains)>,
    ) -> Result<ValuePointerGuards, Error> {
        let mut joined = ValuePointerGuards::new();
        if left.is_empty() && right.is_empty() {
            return Ok(joined);
        }
        let layout = match domains {
            Some((ty, _, _)) => {
                super::memory::pointer_input_layout(self.package, ty, &mut self.fuel)
                    .map_err(|reason| error(source, reason))?
            }
            None => vec![],
        };
        for (own, own_origins, other, other_origins, other_domains) in [
            (
                left,
                left_origins,
                right,
                right_origins,
                domains.map(|(_, _, right)| right),
            ),
            (
                right,
                right_origins,
                left,
                left_origins,
                domains.map(|(_, left, _)| left),
            ),
        ] {
            for (slot, entries) in own {
                self.fuel = self
                    .fuel
                    .checked_sub(slot.len().saturating_add(1))
                    .ok_or_else(|| error(source, "result pointer guard join budget exhausted"))?;
                for (target, conditions) in entries {
                    self.charge_pointer_conditions(conditions, source)?;
                    if !own_origins
                        .get(slot)
                        .is_some_and(|targets| targets.contains(target))
                    {
                        continue;
                    }
                    let other_targets = other_origins.get(slot);
                    if other_targets.is_none() {
                        // Missing metadata is normally unknown. Only a complete
                        // layout plus excluded tags proves this physical slot
                        // absent; overlapping leaves must all be excluded.
                        let mut absent = false;
                        if let Some(domains) = other_domains {
                            for leaf in &layout {
                                self.fuel = self
                                    .fuel
                                    .checked_sub(leaf.storage_fields.len() + 1)
                                    .ok_or_else(|| {
                                        error(source, "absent pointer slot budget exhausted")
                                    })?;
                                if &leaf.storage_fields != slot {
                                    continue;
                                }
                                let activity = super::memory::storage_guard_activity(
                                    &leaf.guards,
                                    &mut self.fuel,
                                    |path, tag| match domains.get(path) {
                                        Some(tags) if !tags.contains(&tag) => {
                                            super::memory::GuardActivity::Inactive
                                        }
                                        Some(tags) if tags.len() == 1 => {
                                            super::memory::GuardActivity::Active
                                        }
                                        _ => super::memory::GuardActivity::Conditional,
                                    },
                                )
                                .map_err(|reason| error(source, reason))?;
                                absent = activity == super::memory::GuardActivity::Inactive;
                                if !absent {
                                    break;
                                }
                            }
                        }
                        if !absent {
                            continue;
                        }
                    }
                    let retained = if other_targets.is_some_and(|targets| targets.contains(target))
                    {
                        let Some(other_conditions) =
                            other.get(slot).and_then(|entries| entries.get(target))
                        else {
                            continue;
                        };
                        let mut common = vec![];
                        for condition in conditions {
                            self.charge_pointer_conditions(other_conditions, source)?;
                            if other_conditions.contains(condition) {
                                common.push(condition.clone());
                            }
                        }
                        common
                    } else {
                        conditions.clone()
                    };
                    if !retained.is_empty() {
                        joined
                            .entry(slot.clone())
                            .or_default()
                            .insert(target.clone(), retained);
                    }
                }
            }
        }
        Ok(joined)
    }

    fn copy_value_pointer_guards(
        &mut self,
        guards: &ValuePointerGuards,
        source: SourceRange,
    ) -> Result<ValuePointerGuards, Error> {
        for (path, entries) in guards {
            self.fuel = self
                .fuel
                .checked_sub(path.len() + 1)
                .ok_or_else(|| error(source, "pointer guard path budget exhausted"))?;
            for (target, conditions) in entries {
                self.fuel = self
                    .fuel
                    .checked_sub(target.fields.len() + 1)
                    .ok_or_else(|| error(source, "pointer guard target budget exhausted"))?;
                self.charge_pointer_conditions(conditions, source)?;
            }
        }
        Ok(guards.clone())
    }
    fn value_pointer_guards(
        &mut self,
        value: &Expr,
        flow: &Flow,
        source: SourceRange,
    ) -> Result<ValuePointerGuards, Error> {
        let Some(place) = expression_place(value) else {
            return Ok(BTreeMap::new());
        };
        self.stored_pointer_guards(&place, flow, source)
    }
    fn stored_pointer_guards(
        &mut self,
        place: &Place,
        flow: &Flow,
        source: SourceRange,
    ) -> Result<ValuePointerGuards, Error> {
        let mut result = BTreeMap::new();
        for (slot, entries) in flow
            .pointer_guards
            .range(place.clone()..)
            .take_while(|(slot, _)| slot.root == place.root)
        {
            self.fuel = self
                .fuel
                .checked_sub(slot.fields.len() + 1)
                .ok_or_else(|| error(source, "pointer guard extraction budget exhausted"))?;
            let Some(relative) = slot.fields.strip_prefix(place.fields.as_slice()) else {
                continue;
            };
            let mut projected = BTreeMap::new();
            for (target, conditions) in entries {
                self.charge_pointer_conditions(conditions, source)?;
                let guards: Vec<_> = conditions
                    .iter()
                    .filter_map(|(path, tag)| {
                        path.strip_prefix(place.fields.as_slice())
                            .map(|path| (path.to_vec(), *tag))
                    })
                    .collect();
                if !guards.is_empty() {
                    projected.insert(target.clone(), guards);
                }
            }
            if !projected.is_empty() {
                result.insert(relative.to_vec(), projected);
            }
        }
        Ok(result)
    }

    /// Qualifiers belong to the assigned value. Rebase both storage slots and
    /// tag paths under the destination, never the pointee's address.
    fn install_pointer_guards(
        &mut self,
        flow: &mut Flow,
        destination: &Place,
        guards: ValuePointerGuards,
        source: SourceRange,
    ) -> Result<(), Error> {
        for (fields, entries) in guards {
            self.fuel = self
                .fuel
                .checked_sub(
                    destination
                        .fields
                        .len()
                        .saturating_add(fields.len())
                        .saturating_add(1),
                )
                .ok_or_else(|| error(source, "pointer guard installation budget exhausted"))?;
            let mut slot = destination.clone();
            slot.fields.extend(fields);
            let mut installed = PointerTargetGuards::new();
            for (target, conditions) in entries {
                let mut rebased = Vec::with_capacity(conditions.len());
                for (fields, tag) in conditions {
                    self.fuel = self
                        .fuel
                        .checked_sub(
                            destination
                                .fields
                                .len()
                                .saturating_add(fields.len())
                                .saturating_add(1),
                        )
                        .ok_or_else(|| error(source, "pointer guard rebasing budget exhausted"))?;
                    let mut path = destination.fields.clone();
                    path.extend(fields);
                    rebased.push((path, tag));
                }
                installed.insert(target, rebased);
            }
            flow.pointer_guards.insert(slot, installed);
        }
        Ok(())
    }

    fn forget_pointer_subtree(
        &mut self,
        flow: &mut Flow,
        destination: &Place,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.fuel = self
            .fuel
            .checked_sub(flow.pointers.len())
            .ok_or_else(|| error(source, "pointer storage budget exhausted"))?;
        flow.pointers.retain(|slot, _| {
            slot.root != destination.root || !slot.fields.starts_with(&destination.fields)
        });
        self.fuel = self
            .fuel
            .checked_sub(flow.pointer_guards.len())
            .ok_or_else(|| error(source, "pointer guard invalidation budget exhausted"))?;
        flow.pointer_guards.retain(|slot, _| {
            slot.root != destination.root || !slot.fields.starts_with(&destination.fields)
        });
        Ok(())
    }
    fn replace_pointer_slot(
        &mut self,
        flow: &mut Flow,
        destination: &Place,
        pointer: Option<BTreeSet<Place>>,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.forget_pointer_subtree(flow, destination, source)?;
        if let Some(pointer) = pointer {
            self.pointer_work(&pointer, source)?;
            flow.pointers.insert(destination.clone(), pointer);
        }
        Ok(())
    }
    fn install_value_origins(
        &mut self,
        flow: &mut Flow,
        destination: &Place,
        origins: ValueOrigins,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.forget_pointer_subtree(flow, destination, source)?;
        for (path, targets) in origins {
            self.pointer_work(&targets, source)?;
            self.fuel = self
                .fuel
                .checked_sub(
                    path.len()
                        .saturating_add(destination.fields.len())
                        .saturating_add(1),
                )
                .ok_or_else(|| error(source, "value origin budget exhausted"))?;
            let mut slot = destination.clone();
            slot.fields.extend(path);
            flow.pointers.insert(slot, targets);
        }
        Ok(())
    }
    fn read_aggregate_origins(
        &mut self,
        expr: &Expr,
        pointer: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        let targets = self.evaluate_pointer_value(pointer, flow, depth + 1)?;
        if !flow.live {
            return Ok(BTreeMap::new());
        }
        let targets = targets
            .ok_or_else(|| error(expr.source, "aggregate read needs established storage"))?;
        if !super::memory::copy_readable_representation(self.package, &expr.ty, &mut self.fuel)
            .map_err(|reason| error(expr.source, reason))?
            || (self.delivery)(&expr.ty).map_err(|reason| error(expr.source, reason))?
                != Delivery::Copy
        {
            return Err(error(
                expr.source,
                "aggregate pointer read requires checked Copy permission",
            ));
        }
        let layout = super::memory::pointer_input_layout(self.package, &expr.ty, &mut self.fuel)
            .map_err(|reason| error(expr.source, reason))?;
        let mut merged: Option<(
            ValueOrigins,
            ValuePointerGuards,
            EnumDomains,
            BTreeMap<Vec<usize>, Callable>,
        )> = None;
        for target in &targets {
            flow.state.require_present(target).map_err(|reason| {
                error(
                    expr.source,
                    format!("aggregate read target unavailable: {reason:?}"),
                )
            })?;
            let actual = flow
                .types
                .get(&target.root)
                .ok_or_else(|| error(expr.source, "aggregate read target type missing"))?;
            if owned_field_type(
                self.package,
                actual,
                &target.fields,
                &mut self.fuel,
                expr.source,
            )? != expr.ty
            {
                return Err(error(expr.source, "aggregate read target type mismatch"));
            }
            let domains = self.stored_enum_domains(flow, target, expr.source)?;
            let code = self.stored_code(flow, target, expr.source)?;
            let mut guards = self.stored_pointer_guards(target, flow, expr.source)?;
            let mut origins = ValueOrigins::new();
            for leaf in &layout {
                self.fuel = self
                    .fuel
                    .checked_sub(
                        target
                            .fields
                            .len()
                            .saturating_add(leaf.storage_fields.len())
                            .saturating_add(1),
                    )
                    .ok_or_else(|| error(expr.source, "aggregate read origin budget exhausted"))?;
                let mut slot = target.clone();
                slot.fields.extend(&leaf.storage_fields);
                let activity = super::memory::storage_guard_activity(
                    &leaf.guards,
                    &mut self.fuel,
                    |path, tag| match domains.get(path) {
                        Some(tags) if !tags.contains(&tag) => {
                            super::memory::GuardActivity::Inactive
                        }
                        Some(tags) if tags.len() == 1 => super::memory::GuardActivity::Active,
                        _ => super::memory::GuardActivity::Conditional,
                    },
                )
                .map_err(|reason| error(expr.source, reason))?;
                let values = if activity == super::memory::GuardActivity::Inactive {
                    BTreeSet::new()
                } else if let Some(values) = flow.pointers.get(&slot).filter(|_| {
                    // A physical slot may already hold a premise for another
                    // variant. Acquire this guarded leaf independently.
                    leaf.guards.is_empty()
                        || !matches!(target.root.owner,
                        x if x == u64::MAX - 1 || x == u64::MAX - 2 || x == u64::MAX - 4)
                }) {
                    self.pointer_work(values, expr.source)?;
                    values.clone()
                } else if !leaf.guards.is_empty() {
                    let projection = super::memory::project_owned_storage(
                        self.package,
                        &expr.ty,
                        &leaf.input_fields,
                        &mut self.fuel,
                    )
                    .map_err(|reason| error(expr.source, reason))?;
                    let Term::Pointer(subject) = &projection.ty else {
                        return Err(error(expr.source, "aggregate pointer leaf type mismatch"));
                    };
                    match self.borrowed_guarded_contents(
                        flow,
                        target,
                        &projection.fields,
                        &leaf.guards,
                        subject,
                        expr.source,
                    )? {
                        Some(values) => {
                            self.merge_pointer_conditions(
                                guards.entry(leaf.storage_fields.clone()).or_default(),
                                &values,
                                &leaf.guards,
                                expr.source,
                            )?;
                            values
                        }
                        None => BTreeSet::from([unknown_pointer_target()]),
                    }
                } else {
                    self.borrowed_pointer_contents(flow, &slot, expr.source)?;
                    match flow.pointers.get(&slot) {
                        Some(values) => {
                            self.pointer_work(values, expr.source)?;
                            values.clone()
                        }
                        None => BTreeSet::from([unknown_pointer_target()]),
                    }
                };
                origins
                    .entry(leaf.storage_fields.clone())
                    .or_default()
                    .extend(values);
            }
            if let Some((previous, previous_guards, previous_domains, previous_code)) = &mut merged
            {
                *previous_guards = self.join_result_pointer_guards(
                    previous_guards,
                    previous,
                    &guards,
                    &origins,
                    expr.source,
                    Some((&expr.ty, previous_domains, &domains)),
                )?;
                self.join_value_origins(
                    &expr.ty,
                    previous,
                    previous_domains,
                    &origins,
                    &domains,
                    expr.source,
                )?;
                *previous_code = join_guarded_callables(
                    previous_code,
                    &code,
                    previous_domains,
                    &domains,
                    &mut self.fuel,
                    expr.source,
                )?;
                *previous_domains =
                    join_enum_domains(previous_domains, &domains, &mut self.fuel, expr.source)?;
            } else {
                merged = Some((origins, guards, domains, code));
            }
        }
        let (origins, guards, domains, code) = merged.unwrap_or_default();
        flow.value_pointer_guards = guards;
        flow.value_variants = domains;
        flow.value_callables = code;
        flow.value_callable = flow.value_callables.get(&vec![]).cloned();
        Ok(origins)
    }

    fn evaluate_value_origins(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        // Residual method values carry selected implementation evidence just
        // like named functions. Keep interface predicate checks in selection;
        // behavior-only selection would erase invocation obligations here.
        if flow.live
            && matches!(expr.kind, K::TraitMethod { .. })
            && self
                .package
                .is_some_and(|package| !package.runtime_instances.is_empty())
            && matches!(expr.ty.callable_signature(), Term::Function { parameters, .. }
                if parameters.iter().all(|parameter| parameter.sort == nir::Sort::Runtime))
        {
            let selected = self
                .method_callees
                .select(self.package.unwrap(), expr, &mut self.fuel)
                .map_err(|reason| error(expr.source, reason))?;
            return self.evaluate_value_origins(&selected, flow, depth + 1);
        }
        let place_guards = self.value_pointer_guards(expr, flow, expr.source)?;
        flow.value_pointer_guards.clear();
        flow.value_callable = None;
        flow.value_callables.clear();
        flow.value_variants.clear();
        let origins = self.evaluate_value_origins_inner(expr, flow, depth)?;
        // Pointer and aggregate result paths bypass expr_inner's post-call
        // hook. Their suspended calls still require the installer's unwind
        // plan, including a handler's lexical return.
        if flow.live
            && matches!(&expr.kind, K::Call { callee, .. }
                if matches!(callee.ty.callable_signature(), Term::Function { result, .. }
                    if matches!(result.as_ref(), Term::Computation { effects, .. } if !effects.is_empty())))
            && !self
                .report
                .effect_exits
                .iter()
                .any(|(source, _)| *source == expr.source)
        {
            self.effect_escape_checkpoint(flow, expr.source)?;
        }
        if expression_place(expr).is_some() {
            flow.value_pointer_guards = place_guards;
        } else if !matches!(
            &expr.kind,
            K::Tuple(_)
                | K::AnonymousRecord { .. }
                | K::Record { .. }
                | K::Variant { .. }
                | K::ErrorConvert { .. }
                | K::Field { .. }
                | K::AnonymousField { .. }
                | K::NewtypeConvert(_)
                | K::Snapshot(_)
                | K::Call { .. }
                | K::Closure { .. }
                | K::EffectRequest { .. }
                | K::EffectHandle { .. }
                | K::EffectComputation { .. }
                | K::EffectRun { .. }
                | K::ReadPointer(_)
                | K::If {
                    else_branch: Some(_),
                    ..
                }
                | K::Match { .. }
                | K::Block {
                    result: Some(_),
                    ..
                }
                | K::CallPreparation {
                    result: Some(_),
                    ..
                }
        ) {
            // Nested evaluation may leave operand evidence behind. Only paths
            // which explicitly transport the result can publish that evidence.
            flow.value_pointer_guards.clear();
        }
        if !flow.live {
            flow.value_pointer_guards.clear();
        }
        if matches!(expr.ty, Term::Pointer(_)) {
            flow.value_variants.clear();
        }
        if !flow.live
            || !matches!(
                completion_type(&expr.ty).callable_signature(),
                Term::Function { .. } | Term::UnsafeFunction(_)
            )
        {
            flow.value_callable = None;
        }
        if !flow.live {
            flow.value_callables.clear();
            flow.value_variants.clear();
        } else if let Some(callable) = &flow.value_callable {
            flow.value_callables.insert(vec![], callable.clone());
        }
        Ok(origins)
    }
    fn storage_inputs_are_closed(&mut self, ty: &Term, source: SourceRange) -> Result<bool, Error> {
        // Whole generic values can be transferred before their field layout is
        // known. Materialized calls are checked again with concrete inputs.
        super::memory::bound_type(ty, &mut self.fuel).map_err(|reason| error(source, reason))?;
        Ok(ty
            .rigid_parameters()
            .map_err(|reason| error(source, reason.to_string()))?
            .is_empty()
            && ty
                .substitution_parameters()
                .map_err(|reason| error(source, reason.to_string()))?
                .is_empty())
    }

    #[inline(never)]
    fn evaluate_named_pointer_aggregate(
        &mut self,
        expr: &Expr,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
        parameter_bindings: &[LocalId],
        flow: &mut Flow,
        depth: usize,
    ) -> Result<(ValueOrigins, ValuePointerGuards), Error> {
        // Preserve aggregate return origins when a static method is resolved.
        // Full selection rejects uncomposed interface predicates; behavior-only
        // selection would erase obligations. Routing through `expr` alone would
        // discard the returned field map.
        let selected;
        let callee = if matches!(callee.kind, K::TraitMethod { .. }) {
            let package = self
                .package
                .ok_or_else(|| error(expr.source, "method memory analysis requires a package"))?;
            selected = self
                .method_callees
                .select(package, callee, &mut self.fuel)
                .map_err(|reason| error(expr.source, reason))?;
            &selected
        } else {
            callee
        };
        if !matches!(
            callee.kind,
            K::Local(_) | K::Function(_) | K::FunctionInstance { .. }
        ) {
            return self
                .call_temporary_with_origins(
                    expr,
                    callee,
                    arguments,
                    parameter_bindings,
                    flow,
                    depth + 1,
                )
                .map(|(_, origins, guards)| (origins, guards));
        }
        if matches!(callee.kind, K::Local(_)) {
            // Resolve identity before arguments while retaining the observed code slot.
            if let Some(resolved) = self.indirect_named_call(expr, flow) {
                self.observe_named_callee(callee, flow)?;
                let origins = self.evaluate_value_origins(&resolved, flow, depth + 1)?;
                return Ok((origins, std::mem::take(&mut flow.value_pointer_guards)));
            }
            let K::Local(binding) = callee.kind else {
                unreachable!()
            };
            if let Some(callable) = flow
                .callables
                .get(&root(binding))
                .filter(|callable| matches!(callable.definition.kind, K::Closure { .. }))
                .cloned()
            {
                return self
                    .call_closure_with_origins(
                        &Place {
                            root: root(binding),
                            fields: vec![],
                        },
                        &callable,
                        arguments,
                        flow,
                        expr.source,
                        depth + 1,
                    )
                    .map(|(_, origins, guards)| (origins, guards));
            }
            let callable = flow
                .callables
                .get(&root(binding))
                .filter(|callable| !callable.alternatives.is_empty())
                .cloned()
                .ok_or_else(|| {
                    error(
                        expr.source,
                        "aggregate call result requires named callable provenance",
                    )
                })?;
            self.observe_named_callee(callee, flow)?;
            if !flow.live {
                return Ok((BTreeMap::new(), BTreeMap::new()));
            }
            return self
                .call_named_candidates_with_origins(
                    expr,
                    &callable,
                    arguments,
                    parameter_bindings,
                    flow,
                    depth + 1,
                )
                .map(|(_, origins, guards)| (origins, guards));
        }
        let mut prepared_callables = vec![];
        let mut prepared_enums = vec![];
        let mut prepared_borrowed = vec![];
        let (_, _, origins, guards) = self.memory_call(
            callee,
            arguments,
            parameter_bindings,
            &[],
            &mut prepared_callables,
            &mut prepared_enums,
            &mut prepared_borrowed,
            flow,
            expr.source,
            depth + 1,
        )?;
        if flow.live {
            flow.value_callable = self.record_named_call(
                callee,
                prepared_callables,
                prepared_enums,
                &prepared_borrowed,
                expr.source,
                flow,
            )?;
        }
        self.refine_absent_result_payloads(&expr.ty, &origins, &guards, flow, expr.source)?;
        Ok((origins, guards))
    }

    #[inline(never)]
    fn error_transport_origins(
        &mut self,
        expr: &Expr,
        value: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        if matches!(
            &expr.kind,
            K::ErrorConvert {
                conversion: nir::error_results::Conversion::Embed,
                ..
            }
        ) {
            if let Some(cleanup) = self.cleanup.as_deref_mut() {
                cleanup
                    .check_error_embedding(&value.ty, &expr.ty, &mut self.fuel)
                    .map_err(|e| error(expr.source, e))?;
            }
        }
        let mapping = super::memory::error_tag_mapping(expr, self.package, &mut self.fuel)
            .map_err(|e| error(expr.source, e))?;
        let origins = self.evaluate_value_origins(value, flow, depth + 1)?;
        if !flow.live {
            return Ok(origins);
        }
        let mut callables = BTreeMap::new();
        for (mut path, value) in std::mem::take(&mut flow.value_callables) {
            super::memory::remap_error_path(&mut path, &mapping, &mut self.fuel)
                .map_err(|e| error(expr.source, e))?;
            callables.insert(path, value);
        }
        flow.value_callables = callables;
        let mut domains = BTreeMap::new();
        for (mut path, mut tags) in std::mem::take(&mut flow.value_variants) {
            self.fuel = self
                .fuel
                .checked_sub(tags.len())
                .ok_or_else(|| error(expr.source, "error domain budget exhausted"))?;
            tags = tags
                .into_iter()
                .map(|tag| {
                    mapping
                        .remap_tag(&path, tag, &mut self.fuel)
                        .map_err(|e| error(expr.source, e.to_string()))
                })
                .collect::<Result<_, _>>()?;
            super::memory::remap_error_path(&mut path, &mapping, &mut self.fuel)
                .map_err(|e| error(expr.source, e))?;
            domains.insert(path, tags);
        }
        flow.value_variants = domains;
        for guards in flow.value_pointer_guards.values_mut() {
            for conditions in guards.values_mut() {
                self.charge_pointer_conditions(conditions, expr.source)?;
                for (path, tag) in conditions {
                    *tag = mapping
                        .remap_tag(path, *tag, &mut self.fuel)
                        .map_err(|e| error(expr.source, e.to_string()))?;
                }
            }
        }
        flow.value_callable = None;
        return Ok(origins);
    }

    fn evaluate_value_origins_inner(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        if !flow.live {
            return Ok(BTreeMap::new());
        }
        if self.fuel == 0 || depth > 128 {
            return Err(error(expr.source, "value origin budget exhausted"));
        }
        self.fuel -= 1;
        if let K::EffectHandle {
            computation,
            handlers,
            outer,
        } = &expr.kind
        {
            let (may_reject, return_values, loop_targets, mut result_inputs) =
                self.handler_resources(expr, handlers, flow, depth + 1)?;
            let may_return = !return_values.is_empty();
            if may_reject {
                // Providers run before any subject argument is prepared. Their
                // failure therefore unwinds the installer's current owners.
                let mut rejected = flow.clone();
                self.exit(&mut rejected, expr.source, CleanupReason::PredicateFailure)?;
            }
            if may_return && self.handler_result.is_some() {
                for value in &return_values {
                    self.check_handler_return_origins(flow, &value.origins, expr.source)?;
                }
                self.fuel = self
                    .fuel
                    .checked_sub(return_values.len())
                    .ok_or_else(|| error(expr.source, "handler return summary budget exhausted"))?;
                self.report
                    .lexical_returns
                    .extend(return_values.iter().cloned());
            }
            for mut input in self.copy_effect_result_inputs(expr.source)? {
                if outer.get(input.slot) == Some(&input.operation) {
                    input.slot = input
                        .slot
                        .checked_add(handlers.len())
                        .ok_or_else(|| error(expr.source, "effect result slot overflow"))?;
                    result_inputs.push(input);
                }
            }
            let memory_inputs = self.installed_effect_memory_inputs(
                handlers,
                outer,
                &result_inputs,
                flow,
                expr.source,
            )?;
            self.fuel = self
                .fuel
                .checked_sub(flow.effect_mutable_roots.len())
                .ok_or_else(|| {
                    error(
                        expr.source,
                        "handler capture mutation scope budget exhausted",
                    )
                })?;
            let previous_mutable = flow.effect_mutable_roots.clone();
            for handler in handlers {
                for arm in &handler.arms {
                    let captures = nir::free_bindings::collect_arm(arm, &mut self.fuel)
                        .map_err(|reason| error(expr.source, reason))?;
                    for binding in captures {
                        let id = root(binding);
                        flow.effect_mutable_roots.insert(id);
                        flow.enum_domains.remove(&id);
                    }
                }
            }
            let previous_memory = std::mem::replace(&mut self.effect_memory_inputs, memory_inputs);
            let previous_results = std::mem::replace(&mut self.effect_result_inputs, result_inputs);
            let previous = self.effect_return_enabled;
            let return_floor = self.effect_return_values.len();
            self.effect_return_values.extend(return_values);
            self.fuel = self
                .fuel
                .checked_sub(
                    self.effect_loop_targets
                        .len()
                        .saturating_add(loop_targets.len()),
                )
                .ok_or_else(|| error(expr.source, "effect loop target budget exhausted"))?;
            let previous_loops = self.effect_loop_targets.clone();
            self.effect_loop_targets
                .extend(loop_targets.iter().copied());
            for &(target, is_break) in &loop_targets {
                if self.handler_result.is_some()
                    && !self.loops.iter().any(|frame| frame.identity == target)
                {
                    self.report.lexical_loop_exits.insert((target, is_break));
                }
            }
            self.effect_return_enabled |= may_return;
            let result = (|| {
                if (may_return || !loop_targets.is_empty())
                    && handlers
                        .iter()
                        .any(|h| h.abi == nir::effects::Abi::DirectPassing)
                {
                    if self.handler_result.is_some() {
                        self.effect_escape_checkpoint(flow, expr.source)?;
                    } else {
                        if may_return {
                            self.effect_return_checkpoint(flow, expr.source, depth + 1)?;
                        }
                        self.effect_loop_checkpoints(flow, expr.source)?;
                    }
                }
                self.evaluate_value_origins(computation, flow, depth + 1)
            })();
            self.effect_return_enabled = previous;
            self.effect_return_values.truncate(return_floor);
            self.effect_loop_targets = previous_loops;
            flow.effect_mutable_roots = previous_mutable;
            self.effect_memory_inputs = previous_memory;
            self.effect_result_inputs = previous_results;
            return result;
        }
        if let K::EffectRequest {
            abi,
            signature,
            arguments,
            ..
        } = &expr.kind
        {
            return self.effect_request_resources(
                expr,
                *abi,
                signature,
                arguments,
                flow,
                depth + 1,
            );
        }
        // Transparent evaluation boundaries must run before pointer handling:
        // a run can deliver a borrowed pointer or a closure with provenance.
        match &expr.kind {
            K::EffectComputation { effects, body } => {
                // Term::computation canonicalizes an empty row to its result.
                // The explicit boundary still carries evaluation/provenance.
                if effects.is_empty()
                    && (expr.ty == body.ty || body.ty == Term::Primitive(nir::Primitive::Never))
                {
                    return self.evaluate_value_origins(body, flow, depth + 1);
                }
                let Term::Computation {
                    effects: declared,
                    result,
                } = &expr.ty
                else {
                    return Err(error(
                        expr.source,
                        "computation boundary requires a computation type",
                    ));
                };
                if effects != declared
                    || (body.ty != **result && body.ty != Term::Primitive(nir::Primitive::Never))
                {
                    return Err(error(
                        expr.source,
                        "computation boundary disagrees with its completion type",
                    ));
                }
                return self.evaluate_value_origins(body, flow, depth + 1);
            }
            K::EffectRun { computation, slots } => {
                let Term::Computation { effects, result } = &computation.ty else {
                    return Err(error(expr.source, "effect run requires a computation"));
                };
                if expr.ty != **result {
                    return Err(error(
                        expr.source,
                        "effect run disagrees with its completion type",
                    ));
                }
                let previous = self.copy_effect_result_inputs(expr.source)?;
                let mut routed = vec![];
                if slots.len() != effects.len() {
                    return Err(error(expr.source, "effect result routing arity mismatch"));
                }
                for (slot, (operation, outer_slot)) in effects.iter().zip(slots).enumerate() {
                    for input in &previous {
                        input.charge(&mut self.fuel, expr.source)?;
                        if input.slot == *outer_slot && &input.operation == operation {
                            let mut input = input.clone();
                            input.slot = slot;
                            routed.push(input);
                            break;
                        }
                    }
                }
                self.effect_result_inputs = routed;
                let result = self.evaluate_value_origins(computation, flow, depth + 1);
                self.effect_result_inputs = previous;
                return result;
            }
            _ => {}
        }
        if let K::NewtypeConvert(value) = &expr.kind {
            let package = self.package.ok_or_else(|| {
                error(expr.source, "newtype conversion requires nominal metadata")
            })?;
            let conversion = nir::newtypes::conversion(
                &package.newtypes,
                &value.ty,
                &expr.ty,
                &[],
                &mut self.fuel,
            )
            .map_err(|reason| error(expr.source, reason.to_string()))?
            .ok_or_else(|| error(expr.source, "invalid newtype conversion edge"))?;
            let origins = self.evaluate_value_origins(value, flow, depth + 1)?;
            if !flow.live {
                return Ok(origins);
            }
            // These are private representation paths, not source projections.
            // Evaluate the complete operand once before moving its evidence.
            fn remap<T>(
                values: BTreeMap<Vec<usize>, T>,
                direction: nir::newtypes::Direction,
                fuel: &mut usize,
                source: SourceRange,
            ) -> Result<BTreeMap<Vec<usize>, T>, Error> {
                let mut result = BTreeMap::new();
                for (mut path, value) in values {
                    *fuel = fuel
                        .checked_sub(path.len().saturating_add(1))
                        .ok_or_else(|| error(source, "newtype origin budget exhausted"))?;
                    match direction {
                        nir::newtypes::Direction::Wrap => path.insert(0, 0),
                        nir::newtypes::Direction::Unwrap => {
                            if path.first() != Some(&0) {
                                return Err(error(source, "invalid newtype representation origin"));
                            }
                            path.remove(0);
                        }
                    }
                    result.insert(path, value);
                }
                Ok(result)
            }
            let direction = conversion.direction();
            let origins = remap(origins, direction, &mut self.fuel, expr.source)?;
            let mut guards = remap(
                std::mem::take(&mut flow.value_pointer_guards),
                direction,
                &mut self.fuel,
                expr.source,
            )?;
            for entries in guards.values_mut() {
                for conditions in entries.values_mut() {
                    self.charge_pointer_conditions(conditions, expr.source)?;
                    for (path, _) in conditions {
                        match direction {
                            nir::newtypes::Direction::Wrap => path.insert(0, 0),
                            nir::newtypes::Direction::Unwrap => {
                                if path.first() != Some(&0) {
                                    return Err(error(
                                        expr.source,
                                        "invalid newtype pointer guard",
                                    ));
                                }
                                path.remove(0);
                            }
                        }
                    }
                }
            }
            flow.value_pointer_guards = guards;
            flow.value_callables = remap(
                std::mem::take(&mut flow.value_callables),
                direction,
                &mut self.fuel,
                expr.source,
            )?;
            flow.value_variants = remap(
                std::mem::take(&mut flow.value_variants),
                direction,
                &mut self.fuel,
                expr.source,
            )?;
            flow.value_callable = flow.value_callables.get(&vec![]).cloned();
            return Ok(origins);
        }
        if let K::ErrorConvert {
            conversion:
                nir::error_results::Conversion::Embed | nir::error_results::Conversion::Product,
            value,
        } = &expr.kind
        {
            return self.error_transport_origins(expr, value, flow, depth);
        }
        if let K::UnsafeScope(value) = &expr.kind {
            if value.ty != expr.ty {
                return Err(error(expr.source, "unsafe scope changes its body type"));
            }
            return self.evaluate_value_origins(value, flow, depth + 1);
        }
        if let K::Snapshot(value) = &expr.kind {
            if value.ty != expr.ty
                || (self.delivery)(&expr.ty).map_err(|e| error(expr.source, e))? != Delivery::Copy
            {
                return Err(error(
                    expr.source,
                    "entry snapshot requires checked Copy permission",
                ));
            }
            return self.evaluate_value_origins(value, flow, depth + 1);
        }
        if matches!(expr.kind, K::Call { .. } | K::ReadPointer(_))
            && !matches!(completion_type(&expr.ty), Term::Pointer(_))
        {
            if let Some((pointer_depth, _)) = super::memory::pointer_representation(
                self.package,
                completion_type(&expr.ty),
                &mut self.fuel,
            )
            .map_err(|reason| error(expr.source, reason))?
            {
                let targets = self.evaluate_pointer_value(expr, flow, depth + 1)?;
                flow.value_pointer_guards.clear();
                return Ok(targets
                    .map(|targets| BTreeMap::from([(vec![0; pointer_depth], targets)]))
                    .unwrap_or_default());
            }
        }
        if matches!(completion_type(&expr.ty), Term::Pointer(_)) {
            let targets = self.evaluate_pointer_value(expr, flow, depth + 1)?;
            flow.value_pointer_guards.clear();
            return Ok(targets
                .map(|targets| BTreeMap::from([(vec![], targets)]))
                .unwrap_or_default());
        }
        match &expr.kind {
            K::ReadPointer(pointer)
                if !super::memory::pointer_input_layout(self.package, &expr.ty, &mut self.fuel)
                    .map_err(|reason| error(expr.source, reason))?
                    .is_empty() =>
            {
                self.read_aggregate_origins(expr, pointer, flow, depth + 1)
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } if self.package.is_some()
                && matches!(
                    completion_type(&expr.ty),
                    Term::Tuple(_)
                        | Term::Record(_)
                        | Term::Nominal { .. }
                        | Term::NormalForm { .. }
                        | Term::Closure { .. }
                        | Term::ErrorResult { .. }
                )
                && self.storage_inputs_are_closed(completion_type(&expr.ty), expr.source)?
                && !super::memory::pointer_input_layout(
                    self.package,
                    completion_type(&expr.ty),
                    &mut self.fuel,
                )
                .map_err(|reason| error(expr.source, reason))?
                .is_empty() =>
            {
                let (origins, guards) = self.evaluate_named_pointer_aggregate(
                    expr,
                    callee,
                    arguments,
                    parameter_bindings,
                    flow,
                    depth,
                )?;
                flow.value_pointer_guards = guards;
                Ok(origins)
            }
            K::Closure { .. } => {
                flow.constructed_capture_origins.clear();
                self.expr(expr, flow, depth + 1)?;
                Ok(std::mem::take(&mut flow.constructed_capture_origins))
            }
            K::AnonymousField { .. } | K::Field { .. } if expression_place(expr).is_none() => {
                self.project_temporary(expr, flow, depth + 1)
            }
            K::Match { value, arms } => self.in_temporary_scope(
                expr.source,
                TemporaryBoundary::Match,
                flow,
                |this, flow| this.match_arms(value, arms, flow, expr.source, depth + 1),
            ),
            K::Tuple(_)
            | K::AnonymousRecord { .. }
            | K::Record { .. }
            | K::Variant { .. }
            | K::ErrorConvert { .. } => self.pending_frame(expr.source, |this| {
                let (fields, variant) = super::memory::owned_constructor(expr, &mut this.fuel)
                    .map_err(|reason| error(expr.source, reason))?;
                let mut origins = BTreeMap::new();
                let mut callables = BTreeMap::new();
                let mut domains = EnumDomains::new();
                let mut pointer_guards = ValuePointerGuards::new();
                if let Some(tag) = variant {
                    domains.insert(vec![], BTreeSet::from([tag]));
                }
                for (index, field) in fields.iter().enumerate() {
                    let children = this.evaluate_value_origins(field, flow, depth + 1)?;
                    if !flow.live {
                        return Ok(BTreeMap::new());
                    }
                    for (mut slot, mut entries) in std::mem::take(&mut flow.value_pointer_guards) {
                        this.fuel = this
                            .fuel
                            .checked_sub(slot.len().saturating_add(1))
                            .ok_or_else(|| {
                                error(expr.source, "constructed pointer guard budget exhausted")
                            })?;
                        slot.insert(0, index);
                        for conditions in entries.values_mut() {
                            this.charge_pointer_conditions(conditions, expr.source)?;
                            for (path, _) in conditions {
                                path.insert(0, index);
                            }
                        }
                        pointer_guards.insert(slot, entries);
                    }
                    for (path, tags) in std::mem::take(&mut flow.value_variants) {
                        this.fuel = this
                            .fuel
                            .checked_sub(path.len().saturating_add(tags.len()).saturating_add(3))
                            .ok_or_else(|| {
                                error(expr.source, "enum domain construction budget exhausted")
                            })?;
                        let mut nested = match variant {
                            Some(tag) => vec![VARIANT_PATH, tag, index],
                            _ => vec![index],
                        };
                        nested.extend(path);
                        domains.insert(nested, tags);
                    }
                    for (path, callable) in std::mem::take(&mut flow.value_callables) {
                        let qualifier = variant;
                        let prefix_len = if qualifier.is_some() { 3 } else { 1 };
                        this.fuel = this
                            .fuel
                            .checked_sub(path.len().saturating_add(prefix_len))
                            .ok_or_else(|| {
                                error(expr.source, "callable origin budget exhausted")
                            })?;
                        let mut nested = match qualifier {
                            Some(variant) => vec![VARIANT_PATH, variant, index],
                            None => vec![index],
                        };
                        nested.extend(path);
                        callables.insert(nested, callable);
                    }
                    this.remember_pending_origins(field, index, flow, children.clone())?;
                    for (path, targets) in children {
                        this.fuel = this
                            .fuel
                            .checked_sub(path.len().saturating_add(1))
                            .ok_or_else(|| error(expr.source, "value origin budget exhausted"))?;
                        let mut nested = vec![index];
                        nested.extend(path);
                        if let Some(tag) = variant {
                            this.pointer_work(&targets, expr.source)?;
                            let entries = pointer_guards.entry(nested.clone()).or_default();
                            for target in &targets {
                                entries
                                    .entry(target.clone())
                                    .or_default()
                                    .push((vec![], tag));
                            }
                        }
                        origins.insert(nested, targets);
                    }
                }
                flow.value_callables = callables;
                flow.value_variants = domains;
                flow.value_pointer_guards = pointer_guards;
                Ok(origins)
            }),
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let outer = flow.roots.clone();
                for statement in statements {
                    if matches!(expr.kind, K::CallPreparation { .. }) {
                        self.prepared_binding(statement, flow, depth + 1)?;
                    } else {
                        self.full_expression(statement, flow, depth + 1)?;
                    }
                }
                let origins = if let Some(result) = result {
                    self.evaluate_value_origins(result, flow, depth + 1)?
                } else {
                    flow.value_callable = None;
                    flow.value_callables.clear();
                    flow.value_variants.clear();
                    BTreeMap::new()
                };
                if flow.live {
                    if result.is_some()
                        && self.temporary_scope.boundary == TemporaryBoundary::FunctionResult
                        && self.temporary_scope.source == expr.source
                    {
                        self.finish_temporaries(flow, expr.source, self.temporary_scope_id)?;
                    }
                    let locals: Vec<_> = flow
                        .order
                        .iter()
                        .rev()
                        .filter(|id| !outer.contains(id))
                        .copied()
                        .collect();
                    self.check_scope_origins(&origins, &locals, expr.source)?;
                    for id in locals {
                        self.retire(flow, id, expr.source)?;
                    }
                }
                Ok(origins)
            }
            K::If {
                condition,
                then_branch,
                else_branch: Some(other),
            } => {
                self.in_temporary_scope(
                    condition.source,
                    TemporaryBoundary::Condition,
                    flow,
                    |this, flow| this.expr(condition, flow, depth + 1),
                )?;
                if !flow.live {
                    return Ok(BTreeMap::new());
                }
                if let Some(value) = constant_bool(condition, 0) {
                    return self.evaluate_value_origins(
                        if value { then_branch } else { other },
                        flow,
                        depth + 1,
                    );
                }
                let mut right = flow.clone();
                let mut left_origins = self.evaluate_value_origins(then_branch, flow, depth + 1)?;
                let right_origins = self.evaluate_value_origins(other, &mut right, depth + 1)?;
                match (flow.live, right.live) {
                    (true, true) => {
                        let result_guards = self.join_result_pointer_guards(
                            &flow.value_pointer_guards,
                            &left_origins,
                            &right.value_pointer_guards,
                            &right_origins,
                            expr.source,
                            Some((
                                completion_type(&expr.ty),
                                &flow.value_variants,
                                &right.value_variants,
                            )),
                        )?;
                        self.join_value_origins(
                            completion_type(&expr.ty),
                            &mut left_origins,
                            &flow.value_variants,
                            &right_origins,
                            &right.value_variants,
                            expr.source,
                        )?;
                        self.join_flow(flow, &right, expr.source)?;
                        flow.value_pointer_guards = result_guards;
                        Ok(left_origins)
                    }
                    (false, true) => {
                        *flow = right;
                        Ok(right_origins)
                    }
                    (true, false) => Ok(left_origins),
                    (false, false) => Ok(BTreeMap::new()),
                }
            }
            _ => {
                let mut origins = BTreeMap::new();
                let mut leaves = BTreeMap::new();
                if let Some(source) = expression_place(expr) {
                    for (slot, callable) in &flow.callable_fields {
                        self.fuel = self
                            .fuel
                            .checked_sub(slot.fields.len().saturating_add(1))
                            .ok_or_else(|| error(expr.source, "callable field budget exhausted"))?;
                        if slot.root == source.root && slot.fields.starts_with(&source.fields) {
                            leaves.insert(
                                slot.fields[source.fields.len()..].to_vec(),
                                callable.clone(),
                            );
                        }
                    }
                    for (slot, targets) in &flow.pointers {
                        self.fuel = self
                            .fuel
                            .checked_sub(slot.fields.len().saturating_add(1))
                            .ok_or_else(|| error(expr.source, "value origin budget exhausted"))?;
                        if slot.root == source.root && slot.fields.starts_with(&source.fields) {
                            self.pointer_work(targets, expr.source)?;
                            origins.insert(
                                slot.fields[source.fields.len()..].to_vec(),
                                targets.clone(),
                            );
                        }
                    }
                }
                self.expr(expr, flow, depth + 1)?;
                if expression_place(expr).is_some() {
                    flow.value_callables = leaves;
                } else {
                    flow.value_pointer_guards.clear();
                }
                Ok(origins)
            }
        }
    }
    // Evaluate once and return the provenance of that evaluation. In particular,
    // call results must use the actual prepared arguments, not re-evaluate a
    // callee or inspect a caller state changed by later argument effects.
    fn evaluate_pointer_value(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<Option<BTreeSet<Place>>, Error> {
        if !flow.live {
            return Ok(None);
        }
        if self.fuel == 0 || depth > 128 {
            return Err(error(expr.source, "pointer evaluation budget exhausted"));
        }
        self.fuel -= 1;
        if let Some(message) = nir::program::pointer_operation_error_in(
            expr,
            self.package.and_then(|package| package.target),
        ) {
            return Err(error(expr.source, message));
        }
        if super::memory::pointer_representation(
            self.package,
            completion_type(&expr.ty),
            &mut self.fuel,
        )
        .map_err(|reason| error(expr.source, reason))?
        .is_some()
        {
            match &expr.kind {
                K::EffectHandle { .. }
                | K::EffectRun { .. }
                | K::EffectComputation { .. }
                | K::EffectRequest { .. } => {
                    // Pointer-valued block tails enter this path directly.
                    // Preserve the effect completion's provenance instead of
                    // evaluating it as a discarded expression before retirement.
                    let mut origins = self.evaluate_value_origins(expr, flow, depth + 1)?;
                    return Ok(origins.remove(&vec![]));
                }
                K::NewtypeConvert(_) => {
                    let mut origins = self.evaluate_value_origins(expr, flow, depth + 1)?;
                    return Ok(origins.remove(&vec![]));
                }

                K::AnonymousField { .. } | K::Field { .. } if expression_place(expr).is_none() => {
                    let mut origins = self.project_temporary(expr, flow, depth + 1)?;
                    return Ok(origins.remove(&vec![]));
                }
                K::Match { value, arms } => {
                    let mut origins = self.in_temporary_scope(
                        expr.source,
                        TemporaryBoundary::Match,
                        flow,
                        |this, flow| this.match_arms(value, arms, flow, expr.source, depth + 1),
                    )?;
                    return Ok(origins.remove(&vec![]));
                }
                K::Block { statements, result } | K::CallPreparation { statements, result } => {
                    let outer = flow.roots.clone();
                    for statement in statements {
                        if matches!(expr.kind, K::CallPreparation { .. }) {
                            self.prepared_binding(statement, flow, depth + 1)?;
                        } else {
                            self.full_expression(statement, flow, depth + 1)?;
                        }
                    }
                    let targets = if let Some(result) = result {
                        self.evaluate_pointer_value(result, flow, depth + 1)?
                    } else {
                        None
                    };
                    if flow.live {
                        if result.is_some()
                            && self.temporary_scope.boundary == TemporaryBoundary::FunctionResult
                            && self.temporary_scope.source == expr.source
                        {
                            self.finish_temporaries(flow, expr.source, self.temporary_scope_id)?;
                        }
                        let locals: Vec<_> = flow
                            .order
                            .iter()
                            .rev()
                            .filter(|id| !outer.contains(id))
                            .copied()
                            .collect();
                        if let Some(targets) = &targets {
                            self.check_scope_targets(targets, &locals, expr.source)?;
                        }
                        for id in locals {
                            self.retire(flow, id, expr.source)?;
                        }
                    }
                    return Ok(targets);
                }
                K::If {
                    condition,
                    then_branch,
                    else_branch: Some(other),
                } => {
                    self.in_temporary_scope(
                        condition.source,
                        TemporaryBoundary::Condition,
                        flow,
                        |this, flow| this.expr(condition, flow, depth + 1),
                    )?;
                    if !flow.live {
                        return Ok(None);
                    }
                    if let Some(value) = constant_bool(condition, 0) {
                        return self.evaluate_pointer_value(
                            if value { then_branch } else { other },
                            flow,
                            depth + 1,
                        );
                    }
                    let mut right = flow.clone();
                    let left_targets = self.evaluate_pointer_value(then_branch, flow, depth + 1)?;
                    let right_targets =
                        self.evaluate_pointer_value(other, &mut right, depth + 1)?;
                    return match (flow.live, right.live) {
                        (true, true) => {
                            self.join_flow(flow, &right, expr.source)?;
                            match (left_targets, right_targets) {
                                (Some(mut left), Some(right)) => {
                                    self.pointer_work(&right, expr.source)?;
                                    left.extend(right);
                                    Ok(Some(left))
                                }
                                (Some(mut targets), None) | (None, Some(mut targets)) => {
                                    targets.insert(unknown_pointer_target());
                                    Ok(Some(targets))
                                }
                                (None, None) => Ok(None),
                            }
                        }
                        (false, true) => {
                            *flow = right;
                            Ok(right_targets)
                        }
                        (true, false) => Ok(left_targets),
                        (false, false) => Ok(None),
                    };
                }
                K::ReadPointer(pointer) => {
                    if (self.delivery)(&expr.ty).map_err(|e| error(expr.source, e))?
                        != Delivery::Copy
                    {
                        return Err(error(
                            expr.source,
                            "pointer read requires checked Copy permission",
                        ));
                    }
                    let (pointer_depth, _) = super::memory::pointer_representation(
                        self.package,
                        &expr.ty,
                        &mut self.fuel,
                    )
                    .map_err(|reason| error(expr.source, reason))?
                    .ok_or_else(|| error(expr.source, "pointer read representation is missing"))?;
                    let slots = self.evaluate_pointer_value(pointer, flow, depth + 1)?;
                    if !flow.live {
                        return Ok(None);
                    }
                    let slots = slots.ok_or_else(|| {
                        error(expr.source, "pointer slot provenance is not established")
                    })?;
                    let mut result = BTreeSet::new();
                    let mut known = true;
                    for slot in &slots {
                        flow.state.require_present(slot).map_err(|e| {
                            error(
                                expr.source,
                                format!("pointer read target unavailable: {e:?}"),
                            )
                        })?;
                        let mut leaf = slot.clone();
                        leaf.fields.extend(std::iter::repeat_n(0, pointer_depth));
                        self.borrowed_pointer_contents(flow, &leaf, expr.source)?;
                        if let Some(targets) = flow.pointers.get(&leaf) {
                            self.pointer_work(targets, expr.source)?;
                            result.extend(targets.iter().cloned());
                        } else {
                            known = false;
                        }
                    }
                    if !known && !result.is_empty() {
                        result.insert(unknown_pointer_target());
                    }
                    return Ok(if known || !result.is_empty() {
                        Some(result)
                    } else {
                        None
                    });
                }
                K::Call {
                    callee,
                    arguments,
                    parameter_bindings,
                } => {
                    if !matches!(
                        callee.kind,
                        K::Local(_)
                            | K::Function(_)
                            | K::FunctionInstance { .. }
                            | K::TraitMethod { .. }
                    ) {
                        return self.call_temporary(
                            expr,
                            callee,
                            arguments,
                            parameter_bindings,
                            flow,
                            depth + 1,
                        );
                    }
                    if let K::Local(binding) = callee.kind
                        && let Some(callable) = flow
                            .callables
                            .get(&root(binding))
                            .filter(|c| !c.alternatives.is_empty())
                            .cloned()
                    {
                        self.observe_named_callee(callee, flow)?;
                        if !flow.live {
                            return Ok(None);
                        }
                        return self.call_named_candidates(
                            expr,
                            &callable,
                            arguments,
                            parameter_bindings,
                            flow,
                            depth + 1,
                        );
                    }
                    if let K::Local(binding) = callee.kind
                        && let Some(callable) = flow
                            .callables
                            .get(&root(binding))
                            .filter(|c| matches!(c.definition.kind, K::Closure { .. }))
                            .cloned()
                    {
                        return self.call_closure(
                            root(binding),
                            &callable,
                            arguments,
                            flow,
                            expr.source,
                            depth + 1,
                        );
                    }
                    if let Some(resolved) = self.indirect_named_call(expr, flow) {
                        self.observe_named_callee(callee, flow)?;
                        if !flow.live {
                            return Ok(None);
                        }
                        return self.evaluate_pointer_value(&resolved, flow, depth + 1);
                    }
                    let mut prepared_callables = vec![];
                    let mut prepared_enums = vec![];
                    let mut prepared_borrowed = vec![];
                    let (result, _, _, _) = self.memory_call(
                        callee,
                        arguments,
                        parameter_bindings,
                        &[],
                        &mut prepared_callables,
                        &mut prepared_enums,
                        &mut prepared_borrowed,
                        flow,
                        expr.source,
                        depth + 1,
                    )?;
                    if flow.live {
                        self.record_named_call(
                            callee,
                            prepared_callables,
                            prepared_enums,
                            &prepared_borrowed,
                            expr.source,
                            flow,
                        )?;
                    }
                    return Ok(result);
                }
                K::PointerAnonymousField { pointer, index, .. }
                | K::PointerField { pointer, index, .. } => {
                    let result = self.evaluate_pointer_value(pointer, flow, depth + 1)?;
                    if !flow.live {
                        return Ok(None);
                    }
                    let targets = result.ok_or_else(|| {
                        error(expr.source, "field pointer provenance is not established")
                    })?;
                    self.pointer_work(&targets, expr.source)?;
                    let targets: BTreeSet<_> = targets
                        .into_iter()
                        .map(|mut place| {
                            place.fields.push(*index);
                            place
                        })
                        .collect();
                    for target in &targets {
                        flow.state.require_place(target).map_err(|e| {
                            error(
                                expr.source,
                                format!("field pointer target unavailable: {e:?}"),
                            )
                        })?;
                    }
                    return Ok(Some(targets));
                }
                _ => (),
            }
        }
        let pointer = self.pointer_initializer(expr, flow);
        self.expr(expr, flow, depth + 1)?;
        Ok(pointer)
    }
    fn pointer_initializer(&mut self, expr: &Expr, flow: &Flow) -> Option<BTreeSet<Place>> {
        if matches!(expr.ty, Term::Pointer(_)) {
            self.pointer_target(expr, flow, 0).ok()
        } else {
            None
        }
    }
    // Keep scratch reports out of the recursive expression walk's stack frame.
    fn check_observation_expression(
        &mut self,
        expr: &Expr,
        flow: &Flow,
        depth: usize,
    ) -> Result<(), Error> {
        if flow.live && self.observation_region {
            if matches!(expr.kind, K::Call { .. }) {
                // Validate with actual in-scope code identities; discard scratch
                // plans so ordinary traversal publishes execution exactly once.
                self.observation_region = false;
                let report = self.take_nested_report(expr.source)?;
                let observations = self.candidate_observations.take();
                let checked = self.observe_guard_call(expr, flow, depth + 1, true);
                self.report = report;
                self.candidate_observations = observations;
                self.observation_region = true;
                checked?;
            }
            if let K::Let { value, .. } | K::LetPattern { value, .. } = &expr.kind {
                if matches!(value.ty, Term::Function { .. } | Term::UnsafeFunction(_))
                    && !self.observation_value_type(&value.ty, expr.source)?
                {
                    return Err(error(
                        expr.source,
                        "observation region code binding requires Copy",
                    ));
                }
            }
        }
        Ok(())
    }
    fn expr(&mut self, expr: &Expr, flow: &mut Flow, depth: usize) -> Result<(), Error> {
        if flow.live && self.observation_region {
            self.check_observation_expression(expr, flow, depth)?;
        }
        flow.value_callables.clear();
        flow.value_variants.clear();
        let mut delivered_fields = None;
        let mut delivered_variants = None;
        if let Some(place) = expression_place(expr) {
            let mut leaves = BTreeMap::new();
            for (slot, value) in &flow.callable_fields {
                self.fuel = self
                    .fuel
                    .checked_sub(slot.fields.len().saturating_add(1))
                    .ok_or_else(|| error(expr.source, "callable field budget exhausted"))?;
                if slot.root == place.root && slot.fields.starts_with(&place.fields) {
                    leaves.insert(slot.fields[place.fields.len()..].to_vec(), value.clone());
                }
            }
            delivered_fields = Some(leaves);
            let mut domains = EnumDomains::new();
            if let Some(stored) = flow.enum_domains.get(&place.root) {
                for (path, tags) in stored {
                    self.fuel = self
                        .fuel
                        .checked_sub(path.len().saturating_add(tags.len()).saturating_add(1))
                        .ok_or_else(|| {
                            error(expr.source, "enum domain delivery budget exhausted")
                        })?;
                    if let Some(relative) = path.strip_prefix(place.fields.as_slice()) {
                        domains.insert(relative.to_vec(), tags.clone());
                    }
                }
            }
            delivered_variants = Some(domains);
        }
        flow.value_callable = None;
        let callable_type = matches!(
            completion_type(&expr.ty).callable_signature(),
            Term::Function { .. } | Term::UnsafeFunction(_)
        );
        let known = if flow.live && callable_type && !matches!(expr.kind, K::Closure { .. }) {
            callable_source(expr, flow, &mut self.fuel)?
        } else {
            None
        };
        self.expr_inner(expr, flow, depth)?;
        if flow.live
            && matches!(&expr.kind, K::Call { callee, .. }
            if matches!(callee.ty.callable_signature(), Term::Function { result, .. }
                if matches!(result.as_ref(), Term::Computation { effects, .. } if !effects.is_empty())))
        {
            if !self
                .report
                .effect_exits
                .iter()
                .any(|(source, _)| *source == expr.source)
            {
                self.effect_escape_checkpoint(flow, expr.source)?;
            }
        }
        if !flow.live {
            flow.value_callables.clear();
            flow.value_variants.clear();
        } else if let Some(leaves) = delivered_fields {
            flow.value_callables = leaves;
        } else if !matches!(
            expr.kind,
            K::Tuple(_)
                | K::Closure { .. }
                | K::Record { .. }
                | K::AnonymousRecord { .. }
                | K::Variant { .. }
                | K::ErrorConvert { .. }
                | K::EffectRequest { .. }
                | K::EffectHandle { .. }
                | K::EffectComputation { .. }
                | K::EffectRun { .. }
                | K::Block { .. }
                | K::CallPreparation { .. }
                | K::If { .. }
                | K::Match { .. }
                | K::Call { .. }
                | K::Field { .. }
                | K::AnonymousField { .. }
                | K::NewtypeConvert(_)
                | K::ReadPointer(_)
        ) {
            flow.value_callables.clear();
            flow.value_variants.clear();
        }
        if !flow.live {
            flow.value_variants.clear();
        } else if let Some(domains) = delivered_variants {
            flow.value_variants = domains;
        }
        if !flow.live || !callable_type {
            flow.value_callable = None;
        } else if known.is_some() && !matches!(expr.kind, K::Closure { .. }) {
            flow.value_callable = known;
        } else if !matches!(
            expr.kind,
            K::Field { .. }
                | K::AnonymousField { .. }
                | K::Closure { .. }
                | K::EffectRequest { .. }
                | K::EffectHandle { .. }
                | K::EffectComputation { .. }
                | K::EffectRun { .. }
                | K::Block { .. }
                | K::CallPreparation { .. }
                | K::If { .. }
                | K::Match { .. }
                | K::Call { .. }
                | K::NewtypeConvert(_)
                | K::ReadPointer(_)
        ) {
            flow.value_callable = None;
        }
        Ok(())
    }
    fn check_observation_write_targets(
        &mut self,
        targets: &BTreeSet<Place>,
        source: SourceRange,
    ) -> Result<(), Error> {
        if self.observation_region {
            self.fuel = self
                .fuel
                .checked_sub(targets.len().saturating_add(1))
                .ok_or_else(|| error(source, "observation write budget exhausted"))?;
            if targets.is_empty()
                || targets
                    .iter()
                    .any(|target| !self.observation_locals.contains(&target.root))
            {
                return Err(error(
                    source,
                    "observation region pointer write requires private local targets",
                ));
            }
        }
        Ok(())
    }
    // Keep the pointer update's candidate maps off the recursive expression frame.
    #[inline(never)]
    fn write_pointer(
        &mut self,
        pointer: &Expr,
        value: &Expr,
        flow: &mut Flow,
        source: SourceRange,
        depth: usize,
    ) -> Result<(), Error> {
        let target = self.evaluate_pointer_value(pointer, flow, depth + 1)?;
        if !flow.live {
            return Ok(());
        }
        let target =
            target.ok_or_else(|| error(source, "pointer target provenance is not established"))?;
        self.check_observation_write_targets(&target, source)?;
        let pointer_depth =
            super::memory::pointer_representation(self.package, &value.ty, &mut self.fuel)
                .map_err(|reason| error(source, reason))?
                .map(|(depth, _)| depth);
        let mut origins = self.evaluate_value_origins(value, flow, depth + 1)?;
        let replacement = pointer_depth.and_then(|depth| origins.remove(&vec![0; depth]));
        if flow.live {
            let aggregate_layout = if pointer_depth.is_none() {
                super::memory::pointer_input_layout(self.package, &value.ty, &mut self.fuel)
                    .map_err(|reason| error(source, reason))?
            } else {
                vec![]
            };
            if !super::memory::copy_readable_representation(self.package, &value.ty, &mut self.fuel)
                .map_err(|reason| error(source, reason))?
            {
                return Err(error(
                    source,
                    "pointer write requires a memory transfer contract",
                ));
            }
            if (self.delivery)(&value.ty).map_err(|reason| error(source, reason))? != Delivery::Copy
            {
                return Err(error(
                    source,
                    "pointer write requires checked Copy permission",
                ));
            }
            for slot in &target {
                flow.state.require_present(slot).map_err(|e| {
                    error(source, format!("pointer write target unavailable: {e:?}"))
                })?;
            }
            let exact = target.len() == 1;
            let replacement_domains =
                copy_enum_domains(&flow.value_variants, &mut self.fuel, source)?;
            for callable in flow.value_callables.values() {
                bound_callable_copy(callable, &mut self.fuel, source, 0)?;
            }
            let replacement_code = flow.value_callables.clone();
            let replacement_guards =
                self.copy_value_pointer_guards(&flow.value_pointer_guards, source)?;
            for slot in &target {
                let previous_domains = self.stored_enum_domains(flow, slot, source)?;
                let code = if exact {
                    for callable in replacement_code.values() {
                        bound_callable_copy(callable, &mut self.fuel, source, 0)?;
                    }
                    replacement_code.clone()
                } else {
                    let previous_code = self.stored_code(flow, slot, source)?;
                    join_guarded_callables(
                        &previous_code,
                        &replacement_code,
                        &previous_domains,
                        &replacement_domains,
                        &mut self.fuel,
                        source,
                    )?
                };
                let domains = if exact {
                    copy_enum_domains(&replacement_domains, &mut self.fuel, source)?
                } else {
                    let previous = self.stored_enum_domains(flow, slot, source)?;
                    join_enum_domains(&previous, &replacement_domains, &mut self.fuel, source)?
                };
                if slot.fields.is_empty() {
                    flow.callables.remove(&slot.root);
                    if let Some(callable) = code.get(&vec![]) {
                        self.store_callable(flow, slot.root, callable.clone(), source)?;
                    }
                }
                self.install_callable_fields(flow, slot, code, source)?;
                self.install_enum_domains(flow, slot, domains, source)?;
                if !aggregate_layout.is_empty() {
                    // Replace the entire value, not individual layout leaves:
                    // distinct enum alternatives can share a physical slot.
                    let (updated, guards) = if exact {
                        for targets in origins.values() {
                            self.pointer_work(targets, source)?;
                        }
                        (
                            origins.clone(),
                            self.copy_value_pointer_guards(&replacement_guards, source)?,
                        )
                    } else {
                        let mut previous = ValueOrigins::new();
                        for (stored, targets) in flow
                            .pointers
                            .range(slot.clone()..)
                            .take_while(|(stored, _)| stored.root == slot.root)
                        {
                            self.fuel = self.fuel.checked_sub(stored.fields.len() + 1).ok_or_else(
                                || error(source, "pointer replacement extraction budget exhausted"),
                            )?;
                            if let Some(path) = stored.fields.strip_prefix(slot.fields.as_slice()) {
                                self.pointer_work(targets, source)?;
                                previous.insert(path.to_vec(), targets.clone());
                            }
                        }
                        let previous_guards = self.stored_pointer_guards(slot, flow, source)?;
                        let guards = self.join_result_pointer_guards(
                            &previous_guards,
                            &previous,
                            &replacement_guards,
                            &origins,
                            source,
                            Some((&value.ty, &previous_domains, &replacement_domains)),
                        )?;
                        self.join_value_origins(
                            &value.ty,
                            &mut previous,
                            &previous_domains,
                            &origins,
                            &replacement_domains,
                            source,
                        )?;
                        (previous, guards)
                    };
                    self.install_value_origins(flow, slot, updated, source)?;
                    self.install_pointer_guards(flow, slot, guards, source)?;
                    continue;
                }
                let mut leaf = slot.clone();
                leaf.fields
                    .extend(std::iter::repeat_n(0, pointer_depth.unwrap_or(0)));
                let slot = &leaf;
                let updated = if exact {
                    if let Some(replacement) = &replacement {
                        self.pointer_work(replacement, source)?;
                    }
                    replacement.clone()
                } else {
                    match (flow.pointers.get(slot), &replacement) {
                        (Some(previous), Some(replacement)) => {
                            self.pointer_work(previous, source)?;
                            self.pointer_work(replacement, source)?;
                            let mut all = previous.clone();
                            all.extend(replacement.iter().cloned());
                            Some(all)
                        }
                        _ => None,
                    }
                };
                self.replace_pointer_slot(flow, slot, updated, source)?;
            }
        }
        Ok(())
    }
    fn expr_inner(&mut self, expr: &Expr, flow: &mut Flow, depth: usize) -> Result<(), Error> {
        if !flow.live {
            return Ok(());
        }
        if self.fuel == 0 || depth > 128 {
            return Err(error(expr.source, "move analysis budget exhausted"));
        }
        self.fuel -= 1;
        let source = expr.source;
        if let Some(message) = nir::program::pointer_operation_error_in(
            expr,
            self.package.and_then(|package| package.target),
        ) {
            return Err(error(source, message));
        }
        // Keep recursive lexical/control nodes off the large operation match
        // frame. Inline expansion adds these nodes even to short source bodies.
        match &expr.kind {
            K::Block { .. } | K::CallPreparation { .. } | K::If { .. } => {
                self.expr_control(expr, flow, depth)
            }
            _ => self.expr_operation(expr, flow, depth),
        }
    }
    #[inline(never)]
    fn expr_control(&mut self, expr: &Expr, flow: &mut Flow, depth: usize) -> Result<(), Error> {
        let source = expr.source;
        match &expr.kind {
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let outer = flow.roots.clone();
                for statement in statements {
                    if matches!(expr.kind, K::CallPreparation { .. }) {
                        self.prepared_binding(statement, flow, depth + 1)?;
                    } else {
                        self.full_expression(statement, flow, depth + 1)?;
                    }
                }
                if let Some(result) = result {
                    self.expr(result, flow, depth + 1)?;
                    if flow.live
                        && self.temporary_scope.boundary == TemporaryBoundary::FunctionResult
                        && self.temporary_scope.source == source
                    {
                        self.finish_temporaries(flow, source, self.temporary_scope_id)?;
                    }
                }
                if flow.live {
                    let locals: Vec<_> = flow
                        .order
                        .iter()
                        .rev()
                        .filter(|id| !outer.contains(id))
                        .copied()
                        .collect();
                    for id in locals {
                        self.retire(flow, id, source)?;
                    }
                }
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.in_temporary_scope(
                    condition.source,
                    TemporaryBoundary::Condition,
                    flow,
                    |this, flow| this.expr(condition, flow, depth + 1),
                )?;
                if flow.live {
                    match constant_bool(condition, 0) {
                        Some(true) => self.scope(then_branch, flow, depth + 1)?,
                        Some(false) => {
                            if let Some(other) = else_branch {
                                self.scope(other, flow, depth + 1)?;
                            }
                        }
                        _ => self.fork(then_branch, else_branch.as_deref(), flow, depth)?,
                    }
                }
            }
            _ => unreachable!("control dispatcher"),
        }
        Ok(())
    }
    #[inline(never)]
    fn expr_operation(&mut self, expr: &Expr, flow: &mut Flow, depth: usize) -> Result<(), Error> {
        let source = expr.source;
        match &expr.kind {
            K::Intrinsic { inputs, .. } => {
                nir::intrinsics::checked(expr).map_err(|_| {
                    error(
                        source,
                        "intrinsic ownership behavior requires a valid target protocol",
                    )
                })?;
                for input in inputs {
                    self.expr(input, flow, depth + 1)?;
                }
            }
            K::AddressOf(place) => {
                flow.memory_exposed = true;
                let target = expression_place(place)
                    .ok_or_else(|| error(source, "address requires a local place"))?;
                flow.state
                    .require_place(&target)
                    .map_err(|e| error(source, format!("address target unavailable: {e:?}")))?;
            }
            K::PointerAnonymousField { .. } | K::PointerField { .. } => {
                self.evaluate_pointer_value(expr, flow, depth + 1)?;
            }
            K::ReadPointer(pointer) => {
                if !super::memory::pointer_input_layout(self.package, &expr.ty, &mut self.fuel)
                    .map_err(|reason| error(source, reason))?
                    .is_empty()
                    && super::memory::pointer_representation(self.package, &expr.ty, &mut self.fuel)
                        .map_err(|reason| error(source, reason))?
                        .is_none()
                {
                    self.read_aggregate_origins(expr, pointer, flow, depth + 1)?;
                    return Ok(());
                }
                let target = self.evaluate_pointer_value(pointer, flow, depth + 1)?;
                if !flow.live {
                    return Ok(());
                }
                let target = target
                    .ok_or_else(|| error(source, "pointer target provenance is not established"))?;
                if !super::memory::readable_representation(self.package, &expr.ty, &mut self.fuel)
                    .map_err(|reason| error(source, reason))?
                {
                    return Err(error(
                        source,
                        "owned pointer read requires a memory transfer contract",
                    ));
                }
                if (self.delivery)(&expr.ty).map_err(|e| error(source, e))? != Delivery::Copy {
                    return Err(error(
                        source,
                        "pointer read requires checked Copy permission",
                    ));
                }
                let mut domains = None;
                let mut code = None;
                for target in &target {
                    flow.state.require_present(target).map_err(|e| {
                        error(source, format!("pointer read target unavailable: {e:?}"))
                    })?;
                    let next = self.stored_enum_domains(flow, target, source)?;
                    let next_code = self.stored_code(flow, target, source)?;
                    code = Some(if let Some(previous) = code {
                        join_guarded_callables(
                            &previous,
                            &next_code,
                            domains.as_ref().unwrap(),
                            &next,
                            &mut self.fuel,
                            source,
                        )?
                    } else {
                        next_code
                    });
                    domains = Some(if let Some(previous) = domains {
                        join_enum_domains(&previous, &next, &mut self.fuel, source)?
                    } else {
                        next
                    });
                }
                flow.value_variants = domains.unwrap_or_default();
                flow.value_callables = code.unwrap_or_default();
                flow.value_callable = flow.value_callables.get(&vec![]).cloned();
            }
            K::BitWritePointer {
                pointer,
                selections,
                value,
            } => {
                let Term::Pointer(storage) = &pointer.ty else {
                    return Err(error(source, "bit writeback requires a pointer"));
                };
                self.fuel = self
                    .fuel
                    .checked_sub(selections.len())
                    .ok_or_else(|| error(source, "bit selection ownership budget exhausted"))?;
                if let Some(reason) = nir::program::bit_update_error_in(
                    storage,
                    selections,
                    &value.ty,
                    &expr.ty,
                    self.package.and_then(|package| package.target),
                ) {
                    return Err(error(source, reason));
                }
                self.in_temporary_scope(
                    source,
                    TemporaryBoundary::FullExpression,
                    flow,
                    |this, flow| {
                        let target = this.evaluate_pointer_value(pointer, flow, depth + 1)?;
                        if !flow.live {
                            return Ok(());
                        }
                        let targets = target.ok_or_else(|| {
                            error(source, "pointer target provenance is not established")
                        })?;
                        this.check_observation_write_targets(&targets, source)?;
                        for input in selections
                            .iter()
                            .flat_map(nir::program::BitSelection::expressions)
                        {
                            this.expr(input, flow, depth + 1)?;
                        }
                        this.expr(value, flow, depth + 1)?;
                        if flow.live {
                            for target in targets {
                                flow.state.require_present(&target).map_err(|reason| {
                                    error(
                                        source,
                                        format!("bit writeback target unavailable: {reason:?}"),
                                    )
                                })?;
                            }
                        }
                        Ok(())
                    },
                )?;
            }
            K::WritePointer { pointer, value } => {
                self.write_pointer(pointer, value, flow, source, depth)?;
            }
            K::Local(_) | K::AnonymousField { .. } | K::Field { .. } => {
                let Some(p) = expression_place(expr) else {
                    return self.project_temporary(expr, flow, depth + 1).map(|_| ());
                };
                flow.state.require_present(&p).map_err(|reason| {
                    error(
                        source,
                        if reason == super::ownership::Error::MissingPlace {
                            format!("missing place or field shape semantics for {p}")
                        } else {
                            format!("use of unavailable value {p}")
                        },
                    )
                })?;
                if (self.delivery)(&expr.ty).map_err(|e| error(source, e))? == Delivery::Move {
                    flow.state
                        .take(&p)
                        .map_err(|e| error(source, format!("move: {e:?}")))?;
                    self.report.transfers.push((source, p.clone()));
                    self.forget_callable_fields(flow, &p, source)?;
                    if p.fields.is_empty() {
                        flow.callables.remove(&p.root);
                    }
                } else {
                    self.report.copies.push((source, p));
                }
            }
            K::AssociatedValue { .. } => self.observe_associated_value(&expr.ty, source)?,
            K::ComptimeValue(_)
            | K::Literal(_)
            | K::Unit
            | K::Function(_)
            | K::FunctionInstance { .. }
            | K::TraitMethod { .. } => (),
            K::Closure {
                parameters,
                captures,
                body,
            } => {
                let Term::Function {
                    parameters: parameter_types,
                    ..
                } = expr.ty.callable_signature()
                else {
                    return Err(error(source, "closure lacks a callable signature"));
                };
                if parameters.len() != parameter_types.len() {
                    return Err(error(source, "closure parameter count mismatch"));
                }
                let expected_captures = match &expr.ty {
                    Term::Closure { environment, .. } => environment.as_slice(),
                    Term::Function { .. } | Term::UnsafeFunction(_) => &[],
                    _ => return Err(error(source, "unsupported closure phase")),
                };
                if expected_captures.len() != captures.len() {
                    return Err(error(source, "closure capture count mismatch"));
                }
                let mut capture_origins = BTreeMap::new();
                let mut borrowed_roots = BTreeMap::new();
                let mut capture_guards = ValuePointerGuards::new();
                let mut construction = Callable {
                    choices: vec![],
                    enum_captures: Default::default(),
                    definition: expr.clone(),
                    alternatives: vec![],
                    generation: 0,
                    captures: BTreeMap::new(),
                };
                let mut inner = Flow {
                    pointer_slot_mutations: Some(self.pointer_slot_mutations(expr)?),
                    borrowed_scope: self.fresh_borrowed_scope(source)?,
                    live: true,
                    ..Default::default()
                };
                self.pending_frame(source, |this| {
                    for (slot, (capture, expected)) in
                        captures.iter().zip(expected_captures).enumerate()
                    {
                        if &capture.ty != expected {
                            return Err(error(source, "closure capture type mismatch"));
                        }
                        let lexical;
                        let value = if let Some(value) = &capture.value {
                            value.as_ref()
                        } else {
                            lexical = Expr {
                                source,
                                ty: capture.ty.clone(),
                                kind: K::Local(capture.binding),
                            };
                            &lexical
                        };
                        let callable = callable_source(value, flow, &mut this.fuel)?;
                        // Sample each capture at its own evaluation point. Earlier
                        // initializers may legitimately replace a later source.
                        let origins = this.evaluate_value_origins(value, flow, depth + 1)?;
                        if !flow.live {
                            return Ok(());
                        }
                        for (mut path, mut entries) in
                            std::mem::take(&mut flow.value_pointer_guards)
                        {
                            this.fuel = this
                                .fuel
                                .checked_sub(path.len().saturating_add(1))
                                .ok_or_else(|| {
                                    error(source, "capture pointer guard budget exhausted")
                                })?;
                            path.insert(0, slot);
                            for conditions in entries.values_mut() {
                                this.charge_pointer_conditions(conditions, source)?;
                                for (path, _) in conditions {
                                    path.insert(0, slot);
                                }
                            }
                            capture_guards.insert(path, entries);
                        }
                        let callable = flow.value_callable.clone().or(callable);
                        let fields = flow.value_callables.clone();
                        let domains =
                            copy_enum_domains(&flow.value_variants, &mut this.fuel, source)?;
                        if !domains.is_empty() {
                            construction
                                .enum_captures
                                .insert(root(capture.binding), domains.clone());
                        }

                        construction
                            .captures
                            .retain(|place, _| place.root != root(capture.binding));
                        if let Some(value) = &callable {
                            construction.captures.insert(
                                Place {
                                    root: root(capture.binding),
                                    fields: vec![],
                                },
                                value.clone(),
                            );
                        }
                        for (path, value) in fields {
                            if !path.is_empty() {
                                this.fuel = this
                                    .fuel
                                    .checked_sub(path.len().saturating_add(1))
                                    .ok_or_else(|| {
                                        error(source, "capture field budget exhausted")
                                    })?;
                                construction.captures.insert(
                                    Place {
                                        root: root(capture.binding),
                                        fields: path,
                                    },
                                    value,
                                );
                            }
                        }
                        for (path, targets) in &origins {
                            this.fuel = this
                                .fuel
                                .checked_sub(path.len().saturating_add(1))
                                .ok_or_else(|| {
                                    error(source, "closure capture origin budget exhausted")
                                })?;
                            this.pointer_work(targets, source)?;
                            let mut captured = vec![slot];
                            captured.extend(path);
                            capture_origins.insert(captured, targets.clone());
                        }
                        let input_origins = this.memory_input_targets(
                            &capture.ty,
                            &origins,
                            &flow.value_variants,
                            source,
                        )?;
                        this.remember_pending_origins(value, slot, flow, origins)?;
                        this.declare(&mut inner, capture.binding, &capture.ty, source)?;
                        let fields = this.declare_input_field_borrows(
                            &mut inner,
                            capture.binding,
                            &capture.ty,
                            &input_origins,
                            source,
                        )?;
                        for (path, targets) in fields {
                            for target in targets {
                                let pointee = inner.types.get(&target.root).ok_or_else(|| {
                                    error(source, "closure captured field type missing")
                                })?;
                                super::memory::bound_type(pointee, &mut this.fuel)
                                    .map_err(|reason| error(source, reason))?;
                                borrowed_roots.insert(
                                    target.root,
                                    BorrowedRoot::Parameter {
                                        parameter: parameters.len() + slot,
                                        input_fields: path.clone(),
                                        pointee: pointee.clone(),
                                    },
                                );
                            }
                        }
                        if let Some((_, subject)) = super::memory::pointer_representation(
                            this.package,
                            &capture.ty,
                            &mut this.fuel,
                        )
                        .map_err(|reason| error(source, reason))?
                        {
                            this.declare_closure_borrow(
                                &mut inner,
                                parameters.len() + slot,
                                capture.binding,
                                &subject,
                                source,
                            )?;
                            borrowed_roots.insert(
                                PlaceId {
                                    owner: u64::MAX - 1,
                                    local: u32::try_from(parameters.len() + slot).map_err(
                                        |_| error(source, "too many closure pointer parameters"),
                                    )?,
                                },
                                BorrowedRoot::Parameter {
                                    parameter: parameters.len() + slot,
                                    input_fields: vec![],
                                    pointee: subject,
                                },
                            );
                        }
                        this.install_enum_domains(
                            &mut inner,
                            &Place {
                                root: root(capture.binding),
                                fields: vec![],
                            },
                            domains,
                            source,
                        )?;
                        if let Some(callable) = callable {
                            this.store_callable(
                                &mut inner,
                                root(capture.binding),
                                callable,
                                source,
                            )?;
                        }
                        for (place, evidence) in &construction.captures {
                            if place.root == root(capture.binding) && !place.fields.is_empty() {
                                inner
                                    .callable_fields
                                    .insert(place.clone(), evidence.clone());
                            }
                        }
                    }
                    Ok(())
                })?;
                if !flow.live {
                    return Ok(());
                }
                let capture_roots = inner.roots.clone();
                for (slot, (binding, parameter)) in
                    parameters.iter().zip(parameter_types).enumerate()
                {
                    self.declare(&mut inner, *binding, &parameter.ty, source)?;
                    if let Some((_, subject)) = super::memory::pointer_representation(
                        self.package,
                        &parameter.ty,
                        &mut self.fuel,
                    )
                    .map_err(|reason| error(source, reason))?
                    {
                        self.declare_closure_borrow(&mut inner, slot, *binding, &subject, source)?;
                        borrowed_roots.insert(
                            PlaceId {
                                owner: u64::MAX - 1,
                                local: u32::try_from(slot).map_err(|_| {
                                    error(source, "too many closure pointer parameters")
                                })?,
                            },
                            BorrowedRoot::Parameter {
                                parameter: slot,
                                input_fields: vec![],
                                pointee: subject,
                            },
                        );
                    }
                    if self.package.is_some()
                        && self.storage_inputs_are_closed(&parameter.ty, source)?
                    {
                        // Formation has no concrete arguments. Assume each input
                        // field conditionally; invocation still establishes the
                        // actual targets and checks the full memory footprint.
                        let layout = super::memory::pointer_input_layout(
                            self.package,
                            &parameter.ty,
                            &mut self.fuel,
                        )
                        .map_err(|reason| error(source, reason))?;
                        let premises = layout
                            .into_iter()
                            .filter(|leaf| !leaf.input_fields.is_empty())
                            .map(|leaf| (leaf.input_fields, BTreeSet::new()))
                            .collect();
                        let fields = self.declare_input_field_borrows(
                            &mut inner,
                            *binding,
                            &parameter.ty,
                            &premises,
                            source,
                        )?;
                        for (path, targets) in fields {
                            for target in targets {
                                let pointee = inner.types.get(&target.root).ok_or_else(|| {
                                    error(source, "closure parameter field type missing")
                                })?;
                                super::memory::bound_type(pointee, &mut self.fuel)
                                    .map_err(|reason| error(source, reason))?;
                                borrowed_roots.insert(
                                    target.root,
                                    BorrowedRoot::Parameter {
                                        parameter: slot,
                                        input_fields: path.clone(),
                                        pointee: pointee.clone(),
                                    },
                                );
                            }
                        }
                    }
                }
                // Analyze a possible invocation independently from construction.
                // Its surviving environment remains stored; return only cleans
                // invocation-local responsibilities, never retained captures.
                let outer_report = std::mem::take(&mut self.report);
                // Formation uses the same conditional provenance as invocation.
                // It is evidence for observations, never ownership of pointees.
                self.report.borrowed_roots = borrowed_roots;
                let outer_captured_callables = std::mem::replace(
                    &mut self.captured_roots,
                    captures.iter().map(|c| root(c.binding)).collect(),
                );
                let outer_persistent =
                    std::mem::replace(&mut self.persistent_roots, capture_roots.clone());
                let outer_pending = std::mem::take(&mut self.pending);
                let outer_loops = std::mem::take(&mut self.loops);
                let outer_contract = self.exit_contract.take();
                let outer_handler = self.handler_result.take();
                let outer_effect_return = std::mem::replace(&mut self.effect_return_enabled, false);
                let outer_return_values = std::mem::take(&mut self.effect_return_values);
                let outer_effect_loops = std::mem::take(&mut self.effect_loop_targets);
                let outer_effect_memory = std::mem::take(&mut self.effect_memory_inputs);
                let outer_effect_results = std::mem::take(&mut self.effect_result_inputs);
                let outer_completions = std::mem::take(&mut self.handler_completions);
                let outer_defer = std::mem::replace(&mut self.defer_unknown_calls, true);
                let result = self.in_temporary_scope(
                    body.source,
                    TemporaryBoundary::FunctionResult,
                    &mut inner,
                    |this, inner| this.expr(body, inner, depth + 1),
                );
                self.defer_unknown_calls = outer_defer;
                self.loops = outer_loops;
                self.exit_contract = outer_contract;
                self.handler_result = outer_handler;
                self.effect_return_enabled = outer_effect_return;
                self.effect_return_values = outer_return_values;
                self.effect_loop_targets = outer_effect_loops;
                self.effect_memory_inputs = outer_effect_memory;
                self.effect_result_inputs = outer_effect_results;
                self.handler_completions = outer_completions;
                let result = result.and_then(|()| {
                    if inner.live {
                        self.exit(&mut inner, source, CleanupReason::FunctionEnd)
                    } else {
                        Ok(())
                    }
                });
                self.pending = outer_pending;
                let invocation = std::mem::replace(&mut self.report, outer_report);
                self.persistent_roots = outer_persistent;
                self.captured_roots = outer_captured_callables;
                result?;
                self.report.closures.push(ClosureReport {
                    invoked: false,
                    definition: expr.clone(),
                    source,
                    captures: captures
                        .iter()
                        .map(|capture| root(capture.binding))
                        .collect(),
                    invocation,
                });
                // Capture initializer code leaves are not fields of the newly
                // constructed callable; its environment has separate evidence.
                flow.value_callables.clear();
                flow.value_variants =
                    self.capture_storage_domains(captures, &construction.enum_captures, source)?;
                flow.value_callable = Some(construction);
                flow.constructed_capture_origins = capture_origins;
                flow.value_pointer_guards = capture_guards;
            }
            K::Let { .. } => {
                self.in_temporary_scope(
                    source,
                    TemporaryBoundary::FullExpression,
                    flow,
                    |this, flow| this.prepared_binding(expr, flow, depth),
                )?;
            }
            K::Match { value, arms } => {
                self.in_temporary_scope(source, TemporaryBoundary::Match, flow, |this, flow| {
                    this.match_arms(value, arms, flow, source, depth + 1)
                        .map(|_| ())
                })?
            }
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => self.in_temporary_scope(
                source,
                TemporaryBoundary::FullExpression,
                flow,
                |this, flow| {
                    this.let_pattern(
                        pattern,
                        value,
                        guard.as_deref(),
                        otherwise.as_deref(),
                        flow,
                        source,
                        depth + 1,
                    )
                },
            )?,
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
                let reason = match &expr.kind {
                    K::BitAssign { start, end, .. } => nir::program::bit_writeback_error_in(
                        storage_type,
                        *start,
                        *end,
                        &value.ty,
                        &expr.ty,
                        self.package.and_then(|package| package.target),
                    ),
                    K::BitUpdate { selections, .. } => {
                        self.fuel = self.fuel.checked_sub(selections.len()).ok_or_else(|| {
                            error(source, "bit selection ownership budget exhausted")
                        })?;
                        nir::program::bit_update_error_in(
                            storage_type,
                            selections,
                            &value.ty,
                            &expr.ty,
                            self.package.and_then(|package| package.target),
                        )
                    }
                    _ => unreachable!(),
                };
                if let Some(reason) = reason {
                    return Err(error(source, reason));
                }
                let mut actual = flow
                    .types
                    .get(&root(target.root))
                    .cloned()
                    .ok_or_else(|| error(source, "bit writeback root is not in scope"))?;
                for (declaration, index) in &target.fields {
                    if !match (&actual, declaration) {
                        (
                            Term::Nominal {
                                declaration: owner, ..
                            },
                            nir::program::FieldOwner::Nominal(id),
                        ) => owner == id,
                        (Term::Record(_), nir::program::FieldOwner::Anonymous) => true,
                        _ => false,
                    } {
                        return Err(error(source, "bit writeback field declaration mismatch"));
                    }
                    actual = self.field_type(&actual, &[*index], source)?;
                }
                if actual != *storage_type {
                    return Err(error(source, "bit writeback storage type mismatch"));
                }
                let destination = Place {
                    root: root(target.root),
                    fields: target.fields.iter().map(|(_, index)| *index).collect(),
                };
                self.in_temporary_scope(
                    source,
                    TemporaryBoundary::FullExpression,
                    flow,
                    |this, flow| {
                        if let K::BitUpdate { selections, .. } = &expr.kind {
                            for input in selections
                                .iter()
                                .flat_map(nir::program::BitSelection::expressions)
                            {
                                this.expr(input, flow, depth + 1)?;
                            }
                        }
                        // Preserved bits are read after the RHS finishes. A RHS may
                        // replace the destination, move its owner, or exit entirely.
                        this.expr(value, flow, depth + 1)?;
                        if flow.live {
                            flow.state.require_present(&destination).map_err(|reason| {
                                error(
                                    source,
                                    format!("bit writeback target unavailable: {reason:?}"),
                                )
                            })?;
                        }
                        // This updates scalar contents only. It neither initializes
                        // an absent owner nor replaces ownership/alias identities.
                        Ok(())
                    },
                )?;
            }
            K::Assign {
                place: target,
                value,
            } => {
                if self.captured_roots.contains(&root(target.root))
                    && !matches!(
                        value.ty,
                        Term::Function { .. } | Term::UnsafeFunction(_) | Term::Closure { .. }
                    )
                    && !super::memory::capture_pointer_replacement_layout(
                        self.package,
                        &value.ty,
                        &mut self.fuel,
                    )
                    .map_err(|reason| {
                        error(
                            source,
                            format!(
                                "captured write requires an established memory contract: {reason}"
                            ),
                        )
                    })?
                {
                    super::memory::require_pointer_free_owned(
                        self.package,
                        &value.ty,
                        &mut self.fuel,
                        0,
                    )
                    .map_err(|reason| {
                        error(
                            source,
                            format!(
                                "captured write requires an established memory contract: {reason}"
                            ),
                        )
                    })?;
                }

                let callable = callable_source(value, flow, &mut self.fuel)?;
                // RHS completes before retiring the previous destination. A
                // self-move therefore does not schedule destruction of the RHS.
                self.in_temporary_scope(
                    source,
                    TemporaryBoundary::FullExpression,
                    flow,
                    |this, flow| {
                        let origins = this.evaluate_value_origins(value, flow, depth + 1)?;
                        let pointer_guards =
                            this.copy_value_pointer_guards(&flow.value_pointer_guards, source)?;
                        let callable = flow.value_callable.clone().or(callable);
                        if flow.live {
                            if target.fields.is_empty() {
                                flow.callables.remove(&root(target.root));
                                if let Some(callable) = callable {
                                    this.store_callable(flow, root(target.root), callable, source)?;
                                }
                            }
                            let destination = Place {
                                root: root(target.root),
                                fields: target.fields.iter().map(|(_, index)| *index).collect(),
                            };
                            let (variants, domains) =
                                this.cleanup_variants(flow, &destination, source)?;
                            if let Some(action) = this.owned_action(
                                &flow.state,
                                destination.clone(),
                                &value.ty,
                                super::drop::PointerMemory {
                                    types: &flow.types,
                                    slots: &flow.pointers,
                                    enum_domains: &flow.enum_domains,
                                    pointer_conditions: &flow.pointer_guards,
                                    retired: &[],
                                },
                                &variants,
                                &domains,
                                source,
                            )? {
                                this.report.cleanup_sites.push(CleanupSite {
                                    default_origin: None,
                                    result_origin: None,
                                    interface_precondition: false,
                                    interface_postcondition: false,
                                    source,
                                    reason: CleanupReason::Replacement,
                                    actions: vec![action],
                                });
                            }
                            let leaves = flow.value_callables.clone();
                            this.install_callable_fields(flow, &destination, leaves, source)?;
                            this.install_enum_domains(
                                flow,
                                &destination,
                                flow.value_variants.clone(),
                                source,
                            )?;
                            this.install_value_origins(flow, &destination, origins, source)?;
                            this.install_pointer_guards(
                                flow,
                                &destination,
                                pointer_guards,
                                source,
                            )?;
                            for (place, availability) in flow
                                .state
                                .replace_with_budget(&destination, &mut this.fuel)
                                .map_err(|e| error(source, format!("assignment: {e:?}")))?
                                .into_iter()
                                .rev()
                            {
                                this.report.cleanups.push(Cleanup {
                                    source,
                                    place,
                                    availability,
                                });
                            }
                        }
                        Ok(())
                    },
                )?;
            }
            K::Loop { identity, body } => {
                self.loop_body(*identity, body, flow, source, depth + 1)?
            }
            K::Break(identity) => self.loop_edge(*identity, true, flow, source)?,
            K::Continue(identity) => self.loop_edge(*identity, false, flow, source)?,
            K::Return(value) => {
                let origins = if let Some(value) = value {
                    self.in_temporary_scope(
                        source,
                        TemporaryBoundary::ReturnValue,
                        flow,
                        |this, flow| this.evaluate_value_origins(value, flow, depth + 1),
                    )?
                } else {
                    BTreeMap::new()
                };
                if flow.live {
                    let result_callable = flow.value_callable.clone();
                    let result_fields = flow.value_callables.clone();
                    let result_variants = flow.value_variants.clone();
                    if self.handler_result.is_some() {
                        if let Some(value) = value {
                            self.check_handler_return_layout(&value.ty, flow, &origins, source)?;
                        }
                        self.check_return_origins(flow, &origins, source)?;
                        self.check_handler_return_origins(flow, &origins, source)?;
                        let callable = flow
                            .value_callable
                            .as_ref()
                            .map(|value| export_callable_evidence(value, &mut self.fuel, 0))
                            .transpose()?;
                        let mut fields = BTreeMap::new();
                        for (path, value) in &flow.value_callables {
                            if path.is_empty() {
                                continue;
                            }
                            self.fuel = self
                                .fuel
                                .checked_sub(path.len().saturating_add(1))
                                .ok_or_else(|| {
                                    error(source, "handler return field budget exhausted")
                                })?;
                            fields.insert(
                                path.clone(),
                                export_callable_evidence(value, &mut self.fuel, 0)?,
                            );
                        }
                        let variants =
                            copy_enum_domains(&flow.value_variants, &mut self.fuel, source)?;
                        let pointer_guards =
                            self.copy_value_pointer_guards(&flow.value_pointer_guards, source)?;
                        self.report
                            .lexical_returns
                            .push(std::sync::Arc::new(HandlerReturnValue {
                                source,
                                origins,
                                pointer_guards,
                                callable,
                                fields,
                                variants,
                            }));
                    } else {
                        self.check_function_result(flow, origins, source, depth + 1)?;
                    }
                    if flow.live {
                        flow.value_callable = result_callable;
                        flow.value_callables = result_fields;
                        flow.value_variants = result_variants;
                    }
                }
                if flow.live {
                    self.exit(flow, source, CleanupReason::Return)?;
                }
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } => {
                if self.defer_unknown_calls
                    && !matches!(callee.kind, K::Local(_))
                    && expression_place(callee).is_some()
                    && callable_source(callee, flow, &mut self.fuel)?.is_none()
                {
                    self.expr(callee, flow, depth + 1)?;
                    self.prepare_values(
                        source,
                        arguments.iter().map(|a| (a.parameter, &a.value)),
                        flow,
                        depth + 1,
                    )?;
                    self.report.deferred_calls.push(source);
                    flow.value_callable = None;
                    flow.value_callables.clear();
                    flow.value_variants.clear();
                    return Ok(());
                }
                if !matches!(
                    callee.kind,
                    K::Local(_)
                        | K::Function(_)
                        | K::FunctionInstance { .. }
                        | K::TraitMethod { .. }
                ) {
                    return self
                        .call_temporary(
                            expr,
                            callee,
                            arguments,
                            parameter_bindings,
                            flow,
                            depth + 1,
                        )
                        .map(|_| ());
                }
                if let K::Local(binding) = callee.kind
                    && let Some(callable) = flow
                        .callables
                        .get(&root(binding))
                        .filter(|c| !c.alternatives.is_empty())
                        .cloned()
                {
                    self.observe_named_callee(callee, flow)?;
                    if !flow.live {
                        return Ok(());
                    }
                    return self
                        .call_named_candidates(
                            expr,
                            &callable,
                            arguments,
                            parameter_bindings,
                            flow,
                            depth + 1,
                        )
                        .map(|_| ());
                }
                if let Some(resolved) = self.indirect_named_call(expr, flow) {
                    self.observe_named_callee(callee, flow)?;
                    if !flow.live {
                        return Ok(());
                    }
                    return self.expr(&resolved, flow, depth + 1);
                }
                if let K::Local(binding) = callee.kind
                    && let Some(callable) = flow
                        .callables
                        .get(&root(binding))
                        .filter(|c| matches!(c.definition.kind, K::Closure { .. }))
                        .cloned()
                {
                    return self
                        .call_closure(root(binding), &callable, arguments, flow, source, depth + 1)
                        .map(|_| ());
                }
                let mut pointer_arguments = false;
                for argument in arguments {
                    // An opaque aggregate local may already carry pointers even
                    // when evaluating it contains no address-taking expression.
                    if self.package.is_some()
                        && !matches!(
                            argument.value.ty,
                            Term::Function { .. }
                                | Term::UnsafeFunction(_)
                                | Term::Closure { .. }
                                | Term::Rigid { .. }
                                | Term::Bound { .. }
                                | Term::Hole { .. }
                                | Term::Runtime(_)
                                | Term::AssociatedType { .. }
                        )
                        && self.storage_inputs_are_closed(&argument.value.ty, source)?
                    {
                        pointer_arguments |= !super::memory::pointer_input_layout(
                            self.package,
                            &argument.value.ty,
                            &mut self.fuel,
                        )
                        .map_err(|reason| error(source, reason))?
                        .is_empty();
                    }
                    // A pointer constructed inside a record/block is just as
                    // observable as a pointer-valued root argument. Inspect
                    // evaluation structure without evaluating it twice or
                    // treating deferred closure bodies as argument effects.
                    nir::visit::expression_in_scope(
                        &argument.value,
                        &mut self.fuel,
                        |node, fuel| {
                            if !pointer_arguments {
                                pointer_arguments = super::memory::pointer_representation(
                                    self.package,
                                    &node.ty,
                                    fuel,
                                )?
                                .is_some();
                            }
                            Ok(())
                        },
                    )
                    .map_err(|reason| error(source, reason))?;
                }
                if matches!(callee.kind, K::TraitMethod { .. })
                    && (flow.memory_exposed || pointer_arguments)
                {
                    let package = self.package.ok_or_else(|| {
                        error(source, "method memory analysis requires a package")
                    })?;
                    let selected = self
                        .method_callees
                        .select(package, callee, &mut self.fuel)
                        .map_err(|reason| error(source, reason))?;
                    let resolved = Expr {
                        source,
                        ty: expr.ty.clone(),
                        kind: K::Call {
                            callee: Box::new(selected),
                            arguments: arguments.clone(),
                            parameter_bindings: parameter_bindings.clone(),
                        },
                    };
                    return self.expr(&resolved, flow, depth + 1);
                }
                if flow.memory_exposed || pointer_arguments {
                    if matches!(callee.kind, K::Function(_) | K::FunctionInstance { .. }) {
                        let mut prepared_callables = vec![];
                        let mut prepared_enums = vec![];
                        let mut prepared_borrowed = vec![];
                        let result = self
                            .memory_call(
                                callee,
                                arguments,
                                parameter_bindings,
                                &[],
                                &mut prepared_callables,
                                &mut prepared_enums,
                                &mut prepared_borrowed,
                                flow,
                                source,
                                depth + 1,
                            )
                            .map(|_| ());
                        result?;
                        flow.value_callable = if flow.live {
                            self.record_named_call(
                                callee,
                                prepared_callables,
                                prepared_enums,
                                &prepared_borrowed,
                                source,
                                flow,
                            )?
                        } else {
                            None
                        };
                        return Ok(());
                    }
                }
                if self.defer_unknown_calls
                    && matches!(callee.kind, K::Local(_))
                    && matches!(
                        callee.ty.callable_signature(),
                        Term::Function { .. } | Term::UnsafeFunction(_)
                    )
                {
                    // Construction does not execute this body. Defer the unknown
                    // callback's state transform; actual calls are reanalyzed
                    // with argument provenance and cannot use this evidence.
                    self.prepare_values(
                        source,
                        arguments.iter().map(|a| (a.parameter, &a.value)),
                        flow,
                        depth + 1,
                    )?;
                    self.report.deferred_calls.push(source);
                    flow.value_callable = None;
                    flow.value_callables.clear();
                    flow.value_variants.clear();
                    return Ok(());
                }
                if flow.memory_exposed || pointer_arguments {
                    return Err(error(
                        source,
                        "call with pointer dependencies requires a memory contract",
                    ));
                }
                if let K::Local(binding) = &callee.kind {
                    if let Some(definition) = flow.callables.get(&root(*binding)).cloned() {
                        return self
                            .call_closure(
                                root(*binding),
                                &definition,
                                arguments,
                                flow,
                                source,
                                depth + 1,
                            )
                            .map(|_| ());
                    }
                    // Moving a closure removes its callable evidence. Report
                    // the unavailable owner before diagnosing unknown code;
                    // known, partially consumed closures retain their own
                    // per-capture invocation checks above.
                    let place = Place {
                        root: root(*binding),
                        fields: vec![],
                    };
                    flow.state.require_present(&place).map_err(|reason| {
                        error(
                            source,
                            format!("call of unavailable value {place}: {reason:?}"),
                        )
                    })?;
                }
                if matches!(callee.ty, Term::Closure { .. })
                    || matches!(callee.kind, K::Closure { .. })
                {
                    return Err(error(
                        source,
                        "closure environment state transfer requires a known live local closure",
                    ));
                }
                self.expr(callee, flow, depth + 1)?;
                if !flow.live {
                    return Ok(());
                }
                if arguments.iter().any(|a| a.is_default) {
                    self.prepare_defaults(
                        callee,
                        arguments,
                        parameter_bindings,
                        flow,
                        source,
                        depth + 1,
                    )?;
                } else {
                    let mut callables = vec![];
                    let mut enums = vec![];
                    self.pending_frame(source, |this| {
                        for argument in arguments {
                            let callable = callable_source(&argument.value, flow, &mut this.fuel)?;
                            let origins =
                                this.evaluate_value_origins(&argument.value, flow, depth + 1)?;
                            let callable = flow.value_callable.clone().or(callable);
                            if !flow.live {
                                return Ok(());
                            }
                            record_enum_parameter(
                                &mut enums,
                                argument.parameter,
                                &flow.value_variants,
                                &mut this.fuel,
                                source,
                            )?;
                            this.remember_pending_origins(
                                &argument.value,
                                argument.parameter,
                                flow,
                                origins,
                            )?;
                            if let Some(callable) = callable {
                                callables.push(export_callable_parameter(
                                    argument.parameter,
                                    &callable,
                                    &mut this.fuel,
                                )?);
                            }
                            for (path, value) in &flow.value_callables {
                                if !path.is_empty() {
                                    let mut input = export_callable_parameter(
                                        argument.parameter,
                                        value,
                                        &mut this.fuel,
                                    )?;
                                    input.fields = path.clone();
                                    callables.push(input);
                                }
                            }
                        }
                        this.check_interface_preparation(callee, flow, source)
                    })?;
                    if flow.live {
                        flow.value_callable =
                            self.record_named_call(callee, callables, enums, &[], source, flow)?;
                    }
                }
                if flow.live
                    && self
                        .call_may_reject(callee)
                        .map_err(|reason| error(source, reason))?
                {
                    // Complete arguments already belong to the callee. Only
                    // caller-owned positions and enclosing preparation survive.
                    let mut failed = flow.clone();
                    self.exit(&mut failed, source, CleanupReason::PredicateFailure)?;
                }
                if flow.live && flow.memory_exposed {
                    return Err(error(
                        source,
                        "call with pointer dependencies requires a memory contract",
                    ));
                }
                self.check_interface_result(callee, expr, flow)?;
            }
            K::Tuple(_)
            | K::AnonymousRecord { .. }
            | K::Record { .. }
            | K::Variant { .. }
            | K::ErrorConvert { .. } => {
                self.evaluate_value_origins(expr, flow, depth + 1)?;
            }
            K::EffectHandle { .. } => {
                self.evaluate_value_origins(expr, flow, depth + 1)?;
            }
            K::EffectResume(value) => {
                let result = self
                    .handler_result
                    .as_ref()
                    .ok_or_else(|| error(source, "resume requires a handler resource context"))?;
                if value.ty != *result && value.ty != Term::Primitive(nir::Primitive::Never) {
                    return Err(error(source, "handler resume result type mismatch"));
                }
                let origins = self.in_temporary_scope(
                    source,
                    TemporaryBoundary::ReturnValue,
                    flow,
                    |this, flow| this.evaluate_value_origins(value, flow, depth + 1),
                )?;
                if flow.live {
                    self.check_return_origins(flow, &origins, source)?;
                    self.capture_handler_completion(flow, &origins, source)?;
                    self.exit(flow, source, CleanupReason::EffectResume)?;
                }
            }
            K::EffectRequest {
                abi,
                signature,
                arguments,
                ..
            } => {
                self.effect_request_resources(expr, *abi, signature, arguments, flow, depth + 1)?;
            }
            K::EffectComputation { .. }
            | K::EffectRun { .. }
            | K::Snapshot(_)
            | K::NewtypeConvert(_) => {
                self.evaluate_value_origins(expr, flow, depth + 1)?;
            }
            K::RequirePredicate(condition)
            | K::EnsurePredicate(condition)
            | K::CheckPredicate(condition) => {
                if expr.ty != Term::Primitive(nir::Primitive::Void)
                    || !nir::program::checkable_predicate(condition)
                {
                    return Err(error(source, "invalid runtime predicate check"));
                }
                self.check_contract_predicate(condition, flow, depth + 1)?;
            }
            K::UnsafeScope(value) => self.expr(value, flow, depth + 1)?,
            K::Discard(value) => {
                let origins = self.evaluate_value_origins(value, flow, depth + 1)?;
                if flow.live {
                    self.temporary(value, origins, flow)?;
                }
            }
            K::EnumName(value) => {
                if let Some(place) = expression_place(value) {
                    flow.state.require_present(&place).map_err(|reason| {
                        error(
                            source,
                            format!("enum name requires an available value: {reason:?}"),
                        )
                    })?;
                } else {
                    let origins = self.evaluate_value_origins(value, flow, depth + 1)?;
                    if flow.live {
                        self.temporary(value, origins, flow)?;
                    }
                }
            }
            K::Not(value)
            | K::IntegerNegate(value)
            | K::FloatNegate(value)
            | K::BitConvert(value)
            | K::TakeBits(value)
            | K::SymbolName(value)
            | K::BitSlice { value, .. } => self.expr(value, flow, depth + 1)?,
            K::BitWindow { value, start, end } => {
                self.expr(value, flow, depth + 1)?;
                self.expr(start, flow, depth + 1)?;
                self.expr(end, flow, depth + 1)?;
            }
            K::TemplateEqual(left, right) => {
                self.observe_template_identity(left, flow)?;
                self.observe_template_identity(right, flow)?;
            }
            K::IntegerBinary { left, right, .. }
            | K::FloatBinary { left, right, .. }
            | K::ScalarEqual(left, right)
            | K::ScalarLess(left, right)
            | K::ScalarLessEqual(left, right)
            | K::ScalarGreater(left, right)
            | K::ScalarGreaterEqual(left, right)
            | K::BitEqual(left, right)
            | K::BitConcat(left, right)
            | K::BitIndex(left, right) => {
                self.expr(left, flow, depth + 1)?;
                self.expr(right, flow, depth + 1)?;
            }
            K::And(left, right) | K::Or(left, right) => {
                self.expr(left, flow, depth + 1)?;
                if flow.live && expr.ty != Term::Primitive(nir::Primitive::Bool) {
                    // BitVec and/or always evaluate both operands. In particular,
                    // RHS ownership transfers must not become conditional.
                    self.scope(right, flow, depth + 1)?;
                } else if flow.live {
                    let skip = constant_bool(left, 0).map(|value| {
                        if matches!(expr.kind, K::And(..)) {
                            !value
                        } else {
                            value
                        }
                    });
                    match skip {
                        Some(true) => (),
                        Some(false) => self.scope(right, flow, depth + 1)?,
                        None => self.fork(right, None, flow, depth)?,
                    }
                }
            }
            _ => {
                return Err(error(
                    source,
                    "operation requires additional move/observation semantics",
                ));
            }
        }
        Ok(())
    }
}
/// `delivery` must supply checked representation-copy permission, not merely
/// the existence of a Copy impl. This analysis alone does not discharge cleanup,
/// dependency, observation, or alias obligations and is not a safety certificate.
pub fn analyze(
    function: &Function,
    delivery: impl FnMut(&Term) -> Result<Delivery, String>,
    fuel: usize,
) -> Result<Report, Error> {
    analyze_with_shapes(
        None,
        function,
        delivery,
        |_| Ok(Shape::Value),
        fuel,
        None,
        &[],
        &[],
        &[],
        vec![],
        vec![],
        &[],
    )
}

fn analyze_with_shapes<'a>(
    package: Option<&'a Package>,
    function: &'a Function,
    delivery: impl FnMut(&Term) -> Result<Delivery, String>,
    shape: impl FnMut(&Term) -> Result<Shape, String>,
    fuel: usize,
    cleanup: Option<&'a mut dyn CleanupPolicy>,
    borrowed_parameters: &[(usize, Term)],
    callable_parameters: &[CallableParameter],
    enum_parameters: &[EnumParameter],
    return_stack: Vec<(nir::DeclarationId, Vec<Term>)>,
    effect_result_inputs: Vec<EffectResultInput>,
    borrowed_fields: &[BorrowedField],
) -> Result<Report, Error> {
    analyze_with_shapes_and_borrowed_code(
        package,
        function,
        delivery,
        shape,
        fuel,
        cleanup,
        borrowed_parameters,
        callable_parameters,
        enum_parameters,
        return_stack,
        effect_result_inputs,
        borrowed_fields,
        &[],
        &[],
    )
}

fn analyze_with_shapes_and_borrowed_code<'a>(
    package: Option<&'a Package>,
    function: &'a Function,
    delivery: impl FnMut(&Term) -> Result<Delivery, String>,
    shape: impl FnMut(&Term) -> Result<Shape, String>,
    fuel: usize,
    cleanup: Option<&'a mut dyn CleanupPolicy>,
    borrowed_parameters: &[(usize, Term)],
    callable_parameters: &[CallableParameter],
    enum_parameters: &[EnumParameter],
    return_stack: Vec<(nir::DeclarationId, Vec<Term>)>,
    effect_result_inputs: Vec<EffectResultInput>,
    borrowed_fields: &[BorrowedField],
    borrowed_code: &[BorrowedCode],
    trivial_cleanup_types: &[Term],
) -> Result<Report, Error> {
    // Callers supply many distinct closure types. Keep the large recursive
    // analyzer shared instead of monomorphizing it for every policy closure.
    let mut delivery = delivery;
    let mut shape = shape;
    let delivery: &mut dyn FnMut(&Term) -> Result<Delivery, String> = &mut delivery;
    let shape: &mut dyn FnMut(&Term) -> Result<Shape, String> = &mut shape;
    let source = function.body.source;
    if function.phase != FunctionPhase::Runtime {
        return Err(error(source, "move analysis requires a runtime function"));
    }
    let initial_fuel = fuel;
    let mut fuel = fuel;
    for ty in trivial_cleanup_types {
        super::memory::bound_type(ty, &mut fuel).map_err(|e| error(source, e))?;
    }
    let mut effect_memory_inputs = Vec::new();
    for input in &effect_result_inputs {
        effect_memory_inputs.push(input.memory_input(&mut fuel, source)?);
    }
    let mut analyzer = Analyzer {
        trivial_cleanup_types: trivial_cleanup_types.to_vec(),
        cleanup,
        package,
        exit_contract: Some(function),
        handler_result: None,
        effect_return_enabled: false,
        effect_return_values: vec![],
        effect_loop_targets: BTreeSet::new(),
        effect_memory_inputs,
        effect_result_inputs,
        handler_completions: vec![],
        rejection: super::rejection::Cache::new(package),
        delivery,
        shape,
        fuel,
        report: Report::default(),
        candidate_observations: None,
        observation_region: false,
        observation_locals: BTreeSet::new(),
        method_callees: Default::default(),
        persistent_roots: BTreeSet::new(),
        captured_roots: BTreeSet::new(),
        defer_unknown_calls: false,
        return_stack,
        next_generation: 0,
        next_borrowed_pointer: 0,
        next_borrowed_scope: 1,
        borrowed_pointees: BTreeMap::new(),
        next_sequence: 1,
        temporary_scope_id: 0,
        temporary_scope: TemporaryScope {
            source,
            boundary: TemporaryBoundary::FunctionResult,
        },
        loops: vec![],
        pending: vec![],
    };
    let mut flow = Flow {
        live: true,
        ..Default::default()
    };
    let Term::Function { parameters, .. } = function.signature.callable_signature() else {
        return Err(error(source, "missing callable signature"));
    };
    if parameters.len() != function.parameters.len() {
        return Err(error(source, "parameter signature mismatch"));
    }
    for (parameter, signature) in function.parameters.iter().zip(parameters) {
        analyzer.declare(&mut flow, *parameter, &signature.ty, source)?;
    }
    let mut callable_slots = BTreeSet::new();
    for input in callable_parameters {
        analyzer.fuel = analyzer
            .fuel
            .checked_sub(input.fields.len().saturating_add(1))
            .ok_or_else(|| error(source, "callable parameter path budget exhausted"))?;
        if !callable_slots.insert((input.parameter, input.fields.clone())) {
            return Err(error(source, "duplicate callable parameter"));
        }
        let binding = function
            .parameters
            .get(input.parameter)
            .ok_or_else(|| error(source, "callable parameter is missing"))?;
        let callable = import_callable_evidence(
            analyzer.package,
            &input.definition,
            &input.alternatives,
            &input.captures,
            &input.enum_captures,
            &input.choices,
            &owned_field_type(
                analyzer.package,
                &parameters[input.parameter].ty,
                &input.fields,
                &mut analyzer.fuel,
                source,
            )?,
            &mut analyzer.fuel,
            0,
        )?;
        if input.fields.is_empty() {
            analyzer.store_callable(&mut flow, root(*binding), callable, source)?;
        } else {
            analyzer.install_callable_fields(
                &mut flow,
                &Place {
                    root: root(*binding),
                    fields: input.fields.clone(),
                },
                BTreeMap::from([(vec![], callable)]),
                source,
            )?;
        }
    }
    analyzer.report.callable_parameters = callable_parameters.to_vec();
    let mut enum_slots = BTreeSet::new();
    for input in enum_parameters {
        if !enum_slots.insert(input.parameter) {
            return Err(error(source, "duplicate enum parameter"));
        }
        let binding = function
            .parameters
            .get(input.parameter)
            .ok_or_else(|| error(source, "enum parameter is missing"))?;
        if input.domains.is_empty() {
            return Err(error(source, "empty enum parameter premise"));
        }
        validate_enum_domains(
            analyzer.package,
            &parameters[input.parameter].ty,
            &input.domains,
            &mut analyzer.fuel,
            source,
        )?;
        let domains = copy_enum_domains(&input.domains, &mut analyzer.fuel, source)?;
        analyzer.install_enum_domains(
            &mut flow,
            &Place {
                root: root(*binding),
                fields: vec![],
            },
            domains,
            source,
        )?;
    }
    analyzer.report.enum_parameters = enum_parameters.to_vec();
    analyzer.report.effect_parameters = analyzer.copy_effect_result_inputs(source)?;
    let row = match function.signature.callable_signature() {
        Term::Function { result, .. } => match result.as_ref() {
            Term::Computation { effects, .. } => effects.as_slice(),
            _ => &[],
        },
        _ => &[],
    };
    let mut effect_slots = BTreeSet::new();
    for input in &analyzer.effect_result_inputs {
        if !effect_slots.insert(input.slot) || row.get(input.slot) != Some(&input.operation) {
            return Err(error(
                source,
                "effect result premise does not match the callable row",
            ));
        }
    }
    let mut borrowed_slots = BTreeSet::new();
    for (slot, subject) in borrowed_parameters {
        if !borrowed_slots.insert(*slot) {
            return Err(error(source, "duplicate borrowed parameter"));
        }
        super::memory::bound_type(subject, &mut analyzer.fuel)
            .map_err(|reason| error(source, reason))?;
        analyzer
            .report
            .borrowed_parameters
            .push((*slot, subject.clone()));
        let binding = function
            .parameters
            .get(*slot)
            .ok_or_else(|| error(source, "borrowed parameter is missing"))?;
        let representation = super::memory::pointer_representation(
            analyzer.package,
            &parameters[*slot].ty,
            &mut analyzer.fuel,
        )
        .map_err(|reason| error(source, reason))?;
        let Some((pointer_depth, actual)) = representation else {
            return Err(error(
                source,
                "borrowed parameter lacks a pointer representation",
            ));
        };
        if actual != *subject {
            return Err(error(
                source,
                "borrowed pointee type disagrees with parameter",
            ));
        }
        // Conditional function analysis only: each invocation must establish its
        // footprint against actual targets. Pointees never enter cleanup order.
        let pointee = PlaceId {
            owner: u64::MAX - 1,
            local: u32::try_from(*slot)
                .map_err(|_| error(source, "too many borrowed parameters"))?,
        };
        analyzer.report.borrowed_roots.insert(
            pointee,
            BorrowedRoot::Parameter {
                parameter: *slot,
                input_fields: vec![],
                pointee: subject.clone(),
            },
        );
        let shape = (analyzer.shape)(subject).map_err(|e| error(source, e))?;
        flow.state
            .declare(pointee, &shape, true, &mut analyzer.fuel)
            .map_err(|e| error(source, format!("borrowed parameter shape: {e:?}")))?;
        flow.types.insert(pointee, subject.clone());
        flow.pointers.insert(
            Place {
                root: root(*binding),
                fields: vec![0; pointer_depth],
            },
            BTreeSet::from([Place {
                root: pointee,
                fields: vec![],
            }]),
        );
        // This remains conditional: publication requires a body footprint
        // checked at the actual call site. A scalar
        // pointee cannot carry ownership/provenance that a handler invalidates.
        // Entry pointers without this premise remain unestablished.
        if matches!(subject, Term::Primitive(_))
            && nir::program::intrinsic_pointer_value(subject)
            && (analyzer.delivery)(subject).map_err(|reason| error(source, reason))?
                == Delivery::Copy
        {
            flow.effect_borrows.insert(pointee);
        }
        flow.memory_exposed = true;
    }
    let mut seen_fields = BTreeSet::new();
    for premise in borrowed_fields {
        super::memory::bound_type(&premise.pointee, &mut analyzer.fuel)
            .map_err(|reason| error(source, reason))?;
        analyzer.fuel = analyzer
            .fuel
            .checked_sub(premise.fields.len() + 1)
            .ok_or_else(|| error(source, "borrowed field premise budget exhausted"))?;
        if premise.fields.is_empty()
            || !seen_fields.insert((premise.parameter, premise.fields.clone()))
        {
            return Err(error(source, "duplicate or empty borrowed field premise"));
        }
        let parameter = parameters
            .get(premise.parameter)
            .ok_or_else(|| error(source, "borrowed field parameter is missing"))?;
        let leaves =
            super::memory::pointer_input_layout(package, &parameter.ty, &mut analyzer.fuel)
                .map_err(|reason| error(source, reason))?;
        if !leaves
            .iter()
            .any(|leaf| leaf.input_fields == premise.fields && leaf.pointee == premise.pointee)
        {
            return Err(error(
                source,
                "borrowed field pointee disagrees with parameter layout",
            ));
        }
        let binding = function.parameters[premise.parameter];
        let roots = analyzer.declare_input_field_borrows(
            &mut flow,
            binding,
            &parameter.ty,
            &BTreeMap::from([(premise.fields.clone(), BTreeSet::new())]),
            source,
        )?;
        for (path, targets) in roots {
            for target in targets {
                // This is a conditional borrowed-input premise, just like a
                // direct pointer parameter. The caller still establishes the
                // actual target and footprint; scalar storage cannot hide a
                // handler-induced ownership or pointer-provenance transition.
                if matches!(premise.pointee, Term::Primitive(_))
                    && nir::program::intrinsic_pointer_value(&premise.pointee)
                    && (analyzer.delivery)(&premise.pointee)
                        .map_err(|reason| error(source, reason))?
                        == Delivery::Copy
                {
                    flow.effect_borrows.insert(target.root);
                }
                analyzer.report.borrowed_roots.insert(
                    target.root,
                    BorrowedRoot::Parameter {
                        parameter: premise.parameter,
                        input_fields: path.clone(),
                        pointee: premise.pointee.clone(),
                    },
                );
            }
        }
        analyzer.report.borrowed_fields.push(premise.clone());
    }
    analyzer.report.borrowed_code = copy_borrowed_code(borrowed_code, &mut analyzer.fuel, source)?;
    let mut borrowed_code_slots = BTreeSet::new();
    for input in borrowed_code {
        analyzer.fuel = analyzer
            .fuel
            .checked_sub(input.input_fields.len() + 1)
            .ok_or_else(|| error(source, "borrowed code path budget exhausted"))?;
        if !borrowed_code_slots.insert((input.parameter, input.input_fields.clone())) {
            return Err(error(source, "duplicate borrowed code premise"));
        }
        if input.absent {
            if !enum_parameters.iter().any(|premise| {
                premise.parameter == input.parameter
                    && enum_path_excluded(&input.input_fields, &premise.domains)
            }) {
                return Err(error(
                    source,
                    "borrowed code absence lacks its argument variant premise",
                ));
            }
            continue;
        }
        let root = analyzer
            .report
            .borrowed_roots
            .iter()
            .find_map(|(root, origin)| match origin {
                BorrowedRoot::Parameter {
                    parameter,
                    input_fields,
                    pointee,
                } if *parameter == input.parameter
                    && input_fields == &input.input_fields
                    && pointee == &input.pointee =>
                {
                    Some(*root)
                }
                _ => None,
            })
            .ok_or_else(|| error(source, "borrowed code premise lacks a matching root"))?;
        let place = Place {
            root,
            fields: vec![],
        };
        let domains = copy_enum_domains(&input.domains, &mut analyzer.fuel, source)?;
        let mut fields = BTreeMap::new();
        let mut scalar = None;
        for (path, callable) in &input.fields {
            analyzer.fuel = analyzer
                .fuel
                .checked_sub(path.len() + 1)
                .ok_or_else(|| error(source, "borrowed code field budget exhausted"))?;
            bound_callable_copy(callable, &mut analyzer.fuel, source, 0)?;
            if path.is_empty() {
                scalar = Some(callable.clone());
            } else {
                fields.insert(path.clone(), callable.clone());
            }
        }
        analyzer.install_callable_fields(&mut flow, &place, fields, source)?;
        analyzer.install_enum_domains(&mut flow, &place, domains, source)?;
        if let Some(callable) = scalar {
            analyzer.store_callable(&mut flow, root, callable, source)?;
        }
    }
    for snapshot in &function.entry_snapshots {
        if !function.parameters.contains(&snapshot.parameter) {
            return Err(error(source, "entry snapshot source is not a parameter"));
        }
        let ty = flow
            .types
            .get(&root(snapshot.parameter))
            .cloned()
            .ok_or_else(|| error(source, "entry snapshot parameter is missing"))?;
        if (analyzer.delivery)(&ty).map_err(|e| error(source, e))? != Delivery::Copy {
            return Err(error(
                source,
                "entry snapshot requires checked Copy permission",
            ));
        }
        analyzer.declare(&mut flow, snapshot.binding, &ty, source)?;
    }
    for condition in &function.preconditions {
        analyzer.check_contract_predicate(condition, &mut flow, 0)?;
    }
    let origins = analyzer.evaluate_value_origins(&function.body, &mut flow, 0)?;
    let result_callable = flow.value_callable.clone();
    let result_fields = flow.value_callables.clone();
    let result_variants = flow.value_variants.clone();
    analyzer.check_function_result(&mut flow, origins, source, 0)?;
    if flow.live {
        flow.value_callable = result_callable;
        flow.value_callables = result_fields;
        flow.value_variants = result_variants;
    }
    if flow.live {
        analyzer.exit(&mut flow, source, CleanupReason::FunctionEnd)?;
    }
    analyzer.report.entry_steps_used = initial_fuel - analyzer.fuel;
    Ok(analyzer.report)
}

// A computation has no owned runtime representation. Resource evidence belongs
// to its completion value; keep the effect row on the original IR/signature.
fn completion_type(ty: &Term) -> &Term {
    match ty {
        Term::Computation { result, .. } => result,
        ty => ty,
    }
}

fn expression_place(expr: &Expr) -> Option<Place> {
    let mut indices = Vec::new();
    let mut current = expr;
    loop {
        match &current.kind {
            K::Local(id) => {
                indices.reverse();
                return Some(Place {
                    root: root(*id),
                    fields: indices,
                });
            }
            K::AnonymousField { value, index, .. } | K::Field { value, index, .. } => {
                indices.push(*index);
                current = value;
            }
            _ => return None,
        }
    }
}

/// Expand owned struct/tuple positions for field-sensitive analysis. This is a
/// structural availability model, not proof of permission to tear down a value
/// with a destructor or cross-field dependencies. The cleanup planner must
/// validate those independent obligations before executable code is published.
pub fn analyze_in_package(
    package: &nir::program::Package,
    function: &Function,
    delivery: impl FnMut(&Term) -> Result<Delivery, String>,
    fuel: usize,
) -> Result<Report, Error> {
    analyze_in_package_with_cleanup(package, function, delivery, fuel, None)
}

/// Opt-in acquired cleanup planning. Failure returns no partially planned report.
pub fn analyze_in_package_with_cleanup<'a>(
    package: &'a Package,
    function: &'a Function,
    delivery: impl FnMut(&Term) -> Result<Delivery, String>,
    fuel: usize,
    cleanup: Option<&'a mut dyn CleanupPolicy>,
) -> Result<Report, Error> {
    analyze_in_package_with_borrows(package, function, delivery, fuel, cleanup, &[])
}

/// Analyze under explicit borrowed parameter assumptions. This does not prove
/// validity at any call site; callers must check the instantiated memory footprint.
pub fn analyze_in_package_with_borrows<'a>(
    package: &'a Package,
    function: &'a Function,
    delivery: impl FnMut(&Term) -> Result<Delivery, String>,
    fuel: usize,
    cleanup: Option<&'a mut dyn CleanupPolicy>,
    borrowed_parameters: &[(usize, Term)],
) -> Result<Report, Error> {
    analyze_in_package_with_inputs(
        package,
        function,
        delivery,
        fuel,
        cleanup,
        borrowed_parameters,
        &[],
    )
}

/// Analyze a concrete function under explicit memory and callable premises.
/// The returned report records callable inputs; publication must associate it
/// with the matching invocation, never reuse it for arbitrary same-typed values.
pub fn analyze_in_package_with_inputs<'a>(
    package: &'a Package,
    function: &'a Function,
    delivery: impl FnMut(&Term) -> Result<Delivery, String>,
    fuel: usize,
    cleanup: Option<&'a mut dyn CleanupPolicy>,
    borrowed_parameters: &[(usize, Term)],
    callable_parameters: &[CallableParameter],
) -> Result<Report, Error> {
    analyze_in_package_with_context(
        package,
        function,
        delivery,
        fuel,
        cleanup,
        borrowed_parameters,
        callable_parameters,
        &[],
    )
}

/// Analyze under both code identity and conditional enum tag premises.
pub fn analyze_in_package_with_context<'a>(
    package: &'a Package,
    function: &'a Function,
    delivery: impl FnMut(&Term) -> Result<Delivery, String>,
    fuel: usize,
    cleanup: Option<&'a mut dyn CleanupPolicy>,
    borrowed_parameters: &[(usize, Term)],
    callable_parameters: &[CallableParameter],
    enum_parameters: &[EnumParameter],
) -> Result<Report, Error> {
    analyze_in_package_with_effect_context(
        package,
        function,
        delivery,
        fuel,
        cleanup,
        borrowed_parameters,
        callable_parameters,
        enum_parameters,
        vec![],
    )
}

/// Conditional handler-result premises are issued by checked installations.
/// Publication must retain their association with the exact call context.
pub fn analyze_in_package_with_effect_context<'a>(
    package: &'a Package,
    function: &'a Function,
    delivery: impl FnMut(&Term) -> Result<Delivery, String>,
    fuel: usize,
    cleanup: Option<&'a mut dyn CleanupPolicy>,
    borrowed_parameters: &[(usize, Term)],
    callable_parameters: &[CallableParameter],
    enum_parameters: &[EnumParameter],
    effects: Vec<EffectResultInput>,
) -> Result<Report, Error> {
    analyze_in_package_with_field_context(
        package,
        function,
        delivery,
        fuel,
        cleanup,
        borrowed_parameters,
        callable_parameters,
        enum_parameters,
        effects,
        &[],
    )
}

/// Field validity premises must be discharged by the caller's full memory footprint.
pub fn analyze_in_package_with_field_context<'a>(
    package: &'a Package,
    function: &'a Function,
    delivery: impl FnMut(&Term) -> Result<Delivery, String>,
    fuel: usize,
    cleanup: Option<&'a mut dyn CleanupPolicy>,
    borrowed_parameters: &[(usize, Term)],
    callable_parameters: &[CallableParameter],
    enum_parameters: &[EnumParameter],
    effects: Vec<EffectResultInput>,
    borrowed_fields: &[BorrowedField],
) -> Result<Report, Error> {
    analyze_in_package_with_borrowed_code_context(
        package,
        function,
        delivery,
        fuel,
        cleanup,
        borrowed_parameters,
        callable_parameters,
        enum_parameters,
        effects,
        borrowed_fields,
        &[],
    )
}
pub fn analyze_in_package_with_borrowed_code_context<'a>(
    package: &'a Package,
    function: &'a Function,
    delivery: impl FnMut(&Term) -> Result<Delivery, String>,
    fuel: usize,
    cleanup: Option<&'a mut dyn CleanupPolicy>,
    borrowed_parameters: &[(usize, Term)],
    callable_parameters: &[CallableParameter],
    enum_parameters: &[EnumParameter],
    effects: Vec<EffectResultInput>,
    borrowed_fields: &[BorrowedField],
    borrowed_code: &[BorrowedCode],
) -> Result<Report, Error> {
    let mut shape_fuel = fuel;
    analyze_with_shapes_and_borrowed_code(
        Some(package),
        function,
        delivery,
        |ty| ownership_shape(package, ty, &mut shape_fuel, 0),
        fuel,
        cleanup,
        borrowed_parameters,
        callable_parameters,
        enum_parameters,
        vec![],
        effects,
        borrowed_fields,
        borrowed_code,
        &[],
    )
}

fn ownership_shape(
    package: &nir::program::Package,
    ty: &Term,
    fuel: &mut usize,
    depth: usize,
) -> Result<Shape, String> {
    if *fuel == 0 || depth > 128 {
        return Err("ownership shape budget exhausted".into());
    }
    *fuel -= 1;
    let fields = match ty {
        Term::NormalForm { .. } if nir::normal_forms::tuple_fields(Some(package), ty).is_some() => {
            super::memory::bound_type(ty, fuel)?;
            nir::normal_forms::tuple_fields(Some(package), ty)
                .unwrap()
                .to_vec()
        }
        Term::Record(fields) => {
            super::memory::bound_type(ty, fuel)?;
            fields.iter().map(|(_, ty)| ty.clone()).collect()
        }
        Term::Tuple(fields)
        | Term::Closure {
            environment: fields,
            ..
        } => fields.clone(),
        Term::Nominal {
            declaration,
            arguments,
        } => {
            if let Some(definition) = package
                .newtypes
                .iter()
                .find(|definition| definition.declaration == *declaration)
            {
                let representation = definition
                    .representation_budgeted(arguments, &[], fuel)
                    .map_err(|error| format!("ownership representation: {error}"))?;
                return Ok(Shape::Fields(vec![ownership_shape(
                    package,
                    &representation,
                    fuel,
                    depth + 1,
                )?]));
            }
            let Some(definition) = package
                .structs
                .iter()
                .find(|s| s.declaration == *declaration)
            else {
                return Ok(Shape::Value);
            };
            definition
                .instantiate_fields(arguments)
                .map_err(|e| format!("ownership fields: {e:?}"))?
                .into_iter()
                .map(|(_, ty)| ty)
                .collect()
        }
        _ => return Ok(Shape::Value),
    };
    Ok(Shape::Fields(
        fields
            .iter()
            .map(|ty| ownership_shape(package, ty, fuel, depth + 1))
            .collect::<Result<_, _>>()?,
    ))
}

// Same live closure identity can retain its code while captured named targets
// widen. Distinct closure generations remain separate receiver identities.
fn join_capture_evidence(
    left: &BTreeMap<Place, Callable>,
    right: &BTreeMap<Place, Callable>,
    left_tags: &BTreeMap<PlaceId, EnumDomains>,
    right_tags: &BTreeMap<PlaceId, EnumDomains>,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<(BTreeMap<Place, Callable>, BTreeMap<PlaceId, EnumDomains>), Error> {
    let unknown = EnumDomains::new();
    let mut captures = BTreeMap::new();
    for place in left.keys().chain(right.keys()) {
        *fuel = fuel
            .checked_sub(place.fields.len().saturating_add(1))
            .ok_or_else(|| error(source, "capture join budget exhausted"))?;
        if captures.contains_key(place) {
            continue;
        }
        let value = match (left.get(place), right.get(place)) {
            (Some(a), Some(b)) => merge_callable_provenance(a, b, fuel, source, depth + 1)?,
            (Some(a), None)
                if enum_path_excluded(
                    &place.fields,
                    right_tags.get(&place.root).unwrap_or(&unknown),
                ) =>
            {
                Some(a.clone())
            }
            (None, Some(b))
                if enum_path_excluded(
                    &place.fields,
                    left_tags.get(&place.root).unwrap_or(&unknown),
                ) =>
            {
                Some(b.clone())
            }
            _ => None,
        };
        if let Some(value) = value {
            captures.insert(place.clone(), value);
        }
    }
    let mut enums = BTreeMap::new();
    for (root, domains) in left_tags {
        let joined = join_enum_domains(
            domains,
            right_tags.get(root).unwrap_or(&unknown),
            fuel,
            source,
        )?;
        if !joined.is_empty() {
            enums.insert(*root, joined);
        }
    }
    Ok((captures, enums))
}

fn merge_callable_provenance(
    left: &Callable,
    right: &Callable,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<Option<Callable>, Error> {
    if *fuel == 0 || depth >= 128 {
        return Err(error(source, "callable join budget exhausted"));
    }
    *fuel -= 1;
    if let Some(named) = merge_named_callables(left, right, fuel, source)? {
        return Ok(Some(named));
    }
    if !left.choices.is_empty()
        || !right.choices.is_empty()
        || !same_callable_definition(&left.definition, &right.definition)
    {
        if left.definition.ty != right.definition.ty {
            return Ok(None);
        }
        let mut flattened = vec![];
        for value in std::iter::once(left)
            .chain(&left.choices)
            .chain(std::iter::once(right))
            .chain(&right.choices)
        {
            for definition in std::iter::once(&value.definition).chain(&value.alternatives) {
                *fuel = fuel
                    .checked_sub(1)
                    .ok_or_else(|| error(source, "callable candidate join budget exhausted"))?;
                if !matches!(
                    definition.kind,
                    K::Closure { .. } | K::Function(_) | K::FunctionInstance { .. }
                ) {
                    return Ok(None);
                }
                let mut candidate = value.clone();
                candidate.definition = definition.clone();
                candidate.choices.clear();
                candidate.alternatives.clear();
                candidate.generation = 0;
                flattened.push(candidate);
            }
        }
        let mut candidates: Vec<Callable> = vec![];
        for value in flattened {
            *fuel = fuel
                .checked_sub(candidates.len().saturating_add(1))
                .ok_or_else(|| error(source, "closure candidate join budget exhausted"))?;
            if let Some(index) = candidates.iter().position(|previous| {
                same_callable_definition(&previous.definition, &value.definition)
            }) {
                candidates[index] =
                    merge_callable_provenance(&candidates[index], &value, fuel, source, depth + 1)?
                        .ok_or_else(|| {
                            error(
                                source,
                                "closure candidate capture evidence cannot be joined",
                            )
                        })?;
            } else {
                if matches!(value.definition.kind, K::Closure { .. })
                    && candidates.iter().any(|previous| {
                        matches!(previous.definition.kind, K::Closure { .. })
                            && previous.definition.source == value.definition.source
                    })
                {
                    return Err(error(source, "ambiguous closure definition identity"));
                }
                candidates.push(value);
            }
        }
        // Keep a closure representative so dispatch cannot drop its environment.
        candidates.sort_by_key(|value| {
            (
                !matches!(value.definition.kind, K::Closure { .. }),
                value.definition.source.start,
                value.definition.source.end,
            )
        });
        let mut primary = candidates.remove(0);
        primary.choices = candidates;
        return Ok(Some(primary));
    }
    if left.generation != right.generation
        || !same_callable_definition(&left.definition, &right.definition)
        || !left.alternatives.is_empty()
        || !right.alternatives.is_empty()
    {
        return Ok(None);
    }
    let (captures, enum_captures) = join_capture_evidence(
        &left.captures,
        &right.captures,
        &left.enum_captures,
        &right.enum_captures,
        fuel,
        source,
        depth,
    )?;
    Ok(Some(Callable {
        choices: vec![],
        enum_captures,
        definition: left.definition.clone(),
        alternatives: vec![],
        generation: left.generation,
        captures,
    }))
}

fn prepared_memory_code_inputs<'a>(
    package: Option<&Package>,
    prepared: &'a ClosureArguments,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<super::memory::CodeInputs<'a>, Error> {
    let mut result = BTreeMap::new();
    for (parameter, callable) in &prepared.callables {
        if let Some(definitions) = memory_code_definitions(package, callable, fuel, source)? {
            result.insert(
                super::memory::CallableInput {
                    parameter: *parameter,
                    fields: vec![],
                },
                definitions,
            );
        }
    }
    for (parameter, fields) in &prepared.fields {
        for (path, callable) in fields {
            *fuel = fuel
                .checked_sub(path.len() + 1)
                .ok_or_else(|| error(source, "prepared code field budget exhausted"))?;
            if path.is_empty() {
                continue;
            }
            if let Some(definitions) = memory_code_definitions(package, callable, fuel, source)? {
                result.insert(
                    super::memory::CallableInput {
                        parameter: *parameter,
                        fields: path.clone(),
                    },
                    definitions,
                );
            }
        }
    }
    Ok(result)
}

fn memory_code_definitions<'a>(
    package: Option<&Package>,
    callable: &'a Callable,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Option<Vec<super::memory::CodeDefinition<'a>>>, Error> {
    memory_code_definitions_at(package, callable, fuel, source, 0)
}
fn memory_code_definitions_at<'a>(
    package: Option<&Package>,
    callable: &'a Callable,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<Option<Vec<super::memory::CodeDefinition<'a>>>, Error> {
    if depth >= 64 {
        return Err(error(source, "prepared capture depth exhausted"));
    }
    use super::memory::{CallablePlace, CodeDefinition};
    // Named-only evidence already uses the established summary-input route.
    if named_memory_callees(callable, fuel, source)?.is_some() {
        return Ok(None);
    }
    let mut definitions = vec![];
    for candidate in std::iter::once(callable).chain(&callable.choices) {
        if !std::ptr::eq(candidate, callable) && !candidate.choices.is_empty() {
            return Err(error(
                source,
                "nested prepared code choices require flattening",
            ));
        }
        for definition in std::iter::once(&candidate.definition).chain(&candidate.alternatives) {
            *fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| error(source, "prepared code candidate budget exhausted"))?;
            let value = match &definition.kind {
                K::Function(declaration) => CodeDefinition::Named(iir::InstanceKey {
                    declaration: *declaration,
                    arguments: vec![],
                }),
                K::FunctionInstance {
                    declaration,
                    arguments,
                } => {
                    for ty in arguments {
                        super::memory::bound_type(ty, fuel)
                            .map_err(|reason| error(source, reason))?;
                    }
                    CodeDefinition::Named(iir::InstanceKey {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                    })
                }
                K::Closure { captures, .. } => {
                    let mut code = BTreeMap::new();
                    let mut prepared_captures = BTreeMap::new();
                    for capture in captures {
                        if let Some(domains) = candidate.enum_captures.get(&root(capture.binding)) {
                            validate_enum_domains(package, &capture.ty, domains, fuel, source)?;
                            for path in
                                callable_value_paths(package, &capture.ty, fuel, source, depth + 1)?
                            {
                                if enum_path_excluded(&path, domains) {
                                    prepared_captures.insert(
                                        CallablePlace {
                                            binding: capture.binding,
                                            fields: path,
                                        },
                                        vec![],
                                    );
                                }
                            }
                        }
                        for (place, value) in &candidate.captures {
                            *fuel = fuel.checked_sub(place.fields.len() + 1).ok_or_else(|| {
                                error(source, "prepared code capture budget exhausted")
                            })?;
                            if place.root == root(capture.binding) {
                                if let Some(keys) = named_memory_callees(value, fuel, source)? {
                                    code.insert(
                                        CallablePlace {
                                            binding: capture.binding,
                                            fields: place.fields.clone(),
                                        },
                                        keys,
                                    );
                                } else if let Some(definitions) = memory_code_definitions_at(
                                    package,
                                    value,
                                    fuel,
                                    source,
                                    depth + 1,
                                )? {
                                    let input = super::memory::CallableInput {
                                        parameter: 0,
                                        fields: vec![],
                                    };
                                    let mut owned = super::memory::own_code_inputs(
                                        &BTreeMap::from([(input.clone(), definitions)]),
                                        &[],
                                        &[],
                                        fuel,
                                    )
                                    .map_err(|e| error(source, e))?;
                                    prepared_captures.insert(
                                        CallablePlace {
                                            binding: capture.binding,
                                            fields: place.fields.clone(),
                                        },
                                        owned.remove(&input).unwrap(),
                                    );
                                }
                            }
                        }
                    }
                    CodeDefinition::Closure {
                        definition,
                        captures: std::sync::Arc::new(code),
                        prepared_captures: std::sync::Arc::new(prepared_captures),
                    }
                }
                _ => return Ok(None),
            };
            definitions.push(value);
        }
    }
    Ok(Some(definitions))
}

fn prepared_memory_inputs(
    prepared: &ClosureArguments,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<BTreeMap<super::memory::CallableInput, super::memory::CallableResult>, Error> {
    let mut result = BTreeMap::new();
    for (parameter, callable) in &prepared.callables {
        if let Some(value) = named_memory_callees(callable, fuel, source)? {
            result.insert(
                super::memory::CallableInput {
                    parameter: *parameter,
                    fields: vec![],
                },
                value,
            );
        }
    }
    for (parameter, fields) in &prepared.fields {
        for (path, callable) in fields {
            *fuel = fuel
                .checked_sub(path.len() + 1)
                .ok_or_else(|| error(source, "prepared memory field budget exhausted"))?;
            if path.is_empty() {
                continue;
            }
            if let Some(value) = named_memory_callees(callable, fuel, source)? {
                result.insert(
                    super::memory::CallableInput {
                        parameter: *parameter,
                        fields: path.clone(),
                    },
                    value,
                );
            }
        }
    }
    Ok(result)
}

/// Bridge resource-checked callback premises into the memory worklist. Import
/// validation checks each environment before the memory layer borrows its code.
pub fn infer_callable_memory(
    package: &Package,
    declaration: nir::DeclarationId,
    arguments: &[Term],
    inputs: &[CallableParameter],
    fuel: &mut usize,
    source: SourceRange,
) -> Result<super::memory::Summary, String> {
    infer_callable_memory_with_effects(package, declaration, arguments, inputs, &[], fuel, source)
}
/// Preserve the installer's checked effect premises when publishing a
/// call-specific cleanup footprint for a borrowed helper.
pub fn infer_callable_memory_with_effect_context(
    package: &Package,
    declaration: nir::DeclarationId,
    arguments: &[Term],
    inputs: &[CallableParameter],
    effects: &[EffectResultInput],
    fuel: &mut usize,
    source: SourceRange,
) -> Result<super::memory::Summary, String> {
    infer_callable_memory_with_borrowed_context(
        package,
        declaration,
        arguments,
        inputs,
        effects,
        &[],
        fuel,
        source,
    )
}
pub fn infer_callable_memory_with_borrowed_context(
    package: &Package,
    declaration: nir::DeclarationId,
    arguments: &[Term],
    inputs: &[CallableParameter],
    effects: &[EffectResultInput],
    borrowed: &[BorrowedCode],
    fuel: &mut usize,
    source: SourceRange,
) -> Result<super::memory::Summary, String> {
    infer_callable_memory_with_borrowed_cleanup_context(
        package,
        declaration,
        arguments,
        inputs,
        &[],
        effects,
        borrowed,
        None,
        fuel,
        source,
    )
}
/// Preserve the caller's independently selected complete-layout cleanup policy.
pub fn infer_callable_memory_with_borrowed_cleanup_context(
    package: &Package,
    declaration: nir::DeclarationId,
    arguments: &[Term],
    inputs: &[CallableParameter],
    enums: &[EnumParameter],
    effects: &[EffectResultInput],
    borrowed: &[BorrowedCode],
    cleanup: Option<&mut dyn CleanupPolicy>,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<super::memory::Summary, String> {
    let mut memory_inputs = Vec::new();
    for input in effects {
        memory_inputs.push(
            input
                .memory_input(fuel, source)
                .map_err(|e| e.to_string())?,
        );
    }
    infer_callable_memory_with_borrowed(
        package,
        declaration,
        arguments,
        inputs,
        enums,
        &memory_inputs,
        borrowed,
        cleanup,
        false,
        fuel,
        source,
    )
}
fn infer_callable_memory_with_effects(
    package: &Package,
    declaration: nir::DeclarationId,
    arguments: &[Term],
    inputs: &[CallableParameter],
    effects: &[super::memory::EffectInput],
    fuel: &mut usize,
    source: SourceRange,
) -> Result<super::memory::Summary, String> {
    infer_callable_memory_with_borrowed(
        package,
        declaration,
        arguments,
        inputs,
        &[],
        effects,
        &[],
        None,
        false,
        fuel,
        source,
    )
}
fn infer_callable_memory_with_borrowed(
    package: &Package,
    declaration: nir::DeclarationId,
    arguments: &[Term],
    inputs: &[CallableParameter],
    enums: &[EnumParameter],
    effects: &[super::memory::EffectInput],
    borrowed: &[BorrowedCode],
    cleanup: Option<&mut dyn CleanupPolicy>,
    include_cleanup: bool,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<super::memory::Summary, String> {
    let (function, body_arguments) = package
        .runtime_body(declaration, arguments)
        .ok_or("memory function is missing")?;
    super::memory::bound_type(&function.signature, fuel)?;
    for argument in arguments {
        super::memory::bound_type(argument, fuel)?;
    }
    let binders = super::memory::memory_binders(Some(package), &function.type_parameters, fuel)?;
    let signature = function
        .signature
        .instantiate_parameters(&binders, body_arguments)
        .map_err(|e| e.to_string())?;
    let Term::Function { parameters, .. } = signature.callable_signature() else {
        return Err("memory callee lacks a signature".into());
    };
    let mut restored = BTreeMap::new();
    for input in inputs {
        *fuel = fuel
            .checked_sub(input.fields.len() + 1)
            .ok_or("memory callable input budget exhausted")?;
        let parameter = parameters
            .get(input.parameter)
            .ok_or("memory callable input parameter is missing")?;
        let expected = owned_field_type(Some(package), &parameter.ty, &input.fields, fuel, source)
            .map_err(|e| e.to_string())?;
        let callable = import_callable_evidence(
            Some(package),
            &input.definition,
            &input.alternatives,
            &input.captures,
            &input.enum_captures,
            &input.choices,
            &expected,
            fuel,
            0,
        )
        .map_err(|e| e.to_string())?;
        let key = super::memory::CallableInput {
            parameter: input.parameter,
            fields: input.fields.clone(),
        };
        if restored.insert(key, callable).is_some() {
            return Err("duplicate memory callable input".into());
        }
    }
    let named = callable_memory_inputs(inputs, fuel, source).map_err(|e| e.to_string())?;
    let mut code = BTreeMap::new();
    // Absence is a checked tag premise, never inferred from missing code.
    // Without it the memory worklist treats an unreachable variant's call
    // as an unknown dynamic call and loses otherwise complete evidence.
    let mut enum_parameters = BTreeSet::new();
    for premise in enums {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("memory enum input budget exhausted")?;
        if !enum_parameters.insert(premise.parameter) {
            return Err("duplicate memory enum input".into());
        }
        let parameter = parameters
            .get(premise.parameter)
            .ok_or("memory enum input parameter is missing")?;
        validate_enum_domains(Some(package), &parameter.ty, &premise.domains, fuel, source)
            .map_err(|e| e.to_string())?;
        for fields in callable_value_paths(Some(package), &parameter.ty, fuel, source, 0)
            .map_err(|e| e.to_string())?
        {
            if enum_path_excluded(&fields, &premise.domains) {
                let input = super::memory::CallableInput {
                    parameter: premise.parameter,
                    fields,
                };
                if restored.contains_key(&input) {
                    return Err("callable input contradicts enum absence premise".into());
                }
                code.insert(input, vec![]);
            }
        }
    }
    for (input, callable) in &restored {
        if let Some(definitions) = memory_code_definitions(Some(package), callable, fuel, source)
            .map_err(|e| e.to_string())?
        {
            code.insert(input.clone(), definitions);
        }
    }
    let borrowed =
        borrowed_memory_inputs(Some(package), borrowed, fuel, source).map_err(|e| e.to_string())?;
    if !borrowed.is_empty() || (include_cleanup && cleanup.is_some()) {
        return super::memory::infer_with_borrowed_callable_inputs_and_cleanup(
            package,
            declaration,
            arguments,
            &named,
            &code,
            effects,
            &borrowed,
            cleanup,
            fuel,
        );
    }
    super::memory::infer_with_effect_inputs(
        package,
        declaration,
        arguments,
        &named,
        &code,
        effects,
        fuel,
    )
}

/// Export only complete named-code premises. Missing or mixed anonymous leaves
/// remain unproved and cannot authorize a body-derived dynamic footprint.
pub fn callable_memory_inputs(
    inputs: &[CallableParameter],
    fuel: &mut usize,
    source: SourceRange,
) -> Result<BTreeMap<super::memory::CallableInput, super::memory::CallableResult>, Error> {
    let mut result = BTreeMap::new();
    for input in inputs {
        *fuel = fuel
            .checked_sub(input.fields.len() + 1)
            .ok_or_else(|| error(source, "memory callable input budget exhausted"))?;
        if !input.choices.is_empty() {
            continue;
        }
        if let Some(value) =
            named_memory_definitions(&input.definition, &input.alternatives, fuel, source)?
        {
            let key = super::memory::CallableInput {
                parameter: input.parameter,
                fields: input.fields.clone(),
            };
            if result.insert(key, value).is_some() {
                return Err(error(source, "duplicate memory callable input"));
            }
        }
    }
    Ok(result)
}

// Only a complete named set can seed the memory inference environment. A
// closure alternative cannot disappear merely because other targets are named.
fn named_memory_callees(
    callable: &Callable,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Option<super::memory::CallableResult>, Error> {
    if !callable.choices.is_empty() {
        return Ok(None);
    }
    named_memory_definitions(&callable.definition, &callable.alternatives, fuel, source)
}
fn named_memory_definitions(
    definition: &Expr,
    alternatives: &[Expr],
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Option<super::memory::CallableResult>, Error> {
    let mut keys = vec![];
    for definition in std::iter::once(definition).chain(alternatives) {
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(source, "captured callable footprint budget exhausted"))?;
        let key = match &definition.kind {
            K::Function(declaration) => iir::InstanceKey {
                declaration: *declaration,
                arguments: vec![],
            },
            K::FunctionInstance {
                declaration,
                arguments,
            } => {
                for ty in arguments {
                    super::memory::bound_type(ty, fuel).map_err(|reason| error(source, reason))?;
                }
                iir::InstanceKey {
                    declaration: *declaration,
                    arguments: arguments.clone(),
                }
            }
            _ => return Ok(None),
        };
        keys.push(key);
    }
    Ok(Some(if keys.len() == 1 {
        super::memory::CallableResult::Known(keys.remove(0))
    } else {
        super::memory::CallableResult::Candidates(keys)
    }))
}

fn merge_named_callables(
    left: &Callable,
    right: &Callable,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Option<Callable>, Error> {
    if !left.choices.is_empty()
        || !right.choices.is_empty()
        || left.definition.ty != right.definition.ty
        || !matches!(
            left.definition.kind,
            K::Function(_) | K::FunctionInstance { .. }
        )
        || !matches!(
            right.definition.kind,
            K::Function(_) | K::FunctionInstance { .. }
        )
    {
        return Ok(None);
    }
    let mut targets = vec![];
    for target in std::iter::once(&left.definition)
        .chain(&left.alternatives)
        .chain(std::iter::once(&right.definition))
        .chain(&right.alternatives)
    {
        super::memory::bound_type(&target.ty, fuel).map_err(|e| error(source, e))?;
        if let K::FunctionInstance { arguments, .. } = &target.kind {
            for ty in arguments {
                super::memory::bound_type(ty, fuel).map_err(|e| error(source, e))?;
            }
        }
        *fuel = fuel
            .checked_sub(targets.len() + 1)
            .ok_or_else(|| error(source, "callable candidates budget exhausted"))?;
        if !targets
            .iter()
            .any(|previous| same_callable_definition(previous, target))
        {
            targets.push(target.clone());
        }
    }
    targets.sort_by_cached_key(|value| format!("{:?}", value.kind));
    let definition = targets.remove(0);
    Ok(Some(Callable {
        choices: vec![],
        enum_captures: Default::default(),
        definition,
        alternatives: targets,
        generation: 0,
        captures: BTreeMap::new(),
    }))
}
fn same_callable_definition(left: &Expr, right: &Expr) -> bool {
    if matches!(left.kind, K::Function(_) | K::FunctionInstance { .. }) {
        // Reference spans distinguish uses, not named function identities.
        left.ty == right.ty && left.kind == right.kind
    } else {
        left == right
    }
}

fn export_callable_parameter(
    parameter: usize,
    callable: &Callable,
    fuel: &mut usize,
) -> Result<CallableParameter, Error> {
    let evidence = export_callable_evidence(callable, fuel, 0)?;
    Ok(CallableParameter {
        choices: evidence.choices,
        enum_captures: evidence.enum_captures,
        parameter,
        fields: vec![],
        definition: evidence.definition,
        alternatives: evidence.alternatives,
        captures: evidence.captures,
    })
}

fn export_callable_evidence(
    callable: &Callable,
    fuel: &mut usize,
    depth: usize,
) -> Result<CallableEvidence, Error> {
    let source = callable.definition.source;
    if *fuel == 0 || depth >= 128 {
        return Err(error(source, "callable input export budget exhausted"));
    }
    *fuel -= 1;
    for definition in std::iter::once(&callable.definition).chain(&callable.alternatives) {
        super::memory::bound_type(&definition.ty, fuel).map_err(|e| error(source, e))?;
        if let K::FunctionInstance { arguments, .. } = &definition.kind {
            for ty in arguments {
                super::memory::bound_type(ty, fuel).map_err(|e| error(source, e))?;
            }
        }
    }
    let captures = callable
        .captures
        .iter()
        .map(|(binding, value)| {
            Ok((
                binding.clone(),
                export_callable_evidence(value, fuel, depth + 1)?,
            ))
        })
        .collect::<Result<_, Error>>()?;
    Ok(CallableEvidence {
        choices: callable
            .choices
            .iter()
            .map(|choice| export_callable_evidence(choice, fuel, depth + 1))
            .collect::<Result<_, _>>()?,
        enum_captures: copy_captured_enums(&callable.enum_captures, fuel, source)?,
        definition: callable.definition.clone(),
        alternatives: callable.alternatives.clone(),
        captures,
    })
}

fn import_callable_evidence(
    package: Option<&Package>,
    definition: &Expr,
    alternatives: &[Expr],
    captures: &BTreeMap<Place, CallableEvidence>,
    enum_captures: &BTreeMap<PlaceId, EnumDomains>,
    choices: &[CallableEvidence],
    expected: &Term,
    fuel: &mut usize,
    depth: usize,
) -> Result<Callable, Error> {
    let source = definition.source;
    if *fuel == 0 || depth >= 128 {
        return Err(error(source, "callable input validation budget exhausted"));
    }
    *fuel -= 1;
    let mut result: Option<Callable> = None;
    for definition in std::iter::once(definition).chain(alternatives) {
        super::memory::bound_type(&definition.ty, fuel).map_err(|e| error(source, e))?;
        if expected != &definition.ty {
            return Err(error(
                source,
                "callable definition type disagrees with parameter",
            ));
        }
        if !alternatives.is_empty()
            && !matches!(definition.kind, K::Function(_) | K::FunctionInstance { .. })
        {
            return Err(error(
                source,
                "callable alternatives require named function definitions",
            ));
        }
        validate_callable_input(package, definition, fuel, depth)?;
        let value = callable_source_depth(definition, &Flow::default(), fuel, depth)?
            .ok_or_else(|| error(source, "callable input requires a callable definition"))?;
        result = Some(if let Some(previous) = result {
            for candidate in std::iter::once(&previous.definition).chain(&previous.alternatives) {
                if *fuel == 0 {
                    return Err(error(source, "callable input validation budget exhausted"));
                }
                *fuel -= 1;
                if same_callable_definition(candidate, definition) {
                    return Err(error(source, "duplicate callable parameter candidate"));
                }
            }
            merge_named_callables(&previous, &value, fuel, source)?
                .ok_or_else(|| error(source, "incompatible callable parameter candidates"))?
        } else {
            value
        });
    }
    let mut result = result.expect("a representative is always present");
    for (binding, evidence) in captures {
        let K::Closure {
            captures: fields, ..
        } = &definition.kind
        else {
            return Err(error(
                source,
                "named callable cannot carry capture evidence",
            ));
        };
        let field = fields
            .iter()
            .find(|field| root(field.binding) == binding.root)
            .ok_or_else(|| error(source, "callable evidence refers to an unknown capture"))?;
        let nested = import_callable_evidence(
            package,
            &evidence.definition,
            &evidence.alternatives,
            &evidence.captures,
            &evidence.enum_captures,
            &evidence.choices,
            &owned_field_type(package, &field.ty, &binding.fields, fuel, source)?,
            fuel,
            depth + 1,
        )?;
        result.captures.insert(binding.clone(), nested);
    }
    for (binding, domains) in enum_captures {
        let K::Closure {
            captures: fields, ..
        } = &definition.kind
        else {
            return Err(error(
                source,
                "named callable cannot carry enum capture evidence",
            ));
        };
        let field = fields
            .iter()
            .find(|field| root(field.binding) == *binding)
            .ok_or_else(|| error(source, "enum evidence refers to an unknown capture"))?;
        validate_enum_domains(package, &field.ty, domains, fuel, source)?;
        result
            .enum_captures
            .insert(*binding, copy_enum_domains(domains, fuel, source)?);
    }
    for choice in choices {
        if !matches!(definition.kind, K::Closure { .. })
            || !alternatives.is_empty()
            || !matches!(
                choice.definition.kind,
                K::Closure { .. } | K::Function(_) | K::FunctionInstance { .. }
            )
            || !choice.choices.is_empty()
            || !choice.alternatives.is_empty()
        {
            return Err(error(
                source,
                "closure choices require flat callable definitions",
            ));
        }
        let candidate = import_callable_evidence(
            package,
            &choice.definition,
            &[],
            &choice.captures,
            &choice.enum_captures,
            &[],
            expected,
            fuel,
            depth + 1,
        )?;
        *fuel = fuel
            .checked_sub(result.choices.len().saturating_add(1))
            .ok_or_else(|| error(source, "closure choices validation budget exhausted"))?;
        if std::iter::once(&result)
            .chain(&result.choices)
            .any(|previous| {
                same_callable_definition(&previous.definition, &candidate.definition)
                    || (matches!(previous.definition.kind, K::Closure { .. })
                        && matches!(candidate.definition.kind, K::Closure { .. })
                        && previous.definition.source == candidate.definition.source)
            })
        {
            return Err(error(source, "duplicate closure parameter candidate"));
        }
        result.choices.push(candidate);
    }
    Ok(result)
}

// Compatibility views cannot encode candidate sets in one capture initializer.
// Keep them unknown instead of choosing a representative and losing targets.
fn legacy_callable_definitions(
    evidence: &CallableEvidence,
    fuel: &mut usize,
    depth: usize,
) -> Result<Option<Vec<Expr>>, Error> {
    let source = evidence.definition.source;
    if *fuel == 0 || depth >= 128 {
        return Err(error(source, "callable input export budget exhausted"));
    }
    *fuel -= 1;
    if !evidence.enum_captures.is_empty() || !evidence.choices.is_empty() {
        return Ok(None);
    }
    if evidence
        .captures
        .keys()
        .any(|place| !place.fields.is_empty())
    {
        return Ok(None);
    }
    let mut definition = evidence.definition.clone();
    if let K::Closure { captures, .. } = &mut definition.kind {
        for capture in captures {
            if let Some(nested) = evidence.captures.get(&Place {
                root: root(capture.binding),
                fields: vec![],
            }) {
                let Some(mut definitions) = legacy_callable_definitions(nested, fuel, depth + 1)?
                else {
                    return Ok(None);
                };
                if definitions.len() != 1 {
                    return Ok(None);
                }
                capture.value = Some(Box::new(definitions.remove(0)));
            }
        }
    }
    let mut values = vec![definition];
    values.extend(evidence.alternatives.iter().cloned());
    Ok(Some(values))
}

fn restore_callable_evidence(
    evidence: &CallableEvidence,
    fuel: &mut usize,
    depth: usize,
) -> Result<Callable, Error> {
    if *fuel == 0 || depth >= 128 {
        return Err(error(
            evidence.definition.source,
            "callable return provenance budget exhausted",
        ));
    }
    *fuel -= 1;
    for definition in std::iter::once(&evidence.definition).chain(&evidence.alternatives) {
        super::memory::bound_type(&definition.ty, fuel).map_err(|e| error(definition.source, e))?;
        if let K::FunctionInstance { arguments, .. } = &definition.kind {
            for ty in arguments {
                super::memory::bound_type(ty, fuel).map_err(|e| error(definition.source, e))?;
            }
        }
    }
    let captures = evidence
        .captures
        .iter()
        .map(|(binding, value)| {
            Ok((
                binding.clone(),
                restore_callable_evidence(value, fuel, depth + 1)?,
            ))
        })
        .collect::<Result<_, Error>>()?;
    Ok(Callable {
        choices: evidence
            .choices
            .iter()
            .map(|choice| restore_callable_evidence(choice, fuel, depth + 1))
            .collect::<Result<_, _>>()?,
        enum_captures: copy_captured_enums(
            &evidence.enum_captures,
            fuel,
            evidence.definition.source,
        )?,
        definition: evidence.definition.clone(),
        alternatives: evidence.alternatives.clone(),
        generation: 0,
        captures,
    })
}

fn callable_aggregate_return_summary(
    report: &Report,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<(BTreeMap<Vec<usize>, Callable>, EnumDomains), Error> {
    if report.callable_field_exits.len() != report.enum_variant_exits.len() {
        return Err(error(
            source,
            "callable result and enum domain exits disagree",
        ));
    }
    let mut result: Option<(BTreeMap<Vec<usize>, Callable>, EnumDomains)> = None;
    for (exit, domains) in report
        .callable_field_exits
        .iter()
        .zip(&report.enum_variant_exits)
    {
        let mut fields = BTreeMap::new();
        for (path, evidence) in exit {
            *fuel = fuel
                .checked_sub(path.len().saturating_add(1))
                .ok_or_else(|| error(source, "callable field return budget exhausted"))?;
            fields.insert(path.clone(), restore_callable_evidence(evidence, fuel, 0)?);
        }
        result = Some(if let Some((previous, previous_domains)) = result {
            let merged = join_guarded_callables(
                &previous,
                &fields,
                &previous_domains,
                domains,
                fuel,
                source,
            )?;
            (
                merged,
                join_enum_domains(&previous_domains, domains, fuel, source)?,
            )
        } else {
            (fields, copy_enum_domains(domains, fuel, source)?)
        });
    }
    Ok(result.unwrap_or_default())
}

/// Callable input positions requiring caller-established code identity.
pub fn callable_parameter_paths(
    package: &Package,
    signature: &Term,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Vec<(usize, Vec<usize>)>, Error> {
    let Term::Function { parameters, .. } = signature.callable_signature() else {
        return Err(error(source, "missing callable signature"));
    };
    let mut result = vec![];
    for (slot, parameter) in parameters.iter().enumerate() {
        for path in callable_value_paths(Some(package), &parameter.ty, fuel, source, 0)? {
            result.push((slot, path));
        }
    }
    Ok(result)
}

fn callable_variant_layout(
    definition: &nir::program::Enum,
    arguments: &[Term],
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Vec<nir::program::Variant>, Error> {
    // Charge even empty variants before allocating the instantiated layout.
    *fuel = fuel
        .checked_sub(definition.variants.len())
        .ok_or_else(|| error(source, "callable variant layout budget exhausted"))?;
    for variant in &definition.variants {
        for field in &variant.fields {
            super::memory::bound_type(field, fuel).map_err(|e| error(source, e))?;
            for argument in arguments {
                super::memory::bound_type(argument, fuel).map_err(|e| error(source, e))?;
            }
        }
    }
    definition
        .instantiate_variants(arguments)
        .map_err(|e| error(source, format!("callable variant fields: {e}")))
}

fn callable_value_paths(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<Vec<Vec<usize>>, Error> {
    if *fuel == 0 || depth >= 128 {
        return Err(error(source, "callable input layout budget exhausted"));
    }
    *fuel -= 1;
    if matches!(
        ty,
        Term::Function { .. } | Term::UnsafeFunction(_) | Term::Closure { .. }
    ) {
        return Ok(vec![vec![]]);
    }
    let fields: Vec<Term> = match ty {
        Term::NormalForm { .. } if nir::normal_forms::tuple_fields(package, ty).is_some() => {
            super::memory::bound_type(ty, fuel).map_err(|e| error(source, e))?;
            nir::normal_forms::tuple_fields(package, ty)
                .unwrap()
                .to_vec()
        }
        Term::Tuple(fields) => {
            super::memory::bound_type(ty, fuel).map_err(|e| error(source, e))?;
            fields.clone()
        }
        Term::Record(fields) => {
            super::memory::bound_type(ty, fuel).map_err(|e| error(source, e))?;
            fields.iter().map(|(_, ty)| ty.clone()).collect()
        }
        Term::ErrorResult { .. } => {
            let mut result = vec![];
            for (variant, layout) in super::memory::owned_variants(package, ty, fuel)
                .map_err(|reason| error(source, reason))?
                .into_iter()
                .enumerate()
            {
                for (field, ty) in layout.fields.iter().enumerate() {
                    for path in callable_value_paths(package, ty, fuel, source, depth + 1)? {
                        *fuel = fuel
                            .checked_sub(path.len().saturating_add(3))
                            .ok_or_else(|| error(source, "error callable path budget exhausted"))?;
                        let mut qualified = vec![VARIANT_PATH, variant, field];
                        qualified.extend(path);
                        result.push(qualified);
                    }
                }
            }
            return Ok(result);
        }
        Term::Nominal {
            declaration,
            arguments,
        } => {
            if let Some(definition) = package.and_then(|package| {
                package
                    .newtypes
                    .iter()
                    .find(|definition| definition.declaration == *declaration)
            }) {
                let representation = definition
                    .representation_budgeted(arguments, &[], fuel)
                    .map_err(|reason| error(source, reason.to_string()))?;
                let mut paths =
                    callable_value_paths(package, &representation, fuel, source, depth + 1)?;
                for path in &mut paths {
                    *fuel = fuel
                        .checked_sub(path.len().saturating_add(1))
                        .ok_or_else(|| error(source, "newtype callable path budget exhausted"))?;
                    path.insert(0, 0);
                }
                return Ok(paths);
            }
            if let Some(definition) =
                package.and_then(|p| p.enums.iter().find(|e| e.declaration == *declaration))
            {
                let variants = callable_variant_layout(definition, arguments, fuel, source)?;
                let mut result = vec![];
                for (variant, layout) in variants.iter().enumerate() {
                    for (field, ty) in layout.fields.iter().enumerate() {
                        for path in callable_value_paths(package, ty, fuel, source, depth + 1)? {
                            *fuel = fuel.checked_sub(path.len().saturating_add(3)).ok_or_else(
                                || error(source, "callable input layout budget exhausted"),
                            )?;
                            let mut qualified = vec![VARIANT_PATH, variant, field];
                            qualified.extend(path);
                            result.push(qualified);
                        }
                    }
                }
                return Ok(result);
            }
            let Some(definition) =
                package.and_then(|p| p.structs.iter().find(|s| s.declaration == *declaration))
            else {
                return Ok(vec![]);
            };
            for (_, field) in &definition.fields {
                super::memory::bound_type(field, fuel).map_err(|e| error(source, e))?;
                for argument in arguments {
                    super::memory::bound_type(argument, fuel).map_err(|e| error(source, e))?;
                }
            }
            definition
                .instantiate_fields(arguments)
                .map_err(|e| error(source, format!("callable parameter fields: {e}")))?
                .into_iter()
                .map(|(_, ty)| ty)
                .collect()
        }
        _ => return Ok(vec![]),
    };
    let mut result = vec![];
    for (index, field) in fields.iter().enumerate() {
        for path in callable_value_paths(package, field, fuel, source, depth + 1)? {
            *fuel = fuel
                .checked_sub(path.len().saturating_add(1))
                .ok_or_else(|| error(source, "callable input layout budget exhausted"))?;
            let mut nested = vec![index];
            nested.extend(path);
            result.push(nested);
        }
    }
    Ok(result)
}

// Closed return captures may contain plain data and environment-free function
// slots. Function slots require separate complete, stable code evidence. Enum
// domains and nested closure environments need richer state transport. Pointer
// layouts additionally require a complete independently checked result contract.
// Copy acquisition is checked separately; this predicate never supplies it.
fn recoverable_return_capture_layout(
    package: Option<&Package>,
    ty: &Term,
    pointer_contract: bool,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<bool, Error> {
    if depth >= 128 || *fuel == 0 {
        return Err(error(source, "returned capture layout budget exhausted"));
    }
    *fuel -= 1;
    super::memory::bound_type(ty, fuel).map_err(|e| error(source, e))?;
    if matches!(ty, Term::Primitive(_)) {
        return Ok(nir::program::intrinsic_pointer_value(ty));
    }
    if matches!(ty, Term::Pointer(_)) {
        return Ok(pointer_contract);
    }
    if matches!(ty, Term::Function { .. } | Term::UnsafeFunction(_)) {
        return Ok(true);
    }
    let fields = if let Some(fields) = nir::normal_forms::tuple_fields(package, ty) {
        fields.to_vec()
    } else {
        match ty {
            Term::ErrorResult { errors, result } => std::iter::once(result.as_ref())
                .chain(errors.iter())
                .cloned()
                .collect(),
            Term::Tuple(fields) => fields.clone(),
            Term::Record(fields) => fields.iter().map(|(_, ty)| ty.clone()).collect(),
            Term::Nominal {
                declaration,
                arguments,
            } => {
                if let Some(definition) = package.and_then(|package| {
                    package
                        .newtypes
                        .iter()
                        .find(|definition| definition.declaration == *declaration)
                }) {
                    let representation = definition
                        .representation_budgeted(arguments, &[], fuel)
                        .map_err(|reason| error(source, reason.to_string()))?;
                    return recoverable_return_capture_layout(
                        package,
                        &representation,
                        pointer_contract,
                        fuel,
                        source,
                        depth + 1,
                    );
                }
                if let Some(definition) =
                    package.and_then(|p| p.enums.iter().find(|e| e.declaration == *declaration))
                {
                    for variant in callable_variant_layout(definition, arguments, fuel, source)? {
                        for field in variant.fields {
                            if !recoverable_return_capture_layout(
                                package,
                                &field,
                                pointer_contract,
                                fuel,
                                source,
                                depth + 1,
                            )? {
                                return Ok(false);
                            }
                        }
                    }
                    return Ok(true);
                }
                let Some(definition) =
                    package.and_then(|p| p.structs.iter().find(|s| s.declaration == *declaration))
                else {
                    return Ok(false);
                };
                for (_, ty) in &definition.fields {
                    super::memory::bound_type(ty, fuel).map_err(|e| error(source, e))?;
                }
                definition
                    .instantiate_fields(arguments)
                    .map_err(|e| error(source, format!("returned capture fields: {e}")))?
                    .into_iter()
                    .map(|(_, ty)| ty)
                    .collect()
            }
            _ => return Ok(false),
        }
    };
    for field in &fields {
        if !recoverable_return_capture_layout(
            package,
            field,
            pointer_contract,
            fuel,
            source,
            depth + 1,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn contains_return_evidence(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<bool, Error> {
    contains_return_evidence_kind(package, ty, fuel, source, depth, true, false)
}
fn contains_callable_value(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<bool, Error> {
    contains_return_evidence_kind(package, ty, fuel, source, depth, false, false)
}

fn contains_effect_value(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<bool, Error> {
    Ok(contains_callable_value(package, ty, fuel, source, depth)?
        || contains_pointer_value(package, ty, fuel, source, depth)?)
}
fn contains_pointer_value(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<bool, Error> {
    contains_return_evidence_kind(package, ty, fuel, source, depth, false, true)
}

// Code identities and enum tags both require a contextual return summary.
fn contains_return_evidence_kind(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
    include_tags: bool,
    pointers_only: bool,
) -> Result<bool, Error> {
    if *fuel == 0 || depth >= 128 {
        return Err(error(source, "callable result layout budget exhausted"));
    }
    *fuel -= 1;
    if let Some(fields) = nir::normal_forms::tuple_fields(package, ty) {
        for field in fields {
            if contains_return_evidence_kind(
                package,
                field,
                fuel,
                source,
                depth + 1,
                include_tags,
                pointers_only,
            )? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    match ty {
        Term::ErrorResult { errors, result } => {
            super::memory::bound_type(ty, fuel).map_err(|e| error(source, e))?;
            if include_tags {
                return Ok(true);
            }
            for field in std::iter::once(result.as_ref()).chain(errors.iter()) {
                if contains_return_evidence_kind(
                    package,
                    field,
                    fuel,
                    source,
                    depth + 1,
                    false,
                    pointers_only,
                )? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        Term::Pointer(_) if pointers_only => return Ok(true),
        Term::Closure { environment, .. } if pointers_only => {
            for field in environment {
                if contains_return_evidence_kind(
                    package,
                    field,
                    fuel,
                    source,
                    depth + 1,
                    false,
                    true,
                )? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        Term::Function { .. } | Term::UnsafeFunction(_) | Term::Closure { .. } => {
            return Ok(!pointers_only);
        }
        Term::Tuple(fields) => {
            for field in fields {
                if contains_return_evidence_kind(
                    package,
                    field,
                    fuel,
                    source,
                    depth + 1,
                    include_tags,
                    pointers_only,
                )? {
                    return Ok(true);
                }
            }
        }
        Term::Record(fields) => {
            for (_, field) in fields {
                if contains_return_evidence_kind(
                    package,
                    field,
                    fuel,
                    source,
                    depth + 1,
                    include_tags,
                    pointers_only,
                )? {
                    return Ok(true);
                }
            }
        }
        Term::Nominal {
            declaration,
            arguments,
        } => {
            if let Some(definition) = package.and_then(|package| {
                package
                    .newtypes
                    .iter()
                    .find(|definition| definition.declaration == *declaration)
            }) {
                let representation = definition
                    .representation_budgeted(arguments, &[], fuel)
                    .map_err(|reason| error(source, reason.to_string()))?;
                return contains_return_evidence_kind(
                    package,
                    &representation,
                    fuel,
                    source,
                    depth + 1,
                    include_tags,
                    pointers_only,
                );
            }
            if let Some(definition) =
                package.and_then(|p| p.enums.iter().find(|e| e.declaration == *declaration))
            {
                // A tag constrains reachable branches even when no variant
                // stores code. Validate its instantiated layout before requesting
                // the bounded body summary that supplies actual tag evidence.
                let variants = callable_variant_layout(definition, arguments, fuel, source)?;
                if include_tags {
                    return Ok(true);
                }
                for variant in variants {
                    for field in variant.fields {
                        if contains_return_evidence_kind(
                            package,
                            &field,
                            fuel,
                            source,
                            depth + 1,
                            false,
                            pointers_only,
                        )? {
                            return Ok(true);
                        }
                    }
                }
                return Ok(false);
            }
            let Some(definition) =
                package.and_then(|p| p.structs.iter().find(|s| s.declaration == *declaration))
            else {
                return Ok(false);
            };
            // Charge the declaration and substitution inputs before allocating
            // instantiated layouts. Structural tuple/record traversal borrows.
            for (_, field) in &definition.fields {
                super::memory::bound_type(field, fuel).map_err(|e| error(source, e))?;
                for argument in arguments {
                    super::memory::bound_type(argument, fuel).map_err(|e| error(source, e))?;
                }
            }
            let fields = definition
                .instantiate_fields(arguments)
                .map_err(|e| error(source, format!("callable result fields: {e}")))?;
            for (_, field) in fields {
                if contains_return_evidence_kind(
                    package,
                    &field,
                    fuel,
                    source,
                    depth + 1,
                    include_tags,
                    pointers_only,
                )? {
                    return Ok(true);
                }
            }
        }
        _ => (),
    }
    Ok(false)
}

fn callable_return_summary(
    report: &Report,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Option<Callable>, Error> {
    let mut result: Option<Callable> = None;
    for evidence in &report.callable_evidence_exits {
        let Some(evidence) = evidence else {
            return Ok(None);
        };
        let value = restore_callable_evidence(evidence, fuel, 0)?;
        result = Some(if let Some(previous) = result {
            if same_callable_definition(&previous.definition, &value.definition)
                && previous.choices == value.choices
                && previous.alternatives == value.alternatives
                && previous.captures == value.captures
                && previous.enum_captures == value.enum_captures
            {
                previous
            } else if let Some(merged) =
                merge_callable_provenance(&previous, &value, fuel, source, 0)?
            {
                merged
            } else {
                return Ok(None);
            }
        } else {
            value
        });
    }
    Ok(result)
}

fn validate_callable_input(
    package: Option<&Package>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), Error> {
    if *fuel == 0 || depth >= 128 {
        return Err(error(
            expr.source,
            "callable input validation budget exhausted",
        ));
    }
    *fuel -= 1;
    let named = match &expr.kind {
        K::Function(declaration) => Some((*declaration, &[][..])),
        K::FunctionInstance {
            declaration,
            arguments,
        } => Some((*declaration, arguments.as_slice())),
        _ => None,
    };
    if let Some((declaration, arguments)) = named {
        let (function, body_arguments) = package
            .ok_or_else(|| error(expr.source, "callable input refers to an unknown function"))?
            .runtime_body_checked(declaration, arguments)
            .map_err(|reason| error(expr.source, format!("callable input {reason}")))?;
        if function.phase != FunctionPhase::Runtime {
            return Err(error(
                expr.source,
                "callable input requires a runtime function",
            ));
        }
        if body_arguments.len() != function.type_parameters.len() {
            return Err(error(
                expr.source,
                "callable input has invalid function type arguments",
            ));
        }
        super::memory::bound_type(&function.signature, fuel).map_err(|e| error(expr.source, e))?;
        for argument in arguments {
            super::memory::bound_type(argument, fuel).map_err(|e| error(expr.source, e))?;
        }
        let binders = super::memory::memory_binders(package, &function.type_parameters, fuel)
            .map_err(|e| error(expr.source, e))?;
        let signature = function
            .signature
            .instantiate_parameters(&binders, body_arguments)
            .map_err(|_| {
                error(
                    expr.source,
                    "callable input has invalid function type arguments",
                )
            })?;
        let signature = signature.with_unsafe_call_requirement(function.is_unsafe);
        if signature != expr.ty
            || !matches!(signature, Term::Function { .. } | Term::UnsafeFunction(_))
        {
            return Err(error(
                expr.source,
                "callable input function signature mismatch",
            ));
        }
        return Ok(());
    }
    let K::Closure {
        parameters,
        captures,
        ..
    } = &expr.kind
    else {
        return Err(error(
            expr.source,
            "callable input requires a closure definition",
        ));
    };
    let (environment, signature) = match &expr.ty {
        Term::Closure {
            environment,
            signature,
        } => (environment.as_slice(), signature.as_ref()),
        Term::Function { .. } | Term::UnsafeFunction(_) => (&[][..], &expr.ty),
        _ => {
            return Err(error(
                expr.source,
                "callable input requires a runtime closure",
            ));
        }
    };
    let Term::Function {
        parameters: types, ..
    } = signature
    else {
        return Err(error(
            expr.source,
            "callable input lacks a runtime signature",
        ));
    };
    if parameters.len() != types.len() || captures.len() != environment.len() {
        return Err(error(
            expr.source,
            "callable input shape disagrees with type",
        ));
    }
    let mut bindings = BTreeSet::new();
    for binding in parameters {
        if !bindings.insert(*binding) {
            return Err(error(expr.source, "duplicate callable input binding"));
        }
    }
    for (capture, ty) in captures.iter().zip(environment) {
        if &capture.ty != ty || !bindings.insert(capture.binding) {
            return Err(error(expr.source, "invalid callable input capture"));
        }
        if let Some(value) = &capture.value {
            if value.ty != capture.ty {
                return Err(error(
                    expr.source,
                    "callable input capture initializer type mismatch",
                ));
            }
            if matches!(
                value.kind,
                K::Closure { .. } | K::Function(_) | K::FunctionInstance { .. }
            ) {
                validate_callable_input(package, value, fuel, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn callable_source(expr: &Expr, flow: &Flow, fuel: &mut usize) -> Result<Option<Callable>, Error> {
    callable_source_depth(expr, flow, fuel, 0)
}

fn owned_field_type(
    package: Option<&Package>,
    ty: &Term,
    path: &[usize],
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Term, Error> {
    let mut current = ty.clone();
    let mut components = path.iter();
    while let Some(index) = components.next() {
        if *fuel == 0 {
            return Err(error(source, "temporary field type budget exhausted"));
        }
        *fuel -= 1;
        let fields: Vec<Term> = match &current {
            Term::NormalForm { .. }
                if nir::normal_forms::tuple_fields(package, &current).is_some() =>
            {
                super::memory::bound_type(&current, fuel).map_err(|e| error(source, e))?;
                nir::normal_forms::tuple_fields(package, &current)
                    .unwrap()
                    .to_vec()
            }
            Term::Record(fields) => fields.iter().map(|(_, ty)| ty.clone()).collect(),
            Term::Tuple(fields)
            | Term::Closure {
                environment: fields,
                ..
            } => fields.clone(),
            Term::ErrorResult { .. } => {
                if *index != VARIANT_PATH {
                    return Err(error(
                        source,
                        "error payload requires a branch-qualified path",
                    ));
                }
                let variant = components
                    .next()
                    .ok_or_else(|| error(source, "missing error branch index"))?;
                let field = components
                    .next()
                    .ok_or_else(|| error(source, "missing error payload field"))?;
                current = super::memory::owned_variants(package, &current, fuel)
                    .map_err(|reason| error(source, reason))?
                    .get(*variant)
                    .and_then(|v| v.fields.get(*field))
                    .cloned()
                    .ok_or_else(|| error(source, "error payload path out of bounds"))?;
                continue;
            }
            Term::Nominal {
                declaration,
                arguments,
            } => {
                if let Some(definition) = package.and_then(|package| {
                    package
                        .newtypes
                        .iter()
                        .find(|definition| definition.declaration == *declaration)
                }) {
                    if *index != 0 {
                        return Err(error(
                            source,
                            "newtype representation path index out of bounds",
                        ));
                    }
                    current = definition
                        .representation_budgeted(arguments, &[], fuel)
                        .map_err(|reason| error(source, reason.to_string()))?;
                    continue;
                }
                if *index == VARIANT_PATH {
                    let variant = components
                        .next()
                        .ok_or_else(|| error(source, "missing callable variant index"))?;
                    let field = components
                        .next()
                        .ok_or_else(|| error(source, "missing callable payload field"))?;
                    let definition = package
                        .and_then(|p| p.enums.iter().find(|e| e.declaration == *declaration))
                        .ok_or_else(|| {
                            error(source, "callable variant path requires enum metadata")
                        })?;
                    let variants = callable_variant_layout(definition, arguments, fuel, source)?;
                    current = variants
                        .get(*variant)
                        .and_then(|v| v.fields.get(*field))
                        .cloned()
                        .ok_or_else(|| {
                            error(source, "callable variant field index out of bounds")
                        })?;
                    continue;
                }
                let definition = package
                    .and_then(|p| p.structs.iter().find(|s| s.declaration == *declaration))
                    .ok_or_else(|| {
                        error(
                            source,
                            "temporary projection requires known struct metadata",
                        )
                    })?;
                definition
                    .instantiate_fields(arguments)
                    .map_err(|e| error(source, format!("temporary projection fields: {e:?}")))?
                    .into_iter()
                    .map(|(_, ty)| ty)
                    .collect()
            }
            _ => {
                return Err(error(
                    source,
                    "temporary projection has no owned field layout",
                ));
            }
        };
        current = fields
            .get(*index)
            .cloned()
            .ok_or_else(|| error(source, "temporary projection field index out of bounds"))?;
    }
    Ok(current)
}

fn callable_source_depth(
    expr: &Expr,
    flow: &Flow,
    fuel: &mut usize,
    depth: usize,
) -> Result<Option<Callable>, Error> {
    if *fuel == 0 || depth >= 128 {
        return Err(error(expr.source, "closure provenance budget exhausted"));
    }
    *fuel -= 1;
    match &expr.kind {
        K::Closure { captures, .. } => {
            let mut known = BTreeMap::new();
            for capture in captures {
                let lexical;
                let value = if let Some(value) = &capture.value {
                    value.as_ref()
                } else {
                    lexical = Expr {
                        source: expr.source,
                        ty: capture.ty.clone(),
                        kind: K::Local(capture.binding),
                    };
                    &lexical
                };
                if let Some(callable) = callable_source_depth(value, flow, fuel, depth + 1)? {
                    known.insert(
                        Place {
                            root: root(capture.binding),
                            fields: vec![],
                        },
                        callable,
                    );
                }
                if let Some(source) = expression_place(value) {
                    for (place, callable) in &flow.callable_fields {
                        *fuel = fuel
                            .checked_sub(place.fields.len().saturating_add(1))
                            .ok_or_else(|| error(expr.source, "capture field budget exhausted"))?;
                        if place.root == source.root && place.fields.starts_with(&source.fields) {
                            known.insert(
                                Place {
                                    root: root(capture.binding),
                                    fields: place.fields[source.fields.len()..].to_vec(),
                                },
                                callable.clone(),
                            );
                        }
                    }
                }
            }
            Ok(Some(Callable {
                choices: vec![],
                enum_captures: Default::default(),
                definition: expr.clone(),
                alternatives: vec![],
                generation: 0,
                captures: known,
            }))
        }
        K::Function(_) | K::FunctionInstance { .. } => Ok(Some(Callable {
            choices: vec![],
            enum_captures: Default::default(),
            definition: expr.clone(),
            alternatives: vec![],
            generation: 0,
            captures: BTreeMap::new(),
        })),
        K::Local(binding) => Ok(flow.callables.get(&root(*binding)).cloned()),
        K::Field { .. } | K::AnonymousField { .. } => {
            Ok(expression_place(expr).and_then(|place| flow.callable_fields.get(&place).cloned()))
        }
        _ => Ok(None),
    }
}

// Only syntactically pure Boolean trees are folded; no program-state claim is
// obtained from a call, assignment, local, predicate or opaque normal form.

#[cfg(test)]
mod record_shape_tests {
    use super::*;
    #[test]
    fn handler_return_display_keeps_origins_outside_variant_domains() {
        let target = Place {
            root: PlaceId { owner: 7, local: 8 },
            fields: vec![9],
        };
        let mut value = HandlerReturnValue {
            source: SourceRange { start: 2, end: 3 },
            origins: BTreeMap::from([(vec![0], BTreeSet::from([target]))]),
            pointer_guards: BTreeMap::new(),
            callable: None,
            fields: BTreeMap::new(),
            variants: BTreeMap::from([(vec![1], BTreeSet::from([2]))]),
        };
        assert_eq!(
            value.to_string(),
            "(handler-return (source (source 2 3)) (callable) (fields) (variants (domain  1 (tags 2))) (origins (leaf  0 (targets (owned-place (owner 7) (local 8) (fields 9))))) (pointer-guards))"
        );
        let target = value.origins[&vec![0]].first().unwrap().clone();
        value
            .pointer_guards
            .insert(vec![0], BTreeMap::from([(target, vec![(vec![1], 2)])]));
        let dump = value.to_string();
        assert!(dump.contains("(pointer-guards (leaf (path 0) (targets (target (owned-place"));
        assert!(dump.contains("(conditions (guard (path 1) (tag 2)))"));
        let mut balance = 0;
        for byte in dump.bytes() {
            if byte == b'(' {
                balance += 1;
            }
            if byte == b')' {
                balance -= 1;
            }
            assert!(balance >= 0, "{dump}");
        }
        assert_eq!(balance, 0, "{dump}");
    }

    #[test]
    fn sealed_effect_parameter_access_survives_callable_memory_bridge() {
        let source = SourceRange { start: 0, end: 1 };
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let pointer = Term::Pointer(Box::new(boolean.clone()));
        let declaration = nir::DeclarationId {
            package: 0,
            local: 1,
        };
        let effect = nir::DeclarationId {
            package: 0,
            local: 2,
        };
        let operation = nir::effects::Operation {
            declaration: effect,
            arguments: vec![],
        };
        let parameter = LocalId { owner: 1, index: 0 };
        let parameters = vec![nir::Parameter {
            sort: nir::Sort::Runtime,
            ty: pointer.clone(),
        }];
        let signature = Term::Function {
            parameters: parameters.clone(),
            result: Box::new(boolean.clone()),
        };
        let computation = Term::Computation {
            effects: vec![operation.clone()],
            result: Box::new(boolean.clone()),
        };
        let mut package = Package::default();
        package.effects.push(nir::effects::Declaration {
            declaration: effect,
            group: None,
            name: "touch".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            signature: signature.clone(),
            abi: nir::effects::Abi::Direct,
            inline: false,
            requirements: vec![],
            capabilities: vec![],
        });
        package.functions.push(Function {
            phase: FunctionPhase::Runtime,
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
            name: "subject".into(),
            parameters: vec![parameter],
            defaults: vec![],
            signature: Term::Function {
                parameters,
                result: Box::new(computation.clone()),
            },
            body: Expr {
                source,
                ty: computation,
                kind: K::EffectComputation {
                    effects: vec![operation.clone()],
                    body: Box::new(Expr {
                        source,
                        ty: boolean,
                        kind: K::EffectRequest {
                            operation: operation.clone(),
                            abi: nir::effects::Abi::Direct,
                            allowed_index: 0,
                            signature: Box::new(signature.clone()),
                            arguments: vec![Expr {
                                source,
                                ty: pointer,
                                kind: K::Local(parameter),
                            }],
                        },
                    }),
                },
            },
        });
        // Unit-level checked evidence exercises the bridge independently of
        // source installer derivation, which remains a separate prerequisite.
        let mut evidence = EffectResultInput {
            owned_inputs: false,
            slot: 0,
            operation,
            abi: nir::effects::Abi::Direct,
            signature,
            values: vec![],
            forwarded_parameters: None,
            copy_parameters: BTreeSet::from([0]),
            parameter_accesses: BTreeSet::from([super::super::memory::EffectParameterAccess {
                parameter: 0,
                input_fields: vec![],
                read_value: true,
                write: true,
            }]),
        };
        let infer = |evidence: &EffectResultInput, fuel: &mut usize| {
            infer_callable_memory_with_effect_context(
                &package,
                declaration,
                &[],
                &[],
                std::slice::from_ref(evidence),
                fuel,
                source,
            )
        };
        assert!(infer(&evidence, &mut 0).is_err());
        let summary = infer(&evidence, &mut 100_000).unwrap();
        assert_eq!(summary.accesses.len(), 1);
        assert_eq!(summary.accesses[0].target.parameter, 0);
        assert!(summary.accesses[0].read_value && summary.accesses[0].write);
        assert!(
            evidence
                .to_string()
                .contains("(parameter-accesses (parameter-access")
        );
        evidence.parameter_accesses.clear();
        assert!(infer(&evidence, &mut 100_000).unwrap().accesses.is_empty());
    }

    #[test]
    fn enum_domains_join_only_under_established_guards() {
        let source = SourceRange { start: 0, end: 0 };
        let nested = vec![VARIANT_PATH, 0, 0];
        let left = EnumDomains::from([
            (vec![], BTreeSet::from([0])),
            (nested.clone(), BTreeSet::from([2])),
        ]);
        let right = EnumDomains::from([(vec![], BTreeSet::from([1]))]);
        assert!(join_enum_domains(&left, &right, &mut 0, source).is_err());
        let joined = join_enum_domains(&left, &right, &mut 100, source).unwrap();
        assert_eq!(joined.get(&vec![]), Some(&BTreeSet::from([0, 1])));
        assert_eq!(joined.get(&nested), Some(&BTreeSet::from([2])));
        let mut absent = nested.clone();
        absent.extend([VARIANT_PATH, 3, 0]);
        let mut present = nested;
        present.extend([VARIANT_PATH, 2, 0]);
        assert!(enum_path_excluded(&absent, &joined));
        assert!(!enum_path_excluded(&present, &joined));
        assert!(
            join_enum_domains(&left, &EnumDomains::new(), &mut 100, source)
                .unwrap()
                .is_empty()
        );
        // The same outer tag with an unknown nested tag does not exclude it.
        let same = EnumDomains::from([(vec![], BTreeSet::from([0]))]);
        assert_eq!(
            join_enum_domains(&left, &same, &mut 100, source).unwrap(),
            same
        );
    }
    #[test]
    fn enum_callable_dumps_name_variant_guards() {
        let definition = Expr {
            source: SourceRange { start: 0, end: 0 },
            ty: Term::Primitive(nir::Primitive::Void),
            kind: K::Unit,
        };
        let evidence = CallableEvidence {
            choices: vec![],
            enum_captures: Default::default(),
            definition: definition.clone(),
            alternatives: vec![],
            captures: BTreeMap::new(),
        };
        let parameter = CallableParameter {
            choices: vec![],
            enum_captures: Default::default(),
            parameter: 0,
            fields: vec![2, VARIANT_PATH, 1, 0],
            definition,
            alternatives: vec![],
            captures: BTreeMap::from([(
                Place {
                    root: PlaceId { owner: 3, local: 4 },
                    fields: vec![VARIANT_PATH, 2, 0],
                },
                evidence,
            )]),
        };
        let dump = parameter.to_string();
        assert!(dump.contains("(fields 2 (variant 1) 0)"), "{dump}");
        assert!(
            dump.contains("(owned-place (owner 3) (local 4) (fields (variant 2) 0))"),
            "{dump}"
        );
        assert!(!dump.contains(&usize::MAX.to_string()), "{dump}");
    }
    #[test]
    fn newtype_return_capture_layout_is_bounded_and_preserves_pointer_rejection() {
        let source = SourceRange { start: 0, end: 0 };
        let declaration = nir::DeclarationId {
            package: 0,
            local: 100,
        };
        let ty = Term::Nominal {
            declaration,
            arguments: vec![],
        };
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let mut package = Package::default();
        package.newtypes.push(nir::newtypes::Newtype {
            capture_count: 0,
            parameter_types: vec![],
            declaration,
            name: "Flag".into(),
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            representation: boolean.clone(),
        });
        assert!(
            recoverable_return_capture_layout(Some(&package), &ty, false, &mut 100, source, 0)
                .unwrap()
        );
        assert!(
            recoverable_return_capture_layout(Some(&package), &ty, false, &mut 0, source, 0)
                .is_err()
        );
        package.newtypes[0].representation = Term::Pointer(Box::new(boolean.clone()));
        assert!(
            !recoverable_return_capture_layout(Some(&package), &ty, false, &mut 100, source, 0)
                .unwrap()
        );
        assert!(
            recoverable_return_capture_layout(Some(&package), &ty, true, &mut 100, source, 0)
                .unwrap()
        );
        package.newtypes[0].representation = ty.clone();
        assert!(
            recoverable_return_capture_layout(Some(&package), &ty, false, &mut 100, source, 0)
                .is_err()
        );
        package.newtypes[0].representation = boolean;
        assert!(
            recoverable_return_capture_layout(Some(&package), &ty, false, &mut 100, source, 0)
                .unwrap()
        );
    }

    #[test]
    fn enum_evidence_layout_validates_tags_fields_substitution_and_budget() {
        let source = SourceRange { start: 0, end: 0 };
        let declaration = nir::DeclarationId {
            package: 0,
            local: 0,
        };
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let mut package = Package::default();
        package.enums.push(nir::program::Enum {
            parameter_types: vec![],
            groups: vec![],
            encoding: None,
            is_intrinsic: false,
            symbol_encoding: None,
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration,
            name: "Choice".into(),
            variants: vec![
                nir::program::Variant {
                    name: "empty".into(),
                    shape: nir::program::VariantShape::Unit,
                    fields: vec![],
                },
                nir::program::Variant {
                    name: "value".into(),
                    shape: nir::program::VariantShape::Tuple,
                    fields: vec![Term::Tuple(vec![boolean.clone()])],
                },
            ],
        });
        let mut empty_variants = package.enums[0].clone();
        for variant in &mut empty_variants.variants {
            variant.fields.clear();
        }
        assert!(callable_variant_layout(&empty_variants, &[], &mut 1, source).is_err());
        assert_eq!(
            callable_variant_layout(&empty_variants, &[], &mut 2, source)
                .unwrap()
                .len(),
            2
        );
        let ty = Term::Nominal {
            declaration,
            arguments: vec![],
        };
        let path = [VARIANT_PATH, 1, 0, 0];
        assert!(owned_field_type(Some(&package), &ty, &path, &mut 0, source).is_err());
        assert_eq!(
            owned_field_type(Some(&package), &ty, &path, &mut 100, source).unwrap(),
            boolean
        );
        for path in [
            vec![0],
            vec![VARIANT_PATH],
            vec![VARIANT_PATH, 1],
            vec![VARIANT_PATH, 0, 0],
            vec![VARIANT_PATH, 99, 0],
            vec![VARIANT_PATH, 1, 99],
        ] {
            assert!(
                owned_field_type(Some(&package), &ty, &path, &mut 100, source).is_err(),
                "{path:?}"
            );
        }
        let invalid_arguments = Term::Nominal {
            declaration,
            arguments: vec![boolean],
        };
        assert!(
            owned_field_type(Some(&package), &invalid_arguments, &path, &mut 100, source).is_err()
        );
    }
    #[test]
    fn enum_callable_paths_require_the_matching_tag_and_preserve_siblings() {
        let source = SourceRange { start: 0, end: 0 };
        let callable = Callable {
            choices: vec![],
            enum_captures: Default::default(),
            definition: Expr {
                source,
                ty: Term::Primitive(nir::Primitive::Void),
                kind: K::Unit,
            },
            alternatives: vec![],
            generation: 0,
            captures: BTreeMap::new(),
        };
        let leaves = BTreeMap::from([
            (vec![0, VARIANT_PATH, 0, 0], callable.clone()),
            (vec![0, VARIANT_PATH, 1, 0], callable.clone()),
            (
                vec![0, VARIANT_PATH, 1, 1, VARIANT_PATH, 2, 0],
                callable.clone(),
            ),
            (vec![1, VARIANT_PATH, 0, 0], callable.clone()),
            // An unqualified path is not a proof for any enum payload.
            (vec![0, 2], callable),
        ]);
        assert!(select_variant_callables(&leaves, &[0], 1, &mut 0, source).is_err());
        let selected = select_variant_callables(&leaves, &[0], 1, &mut 100, source).unwrap();
        assert_eq!(
            selected.keys().cloned().collect::<Vec<_>>(),
            vec![
                vec![0, 0],
                vec![0, 1, VARIANT_PATH, 2, 0],
                vec![1, VARIANT_PATH, 0, 0],
            ]
        );
        let nested = select_variant_callables(&selected, &[0, 1], 2, &mut 100, source).unwrap();
        assert!(nested.contains_key(&vec![0, 1, 0]));
        assert!(!nested.contains_key(&vec![0, 1, VARIANT_PATH, 2, 0]));
        let wrong = select_variant_callables(&leaves, &[0], 3, &mut 100, source).unwrap();
        assert_eq!(
            wrong.keys().cloned().collect::<Vec<_>>(),
            vec![vec![1, VARIANT_PATH, 0, 0]]
        );
    }
    #[test]
    fn record_partial_moves_preserve_siblings_and_reinitialization() {
        let scalar = Term::Primitive(nir::Primitive::Bool);
        let ty = Term::Record(vec![
            ("first".into(), scalar.clone()),
            (
                "nested".into(),
                Term::Record(vec![("second".into(), scalar)]),
            ),
        ]);
        let shape = ownership_shape(&nir::program::Package::default(), &ty, &mut 1000, 0).unwrap();
        let root = PlaceId { owner: 1, local: 0 };
        let parent = Place {
            root,
            fields: vec![],
        };
        let first = Place {
            root,
            fields: vec![0],
        };
        let second = Place {
            root,
            fields: vec![1, 0],
        };
        let mut state = State::default();
        state.declare(root, &shape, true, &mut 1000).unwrap();
        state.take(&second).unwrap();
        assert!(state.require_present(&parent).is_err());
        state.require_present(&first).unwrap();
        assert!(state.take(&second).is_err());
        state.install(&second).unwrap();
        state.take(&parent).unwrap();
        assert!(state.remaining().is_empty());
    }
    #[test]
    fn empty_record_still_owns_a_responsibility_and_shape_budgets_recover() {
        let package = nir::program::Package::default();
        let ty = Term::Record(vec![]);
        assert!(ownership_shape(&package, &ty, &mut 0, 0).is_err());
        let shape = ownership_shape(&package, &ty, &mut 100, 0).unwrap();
        let root = PlaceId { owner: 1, local: 0 };
        let mut state = State::default();
        state.declare(root, &shape, true, &mut 100).unwrap();
        assert_eq!(state.remaining().len(), 1);
        state
            .take(&Place {
                root,
                fields: vec![],
            })
            .unwrap();
        assert!(state.remaining().is_empty());
    }
}
