//! Finite storage checks over completed declaration descriptions. Normal-form
//! heads stay opaque except for the configured language Tuple constructor.
use nir::{DeclarationId, Term, program as n};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    Budget,
    Invalid,
    Stale,
}

/// Resource limits are retryable outcomes, not evidence of an invalid layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    message: &'static str,
}
impl Failure {
    const fn budget(message: &'static str) -> Self {
        Self {
            kind: FailureKind::Budget,
            message,
        }
    }
    const fn invalid(message: &'static str) -> Self {
        Self {
            kind: FailureKind::Invalid,
            message,
        }
    }
    const fn stale(message: &'static str) -> Self {
        Self {
            kind: FailureKind::Stale,
            message,
        }
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}
impl std::error::Error for Failure {}

enum Description<'a, T> {
    Borrowed(&'a T),
    Shared(Arc<[T]>, usize),
}
impl<T> Description<'_, T> {
    fn get(&self) -> &T {
        match self {
            Self::Borrowed(value) => value,
            Self::Shared(batch, index) => &batch[*index],
        }
    }
}

enum Descriptions<'a, T> {
    Table(&'a BTreeMap<DeclarationId, T>),
    References(BTreeMap<DeclarationId, &'a T>),
    Shared(BTreeMap<DeclarationId, Description<'a, T>>),
    Overlay(&'a Descriptions<'a, T>, BTreeMap<DeclarationId, &'a T>),
}
impl<T> Descriptions<'_, T> {
    fn get(&self, id: &DeclarationId) -> Option<&T> {
        match self {
            Self::Table(table) => table.get(id),
            Self::References(table) => table.get(id).copied(),
            Self::Shared(table) => table.get(id).map(Description::get),
            Self::Overlay(base, additions) => additions.get(id).copied().or_else(|| base.get(id)),
        }
    }
}

/// The borrowed world cannot change while successful roots are memoized.
/// Failed roots leave no active traversal behind and can be checked again.
pub struct Checker<'a> {
    tuple: Option<&'a nir::normal_forms::Head>,
    structs: Descriptions<'a, n::Struct>,
    enums: Descriptions<'a, n::Enum>,
    newtypes: Descriptions<'a, nir::newtypes::Newtype>,
    done: HashSet<Term>,
    prior_done: Option<&'a HashSet<Term>>,
}
impl<'a> Checker<'a> {
    pub fn new(
        tuple: Option<&'a nir::normal_forms::Head>,
        structs: &'a BTreeMap<DeclarationId, n::Struct>,
        enums: &'a BTreeMap<DeclarationId, n::Enum>,
        newtypes: &'a BTreeMap<DeclarationId, nir::newtypes::Newtype>,
    ) -> Self {
        Self {
            tuple,
            structs: Descriptions::Table(structs),
            enums: Descriptions::Table(enums),
            newtypes: Descriptions::Table(newtypes),
            done: HashSet::new(),
            prior_done: None,
        }
    }

    /// Combine immutable baseline and generated descriptions without cloning
    /// their field trees. The caller precharges construction of these indices
    /// and admits declaration identities before entering this representation check.
    pub fn from_references(
        tuple: Option<&'a nir::normal_forms::Head>,
        structs: BTreeMap<DeclarationId, &'a n::Struct>,
        enums: BTreeMap<DeclarationId, &'a n::Enum>,
        newtypes: BTreeMap<DeclarationId, &'a nir::newtypes::Newtype>,
    ) -> Self {
        Self {
            tuple,
            structs: Descriptions::References(structs),
            enums: Descriptions::References(enums),
            newtypes: Descriptions::References(newtypes),
            done: HashSet::new(),
            prior_done: None,
        }
    }

    pub fn check(&mut self, ty: &Term, fuel: &mut usize) -> Result<(), Failure> {
        self.visit(ty, &mut HashSet::new(), fuel, 0)
    }

    fn visit(
        &mut self,
        ty: &Term,
        active: &mut HashSet<Term>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<(), Failure> {
        const BUDGET: Failure = Failure::budget("struct representation expansion budget exhausted");
        *fuel = fuel.checked_sub(1).ok_or(BUDGET)?;
        if depth >= 128 {
            return Err(BUDGET);
        }
        // Account for hashing/cloning a growing generic application before it
        // reaches the active/done tables. This is the caller's shared budget.
        nir::type_computations::charge_term(ty, fuel).map_err(|_| BUDGET)?;
        match ty {
            Term::Nominal {
                declaration,
                arguments,
            } if !self.done.contains(ty)
                && !self.prior_done.is_some_and(|done| done.contains(ty)) =>
            {
                if !active.insert(ty.clone()) {
                    return Err(Failure::invalid(
                        "recursive by-value struct representation or enum payload",
                    ));
                }
                if let Some(def) = self.structs.get(declaration) {
                    let substitution = nir::DeclarationSubstitution::bounded(
                        &def.type_parameters,
                        &def.parameter_types,
                        arguments,
                        &[],
                        fuel,
                    )
                    .map_err(|failure| match failure {
                        nir::type_computations::Error::Scope(_) => {
                            Failure::invalid("invalid struct type arguments in representation")
                        }
                        _ => BUDGET,
                    })?;
                    let fields = def
                        .fields
                        .iter()
                        .map(|(_, field)| {
                            substitution
                                .instantiate_bounded(field, fuel)
                                .map_err(|failure| match failure {
                                    nir::type_computations::Error::Scope(_) => {
                                        Failure::invalid("invalid struct field in representation")
                                    }
                                    _ => BUDGET,
                                })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    for field in fields {
                        self.visit(&field, active, fuel, depth + 1)?;
                    }
                } else if let Some(def) = self.enums.get(declaration) {
                    let substitution = nir::DeclarationSubstitution::bounded(
                        &def.type_parameters,
                        &def.parameter_types,
                        arguments,
                        &[],
                        fuel,
                    )
                    .map_err(|failure| match failure {
                        nir::type_computations::Error::Scope(_) => {
                            Failure::invalid("invalid enum type arguments in representation")
                        }
                        _ => BUDGET,
                    })?;
                    let fields = def
                        .variants
                        .iter()
                        .flat_map(|v| &v.fields)
                        .map(|field| {
                            substitution
                                .instantiate_bounded(field, fuel)
                                .map_err(|failure| match failure {
                                    nir::type_computations::Error::Scope(_) => {
                                        Failure::invalid("invalid enum payload in representation")
                                    }
                                    _ => BUDGET,
                                })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    for field in fields {
                        self.visit(&field, active, fuel, depth + 1)?;
                    }
                } else if let Some(def) = self.newtypes.get(declaration) {
                    let representation = def
                        .representation_budgeted(arguments, &[], fuel)
                        .map_err(|failure| match failure {
                            nir::newtypes::ConversionError::Budget => BUDGET,
                            nir::newtypes::ConversionError::Type(_)
                            | nir::newtypes::ConversionError::DuplicateIdentity(_) => {
                                Failure::invalid("invalid newtype representation")
                            }
                        })?;
                    self.visit(&representation, active, fuel, depth + 1)?;
                } else {
                    return Err(Failure::invalid(
                        "nominal declaration is missing from representation world",
                    ));
                }
                active.remove(ty);
                self.done.insert(ty.clone());
                Ok(())
            }
            Term::NormalForm { .. } => {
                if let Some(items) = self.tuple.and_then(|head| head.tuple_elements(ty)) {
                    for item in items {
                        self.visit(item, active, fuel, depth + 1)?;
                    }
                }
                Ok(())
            }
            Term::Tuple(items)
            | Term::Closure {
                environment: items, ..
            } => {
                for item in items {
                    self.visit(item, active, fuel, depth + 1)?;
                }
                Ok(())
            }
            Term::Record(fields) => {
                for (_, field) in fields {
                    self.visit(field, active, fuel, depth + 1)?;
                }
                Ok(())
            }
            Term::Optional(inner) => self.visit(inner, active, fuel, depth + 1),
            Term::ErrorResult { errors, result } => {
                self.visit(result, active, fuel, depth + 1)?;
                for label in errors {
                    self.visit(label, active, fuel, depth + 1)?;
                }
                Ok(())
            }
            // Pointer and function signatures do not embed their referents or
            // parameter/result values. Other normal-form heads are not unfolded.
            _ => Ok(()),
        }
    }
}

/// Append-only representation world for checked, concrete builder results.
/// A candidate borrows the committed indices and memoized proofs; its own proof
/// cache remains isolated until publication. Failed or dropped attempts cannot
/// teach the committed world anything about a replaceable candidate identity.
pub struct IncrementalChecker<'a> {
    world: Checker<'a>,
    identity: Arc<()>,
    revision: u64,
}

pub struct PreparedRepresentations {
    definitions: Arc<[n::Struct]>,
    enums: Arc<[n::Enum]>,
    done: HashSet<Term>,
    identity: Arc<()>,
    revision: u64,
}

impl<'a> IncrementalChecker<'a> {
    pub fn new(package: &'a n::Package, fuel: &mut usize) -> Result<Self, Failure> {
        *fuel = fuel
            .checked_sub(
                package
                    .structs
                    .len()
                    .saturating_add(package.enums.len())
                    .saturating_add(package.newtypes.len())
                    .saturating_add(package.normal_forms.len())
                    .saturating_add(1),
            )
            .ok_or(Failure::budget(
                "struct representation index budget exhausted",
            ))?;
        Ok(Self {
            world: Checker {
                tuple: package
                    .normal_forms
                    .iter()
                    .find(|head| head.role == nir::normal_forms::Role::Tuple),
                structs: Descriptions::Shared(
                    package
                        .structs
                        .iter()
                        .map(|def| (def.declaration, Description::Borrowed(def)))
                        .collect(),
                ),
                enums: Descriptions::Shared(
                    package
                        .enums
                        .iter()
                        .map(|def| (def.declaration, Description::Borrowed(def)))
                        .collect(),
                ),
                newtypes: Descriptions::References(
                    package
                        .newtypes
                        .iter()
                        .map(|def| (def.declaration, def))
                        .collect(),
                ),
                done: HashSet::new(),
                prior_done: None,
            },
            identity: Arc::new(()),
            revision: 0,
        })
    }

    /// Declaration admission must precede this check. This layer checks finite
    /// storage only; it does not authorize field types, scope or trait evidence.
    pub fn prepare(
        &self,
        definitions: Arc<[n::Struct]>,
        fuel: &mut usize,
    ) -> Result<PreparedRepresentations, Failure> {
        self.prepare_batch(definitions, Arc::from([]), fuel)
    }

    /// Check a mixed concrete batch in one tentative world. Forward edges may
    /// cross declaration kinds; no successful subroot escapes a rejected batch.
    /// Ordinary declaration admission is still a separate prerequisite.
    pub fn prepare_batch(
        &self,
        definitions: Arc<[n::Struct]>,
        enums: Arc<[n::Enum]>,
        fuel: &mut usize,
    ) -> Result<PreparedRepresentations, Failure> {
        *fuel = fuel
            .checked_sub(
                definitions
                    .len()
                    .saturating_add(enums.len())
                    .saturating_mul(3)
                    .saturating_add(1),
            )
            .ok_or(Failure::budget(
                "struct representation index budget exhausted",
            ))?;
        let mut additions = BTreeMap::new();
        for def in definitions.iter() {
            if self.world.structs.get(&def.declaration).is_some()
                || self.world.enums.get(&def.declaration).is_some()
                || self.world.newtypes.get(&def.declaration).is_some()
                || additions.insert(def.declaration, def).is_some()
            {
                return Err(Failure::invalid("struct representation identity collision"));
            }
        }
        let mut enum_additions = BTreeMap::new();
        for def in enums.iter() {
            if self.world.structs.get(&def.declaration).is_some()
                || self.world.enums.get(&def.declaration).is_some()
                || self.world.newtypes.get(&def.declaration).is_some()
                || additions.contains_key(&def.declaration)
                || enum_additions.insert(def.declaration, def).is_some()
            {
                return Err(Failure::invalid("enum representation identity collision"));
            }
        }
        let mut candidate = Checker {
            tuple: self.world.tuple,
            structs: Descriptions::Overlay(&self.world.structs, additions),
            enums: Descriptions::Overlay(&self.world.enums, enum_additions),
            newtypes: Descriptions::Overlay(&self.world.newtypes, BTreeMap::new()),
            done: HashSet::new(),
            prior_done: Some(&self.world.done),
        };
        for declaration in definitions
            .iter()
            .map(|def| def.declaration)
            .chain(enums.iter().map(|def| def.declaration))
        {
            candidate.check(
                &Term::Nominal {
                    declaration,
                    arguments: vec![],
                },
                fuel,
            )?;
        }
        // Commit hashes these immutable proofs into the persistent cache.
        // Reserve that work before issuing a receipt, even if it is dropped.
        for ty in &candidate.done {
            nir::type_computations::charge_term(ty, fuel)
                .map_err(|_| Failure::budget("struct representation memo budget exhausted"))?;
        }
        let done = std::mem::take(&mut candidate.done);
        drop(candidate);
        Ok(PreparedRepresentations {
            definitions,
            enums,
            done,
            identity: Arc::clone(&self.identity),
            revision: self.revision,
        })
    }

    pub fn commit(&mut self, prepared: PreparedRepresentations) -> Result<(), Failure> {
        let next = self.revision.checked_add(1);
        if !Arc::ptr_eq(&self.identity, &prepared.identity)
            || prepared.revision != self.revision
            || next.is_none()
        {
            return Err(Failure::stale(
                "struct representation candidate is stale or belongs to another world",
            ));
        }
        let Descriptions::Shared(table) = &mut self.world.structs else {
            unreachable!()
        };
        for (index, def) in prepared.definitions.iter().enumerate() {
            table.insert(
                def.declaration,
                Description::Shared(Arc::clone(&prepared.definitions), index),
            );
        }
        let Descriptions::Shared(table) = &mut self.world.enums else {
            unreachable!()
        };
        for (index, def) in prepared.enums.iter().enumerate() {
            table.insert(
                def.declaration,
                Description::Shared(Arc::clone(&prepared.enums), index),
            );
        }
        self.world.done.extend(prepared.done);
        self.revision = next.unwrap();
        Ok(())
    }
}

impl std::fmt::Display for IncrementalChecker<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(representation-world (revision {}) (proved {}))",
            self.revision,
            self.world.done.len()
        )
    }
}
impl std::fmt::Display for PreparedRepresentations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(prepared-representations (revision {}) (definitions",
            self.revision
        )?;
        for def in self.definitions.iter() {
            write!(f, " {}", def.declaration)?;
        }
        f.write_str(") (enums")?;
        for def in self.enums.iter() {
            write!(f, " {}", def.declaration)?;
        }
        write!(f, ") (proved {}))", self.done.len())
    }
}
