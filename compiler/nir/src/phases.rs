//! Execution-phase boundary for runtime and phase-bearing comptime callables.
//! This validates phase only, not resource legality, purity, or termination.
use crate::program::{Expr, ExprKind as K, FunctionPhase as Phase, Package, Pattern, SourceRange};
use crate::{Primitive, Sort, Term, contracts::CallableProgram};
use std::collections::{BTreeMap, BTreeSet};

mod comptime_storage;
#[cfg(test)]
mod inspection_tests;
mod member_types;
mod parameters;
mod storage_cache;
mod storage_error;
mod trait_index;
mod worklist;
pub use storage_error::StorageError;
use worklist::{Work, Worklist};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub source: SourceRange,
    pub message: &'static str,
}

pub fn validate(package: &Package) -> Result<(), Error> {
    machine_arithmetic_functions(package).map(|_| ())
}
/// Full phase traversal, also collecting owners whose bodies/defaults contain
/// machine arithmetic (including nested closures and patterns).
pub fn machine_arithmetic_functions(
    package: &Package,
) -> Result<std::collections::BTreeSet<crate::DeclarationId>, Error> {
    machine_arithmetic_functions_with_budgets(package, &mut 1_000_000, &mut 1_000_000)
}

// Declaration admission and expression inspection own separate allowances.
// This is not yet the VM budget: some metadata and shape checks remain local.
fn machine_arithmetic_functions_with_budgets(
    package: &Package,
    declaration_fuel: &mut usize,
    inspection_fuel: &mut usize,
) -> Result<std::collections::BTreeSet<crate::DeclarationId>, Error> {
    let mut machine_owners = std::collections::BTreeSet::new();
    let parameter_domains = parameters::domains(package, declaration_fuel)
        .map_err(parameters::Failure::into_diagnostic)?;
    for definition in &package.enums {
        definition
            .validate_encoding_in(package.target)
            .map_err(|message| Error {
                source: SourceRange { start: 0, end: 0 },
                message,
            })?;
    }
    let trait_index =
        trait_index::Index::new(package, declaration_fuel).map_err(|error| Error {
            source: SourceRange { start: 0, end: 0 },
            message: error.message(),
        })?;
    // Identity membership is queried for every retained callable. Index once:
    // scanning function vectors here makes snapshot admission quadratic.
    let body_ids: BTreeSet<_> = package.functions.iter().map(|f| f.declaration).collect();
    let runtime_ids: BTreeSet<_> = package
        .runtime_function_identities
        .iter()
        .map(|f| f.declaration)
        .collect();
    let mut quote_ids = BTreeSet::new();
    let mut staged_quote_ids = BTreeSet::new();
    for template in &package.quote_templates {
        if template.phase != Phase::Runtime
            || !quote_ids.insert(template.declaration)
            || body_ids.contains(&template.declaration)
            || runtime_ids.contains(&template.declaration)
        {
            return Err(Error {
                source: template.body.source,
                message: "invalid or duplicate quote template identity",
            });
        }
        let parameters = parameters::validate_frame(template, true, declaration_fuel)
            .map_err(parameters::Failure::into_diagnostic)?;
        if parameters
            .iter()
            .any(|parameter| parameter.sort != Sort::Runtime)
        {
            staged_quote_ids.insert(template.declaration);
        }
    }
    let mut identity_ids = std::collections::BTreeSet::new();
    for identity in &package.runtime_function_identities {
        if !matches!(
            identity.signature.callable_signature(),
            Term::Function { .. }
        ) || identity.signature.is_comptime_callable()
            || !identity_ids.insert(identity.declaration)
            || body_ids.contains(&identity.declaration)
        {
            return Err(Error {
                source: SourceRange { start: 0, end: 0 },
                message: "invalid or duplicate runtime function identity",
            });
        }
    }
    crate::normal_forms::validate(package).map_err(|message| Error {
        source: SourceRange { start: 0, end: 0 },
        message,
    })?;
    for function in &package.functions {
        parameters::validate_frame(function, false, declaration_fuel)
            .map_err(parameters::Failure::into_diagnostic)?;
    }
    let functions: BTreeMap<_, _> = package
        .functions
        .iter()
        .chain(&package.quote_templates)
        .map(|f| {
            (
                f.declaration,
                (f.phase, !f.preconditions.is_empty(), f.is_unsafe),
            )
        })
        .chain(package.runtime_function_identities.iter().map(|f| {
            (
                f.declaration,
                (Phase::Runtime, false, f.signature.is_unsafe_callable()),
            )
        }))
        .collect();
    let parameter_frames: BTreeMap<_, _> = package
        .functions
        .iter()
        .chain(&package.quote_templates)
        .map(|function| (function.declaration, function.parameters.as_slice()))
        .collect();
    let mut storage_queries = storage_cache::StorageQueries::new(package);
    // Validation borrows method contracts. Only actual specialization needs
    // an owned summary, and that path has its own caller-supplied budget.
    let programs = || {
        package
            .functions
            .iter()
            .chain(&package.quote_templates)
            .map(|function| Ok(CallableProgram::function(function)))
            .chain(
                package
                    .traits
                    .iter()
                    .flat_map(|t| &t.methods)
                    .map(|method| {
                        CallableProgram::method(method).map_err(|_| Error {
                            source: SourceRange { start: 0, end: 0 },
                            message: "invalid trait program contract signature",
                        })
                    }),
            )
    };
    // Each work item carries its execution context. Staged inputs/defaults do
    // not change the owner's declaration phase or require cloned summary bodies.
    for function in programs() {
        let function = function?;
        let ct = function.phase != Phase::Runtime;
        if !ct {
            storage_queries
                .check_runtime(function.signature, &[], inspection_fuel)
                .map_err(|failure| Error {
                    source: function.source,
                    message: failure.message(),
                })?;
        }
        let inspection = parameters::InspectionFrame::new(function, inspection_fuel)
            .map_err(parameters::Failure::into_diagnostic)?;
        let caller_frames = &inspection.frames;
        let frozen = &inspection.frozen;
        let fail = |source, message| Error { source, message };
        let mut work = Worklist::new(function.source);
        work.extend(
            function
                .body
                .into_iter()
                .map(|body| Work::Expr(body, false, ct)),
            inspection_fuel,
        )?;
        if !function.preconditions.is_empty() || !function.postconditions.is_empty() {
            machine_owners.insert(function.declaration);
            if function.phase == Phase::NormalForm {
                return Err(fail(
                    function.source,
                    "normal-form program preconditions are not implemented",
                ));
            }
            for condition in function.preconditions.iter().chain(function.postconditions) {
                if !crate::program::checkable_predicate(condition, inspection_fuel).map_err(
                    |_| {
                        fail(
                            condition.source,
                            "program predicate inspection budget exhausted",
                        )
                    },
                )? {
                    return Err(fail(condition.source, "invalid program contract predicate"));
                }
                work.push(Work::Expr(condition, false, ct), inspection_fuel)?;
            }
        }
        work.extend(
            function.defaults.iter().map(|(index, value)| {
                let staged = matches!(inspection.signature,
                        crate::Term::Function {parameters,..}
                        if parameters.get(*index).is_some_and(|p|p.sort!=crate::Sort::Runtime));
                Work::Expr(value, false, ct || staged)
            }),
            inspection_fuel,
        )?;
        while let Some(node) = work.pop() {
            match node {
                Work::Pattern(pattern, ct) => match pattern {
                    Pattern::NominalType { fields, .. } => {
                        if !ct {
                            return Err(fail(
                                function.source,
                                "nominal input decomposition requires comptime",
                            ));
                        }
                        work.extend(
                            fields.iter().map(|field| Work::Pattern(field, ct)),
                            inspection_fuel,
                        )?;
                    }
                    Pattern::Then { first, value, next } => {
                        work.push(Work::Pattern(first, ct), inspection_fuel)?;
                        work.push(Work::Expr(value, false, ct), inspection_fuel)?;
                        work.push(Work::Pattern(next, ct), inspection_fuel)?;
                    }
                    Pattern::Guard { pattern, condition } => {
                        work.push(Work::Pattern(pattern, ct), inspection_fuel)?;
                        work.push(Work::Expr(condition, false, ct), inspection_fuel)?;
                    }
                    Pattern::Remap { pattern, bindings } => {
                        if bindings
                            .iter()
                            .any(|(_, destination)| frozen.contains(destination))
                        {
                            return Err(fail(function.source, "entry snapshots are immutable"));
                        }
                        work.push(Work::Pattern(pattern, ct), inspection_fuel)?;
                    }
                    Pattern::Observe {
                        inner: pattern,
                        binding,
                    }
                    | Pattern::Binding {
                        inner: pattern,
                        binding,
                    } => {
                        if frozen.contains(binding) {
                            return Err(fail(function.source, "entry snapshots are immutable"));
                        }
                        work.push(Work::Pattern(pattern, ct), inspection_fuel)?;
                    }
                    Pattern::OptionalPresent(inner) | Pattern::ErrorResult { inner, .. } => {
                        work.push(Work::Pattern(inner, ct), inspection_fuel)?
                    }
                    Pattern::Record { fields, .. }
                    | Pattern::AnonymousRecord { fields, .. }
                    | Pattern::Variant { fields, .. }
                    | Pattern::NormalForm {
                        arguments: fields, ..
                    }
                    | Pattern::TypeTuple(fields)
                    | Pattern::Tuple(fields)
                    | Pattern::Or(fields) => {
                        work.extend(
                            fields.iter().map(|pattern| Work::Pattern(pattern, ct)),
                            inspection_fuel,
                        )?;
                    }
                    Pattern::CodeEvidence { .. } => {
                        return Err(fail(function.source, "unresolved pattern-code evidence"));
                    }
                    Pattern::OptionalAbsent
                    | Pattern::Wild
                    | Pattern::Unit
                    | Pattern::Literal(_) => {}
                },
                Work::Expr(expr, _callee, ct) => {
                    let written = match &expr.kind {
                        K::Let { binding, .. } => Some(*binding),
                        K::Assign { place, .. }
                        | K::BitAssign { place, .. }
                        | K::BitUpdate { place, .. } => Some(place.root),
                        _ => None,
                    };
                    if written.is_some_and(|id| frozen.contains(&id)) {
                        return Err(fail(expr.source, "entry snapshots are immutable"));
                    }
                    let machine =
                        crate::program::machine_integer_type_in(expr, package.target).is_some();
                    // An admitted generic operation is checked after its owned
                    // type parameter is substituted. Do not confuse that open
                    // domain with the concrete comptime Integer domain, or let
                    // a result annotation hide a differently typed operand.
                    let owned_integer_domain = |ty: &Term| {
                        matches!(ty,
                        Term::Rigid { declaration, sort: crate::Sort::Type }
                        if function.type_parameters.contains(declaration))
                    };
                    let agrees = |ty: &Term, domain: &Term| {
                        ty == domain || *ty == Term::Primitive(crate::Primitive::Never)
                    };
                    let deferred_integer = match &expr.kind {
                        K::IntegerNegate(value) => {
                            owned_integer_domain(&value.ty) && expr.ty == value.ty
                        }
                        K::IntegerBinary { left, right, .. } => {
                            // A diverging operand supplies no numeric domain,
                            // and no operation executes after its exit.
                            let domain = if left.ty == Term::Primitive(crate::Primitive::Never) {
                                &right.ty
                            } else {
                                &left.ty
                            };
                            owned_integer_domain(domain)
                                && agrees(&left.ty, domain)
                                && agrees(&right.ty, domain)
                                && (expr.ty == *domain
                                    || (expr.ty == Term::Primitive(crate::Primitive::Never)
                                        && (left.ty == Term::Primitive(crate::Primitive::Never)
                                            || right.ty
                                                == Term::Primitive(crate::Primitive::Never))))
                        }
                        _ => false,
                    };
                    // Comptime positions are checked concretely by invocation;
                    // residual runtime positions need a static program proof.
                    if machine
                        || deferred_integer
                        || (!ct
                            && matches!(
                                expr.kind,
                                K::BitIndex(..)
                                    | K::BitWindow { .. }
                                    | K::BitUpdate { .. }
                                    | K::BitWritePointer { .. }
                            ))
                    {
                        machine_owners.insert(function.declaration);
                    }
                    if !ct {
                        storage_queries
                            .check_runtime(&expr.ty, caller_frames, inspection_fuel)
                            .map_err(|failure| fail(expr.source, failure.message()))?;
                        if !machine
                            && matches!(expr.kind, K::IntegerNegate(_) | K::IntegerBinary { .. })
                            && !deferred_integer
                        {
                            return Err(Error {
                                source: expr.source,
                                message: "comptime Integer operation requires evaluation and Splice before runtime",
                            });
                        }
                    }
                    let error = |message| fail(expr.source, message);
                    // Pointer type construction describes runtime storage and is
                    // legal comptime data. Executing a pointer operation is not:
                    // receiver updates use ComptimeReference and ordinary places.
                    if ct
                        && matches!(
                            expr.kind,
                            K::AddressOf(_)
                                | K::ReadPointer(_)
                                | K::WritePointer { .. }
                                | K::PointerField { .. }
                                | K::PointerAnonymousField { .. }
                                | K::BitWritePointer { .. }
                        )
                    {
                        return Err(error(
                            "pointer operations cannot execute at comptime; use value-reference self",
                        ));
                    }
                    if let Some(message) =
                        crate::program::pointer_operation_error_in(expr, package.target)
                    {
                        return Err(error(message));
                    }
                    if let K::UnsafeScope(body) = &expr.kind {
                        if body.ty != expr.ty {
                            return Err(error("unsafe scope changes its body type"));
                        }
                    }
                    if let K::AddressOf(place) = &expr.kind {
                        let mut place = place.as_ref();
                        while let K::AnonymousField { value, .. } | K::Field { value, .. } =
                            &place.kind
                        {
                            place = value;
                        }
                        if matches!(&place.kind,K::Local(id) if frozen.contains(id)) {
                            return Err(error("entry snapshots are immutable"));
                        }
                    }
                    match &expr.kind {
                        K::Intrinsic {
                            tag,
                            inputs,
                            binding,
                        } => {
                            let comptime = binding.as_ref().is_some_and(|b| b.is_comptime());
                            if ct && !comptime {
                                return Err(error("target intrinsic cannot execute at comptime"));
                            }
                            if !ct && comptime {
                                return Err(error("comptime intrinsic cannot execute at runtime"));
                            }
                            crate::intrinsics::validate_tag(package, *tag).map_err(error)?;
                            if let Some(binding) = binding {
                                binding
                                    .validate_template(*tag, inputs, &expr.ty)
                                    .map_err(error)?;
                                binding
                                    .validate_target_layout(package.target)
                                    .map_err(error)?;
                            }
                            work.extend(
                                inputs.iter().map(|e| Work::Expr(e, false, ct)),
                                inspection_fuel,
                            )?;
                        }
                        K::EffectHandle {
                            computation,
                            handlers,
                            ..
                        } => {
                            if ct {
                                return Err(error("comptime effect handlers are not implemented"));
                            }
                            work.push(Work::Expr(computation, false, ct), inspection_fuel)?;
                            for handler in handlers {
                                for arm in &handler.arms {
                                    work.push(Work::Pattern(&arm.pattern, ct), inspection_fuel)?;
                                    if let Some(g) = &arm.guard {
                                        work.push(Work::Expr(g, false, ct), inspection_fuel)?;
                                    }
                                    work.push(Work::Expr(&arm.body, false, ct), inspection_fuel)?;
                                }
                            }
                        }
                        K::EffectResume(body)
                        | K::EffectComputation { body, .. }
                        | K::EffectRun {
                            computation: body, ..
                        } => {
                            if ct {
                                return Err(error(
                                    "comptime effect computations are not implemented",
                                ));
                            }
                            work.push(Work::Expr(body, false, ct), inspection_fuel)?;
                        }
                        K::EffectRequest { arguments, .. } => {
                            if ct {
                                return Err(error("comptime effect requests are not implemented"));
                            }
                            work.extend(
                                arguments
                                    .iter()
                                    .map(|argument| Work::Expr(argument, false, ct)),
                                inspection_fuel,
                            )?;
                        }
                        K::Function(id)
                        | K::FunctionInstance {
                            declaration: id, ..
                        } => {
                            if quote_ids.contains(id) {
                                if !_callee {
                                    return Err(error(
                                        "quote template cannot escape as a runtime function value",
                                    ));
                                }
                                if !quote_ids.contains(&function.declaration)
                                    && !staged_quote_ids.contains(id)
                                {
                                    return Err(error(
                                        "quote application requires deterministic expansion before runtime execution",
                                    ));
                                }
                            }
                            if expr.ty.is_normal_form_callable() {
                                let descriptor = package
                                    .normal_forms
                                    .iter()
                                    .find(|head| head.declaration == *id)
                                    .ok_or_else(|| {
                                        error("normal-form callable declaration is missing")
                                    })?;
                                if !ct || !matches!(expr.kind, K::Function(_)) {
                                    return Err(error(
                                        "normal-form heads cannot be invoked as ordinary functions or stored at runtime",
                                    ));
                                }
                                if descriptor.callable_type().ok().as_ref() != Some(&expr.ty) {
                                    return Err(error(
                                        "normal-form callable type disagrees with its declaration",
                                    ));
                                }
                                continue;
                            }
                            let target = functions
                                .get(id)
                                .ok_or_else(|| error("function phase declaration is missing"))?;
                            if expr.ty.is_unsafe_callable() != target.2 {
                                return Err(error(
                                    "function value type does not preserve its declaration responsibility",
                                ));
                            }
                            if target.1 {
                                machine_owners.insert(function.declaration);
                                // Runtime staged identities are specialized back
                                // to their declaration before executable calls;
                                // specialization retains the declaration contract.
                                let retained_template = target.0 == Phase::Runtime
                                    && matches!(expr.ty.callable_signature(), Term::Function { parameters, .. }
                                        if parameters.iter().any(|p| p.sort != Sort::Runtime));
                                if !_callee && !retained_template {
                                    return Err(error(
                                        "function value would erase program preconditions; callable program contracts are not implemented",
                                    ));
                                }
                            }
                            if target.0 == Phase::NormalForm {
                                return Err(error(
                                    "normal-form heads cannot be invoked as ordinary functions",
                                ));
                            }
                            // Comptime may carry a runtime declaration identity.
                            // Only invocation requires that declaration's phase;
                            // its function type still records the runtime phase.
                            if ct != (target.0 == Phase::Comptime) && (_callee || !ct) {
                                return Err(error(if ct {
                                    "runtime function is unavailable during comptime execution"
                                } else {
                                    "comptime result invocation and runtime splicing are not yet implemented"
                                }));
                            }
                            if expr.ty.is_comptime_callable() != (target.0 == Phase::Comptime) {
                                return Err(error(
                                    "function value type does not preserve its declaration phase",
                                ));
                            }
                        }
                        K::AssociatedValue {
                            interface,
                            member,
                            subject,
                        } => {
                            let (contract, member) = trait_index
                                .associated(interface.declaration, *member, inspection_fuel)
                                .map_err(|failure| error(failure.message()))?;
                            if interface.arguments.len() != contract.type_parameters.len() {
                                return Err(error(
                                    "associated value interface argument count mismatch",
                                ));
                            }
                            let ty = member_types::associated(
                                contract,
                                member,
                                subject,
                                &interface.arguments,
                                caller_frames,
                                inspection_fuel,
                            )
                            .map_err(|failure| {
                                error(match failure {
                                    crate::type_computations::Error::Budget => {
                                        "associated value contract inspection budget exhausted"
                                    }
                                    _ => "associated value contract substitution failed",
                                })
                            })?;
                            if ty != expr.ty {
                                return Err(error(
                                    "associated value type does not preserve its contract",
                                ));
                            }
                        }
                        K::TraitMethod {
                            method_arguments,
                            interface,
                            interface_arguments,
                            member,
                            subject,
                        } => {
                            let (contract, method) = trait_index
                                .method(*interface, *member, inspection_fuel)
                                .map_err(|failure| error(failure.message()))?;
                            if contract.comptime_only && !ct {
                                return Err(error(
                                    "comptime-only trait method requires comptime context",
                                ));
                            }
                            if method.is_quote && !_callee {
                                return Err(error(
                                    "quote trait method cannot escape as a runtime function value",
                                ));
                            }
                            if !method.program.is_empty() {
                                machine_owners.insert(function.declaration);
                                let retained_template = !method.signature.is_comptime_callable()
                                    && matches!(method.signature.callable_signature(), Ok(Term::Function { parameters, .. })
                                        if parameters.iter().any(|p| p.sort != Sort::Runtime));
                                if !_callee
                                    && !method.program.preconditions.is_empty()
                                    && !retained_template
                                {
                                    return Err(error(
                                        "trait method value would erase program contracts; callable program contracts are not implemented",
                                    ));
                                }
                            }
                            if interface_arguments.len() != contract.type_parameters.len()
                                || method_arguments.len() != method.type_parameters.len()
                            {
                                return Err(error("trait method type argument count mismatch"));
                            }
                            let signature = member_types::method(
                                contract,
                                method,
                                subject,
                                interface_arguments,
                                method_arguments,
                                &parameter_domains,
                                caller_frames,
                                inspection_fuel,
                            )
                            .map_err(|failure| {
                                error(match failure {
                                    crate::type_computations::Error::Budget => {
                                        "trait method contract inspection budget exhausted"
                                    }
                                    _ => "trait method contract substitution failed",
                                })
                            })?;
                            if signature != expr.ty {
                                return Err(error(
                                    "trait method value type does not preserve its contract",
                                ));
                            }
                            if ct != expr.ty.is_comptime_callable() && (_callee || !ct) {
                                return Err(error(
                                    "trait method is unavailable in this execution phase",
                                ));
                            }
                        }
                        K::Call {
                            callee,
                            arguments,
                            parameter_bindings,
                        } => {
                            if callee.ty.is_normal_form_callable()
                                && (!ct
                                    || !parameter_bindings.is_empty()
                                    || !matches!(
                                        expr.ty,
                                        Term::Primitive(Primitive::Type | Primitive::Never)
                                    )
                                    || arguments.iter().any(|argument| {
                                        argument.is_default
                                            || argument.passing
                                                != crate::program::ArgumentPassing::Value
                                    }))
                            {
                                return Err(error(
                                    "invalid normal-form application phase or argument plan",
                                ));
                            }
                            for argument in arguments {
                                if argument.passing != crate::program::ArgumentPassing::Value
                                    && (!ct
                                        || !callee.ty.is_comptime_callable()
                                        || argument.is_default
                                        || argument.parameter != 0)
                                {
                                    return Err(error(
                                        "value-reference transport requires an explicit comptime receiver",
                                    ));
                                }
                            }
                            if ct != callee.ty.is_comptime_callable()
                                && !matches!(
                                    callee.kind,
                                    K::Function(_) | K::FunctionInstance { .. }
                                )
                            {
                                return Err(error(
                                    "callable type is unavailable in this execution phase",
                                ));
                            }
                            work.push(Work::Expr(callee, true, ct), inspection_fuel)?;
                            let Term::Function { parameters, .. } = callee.ty.callable_signature()
                            else {
                                return Err(error("call target has no callable signature"));
                            };
                            if arguments.len() != parameters.len() {
                                return Err(error(
                                    "call argument count does not match parameter frame",
                                ));
                            }
                            if !parameter_bindings.is_empty()
                                && (parameter_bindings.len() != parameters.len()
                                    || parameter_bindings.iter().collect::<BTreeSet<_>>().len()
                                        != parameter_bindings.len())
                            {
                                return Err(error("invalid call parameter binding frame"));
                            }
                            if !parameter_bindings.is_empty() {
                                let target = match &callee.kind {
                                    K::ComptimeFunctionInstance {
                                        declaration: id, ..
                                    }
                                    | K::Function(id)
                                    | K::FunctionInstance {
                                        declaration: id, ..
                                    } => parameter_frames.get(id),
                                    _ => None,
                                };
                                if target
                                    .is_some_and(|target| *target != parameter_bindings.as_slice())
                                {
                                    return Err(error(
                                        "call parameter bindings differ from target declaration",
                                    ));
                                }
                            }
                            let mut supplied = BTreeSet::new();
                            let mut available = BTreeSet::new();
                            let mut last_default = None;
                            for argument in arguments {
                                let parameter =
                                    parameters.get(argument.parameter).ok_or_else(|| {
                                        error("call argument parameter index out of range")
                                    })?;
                                if !supplied.insert(argument.parameter) {
                                    return Err(error("duplicate call argument parameter slot"));
                                }
                                if argument.is_default {
                                    if parameter_bindings.is_empty()
                                        || last_default
                                            .is_some_and(|last| last >= argument.parameter)
                                    {
                                        return Err(error(
                                            "invalid call default argument order or binding frame",
                                        ));
                                    }
                                    // Staged defaults can retain substituted caller
                                    // symbols until specialization. Runtime defaults
                                    // execute in the prepared parameter frame only.
                                    if parameter.sort == Sort::Runtime {
                                        let free = crate::free_bindings::collect(
                                            &argument.value,
                                            inspection_fuel,
                                        )
                                        .map_err(|reason| {
                                            error(match reason {
                                                crate::free_bindings::Error::Budget(_)
                                                | crate::free_bindings::Error::Depth(_) => {
                                                    "call default binding analysis budget exhausted"
                                                }
                                                crate::free_bindings::Error::Invalid(_) => {
                                                    "invalid call default binding scope"
                                                }
                                            })
                                        })?;
                                        if !free.is_subset(&available) {
                                            return Err(error(
                                                "call default references an unavailable parameter binding",
                                            ));
                                        }
                                    }
                                    last_default = Some(argument.parameter);
                                } else if last_default.is_some() {
                                    return Err(error(
                                        "explicit call argument follows a default argument",
                                    ));
                                }
                                if let Some(binding) = parameter_bindings.get(argument.parameter) {
                                    available.insert(*binding);
                                }
                                // Staged arguments execute before this runtime invocation.
                                // This changes execution phase only; the frontend separately
                                // proves that referenced inputs are available at that stage.
                                let input_ct = ct || parameter.sort != crate::Sort::Runtime;
                                work.push(
                                    Work::Expr(&argument.value, false, input_ct),
                                    inspection_fuel,
                                )?;
                            }
                        }
                        // Immutable type observations may remain symbolic until
                        // specialization. Their Type operands are compile-time
                        // descriptions, never runtime storage or runtime reads.
                        K::TypeEqual(left, right)
                            if matches!(left.kind, K::TypeValue(_))
                                && matches!(right.kind, K::TypeValue(_)) =>
                        {
                            work.push(Work::Expr(left, false, true), inspection_fuel)?;
                            work.push(Work::Expr(right, false, true), inspection_fuel)?;
                        }
                        K::Closure {
                            captures,
                            parameters,
                            body,
                        } => {
                            let Term::Function {
                                parameters: inputs, ..
                            } = expr.ty.callable_signature()
                            else {
                                return Err(error("closure has no callable signature"));
                            };
                            if inputs.len() != parameters.len() {
                                return Err(error("closure parameter frame arity mismatch"));
                            }
                            let mut identities = BTreeSet::new();
                            if parameters.iter().any(|id| !identities.insert(*id))
                                || captures
                                    .iter()
                                    .any(|capture| !identities.insert(capture.binding))
                            {
                                return Err(error(
                                    "duplicate closure parameter or capture identity",
                                ));
                            }
                            if let Term::Closure { environment, .. } = &expr.ty {
                                if environment.len() != captures.len() {
                                    return Err(error(
                                        "closure capture environment arity mismatch",
                                    ));
                                }
                            } else if matches!(
                                expr.ty,
                                Term::Function { .. }
                                    | Term::ComptimeFunction(_)
                                    | Term::NormalFormFunction(..)
                                    | Term::UnsafeFunction(_)
                            ) && !captures.is_empty()
                            {
                                return Err(error(
                                    "closure captures are missing from its callable type",
                                ));
                            }
                            if ct != expr.ty.is_comptime_callable() {
                                return Err(error(
                                    "closure type does not preserve its execution phase",
                                ));
                            }
                            work.push(Work::Expr(body, false, ct), inspection_fuel)?;
                            work.extend(
                                captures
                                    .iter()
                                    .filter_map(|c| c.value.as_deref())
                                    .map(|e| Work::Expr(e, false, ct)),
                                inspection_fuel,
                            )?;
                        }
                        K::NormalFormType {
                            head,
                            arguments,
                            plan,
                        } => {
                            work.extend(
                                plan.iter()
                                    .flat_map(|p| &p.preparations)
                                    .map(|p| Work::Expr(&p.expression, false, ct)),
                                inspection_fuel,
                            )?;
                            work.extend(
                                arguments
                                    .iter()
                                    .flat_map(|a| a.patterns())
                                    .map(|pattern| Work::Pattern(pattern, ct)),
                                inspection_fuel,
                            )?;
                            if !ct {
                                return Err(error(
                                    "normal-form type construction requires comptime",
                                ));
                            }
                            let metadata = package
                                .normal_forms
                                .iter()
                                .find(|h| h.declaration == *head)
                                .ok_or_else(|| {
                                    error("normal-form constructor metadata is missing")
                                })?;
                            if plan
                                .as_ref()
                                .is_some_and(|plan| !plan.valid(arguments.len()))
                            {
                                return Err(error("invalid normal-form input plan"));
                            }
                            if arguments.len() != metadata.parameters.len() {
                                return Err(error(
                                    "normal-form constructor argument count mismatch",
                                ));
                            }
                            work.extend(
                                arguments
                                    .iter()
                                    .flat_map(|a| a.expressions())
                                    .map(|e| Work::Expr(e, false, ct)),
                                inspection_fuel,
                            )?;
                        }
                        K::ClosureType {
                            environment,
                            signature,
                        } => {
                            work.extend(
                                environment.iter().map(|e| Work::Expr(e, false, ct)),
                                inspection_fuel,
                            )?;
                            work.push(Work::Expr(signature, false, ct), inspection_fuel)?;
                        }
                        K::CheckTypeAlias {
                            declaration,
                            arguments,
                            body,
                            preparations,
                            ..
                        } => {
                            if !ct {
                                return Err(error(
                                    "alias formation guard requires comptime execution",
                                ));
                            }
                            let contract = package
                                .alias_contracts
                                .iter()
                                .find(|contract| contract.declaration == *declaration)
                                .ok_or_else(|| error("missing alias formation contract"))?;
                            let diverges = arguments
                                .iter()
                                .any(crate::program::NormalFormArgument::is_diverging)
                                || preparations
                                    .iter()
                                    .any(|p| p.expression.ty == Term::Primitive(Primitive::Never));
                            let expected = if diverges {
                                Term::Primitive(Primitive::Never)
                            } else {
                                body.as_ref()
                                    .map_or(Term::Primitive(Primitive::Void), |body| {
                                        body.ty.clone()
                                    })
                            };
                            if arguments.len() != contract.parameters.len()
                                || expr.kind.nominal_local_bindings().is_none()
                                || body.as_ref().is_some_and(|body| {
                                    !matches!(
                                        body.ty,
                                        Term::Primitive(Primitive::Type | Primitive::Never)
                                    )
                                })
                                || expr.ty != expected
                                || arguments.iter().enumerate().any(|(index, argument)| {
                                    let domain = contract
                                        .parameter_types
                                        .get(index)
                                        .unwrap_or(&Term::Primitive(Primitive::Type));
                                    !contract
                                        .parameters
                                        .iter()
                                        .any(|id| domain.mentions_rigid(*id))
                                        && argument.expression().is_some_and(|argument| {
                                            &argument.ty != domain
                                                && argument.ty != Term::Primitive(Primitive::Never)
                                        })
                                })
                            {
                                return Err(error("invalid alias formation guard inputs"));
                            }
                            if let Some(body) = body {
                                work.push(Work::Expr(body, false, ct), inspection_fuel)?;
                            }
                            work.extend(
                                preparations
                                    .iter()
                                    .map(|p| Work::Expr(&p.expression, false, ct)),
                                inspection_fuel,
                            )?;
                            work.extend(
                                arguments
                                    .iter()
                                    .flat_map(|a| a.expressions())
                                    .map(|e| Work::Expr(e, false, ct)),
                                inspection_fuel,
                            )?;
                            work.extend(
                                arguments
                                    .iter()
                                    .flat_map(|a| a.patterns())
                                    .map(|p| Work::Pattern(p, ct)),
                                inspection_fuel,
                            )?;
                        }
                        K::NominalType {
                            arguments,
                            preparations,
                            ..
                        } => {
                            work.extend(
                                preparations
                                    .iter()
                                    .map(|p| Work::Expr(&p.expression, false, ct)),
                                inspection_fuel,
                            )?;
                            work.extend(
                                arguments
                                    .iter()
                                    .flat_map(|a| a.patterns())
                                    .map(|pattern| Work::Pattern(pattern, ct)),
                                inspection_fuel,
                            )?;
                            if !ct {
                                return Err(error(
                                    "nominal type construction requires comptime execution",
                                ));
                            }
                            work.extend(
                                arguments
                                    .iter()
                                    .flat_map(|a| a.expressions())
                                    .map(|e| Work::Expr(e, false, ct)),
                                inspection_fuel,
                            )?;
                        }
                        K::FunctionType {
                            parameters,
                            comptime_parameters,
                            ..
                        } => {
                            if parameters.len() != comptime_parameters.len() {
                                return Err(error(
                                    "function type parameter stages must match domains",
                                ));
                            }
                            work.extend(
                                parameters.iter().map(|e| Work::Expr(e, false, ct)),
                                inspection_fuel,
                            )?;
                        }
                        K::TypeList(fields)
                        | K::Tuple(fields)
                        | K::Record { fields, .. }
                        | K::Variant { fields, .. } => work.extend(
                            fields.iter().map(|e| Work::Expr(e, false, ct)),
                            inspection_fuel,
                        )?,
                        K::AnonymousRecord { fields } => {
                            match &expr.ty {
                                crate::Term::Record(types) if types.len() == fields.len() => {}
                                crate::Term::Primitive(crate::Primitive::Never)
                                    if fields.iter().any(|field| {
                                        field.ty == crate::Term::Primitive(crate::Primitive::Never)
                                    }) => {}
                                _ => {
                                    return Err(error(
                                        "anonymous record construction requires matching named field types",
                                    ));
                                }
                            }
                            work.extend(
                                fields.iter().map(|e| Work::Expr(e, false, ct)),
                                inspection_fuel,
                            )?;
                        }
                        K::Object { names, fields } => {
                            let diverges = fields.iter().any(|field| {
                                field.ty == crate::Term::Primitive(crate::Primitive::Never)
                            });
                            let expected = crate::Term::Primitive(if diverges {
                                crate::Primitive::Never
                            } else {
                                crate::Primitive::Object
                            });
                            if !ct || names.len() != fields.len() || expr.ty != expected {
                                return Err(error(
                                    "Object construction requires comptime evaluation and matching entries",
                                ));
                            }
                            work.extend(
                                fields.iter().map(|e| Work::Expr(e, false, ct)),
                                inspection_fuel,
                            )?;
                        }
                        K::AnonymousField { value, name, index } => {
                            match &value.ty {
                                crate::Term::Record(fields)
                                    if fields
                                        .get(*index)
                                        .is_some_and(|(field, _)| field == name) => {}
                                crate::Term::AssociatedType { .. }
                                | crate::Term::Rigid { .. }
                                | crate::Term::NormalForm { .. }
                                | crate::Term::Bound { .. }
                                | crate::Term::Hole { .. } => {}
                                _ => {
                                    return Err(error(
                                        "anonymous field projection requires a matching record label and index",
                                    ));
                                }
                            }
                            work.push(Work::Expr(value, false, ct), inspection_fuel)?;
                        }
                        K::Snapshot(value) => {
                            if value.ty != expr.ty {
                                return Err(error("entry snapshot must preserve its operand type"));
                            }
                            work.push(Work::Expr(value, false, ct), inspection_fuel)?;
                        }
                        K::RequirePredicate(condition)
                        | K::EnsurePredicate(condition)
                        | K::CheckPredicate(condition) => {
                            if matches!(expr.kind, K::RequirePredicate(_) | K::EnsurePredicate(_)) {
                                machine_owners.insert(function.declaration);
                            }
                            if expr.ty != Term::Primitive(Primitive::Void)
                                || !crate::program::checkable_predicate(condition, inspection_fuel)
                                    .map_err(|_| {
                                        fail(
                                            condition.source,
                                            "program predicate inspection budget exhausted",
                                        )
                                    })?
                            {
                                return Err(error(
                                    "program check requires a supported effect-free bool predicate and void result",
                                ));
                            }
                            work.push(Work::Expr(condition, false, ct), inspection_fuel)?;
                        }
                        K::ComptimeFunctionInstance {
                            declaration,
                            subject,
                        } => {
                            let member = trait_index
                                .retained_method(*declaration, inspection_fuel)
                                .map_err(|failure| error(failure.message()))?;
                            let trait_target;
                            let target = if let Some(member) = member {
                                trait_target = (
                                    if member.signature.is_comptime_callable() {
                                        Phase::Comptime
                                    } else {
                                        Phase::Runtime
                                    },
                                    !member.program.preconditions.is_empty(),
                                    member.is_unsafe,
                                );
                                &trait_target
                            } else {
                                functions
                                    .get(declaration)
                                    .ok_or_else(|| error("function phase declaration is missing"))?
                            };
                            if !ct || target.0 != Phase::Comptime || !expr.ty.is_comptime_callable()
                            {
                                return Err(error(
                                    "dynamic function instantiation requires comptime invocation",
                                ));
                            }
                            if expr.ty.is_unsafe_callable() != target.2 {
                                return Err(error(
                                    "function value type does not preserve its declaration responsibility",
                                ));
                            }
                            if target.1 {
                                machine_owners.insert(function.declaration);
                                if !_callee {
                                    return Err(error(
                                        "function value would erase program preconditions; callable program contracts are not implemented",
                                    ));
                                }
                            }
                            work.push(Work::Expr(subject, false, true), inspection_fuel)?;
                        }
                        K::BuildStruct(value)
                        | K::BuildNewtype(value)
                        | K::BuildEnum(value)
                        | K::EnumStructure { subject: value, .. }
                        | K::ObjectQuery { subject: value, .. }
                        | K::ObjectEdit { subject: value, .. }
                        | K::NewtypeRepresentation(value)
                        | K::StructFieldTypes(value)
                        | K::StructFieldNames(value) => {
                            if !ct {
                                return Err(fail(
                                    expr.source,
                                    "definition construction or struct reflection requires comptime invocation",
                                ));
                            }
                            work.push(Work::Expr(value, false, true), inspection_fuel)?;
                        }
                        K::AddressOf(value)
                        | K::ReadPointer(value)
                        | K::PointerAnonymousField { pointer: value, .. }
                        | K::PointerField { pointer: value, .. }
                        | K::Field { value, .. }
                        | K::Let { value, .. }
                        | K::Assign { value, .. }
                        | K::BitAssign { value, .. }
                        | K::UnsafeScope(value)
                        | K::Discard(value)
                        | K::BitSlice { value, .. }
                        | K::OptionalPresent(value)
                        | K::OptionalType(value)
                        | K::PointerType(value)
                        | K::ListLength(value)
                        | K::BitVecType(value)
                        | K::IntegerNegate(value)
                        | K::FloatNegate(value)
                        | K::BitConvert(value)
                        | K::ErrorConvert { value: value, .. }
                        | K::NewtypeConvert(value)
                        | K::TakeBits(value)
                        | K::SymbolName(value)
                        | K::EnumName(value)
                        | K::Not(value)
                        | K::Loop { body: value, .. } => {
                            work.push(Work::Expr(value, false, ct), inspection_fuel)?
                        }
                        K::ComptimeIntegerConvert(value) => {
                            if !ct
                                || !crate::program::comptime_integer_conversion_valid(
                                    &value.ty, &expr.ty,
                                )
                            {
                                return Err(fail(
                                    expr.source,
                                    "integer conversion requires comptime integer domains",
                                ));
                            }
                            work.push(Work::Expr(value, false, true), inspection_fuel)?;
                        }
                        K::IntegerRepresentation(value) => {
                            if !crate::program::integer_representation_valid(&value.ty, &expr.ty) {
                                return Err(fail(
                                    expr.source,
                                    "integer representation domain mismatch",
                                ));
                            }
                            work.push(Work::Expr(value, false, ct), inspection_fuel)?;
                        }
                        K::TextQuery { subject, query } => {
                            if !query.validate(&subject.ty, &expr.ty) {
                                return Err(fail(expr.source, "text query domain mismatch"));
                            }
                            work.push(Work::Expr(subject, false, ct), inspection_fuel)?;
                        }
                        K::WritePointer {
                            pointer: a,
                            value: b,
                        }
                        | K::FunctionResultType {
                            signature: a,
                            result: b,
                        }
                        | K::IntegerBinary {
                            left: a, right: b, ..
                        }
                        | K::FloatBinary {
                            left: a, right: b, ..
                        }
                        | K::ScalarLess(a, b)
                        | K::ScalarGreater(a, b)
                        | K::ScalarGreaterEqual(a, b)
                        | K::ScalarLessEqual(a, b)
                        | K::TemplateEqual(a, b)
                        | K::ScalarEqual(a, b)
                        | K::TypeEqual(a, b)
                        | K::ListConcat(a, b)
                        | K::TypeListIndex(a, b)
                        | K::BitEqual(a, b)
                        | K::BitConcat(a, b)
                        | K::And(a, b)
                        | K::Or(a, b) => {
                            work.push(Work::Expr(a, false, ct), inspection_fuel)?;
                            work.push(Work::Expr(b, false, ct), inspection_fuel)?;
                        }
                        K::BitWritePointer {
                            pointer,
                            selections,
                            value,
                        } => {
                            *inspection_fuel = inspection_fuel
                                .checked_sub(selections.len())
                                .ok_or_else(|| {
                                    error("bit selection validation budget exhausted")
                                })?;
                            let Term::Pointer(storage) = &pointer.ty else {
                                return Err(error("bit writeback requires a pointer"));
                            };
                            if let Some(message) = crate::program::bit_update_error_in(
                                storage,
                                selections,
                                &value.ty,
                                &expr.ty,
                                package.target,
                            ) {
                                return Err(error(message));
                            }
                            work.push(Work::Expr(pointer, false, ct), inspection_fuel)?;
                            for input in selections
                                .iter()
                                .flat_map(crate::program::BitSelection::expressions)
                            {
                                work.push(Work::Expr(input, false, ct), inspection_fuel)?;
                            }
                            work.push(Work::Expr(value, false, ct), inspection_fuel)?;
                        }
                        K::BitUpdate {
                            storage_type,
                            selections,
                            value,
                            ..
                        } => {
                            *inspection_fuel = inspection_fuel
                                .checked_sub(selections.len())
                                .ok_or_else(|| {
                                    error("bit selection validation budget exhausted")
                                })?;
                            if let Some(message) = crate::program::bit_update_error_in(
                                storage_type,
                                selections,
                                &value.ty,
                                &expr.ty,
                                package.target,
                            ) {
                                return Err(error(message));
                            }
                            for input in selections
                                .iter()
                                .flat_map(crate::program::BitSelection::expressions)
                            {
                                work.push(Work::Expr(input, false, ct), inspection_fuel)?;
                            }
                            work.push(Work::Expr(value, false, ct), inspection_fuel)?;
                        }
                        K::BitWindow { value, start, end } => {
                            let never = Term::Primitive(Primitive::Never);
                            if matches!(value.ty, Term::Primitive(p) if !matches!(p, Primitive::BitVec(_)))
                                || [start, end].iter().any(|e|matches!(e.ty, Term::Primitive(p)
                                    if !matches!(p, Primitive::Integer | Primitive::Never) && package.target.map_or_else(|| p.fixed_integer_layout(), |target| target.integer_layout(p)).is_none()))
                                || if start.ty == never || end.ty == never {expr.ty != never}
                                    else {!matches!(expr.ty, Term::Primitive(Primitive::BitVec(_)))} {
                                return Err(error("invalid dynamic BitVec slice operand or result type"));
                            }
                            work.push(Work::Expr(value, false, ct), inspection_fuel)?;
                            work.push(Work::Expr(start, false, ct), inspection_fuel)?;
                            work.push(Work::Expr(end, false, ct), inspection_fuel)?;
                        }
                        K::BitIndex(value, index) => {
                            let never = Term::Primitive(Primitive::Never);
                            // Abstract operand views require declaration type
                            // premises in the mandatory program verifier.
                            let bad_value = matches!(value.ty, Term::Primitive(p) if !matches!(p, Primitive::BitVec(_)));
                            let bad_index = matches!(index.ty, Term::Primitive(p)
                                if !matches!(p, Primitive::Integer | Primitive::Never) && package.target.map_or_else(|| p.fixed_integer_layout(), |target| target.integer_layout(p)).is_none());
                            if bad_value
                                || bad_index
                                || expr.ty
                                    != if index.ty == never {
                                        never
                                    } else {
                                        Term::Primitive(Primitive::BitVec(1))
                                    }
                            {
                                return Err(error(
                                    "invalid dynamic BitVec index operand or result type",
                                ));
                            }
                            work.push(Work::Expr(value, false, ct), inspection_fuel)?;
                            work.push(Work::Expr(index, false, ct), inspection_fuel)?;
                        }
                        K::Match { value, arms } => {
                            work.push(Work::Expr(value, false, ct), inspection_fuel)?;
                            for arm in arms {
                                work.push(Work::Pattern(&arm.pattern, ct), inspection_fuel)?;
                                if let Some(g) = &arm.guard {
                                    work.push(Work::Expr(g, false, ct), inspection_fuel)?;
                                }
                                work.push(Work::Expr(&arm.body, false, ct), inspection_fuel)?;
                            }
                        }
                        K::Block { statements, result }
                        | K::CallPreparation { statements, result } => {
                            if matches!(expr.kind, K::CallPreparation { .. }) {
                                let mut bindings = BTreeSet::new();
                                for statement in statements {
                                    let declared = match &statement.kind {
                                        K::Let { binding, .. } => vec![*binding],
                                        K::LetPattern {
                                            pattern:
                                                pattern @ (Pattern::Tuple(_)
                                                | Pattern::Binding { .. }
                                                | Pattern::Wild),
                                            value,
                                            guard: None,
                                            otherwise: None,
                                        } => crate::preparation::pattern_bindings(
                                            &package.normal_forms,
                                            pattern,
                                            &value.ty,
                                            inspection_fuel,
                                        )
                                        .map_err(|_| {
                                            fail(
                                                statement.source,
                                                "invalid or excessive preparation binding pattern",
                                            )
                                        })?
                                        .ok_or_else(|| {
                                            fail(
                                                statement.source,
                                                "spread preparation requires known tuple layouts",
                                            )
                                        })?
                                        .into_iter()
                                        .map(|(binding, _)| binding)
                                        .collect(),
                                        _ => {
                                            return Err(fail(
                                                statement.source,
                                                "call preparation requires binding statements",
                                            ));
                                        }
                                    };
                                    for binding in declared {
                                        if !bindings.insert(binding) {
                                            return Err(fail(
                                                statement.source,
                                                "call preparation bindings must be distinct",
                                            ));
                                        }
                                    }
                                }
                                if !result.as_ref().is_some_and(|value| {
                                    matches!(
                                        value.kind,
                                        K::Call { .. }
                                            | K::Variant { .. }
                                            | K::EffectRequest { .. }
                                            | K::Block { .. }
                                    )
                                }) {
                                    return Err(fail(
                                        expr.source,
                                        "call preparation requires a normalized call, variant or expanded call block, or an effect request",
                                    ));
                                }
                            }
                            let staged = crate::preparation::inputs_with_layouts(
                                &package.normal_forms,
                                expr,
                                inspection_fuel,
                            )
                            .map_err(|_| {
                                fail(
                                    expr.source,
                                    "invalid or excessive staged preparation dependencies",
                                )
                            })?
                            .map(|(indices, _)| indices)
                            .unwrap_or_default();
                            work.extend(
                                statements.iter().enumerate().map(|(index, e)| {
                                    Work::Expr(e, false, ct || staged.contains(&index))
                                }),
                                inspection_fuel,
                            )?;
                            if let Some(result) = result {
                                work.push(Work::Expr(result, false, ct), inspection_fuel)?;
                            }
                        }
                        K::LetPattern {
                            pattern,
                            value,
                            guard,
                            otherwise,
                        } => {
                            work.push(Work::Pattern(pattern, ct), inspection_fuel)?;
                            work.push(Work::Expr(value, false, ct), inspection_fuel)?;
                            if let Some(g) = guard {
                                work.push(Work::Expr(g, false, ct), inspection_fuel)?;
                            }
                            if let Some(e) = otherwise {
                                work.push(Work::Expr(e, false, ct), inspection_fuel)?;
                            }
                        }
                        K::If {
                            condition,
                            then_branch,
                            else_branch,
                        } => {
                            work.push(Work::Expr(condition, false, ct), inspection_fuel)?;
                            work.push(Work::Expr(then_branch, false, ct), inspection_fuel)?;
                            if let Some(e) = else_branch {
                                work.push(Work::Expr(e, false, ct), inspection_fuel)?;
                            }
                        }
                        K::Return(value) => {
                            if let Some(e) = value {
                                work.push(Work::Expr(e, false, ct), inspection_fuel)?;
                            }
                        }
                        K::BuildMetadata {
                            selector, entries, ..
                        } => {
                            if !ct {
                                return Err(fail(
                                    expr.source,
                                    "package metadata requires comptime invocation",
                                ));
                            }
                            if selector.ty != Term::Primitive(crate::Primitive::Str) {
                                return Err(fail(
                                    expr.source,
                                    "package metadata requires a str selector",
                                ));
                            }
                            let mut aliases = std::collections::BTreeSet::new();
                            for (alias, _, value) in entries {
                                if alias.is_empty() || !aliases.insert(alias) {
                                    return Err(fail(
                                        expr.source,
                                        "ambiguous package metadata aliases",
                                    ));
                                }
                                if let Some(value) = value {
                                    value.check_scope(&[]).map_err(|_| {
                                        fail(expr.source, "package metadata has invalid scope")
                                    })?;
                                }
                            }
                            work.push(Work::Expr(selector, false, ct), inspection_fuel)?;
                        }
                        K::StagedValue(value) => {
                            let Term::Function { parameters, .. } = inspection.signature else {
                                return Err(fail(
                                    expr.source,
                                    "staged value has no invocation frame",
                                ));
                            };
                            crate::comptime_values::validate_staged_in(
                                value,
                                &expr.ty,
                                package.target,
                                parameters,
                                inspection_fuel,
                            )
                            .map_err(|error| fail(expr.source, error.message()))?;
                        }
                        K::ComptimeValue(value) => {
                            crate::comptime_values::validate_in(
                                value,
                                &expr.ty,
                                package.target,
                                inspection_fuel,
                            )
                            .map_err(|error| fail(expr.source, error.message()))?;
                        }
                        K::TypeValue(_)
                        | K::Literal(_)
                        | K::Local(_)
                        | K::OptionalAbsent
                        | K::Unit
                        | K::Break(_)
                        | K::Continue(_) => {}
                    }
                }
            }
        }
        if let Term::Function { parameters, result } = inspection.signature {
            for domain in parameters
                .iter()
                .filter(|parameter| ct || parameter.sort != Sort::Runtime)
                .map(|parameter| &parameter.ty)
                .chain(ct.then_some(result.as_ref()))
            {
                storage_queries
                    .check_comptime(domain, caller_frames, inspection_fuel)
                    .map_err(|error| Error {
                        source: function.source,
                        message: error.message(),
                    })?;
            }
        }
        crate::call_safety::validate_program(function, inspection_fuel).map_err(
            |error| match error {
                crate::call_safety::Error::Budget(_) => Error {
                    source: function.source,
                    message: "call safety validation budget exhausted",
                },
                crate::call_safety::Error::Invalid { source, message } => Error {
                    source: source.unwrap_or(function.source),
                    message,
                },
            },
        )?;
    }
    Ok(machine_owners)
}

// Inspect stored fields, not nominal type arguments: a phantom Integer argument
// need not create an Integer value. Opaque normal-form heads are never unfolded.
/// Check a known runtime domain even when its enclosing ordinary signature
/// contains separate Any descriptions and cannot yet be represented as a Term.
/// Unknown descriptions are deliberately not accepted by this API.
pub fn validate_runtime_storage(
    ty: &Term,
    package: &Package,
    frames: &[Vec<crate::Sort>],
) -> Result<(), &'static str> {
    validate_runtime_storage_inner(ty, package, frames, None, &mut 100_000)
        .map_err(StorageError::message)
}

/// Validate storage against already instantiated nominal fields. The resolver
/// must return the exact layout or an explicit missing/invalid/budget error.
/// It never receives opaque normal-form heads for body reduction.
pub fn validate_runtime_storage_with_nominals(
    ty: &Term,
    package: &Package,
    frames: &[Vec<crate::Sort>],
    resolve: &dyn Fn(&Term, &mut usize) -> Result<Vec<Term>, StorageError>,
    fuel: &mut usize,
) -> Result<(), StorageError> {
    validate_runtime_storage_inner(ty, package, frames, Some(resolve), fuel)
}

/// Check a completed comptime payload domain against the caller's current
/// nominal catalog, including definitions published during invocation. Type
/// descriptions and runtime code signatures are not stored pointer payloads.
pub fn validate_comptime_storage_with_nominals(
    ty: &Term,
    package: &Package,
    frames: &[Vec<Sort>],
    resolve: &dyn Fn(&Term, &mut usize) -> Result<Vec<Term>, StorageError>,
    fuel: &mut usize,
) -> Result<(), StorageError> {
    comptime_storage::validate_resolved(ty, package, frames, Some(resolve), fuel)
}

/// Validate completed storage using artifact-local nominal and normal-form
/// layouts. The resolver supplies all stored fields, including inactive
/// variants; metadata-only type arguments do not themselves count as storage.
pub fn validate_comptime_storage_with_layouts(
    ty: &Term,
    frames: &[Vec<Sort>],
    resolve: &dyn Fn(&Term, &mut usize) -> Result<Vec<Term>, StorageError>,
    fuel: &mut usize,
) -> Result<(), StorageError> {
    comptime_storage::validate_layouts(ty, frames, resolve, fuel)
}

type NominalStorageResolver<'a> =
    Option<&'a dyn Fn(&Term, &mut usize) -> Result<Vec<Term>, StorageError>>;
fn validate_runtime_storage_inner(
    ty: &Term,
    package: &Package,
    frames: &[Vec<crate::Sort>],
    resolve: NominalStorageResolver<'_>,
    fuel: &mut usize,
) -> Result<(), StorageError> {
    crate::type_computations::charge_term(ty, fuel)?;
    if ty.sort() != crate::Sort::Type || ty.check_scope(frames).is_err() {
        return Err(StorageError::Invalid(
            "runtime storage requires a Type in the declared input scope",
        ));
    }
    match comptime_storage_resolved(
        ty,
        package,
        frames,
        &mut std::collections::HashSet::new(),
        0,
        resolve,
        fuel,
    ) {
        Ok(false) => Ok(()),
        Ok(true) => Err(StorageError::Invalid(
            "comptime-only value cannot inhabit runtime storage; Splice is required",
        )),
        Err(error) => Err(error),
    }
}

fn comptime_storage_resolved(
    ty: &Term,
    package: &Package,
    frames: &[Vec<crate::Sort>],
    seen: &mut std::collections::HashSet<(Term, Vec<Vec<crate::Sort>>)>,
    depth: usize,
    resolve: NominalStorageResolver<'_>,
    fuel: &mut usize,
) -> Result<bool, StorageError> {
    use crate::{Primitive as P, Term as T};
    crate::type_computations::charge_term(ty, fuel)?;
    if depth > 128 {
        return Err(StorageError::Budget);
    }
    // Closed recursive storage types do not depend on intervening function
    // frames. Canonicalize their cache scope so a recursive callback signature
    // reaches the existing cycle guard instead of growing frames indefinitely.
    let identity_frames = if ty.check_scope(&[]).is_ok() {
        vec![]
    } else {
        frames.to_vec()
    };
    if !seen.insert((ty.clone(), identity_frames)) {
        return Ok(false);
    }
    let children: Vec<T> = match ty {
        T::ClosureContextType | T::ClosureContextValue(_) => return Ok(true),
        // Generic declarations retain a scoped context parameter. Concrete
        // instantiation substitutes it and checks the actual capture storage.
        // Unscoped or computed contexts cannot establish a runtime layout.
        T::ContextualClosure { context, signature } => {
            let scoped = match context.as_ref() {
                T::Bound {
                    sort: crate::Sort::ComptimeValue,
                    ..
                } => context.check_scope(frames).is_ok(),
                T::Rigid {
                    declaration,
                    sort: crate::Sort::ComptimeValue,
                } => {
                    package
                        .declaration_parameter_domains_checked(fuel)?
                        .get(declaration)
                        == Some(&T::ClosureContextType)
                }
                _ => false,
            };
            if !scoped {
                return Err(StorageError::Invalid(
                    "closure storage context lacks scoped evidence",
                ));
            }
            vec![(**signature).clone()]
        }
        T::Primitive(P::Integer | P::Float | P::Type | P::Object)
        | T::ComptimeFunction(_)
        | T::NormalFormFunction(..) => {
            return Ok(true);
        }
        T::NormalForm { head, .. } => {
            match package
                .normal_forms
                .iter()
                .find(|definition| definition.declaration == *head)
            {
                Some(definition) if definition.role == crate::normal_forms::Role::TypeList => {
                    return Ok(true);
                }
                Some(definition) if definition.role == crate::normal_forms::Role::Tuple => {
                    // Inspect only the trusted tuple representation contract.
                    // An unresolved generic list remains deferred until concrete
                    // instantiation; ordinary factory bodies are never unfolded.
                    definition.tuple_elements(ty).unwrap_or_default().to_vec()
                }
                _ => vec![],
            }
        }
        T::ErrorResult { errors, result } => errors
            .iter()
            .cloned()
            .chain(std::iter::once((**result).clone()))
            .collect(),
        // Effect identities describe control requirements, not stored values.
        // The computation's returned payload still obeys the storage phase.
        T::Computation { result, .. } => vec![(**result).clone()],
        T::Tuple(items) => items.clone(),
        T::Record(fields) => fields.iter().map(|(_, ty)| ty.clone()).collect(),
        T::Optional(inner) | T::Pointer(inner) | T::UnsafeFunction(inner) => {
            vec![(**inner).clone()]
        }
        T::Function { parameters, result } => {
            let mut nested = frames.to_vec();
            nested.push(parameters.iter().map(|p| p.sort).collect());
            for child in parameters
                .iter()
                .filter(|p| p.sort == crate::Sort::Runtime)
                .map(|p| &p.ty)
                .chain(std::iter::once(result.as_ref()))
            {
                if comptime_storage_resolved(
                    child,
                    package,
                    &nested,
                    seen,
                    depth + 1,
                    resolve,
                    fuel,
                )? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        T::Closure {
            environment,
            signature,
        } => environment
            .iter()
            .cloned()
            .chain(std::iter::once((**signature).clone()))
            .collect(),
        T::Nominal {
            declaration,
            arguments,
        } => {
            if let Some(resolve) = resolve {
                resolve(ty, fuel)?
            } else if let Some(def) = package
                .structs
                .iter()
                .find(|d| d.declaration == *declaration)
            {
                def.instantiate_stored_fields(arguments, frames, fuel)?
            } else if let Some(def) = package.enums.iter().find(|d| d.declaration == *declaration) {
                def.instantiate_stored_fields(arguments, frames, fuel)?
            } else if let Some(def) = package
                .newtypes
                .iter()
                .find(|d| d.declaration == *declaration)
            {
                vec![def.representation_budgeted(arguments, frames, fuel)?]
            } else {
                vec![]
            }
        }
        _ => vec![],
    };
    for child in &children {
        if comptime_storage_resolved(child, package, frames, seen, depth + 1, resolve, fuel)? {
            return Ok(true);
        }
    }
    Ok(false)
}
