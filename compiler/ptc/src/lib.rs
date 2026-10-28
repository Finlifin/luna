//! Type-proposition coordination. No program state, memory predicates, user
//! equality overloads or conversion relations are admitted here.
//!
//! These theory fragments cover type equality, integer difference logic, Boolean combinations,
//! closed structural constructors and explicitly admitted normal-form
//! congruence. Abstract satisfiability is never itself a source witness.
mod effects;
mod error_results;
mod traits;
pub use traits::TraitQuery;
mod classification;
pub use classification::{KindQuery, TypeKind};
mod integer;
pub use integer::{IntLe, IntegerTerm};

use nir::{DeclarationId, Primitive, Sort, Term};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Equality(pub Term, pub Term);
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Atom {
    TypeEqual(Equality),
    IntLe(IntLe),
    Kind(KindQuery),
    Trait(TraitQuery),
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Formula {
    True,
    False,
    Equal(Equality),
    IntLe(IntLe),
    Kind(KindQuery),
    Trait(TraitQuery),
    Not(Box<Formula>),
    And(Vec<Formula>),
    Or(Vec<Formula>),
}
impl Formula {
    pub fn from_predicate(predicate: &nir::program::TypePredicate) -> Self {
        use nir::program::TypePredicate as P;
        match predicate {
            P::Trait(requirement) => Self::Trait(requirement.into()),
            P::Bool(true) => Self::True,
            P::Bool(false) => Self::False,
            P::Equal(a, b) => Self::equal(a.clone(), b.clone()),
            P::Not(inner) => Self::from_predicate(inner).negate(),
            P::And(items) => Self::And(items.iter().map(Self::from_predicate).collect()),
            P::Or(items) => Self::Or(items.iter().map(Self::from_predicate).collect()),
        }
    }
    pub fn equal(left: Term, right: Term) -> Self {
        Self::Equal(Equality(left, right))
    }
    pub fn int_le(left: IntegerTerm, right: IntegerTerm) -> Self {
        Self::IntLe(IntLe { left, right })
    }
    pub fn kind(ty: Term, kind: TypeKind) -> Self {
        Self::Kind(KindQuery { ty, kind })
    }
    pub fn negate(self) -> Self {
        Self::Not(Box::new(self))
    }
    pub fn implies(self, consequence: Formula) -> Self {
        Self::Or(vec![self.negate(), consequence])
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    // Branches share immutable snapshots. Each mutation detaches only its own
    // component; no candidate may add authority or premises to another branch.
    scope: Arc<Vec<Vec<Sort>>>,
    assumptions: Arc<Vec<Formula>>,
    congruence_heads: Arc<BTreeSet<DeclarationId>>,
    nominal_constructors: Arc<BTreeSet<DeclarationId>>,
    /// Trusted Tuple classification/layout contracts; never body equations.
    tuple_heads: Arc<BTreeSet<DeclarationId>>,
    /// Trusted List domain and candidate metadata; never a solver injectivity rule.
    list_inference_heads: Arc<BTreeSet<DeclarationId>>,
}
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub steps: usize,
    pub max_depth: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            steps: 100_000,
            max_depth: 128,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnknownReason {
    BudgetExhausted,
    UnvalidatedModel,
    ArithmeticRange,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    BudgetExhausted,
    IllFormed(String),
}
impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BudgetExhausted => write!(f, "(type-proposition-validation-budget-exhausted)"),
            Self::IllFormed(message) => write!(f, "(ill-formed-type-proposition {message:?})"),
        }
    }
}
impl std::error::Error for ValidationError {}
#[derive(Clone, Debug)]
pub enum Outcome {
    Entailed(Proof),
    NotEntailed(Witness),
    Unknown {
        reason: UnknownReason,
        residual: Formula,
    },
    IllFormed(String),
}
#[derive(Clone, Debug)]
pub struct Proof {
    pub scope: Vec<Vec<Sort>>,
    // Proofs own their support. There is no cross-context learned fact cache.
    pub assumptions: Vec<Formula>,
    pub goal: Formula,
    pub congruence_heads: BTreeSet<DeclarationId>,
    pub nominal_constructors: BTreeSet<DeclarationId>,
    pub tuple_heads: BTreeSet<DeclarationId>,
    pub tree: ProofTree,
}
#[derive(Clone, Debug)]
pub enum ProofTree {
    BooleanConflict,
    TheoryConflict(Vec<(Atom, bool)>),
    Split {
        atom: Atom,
        negative: Box<ProofTree>,
        positive: Box<ProofTree>,
    },
}
#[derive(Clone, Debug)]
pub struct Witness {
    pub tuple_heads: BTreeSet<DeclarationId>,
    pub scope: Vec<Vec<Sort>>,
    pub nominal_constructors: BTreeSet<DeclarationId>,
    pub congruence_heads: BTreeSet<DeclarationId>,
    pub types: BTreeMap<DeclarationId, Term>,
    pub integers: BTreeMap<DeclarationId, i128>,
    pub assumptions: Vec<Formula>,
    pub goal: Formula,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofError {
    Invalid,
    BudgetExhausted,
    ArithmeticRange,
}
impl Context {
    /// An explicit lexical type-input scope. The default context remains closed.
    /// Each frame lists its binding sorts, outermost first. Proofs retain these
    /// frames and cannot be replayed after changing or removing them.
    pub fn with_scope(scope: Vec<Vec<Sort>>) -> Self {
        Self {
            scope: Arc::new(scope),
            ..Self::default()
        }
    }
    /// Copy admitted global theory into a newly declared lexical scope.
    /// Local assumptions must be supplied explicitly in that scope.
    pub fn theory_in_scope(&self, scope: Vec<Vec<Sort>>) -> Self {
        Self {
            scope: Arc::new(scope),
            assumptions: Arc::default(),
            congruence_heads: self.congruence_heads.clone(),
            nominal_constructors: self.nominal_constructors.clone(),
            tuple_heads: self.tuple_heads.clone(),
            list_inference_heads: self.list_inference_heads.clone(),
        }
    }
    pub fn scope(&self) -> &[Vec<Sort>] {
        &self.scope
    }
    pub fn admit_tuple_head(&mut self, head: DeclarationId) {
        Arc::make_mut(&mut self.tuple_heads).insert(head);
    }
    pub fn admit_list_inference_head(&mut self, head: DeclarationId) {
        Arc::make_mut(&mut self.list_inference_heads).insert(head);
    }
    pub fn is_list_inference_head(&self, head: DeclarationId) -> bool {
        self.list_inference_heads.contains(&head)
    }
    /// Recognize the compiler-bound List domain without unfolding its head.
    pub fn is_list_type(&self, ty: &Term) -> bool {
        matches!(ty, Term::NormalForm { head, sort: Sort::Type, arguments }
            if self.is_list_inference_head(*head) && arguments.len() == 1
                && arguments[0].sort() == Sort::Type)
    }
    pub fn is_tuple_head(&self, head: DeclarationId) -> bool {
        self.tuple_heads.contains(&head)
    }
    pub fn is_tuple_type(&self, ty: &Term) -> bool {
        matches!(ty, Term::NormalForm { head, sort: Sort::Type, arguments }
            if self.is_tuple_head(*head) && arguments.len() == 1 && arguments[0].sort() == Sort::ComptimeValue)
    }
    fn evaluate_kind(&self, kind: TypeKind, ty: &Term) -> Option<bool> {
        evaluate_kind_with_heads(kind, ty, &self.nominal_constructors, &self.tuple_heads)
    }
    pub fn assumptions(&self) -> &[Formula] {
        &self.assumptions
    }
    /// Restore declaration theory from checked owned NIR, without importing any
    /// caller assumptions or equations from function bodies. External head
    /// descriptors are compiler-issued contracts, as in required computation.
    pub fn from_package(package: &nir::program::Package) -> Result<Self, &'static str> {
        nir::normal_forms::validate(package)?;
        let mut context = Self::default();
        for declaration in package
            .structs
            .iter()
            .map(|s| s.declaration)
            .chain(package.enums.iter().map(|e| e.declaration))
            .chain(package.newtypes.iter().map(|n| n.declaration))
        {
            context.admit_nominal_constructor(declaration);
        }
        for head in &package.normal_forms {
            context.admit_congruence(head.declaration);
            if head.role == nir::normal_forms::Role::TypeList {
                context.admit_list_inference_head(head.declaration);
            }
            if head.role == nir::normal_forms::Role::Tuple {
                context.admit_tuple_head(head.declaration);
            }
        }
        Ok(context)
    }
    /// Replay a refutation against this exact scope and its admitted contracts.
    /// This checks the tree, not merely whether a fresh search proves the goal.
    pub fn verify_proof(&self, proof: &Proof, budget: Budget) -> Result<(), ProofError> {
        let max_depth = budget.max_depth.min(256);
        let mut steps = budget.steps;
        for formula in self
            .assumptions
            .iter()
            .chain(proof.assumptions.iter())
            .chain(std::iter::once(&proof.goal))
        {
            if !within_limits(formula, max_depth, &mut steps) {
                return Err(ProofError::BudgetExhausted);
            }
        }
        if proof.scope != *self.scope
            || proof.assumptions != *self.assumptions
            || proof.congruence_heads != *self.congruence_heads
            || proof.nominal_constructors != *self.nominal_constructors
            || proof.tuple_heads != *self.tuple_heads
        {
            return Err(ProofError::Invalid);
        }
        let mut formulas = (*self.assumptions).clone();
        formulas.push(proof.goal.clone().negate());
        let formula = Formula::And(formulas);
        let mut atoms = Vec::new();
        let mut terms = Terms::default();
        collect(&formula, &mut atoms, &mut terms, 0, max_depth, &self.scope).map_err(
            |message| {
                if message == "depth budget" {
                    ProofError::BudgetExhausted
                } else {
                    ProofError::Invalid
                }
            },
        )?;
        let mut search = Search {
            required_atoms: None,
            context: self,
            formula: &formula,
            atoms: &atoms,
            terms: &terms,
            steps,
            max_depth,
        };
        search.check_tree(&proof.tree, &mut vec![None; atoms.len()], 0)
    }
    /// Validate a concrete counterexample against this exact scope, without
    /// rerunning proof search or trusting the producer's Boolean assignment.
    pub fn verify_witness(&self, witness: &Witness, budget: Budget) -> Result<(), ProofError> {
        let mut fuel = budget.steps;
        let limit = budget.max_depth.min(256);
        for formula in self
            .assumptions
            .iter()
            .chain(&witness.assumptions)
            .chain(std::iter::once(&witness.goal))
        {
            if !within_limits(formula, limit, &mut fuel) {
                return Err(ProofError::BudgetExhausted);
            }
        }
        for ty in witness.types.values() {
            if !within_input_limits(Input::Term(ty), limit, &mut fuel) {
                return Err(ProofError::BudgetExhausted);
            }
            if ty.check_scope(&[]).is_err() || !closed_type(ty, &self.nominal_constructors) {
                return Err(ProofError::Invalid);
            }
        }
        if fuel < witness.integers.len() {
            return Err(ProofError::BudgetExhausted);
        }
        if witness.scope != *self.scope
            || witness.assumptions != *self.assumptions
            || witness.nominal_constructors != *self.nominal_constructors
            || witness.congruence_heads != *self.congruence_heads
            || witness.tuple_heads != *self.tuple_heads
        {
            return Err(ProofError::Invalid);
        }
        let mut atoms = Vec::new();
        let mut terms = Terms::default();
        for formula in self
            .assumptions
            .iter()
            .chain(std::iter::once(&witness.goal))
        {
            collect(formula, &mut atoms, &mut terms, 0, limit, &self.scope)
                .map_err(|_| ProofError::Invalid)?;
        }
        if atoms.iter().any(|atom| {
            model_atom(
                atom,
                &witness.types,
                &witness.integers,
                &self.nominal_constructors,
                &self.tuple_heads,
            )
            .is_none()
        }) {
            return Err(ProofError::Invalid);
        }
        let evaluate = |formula| {
            evaluate_model(
                formula,
                &witness.types,
                &witness.integers,
                &self.nominal_constructors,
                &self.tuple_heads,
            )
        };
        if self
            .assumptions
            .iter()
            .all(|fact| evaluate(fact) == Some(true))
            && evaluate(&witness.goal) == Some(false)
        {
            Ok(())
        } else {
            Err(ProofError::Invalid)
        }
    }
    /// Recover an already-mentioned structural type using equality evidence.
    /// This is an operation-local view, never normal-form reduction. Candidates
    /// under binders are not exported and disjunctions are proved, not guessed.
    pub fn structural_view(
        &self,
        ty: &Term,
        budget: Budget,
    ) -> Result<Option<(Term, Proof)>, UnknownReason> {
        self.structural_view_metered(ty, budget).0
    }

    /// Report traversal and proof work actually consumed, including failure.
    pub fn structural_view_metered(
        &self,
        ty: &Term,
        budget: Budget,
    ) -> (Result<Option<(Term, Proof)>, UnknownReason>, usize) {
        self.structural_view_with_heads_metered(ty, budget, |_| false)
    }

    /// Include caller-designated opaque heads as candidate representations.
    /// Only a proof of the original type equality exports a candidate; this
    /// neither unfolds a head nor asserts its injectivity or structural kind.
    pub fn structural_view_with_heads_metered(
        &self,
        ty: &Term,
        budget: Budget,
        accepts_head: impl Fn(DeclarationId) -> bool,
    ) -> (Result<Option<(Term, Proof)>, UnknownReason>, usize) {
        let mut fuel = budget.steps;
        let result = (|| {
            let depth = budget.max_depth.min(256);
            if !within_input_limits(Input::Term(ty), depth, &mut fuel) {
                return Err(UnknownReason::BudgetExhausted);
            }
            for assumption in self.assumptions.iter() {
                if !within_limits(assumption, depth, &mut fuel) {
                    return Err(UnknownReason::BudgetExhausted);
                }
            }
            let mut pending = vec![Input::Term(ty)];
            pending.extend(self.assumptions.iter().rev().map(Input::Formula));
            let mut candidates = Vec::new();
            let mut seen = std::collections::HashSet::new();
            while let Some(input) = pending.pop() {
                fuel = fuel.checked_sub(1).ok_or(UnknownReason::BudgetExhausted)?;
                match input {
                    Input::Formula(Formula::Equal(Equality(a, b))) => {
                        pending.push(Input::Term(b));
                        pending.push(Input::Term(a));
                    }
                    Input::Formula(Formula::Kind(query)) => pending.push(Input::Term(&query.ty)),
                    Input::Formula(Formula::Trait(query)) => {
                        pending.extend(query.inputs().map(Input::Term))
                    }
                    Input::Formula(Formula::Not(inner)) => pending.push(Input::Formula(inner)),
                    Input::Formula(Formula::And(items) | Formula::Or(items)) => {
                        pending.extend(items.iter().rev().map(Input::Formula))
                    }
                    Input::Term(term) => {
                        // A named product is also a usable structural shape when
                        // equality is proven. This does not grant constructor
                        // injectivity or decide field-permutation identity.
                        if (constructor(term).is_some()
                            || matches!(term, Term::Record(_))
                            || matches!(term, Term::NormalForm { head, sort: Sort::Type, .. } if accepts_head(*head)))
                            && term.check_scope(&self.scope).is_ok()
                            && seen.insert(term)
                        {
                            candidates.push(term);
                        }
                        pending.extend(subterms(term).into_iter().rev().map(Input::Term));
                    }
                    _ => {}
                }
            }
            let mut unknown = None;
            let count = candidates.len();
            for (index, candidate) in candidates.into_iter().enumerate() {
                let allowance = fuel / (count - index);
                if allowance == 0 {
                    return Err(UnknownReason::BudgetExhausted);
                }
                let mut proof_budget = Budget {
                    steps: allowance,
                    max_depth: depth,
                };
                let outcome = self.prove_metered(
                    Formula::equal(ty.clone(), candidate.clone()),
                    &mut proof_budget,
                );
                fuel -= allowance - proof_budget.steps;
                match outcome {
                    Outcome::Entailed(proof) => return Ok(Some((candidate.clone(), proof))),
                    Outcome::Unknown { reason, .. } => unknown = Some(reason),
                    Outcome::NotEntailed(_) | Outcome::IllFormed(_) => {}
                }
            }
            match unknown {
                Some(reason) => Err(reason),
                None => Ok(None),
            }
        })();
        (result, budget.steps.saturating_sub(fuel))
    }

    pub fn assume(&mut self, fact: Formula) {
        Arc::make_mut(&mut self.assumptions).push(fact);
    }
    /// The caller supplies the audited head contract, never just a purity flag.
    pub fn admit_congruence(&mut self, head: DeclarationId) {
        Arc::make_mut(&mut self.congruence_heads).insert(head);
    }
    /// Register a generative nominal declaration whose semantic arguments are
    /// part of its identity. This does not admit any normal-form inverse.
    pub fn admit_nominal_constructor(&mut self, declaration: DeclarationId) {
        Arc::make_mut(&mut self.nominal_constructors).insert(declaration);
    }
    pub fn is_nominal_constructor(&self, declaration: DeclarationId) -> bool {
        self.nominal_constructors.contains(&declaration)
    }
    /// Branch facts are isolated by copy-on-write ownership. Read-only forks
    /// do not copy formulas or declaration sets. Dropping a fork cannot leave
    /// equalities or learned clauses in its parent.
    pub fn fork(&self) -> Self {
        self.clone()
    }
    /// Global declaration checks retain admitted theory identities but may not
    /// rely on a particular caller's local premises (even a false premise).
    pub fn theory_context(&self) -> Self {
        Self {
            scope: Arc::default(),
            assumptions: Arc::default(),
            congruence_heads: self.congruence_heads.clone(),
            nominal_constructors: self.nominal_constructors.clone(),
            tuple_heads: self.tuple_heads.clone(),
            list_inference_heads: self.list_inference_heads.clone(),
        }
    }
    pub fn prove(&self, goal: Formula, mut budget: Budget) -> Outcome {
        self.prove_metered(goal, &mut budget)
    }

    /// Check the proposition language and lexical scope without proving the
    /// goal or adding it as an assumption. Deferred requirements need this
    /// check even when no instance exists yet.
    pub fn validate_formula(
        &self,
        goal: &Formula,
        budget: &mut Budget,
    ) -> Result<(), ValidationError> {
        let limit = budget.max_depth.min(128);
        if !within_limits(goal, limit, &mut budget.steps) {
            return Err(ValidationError::BudgetExhausted);
        }
        let mut work = vec![goal];
        while let Some(formula) = work.pop() {
            validate_atom_inputs(formula, &self.scope).map_err(ValidationError::IllFormed)?;
            match formula {
                Formula::Not(inner) => work.push(inner),
                Formula::And(parts) | Formula::Or(parts) => work.extend(parts),
                _ => {}
            }
        }
        Ok(())
    }

    /// Consume the supplied work allowance on every outcome, including failure.
    /// The remaining allowance can be shared with subsequent compiler work.
    pub fn prove_metered(&self, goal: Formula, budget: &mut Budget) -> Outcome {
        let max_depth = budget.max_depth.min(256);
        let mut steps = budget.steps;
        let admitted = !self
            .assumptions
            .iter()
            .chain(std::iter::once(&goal))
            .any(|formula| !within_limits(formula, max_depth, &mut steps));
        budget.steps = steps;
        if !admitted {
            return Outcome::Unknown {
                reason: UnknownReason::BudgetExhausted,
                residual: goal,
            };
        }
        let mut atoms = Vec::new();
        let mut terms = Terms::default();
        let mut formulas = (*self.assumptions).clone();
        formulas.push(goal.clone().negate());
        let residual = Formula::And(formulas);
        if let Err(message) = collect(&residual, &mut atoms, &mut terms, 0, max_depth, &self.scope)
        {
            return if message == "depth budget" {
                Outcome::Unknown {
                    reason: UnknownReason::BudgetExhausted,
                    residual: goal,
                }
            } else {
                Outcome::IllFormed(message)
            };
        }
        let mut search = Search {
            required_atoms: None,
            context: self,
            formula: &residual,
            atoms: &atoms,
            terms: &terms,
            steps,
            max_depth,
        };
        let mut assignment = vec![None; atoms.len()];
        let outcome = match search.solve(&mut assignment, 0) {
            SearchResult::Unsat(tree) => Outcome::Entailed(Proof {
                scope: (*self.scope).clone(),
                assumptions: (*self.assumptions).clone(),
                goal,
                congruence_heads: (*self.congruence_heads).clone(),
                nominal_constructors: (*self.nominal_constructors).clone(),
                tuple_heads: (*self.tuple_heads).clone(),
                tree,
            }),
            SearchResult::Model(model) => Outcome::NotEntailed(Witness {
                tuple_heads: (*self.tuple_heads).clone(),
                scope: (*self.scope).clone(),
                nominal_constructors: (*self.nominal_constructors).clone(),
                congruence_heads: (*self.congruence_heads).clone(),
                types: model.types,
                integers: model.integers,
                assumptions: (*self.assumptions).clone(),
                goal,
            }),
            SearchResult::Unknown(reason) => Outcome::Unknown {
                reason,
                residual: goal,
            },
        };
        budget.steps = search.steps;
        outcome
    }
}

// Bound input traversal before recursive validation, interning and proof search.
enum Input<'a> {
    Formula(&'a Formula),
    Term(&'a Term),
}
pub fn within_limits(formula: &Formula, limit: usize, fuel: &mut usize) -> bool {
    within_input_limits(Input::Formula(formula), limit, fuel)
}
fn within_input_limits(input: Input<'_>, limit: usize, fuel: &mut usize) -> bool {
    let mut pending = vec![(input, 0usize)];
    while let Some((input, depth)) = pending.pop() {
        if depth >= limit || *fuel == 0 {
            return false;
        }
        *fuel -= 1;
        // Pending roots already require at least one step each. Check width
        // before pushing children so a tiny budget cannot allocate a huge work
        // list merely to discover exhaustion on the next iteration.
        let room = fuel.saturating_sub(pending.len());
        match input {
            Input::Formula(Formula::Equal(Equality(a, b))) => {
                pending.push((Input::Term(a), depth + 1));
                pending.push((Input::Term(b), depth + 1));
            }
            Input::Formula(Formula::Kind(query)) => {
                pending.push((Input::Term(&query.ty), depth + 1))
            }
            Input::Formula(Formula::Trait(query)) => {
                if query.arguments.len().saturating_add(1) > room {
                    return false;
                }
                pending.extend(query.inputs().map(|term| (Input::Term(term), depth + 1)));
            }
            Input::Formula(Formula::Not(inner)) => pending.push((Input::Formula(inner), depth + 1)),
            Input::Formula(Formula::And(items) | Formula::Or(items)) => {
                if items.len() > room {
                    return false;
                }
                for item in items {
                    pending.push((Input::Formula(item), depth + 1));
                }
            }
            Input::Term(term) => match term {
                Term::ErrorResult { errors, result } => {
                    if errors.len().saturating_add(1) > room {
                        return false;
                    }
                    pending.push((Input::Term(result), depth + 1));
                    pending.extend(errors.iter().map(|ty| (Input::Term(ty), depth + 1)));
                }
                Term::Computation { effects, result } => {
                    if effects.len() > *fuel {
                        return false;
                    }
                    *fuel -= effects.len();
                    let width = effects.iter().try_fold(1usize, |n, operation| {
                        n.checked_add(operation.arguments.len())
                    });
                    if width.is_none_or(|n| n > fuel.saturating_sub(pending.len())) {
                        return false;
                    }
                    pending.push((Input::Term(result), depth + 1));
                    for operation in effects {
                        pending.extend(
                            operation
                                .arguments
                                .iter()
                                .map(|t| (Input::Term(t), depth + 1)),
                        );
                    }
                }
                Term::AssociatedType {
                    arguments, subject, ..
                } => {
                    if arguments.len().saturating_add(1) > room {
                        return false;
                    }
                    pending.push((Input::Term(subject), depth + 1));
                    for argument in arguments {
                        pending.push((Input::Term(argument), depth + 1));
                    }
                }
                Term::Optional(inner)
                | Term::Pointer(inner)
                | Term::ComptimeFunction(inner)
                | Term::UnsafeFunction(inner) => pending.push((Input::Term(inner), depth + 1)),
                Term::OptionalValue { ty, value } => {
                    pending.push((Input::Term(ty), depth + 1));
                    if let Some(value) = value {
                        pending.push((Input::Term(value), depth + 1));
                    }
                }
                Term::ErrorValue { ty, label, value } => {
                    pending.push((Input::Term(ty), depth + 1));
                    if let Some(label) = label {
                        pending.push((Input::Term(label), depth + 1));
                    }
                    pending.push((Input::Term(value), depth + 1));
                }
                Term::NominalValue { ty, fields, .. } => {
                    if fields.len().saturating_add(1) > room {
                        return false;
                    }
                    pending.push((Input::Term(ty), depth + 1));
                    pending.extend(fields.iter().map(|field| (Input::Term(field), depth + 1)));
                }
                Term::Record(fields) | Term::RecordValue(fields) => {
                    if fields.len() > room {
                        return false;
                    }
                    for (name, ty) in fields {
                        let Some(remaining) = fuel.checked_sub(name.len()) else {
                            return false;
                        };
                        *fuel = remaining;
                        pending.push((Input::Term(ty), depth + 1));
                    }
                }
                Term::Tuple(items)
                | Term::ClosureContextValue(items)
                | Term::TypeListValue(items)
                | Term::TypeListConcat(items)
                | Term::TupleValue(items)
                | Term::Nominal {
                    arguments: items, ..
                }
                | Term::NormalForm {
                    arguments: items, ..
                } => {
                    if items.len() > room {
                        return false;
                    }
                    for item in items {
                        pending.push((Input::Term(item), depth + 1));
                    }
                }
                Term::Function { parameters, result } => {
                    if parameters.len().saturating_add(1) > room {
                        return false;
                    }
                    pending.push((Input::Term(result), depth + 1));
                    for parameter in parameters {
                        pending.push((Input::Term(&parameter.ty), depth + 1));
                    }
                }
                Term::ContextualClosure { context, signature } => {
                    if room < 2 {
                        return false;
                    }
                    pending.push((Input::Term(context), depth + 1));
                    pending.push((Input::Term(signature), depth + 1));
                }
                Term::Closure {
                    environment,
                    signature,
                } => {
                    if environment.len().saturating_add(1) > room {
                        return false;
                    }
                    pending.push((Input::Term(signature), depth + 1));
                    for item in environment {
                        pending.push((Input::Term(item), depth + 1));
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    true
}

#[derive(Default)]
struct Terms {
    nodes: Vec<Term>,
    ids: HashMap<Term, usize>,
    children: Vec<Vec<usize>>,
}
impl Terms {
    fn insert(&mut self, term: &Term) -> usize {
        if let Some(id) = self.ids.get(term) {
            return *id;
        }
        let children = subterms(term)
            .into_iter()
            .map(|child| self.insert(child))
            .collect();
        let id = self.nodes.len();
        self.nodes.push(term.clone());
        self.children.push(children);
        self.ids.insert(term.clone(), id);
        id
    }
}
fn subterms(term: &Term) -> Vec<&Term> {
    match term {
        Term::ErrorResult { errors, result } if !has_bound(term) => errors
            .iter()
            .chain(std::iter::once(result.as_ref()))
            .collect(),
        Term::Computation { effects, result } if !has_bound(term) => effects
            .iter()
            .flat_map(|operation| operation.arguments.iter())
            .chain(std::iter::once(result.as_ref()))
            .collect(),
        Term::AssociatedType {
            arguments, subject, ..
        } => arguments
            .iter()
            .chain(std::iter::once(subject.as_ref()))
            .collect(),
        Term::OptionalValue { ty, value } => std::iter::once(ty.as_ref())
            .chain(value.as_deref())
            .collect(),
        Term::ErrorValue { ty, label, value } => std::iter::once(ty.as_ref())
            .chain(label.as_deref())
            .chain(std::iter::once(value.as_ref()))
            .collect(),
        Term::NominalValue { ty, fields, .. } => {
            std::iter::once(ty.as_ref()).chain(fields.iter()).collect()
        }
        Term::Record(fields) | Term::RecordValue(fields) => {
            fields.iter().map(|(_, ty)| ty).collect()
        }
        Term::Tuple(items)
        | Term::ClosureContextValue(items)
        | Term::TypeListValue(items)
        | Term::TypeListConcat(items)
        | Term::TupleValue(items)
        | Term::Nominal {
            arguments: items, ..
        }
        | Term::NormalForm {
            arguments: items, ..
        } => items.iter().collect(),
        Term::Optional(inner)
        | Term::Pointer(inner)
        | Term::ComptimeFunction(inner)
        | Term::UnsafeFunction(inner) => {
            vec![inner]
        }
        Term::Function { parameters, result } if !has_bound(term) => parameters
            .iter()
            .map(|p| &p.ty)
            .chain(std::iter::once(result.as_ref()))
            .collect(),
        Term::ContextualClosure { context, signature } if !has_bound(term) => {
            vec![context, signature]
        }
        Term::Closure {
            environment,
            signature,
        } if !has_bound(term) => environment
            .iter()
            .chain(std::iter::once(signature.as_ref()))
            .collect(),
        _ => vec![],
    }
}
fn has_bound(term: &Term) -> bool {
    match term {
        Term::ErrorResult { errors, result } => errors.iter().any(has_bound) || has_bound(result),
        Term::Computation { effects, result } => {
            has_bound(result)
                || effects
                    .iter()
                    .any(|operation| operation.arguments.iter().any(has_bound))
        }
        Term::AssociatedType {
            arguments, subject, ..
        } => arguments.iter().any(has_bound) || has_bound(subject),
        Term::OptionalValue { ty, value } => {
            has_bound(ty) || value.as_ref().is_some_and(|value| has_bound(value))
        }
        Term::ErrorValue { ty, label, value } => {
            has_bound(ty)
                || label.as_ref().is_some_and(|label| has_bound(label))
                || has_bound(value)
        }
        Term::NominalValue { ty, fields, .. } => has_bound(ty) || fields.iter().any(has_bound),
        Term::Record(fields) | Term::RecordValue(fields) => {
            fields.iter().any(|(_, ty)| has_bound(ty))
        }
        Term::Bound { .. } | Term::Hole { .. } => true,
        Term::Function { parameters, result } => {
            parameters.iter().any(|p| has_bound(&p.ty)) || has_bound(result)
        }
        Term::ContextualClosure { context, signature } => {
            has_bound(context) || has_bound(signature)
        }
        Term::Closure {
            environment,
            signature,
        } => environment.iter().any(has_bound) || has_bound(signature),
        Term::Tuple(items)
        | Term::ClosureContextValue(items)
        | Term::TypeListValue(items)
        | Term::TypeListConcat(items)
        | Term::TupleValue(items)
        | Term::Nominal {
            arguments: items, ..
        }
        | Term::NormalForm {
            arguments: items, ..
        } => items.iter().any(has_bound),
        Term::Optional(inner)
        | Term::Pointer(inner)
        | Term::ComptimeFunction(inner)
        | Term::UnsafeFunction(inner) => has_bound(inner),
        _ => false,
    }
}
// Shared admission rules for validation-only checks and proof collection.
fn validate_atom_inputs(formula: &Formula, scope: &[Vec<Sort>]) -> Result<(), String> {
    match formula {
        Formula::Equal(atom) => {
            for term in [&atom.0, &atom.1] {
                if term.sort() != Sort::Type {
                    return Err("type equality requires two Type terms".into());
                }
                term.check_scope(scope)
                    .map_err(|e| format!("invalid type term: {e:?}"))?;
            }
        }
        Formula::Kind(query) => {
            if query.ty.sort() != Sort::Type {
                return Err("type classification requires a Type term".into());
            }
            query
                .ty
                .check_scope(scope)
                .map_err(|e| format!("invalid classified type: {e:?}"))?;
        }
        Formula::Trait(query) => {
            query.validate_inputs(scope)?;
        }
        _ => {}
    }
    Ok(())
}
fn collect(
    formula: &Formula,
    atoms: &mut Vec<Atom>,
    terms: &mut Terms,
    depth: usize,
    limit: usize,
    scope: &[Vec<Sort>],
) -> Result<(), String> {
    if depth >= limit {
        return Err("depth budget".into());
    }
    validate_atom_inputs(formula, scope)?;
    match formula {
        Formula::Equal(atom) => {
            for term in [&atom.0, &atom.1] {
                terms.insert(term);
            }
            let atom = Atom::TypeEqual(atom.clone());
            if !atoms.contains(&atom) {
                atoms.push(atom);
            }
        }
        Formula::Kind(query) => {
            terms.insert(&query.ty);
            let atom = Atom::Kind(query.clone());
            if !atoms.contains(&atom) {
                atoms.push(atom);
            }
        }
        Formula::Trait(query) => {
            for term in query.inputs() {
                terms.insert(term);
            }
            let atom = Atom::Trait(query.clone());
            if !atoms.contains(&atom) {
                atoms.push(atom);
            }
        }
        Formula::IntLe(atom) => {
            let atom = Atom::IntLe(atom.clone());
            if !atoms.contains(&atom) {
                atoms.push(atom);
            }
        }
        Formula::Not(inner) => collect(inner, atoms, terms, depth + 1, limit, scope)?,
        Formula::And(items) | Formula::Or(items) => {
            for item in items {
                collect(item, atoms, terms, depth + 1, limit, scope)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn evaluate(formula: &Formula, atom_value: &impl Fn(&Atom) -> Option<bool>) -> Option<bool> {
    match formula {
        Formula::True => Some(true),
        Formula::False => Some(false),
        Formula::Equal(atom) => atom_value(&Atom::TypeEqual(atom.clone())),
        Formula::Kind(query) => atom_value(&Atom::Kind(query.clone())),
        Formula::Trait(query) => atom_value(&Atom::Trait(query.clone())),
        Formula::IntLe(atom) => atom_value(&Atom::IntLe(atom.clone())),
        Formula::Not(inner) => evaluate(inner, atom_value).map(|b| !b),
        Formula::And(items) => {
            let mut all = true;
            for item in items {
                match evaluate(item, atom_value) {
                    Some(false) => return Some(false),
                    None => all = false,
                    _ => {}
                }
            }
            all.then_some(true)
        }
        Formula::Or(items) => {
            let mut all = true;
            for item in items {
                match evaluate(item, atom_value) {
                    Some(true) => return Some(true),
                    None => all = false,
                    _ => {}
                }
            }
            all.then_some(false)
        }
    }
}

struct Classes {
    parent: Vec<usize>,
    size: Vec<usize>,
}
impl Classes {
    fn new(count: usize) -> Self {
        Self {
            parent: (0..count).collect(),
            size: vec![1; count],
        }
    }
    fn root(&self, mut id: usize) -> usize {
        while self.parent[id] != id {
            id = self.parent[id];
        }
        id
    }
    fn merge(&mut self, a: usize, b: usize) -> bool {
        let (mut a, mut b) = (self.root(a), self.root(b));
        if a == b {
            return false;
        }
        if self.size[a] < self.size[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b] = a;
        self.size[a] += self.size[b];
        true
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Head {
    Primitive(Primitive),
    Optional,
    Pointer,
    ComptimeFunction,
    UnsafeFunction,
    Tuple(usize),
    Function(Vec<Sort>),
    Closure(usize),
    Nominal(DeclarationId, usize),
}
fn constructor(term: &Term) -> Option<Head> {
    match term {
        Term::Primitive(p) if *p != Primitive::Unit => Some(Head::Primitive(*p)),
        Term::Optional(_) => Some(Head::Optional),
        Term::Pointer(_) => Some(Head::Pointer),
        Term::ComptimeFunction(_) => Some(Head::ComptimeFunction),
        Term::UnsafeFunction(_) => Some(Head::UnsafeFunction),
        Term::Tuple(items) => Some(Head::Tuple(items.len())),
        Term::Function { parameters, .. } if !has_bound(term) => {
            Some(Head::Function(parameters.iter().map(|p| p.sort).collect()))
        }
        Term::Closure { environment, .. } if !has_bound(term) => {
            Some(Head::Closure(environment.len()))
        }
        Term::Nominal {
            declaration,
            arguments,
        } => Some(Head::Nominal(*declaration, arguments.len())),
        _ => None,
    }
}

/// Constant-work disjointness for admitted outer type constructors. Reuse the
/// theory's constructor identities, without inspecting inputs or assuming
/// injectivity. Unit aliases, projections, opaque computations and callable
/// binders deliberately stay on the ordinary proof path.
pub fn distinct_outer_type_heads(left: &Term, right: &Term) -> bool {
    let head = |term: &Term| match term {
        Term::Primitive(_)
        | Term::Optional(_)
        | Term::Pointer(_)
        | Term::ComptimeFunction(_)
        | Term::UnsafeFunction(_)
        | Term::Tuple(_)
        | Term::Nominal { .. } => constructor(term),
        _ => None,
    };
    matches!((head(left), head(right)), (Some(left), Some(right)) if left != right)
}

#[cfg(test)]
mod outer_type_head_tests {
    use super::*;

    #[test]
    fn cheap_separation_agrees_with_full_constructor_proofs() {
        let boolean = Term::Primitive(Primitive::Bool);
        let character = Term::Primitive(Primitive::Char);
        for (left, right) in [
            (boolean.clone(), character.clone()),
            (
                Term::Pointer(Box::new(boolean.clone())),
                Term::Optional(Box::new(boolean.clone())),
            ),
            (
                Term::Tuple(vec![boolean.clone()]),
                Term::Tuple(vec![boolean.clone(), character]),
            ),
        ] {
            assert!(distinct_outer_type_heads(&left, &right));
            let outcome =
                Context::default().prove(Formula::equal(left, right).negate(), Budget::default());
            assert!(matches!(outcome, Outcome::Entailed(_)), "{outcome}");
        }
    }

    #[test]
    fn shared_heads_and_unit_aliases_still_need_the_full_query() {
        let boolean = Term::Primitive(Primitive::Bool);
        assert!(!distinct_outer_type_heads(
            &Term::Primitive(Primitive::Unit),
            &Term::Tuple(vec![])
        ));
        assert!(!distinct_outer_type_heads(
            &Term::Pointer(Box::new(boolean)),
            &Term::Pointer(Box::new(Term::Primitive(Primitive::Char)))
        ));
    }
}
impl Context {
    /// Forward congruence premises for an admitted pair of heads. Consumers
    /// must prove every returned pair equal; this does not grant injectivity or
    /// unfold opaque heads. Function binders still require scope-aware handling.
    pub fn congruence_arguments<'a>(
        &self,
        left: &'a Term,
        right: &'a Term,
    ) -> Option<Vec<(&'a Term, &'a Term)>> {
        if !same_congruent_head(left, right, self) {
            return None;
        }
        let a = subterms(left);
        let b = subterms(right);
        (a.len() == b.len()).then(|| a.into_iter().zip(b).collect())
    }
}

fn same_congruent_head(a: &Term, b: &Term, context: &Context) -> bool {
    if let (Term::OptionalValue { value: a, .. }, Term::OptionalValue { value: b, .. }) = (a, b) {
        return a.is_some() == b.is_some();
    }
    if let (Term::ErrorValue { label: a, .. }, Term::ErrorValue { label: b, .. }) = (a, b) {
        return a.is_some() == b.is_some();
    }
    if let (Term::Computation { effects: left, .. }, Term::Computation { effects: right, .. }) =
        (a, b)
    {
        // Only lift equality through an identical occurrence layout. This does
        // not assert injectivity, disjointness or unproved row commutations.
        return !has_bound(a)
            && !has_bound(b)
            && left.len() == right.len()
            && left.iter().zip(right).all(|(a, b)| {
                a.declaration == b.declaration && a.arguments.len() == b.arguments.len()
            });
    }
    if let (
        Term::AssociatedType {
            origin: a,
            arguments: aa,
            ..
        },
        Term::AssociatedType {
            origin: b,
            arguments: ba,
            ..
        },
    ) = (a, b)
    {
        // A projection is a query with fixed origin, not an injective constructor.
        return a == b && aa.len() == ba.len();
    }
    // Type identity changes inside immutable tuple inputs lift forward through
    // the tuple. This does not admit a standalone value proposition, or make
    // an enclosing opaque normal-form application injective.
    if let (
        Term::NominalValue {
            variant: a,
            fields: af,
            ..
        },
        Term::NominalValue {
            variant: b,
            fields: bf,
            ..
        },
    ) = (a, b)
    {
        return a == b && af.len() == bf.len();
    }
    // Equal field types lift through the same named product layout. This is
    // forward congruence only: no permutation or inverse field equation is
    // introduced, and value records remain distinct from record Types.
    if let (Term::Record(a), Term::Record(b)) | (Term::RecordValue(a), Term::RecordValue(b)) =
        (a, b)
    {
        return a.len() == b.len() && a.iter().zip(b).all(|((a, _), (b, _))| a == b);
    }
    if let (Term::TypeListValue(a), Term::TypeListValue(b))
    | (Term::TypeListConcat(a), Term::TypeListConcat(b))
    | (Term::TupleValue(a), Term::TupleValue(b)) = (a, b)
    {
        return a.len() == b.len();
    }
    if let (Some(a), Some(b)) = (constructor(a), constructor(b)) {
        return a == b;
    }
    matches!((a, b), (Term::NormalForm { head: a, sort: sa, arguments: aa }, Term::NormalForm { head: b, sort: sb, arguments: ab }) if a == b && sa == sb && aa.len() == ab.len() && context.congruence_heads.contains(a))
}
struct Search<'a> {
    // Query-local search order only; never shared with another context, goal
    // or proof check. Contains no assignment facts.
    required_atoms: Option<Vec<usize>>,
    context: &'a Context,
    formula: &'a Formula,
    atoms: &'a [Atom],
    terms: &'a Terms,
    steps: usize,
    max_depth: usize,
}
struct Model {
    types: BTreeMap<DeclarationId, Term>,
    integers: BTreeMap<DeclarationId, i128>,
}
struct TheoryModel {
    classes: Classes,
    kinds: Vec<(usize, TypeKind, bool)>,
    integers: BTreeMap<DeclarationId, i128>,
}
enum SearchResult {
    Unsat(ProofTree),
    Model(Model),
    Unknown(UnknownReason),
}
impl Search<'_> {
    fn check_tree(
        &mut self,
        tree: &ProofTree,
        assignment: &mut [Option<bool>],
        depth: usize,
    ) -> Result<(), ProofError> {
        if depth >= self.max_depth || self.tick().is_err() {
            return Err(ProofError::BudgetExhausted);
        }
        match tree {
            ProofTree::BooleanConflict => {
                if evaluate(self.formula, &|atom| {
                    assignment[self.atoms.iter().position(|a| a == atom).unwrap()]
                }) == Some(false)
                {
                    Ok(())
                } else {
                    Err(ProofError::Invalid)
                }
            }
            ProofTree::TheoryConflict(literals) => {
                let expected: Vec<_> = self
                    .atoms
                    .iter()
                    .zip(assignment.iter())
                    .filter_map(|(a, b)| b.map(|b| (a, b)))
                    .collect();
                if literals.len() != expected.len()
                    || !literals
                        .iter()
                        .zip(expected)
                        .all(|((a, b), (c, d))| a == c && *b == d)
                {
                    return Err(ProofError::Invalid);
                }
                match self.theory(assignment) {
                    Ok(None) => Ok(()),
                    Ok(Some(_)) => Err(ProofError::Invalid),
                    Err(UnknownReason::ArithmeticRange) => Err(ProofError::ArithmeticRange),
                    Err(_) => Err(ProofError::BudgetExhausted),
                }
            }
            ProofTree::Split {
                atom,
                negative,
                positive,
            } => {
                let index = self
                    .atoms
                    .iter()
                    .position(|a| a == atom)
                    .ok_or(ProofError::Invalid)?;
                if assignment[index].is_some() {
                    return Err(ProofError::Invalid);
                }
                assignment[index] = Some(false);
                let result = self.check_tree(negative, assignment, depth + 1);
                assignment[index] = Some(true);
                let result = result.and_then(|()| self.check_tree(positive, assignment, depth + 1));
                assignment[index] = None;
                result
            }
        }
    }
    fn tick(&mut self) -> Result<(), ()> {
        if self.steps == 0 {
            Err(())
        } else {
            self.steps -= 1;
            Ok(())
        }
    }
    /// Prefer literals required by the Boolean skeleton before choosing a
    /// disjunctive branch. This is only a search-order change: both children
    /// are still represented by the existing, independently checked Split.
    fn required_atom(&mut self, assignment: &[Option<bool>]) -> Result<Option<usize>, ()> {
        if self.required_atoms.is_none() {
            self.steps = self.steps.checked_sub(self.atoms.len()).ok_or(())?;
            let mut seen = vec![false; self.atoms.len()];
            let mut order = Vec::new();
            let mut pending = vec![(self.formula, true)];
            while let Some((formula, truth)) = pending.pop() {
                self.tick()?;
                match formula {
                    Formula::Not(inner) => pending.push((inner, !truth)),
                    Formula::And(parts) if truth => {
                        self.steps = self.steps.checked_sub(parts.len()).ok_or(())?;
                        pending.extend(parts.iter().rev().map(|part| (part, true)));
                    }
                    Formula::Or(parts) if !truth => {
                        self.steps = self.steps.checked_sub(parts.len()).ok_or(())?;
                        pending.extend(parts.iter().rev().map(|part| (part, false)));
                    }
                    Formula::Equal(_)
                    | Formula::Kind(_)
                    | Formula::Trait(_)
                    | Formula::IntLe(_) => {
                        for (index, atom) in self.atoms.iter().enumerate() {
                            self.steps = self.steps.checked_sub(1).ok_or(())?;
                            let matches = match (formula, atom) {
                                (Formula::Equal(a), Atom::TypeEqual(b)) => a == b,
                                (Formula::Kind(a), Atom::Kind(b)) => a == b,
                                (Formula::Trait(a), Atom::Trait(b)) => a == b,
                                (Formula::IntLe(a), Atom::IntLe(b)) => a == b,
                                _ => false,
                            };
                            if matches {
                                if !seen[index] {
                                    seen[index] = true;
                                    order.push(index);
                                }
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
            self.required_atoms = Some(order);
        }
        for index in self.required_atoms.as_ref().unwrap() {
            self.steps = self.steps.checked_sub(1).ok_or(())?;
            if assignment[*index].is_none() {
                return Ok(Some(*index));
            }
        }
        Ok(None)
    }
    fn solve(&mut self, assignment: &mut [Option<bool>], depth: usize) -> SearchResult {
        if depth >= self.max_depth || self.tick().is_err() {
            return SearchResult::Unknown(UnknownReason::BudgetExhausted);
        }
        if evaluate(self.formula, &|atom| {
            assignment[self.atoms.iter().position(|a| a == atom).unwrap()]
        }) == Some(false)
        {
            return SearchResult::Unsat(ProofTree::BooleanConflict);
        }
        let classes = match self.theory(assignment) {
            Ok(Some(classes)) => classes,
            Ok(None) => {
                return SearchResult::Unsat(ProofTree::TheoryConflict(
                    self.atoms
                        .iter()
                        .zip(assignment.iter())
                        .filter_map(|(a, b)| b.map(|b| (a.clone(), b)))
                        .collect(),
                ));
            }
            Err(reason) => return SearchResult::Unknown(reason),
        };
        let required = match self.required_atom(assignment) {
            Ok(index) => index,
            Err(()) => return SearchResult::Unknown(UnknownReason::BudgetExhausted),
        };
        if let Some(index) = required.or_else(|| assignment.iter().position(Option::is_none)) {
            assignment[index] = Some(false);
            let negative = self.solve(assignment, depth + 1);
            assignment[index] = None;
            if matches!(negative, SearchResult::Model(_)) {
                return negative;
            }
            assignment[index] = Some(true);
            let positive = self.solve(assignment, depth + 1);
            assignment[index] = None;
            match (negative, positive) {
                (SearchResult::Unsat(negative), SearchResult::Unsat(positive)) => {
                    SearchResult::Unsat(ProofTree::Split {
                        atom: self.atoms[index].clone(),
                        negative: Box::new(negative),
                        positive: Box::new(positive),
                    })
                }
                (_, model @ SearchResult::Model(_)) => model,
                (SearchResult::Unknown(reason), _) | (_, SearchResult::Unknown(reason)) => {
                    SearchResult::Unknown(reason)
                }
                _ => unreachable!(),
            }
        } else {
            self.witness(&classes)
                .map(SearchResult::Model)
                .unwrap_or(SearchResult::Unknown(UnknownReason::UnvalidatedModel))
        }
    }
    fn theory(
        &mut self,
        assignment: &[Option<bool>],
    ) -> Result<Option<TheoryModel>, UnknownReason> {
        let mut classes = Classes::new(self.terms.nodes.len());
        for (atom, value) in self.atoms.iter().zip(assignment) {
            if let Atom::TypeEqual(atom) = atom {
                if *value == Some(true) {
                    classes.merge(self.terms.ids[&atom.0], self.terms.ids[&atom.1]);
                }
            }
        }
        loop {
            let mut changed = false;
            for a in 0..self.terms.nodes.len() {
                for b in 0..a {
                    self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
                    let (ta, tb) = (&self.terms.nodes[a], &self.terms.nodes[b]);
                    if classes.root(a) == classes.root(b) {
                        // A trusted Tuple descriptor grants classification,
                        // not unfolding, injectivity or arbitrary head disjointness.
                        if (self.context.is_tuple_type(ta)
                            && self.context.evaluate_kind(TypeKind::Tuple, tb) == Some(false))
                            || (self.context.is_tuple_type(tb)
                                && self.context.evaluate_kind(TypeKind::Tuple, ta) == Some(false))
                        {
                            return Ok(None);
                        }
                        // The compiler-bound List family is distinct from
                        // established structural constructors. This says nothing
                        // about arbitrary opaque normal-form heads or List input
                        // injectivity, and never unfolds a factory body.
                        let non_list = |term: &Term| {
                            constructor(term).is_some()
                                || self.context.is_tuple_type(term)
                                || matches!(term, Term::Record(_))
                        };
                        if (self.context.is_list_type(ta) && non_list(tb))
                            || (self.context.is_list_type(tb) && non_list(ta))
                        {
                            return Ok(None);
                        }
                        if self.error_result_distinct(a, b, &classes, assignment)? {
                            return Ok(None);
                        }
                        if self.computation_distinct(a, b, &classes, assignment)? {
                            return Ok(None);
                        }
                        // A named structural product is distinct from the
                        // established positional/nominal constructor families.
                        // Do not decide record permutations or decompose them.
                        if (matches!(ta, Term::Record(_)) && constructor(tb).is_some())
                            || (matches!(tb, Term::Record(_)) && constructor(ta).is_some())
                        {
                            return Ok(None);
                        }
                        if let (Some(ha), Some(hb)) = (constructor(ta), constructor(tb)) {
                            if ha != hb {
                                return Ok(None);
                            }
                            // Nominal argument injectivity needs its own declaration contract.
                            if !matches!(ha, Head::Nominal(..))
                                || matches!(ha, Head::Nominal(head, _) if self.context.is_nominal_constructor(head))
                            {
                                // Closed scalar indices are part of a nominal
                                // type's identity, not program-state predicates.
                                if let (
                                    Term::Nominal {
                                        arguments: left, ..
                                    },
                                    Term::Nominal {
                                        arguments: right, ..
                                    },
                                ) = (ta, tb)
                                {
                                    if left.iter().zip(right).any(|(left, right)| {
                                        left.scalar_index_domain().is_some()
                                            && right.scalar_index_domain().is_some()
                                            && left != right
                                    }) {
                                        return Ok(None);
                                    }
                                }
                                for (&ca, &cb) in
                                    self.terms.children[a].iter().zip(&self.terms.children[b])
                                {
                                    changed |= classes.merge(ca, cb);
                                }
                            }
                        }
                    } else if same_congruent_head(ta, tb, self.context)
                        && self.terms.children[a]
                            .iter()
                            .zip(&self.terms.children[b])
                            .all(|(&a, &b)| classes.root(a) == classes.root(b))
                    {
                        changed |= classes.merge(a, b);
                    } else if self.error_result_congruent(a, b, &classes)? {
                        changed |= classes.merge(a, b);
                    } else if self.computation_congruent(a, b, &classes, assignment)? {
                        changed |= classes.merge(a, b);
                    }
                }
            }
            if !changed {
                break;
            }
        }
        for (atom, value) in self.atoms.iter().zip(assignment) {
            if let Atom::TypeEqual(atom) = atom {
                if *value == Some(false)
                    && classes.root(self.terms.ids[&atom.0])
                        == classes.root(self.terms.ids[&atom.1])
                {
                    return Ok(None);
                }
            }
        }
        // Equal inputs denote the same trait proposition, including retained
        // comptime indices. This is congruence only: equal truth values never
        // imply equal subjects or equal indices, and introduce no value equations.
        let mut trait_truths = BTreeMap::new();
        for (atom, value) in self.atoms.iter().zip(assignment) {
            let (Atom::Trait(query), Some(value)) = (atom, value) else {
                continue;
            };
            let mut inputs = Vec::new();
            for term in query.inputs() {
                self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
                inputs.push(classes.root(self.terms.ids[term]));
            }
            if trait_truths
                .insert((query.interface, inputs), *value)
                .is_some_and(|old| old != *value)
            {
                return Ok(None);
            }
        }
        let mut kinds: Vec<(usize, TypeKind, bool)> = Vec::new();
        for (atom, value) in self.atoms.iter().zip(assignment) {
            let (Atom::Kind(query), Some(value)) = (atom, value) else {
                continue;
            };
            let root = classes.root(self.terms.ids[&query.ty]);
            for (id, term) in self.terms.nodes.iter().enumerate() {
                self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
                if classes.root(id) == root
                    && self
                        .context
                        .evaluate_kind(query.kind, term)
                        .is_some_and(|actual| actual != *value)
                {
                    return Ok(None);
                }
            }
            for &(other_root, other, truth) in &kinds {
                if root == other_root
                    && ((*value && truth && query.kind.disjoint(other))
                        || (*value && !truth && query.kind.implies(other))
                        || (!*value && truth && other.implies(query.kind)))
                {
                    return Ok(None);
                }
            }
            kinds.push((root, query.kind, *value));
        }
        let integers = integer::solve(
            self.atoms
                .iter()
                .zip(assignment)
                .filter_map(|(atom, value)| {
                    if let Atom::IntLe(atom) = atom {
                        Some((atom, *value))
                    } else {
                        None
                    }
                }),
            &mut self.steps,
        )?;
        Ok(integers.map(|integers| TheoryModel {
            classes,
            integers,
            kinds,
        }))
    }
    fn witness(&mut self, theory: &TheoryModel) -> Option<Model> {
        let classes = &theory.classes;
        // Reify actual closed types, not arbitrary EUF domain elements. Missing
        // value-store, runtime or opaque-call interpretations keep Unknown.
        let mut types = BTreeMap::new();
        let mut representatives = HashMap::new();
        for (id, term) in self.terms.nodes.iter().enumerate() {
            if !matches!(
                term,
                Term::Rigid {
                    sort: Sort::Type,
                    ..
                }
            ) {
                representatives.entry(classes.root(id)).or_insert(id);
            }
        }
        let mut free = HashMap::new();
        for &(root, _, _) in &theory.kinds {
            if !representatives.contains_key(&root) && !free.contains_key(&root) {
                let candidate = classification::witnesses().into_iter().find(|ty| {
                    theory
                        .kinds
                        .iter()
                        .filter(|(r, _, _)| *r == root)
                        .all(|(_, kind, truth)| kind.evaluate(ty) == Some(*truth))
                })?;
                free.insert(root, candidate);
            }
        }
        for term in &self.terms.nodes {
            if let Term::Rigid {
                declaration,
                sort: Sort::Type,
            } = term
            {
                let id = self.terms.ids[term];
                let value = reify(
                    id,
                    self.terms,
                    classes,
                    &representatives,
                    &mut free,
                    &mut BTreeSet::new(),
                    &self.context.nominal_constructors,
                )?;
                types.insert(*declaration, value);
            }
        }
        // A Boolean short circuit must not conceal an uninterpreted atom in
        // a purported concrete model (opaque calls still require evidence).
        for atom in self.atoms {
            model_atom(
                atom,
                &types,
                &theory.integers,
                &self.context.nominal_constructors,
                &self.context.tuple_heads,
            )?;
        }
        (evaluate_model(
            self.formula,
            &types,
            &theory.integers,
            &self.context.nominal_constructors,
            &self.context.tuple_heads,
        ) == Some(true))
        .then_some(Model {
            types,
            integers: theory.integers.clone(),
        })
    }
}
fn evaluate_kind_with_heads(
    kind: TypeKind,
    ty: &Term,
    nominal: &BTreeSet<DeclarationId>,
    tuple_heads: &BTreeSet<DeclarationId>,
) -> Option<bool> {
    if matches!(ty, Term::NormalForm {head,sort:Sort::Type,arguments}
        if tuple_heads.contains(head) && arguments.len()==1 && arguments[0].sort()==Sort::ComptimeValue)
    {
        Some(TypeKind::Tuple.implies(kind))
    } else if kind == TypeKind::Tuple
        && matches!(ty,Term::Nominal {declaration,..} if nominal.contains(declaration))
    {
        Some(false)
    } else {
        kind.evaluate(ty)
    }
}

fn evaluate_model(
    formula: &Formula,
    types: &BTreeMap<DeclarationId, Term>,
    integers: &BTreeMap<DeclarationId, i128>,
    nominal: &BTreeSet<DeclarationId>,
    tuple_heads: &BTreeSet<DeclarationId>,
) -> Option<bool> {
    evaluate(formula, &|atom| {
        model_atom(atom, types, integers, nominal, tuple_heads)
    })
}
fn model_atom(
    atom: &Atom,
    types: &BTreeMap<DeclarationId, Term>,
    integers: &BTreeMap<DeclarationId, i128>,
    nominal: &BTreeSet<DeclarationId>,
    tuple_heads: &BTreeSet<DeclarationId>,
) -> Option<bool> {
    match atom {
        // An abstract SAT assignment is not implementation-world evidence.
        Atom::Trait(_) => None,
        Atom::IntLe(atom) => atom.evaluate(integers),
        Atom::Kind(query) => {
            // Classification is independent of Tuple elements, but a concrete
            // counterexample must not hide unresolved value-list inputs.
            let closed_tuple = matches!(&query.ty,Term::NormalForm {head,sort:Sort::Type,arguments}
                if tuple_heads.contains(head) && matches!(arguments.as_slice(),[Term::TypeListValue(fields)]
                    if fields.iter().all(|field|closed_type(field,nominal))));
            if closed_tuple {
                evaluate_kind_with_heads(query.kind, &query.ty, nominal, tuple_heads)
            } else {
                evaluate_kind_with_heads(
                    query.kind,
                    &substitute_types(&query.ty, types, nominal)?,
                    nominal,
                    tuple_heads,
                )
            }
        }
        Atom::TypeEqual(atom) => {
            let a = substitute_types(&atom.0, types, nominal)?;
            let b = substitute_types(&atom.1, types, nominal)?;
            if !closed_type(&a, nominal) || !closed_type(&b, nominal) {
                return None;
            }
            Some(a == b)
        }
    }
}

fn reify(
    id: usize,
    terms: &Terms,
    classes: &Classes,
    representatives: &HashMap<usize, usize>,
    free: &mut HashMap<usize, Term>,
    active: &mut BTreeSet<usize>,
    nominal: &BTreeSet<DeclarationId>,
) -> Option<Term> {
    let root = classes.root(id);
    if !active.insert(root) {
        return None;
    }
    let result = if let Some(&representative) = representatives.get(&root) {
        rebuild(&terms.nodes[representative], nominal, &mut |child| {
            reify(
                terms.ids[child],
                terms,
                classes,
                representatives,
                free,
                active,
                nominal,
            )
        })
    } else {
        let next = free.len();
        Some(
            free.entry(root)
                .or_insert_with(|| {
                    let mut ty = Term::Primitive(Primitive::Bool);
                    for _ in 0..=next {
                        ty = Term::Pointer(Box::new(ty));
                    }
                    ty
                })
                .clone(),
        )
    };
    active.remove(&root);
    result
}
fn rebuild(
    term: &Term,
    nominal: &BTreeSet<DeclarationId>,
    child: &mut impl FnMut(&Term) -> Option<Term>,
) -> Option<Term> {
    match term {
        Term::Primitive(p) => Some(Term::Primitive(*p)),
        Term::Optional(t) => Some(Term::Optional(Box::new(child(t)?))),
        Term::Pointer(t) => Some(Term::Pointer(Box::new(child(t)?))),
        Term::ComptimeFunction(t) => Some(Term::ComptimeFunction(Box::new(child(t)?))),
        Term::UnsafeFunction(t) => Some(Term::UnsafeFunction(Box::new(child(t)?))),
        Term::Tuple(items) => Some(Term::Tuple(
            items.iter().map(&mut *child).collect::<Option<_>>()?,
        )),
        Term::Function { parameters, result } if !has_bound(term) => Some(Term::Function {
            parameters: parameters
                .iter()
                .map(|p| {
                    Some(nir::Parameter {
                        sort: p.sort,
                        ty: child(&p.ty)?,
                    })
                })
                .collect::<Option<_>>()?,
            result: Box::new(child(result)?),
        }),
        Term::Closure {
            environment,
            signature,
        } if !has_bound(term) => Some(Term::Closure {
            environment: environment.iter().map(&mut *child).collect::<Option<_>>()?,
            signature: Box::new(child(signature)?),
        }),
        Term::Nominal {
            declaration,
            arguments,
        } if arguments.is_empty() || nominal.contains(declaration) => Some(Term::Nominal {
            declaration: *declaration,
            arguments: arguments.iter().map(&mut *child).collect::<Option<_>>()?,
        }),
        _ => None,
    }
}
fn substitute_types(
    term: &Term,
    types: &BTreeMap<DeclarationId, Term>,
    nominal: &BTreeSet<DeclarationId>,
) -> Option<Term> {
    if let Term::Rigid {
        declaration,
        sort: Sort::Type,
    } = term
    {
        return types.get(declaration).cloned();
    }
    rebuild(term, nominal, &mut |child| {
        substitute_types(child, types, nominal)
    })
}
fn closed_type(term: &Term, nominal: &BTreeSet<DeclarationId>) -> bool {
    if matches!(term, Term::Nominal { declaration, arguments } if !arguments.is_empty() && !nominal.contains(declaration))
    {
        return false;
    }
    constructor(term).is_some()
        && !matches!(term, Term::Primitive(Primitive::Unit))
        && subterms(term)
            .into_iter()
            .all(|child| closed_type(child, nominal))
}

impl fmt::Display for Atom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TypeEqual(atom) => atom.fmt(f),
            Self::IntLe(atom) => atom.fmt(f),
            Self::Kind(query) => query.fmt(f),
            Self::Trait(query) => query.fmt(f),
        }
    }
}
impl fmt::Display for Equality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(type-eq {} {})", self.0, self.1)
    }
}
impl fmt::Display for Formula {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::True => f.write_str("true"),
            Self::False => f.write_str("false"),
            Self::Equal(atom) => atom.fmt(f),
            Self::IntLe(atom) => atom.fmt(f),
            Self::Kind(query) => query.fmt(f),
            Self::Trait(query) => query.fmt(f),
            Self::Not(inner) => write!(f, "(not {inner})"),
            Self::And(items) | Self::Or(items) => {
                f.write_str(if matches!(self, Self::And(_)) {
                    "(and"
                } else {
                    "(or"
                })?;
                for item in items {
                    write!(f, " {item}")?;
                }
                f.write_str(")")
            }
        }
    }
}
impl fmt::Display for ProofTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BooleanConflict => f.write_str("(boolean-conflict)"),
            Self::TheoryConflict(literals) => {
                f.write_str("(theory-conflict")?;
                for (atom, value) in literals {
                    write!(f, " (literal {value} {atom})")?;
                }
                f.write_str(")")
            }
            Self::Split {
                atom,
                negative,
                positive,
            } => write!(
                f,
                "(split {atom} (negative {negative}) (positive {positive}))"
            ),
        }
    }
}
impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entailed(proof) => proof.fmt(f),
            Self::NotEntailed(witness) => {
                f.write_str("(not-entailed (assumptions")?;
                for fact in &witness.assumptions {
                    write!(f, " {fact}")?;
                }
                f.write_str(") (bindings")?;
                for (id, ty) in &witness.types {
                    write!(f, " (type {id} {ty})")?;
                }
                for (id, value) in &witness.integers {
                    write!(f, " (integer {id} {value})")?;
                }
                write!(f, ") (goal {}) (nominal-constructors", witness.goal)?;
                for head in &witness.nominal_constructors {
                    write!(f, " {head}")?;
                }
                f.write_str(") (congruence-heads")?;
                for head in &witness.congruence_heads {
                    write!(f, " {head}")?;
                }
                f.write_str(") (tuple-heads")?;
                for head in &witness.tuple_heads {
                    write!(f, " {head}")?;
                }
                f.write_str(")")?;
                display_scope(f, &witness.scope)?;
                f.write_str(")")
            }
            Self::Unknown { reason, residual } => {
                write!(f, "(unknown (reason {:?}) (residual {residual}))", reason)
            }
            Self::IllFormed(message) => write!(f, "(ill-formed {:?})", message),
        }
    }
}

fn display_scope(f: &mut fmt::Formatter<'_>, scope: &[Vec<Sort>]) -> fmt::Result {
    f.write_str(" (scope")?;
    for frame in scope {
        f.write_str(" (frame")?;
        for sort in frame {
            write!(f, " {sort}")?;
        }
        f.write_str(")")?;
    }
    f.write_str(")")
}

impl fmt::Display for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(type-context")?;
        display_scope(f, &self.scope)?;
        f.write_str(" (assumptions")?;
        for assumption in self.assumptions.iter() {
            write!(f, " {assumption}")?;
        }
        f.write_str(") (congruence-heads")?;
        for declaration in self.congruence_heads.iter() {
            write!(f, " {declaration}")?;
        }
        f.write_str(") (nominal-constructors")?;
        for declaration in self.nominal_constructors.iter() {
            write!(f, " {declaration}")?;
        }
        f.write_str(") (tuple-heads")?;
        for declaration in self.tuple_heads.iter() {
            write!(f, " {declaration}")?;
        }
        f.write_str("))")
    }
}

impl fmt::Display for Proof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let proof = self;

        f.write_str("(entailed (assumptions")?;
        for fact in &proof.assumptions {
            write!(f, " {fact}")?;
        }
        write!(f, ") (goal {}) (congruence-heads", proof.goal)?;
        for head in &proof.congruence_heads {
            write!(f, " {head}")?;
        }
        f.write_str(") (nominal-constructors")?;
        for head in &proof.nominal_constructors {
            write!(f, " {head}")?;
        }
        f.write_str(") (tuple-heads")?;
        for head in &proof.tuple_heads {
            write!(f, " {head}")?;
        }
        f.write_str(")")?;
        display_scope(f, &proof.scope)?;
        write!(f, " (proof {}))", proof.tree)
    }
}

#[cfg(test)]
mod context_snapshot_tests {
    use super::*;

    #[test]
    fn required_atom_order_is_query_local_and_assignment_independent() {
        let equal = |local| {
            Formula::equal(
                Term::Rigid {
                    declaration: DeclarationId { package: 91, local },
                    sort: Sort::Type,
                },
                Term::Primitive(Primitive::Bool),
            )
        };
        let formula = Formula::And(vec![
            equal(0),
            Formula::Or(vec![equal(1), equal(2)]),
            Formula::Not(Box::new(Formula::Not(Box::new(equal(3))))),
        ]);
        let context = Context::with_scope(vec![]);
        let mut atoms = Vec::new();
        let mut terms = Terms::default();
        collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
        let mut search = Search {
            required_atoms: None,
            context: &context,
            formula: &formula,
            atoms: &atoms,
            terms: &terms,
            steps: 0,
            max_depth: 128,
        };
        let mut assignment = vec![None; atoms.len()];
        assert!(search.required_atom(&assignment).is_err());
        assert!(search.required_atoms.is_none());
        // Enough to visit the first literal, but not to finish the skeleton:
        // no partially built priority list may survive this failure either.
        search.steps = 12;
        assert!(search.required_atom(&assignment).is_err());
        assert!(search.required_atoms.is_none());
        search.steps = 1000;
        assignment[0] = Some(true);
        assert_eq!(search.required_atom(&assignment).unwrap(), Some(3));
        let built = search.steps;
        // Backtracking must make the earlier atom eligible again. A cached
        // list of previously unassigned atoms would silently change the order.
        assignment[0] = None;
        assert_eq!(search.required_atom(&assignment).unwrap(), Some(0));
        assert_eq!(built - search.steps, 1);
        assignment[0] = Some(true);
        assignment[3] = Some(true);
        assert_eq!(search.required_atom(&assignment).unwrap(), None);
        // Atoms beneath a positive disjunction remain ordinary choices.
        assert_eq!(assignment[1], None);
        assert_eq!(assignment[2], None);
        // A fresh search cannot inherit the order of an unrelated query.
        let other = Formula::Or(vec![equal(0), equal(3)]);
        let mut fresh = Search {
            required_atoms: None,
            context: &context,
            formula: &other,
            atoms: &atoms,
            terms: &terms,
            steps: 1000,
            max_depth: 128,
        };
        assert_eq!(fresh.required_atom(&vec![None; atoms.len()]).unwrap(), None);
    }

    #[test]
    fn required_literals_precede_disjunctions_with_checkable_proofs() {
        let types: Vec<_> = (0..8)
            .map(|local| Term::Rigid {
                declaration: DeclarationId { package: 90, local },
                sort: Sort::Type,
            })
            .collect();
        let mut context = Context::with_scope(vec![]);
        for ty in &types {
            context.assume(Formula::Or(vec![
                Formula::equal(ty.clone(), Term::Primitive(Primitive::I8)),
                Formula::equal(ty.clone(), Term::Primitive(Primitive::I16)),
            ]));
        }
        context.assume(Formula::And(
            types[1..]
                .iter()
                .map(|ty| Formula::equal(types[0].clone(), ty.clone()))
                .collect(),
        ));
        let goal = Formula::Not(Box::new(Formula::equal(
            types[7].clone(),
            Term::Primitive(Primitive::U8),
        )));
        let small = Budget {
            steps: 4096,
            ..Default::default()
        };
        let Outcome::Entailed(proof) = context.prove(goal.clone(), small) else {
            panic!("required equality premises must be processed before domain choices");
        };
        context.verify_proof(&proof, Budget::default()).unwrap();
        assert!(matches!(
            context.prove(
                goal.clone(),
                Budget {
                    steps: 0,
                    ..Default::default()
                }
            ),
            Outcome::Unknown { .. }
        ));
        assert!(matches!(context.prove(goal, small), Outcome::Entailed(_)));
        let Outcome::NotEntailed(witness) = context.prove(
            Formula::equal(types[0].clone(), Term::Primitive(Primitive::I8)),
            Budget::default(),
        ) else {
            panic!("the alternatives must not become unconditional premises");
        };
        context.verify_witness(&witness, Budget::default()).unwrap();

        let mut negative = Context::with_scope(vec![]);
        negative.assume(Formula::Not(Box::new(Formula::Or(vec![
            Formula::Not(Box::new(Formula::equal(
                types[0].clone(),
                Term::Primitive(Primitive::I8),
            ))),
            Formula::Not(Box::new(Formula::equal(
                types[1].clone(),
                Term::Primitive(Primitive::I8),
            ))),
        ]))));
        let Outcome::Entailed(proof) =
            negative.prove(Formula::equal(types[0].clone(), types[1].clone()), small)
        else {
            panic!("negative disjunction must preserve required literal polarity");
        };
        negative.verify_proof(&proof, Budget::default()).unwrap();
    }

    #[test]
    fn forks_share_support_until_the_affected_component_changes() {
        let head = DeclarationId {
            package: 82,
            local: 101,
        };
        let mut parent = Context::with_scope(vec![vec![Sort::Type]]);
        parent.assume(Formula::And(vec![Formula::True; 256]));
        let mut child = parent.fork();
        assert!(Arc::ptr_eq(&parent.scope, &child.scope));
        assert!(Arc::ptr_eq(&parent.assumptions, &child.assumptions));
        child.assume(Formula::False);
        child.admit_congruence(head);
        child.admit_nominal_constructor(head);
        child.admit_tuple_head(head);
        assert!(!Arc::ptr_eq(&parent.assumptions, &child.assumptions));
        assert!(Arc::ptr_eq(&parent.scope, &child.scope));
        assert_eq!(parent.assumptions.len(), 1);
        assert!(!parent.congruence_heads.contains(&head));
        assert!(!parent.is_nominal_constructor(head));
        assert!(!parent.is_tuple_head(head));
        assert!(matches!(
            parent.prove(Formula::False, Budget::default()),
            Outcome::NotEntailed(_)
        ));
        assert!(matches!(
            child.prove(Formula::False, Budget::default()),
            Outcome::Entailed(_)
        ));

        let theory = child.theory_in_scope(vec![vec![Sort::ComptimeValue]]);
        assert!(theory.assumptions().is_empty());
        assert!(Arc::ptr_eq(&theory.tuple_heads, &child.tuple_heads));
        assert!(Arc::ptr_eq(
            &theory.nominal_constructors,
            &child.nominal_constructors
        ));
    }

    #[test]
    fn proof_support_survives_branch_mutation_but_cannot_be_replayed_there() {
        let parent = Context::default();
        let mut child = parent.fork();
        let Outcome::Entailed(proof) = child.prove(Formula::True, Budget::default()) else {
            panic!("true must be proved");
        };
        child.assume(Formula::False);
        parent.verify_proof(&proof, Budget::default()).unwrap();
        assert_eq!(
            child.verify_proof(&proof, Budget::default()),
            Err(ProofError::Invalid)
        );
        assert!(proof.assumptions.is_empty());
    }
}

#[cfg(test)]
mod tuple_classification_tests {
    use super::*;

    #[test]
    fn completed_nominal_value_indices_require_constructor_authority() {
        let head = DeclarationId {
            package: 82,
            local: 90,
        };
        let indexed = |value| Term::Nominal {
            declaration: head,
            arguments: vec![Term::Bool(value)],
        };
        let goal = Formula::equal(indexed(true), indexed(false)).negate();
        let unknown = Context::default();
        assert!(!matches!(
            unknown.prove(goal.clone(), Budget::default()),
            Outcome::Entailed(_)
        ));
        let mut registered = Context::default();
        registered.admit_nominal_constructor(head);
        let Outcome::Entailed(proof) = registered.prove(goal, Budget::default()) else {
            panic!("completed distinct indices must separate admitted nominal types")
        };
        registered.verify_proof(&proof, Budget::default()).unwrap();
        assert!(unknown.verify_proof(&proof, Budget::default()).is_err());
        assert!(matches!(
            registered.prove(
                Formula::equal(indexed(true), indexed(true)),
                Budget::default()
            ),
            Outcome::Entailed(_)
        ));
    }

    #[test]
    fn tuple_classification_counterexamples_retain_their_authority() {
        let head = DeclarationId {
            package: 82,
            local: 0,
        };
        let nominal_head = DeclarationId {
            package: 82,
            local: 1,
        };
        let tuple = Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![Term::TypeListValue(vec![])],
        };
        let nominal = Term::Nominal {
            declaration: nominal_head,
            arguments: vec![],
        };
        let mut context = Context::default();
        context.admit_tuple_head(head);
        context.admit_nominal_constructor(nominal_head);
        for goal in [
            Formula::kind(tuple.clone(), TypeKind::Boolean),
            Formula::kind(nominal, TypeKind::Tuple),
            Formula::kind(tuple.clone(), TypeKind::Tuple).negate(),
        ] {
            let outcome = context.prove(goal, Budget::default());
            let Outcome::NotEntailed(witness) = &outcome else {
                panic!("{outcome}")
            };
            context.verify_witness(witness, Budget::default()).unwrap();
            assert!(outcome.to_string().contains("(tuple-heads"));
            let mut removed = context.fork();
            Arc::make_mut(&mut removed.tuple_heads).clear();
            assert!(removed.verify_witness(witness, Budget::default()).is_err());
            let mut altered = witness.clone();
            altered.goal = Formula::kind(tuple.clone(), TypeKind::Tuple);
            assert!(context.verify_witness(&altered, Budget::default()).is_err());
        }
        assert!(matches!(
            Context::default().prove(Formula::kind(tuple, TypeKind::Boolean), Budget::default()),
            Outcome::Unknown { .. }
        ));
        let open = Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![Term::Rigid {
                declaration: DeclarationId {
                    package: 82,
                    local: 2,
                },
                sort: Sort::ComptimeValue,
            }],
        };
        assert!(matches!(
            context.prove(Formula::kind(open, TypeKind::Boolean), Budget::default()),
            Outcome::Unknown { .. }
        ));
    }

    #[test]
    fn tuple_classification_is_replayable_without_unfolding_or_injectivity() {
        let head = DeclarationId {
            package: 81,
            local: 0,
        };
        let boolean = Term::Primitive(Primitive::Bool);
        let tuple = |field| Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![Term::TypeListValue(vec![field])],
        };
        let value = tuple(boolean.clone());
        let mut context = Context::default();
        let unrelated = Formula::equal(value.clone(), boolean.clone()).negate();
        assert!(!matches!(
            context.prove(unrelated.clone(), Budget::default()),
            Outcome::Entailed(_)
        ));
        context.admit_tuple_head(head);
        for goal in [
            unrelated,
            Formula::kind(value.clone(), TypeKind::Tuple),
            Formula::kind(value.clone(), TypeKind::Boolean).negate(),
        ] {
            let Outcome::Entailed(proof) = context.prove(goal.clone(), Budget::default()) else {
                panic!("{goal}")
            };
            context.verify_proof(&proof, Budget::default()).unwrap();
            assert!(
                Context::default()
                    .verify_proof(&proof, Budget::default())
                    .is_err()
            );
            assert!(proof.to_string().contains("(tuple-heads"));
        }
        let rigid = Term::Rigid {
            declaration: DeclarationId {
                package: 81,
                local: 1,
            },
            sort: Sort::Type,
        };
        let mut linked = context.fork();
        linked.assume(Formula::equal(rigid.clone(), value.clone()));
        assert!(matches!(
            linked.prove(Formula::kind(rigid, TypeKind::Tuple), Budget::default()),
            Outcome::Entailed(_)
        ));
        // No element injectivity, representation equality, or unregistered nominal exclusion.
        let byte = Term::Primitive(Primitive::U8);
        for goal in [
            Formula::equal(value.clone(), Term::Tuple(vec![boolean.clone()])),
            Formula::equal(
                value.clone(),
                Term::Nominal {
                    declaration: DeclarationId {
                        package: 81,
                        local: 2,
                    },
                    arguments: vec![],
                },
            )
            .negate(),
            Formula::equal(value.clone(), tuple(byte)).negate(),
        ] {
            assert!(!matches!(
                context.prove(goal, Budget::default()),
                Outcome::Entailed(_)
            ));
        }
        let nominal_head = DeclarationId {
            package: 81,
            local: 2,
        };
        let nominal = Term::Nominal {
            declaration: nominal_head,
            arguments: vec![],
        };
        let mut registered = context.fork();
        registered.admit_nominal_constructor(nominal_head);
        for goal in [
            Formula::equal(value.clone(), nominal.clone()).negate(),
            Formula::kind(nominal, TypeKind::Tuple).negate(),
        ] {
            let Outcome::Entailed(proof) = registered.prove(goal, Budget::default()) else {
                panic!("nominal distinction")
            };
            registered.verify_proof(&proof, Budget::default()).unwrap();
            assert!(context.verify_proof(&proof, Budget::default()).is_err());
        }
        assert!(matches!(
            context.prove(
                Formula::kind(value.clone(), TypeKind::Tuple),
                Budget {
                    steps: 0,
                    ..Default::default()
                }
            ),
            Outcome::Unknown { .. }
        ));
        assert!(matches!(
            context.prove(Formula::kind(value, TypeKind::Tuple), Budget::default()),
            Outcome::Entailed(_)
        ));
    }
}
