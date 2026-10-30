//! Execution-phase boundary for runtime and phase-bearing comptime callables.
//! This validates phase only, not resource legality, purity, or termination.
use crate::program::{Expr, ExprKind as K, FunctionPhase as Phase, Package, Pattern, SourceRange};
use crate::{Primitive, Sort, Term};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub source: SourceRange,
    pub message: &'static str,
}
enum Work<'a> {
    Expr(&'a Expr, bool, bool), // direct callee, comptime execution context
    Pattern(&'a Pattern, bool),
}
fn validate_parameter_frame(
    function: &crate::program::Function,
    quote: bool,
) -> Result<&[crate::Parameter], Error> {
    let messages = if quote {
        [
            "quote template parameter frame has no callable signature",
            "quote template parameter frame arity mismatch",
            "duplicate quote template parameter identity",
            "invalid quote template parameter default slot",
        ]
    } else {
        [
            "function parameter frame has no callable signature",
            "function parameter frame arity mismatch",
            "duplicate function parameter identity",
            "invalid function parameter default slot",
        ]
    };
    let fail = |index: usize| Error {
        source: function.body.source,
        message: messages[index],
    };
    let Term::Function { parameters, .. } = function.signature.callable_signature() else {
        return Err(fail(0));
    };
    if parameters.len() != function.parameters.len() {
        return Err(fail(1));
    }
    let mut bindings = BTreeSet::new();
    if function
        .parameters
        .iter()
        .any(|binding| !bindings.insert(*binding))
    {
        return Err(fail(2));
    }
    let mut defaults = BTreeSet::new();
    if function
        .defaults
        .iter()
        .any(|(slot, _)| *slot >= parameters.len() || !defaults.insert(*slot))
    {
        return Err(fail(3));
    }
    let positions: BTreeMap<_, _> = function
        .parameters
        .iter()
        .enumerate()
        .map(|(slot, binding)| (*binding, slot))
        .collect();
    let mut fuel = 100_000;
    for (slot, value) in &function.defaults {
        let free = crate::free_bindings::collect(value, &mut fuel).map_err(|reason| Error {
            source: value.source,
            message: if reason.contains("exhausted") {
                "function default binding analysis budget exhausted"
            } else {
                "invalid function default binding scope"
            },
        })?;
        if free.iter().any(|binding| {
            positions
                .get(binding)
                .is_none_or(|position| position >= slot)
        }) {
            return Err(Error {
                source: value.source,
                message: "function default references an unavailable parameter binding",
            });
        }
    }
    Ok(parameters)
}

pub fn validate(package: &Package) -> Result<(), Error> {
    machine_arithmetic_functions(package).map(|_| ())
}
/// Full phase traversal, also collecting owners whose bodies/defaults contain
/// machine arithmetic (including nested closures and patterns).
pub fn machine_arithmetic_functions(
    package: &Package,
) -> Result<std::collections::BTreeSet<crate::DeclarationId>, Error> {
    let mut machine_owners = std::collections::BTreeSet::new();
    let parameter_domains = package
        .declaration_parameter_domains(&mut 100_000)
        .map_err(|_| Error {
            source: SourceRange { start: 0, end: 0 },
            message: "invalid declaration parameter domains",
        })?;
    crate::type_aliases::validate(&package.alias_contracts).map_err(|message| Error {
        source: SourceRange { start: 0, end: 0 },
        message,
    })?;
    for definition in &package.structs {
        definition.parameter_binders().map_err(|_| Error {
            source: SourceRange { start: 0, end: 0 },
            message: "invalid struct parameter domains",
        })?;
    }
    for definition in &package.enums {
        definition.parameter_binders().map_err(|_| Error {
            source: SourceRange { start: 0, end: 0 },
            message: "invalid enum parameter domains",
        })?;
        definition
            .validate_encoding_in(package.target)
            .map_err(|message| Error {
                source: SourceRange { start: 0, end: 0 },
                message,
            })?;
    }
    let mut membership_fuel = 100_000usize;
    let mut membership = BTreeMap::new();
    for interface in &package.traits {
        let mut methods = BTreeMap::new();
        for member in &interface.methods {
            membership_fuel = membership_fuel
                .checked_sub(1 + member.name.len())
                .ok_or(Error {
                    source: SourceRange { start: 0, end: 0 },
                    message: "trait phase validation budget exhausted",
                })?;
            methods.insert(member.name.as_str(), member.phase);
        }
        membership.insert(interface.declaration, methods);
    }
    for definition in &package.implementations {
        if (!definition.phases.runtime && !definition.phases.comptime)
            || (definition.interface.is_none() && definition.phases.comptime)
        {
            return Err(Error {
                source: SourceRange { start: 0, end: 0 },
                message: "invalid implementation phase availability",
            });
        }
        definition.parameter_binders().map_err(|_| Error {
            source: SourceRange { start: 0, end: 0 },
            message: "invalid implementation parameter domains",
        })?;
        if let Some(members) = definition.interface.and_then(|id| membership.get(&id)) {
            for (name, _) in &definition.methods {
                membership_fuel = membership_fuel.checked_sub(1 + name.len()).ok_or(Error {
                    source: SourceRange { start: 0, end: 0 },
                    message: "trait phase validation budget exhausted",
                })?;
                if members
                    .get(name.as_str())
                    .is_some_and(|phase| !definition.phases.contains(*phase))
                {
                    return Err(Error {
                        source: SourceRange { start: 0, end: 0 },
                        message: "implementation method belongs to an unavailable trait phase",
                    });
                }
            }
        }
    }
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
        let parameters = validate_parameter_frame(template, true)?;
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
        validate_parameter_frame(function, false)?;
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
    let mut storage_cache = std::collections::HashMap::new();
    let mut runtime_storage = |ty: &crate::Term,
                               source,
                               frames: &[Vec<crate::Sort>]|
     -> Result<(), Error> {
        let result = *storage_cache
            .entry((ty.clone(), frames.to_vec()))
            .or_insert_with(|| {
                comptime_storage(
                    ty,
                    package,
                    frames,
                    &mut std::collections::HashSet::new(),
                    0,
                )
            });
        match result {
            Ok(false) => Ok(()),
            Ok(true) => Err(Error {
                source,
                message: "comptime-only value cannot inhabit runtime storage; Splice is required",
            }),
            Err(()) => Err(Error {
                source,
                message: "runtime storage phase could not be established within the type inspection limit",
            }),
        }
    };
    let interface_summaries = package
        .traits
        .iter()
        .flat_map(|t| &t.methods)
        .map(|method| {
            method.program_summary(&[], &[]).map_err(|_| Error {
                source: SourceRange { start: 0, end: 0 },
                message: "invalid trait program contract signature",
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Each work item carries its execution context. Staged inputs/defaults do
    // not change the owner's declaration phase or require cloned summary bodies.
    let mut preparation_fuel = 1_000_000usize;
    for function in package
        .functions
        .iter()
        .chain(&package.quote_templates)
        .chain(&interface_summaries)
    {
        let caller_frames = match function.signature.callable_signature() {
            Term::Function { parameters, .. } => vec![parameters.iter().map(|p| p.sort).collect()],
            _ => vec![],
        };
        let ct = function.phase != Phase::Runtime;
        if !ct {
            runtime_storage(&function.signature, function.body.source, &[])?;
        }
        let fail = |source, message| Error { source, message };

        if function
            .result_binding
            .is_some_and(|binding| function.parameters.contains(&binding))
        {
            return Err(fail(
                function.body.source,
                "result binding aliases an input parameter",
            ));
        }
        let mut frozen = BTreeSet::new();
        for snapshot in &function.entry_snapshots {
            if !function.parameters.contains(&snapshot.parameter)
                || function.parameters.contains(&snapshot.binding)
                || function.result_binding == Some(snapshot.binding)
                || !frozen.insert(snapshot.binding)
            {
                return Err(fail(function.body.source, "invalid entry snapshot binding"));
            }
        }
        let mut work = vec![Work::Expr(&function.body, false, ct)];
        if !function.preconditions.is_empty() || !function.postconditions.is_empty() {
            machine_owners.insert(function.declaration);
            if function.phase == Phase::NormalForm {
                return Err(fail(
                    function.body.source,
                    "normal-form program preconditions are not implemented",
                ));
            }
            for condition in function
                .preconditions
                .iter()
                .chain(&function.postconditions)
            {
                if !crate::program::checkable_predicate(condition) {
                    return Err(fail(condition.source, "invalid program contract predicate"));
                }
                work.push(Work::Expr(condition, false, ct));
            }
        }
        work.extend(function.defaults.iter().map(|(index, value)| {
            let staged = matches!(function.signature.callable_signature(),
                        crate::Term::Function {parameters,..}
                        if parameters.get(*index).is_some_and(|p|p.sort!=crate::Sort::Runtime));
            Work::Expr(value, false, ct || staged)
        }));
        while let Some(node) = work.pop() {
            match node {
                Work::Pattern(pattern, ct) => match pattern {
                    Pattern::NominalType { fields, .. } => {
                        if !ct {
                            return Err(fail(
                                function.body.source,
                                "nominal input decomposition requires comptime",
                            ));
                        }
                        work.extend(fields.iter().map(|field| Work::Pattern(field, ct)));
                    }
                    Pattern::Then { first, value, next } => {
                        work.push(Work::Pattern(first, ct));
                        work.push(Work::Expr(value, false, ct));
                        work.push(Work::Pattern(next, ct));
                    }
                    Pattern::Guard { pattern, condition } => {
                        work.push(Work::Pattern(pattern, ct));
                        work.push(Work::Expr(condition, false, ct));
                    }
                    Pattern::Remap { pattern, bindings } => {
                        if bindings
                            .iter()
                            .any(|(_, destination)| frozen.contains(destination))
                        {
                            return Err(fail(
                                function.body.source,
                                "entry snapshots are immutable",
                            ));
                        }
                        work.push(Work::Pattern(pattern, ct));
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
                            return Err(fail(
                                function.body.source,
                                "entry snapshots are immutable",
                            ));
                        }
                        work.push(Work::Pattern(pattern, ct));
                    }
                    Pattern::OptionalPresent(inner) | Pattern::ErrorResult { inner, .. } => {
                        work.push(Work::Pattern(inner, ct))
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
                        work.extend(fields.iter().map(|pattern| Work::Pattern(pattern, ct)));
                    }
                    Pattern::CodeEvidence { .. } => {
                        return Err(fail(
                            function.body.source,
                            "unresolved pattern-code evidence",
                        ));
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
                        runtime_storage(&expr.ty, expr.source, &caller_frames)?;
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
                            work.extend(inputs.iter().map(|e| Work::Expr(e, false, ct)));
                        }
                        K::EffectHandle {
                            computation,
                            handlers,
                            ..
                        } => {
                            if ct {
                                return Err(error("comptime effect handlers are not implemented"));
                            }
                            work.push(Work::Expr(computation, false, ct));
                            for handler in handlers {
                                for arm in &handler.arms {
                                    work.push(Work::Pattern(&arm.pattern, ct));
                                    if let Some(g) = &arm.guard {
                                        work.push(Work::Expr(g, false, ct));
                                    }
                                    work.push(Work::Expr(&arm.body, false, ct));
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
                            work.push(Work::Expr(body, false, ct));
                        }
                        K::EffectRequest { arguments, .. } => {
                            if ct {
                                return Err(error("comptime effect requests are not implemented"));
                            }
                            work.extend(
                                arguments
                                    .iter()
                                    .map(|argument| Work::Expr(argument, false, ct)),
                            );
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
                            let contract = package
                                .traits
                                .iter()
                                .find(|definition| definition.declaration == interface.declaration)
                                .ok_or_else(|| {
                                    error("associated value has no interface contract")
                                })?;
                            let member = contract
                                .associated
                                .iter()
                                .find(|definition| definition.declaration == *member)
                                .ok_or_else(|| error("associated value has no member contract"))?;
                            if interface.arguments.len() != contract.type_parameters.len() {
                                return Err(error(
                                    "associated value interface argument count mismatch",
                                ));
                            }
                            let arguments: Vec<_> = std::iter::once(subject.clone())
                                .chain(interface.arguments.iter().cloned())
                                .collect();
                            let ty = member
                                .ty
                                .instantiate_parameters(
                                    &contract.binders_with_self(&mut 100_000).map_err(|_| {
                                        error("invalid associated value parameter domains")
                                    })?,
                                    &arguments,
                                )
                                .map_err(|_| {
                                    error("associated value contract substitution failed")
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
                            let contract = package
                                .traits
                                .iter()
                                .find(|definition| definition.declaration == *interface)
                                .ok_or_else(|| error("trait method has no interface contract"))?;
                            let method = contract
                                .methods
                                .iter()
                                .find(|method| method.declaration == *member)
                                .ok_or_else(|| error("trait method has no member contract"))?;
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
                            let parameters: Vec<_> = contract
                                .parameters_with_self()
                                .into_iter()
                                .chain(method.type_parameters.iter().copied())
                                .collect();
                            let domains: Vec<_> = parameters
                                .iter()
                                .map(|id| {
                                    parameter_domains
                                        .get(id)
                                        .cloned()
                                        .unwrap_or(Term::Primitive(Primitive::Type))
                                })
                                .collect();
                            let arguments: Vec<_> = std::iter::once(subject.clone())
                                .chain(interface_arguments.iter().cloned())
                                .chain(method_arguments.iter().cloned())
                                .collect();
                            let substitution = crate::DeclarationSubstitution::new(
                                &parameters,
                                &domains,
                                &arguments,
                                &caller_frames,
                            )
                            .map_err(|_| error("trait method domain substitution failed"))?;
                            let signature = method
                                .signature
                                .try_map_types(|ty| substitution.instantiate(ty))
                                .map_err(|_| error("trait method contract substitution failed"))?;
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
                            work.push(Work::Expr(callee, true, ct));
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
                                            &argument.value, &mut preparation_fuel,
                                        ).map_err(|_| error("invalid call default binding scope or exhausted analysis budget"))?;
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
                                work.push(Work::Expr(&argument.value, false, input_ct));
                            }
                        }
                        // Immutable type observations may remain symbolic until
                        // specialization. Their Type operands are compile-time
                        // descriptions, never runtime storage or runtime reads.
                        K::TypeEqual(left, right)
                            if matches!(left.kind, K::TypeValue(_))
                                && matches!(right.kind, K::TypeValue(_)) =>
                        {
                            work.push(Work::Expr(left, false, true));
                            work.push(Work::Expr(right, false, true));
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
                            work.push(Work::Expr(body, false, ct));
                            work.extend(
                                captures
                                    .iter()
                                    .filter_map(|c| c.value.as_deref())
                                    .map(|e| Work::Expr(e, false, ct)),
                            );
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
                            );
                            work.extend(
                                arguments
                                    .iter()
                                    .flat_map(|a| a.patterns())
                                    .map(|pattern| Work::Pattern(pattern, ct)),
                            );
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
                            );
                        }
                        K::ClosureType {
                            environment,
                            signature,
                        } => {
                            work.extend(environment.iter().map(|e| Work::Expr(e, false, ct)));
                            work.push(Work::Expr(signature, false, ct));
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
                                work.push(Work::Expr(body, false, ct));
                            }
                            work.extend(
                                preparations
                                    .iter()
                                    .map(|p| Work::Expr(&p.expression, false, ct)),
                            );
                            work.extend(
                                arguments
                                    .iter()
                                    .flat_map(|a| a.expressions())
                                    .map(|e| Work::Expr(e, false, ct)),
                            );
                            work.extend(
                                arguments
                                    .iter()
                                    .flat_map(|a| a.patterns())
                                    .map(|p| Work::Pattern(p, ct)),
                            );
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
                            );
                            work.extend(
                                arguments
                                    .iter()
                                    .flat_map(|a| a.patterns())
                                    .map(|pattern| Work::Pattern(pattern, ct)),
                            );
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
                            );
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
                            work.extend(parameters.iter().map(|e| Work::Expr(e, false, ct)));
                        }
                        K::TypeList(fields)
                        | K::TupleType(fields)
                        | K::Tuple(fields)
                        | K::Record { fields, .. }
                        | K::Variant { fields, .. } => {
                            work.extend(fields.iter().map(|e| Work::Expr(e, false, ct)))
                        }
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
                            work.extend(fields.iter().map(|e| Work::Expr(e, false, ct)));
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
                            work.extend(fields.iter().map(|e| Work::Expr(e, false, ct)));
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
                            work.push(Work::Expr(value, false, ct));
                        }
                        K::Snapshot(value) => {
                            if value.ty != expr.ty {
                                return Err(error("entry snapshot must preserve its operand type"));
                            }
                            work.push(Work::Expr(value, false, ct));
                        }
                        K::RequirePredicate(condition)
                        | K::EnsurePredicate(condition)
                        | K::CheckPredicate(condition) => {
                            if matches!(expr.kind, K::RequirePredicate(_) | K::EnsurePredicate(_)) {
                                machine_owners.insert(function.declaration);
                            }
                            if expr.ty != Term::Primitive(Primitive::Void)
                                || !crate::program::checkable_predicate(condition)
                            {
                                return Err(error(
                                    "program check requires a supported effect-free bool predicate and void result",
                                ));
                            }
                            work.push(Work::Expr(condition, false, ct));
                        }
                        K::ComptimeFunctionInstance {
                            declaration,
                            subject,
                        } => {
                            let member = package
                                .traits
                                .iter()
                                .flat_map(|interface| &interface.methods)
                                .find(|member| member.declaration == *declaration);
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
                            work.push(Work::Expr(subject, false, true));
                        }
                        K::BuildStruct(value)
                        | K::BuildEnum(value)
                        | K::EnumStructure { subject: value, .. }
                        | K::ObjectQuery { subject: value, .. }
                        | K::ObjectEdit { subject: value, .. }
                        | K::StructFieldTypes(value)
                        | K::StructFieldNames(value) => {
                            if !ct {
                                return Err(fail(
                                    expr.source,
                                    "definition construction or struct reflection requires comptime invocation",
                                ));
                            }
                            work.push(Work::Expr(value, false, true));
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
                        | K::Loop { body: value, .. } => work.push(Work::Expr(value, false, ct)),
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
                            work.push(Work::Expr(value, false, true));
                        }
                        K::IntegerRepresentation(value) => {
                            if !crate::program::integer_representation_valid(&value.ty, &expr.ty) {
                                return Err(fail(
                                    expr.source,
                                    "integer representation domain mismatch",
                                ));
                            }
                            work.push(Work::Expr(value, false, ct));
                        }
                        K::TextQuery { subject, query } => {
                            if !query.validate(&subject.ty, &expr.ty) {
                                return Err(fail(expr.source, "text query domain mismatch"));
                            }
                            work.push(Work::Expr(subject, false, ct));
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
                            work.push(Work::Expr(a, false, ct));
                            work.push(Work::Expr(b, false, ct));
                        }
                        K::BitWritePointer {
                            pointer,
                            selections,
                            value,
                        } => {
                            preparation_fuel =
                                preparation_fuel.checked_sub(selections.len()).ok_or_else(
                                    || error("bit selection validation budget exhausted"),
                                )?;
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
                            work.push(Work::Expr(pointer, false, ct));
                            for input in selections
                                .iter()
                                .flat_map(crate::program::BitSelection::expressions)
                            {
                                work.push(Work::Expr(input, false, ct));
                            }
                            work.push(Work::Expr(value, false, ct));
                        }
                        K::BitUpdate {
                            storage_type,
                            selections,
                            value,
                            ..
                        } => {
                            preparation_fuel =
                                preparation_fuel.checked_sub(selections.len()).ok_or_else(
                                    || error("bit selection validation budget exhausted"),
                                )?;
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
                                work.push(Work::Expr(input, false, ct));
                            }
                            work.push(Work::Expr(value, false, ct));
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
                            work.push(Work::Expr(value, false, ct));
                            work.push(Work::Expr(start, false, ct));
                            work.push(Work::Expr(end, false, ct));
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
                            work.push(Work::Expr(value, false, ct));
                            work.push(Work::Expr(index, false, ct));
                        }
                        K::Match { value, arms } => {
                            work.push(Work::Expr(value, false, ct));
                            for arm in arms {
                                work.push(Work::Pattern(&arm.pattern, ct));
                                if let Some(g) = &arm.guard {
                                    work.push(Work::Expr(g, false, ct));
                                }
                                work.push(Work::Expr(&arm.body, false, ct));
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
                                            &mut preparation_fuel,
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
                                &mut preparation_fuel,
                            )
                            .map_err(|_| {
                                fail(
                                    expr.source,
                                    "invalid or excessive staged preparation dependencies",
                                )
                            })?
                            .map(|(indices, _)| indices)
                            .unwrap_or_default();
                            work.extend(statements.iter().enumerate().map(|(index, e)| {
                                Work::Expr(e, false, ct || staged.contains(&index))
                            }));
                            if let Some(result) = result {
                                work.push(Work::Expr(result, false, ct));
                            }
                        }
                        K::LetPattern {
                            pattern,
                            value,
                            guard,
                            otherwise,
                        } => {
                            work.push(Work::Pattern(pattern, ct));
                            work.push(Work::Expr(value, false, ct));
                            if let Some(g) = guard {
                                work.push(Work::Expr(g, false, ct));
                            }
                            if let Some(e) = otherwise {
                                work.push(Work::Expr(e, false, ct));
                            }
                        }
                        K::If {
                            condition,
                            then_branch,
                            else_branch,
                        } => {
                            work.push(Work::Expr(condition, false, ct));
                            work.push(Work::Expr(then_branch, false, ct));
                            if let Some(e) = else_branch {
                                work.push(Work::Expr(e, false, ct));
                            }
                        }
                        K::Return(value) => {
                            if let Some(e) = value {
                                work.push(Work::Expr(e, false, ct));
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
                            work.push(Work::Expr(selector, false, ct));
                        }
                        K::ComptimeValue(value) => {
                            crate::comptime_values::validate_in(
                                value,
                                &expr.ty,
                                package.target,
                                &mut 100_000,
                            )
                            .map_err(|error| fail(expr.source, error.message()))?;
                        }
                        K::TypeValue(_)
                        | K::TypeListValue(_)
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
    }
    for function in package
        .functions
        .iter()
        .chain(&package.quote_templates)
        .chain(&interface_summaries)
    {
        crate::call_safety::validate(function, &mut 100_000).map_err(|_| Error {source: function.body.source, message: "unsafe function call requires an unsafe boundary or declaration telescope is invalid"})?;
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
}

/// Validate storage against already instantiated nominal fields. The resolver
/// must return the exact layout for the requested type, or None if unavailable.
/// It never receives opaque normal-form heads for body reduction.
pub fn validate_runtime_storage_with_nominals(
    ty: &Term,
    package: &Package,
    frames: &[Vec<crate::Sort>],
    resolve: &dyn Fn(&Term, &mut usize) -> Option<Vec<Term>>,
    fuel: &mut usize,
) -> Result<(), &'static str> {
    validate_runtime_storage_inner(ty, package, frames, Some(resolve), fuel)
}

type NominalStorageResolver<'a> = Option<&'a dyn Fn(&Term, &mut usize) -> Option<Vec<Term>>>;
fn validate_runtime_storage_inner(
    ty: &Term,
    package: &Package,
    frames: &[Vec<crate::Sort>],
    resolve: NominalStorageResolver<'_>,
    fuel: &mut usize,
) -> Result<(), &'static str> {
    crate::type_computations::charge_term(ty, fuel)
        .map_err(|_| "runtime storage inspection budget exhausted")?;
    if ty.sort() != crate::Sort::Type || ty.check_scope(frames).is_err() {
        return Err("runtime storage requires a Type in the declared input scope");
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
        Ok(true) => Err("comptime-only value cannot inhabit runtime storage; Splice is required"),
        Err(()) => {
            Err("runtime storage phase could not be established within the type inspection limit")
        }
    }
}

fn comptime_storage(
    ty: &crate::Term,
    package: &Package,
    frames: &[Vec<crate::Sort>],
    seen: &mut std::collections::HashSet<(crate::Term, Vec<Vec<crate::Sort>>)>,
    depth: usize,
) -> Result<bool, ()> {
    comptime_storage_resolved(ty, package, frames, seen, depth, None, &mut 100_000)
}

fn comptime_storage_resolved(
    ty: &Term,
    package: &Package,
    frames: &[Vec<crate::Sort>],
    seen: &mut std::collections::HashSet<(Term, Vec<Vec<crate::Sort>>)>,
    depth: usize,
    resolve: NominalStorageResolver<'_>,
    fuel: &mut usize,
) -> Result<bool, ()> {
    use crate::{Primitive as P, Term as T};
    crate::type_computations::charge_term(ty, fuel).map_err(|_| ())?;
    if depth > 128 {
        return Err(());
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
                        .declaration_parameter_domains(&mut 100_000)
                        .map_err(|_| ())?
                        .get(declaration)
                        == Some(&T::ClosureContextType)
                }
                _ => false,
            };
            if !scoped {
                return Err(());
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
                resolve(ty, fuel).ok_or(())?
            } else if let Some(def) = package
                .structs
                .iter()
                .find(|d| d.declaration == *declaration)
            {
                def.instantiate_fields_in_scope(arguments, frames)
                    .map_err(|_| ())?
                    .into_iter()
                    .map(|(_, ty)| ty)
                    .collect()
            } else if let Some(def) = package.enums.iter().find(|d| d.declaration == *declaration) {
                def.instantiate_variants_in_scope(arguments, frames)
                    .map_err(|_| ())?
                    .into_iter()
                    .flat_map(|v| v.fields)
                    .collect()
            } else if let Some(def) = package
                .newtypes
                .iter()
                .find(|d| d.declaration == *declaration)
            {
                vec![
                    def.representation_budgeted(arguments, frames, &mut 100_000)
                        .map_err(|_| ())?,
                ]
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
