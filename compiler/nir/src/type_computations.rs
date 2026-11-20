//! Stable invocation residuals. No inference variables, waiters or interpreter
//! state are stored here; each instantiation supplies its own input frame.
use crate::program::{Expr, ExprKind as K, Literal, LocalId, SourceRange};
use crate::{DeclarationId, Sort, Term, TypeError};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

mod capture;
pub use capture::{
    CaptureBody, CapturedEntry, capture_entry, capture_entry_domains, capture_sparse_entry,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub ty: Term,
    pub value: Term,
}

/// An ordinary comptime call producing Type, not an opaque type constructor.
/// Target admission and execution are checked against the owning package when
/// instantiating. Private fields prevent bypassing binder/dependency validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    target: DeclarationId,
    scope: Vec<Sort>,
    inputs: Vec<Input>,
    dependencies: BTreeSet<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Scope(TypeError),
    RuntimeInput(usize),
    Budget,
    UnsupportedExpression(SourceRange),
    InvalidCall(SourceRange),
    DuplicateBinding(LocalId),
    MissingCapture(LocalId),
    Structure(String),
}
impl From<crate::copy_budget::CopyBudgetExhausted> for Error {
    fn from(_: crate::copy_budget::CopyBudgetExhausted) -> Self {
        Self::Budget
    }
}

impl From<crate::visit::Error> for Error {
    fn from(error: crate::visit::Error) -> Self {
        match error {
            crate::visit::Error::Budget(_) => Self::Budget,
            crate::visit::Error::Invalid(message) => Self::Structure(message.into()),
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scope(error) => write!(f, "invalid residual invocation scope: {error}"),
            Self::RuntimeInput(slot) => write!(
                f,
                "residual comptime invocation cannot read runtime input {slot}"
            ),
            Self::Budget => write!(f, "residual invocation structure budget exhausted"),
            Self::UnsupportedExpression(source) => {
                write!(f, "expression requires a residual code body at {source}")
            }
            Self::InvalidCall(source) => write!(f, "invalid residual Type call at {source}"),
            Self::DuplicateBinding(binding) => {
                write!(f, "duplicate residual input binding {binding}")
            }
            Self::MissingCapture(binding) => {
                write!(f, "comptime entry is missing capture {binding}")
            }
            Self::Structure(message) => write!(f, "invalid comptime entry structure: {message}"),
        }
    }
}

/// Turn an already elaborated expression and its explicit environment into an
/// owned comptime entry. Binding/type legality within the expression remains
/// the producer's responsibility; this is not a replacement for elaboration.
/// Both required evaluation and saved residual bodies use the same entry shape.
pub fn expression_entry(
    declaration: DeclarationId,
    name: String,
    body: Expr,
    inputs: Vec<(LocalId, crate::Parameter)>,
    fuel: &mut usize,
) -> Result<crate::program::Function, Error> {
    *fuel = fuel.checked_sub(inputs.len()).ok_or(Error::Budget)?;
    let mut bindings = BTreeSet::new();
    for (binding, parameter) in &inputs {
        if !bindings.insert(*binding) {
            return Err(Error::DuplicateBinding(*binding));
        }
        bounded(&parameter.ty, fuel)?;
        if parameter.ty.sort() != Sort::Type {
            return Err(Error::Scope(TypeError::WrongSort {
                expected: Sort::Type,
                actual: parameter.ty.sort(),
            }));
        }
    }
    let free = crate::free_bindings::collect(&body, fuel)?;
    if let Some(missing) = free.difference(&bindings).next() {
        return Err(Error::MissingCapture(*missing));
    }
    entry_with_bound_locals(declaration, name, body, inputs, fuel)
}

/// The caller has checked free locals against unique input identities.
fn entry_with_bound_locals(
    declaration: DeclarationId,
    name: String,
    body: Expr,
    inputs: Vec<(LocalId, crate::Parameter)>,
    fuel: &mut usize,
) -> Result<crate::program::Function, Error> {
    crate::visit::expression_checked(&body, fuel, |expr, fuel| {
        bounded(&expr.ty, fuel)?;
        if expr.ty.sort() != Sort::Type {
            return Err(Error::Structure(
                "expression annotation is not a Type".into(),
            ));
        }
        Ok::<_, Error>(())
    })?;
    let result = body.ty.clone();
    let (parameters, types): (Vec<_>, Vec<_>) = inputs.into_iter().unzip();
    Ok(crate::program::Function {
        interface: crate::program::FunctionDeclaration {
            source: body.source,
            declaration,
            name,
            phase: crate::program::FunctionPhase::Comptime,
            inline: false,
            pure: false,
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
            signature: Term::Function {
                parameters: types,
                result: Box::new(result),
            },
            parameters,
            env_parameters: vec![],
            defaults: vec![],
        },
        body,
        runtime_deliveries: Default::default(),
    })
}
impl std::error::Error for Error {}
impl Invocation {
    /// Prepare the input frame from the declaration of a Type computation.
    /// This is an invocation input frame: ordinary parameters execute inside
    /// comptime and require concrete comptime values, not future runtime reads.
    /// The owning package retains statements, control flow and contracts.
    pub fn from_declaration(
        function: &crate::program::FunctionDeclaration,
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        bounded(&function.signature, fuel)?;
        function.signature.check_scope(&[]).map_err(Error::Scope)?;
        let Term::Function { parameters, result } = function.signature.callable_signature() else {
            return Err(Error::InvalidCall(function.source));
        };
        if function.phase != crate::program::FunctionPhase::Comptime
            || !function.type_parameters.is_empty()
            || **result != Term::Primitive(crate::Primitive::Type)
            || function.parameters.len() != parameters.len()
        {
            return Err(Error::InvalidCall(function.source));
        }
        *fuel = fuel.checked_sub(parameters.len()).ok_or(Error::Budget)?;
        let mut ids = BTreeSet::new();
        for id in &function.parameters {
            if !ids.insert(*id) {
                return Err(Error::DuplicateBinding(*id));
            }
        }
        let scope = parameters
            .iter()
            .map(|parameter| match parameter.sort {
                Sort::Runtime if parameter.ty == Term::Primitive(crate::Primitive::Type) => {
                    Sort::Type
                }
                Sort::Runtime => Sort::ComptimeValue,
                sort => sort,
            })
            .collect::<Vec<_>>();
        let inputs = parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| Input {
                ty: parameter.ty.clone(),
                value: Term::Bound {
                    depth: 0,
                    index,
                    sort: scope[index],
                },
            })
            .collect();
        Self::new(function.declaration, scope, inputs, fuel)
    }
    /// Capture a checked direct call whose operands are immutable values or
    /// input bindings. Calls/control flow in operands need a saved code body;
    /// never evaluate or discard them while extracting a value-only residual.
    pub fn from_call(
        root: &Expr,
        captures: &[(LocalId, crate::Parameter)],
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        *fuel = fuel.checked_sub(captures.len()).ok_or(Error::Budget)?;
        let mut bindings = BTreeMap::new();
        let scope = captures
            .iter()
            .map(|(_, parameter)| parameter.sort)
            .collect::<Vec<_>>();
        for (index, (id, parameter)) in captures.iter().enumerate() {
            bounded(&parameter.ty, fuel)?;
            if bindings
                .insert(
                    *id,
                    Input {
                        ty: parameter.ty.clone(),
                        value: Term::Bound {
                            depth: 0,
                            index,
                            sort: parameter.sort,
                        },
                    },
                )
                .is_some()
            {
                return Err(Error::DuplicateBinding(*id));
            }
        }
        let mut call = root;
        loop {
            *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
            if call.ty != Term::Primitive(crate::Primitive::Type) {
                return Err(Error::InvalidCall(call.source));
            }
            match &call.kind {
                K::Block {
                    statements,
                    result: Some(tail),
                }
                | K::CallPreparation {
                    statements,
                    result: Some(tail),
                } => {
                    // Even a dead temporary can carry evaluation/ownership
                    // obligations. It needs a residual body, not extraction.
                    if let Some(statement) = statements.first() {
                        return Err(Error::UnsupportedExpression(statement.source));
                    }
                    call = tail;
                }
                _ => break,
            }
        }
        let K::Call {
            callee,
            arguments,
            env_arguments,
            ..
        } = &call.kind
        else {
            return Err(Error::UnsupportedExpression(call.source));
        };
        let K::Function(target) = callee.kind else {
            return Err(Error::UnsupportedExpression(callee.source));
        };
        bounded(&callee.ty, fuel)?;
        if !callee.ty.is_comptime_callable()
            || !env_arguments.is_empty()
            || call.ty != Term::Primitive(crate::Primitive::Type)
        {
            return Err(Error::InvalidCall(call.source));
        }
        let Term::Function { parameters, result } = callee.ty.callable_signature() else {
            return Err(Error::InvalidCall(call.source));
        };
        if **result != call.ty || arguments.len() != parameters.len() {
            return Err(Error::InvalidCall(call.source));
        }
        *fuel = fuel.checked_sub(arguments.len()).ok_or(Error::Budget)?;
        let mut inputs = vec![None; parameters.len()];
        for argument in arguments {
            let input = capture_value(&argument.value, &bindings, fuel, 0)?;
            let slot = inputs
                .get_mut(argument.parameter)
                .ok_or(Error::InvalidCall(call.source))?;
            if slot.replace(input).is_some() {
                return Err(Error::InvalidCall(call.source));
            }
        }
        let inputs = inputs
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(Error::InvalidCall(call.source))?;
        Self::new(target, scope, inputs, fuel)
    }
    pub fn new(
        target: DeclarationId,
        scope: Vec<Sort>,
        inputs: Vec<Input>,
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        *fuel = fuel.checked_sub(scope.len()).ok_or(Error::Budget)?;
        let frames = vec![scope.clone()];
        let mut dependencies = BTreeSet::new();
        for input in &inputs {
            for term in [&input.ty, &input.value] {
                bounded(term, fuel)?;
                term.check_scope(&frames).map_err(Error::Scope)?;
                for (index, sort) in term.substitution_parameters().map_err(Error::Scope)? {
                    if sort == Sort::Runtime {
                        return Err(Error::RuntimeInput(index));
                    }
                    dependencies.insert(index);
                }
            }
            if input.ty.sort() != Sort::Type {
                return Err(Error::Scope(TypeError::WrongSort {
                    expected: Sort::Type,
                    actual: input.ty.sort(),
                }));
            }
        }
        Ok(Self {
            target,
            scope,
            inputs,
            dependencies,
        })
    }
    pub fn target(&self) -> DeclarationId {
        self.target
    }
    pub fn scope(&self) -> &[Sort] {
        &self.scope
    }
    pub fn inputs(&self) -> &[Input] {
        &self.inputs
    }
    pub fn dependencies(&self) -> &BTreeSet<usize> {
        &self.dependencies
    }
}
fn capture_value(
    expr: &Expr,
    bindings: &BTreeMap<LocalId, Input>,
    fuel: &mut usize,
    depth: usize,
) -> Result<Input, Error> {
    if depth >= 128 {
        return Err(Error::Budget);
    }
    *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
    bounded(&expr.ty, fuel)?;
    let value = match &expr.kind {
        K::Local(binding) => {
            let value = bindings
                .get(binding)
                .ok_or(Error::UnsupportedExpression(expr.source))?;
            if value.ty != expr.ty {
                return Err(Error::InvalidCall(expr.source));
            }
            bounded(&value.value, fuel)?;
            value.value.clone()
        }
        K::ComptimeValue(value) => {
            crate::comptime_values::validate(value, &expr.ty, fuel).map_err(
                |error| match error {
                    crate::comptime_values::Error::Budget => Error::Budget,
                    crate::comptime_values::Error::Invalid(_) => Error::InvalidCall(expr.source),
                },
            )?;
            value.clone()
        }
        K::TypeValue(ty) => {
            if expr.ty != Term::Primitive(crate::Primitive::Type) || ty.sort() != Sort::Type {
                return Err(Error::InvalidCall(expr.source));
            }
            bounded(ty, fuel)?;
            ty.clone()
        }
        K::TypeList(elements) => {
            let Term::NormalForm {
                sort: Sort::Type,
                arguments,
                ..
            } = &expr.ty
            else {
                return Err(Error::InvalidCall(expr.source));
            };
            let [element] = arguments.as_slice() else {
                return Err(Error::InvalidCall(expr.source));
            };
            *fuel = fuel.checked_sub(elements.len()).ok_or(Error::Budget)?;
            let mut fields = Vec::with_capacity(elements.len());
            for value in elements {
                if value.ty != *element {
                    return Err(Error::InvalidCall(value.source));
                }
                fields.push(capture_value(value, bindings, fuel, depth + 1)?.value);
            }
            freeze_list(&expr.ty, fields, expr.source, fuel)?
        }
        K::Unit => Term::Unit,
        K::Literal(Literal::Bool(value)) => Term::Bool(*value),
        K::Literal(Literal::Char(value)) => Term::Char(*value),
        K::Literal(Literal::Float16(value)) => Term::Float16(*value),
        K::Literal(Literal::BFloat16(value)) => Term::BFloat16(*value),
        K::Literal(Literal::Float32(value)) => Term::Float32(*value),
        K::Literal(Literal::Float64(value)) => Term::Float64(*value),
        K::Literal(Literal::SignedInteger(value)) => {
            Term::Integer(comptime_runtime::Integer::from_i128(*value))
        }
        K::Literal(Literal::UnsignedInteger(value)) => {
            Term::Integer(comptime_runtime::Integer::from_u128(*value))
        }
        K::Literal(Literal::Integer(value)) => {
            let mut budget = comptime_runtime::Budget {
                steps: *fuel,
                max_limbs: 4096,
            };
            let result = comptime_runtime::Integer::parse_literal(value, &mut budget);
            *fuel = budget.steps;
            Term::Integer(result.map_err(|error| match error {
                comptime_runtime::Error::BudgetExhausted | comptime_runtime::Error::SizeLimit => {
                    Error::Budget
                }
                _ => Error::InvalidCall(expr.source),
            })?)
        }
        K::Literal(Literal::Float(value)) => {
            if expr.ty != Term::Primitive(crate::Primitive::Float) {
                return Err(Error::InvalidCall(expr.source));
            }
            let mut budget = comptime_runtime::Budget {
                steps: *fuel,
                max_limbs: 4096,
            };
            let result = comptime_runtime::decimal::DecimalValue::parse_literal(value, &mut budget);
            *fuel = budget.steps;
            Term::Float(result.map_err(|error| match error {
                comptime_runtime::Error::BudgetExhausted | comptime_runtime::Error::SizeLimit => {
                    Error::Budget
                }
                _ => Error::InvalidCall(expr.source),
            })?)
        }
        K::Literal(Literal::Symbol(value)) => {
            *fuel = fuel.checked_sub(value.len()).ok_or(Error::Budget)?;
            Term::Symbol(value.as_str().into())
        }
        K::Literal(Literal::String(value)) => {
            *fuel = fuel.checked_sub(value.len()).ok_or(Error::Budget)?;
            Term::String(value.as_str().into())
        }
        _ => return Err(Error::UnsupportedExpression(expr.source)),
    };
    Ok(Input {
        ty: expr.ty.clone(),
        value,
    })
}

// Typed list nodes already carry their elaborated layout authority. Capturing
// their immutable payload does not infer that authority from an opaque head.
fn freeze_list(
    domain: &Term,
    fields: Vec<Term>,
    source: SourceRange,
    fuel: &mut usize,
) -> Result<Term, Error> {
    crate::comptime_values::freeze_admitted_list(domain, fields, fuel).map_err(
        |error| match error {
            crate::comptime_values::Error::Budget => Error::Budget,
            crate::comptime_values::Error::Invalid(_) => Error::InvalidCall(source),
        },
    )
}
pub(crate) use charge_term as bounded;

/// Charge term structure and payloads before cloning or recursive binder work.
/// This is a resource guard, not type or formation validation.
pub fn charge_term(root: &Term, fuel: &mut usize) -> Result<(), Error> {
    charge_term_with_depth(root, fuel, 128)
}

/// Apply a caller's tighter depth limit as well as the implementation ceiling.
pub fn charge_term_with_depth(
    root: &Term,
    fuel: &mut usize,
    max_depth: usize,
) -> Result<(), Error> {
    charge_expanded_term(root, fuel, max_depth, &BTreeMap::new())
}

/// Preflight simultaneous substitution before allocating its expanded result.
/// Repeated occurrences each charge the replacement payload. Replacements are
/// not themselves substituted, and opaque heads are never evaluated.
pub fn instantiate_parameters_bounded(
    root: &Term,
    parameters: &[crate::DeclarationParameter],
    arguments: &[Term],
    frames: &[Vec<Sort>],
    fuel: &mut usize,
) -> Result<Term, Error> {
    if parameters.len() != arguments.len() {
        return Err(Error::Scope(TypeError::ArgumentCount {
            expected: parameters.len(),
            actual: arguments.len(),
        }));
    }
    instantiate_bindings_bounded(
        root,
        parameters.iter().copied().zip(arguments),
        frames,
        fuel,
    )
}

/// Instantiate a structural domain from the supplied declaration bindings.
/// Validate even unused inputs; the expansion budget pays for each occurrence
/// in the result. The caller retains ownership of all replacement values.
pub fn instantiate_bindings_bounded<'a>(
    root: &Term,
    bindings: impl IntoIterator<Item = (crate::DeclarationParameter, &'a Term)>,
    frames: &[Vec<Sort>],
    fuel: &mut usize,
) -> Result<Term, Error> {
    crate::types::ParameterSubstitution::instantiate_bounded(root, bindings, frames, fuel)
}

/// Bound declaration substitution before allocating its result. This accepts
/// signature fragments whose enclosing binder frames remain with the caller;
/// it is not scope or sort admission. The substitution and owning signature
/// must still perform their ordinary checks.
pub fn charge_parameter_substitution(
    root: &Term,
    parameters: &[crate::DeclarationParameter],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<(), Error> {
    if parameters.len() != arguments.len() {
        return Err(Error::Scope(TypeError::ArgumentCount {
            expected: parameters.len(),
            actual: arguments.len(),
        }));
    }
    *fuel = fuel.checked_sub(parameters.len()).ok_or(Error::Budget)?;
    charge_term(root, fuel)?;
    for argument in arguments {
        charge_term(argument, fuel)?;
    }
    let replacements = parameters
        .iter()
        .zip(arguments)
        .map(|(parameter, argument)| (parameter.declaration, argument))
        .collect();
    charge_expanded_term(root, fuel, 128, &replacements)?;
    Ok(())
}

pub(crate) fn charge_expanded_term(
    root: &Term,
    fuel: &mut usize,
    max_depth: usize,
    replacements: &BTreeMap<DeclarationId, &Term>,
) -> Result<(), Error> {
    visit_expanded_term(root, fuel, max_depth, replacements, &mut |_| {})
}

/// Check for a projection anywhere in an operand without unfolding its heads
/// or extracting terms from under callable binders.
pub fn contains_associated_projection(
    root: &Term,
    fuel: &mut usize,
    depth: usize,
) -> Result<bool, Error> {
    let mut found = false;
    visit_expanded_term(root, fuel, depth, &BTreeMap::new(), &mut |term| {
        found |= matches!(term, Term::AssociatedType { .. });
    })?;
    Ok(found)
}

/// Can specialization still change the set of local runtime identities in this
/// argument? An unresolved binder or projection can introduce or merge slots.
pub fn runtime_dependencies_may_change(root: &Term, fuel: &mut usize) -> Result<bool, Error> {
    let mut open = false;
    visit_expanded_term(root, fuel, 128, &BTreeMap::new(), &mut |term| {
        open |= matches!(
            term,
            Term::Rigid { .. }
                | Term::Bound { .. }
                | Term::Hole { .. }
                | Term::AssociatedType { .. }
        );
    })?;
    Ok(open)
}

/// Inspect open declaration inputs without cloning the type or unfolding heads.
pub fn has_rigid_parameters_bounded(root: &Term, fuel: &mut usize) -> Result<bool, Error> {
    mentions_rigid_bounded(root, fuel, |_| true)
}

/// Inspect the whole term under the caller's traversal and payload budget.
/// Only rigid references participate, including those inside callable values.
pub fn mentions_rigid_bounded(
    root: &Term,
    fuel: &mut usize,
    mut matches_id: impl FnMut(DeclarationId) -> bool,
) -> Result<bool, Error> {
    let mut found = false;
    visit_rigid_declarations(root, fuel, |id| found |= matches_id(id))?;
    Ok(found)
}

/// Observe declaration inputs structurally without cloning terms or unfolding
/// opaque heads. The caller must discard the partial set if observation fails.
pub fn rigid_declarations_bounded(
    root: &Term,
    fuel: &mut usize,
    declarations: &mut BTreeSet<DeclarationId>,
) -> Result<(), Error> {
    visit_rigid_declarations(root, fuel, |id| {
        declarations.insert(id);
    })
}

/// Inspect declaration references while charging the same structural and
/// payload budget as a subsequent binder rewrite.
pub(crate) fn visit_rigid_declarations(
    root: &Term,
    fuel: &mut usize,
    mut visit: impl FnMut(DeclarationId),
) -> Result<(), Error> {
    visit_expanded_term(root, fuel, 128, &BTreeMap::new(), &mut |term| {
        if let Term::Rigid { declaration, .. } = term {
            visit(*declaration);
        }
    })
}

/// Collect nominal identities mentioned structurally, without unfolding opaque
/// normal-form heads or substituting open parameters. The walk is budgeted.
pub fn nominal_declarations_bounded(
    root: &Term,
    fuel: &mut usize,
    declarations: &mut BTreeSet<DeclarationId>,
) -> Result<(), Error> {
    visit_expanded_term(root, fuel, 128, &BTreeMap::new(), &mut |term| {
        if let Term::Nominal { declaration, .. } = term {
            declarations.insert(*declaration);
        }
    })
}

pub(crate) fn visit_expanded_term(
    root: &Term,
    fuel: &mut usize,
    max_depth: usize,
    replacements: &BTreeMap<DeclarationId, &Term>,
    visit: &mut impl FnMut(&Term),
) -> Result<(), Error> {
    visit_expanded_term_for_copy(root, fuel, max_depth, replacements, visit)
        .map_err(|_| Error::Budget)
}

/// Preserve the exhausted limit for copy preflight and VM diagnostics. The
/// residual-invocation API above retains its existing aggregate budget error.
pub(crate) fn visit_expanded_term_for_copy(
    root: &Term,
    fuel: &mut usize,
    max_depth: usize,
    replacements: &BTreeMap<DeclarationId, &Term>,
    visit: &mut impl FnMut(&Term),
) -> Result<(), crate::copy_budget::CopyBudgetExhausted> {
    visit_expanded_term_scoped_for_copy(root, fuel, max_depth, replacements, &mut |term, _| {
        visit(term)
    })
}

/// Structural observation with the number of function binders crossed from
/// this term root. Callable phase wrappers and closure environments introduce
/// no binder; function parameter domains and results share one new frame.
pub(crate) fn visit_expanded_term_scoped_for_copy(
    root: &Term,
    fuel: &mut usize,
    max_depth: usize,
    replacements: &BTreeMap<DeclarationId, &Term>,
    visit: &mut dyn FnMut(&Term, u32),
) -> Result<(), crate::copy_budget::CopyBudgetExhausted> {
    visit_expansion(
        root,
        fuel,
        max_depth,
        &|id| replacements.get(&id).copied(),
        visit,
    )
}

/// The receiving telescope already indexes its checked arguments. Look up only
/// replacements reached by the template, charging each actual occurrence.
pub(crate) fn visit_expansion<'a>(
    root: &Term,
    fuel: &mut usize,
    max_depth: usize,
    replacement: &dyn Fn(DeclarationId) -> Option<&'a Term>,
    visit: &mut dyn FnMut(&Term, u32),
) -> Result<(), crate::copy_budget::CopyBudgetExhausted> {
    visit_replacements(
        root,
        fuel,
        max_depth,
        &|term, _| match term {
            Term::Rigid { declaration, .. } => replacement(*declaration),
            _ => None,
        },
        visit,
    )
}

/// Charge the materialized tree of an ordinary input substitution. The depth
/// of a nested function protects its own binders; each use pays for the full
/// replacement, whose contents are never interpreted as another template.
pub(crate) fn charge_frame_expansion(
    root: &Term,
    inputs: &[Term],
    fuel: &mut usize,
) -> Result<(), Error> {
    charge_frame_slots(root, |index| inputs.get(index), fuel)
}

impl Term {
    /// Remove one caller frame, charging only reached replacements and their
    /// materialized occurrences, including those under nested binders.
    pub fn substitute_partial_bounded(
        &self,
        arguments: &[Option<Term>],
        fuel: &mut usize,
    ) -> Result<Term, Error> {
        charge_frame_slots(self, |index| arguments.get(index)?.as_ref(), fuel)?;
        self.substitute_partial(arguments).map_err(Error::Scope)
    }
}

fn charge_frame_slots<'a>(
    root: &Term,
    slot: impl Fn(usize) -> Option<&'a Term>,
    fuel: &mut usize,
) -> Result<(), Error> {
    Ok(visit_replacements(
        root,
        fuel,
        128,
        &|term, nesting| match term {
            Term::Bound { depth, index, .. } if *depth == nesting => slot(*index),
            _ => None,
        },
        &mut |_, _| {},
    )?)
}

fn visit_replacements<'a>(
    root: &Term,
    fuel: &mut usize,
    max_depth: usize,
    replacement: &dyn Fn(&Term, u32) -> Option<&'a Term>,
    visit: &mut dyn FnMut(&Term, u32),
) -> Result<(), crate::copy_budget::CopyBudgetExhausted> {
    use crate::copy_budget::CopyBudgetExhausted as Limit;
    // Keep the next visit inline. Leaves and unary paths have no siblings to store.
    let mut next = Some((root, 0usize, 0u32, true));
    let mut siblings = Vec::new();
    while let Some((term, depth, nesting, substitute)) = next.take().or_else(|| siblings.pop()) {
        *fuel = fuel.checked_sub(1).ok_or(Limit::Steps)?;
        if depth >= max_depth.min(128) {
            return Err(Limit::Depth);
        }
        if substitute && let Some(value) = replacement(term, nesting) {
            next = Some((value, depth, nesting, false));
            continue;
        }
        visit(term, nesting);
        crate::copy_budget::charge_term_payload(term, fuel)?;
        let child_nesting = nesting + u32::from(matches!(term, Term::Function { .. }));
        let mut push = |term| {
            if siblings.len() + usize::from(next.is_some()) >= *fuel {
                return Err(Limit::Steps);
            }
            if let Some(previous) = next.replace((term, depth + 1, child_nesting, substitute)) {
                siblings.push(previous);
            }
            Ok(())
        };
        term.visit_children(&mut push)?;
    }
    Ok(())
}
impl fmt::Display for Invocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(residual-type-invocation (target {}) (scope",
            self.target
        )?;
        for sort in &self.scope {
            write!(f, " {sort}")?;
        }
        write!(f, ") (inputs")?;
        for input in &self.inputs {
            write!(f, " (input (type {}) (value {}))", input.ty, input.value)?;
        }
        write!(f, ") (dependencies")?;
        for slot in &self.dependencies {
            write!(f, " {slot}")?;
        }
        write!(f, "))")
    }
}
