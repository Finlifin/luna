//! Structural implementation indexing, type-only applicability and selection.
//! Type matching and requires produce replayable candidate-local evidence.
//! Phase, coherence, associated members and runtime environment delivery remain
//! separate obligations: type selection alone is not trait satisfaction.
use crate::ptc::Budget;
mod applicability;
mod coherence;
mod inherent;
pub use coherence::{CoherenceReport, PairAssessment};
pub use inherent::{InherentHead, InherentIndex};
mod ownership;
pub use applicability::{HeadMatch, TypeAssessment, TypeEvidence, TypeResolution};
use nir::{DeclarationId, Sort, Term};
pub use ownership::{Ownership, OwnershipError, inherent as inherent_ownership};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Head {
    Nominal(DeclarationId),
    Tuple,
    Record(Vec<String>),
    Pointer,
    ComptimeFunction,
    Function(Vec<Sort>),
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Wildcard,
    Leaf(Term),
    Node(Head, Vec<Key>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    BudgetExhausted,
    DuplicateDeclaration(DeclarationId),
    Ownership(OwnershipError),
}
fn spend(fuel: &mut usize) -> Result<(), Error> {
    *fuel = fuel.checked_sub(1).ok_or(Error::BudgetExhausted)?;
    Ok(())
}
impl Key {
    /// Rigid parameters and unresolved environments remain wildcard positions.
    /// Opaque normal forms are identity leaves only after all their inputs are
    /// stable. They are never decomposed into invertible constructor arguments.
    pub fn from_type(ty: &Term, budget: Budget) -> Result<Self, Error> {
        let mut fuel = budget.steps;
        Self::with_fuel(ty, &mut fuel, budget.max_depth)
    }
    fn with_fuel(ty: &Term, fuel: &mut usize, max_depth: usize) -> Result<Self, Error> {
        fn build(ty: &Term, fuel: &mut usize, depth: usize, limit: usize) -> Result<Key, Error> {
            spend(fuel)?;
            if depth >= limit {
                return Err(Error::BudgetExhausted);
            }
            let (head, children): (Head, Vec<&Term>) = match ty {
                Term::Primitive(_)
                | Term::Unit
                | Term::Bool(_)
                | Term::Integer(_)
                | Term::Float(_)
                | Term::BitVec(_)
                | Term::Float32(_)
                | Term::Float64(_)
                | Term::Char(_)
                | Term::String(_)
                | Term::Symbol(_)
                | Term::Runtime(_)
                | Term::Value { .. } => {
                    return Ok(Key::Leaf(ty.clone()));
                }
                Term::Nominal {
                    declaration,
                    arguments,
                } => {
                    if arguments.len() > *fuel {
                        return Err(Error::BudgetExhausted);
                    }
                    (Head::Nominal(*declaration), arguments.iter().collect())
                }
                Term::Tuple(items) => {
                    if items.len() > *fuel {
                        return Err(Error::BudgetExhausted);
                    }
                    (Head::Tuple, items.iter().collect())
                }
                Term::Pointer(inner) => (Head::Pointer, vec![inner]),
                Term::Record(fields) => {
                    let work = fields
                        .iter()
                        .try_fold(fields.len(), |total, (name, _)| {
                            total.checked_add(name.len())
                        })
                        .ok_or(Error::BudgetExhausted)?;
                    *fuel = fuel.checked_sub(work).ok_or(Error::BudgetExhausted)?;
                    (
                        Head::Record(fields.iter().map(|(name, _)| name.clone()).collect()),
                        fields.iter().map(|(_, ty)| ty).collect(),
                    )
                }
                Term::ComptimeFunction(inner) => (Head::ComptimeFunction, vec![inner]),
                Term::Function { parameters, result } => (
                    Head::Function(parameters.iter().map(|p| p.sort).collect()),
                    parameters
                        .iter()
                        .map(|p| &p.ty)
                        .chain(std::iter::once(result.as_ref()))
                        .collect(),
                ),
                Term::NominalValue { .. }
                | Term::NormalForm { .. }
                | Term::TypeListValue(_)
                | Term::TypeListConcat(_)
                | Term::TupleValue(_)
                | Term::RecordValue(_) => {
                    // Validate and bound the complete opaque value before cloning.
                    let mut work = vec![(ty, depth)];
                    let mut open = false;
                    while let Some((term, d)) = work.pop() {
                        spend(fuel)?;
                        if d >= limit {
                            return Err(Error::BudgetExhausted);
                        }
                        match term {
                            Term::AssociatedType {
                                arguments, subject, ..
                            } => {
                                // Even a closed query can resolve to a different
                                // structural head; it cannot prune candidates.
                                open = true;
                                if arguments.len().saturating_add(1) > *fuel {
                                    return Err(Error::BudgetExhausted);
                                }
                                work.push((subject, d + 1));
                                work.extend(arguments.iter().map(|t| (t, d + 1)));
                            }
                            Term::Rigid { .. } | Term::Bound { .. } | Term::Hole { .. } => {
                                open = true
                            }
                            Term::NormalForm { arguments, .. }
                            | Term::Nominal { arguments, .. }
                            | Term::Tuple(arguments)
                            | Term::TypeListValue(arguments)
                            | Term::TypeListConcat(arguments)
                            | Term::TupleValue(arguments) => {
                                if arguments.len() > *fuel {
                                    return Err(Error::BudgetExhausted);
                                }
                                work.extend(arguments.iter().map(|t| (t, d + 1)))
                            }
                            Term::NominalValue { ty, fields, .. } => {
                                if fields.len().saturating_add(1) > *fuel {
                                    return Err(Error::BudgetExhausted);
                                }
                                work.push((ty, d + 1));
                                work.extend(fields.iter().map(|field| (field, d + 1)));
                            }
                            Term::Record(fields) | Term::RecordValue(fields) => {
                                if fields.len() > *fuel {
                                    return Err(Error::BudgetExhausted);
                                }
                                for (name, ty) in fields {
                                    *fuel = fuel
                                        .checked_sub(name.len())
                                        .ok_or(Error::BudgetExhausted)?;
                                    work.push((ty, d + 1));
                                }
                            }
                            Term::Pointer(t) | Term::ComptimeFunction(t) => work.push((t, d + 1)),
                            // Dependent function/closure structure is not an
                            // implementation index contract yet. Do not prune it.
                            Term::Function { .. }
                            | Term::Closure { .. }
                            | Term::Computation { .. } => {
                                return Ok(Key::Wildcard);
                            }
                            _ => {}
                        }
                    }
                    return Ok(if open {
                        Key::Wildcard
                    } else {
                        Key::Leaf(ty.clone())
                    });
                }
                _ => return Ok(Key::Wildcard),
            };
            if children.len() > *fuel {
                return Err(Error::BudgetExhausted);
            }
            Ok(Key::Node(
                head,
                children
                    .into_iter()
                    .map(|t| build(t, fuel, depth + 1, limit))
                    .collect::<Result<_, _>>()?,
            ))
        }
        // Charge the complete payload before any key clones it, including
        // scalar values nested under an opaque normal-form head. Structural
        // search work below has a separate cost in this same budget.
        nir::type_computations::charge_term_with_depth(ty, fuel, max_depth)
            .map_err(|_| Error::BudgetExhausted)?;
        build(ty, fuel, 0, max_depth.min(128))
    }
    pub fn precision(&self) -> usize {
        let mut pending = vec![(self, 0)];
        let mut minimum = None;
        let mut maximum = 0;
        while let Some((key, depth)) = pending.pop() {
            match key {
                Key::Wildcard => minimum = Some(minimum.map_or(depth, |old: usize| old.min(depth))),
                Key::Leaf(_) => maximum = maximum.max(depth + 1),
                Key::Node(_, children) if children.is_empty() => maximum = maximum.max(depth + 1),
                Key::Node(_, children) => {
                    pending.extend(children.iter().map(|child| (child, depth + 1)))
                }
            }
        }
        minimum.unwrap_or(maximum)
    }
    pub fn cut(&self, depth: usize) -> Self {
        if depth == 0 {
            return Self::Wildcard;
        }
        match self {
            Self::Node(head, children) => Self::Node(
                head.clone(),
                children.iter().map(|child| child.cut(depth - 1)).collect(),
            ),
            other => other.clone(),
        }
    }
    /// May overlap, deliberately over-approximating logical equality for opaque
    /// leaves. Unknown queries visit compatible descendants, not just ancestors.
    fn compatible(&self, other: &Self, fuel: &mut usize) -> Result<bool, Error> {
        let mut pending = vec![(self, other)];
        while let Some((a, b)) = pending.pop() {
            spend(fuel)?;
            match (a, b) {
                (Self::Wildcard, _) | (_, Self::Wildcard) => {}
                (
                    Self::Leaf(
                        Term::NormalForm { .. }
                        | Term::TypeListValue(_)
                        | Term::TypeListConcat(_)
                        | Term::TupleValue(_)
                        | Term::Value { .. },
                    ),
                    _,
                )
                | (
                    _,
                    Self::Leaf(
                        Term::NormalForm { .. }
                        | Term::TypeListValue(_)
                        | Term::TypeListConcat(_)
                        | Term::TupleValue(_)
                        | Term::Value { .. },
                    ),
                ) => {}
                (Self::Leaf(a), Self::Leaf(b)) => {
                    // Every candidate comparison reads its own payload. Paying
                    // for the query key once does not bound a whole bucket scan.
                    nir::type_computations::charge_term(a, fuel)
                        .map_err(|_| Error::BudgetExhausted)?;
                    nir::type_computations::charge_term(b, fuel)
                        .map_err(|_| Error::BudgetExhausted)?;
                    if matches!((a, b), (Term::RecordValue(_), Term::RecordValue(_)))
                        && a.is_structural_index_value()
                        && b.is_structural_index_value()
                    {
                        if a.index_type_relations(b).is_none() {
                            return Ok(false);
                        }
                        continue;
                    }
                    if a != b {
                        return Ok(false);
                    }
                }
                (Self::Node(a, aa), Self::Node(b, ba)) => {
                    for head in [a, b] {
                        let work = match head {
                            Head::Record(names) => names
                                .iter()
                                .try_fold(names.len(), |size, name| size.checked_add(name.len()))
                                .ok_or(Error::BudgetExhausted)?,
                            Head::Function(sorts) => sorts.len(),
                            _ => 0,
                        };
                        *fuel = fuel.checked_sub(work).ok_or(Error::BudgetExhausted)?;
                    }
                    if a != b || aa.len() != ba.len() {
                        return Ok(false);
                    }
                    if aa.len() > *fuel {
                        return Err(Error::BudgetExhausted);
                    }
                    pending.extend(aa.iter().zip(ba));
                }
                _ => return Ok(false),
            }
        }
        Ok(true)
    }
    pub(crate) fn is_open(&self) -> bool {
        let mut pending = vec![self];
        while let Some(key) = pending.pop() {
            match key {
                Self::Wildcard => return true,
                Self::Node(_, children) => pending.extend(children),
                _ => {}
            }
        }
        false
    }
}

#[derive(Clone, Debug)]
pub struct ImplementationHead {
    /// Ordered declaration domains; empty denotes all Type parameters.
    pub parameter_types: Vec<Term>,
    pub type_parameters: Vec<DeclarationId>,
    pub requirements: Vec<nir::program::TypePredicate>,
    pub declaration: DeclarationId,
    /// None is an open trait head; it belongs to a supplemental candidate bucket.
    pub interface: Option<DeclarationId>,
    pub interface_arguments: Vec<Term>,
    pub subject: Term,
}
impl ImplementationHead {
    pub fn parameter_binders(&self) -> Result<Vec<nir::DeclarationParameter>, nir::TypeError> {
        nir::declaration_parameters(&self.type_parameters, &self.parameter_types)
    }
}
#[derive(Clone, Debug)]
struct Entry {
    head: ImplementationHead,
    key: Key,
    arguments: Vec<Key>,
    precision: usize,
}
pub struct Index {
    identity: u64,
    entries: BTreeMap<DeclarationId, Entry>,
    buckets: BTreeMap<Option<DeclarationId>, BTreeSet<DeclarationId>>,
    revision: u64,
}
impl Default for Index {
    fn default() -> Self {
        Self {
            identity: crate::unique(),
            entries: BTreeMap::new(),
            buckets: BTreeMap::new(),
            revision: 0,
        }
    }
}
impl Index {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn insert(&mut self, head: ImplementationHead, budget: Budget) -> Result<(), Error> {
        if self.entries.contains_key(&head.declaration) {
            return Err(Error::DuplicateDeclaration(head.declaration));
        }
        let mut fuel = budget.steps;
        // Open interfaces remain provisional in the supplemental bucket. Once
        // fixed, declarations must pass ownership before becoming observable.
        head.ownership_with_fuel(&mut fuel, budget.max_depth)?;
        let key = Key::with_fuel(&head.subject, &mut fuel, budget.max_depth)?;
        let precision = key.precision();
        let arguments = head
            .interface_arguments
            .iter()
            .map(|ty| Key::with_fuel(ty, &mut fuel, budget.max_depth))
            .collect::<Result<_, _>>()?;
        // Publish only after all keys have validated successfully.
        self.buckets
            .entry(head.interface)
            .or_default()
            .insert(head.declaration);
        self.entries.insert(
            head.declaration,
            Entry {
                head,
                key: key.cut(precision),
                arguments,
                precision,
            },
        );
        self.revision += 1;
        Ok(())
    }
    pub fn candidates(
        &self,
        interface: Option<DeclarationId>,
        arguments: &[Term],
        subject: &Term,
        budget: Budget,
    ) -> Result<Candidates, Error> {
        let mut remaining = budget.steps;
        let subject = Key::with_fuel(subject, &mut remaining, budget.max_depth)?;
        let arguments = arguments
            .iter()
            .map(|ty| Key::with_fuel(ty, &mut remaining, budget.max_depth))
            .collect::<Result<Vec<_>, _>>()?;
        let fixed_interface = interface.is_some() && arguments.iter().all(|arg| !arg.is_open());
        let mut candidates = Vec::new();
        let buckets: Vec<_> = match interface {
            Some(id) => [Some(id), None]
                .into_iter()
                .filter_map(|key| self.buckets.get(&key))
                .collect(),
            None => self.buckets.values().collect(),
        };
        for bucket in buckets {
            for id in bucket {
                spend(&mut remaining)?;
                let entry = &self.entries[id];
                if !entry.key.compatible(&subject, &mut remaining)? {
                    continue;
                }
                let mut compatible = true;
                if interface.is_some() && entry.head.interface.is_some() {
                    compatible = entry.arguments.len() == arguments.len();
                    if compatible {
                        for (a, b) in entry.arguments.iter().zip(&arguments) {
                            if !a.compatible(b, &mut remaining)? {
                                compatible = false;
                                break;
                            }
                        }
                    }
                }
                if compatible {
                    candidates.push(*id);
                }
            }
        }
        candidates.sort_by_key(|id| (std::cmp::Reverse(self.entries[id].precision), *id));
        Ok(Candidates {
            identity: self.identity,
            declarations: candidates,
            revision: self.revision,
            fixed_interface,
        })
    }
    /// Assessments are private to each candidate. No candidate may publish a
    /// substitution while a same/higher precision competitor remains pending.
    pub fn select(
        &self,
        candidates: &Candidates,
        mut assess: impl FnMut(&ImplementationHead) -> Applicability,
    ) -> Selection {
        if candidates.identity != self.identity || candidates.revision != self.revision {
            return Selection::StaleIndex;
        }
        if !candidates.fixed_interface {
            return Selection::UnresolvedInterface;
        }
        let mut levels: BTreeMap<usize, BTreeSet<DeclarationId>> = BTreeMap::new();
        for id in &candidates.declarations {
            let Some(entry) = self.entries.get(id) else {
                return Selection::InvalidCandidate(*id);
            };
            levels.entry(entry.precision).or_default().insert(*id);
        }
        for (_, ids) in levels.into_iter().rev() {
            let mut applicable = Vec::new();
            let mut pending = Vec::new();
            for id in ids {
                match assess(&self.entries[&id].head) {
                    Applicability::Applicable => applicable.push(id),
                    Applicability::Pending => pending.push(id),
                    Applicability::Excluded => {}
                }
            }
            if applicable.len() > 1 {
                return Selection::Conflict(applicable);
            }
            if !pending.is_empty() {
                return Selection::Deferred {
                    applicable,
                    pending,
                };
            }
            if let Some(id) = applicable.pop() {
                return Selection::Selected(id);
            }
        }
        Selection::NoApplicableCandidate
    }
}
#[derive(Clone, Debug)]
pub struct Candidates {
    identity: u64,
    declarations: Vec<DeclarationId>,
    revision: u64,
    fixed_interface: bool,
}
impl Candidates {
    pub fn declarations(&self) -> &[DeclarationId] {
        &self.declarations
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applicability {
    Applicable,
    Excluded,
    Pending,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    StaleIndex,
    UnresolvedInterface,
    Selected(DeclarationId),
    Conflict(Vec<DeclarationId>),
    Deferred {
        applicable: Vec<DeclarationId>,
        pending: Vec<DeclarationId>,
    },
    NoApplicableCandidate,
    InvalidCandidate(DeclarationId),
}
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wildcard => f.write_str("(wildcard)"),
            Self::Leaf(ty) => write!(f, "(identity {ty})"),
            Self::Node(head, children) => {
                f.write_str("(structure ")?;
                match head {
                    Head::Nominal(id) => write!(f, "(nominal {id})")?,
                    Head::Tuple => f.write_str("tuple")?,
                    Head::Record(names) => {
                        f.write_str("(record")?;
                        for name in names {
                            write!(f, " {}", Term::String(name.clone().into()))?;
                        }
                        f.write_str(")")?;
                    }
                    Head::Pointer => f.write_str("pointer")?,
                    Head::ComptimeFunction => f.write_str("comptime-function")?,
                    Head::Function(sorts) => {
                        f.write_str("(function")?;
                        for sort in sorts {
                            write!(f, " {sort}")?;
                        }
                        f.write_str(")")?;
                    }
                }
                for child in children {
                    write!(f, " {child}")?;
                }
                f.write_str(")")
            }
        }
    }
}
impl fmt::Display for Index {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(implementation-index (revision {})", self.revision)?;
        for (id, entry) in &self.entries {
            write!(f, " (implementation (id {id}) (interface ")?;
            match entry.head.interface {
                Some(id) => write!(f, "{id}")?,
                None => f.write_str("open")?,
            }
            write!(
                f,
                ") (subject {}) (precision {}) (key {}) (interface-arguments",
                entry.head.subject, entry.precision, entry.key
            )?;
            for arg in &entry.head.interface_arguments {
                write!(f, " {arg}")?;
            }
            f.write_str(") (type-parameters")?;
            for parameter in &entry.head.type_parameters {
                write!(f, " {parameter}")?;
            }
            f.write_str(") (parameter-types")?;
            for domain in &entry.head.parameter_types {
                write!(f, " {domain}")?;
            }
            f.write_str(") (requires")?;
            for requirement in &entry.head.requirements {
                write!(f, " {requirement}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(")")
    }
}

impl fmt::Display for Candidates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(implementation-candidates (revision {}) (fixed-interface {}) (declarations",
            self.revision, self.fixed_interface
        )?;
        for id in &self.declarations {
            write!(f, " {id}")?;
        }
        f.write_str("))")
    }
}
impl fmt::Display for Selection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Selected(id) => write!(f, "(selected {id})"),
            Self::Conflict(ids) => {
                f.write_str("(conflict")?;
                for id in ids {
                    write!(f, " {id}")?;
                }
                f.write_str(")")
            }
            Self::Deferred {
                applicable,
                pending,
            } => {
                f.write_str("(deferred (applicable")?;
                for id in applicable {
                    write!(f, " {id}")?;
                }
                f.write_str(") (pending")?;
                for id in pending {
                    write!(f, " {id}")?;
                }
                f.write_str("))")
            }
            Self::NoApplicableCandidate => f.write_str("(no-applicable-candidate)"),
            Self::InvalidCandidate(id) => write!(f, "(invalid-candidate {id})"),
            Self::StaleIndex => f.write_str("(stale-index)"),
            Self::UnresolvedInterface => f.write_str("(unresolved-interface)"),
        }
    }
}
