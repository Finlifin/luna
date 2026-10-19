//! Stable invocation residuals. No inference variables, waiters or interpreter
//! state are stored here; each instantiation supplies its own input frame.
use crate::program::{Expr, ExprKind as K, Literal, LocalId, SourceRange};
use crate::{DeclarationId, Sort, Term, TypeError};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
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
    let free = crate::free_bindings::collect(&body, fuel).map_err(Error::Structure)?;
    if let Some(missing) = free.difference(&bindings).next() {
        return Err(Error::MissingCapture(*missing));
    }
    crate::visit::expression(&body, fuel, |expr, fuel| {
        bounded(&expr.ty, fuel).map_err(|error| error.to_string())?;
        if expr.ty.sort() != Sort::Type {
            return Err("expression annotation is not a Type".into());
        }
        Ok(())
    })
    .map_err(Error::Structure)?;
    let result = body.ty.clone();
    let (parameters, types): (Vec<_>, Vec<_>) = inputs.into_iter().unzip();
    Ok(crate::program::Function {
        declaration,
        name,
        phase: crate::program::FunctionPhase::Comptime,
        inline: false,
        pure: false,
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        signature: Term::Function {
            parameters: types,
            result: Box::new(result),
        },
        parameters,
        defaults: vec![],
        body,
    })
}
impl std::error::Error for Error {}
impl Invocation {
    /// Retain the complete checked comptime body through its package identity.
    /// This is an invocation input frame: ordinary parameters execute inside
    /// comptime and require concrete comptime values, not future runtime reads.
    /// The owning package retains statements, control flow and contracts.
    pub fn from_function(
        function: &crate::program::Function,
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        bounded(&function.signature, fuel)?;
        function.signature.check_scope(&[]).map_err(Error::Scope)?;
        let Term::Function { parameters, result } = function.signature.callable_signature() else {
            return Err(Error::InvalidCall(function.body.source));
        };
        if function.phase != crate::program::FunctionPhase::Comptime
            || !function.type_parameters.is_empty()
            || **result != Term::Primitive(crate::Primitive::Type)
            || function.parameters.len() != parameters.len()
        {
            return Err(Error::InvalidCall(function.body.source));
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
            callee, arguments, ..
        } = &call.kind
        else {
            return Err(Error::UnsupportedExpression(call.source));
        };
        let K::Function(target) = callee.kind else {
            return Err(Error::UnsupportedExpression(callee.source));
        };
        bounded(&callee.ty, fuel)?;
        if !callee.ty.is_comptime_callable() || call.ty != Term::Primitive(crate::Primitive::Type) {
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
            let input = capture_value(&argument.value, &bindings, fuel)?;
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
) -> Result<Input, Error> {
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
        K::TypeValue(ty) => {
            bounded(ty, fuel)?;
            ty.clone()
        }
        K::TypeListValue(types) => {
            for ty in types {
                bounded(ty, fuel)?;
            }
            Term::TypeListValue(types.clone())
        }
        K::Unit => Term::Unit,
        K::Literal(Literal::Bool(value)) => Term::Bool(*value),
        K::Literal(Literal::Char(value)) => Term::Char(*value),
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
    root.instantiate_parameters_in_scope(parameters, arguments, frames)
        .map_err(Error::Scope)
}

fn charge_expanded_term(
    root: &Term,
    fuel: &mut usize,
    max_depth: usize,
    replacements: &BTreeMap<DeclarationId, &Term>,
) -> Result<(), Error> {
    let mut work = vec![(root, 0usize, true)];
    while let Some((term, depth, substitute)) = work.pop() {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        if depth >= max_depth.min(128) {
            return Err(Error::Budget);
        }
        if substitute {
            if let Term::Rigid { declaration, .. } = term {
                if let Some(replacement) = replacements.get(declaration) {
                    work.push((replacement, depth, false));
                    continue;
                }
            }
        }
        // Scalar payloads and empty effect occurrences cost space even when
        // they add no child term. Charge them before cloning or substitution.
        let payload = match term {
            Term::String(value) | Term::Symbol(value) => value.len(),
            Term::Integer(value) => value.bit_len().div_ceil(32),
            Term::Float(value) => value
                .digits()
                .len()
                .saturating_add(value.exponent().bit_len().div_ceil(32)),
            Term::BitVec(value) => value.words().len(),
            Term::Record(fields) | Term::RecordValue(fields) => {
                if fields.len() > *fuel {
                    return Err(Error::Budget);
                }
                fields
                    .iter()
                    .try_fold(0usize, |size, (name, _)| {
                        size.checked_add(name.len()).filter(|size| *size <= *fuel)
                    })
                    .ok_or(Error::Budget)?
            }
            Term::Computation { effects, .. } => effects.len(),
            _ => 0,
        };
        *fuel = fuel.checked_sub(payload).ok_or(Error::Budget)?;
        let mut push = |term| {
            if work.len() >= *fuel {
                return Err(Error::Budget);
            }
            work.push((term, depth + 1, substitute));
            Ok(())
        };
        match term {
            Term::Computation { effects, result } => {
                push(result.as_ref())?;
                for effect in effects {
                    for argument in &effect.arguments {
                        push(argument)?;
                    }
                }
            }
            Term::AssociatedType {
                arguments, subject, ..
            } => {
                push(subject.as_ref())?;
                for argument in arguments {
                    push(argument)?;
                }
            }
            Term::Nominal { arguments, .. }
            | Term::ClosureSnapshot { arguments, .. }
            | Term::FunctionValue { arguments, .. }
            | Term::NormalForm { arguments, .. }
            | Term::Tuple(arguments)
            | Term::TupleValue(arguments)
            | Term::TypeListValue(arguments)
            | Term::TypeListConcat(arguments) => {
                for argument in arguments {
                    push(argument)?;
                }
            }
            Term::Closure {
                environment,
                signature,
            } => {
                push(signature.as_ref())?;
                for ty in environment {
                    push(ty)?;
                }
            }
            Term::NominalValue { ty, fields, .. } => {
                push(ty)?;
                for field in fields {
                    push(field)?;
                }
            }
            Term::Record(fields) | Term::RecordValue(fields) => {
                for (_, ty) in fields {
                    push(ty)?;
                }
            }
            Term::Pointer(ty) | Term::ComptimeFunction(ty) => push(ty.as_ref())?,
            Term::Function { parameters, result } => {
                push(result.as_ref())?;
                for parameter in parameters {
                    push(&parameter.ty)?;
                }
            }
            _ => {}
        }
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

/// A compact entry plus its input positions in the original declaration frame.
/// This stable mapping is used to supply each instance, not a mutable waiter.
#[derive(Clone, Debug, PartialEq)]
pub struct CapturedEntry {
    pub function: crate::program::Function,
    pub slots: Vec<usize>,
    source_scope: Vec<Sort>,
}
impl CapturedEntry {
    pub fn source_scope(&self) -> &[Sort] {
        &self.source_scope
    }

    /// Save the call in the source declaration's frame, even though the owned
    /// entry uses a compact frame. Instantiation can then supply its ordinary
    /// input telescope directly; pruned runtime slots never become waiters.
    pub fn invocation(&self, fuel: &mut usize) -> Result<Invocation, Error> {
        *fuel = fuel
            .checked_sub(self.source_scope.len())
            .ok_or(Error::Budget)?;
        let compact = Invocation::from_function(&self.function, fuel)?;
        if compact.scope().len() != self.slots.len() {
            return Err(Error::InvalidCall(self.function.body.source));
        }
        let mut seen = BTreeSet::new();
        let mut mapping = Vec::with_capacity(self.slots.len());
        for (index, slot) in self.slots.iter().copied().enumerate() {
            let sort = *self
                .source_scope
                .get(slot)
                .ok_or(Error::Scope(TypeError::MissingArgument(slot)))?;
            if !seen.insert(slot) {
                return Err(Error::InvalidCall(self.function.body.source));
            }
            if sort != compact.scope()[index] {
                return Err(Error::Scope(TypeError::WrongSort {
                    expected: compact.scope()[index],
                    actual: sort,
                }));
            }
            mapping.push(Term::Bound {
                depth: 0,
                index: slot,
                sort,
            });
        }
        let inputs = compact
            .inputs()
            .iter()
            .map(|input| {
                bounded(&input.ty, fuel)?;
                bounded(&input.value, fuel)?;
                Ok(Input {
                    ty: input.ty.rebind_scope(&mapping).map_err(Error::Scope)?,
                    value: input.value.rebind_scope(&mapping).map_err(Error::Scope)?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Invocation::new(compact.target(), self.source_scope.clone(), inputs, fuel)
    }
}
impl fmt::Display for CapturedEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(captured-comptime-entry (slots")?;
        for slot in &self.slots {
            write!(f, " {slot}")?;
        }
        write!(f, ") (source-scope")?;
        for sort in &self.source_scope {
            write!(f, " {sort}")?;
        }
        write!(f, ") (function {}))", self.function)
    }
}
/// Discover free value inputs and transitive annotation dependencies, retaining
/// their original order. Expr annotations use the enclosing ordinary input
/// frame; nested callable signatures introduce their own Term binder normally.
pub fn capture_entry(
    declaration: DeclarationId,
    name: String,
    body: Expr,
    available: &[(LocalId, crate::Parameter)],
    fuel: &mut usize,
) -> Result<CapturedEntry, Error> {
    *fuel = fuel.checked_sub(available.len()).ok_or(Error::Budget)?;
    let available = available
        .iter()
        .map(|(id, parameter)| (*id, parameter.sort, Some(&parameter.ty)))
        .collect::<Vec<_>>();
    capture_entry_domains(declaration, name, body, &available, fuel)
}

/// Capture a source telescope with unresolved domains. Such slots keep their
/// positions and phase but cannot be captured until their domains are known.
/// Runtime inputs remain forbidden even when their domains are known.
pub fn capture_sparse_entry(
    declaration: DeclarationId,
    name: String,
    body: Expr,
    available: &[(LocalId, Sort, Option<Term>)],
    fuel: &mut usize,
) -> Result<CapturedEntry, Error> {
    *fuel = fuel.checked_sub(available.len()).ok_or(Error::Budget)?;
    let available = available
        .iter()
        .map(|(id, sort, domain)| (*id, *sort, domain.as_ref()))
        .collect::<Vec<_>>();
    capture_entry_domains(declaration, name, body, &available, fuel)
}

fn capture_entry_domains(
    declaration: DeclarationId,
    name: String,
    mut body: Expr,
    available: &[(LocalId, Sort, Option<&Term>)],
    fuel: &mut usize,
) -> Result<CapturedEntry, Error> {
    *fuel = fuel.checked_sub(available.len()).ok_or(Error::Budget)?;
    let mut positions = BTreeMap::new();
    for (slot, (id, sort, domain)) in available.iter().enumerate() {
        if domain.is_none() && *sort == Sort::Type {
            return Err(Error::Scope(TypeError::MissingArgument(slot)));
        }
        if positions.insert(*id, slot).is_some() {
            return Err(Error::DuplicateBinding(*id));
        }
    }
    let mut needed = BTreeSet::new();
    for id in crate::free_bindings::collect(&body, fuel).map_err(Error::Structure)? {
        needed.insert(*positions.get(&id).ok_or(Error::MissingCapture(id))?);
    }
    let frames = vec![
        available
            .iter()
            .map(|(_, sort, _)| *sort)
            .collect::<Vec<_>>(),
    ];
    // Inspect every type-bearing operand, including capture environments and
    // Type values that mention an input without a corresponding Local read.
    body.rewrite_types(&mut |ty| {
        bounded(ty, fuel)?;
        ty.check_scope(&frames).map_err(Error::Scope)?;
        needed.extend(
            ty.substitution_parameters()
                .map_err(Error::Scope)?
                .into_keys(),
        );
        Ok::<_, Error>(ty.clone())
    })?;
    let mut work = needed.iter().copied().collect::<Vec<_>>();
    let mut inspected = BTreeSet::new();
    while let Some(slot) = work.pop() {
        if !inspected.insert(slot) {
            continue;
        }
        let (_, sort, domain) = available
            .get(slot)
            .ok_or(Error::Scope(TypeError::MissingArgument(slot)))?;
        if *sort == Sort::Runtime {
            return Err(Error::RuntimeInput(slot));
        }
        let domain = domain
            .as_ref()
            .ok_or(Error::Scope(TypeError::MissingArgument(slot)))?;
        bounded(domain, fuel)?;
        domain.check_scope(&frames).map_err(Error::Scope)?;
        for dependency in domain
            .substitution_parameters()
            .map_err(Error::Scope)?
            .into_keys()
        {
            if needed.insert(dependency) {
                work.push(dependency);
            }
        }
    }
    let slots = needed.into_iter().collect::<Vec<_>>();
    let mut mapping = vec![None; available.len()];
    for (new, old) in slots.iter().copied().enumerate() {
        mapping[old] = Some(Term::Bound {
            depth: 0,
            index: new,
            sort: available[old].1,
        });
    }
    body.rewrite_types(&mut |ty| {
        bounded(ty, fuel)?;
        ty.rebind_scope_partial(&mapping).map_err(Error::Scope)
    })?;
    let inputs = slots
        .iter()
        .map(|slot| {
            let (id, sort, domain) = &available[*slot];
            Ok((
                *id,
                crate::Parameter {
                    sort: *sort,
                    ty: domain
                        .as_ref()
                        .ok_or(Error::Scope(TypeError::MissingArgument(*slot)))?
                        .rebind_scope_partial(&mapping)
                        .map_err(Error::Scope)?,
                },
            ))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let function = expression_entry(declaration, name, body, inputs, fuel)?;
    function.signature.check_scope(&[]).map_err(Error::Scope)?;
    Ok(CapturedEntry {
        function,
        slots,
        source_scope: frames.into_iter().next().unwrap(),
    })
}
