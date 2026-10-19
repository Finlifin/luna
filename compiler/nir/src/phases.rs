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
pub fn validate(package: &Package) -> Result<(), Error> {
    machine_arithmetic_functions(package).map(|_| ())
}
/// Full phase traversal, also collecting owners whose bodies/defaults contain
/// machine arithmetic (including nested closures and patterns).
pub fn machine_arithmetic_functions(
    package: &Package,
) -> Result<std::collections::BTreeSet<crate::DeclarationId>, Error> {
    let mut machine_owners = std::collections::BTreeSet::new();
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
        definition.validate_encoding().map_err(|message| Error {
            source: SourceRange { start: 0, end: 0 },
            message,
        })?;
    }
    for definition in &package.implementations {
        definition.parameter_binders().map_err(|_| Error {
            source: SourceRange { start: 0, end: 0 },
            message: "invalid implementation parameter domains",
        })?;
    }
    let mut quote_ids = BTreeSet::new();
    let mut staged_quote_ids = BTreeSet::new();
    for template in &package.quote_templates {
        if template.phase != Phase::Runtime
            || !quote_ids.insert(template.declaration)
            || package
                .functions
                .iter()
                .any(|function| function.declaration == template.declaration)
            || package
                .runtime_function_identities
                .iter()
                .any(|function| function.declaration == template.declaration)
        {
            return Err(Error {
                source: template.body.source,
                message: "invalid or duplicate quote template identity",
            });
        }
        if matches!(template.signature.callable_signature(),Term::Function {parameters,..}
            if parameters.iter().any(|parameter|parameter.sort!=Sort::Runtime))
        {
            staged_quote_ids.insert(template.declaration);
        }
    }
    let mut identity_ids = std::collections::BTreeSet::new();
    for identity in &package.runtime_function_identities {
        if !matches!(identity.signature, Term::Function { .. })
            || !identity_ids.insert(identity.declaration)
            || package
                .functions
                .iter()
                .any(|f| f.declaration == identity.declaration)
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
    let functions: BTreeMap<_, _> = package
        .functions
        .iter()
        .chain(&package.quote_templates)
        .map(|f| (f.declaration, (f.phase, !f.preconditions.is_empty())))
        .chain(
            package
                .runtime_function_identities
                .iter()
                .map(|f| (f.declaration, (Phase::Runtime, false))),
        )
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
                    Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => {}
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
                    let machine = crate::program::machine_integer_type(expr).is_some();
                    // Comptime positions are checked concretely by invocation;
                    // residual runtime positions need a static program proof.
                    if machine
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
                        {
                            return Err(Error {
                                source: expr.source,
                                message: "comptime Integer operation requires evaluation and Splice before runtime",
                            });
                        }
                    }
                    let error = |message| fail(expr.source, message);
                    if let Some(message) = crate::program::pointer_operation_error(expr) {
                        return Err(error(message));
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
                            let target = functions
                                .get(id)
                                .ok_or_else(|| error("function phase declaration is missing"))?;
                            if target.1 {
                                machine_owners.insert(function.declaration);
                                if !_callee {
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
                            if matches!(expr.ty, crate::Term::ComptimeFunction(_))
                                != (target.0 == Phase::Comptime)
                            {
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
                                .instantiate_types(&contract.parameters_with_self(), &arguments)
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
                                if !_callee && !method.program.preconditions.is_empty() {
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
                                .binders_with_self(&mut 100_000)
                                .map_err(|_| error("invalid trait parameter domains"))?
                                .into_iter()
                                .chain(method.type_parameters.iter().map(|id| {
                                    crate::DeclarationParameter {
                                        declaration: *id,
                                        sort: crate::Sort::Type,
                                    }
                                }))
                                .collect();
                            let arguments: Vec<_> = std::iter::once(subject.clone())
                                .chain(interface_arguments.iter().cloned())
                                .chain(method_arguments.iter().cloned())
                                .collect();
                            let signature = method
                                .signature
                                .instantiate_parameters_in_scope(
                                    &parameters,
                                    &arguments,
                                    &caller_frames,
                                )
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
                            callee, arguments, ..
                        } => {
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
                            for argument in arguments {
                                let parameter =
                                    parameters.get(argument.parameter).ok_or_else(|| {
                                        error("call argument parameter index out of range")
                                    })?;
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
                        K::Closure { captures, body, .. } => {
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
                            if arguments.len() != contract.parameters.len()
                                || expr.ty != Term::Primitive(Primitive::Void)
                                || arguments
                                    .iter()
                                    .any(|argument| argument.ty != Term::Primitive(Primitive::Type))
                            {
                                return Err(error("invalid alias formation guard inputs"));
                            }
                            work.extend(
                                arguments
                                    .iter()
                                    .map(|argument| Work::Expr(argument, false, ct)),
                            );
                        }
                        K::NominalType {
                            arguments: fields, ..
                        }
                        | K::FunctionType {
                            parameters: fields, ..
                        }
                        | K::TypeList(fields)
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
                        K::AddressOf(value)
                        | K::ReadPointer(value)
                        | K::PointerAnonymousField { pointer: value, .. }
                        | K::PointerField { pointer: value, .. }
                        | K::Field { value, .. }
                        | K::Let { value, .. }
                        | K::Assign { value, .. }
                        | K::BitAssign { value, .. }
                        | K::Discard(value)
                        | K::BitSlice { value, .. }
                        | K::PointerType(value)
                        | K::BitVecType(value)
                        | K::IntegerNegate(value)
                        | K::FloatNegate(value)
                        | K::BitConvert(value)
                        | K::NewtypeConvert(value)
                        | K::TakeBits(value)
                        | K::SymbolName(value)
                        | K::EnumName(value)
                        | K::Not(value)
                        | K::Loop { body: value, .. } => work.push(Work::Expr(value, false, ct)),
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
                        | K::ScalarEqual(a, b)
                        | K::TypeEqual(a, b)
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
                            if let Some(message) = crate::program::bit_update_error(
                                storage, selections, &value.ty, &expr.ty,
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
                            if let Some(message) = crate::program::bit_update_error(
                                storage_type,
                                selections,
                                &value.ty,
                                &expr.ty,
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
                                    if !matches!(p, Primitive::Integer | Primitive::Never) && p.fixed_integer_layout().is_none()))
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
                                if !matches!(p, Primitive::Integer | Primitive::Never) && p.fixed_integer_layout().is_none());
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
                                        K::Call { .. } | K::Variant { .. } | K::Block { .. }
                                    )
                                }) {
                                    return Err(fail(
                                        expr.source,
                                        "call preparation requires a normalized call, variant or expanded call block",
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
                        K::TypeValue(_)
                        | K::TypeListValue(_)
                        | K::Literal(_)
                        | K::Local(_)
                        | K::Unit
                        | K::Break(_)
                        | K::Continue(_) => {}
                    }
                }
            }
        }
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
    if ty.sort() != crate::Sort::Type || ty.check_scope(frames).is_err() {
        return Err("runtime storage requires a Type in the declared input scope");
    }
    match comptime_storage(
        ty,
        package,
        frames,
        &mut std::collections::HashSet::new(),
        0,
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
    use crate::{Primitive as P, Term as T};
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
        T::Primitive(P::Integer | P::Float | P::Type) | T::ComptimeFunction(_) => return Ok(true),
        T::Tuple(items) => items.clone(),
        T::Record(fields) => fields.iter().map(|(_, ty)| ty.clone()).collect(),
        T::Pointer(inner) => vec![(**inner).clone()],
        T::Function { parameters, result } => {
            let mut nested = frames.to_vec();
            nested.push(parameters.iter().map(|p| p.sort).collect());
            for child in parameters
                .iter()
                .filter(|p| p.sort == crate::Sort::Runtime)
                .map(|p| &p.ty)
                .chain(std::iter::once(result.as_ref()))
            {
                if comptime_storage(child, package, &nested, seen, depth + 1)? {
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
            if let Some(def) = package
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
        if comptime_storage(child, package, frames, seen, depth + 1)? {
            return Ok(true);
        }
    }
    Ok(false)
}
