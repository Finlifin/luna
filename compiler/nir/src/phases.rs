//! Execution-phase boundary for runtime and phase-bearing comptime callables.
//! This validates phase only, not resource legality, purity, or termination.
use crate::program::{Expr, ExprKind as K, FunctionPhase as Phase, Package, Pattern, SourceRange};
use crate::{Primitive, Sort, Term, contracts::CallableProgram};
use std::collections::{BTreeMap, BTreeSet};

mod comptime_storage;
pub(crate) mod contextual;
mod declarations;
mod inspection;

pub use inspection::{Inspection, PhaseCheckedPackage};

#[cfg(test)]
mod inspection_tests;

use crate::member_types;

mod parameters;
mod runtime_storage;
mod storage_cache;
mod storage_domains;
mod storage_error;
mod trait_index;
mod worklist;

pub use storage_error::StorageError;
use worklist::{Work, Worklist};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub source: SourceRange,
    pub message: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Budget,
    Depth,
    Invalid,
}

impl Error {
    pub(crate) fn invalid(source: SourceRange, message: &'static str) -> Self {
        Self {
            kind: ErrorKind::Invalid,
            source,
            message,
        }
    }

    pub(crate) fn budget(source: SourceRange, message: &'static str) -> Self {
        Self {
            kind: ErrorKind::Budget,
            source,
            message,
        }
    }

    pub(crate) fn depth(source: SourceRange, message: &'static str) -> Self {
        Self {
            kind: ErrorKind::Depth,
            source,
            message,
        }
    }

    fn call_safety(source: SourceRange, failure: crate::call_safety::Error) -> Self {
        match failure {
            crate::call_safety::Error::Budget(_) => {
                Self::budget(source, "call safety validation budget exhausted")
            }
            crate::call_safety::Error::Invalid {
                source: at,
                message,
            } => Self::invalid(at.unwrap_or(source), message),
        }
    }

    fn storage(source: SourceRange, failure: StorageError) -> Self {
        match failure {
            StorageError::Budget => Self::budget(source, failure.message()),
            StorageError::Invalid(message) => Self::invalid(source, message),
        }
    }

    fn member(source: SourceRange, failure: trait_index::Error) -> Self {
        match failure {
            trait_index::Error::Budget => Self::budget(source, failure.message()),
            trait_index::Error::Invalid(message) => Self::invalid(source, message),
        }
    }

    fn captured(source: SourceRange, failure: crate::comptime_values::Error) -> Self {
        match failure {
            crate::comptime_values::Error::Budget => Self::budget(source, failure.message()),
            crate::comptime_values::Error::Invalid(message) => Self::invalid(source, message),
        }
    }

    fn preparation(source: SourceRange, failure: crate::preparation::Error) -> Self {
        match failure {
            crate::preparation::Error::Budget => Self::budget(source, failure.message()),
            crate::preparation::Error::Invalid(message) => Self::invalid(source, message),
        }
    }
}

/// Machine operations found during phase traversal. Associated programs are
/// identified by the implementation and the interface member together.
#[derive(Clone, Debug)]
pub struct ArithmeticObligations {
    programs: Option<std::sync::Arc<crate::associated_initializers::MemberPrograms>>,
    domains: std::sync::Arc<BTreeMap<crate::DeclarationId, Term>>,
    functions: BTreeSet<crate::DeclarationId>,
    initializers: BTreeSet<(crate::DeclarationId, crate::DeclarationId)>,
}

impl ArithmeticObligations {
    pub fn programs_for<'a>(
        &'a self,
        row: &'a crate::program::Implementation,
    ) -> impl Iterator<
        Item = (
            &'a crate::DeclarationId,
            &'a crate::associated_initializers::AssociatedInitializer,
        ),
    > {
        crate::associated_initializers::programs_for(row, self.programs.as_deref())
    }

    pub fn parameter_domains(&self) -> &BTreeMap<crate::DeclarationId, Term> {
        &self.domains
    }

    pub fn functions(&self) -> &BTreeSet<crate::DeclarationId> {
        &self.functions
    }

    pub fn initializers(&self) -> &BTreeSet<(crate::DeclarationId, crate::DeclarationId)> {
        &self.initializers
    }
}

impl std::fmt::Display for ArithmeticObligations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(machine-arithmetic")?;
        for function in &self.functions {
            write!(f, " {function}")?;
        }
        for (implementation, member) in &self.initializers {
            write!(f, " (associated {implementation} {member})")?;
        }
        f.write_str(")")
    }
}

pub fn validate(package: &Package) -> Result<(), Error> {
    machine_arithmetic_obligations(package).map(|_| ())
}

/// Full phase traversal, also collecting owners whose bodies/defaults contain
/// machine arithmetic (including nested closures and patterns).
pub fn machine_arithmetic_functions(
    package: &Package,
) -> Result<std::collections::BTreeSet<crate::DeclarationId>, Error> {
    Ok(machine_arithmetic_obligations(package)?.functions)
}

pub fn machine_arithmetic_obligations(package: &Package) -> Result<ArithmeticObligations, Error> {
    arithmetic_obligations_with_budgets(package, &mut 1_000_000, &mut 1_000_000)
}

fn arithmetic_obligations_with_budgets(
    package: &Package,
    declaration_fuel: &mut usize,
    inspection_fuel: &mut usize,
) -> Result<ArithmeticObligations, Error> {
    let declarations = declarations::Declarations::new(package, declaration_fuel)?;
    PhaseContext::new(package, &declarations).check_package(inspection_fuel, None)
}

/// Inspect the dependency package and a lexical region in the same declaration
/// world. Only registered functions can be referenced from the region. Report
/// its arithmetic obligations separately even if its owner is also a dependency.
pub fn inspect_definition_region(
    package: &Package,
    region: &crate::program::DefinitionRegion,
) -> Result<(BTreeSet<crate::DeclarationId>, bool), Error> {
    inspect_definition_region_with_declarations(package, &BTreeMap::new(), region)
}

/// Inspect imported interfaces and their references without registering a
/// callable body. This view reports function owners and the lexical region.
pub fn inspect_definition_region_with_declarations(
    package: &Package,
    imports: &crate::callable_source::RegionDeclarations,
    region: &crate::program::DefinitionRegion,
) -> Result<(BTreeSet<crate::DeclarationId>, bool), Error> {
    let (owners, region) = inspect_definition_region_obligations(package, imports, region)?;
    Ok((owners.functions, region))
}

pub fn inspect_definition_region_obligations(
    package: &Package,
    imports: &crate::callable_source::RegionDeclarations,
    region: &crate::program::DefinitionRegion,
) -> Result<(ArithmeticObligations, bool), Error> {
    let mut declaration_fuel = 1_000_000;
    let mut inspection_fuel = 1_000_000;
    let declarations = declarations::Declarations::new(package, &mut declaration_fuel)?;
    let program = CallableProgram::definition_region(region);
    crate::callable_source::validate_frame(program, false, &mut declaration_fuel)?;
    let mut context = PhaseContext::new(package, &declarations);
    context
        .callables
        .import_declarations(imports, &mut declaration_fuel)?;
    let owners = context.check_package(&mut inspection_fuel, None)?;
    let arithmetic = context.inspect(program, &mut inspection_fuel)?;
    // Check the use in its own phase first. For a legal reference, also
    // inspect everything exported by the supplying declaration.
    for declaration in imports.values() {
        context.inspect(
            CallableProgram::declaration(declaration),
            &mut inspection_fuel,
        )?;
    }
    Ok((owners, arithmetic))
}

/// Check a closed invocation body against an already checked source world.
/// This checks its phase and unsafe-call boundary, not typing or resource safety.
/// The body has no declaration identity and is never registered in the package.
pub fn validate_comptime_expression(
    package: &Package,
    body: &Expr,
    fuel: &mut usize,
) -> Result<(), Error> {
    Inspection::new(package).check(body, fuel)
}

struct PhaseContext<'a> {
    package: &'a Package,
    declarations: &'a declarations::Declarations,
    callables: declarations::Interfaces<'a>,
    storage_queries: storage_cache::StorageQueries<'a>,
}

impl<'a> PhaseContext<'a> {
    fn new(package: &'a Package, declarations: &'a declarations::Declarations) -> Self {
        Self {
            package,
            declarations,
            callables: declarations::Interfaces::new(declarations.callables.bind(package)),
            storage_queries: storage_cache::StorageQueries::with_declarations(
                package,
                declarations,
            ),
        }
    }

    fn check_package(
        &mut self,
        fuel: &mut usize,
        programs: Option<std::sync::Arc<crate::associated_initializers::MemberPrograms>>,
    ) -> Result<ArithmeticObligations, Error> {
        let mut machine_owners = ArithmeticObligations {
            programs: programs.clone(),
            domains: self.declarations.parameter_domains.clone(),
            functions: BTreeSet::new(),
            initializers: BTreeSet::new(),
        };
        // Method contracts remain borrowed; inspection grants no specialization
        // or execution authority and never persists a body-validation receipt.
        for function in self
            .package
            .functions
            .iter()
            .chain(&self.package.quote_templates)
        {
            if self.inspect(CallableProgram::function(function), fuel)? {
                machine_owners.functions.insert(function.declaration);
            }
            if !function.runtime_deliveries.is_empty() {
                if function.phase != Phase::Runtime {
                    return Err(Error::invalid(
                        function.source,
                        "runtime deliveries require a runtime function",
                    ));
                }
                *fuel = fuel
                    .checked_sub(function.runtime_deliveries.len())
                    .ok_or_else(|| {
                        Error::budget(
                            function.source,
                            "runtime delivery inspection budget exhausted",
                        )
                    })?;
                for (id, expr) in &function.runtime_deliveries {
                    if !self.package.runtime_domains.contains_key(id) {
                        return Err(Error::invalid(
                            expr.source,
                            "function runtime delivery lacks a runtime domain",
                        ));
                    }
                    let mut delivery_program = CallableProgram::framed_body(
                        expr,
                        &function.signature,
                        Phase::Runtime,
                        &function.parameters,
                        &function.type_parameters,
                    );
                    delivery_program.parts.env_parameters = &function.env_parameters;
                    delivery_program.parts.is_unsafe = function.is_unsafe;
                    self.inspect(delivery_program, fuel)?;
                }
            }
        }
        for method in self
            .package
            .traits
            .iter()
            .flat_map(|interface| &interface.methods)
        {
            use crate::code_interfaces::MethodSignature::*;
            match &method.signature {
                Callable(ty) => {
                    if self.inspect(CallableProgram::ordinary_method(method, ty), fuel)? {
                        machine_owners.functions.insert(method.declaration);
                    }
                    continue;
                }
                Generator(sig) => {
                    contextual::inspect_contract(&mut self.storage_queries, method, sig, fuel)
                }
                PatternGenerator(sig) => {
                    contextual::inspect_contract(&mut self.storage_queries, method, sig, fuel)
                }
                DataConsumer(sig) => {
                    contextual::inspect_contract(&mut self.storage_queries, method, sig, fuel)
                }
            }?;
            let parts = crate::contracts::ProgramParts::method(method);
            let frame = parameters::InspectionFrame::for_parts(parts, None, fuel)
                .map_err(parameters::Failure::into_diagnostic)?;
            if self.inspect_parts(parts, frame, fuel)? {
                machine_owners.functions.insert(method.declaration);
            }
        }
        *fuel = fuel
            .checked_sub(self.package.implementations.len())
            .ok_or_else(|| {
                Error::budget(
                    SourceRange { start: 0, end: 0 },
                    "associated initializer inspection budget exhausted",
                )
            })?;
        for row in &self.package.implementations {
            for (member, initializer) in
                crate::associated_initializers::programs_for(row, programs.as_deref())
            {
                let expr = initializer.expression();
                if self.inspect_comptime_body(expr, &row.type_parameters, fuel)? {
                    machine_owners
                        .initializers
                        .insert((row.declaration, *member));
                }
            }
        }
        if !self.package.runtime_domains.is_empty() {
            *fuel = fuel
                .checked_sub(self.package.runtime_domains.len())
                .ok_or_else(|| {
                    Error::budget(
                        SourceRange { start: 0, end: 0 },
                        "runtime domain validation budget exhausted",
                    )
                })?;
            let mut scope = self
                .storage_queries
                .scope(&[], fuel)
                .map_err(|failure| Error::storage(SourceRange { start: 0, end: 0 }, failure))?;
            for domain in self.package.runtime_domains.values() {
                let Term::Lift(inner) = domain else {
                    return Err(Error::invalid(
                        SourceRange { start: 0, end: 0 },
                        "runtime dependency domain must be a lift type",
                    ));
                };
                scope
                    .check_runtime(inner, fuel)
                    .map_err(|failure| Error::storage(SourceRange { start: 0, end: 0 }, failure))?;
            }
        }
        if !self.package.global_runtime_deliveries.is_empty() {
            *fuel = fuel
                .checked_sub(self.package.global_runtime_deliveries.len())
                .ok_or_else(|| {
                    Error::budget(
                        SourceRange { start: 0, end: 0 },
                        "global runtime delivery inspection budget exhausted",
                    )
                })?;
            for (id, expr) in &self.package.global_runtime_deliveries {
                if !self.package.runtime_domains.contains_key(id) {
                    return Err(Error::invalid(
                        expr.source,
                        "global runtime delivery lacks a runtime domain",
                    ));
                }
                let free = crate::free_bindings::collect(expr, fuel).map_err(|_| {
                    Error::budget(
                        expr.source,
                        "global runtime delivery capture inspection budget exhausted",
                    )
                })?;
                if !free.is_empty() {
                    return Err(Error::invalid(
                        expr.source,
                        "global runtime delivery cannot capture local bindings",
                    ));
                }
                self.storage_queries
                    .scope(&[], fuel)
                    .and_then(|mut scope| scope.check_runtime(&expr.ty, fuel))
                    .map_err(|failure| Error::storage(expr.source, failure))?;
                let signature = Term::Function {
                    parameters: vec![],
                    result: Box::new(expr.ty.clone()),
                };
                self.inspect(
                    CallableProgram::framed_body(expr, &signature, Phase::Runtime, &[], &[]),
                    fuel,
                )?;
            }
        }
        Ok(machine_owners)
    }

    fn inspect_comptime_body(
        &mut self,
        body: &Expr,
        parameters: &[crate::DeclarationId],
        fuel: &mut usize,
    ) -> Result<bool, Error> {
        crate::type_computations::charge_term(&body.ty, fuel)
            .map_err(|_| Error::budget(body.source, "expression signature budget exhausted"))?;
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(body.ty.clone()),
        };
        self.inspect(
            CallableProgram::comptime_body(body, &signature, parameters),
            fuel,
        )
    }

    fn inspect(
        &mut self,
        function: CallableProgram<'_>,
        inspection_fuel: &mut usize,
    ) -> Result<bool, Error> {
        if function.parts.phase == Phase::Runtime {
            self.storage_queries
                .scope(&[], inspection_fuel)
                .and_then(|mut scope| scope.check_runtime(function.signature, inspection_fuel))
                .map_err(|failure| Error::storage(function.parts.source, failure))?;
        }
        let inspection = parameters::InspectionFrame::new(function, inspection_fuel)
            .map_err(parameters::Failure::into_diagnostic)?;
        let arithmetic = self.inspect_parts(function.parts, inspection, inspection_fuel)?;
        crate::call_safety::validate_signature(function.signature)
            .map_err(|failure| Error::call_safety(function.parts.source, failure))?;
        Ok(arithmetic)
    }

    fn inspect_parts(
        &mut self,
        function: crate::contracts::ProgramParts<'_>,
        inspection: parameters::InspectionFrame<'_>,
        inspection_fuel: &mut usize,
    ) -> Result<bool, Error> {
        let Self {
            package,
            declarations,
            callables,
            storage_queries,
        } = self;
        let declarations::Declarations {
            aliases,
            parameter_domains,
            trait_index,
            ..
        } = *declarations;
        let mut arithmetic = false;
        let ct = function.phase != Phase::Runtime;
        let caller_frames = &inspection.frames;
        let mut storage_scope = storage_queries
            .scope(caller_frames, inspection_fuel)
            .map_err(|failure| Error::storage(function.source, failure))?;
        let frozen = &inspection.frozen;
        let fail = Error::invalid;
        if !function.env_parameters.is_empty() {
            if ct {
                return Err(fail(
                    function.source,
                    "runtime environment parameters require a runtime callable",
                ));
            }
            let mut bindings: BTreeSet<crate::program::LocalId> =
                function.parameters.iter().copied().collect();
            let mut declarations = BTreeSet::new();
            for env in function.env_parameters {
                if !bindings.insert(env.binding) {
                    return Err(fail(
                        function.source,
                        "duplicate runtime environment parameter binding",
                    ));
                }
                if !declarations.insert(env.declaration) {
                    return Err(fail(
                        function.source,
                        "duplicate runtime environment parameter declaration",
                    ));
                }
                storage_scope
                    .check_runtime(&env.ty, inspection_fuel)
                    .map_err(|error| Error::storage(function.source, error))?;
            }
        }
        let mut work = Worklist::new(function.source, function.is_unsafe);
        let mut safety_error = None;
        work.extend(
            function
                .body
                .into_iter()
                .map(|body| Work::Expr(body, false, ct)),
            inspection_fuel,
        )?;
        if !function.preconditions.is_empty() || !function.postconditions.is_empty() {
            arithmetic = true;
            if function.phase == Phase::NormalForm {
                return Err(fail(
                    function.source,
                    "normal-form program preconditions are not implemented",
                ));
            }
            for condition in function.preconditions.iter().chain(function.postconditions) {
                if !crate::program::checkable_predicate(condition, inspection_fuel).map_err(
                    |_| {
                        Error::budget(
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
                        Some(crate::Term::Function {parameters,..})
                        if parameters.get(*index).is_some_and(|p|p.sort!=crate::Sort::Runtime));
                Work::Expr(value, false, ct || staged)
            }),
            inspection_fuel,
        )?;
        while let Some(node) = work.pop() {
            match node {
                Work::Argument(input, ct) => work.argument_children(input, ct, inspection_fuel)?,
                Work::Pattern(pattern, ct, source) => match pattern {
                    Pattern::NominalType { fields, .. } => {
                        if !ct {
                            return Err(fail(
                                source,
                                "nominal input decomposition requires comptime",
                            ));
                        }
                        work.extend(
                            fields.iter().map(|field| Work::Pattern(field, ct, source)),
                            inspection_fuel,
                        )?;
                    }
                    Pattern::Then { first, value, next } => {
                        work.push(Work::Pattern(first, ct, source), inspection_fuel)?;
                        work.push(Work::Expr(value, false, ct), inspection_fuel)?;
                        work.push(Work::Pattern(next, ct, source), inspection_fuel)?;
                    }
                    Pattern::Guard { pattern, condition } => {
                        work.push(Work::Pattern(pattern, ct, source), inspection_fuel)?;
                        work.push(Work::Expr(condition, false, ct), inspection_fuel)?;
                    }
                    Pattern::Remap { pattern, bindings } => {
                        if bindings
                            .iter()
                            .any(|(_, destination)| frozen.contains(destination))
                        {
                            return Err(fail(source, "entry snapshots are immutable"));
                        }
                        work.push(Work::Pattern(pattern, ct, source), inspection_fuel)?;
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
                            return Err(fail(source, "entry snapshots are immutable"));
                        }
                        work.push(Work::Pattern(pattern, ct, source), inspection_fuel)?;
                    }
                    Pattern::OptionalPresent(inner) | Pattern::ErrorResult { inner, .. } => {
                        work.push(Work::Pattern(inner, ct, source), inspection_fuel)?
                    }
                    Pattern::AnonymousRecord { names, fields } => {
                        crate::visit::validate_record_pattern(names, fields.len(), inspection_fuel)
                            .map_err(|e| Error::call_safety(source, e.into()))?;
                        work.extend(
                            fields.iter().map(|p| Work::Pattern(p, ct, source)),
                            inspection_fuel,
                        )?;
                    }
                    Pattern::Record { fields, .. }
                    | Pattern::Variant { fields, .. }
                    | Pattern::NormalForm {
                        arguments: fields, ..
                    }
                    | Pattern::TypeTuple(fields)
                    | Pattern::Tuple(fields)
                    | Pattern::Or(fields) => {
                        work.extend(
                            fields
                                .iter()
                                .map(|pattern| Work::Pattern(pattern, ct, source)),
                            inspection_fuel,
                        )?;
                    }
                    Pattern::CodeEvidence { .. } => {
                        return Err(fail(source, "unresolved pattern-code evidence"));
                    }
                    Pattern::OptionalAbsent
                    | Pattern::Wild
                    | Pattern::Unit
                    | Pattern::Literal(_) => {}
                },
                Work::Expr(expr, _callee, ct) => {
                    if safety_error.is_none() {
                        safety_error =
                            crate::call_safety::validate_operation(expr, work.permits_unsafe).err();
                    }
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
                        arithmetic = true;
                    }
                    if !ct {
                        storage_scope
                            .check_runtime(&expr.ty, inspection_fuel)
                            .map_err(|failure| Error::storage(expr.source, failure))?;
                        if !machine
                            && matches!(expr.kind, K::IntegerNegate(_) | K::IntegerBinary { .. })
                            && !deferred_integer
                        {
                            return Err(Error {
                                kind: ErrorKind::Invalid,
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
                                | K::ConstAddress { .. }
                                | K::StaticAddress(_)
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
                    if let Some(message) = crate::program::pointer_operation_error_with_world(
                        &crate::PackageLayoutWorld(package),
                        expr,
                        inspection_fuel,
                    ) {
                        return Err(error(message));
                    }
                    if let K::UnsafeScope(body) = &expr.kind {
                        work.permits_unsafe = true;
                        if body.ty != expr.ty {
                            return Err(error("unsafe scope changes its body type"));
                        }
                    }
                    if let K::ConstAddress { value, .. } = &expr.kind {
                        let free = crate::free_bindings::collect(value, inspection_fuel).map_err(
                            |_| {
                                Error::budget(
                                    value.source,
                                    "const-address capture inspection budget exhausted",
                                )
                            },
                        )?;
                        if !free.is_empty() {
                            return Err(error(
                                "const-address value cannot depend on local bindings",
                            ));
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
                    if let Some((arguments, schedule)) = expr.kind.declaration_inputs() {
                        let checked = schedule.check(arguments.len(), |n| {
                            *inspection_fuel = inspection_fuel.checked_sub(n).ok_or_else(|| {
                                Error::budget(
                                    expr.source,
                                    "input schedule inspection budget exhausted",
                                )
                            })?;
                            Ok::<_, Error>(())
                        })?;
                        if checked.is_none() {
                            return Err(error("invalid declaration input plan"));
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
                            Worklist::charge(inspection_fuel, handlers.len(), expr.source)?;
                            for handler in handlers {
                                for arm in &handler.arms {
                                    work.push(
                                        Work::Pattern(&arm.pattern, ct, expr.source),
                                        inspection_fuel,
                                    )?;
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
                            if callables.source.quote(id).is_some() {
                                if !_callee {
                                    return Err(error(
                                        "quote template cannot escape as a runtime function value",
                                    ));
                                }
                                if !function.declaration.is_some_and(|declaration| {
                                    callables.source.quote(&declaration).is_some()
                                }) && !callables.source.staged_quote(id)
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
                                let signature = normal_form_callable_type_in(
                                    descriptor,
                                    package,
                                    inspection_fuel,
                                )
                                    .map_err(|failure| match failure {
                                        crate::type_computations::Error::Budget => Error::budget(
                                            expr.source,
                                            "normal-form callable inspection budget exhausted",
                                        ),
                                        _ => error("normal-form callable declaration has an invalid signature"),
                                    })?;
                                if signature != expr.ty {
                                    return Err(error(
                                        "normal-form callable type disagrees with its declaration",
                                    ));
                                }
                                continue;
                            }
                            let target = callables
                                .phase(id)
                                .ok_or_else(|| error("function phase declaration is missing"))?;
                            if expr.ty.is_unsafe_callable() != target.2 {
                                return Err(error(
                                    "function value type does not preserve its declaration responsibility",
                                ));
                            }
                            if target.1 {
                                arithmetic = true;
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
                                .associated(
                                    package,
                                    interface.declaration,
                                    *member,
                                    inspection_fuel,
                                )
                                .map_err(|failure| Error::member(expr.source, failure))?;
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
                            .map_err(|failure| match failure {
                                crate::type_computations::Error::Budget => Error::budget(
                                    expr.source,
                                    "associated value contract inspection budget exhausted",
                                ),
                                _ => error("associated value contract substitution failed"),
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
                                .method(package, *interface, *member, inspection_fuel)
                                .map_err(|failure| Error::member(expr.source, failure))?;
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
                                arithmetic = true;
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
                                |id, _| Ok(parameter_domains.get(&id)),
                                caller_frames,
                                inspection_fuel,
                            )
                            .map_err(|failure| match failure {
                                crate::type_computations::Error::Budget => Error::budget(
                                    expr.source,
                                    "trait method contract inspection budget exhausted",
                                ),
                                _ => error("trait method contract substitution failed"),
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
                        K::TraitObjectType { arguments, .. } => {
                            work.extend(
                                arguments.iter().map(|a| Work::Expr(a, false, ct)),
                                inspection_fuel,
                            )?;
                        }
                        K::DynCast(cast) => {
                            if ct && !cast.env_arguments.is_empty() {
                                return Err(error(
                                    "comptime dyn cast cannot capture runtime environment arguments",
                                ));
                            }
                            work.push(Work::Expr(&cast.value, false, ct), inspection_fuel)?;
                            for env_arg in &cast.env_arguments {
                                work.push(Work::Expr(env_arg, false, ct), inspection_fuel)?;
                            }
                        }
                        K::DynCall(call) => {
                            work.push(Work::Expr(&call.receiver, false, ct), inspection_fuel)?;
                            for argument in &call.arguments {
                                work.push(Work::Expr(&argument.value, false, ct), inspection_fuel)?;
                            }
                        }
                        K::Call {
                            callee,
                            arguments,
                            parameter_bindings,
                            env_arguments,
                        } => {
                            if ct && !env_arguments.is_empty() {
                                return Err(error(
                                    "comptime call cannot pass runtime environment arguments",
                                ));
                            }
                            if callee.ty.is_normal_form_callable() {
                                return Err(error(
                                    "ordinary call cannot invoke a normal-form head",
                                ));
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
                            Worklist::charge(
                                inspection_fuel,
                                parameter_bindings.len(),
                                expr.source,
                            )?;
                            if !parameter_bindings.is_empty()
                                && (parameter_bindings.len() != parameters.len()
                                    || parameter_bindings.iter().collect::<BTreeSet<_>>().len()
                                        != parameter_bindings.len())
                            {
                                return Err(error("invalid call parameter binding frame"));
                            }
                            if !parameter_bindings.is_empty() {
                                let target = match &callee.kind {
                                    K::PlannedFunctionInstance {
                                        declaration: id, ..
                                    }
                                    | K::ComptimeFunctionInstance {
                                        declaration: id, ..
                                    }
                                    | K::Function(id)
                                    | K::FunctionInstance {
                                        declaration: id, ..
                                    } => callables
                                        .method_definition(id)
                                        .map(|frame| frame.parameters.as_slice()),
                                    _ => None,
                                };
                                if target
                                    .is_some_and(|target| target != parameter_bindings.as_slice())
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
                                            match reason {
                                                crate::free_bindings::Error::Budget(_) => {
                                                    Error::budget(
                                                        expr.source,
                                                        "call default binding analysis budget exhausted",
                                                    )
                                                }
                                                crate::free_bindings::Error::Invalid(_) => {
                                                    error("invalid call default binding scope")
                                                }
                                            }
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
                            for env_arg in env_arguments {
                                work.push(Work::Expr(env_arg, false, false), inspection_fuel)?;
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
                        K::TypeKind { operand, .. } | K::TypeNotSized { operand }
                            if matches!(operand.kind, K::TypeValue(_)) =>
                        {
                            work.push(Work::Expr(operand, false, true), inspection_fuel)?;
                        }
                        K::TypeLayout {
                            query: crate::TypeLayoutQuery::FieldOffset,
                            operand,
                        } if matches!(
                            &operand.kind,
                            K::Tuple(items) if items.len() == 2 && matches!(items[0].kind, K::TypeValue(_))
                        ) =>
                        {
                            if let K::Tuple(items) = &operand.kind {
                                work.push(Work::Expr(&items[0], false, true), inspection_fuel)?;
                                work.push(Work::Expr(&items[1], false, ct), inspection_fuel)?;
                            }
                        }
                        K::TypeLayout { operand, .. }
                            if matches!(operand.kind, K::TypeValue(_)) =>
                        {
                            work.push(Work::Expr(operand, false, true), inspection_fuel)?;
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
                            Worklist::charge(
                                inspection_fuel,
                                captures.len().saturating_add(parameters.len()),
                                expr.source,
                            )?;
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
                            arguments, plan, ..
                        }
                        | K::ApplyNormalForm {
                            arguments, plan, ..
                        } => {
                            work.extend(
                                plan.iter()
                                    .flat_map(|p| &p.preparations)
                                    .map(|p| Work::Expr(&p.expression, false, ct)),
                                inspection_fuel,
                            )?;
                            if !ct {
                                return Err(error(
                                    "normal-form type construction requires comptime",
                                ));
                            }
                            if expr.ty.sort() != Sort::Type {
                                return Err(error(
                                    "normal-form application result must be a type-sorted term",
                                ));
                            }
                            let count = match &expr.kind {
                                K::NormalFormType { head, .. } => {
                                    let meta = package
                                        .normal_forms
                                        .iter()
                                        .find(|h| h.declaration == *head)
                                        .ok_or_else(|| {
                                            error("normal-form constructor metadata is missing")
                                        })?;
                                    if matches!(
                                        meta.role,
                                        crate::normal_forms::Role::Tuple
                                            | crate::normal_forms::Role::TypeList
                                    ) && !matches!(
                                        expr.ty,
                                        Term::Primitive(Primitive::Type | Primitive::Never)
                                    ) {
                                        return Err(error(
                                            "normal-form application must produce Type",
                                        ));
                                    }
                                    if let Some(function) =
                                        package.functions.iter().find(|f| f.declaration == *head)
                                        && let Term::Function { result, .. } = &function.signature
                                        && result.check_scope(&[]).is_ok()
                                    {
                                        let mut referenced = BTreeSet::new();
                                        crate::type_computations::rigid_declarations_bounded(
                                            result,
                                            inspection_fuel,
                                            &mut referenced,
                                        )
                                        .map_err(|failure| match failure {
                                            crate::type_computations::Error::Budget => {
                                                Error::budget(
                                                    expr.source,
                                                    "normal-form result inspection exhausted its budget",
                                                )
                                            }
                                            _ => error("invalid normal-form head result"),
                                        })?;
                                        if function
                                            .type_parameters
                                            .iter()
                                            .all(|id| !referenced.contains(id))
                                            && expr.ty != Term::Primitive(Primitive::Never)
                                            && expr.ty != **result
                                        {
                                            return Err(error(
                                                "normal-form application result disagrees with head signature",
                                            ));
                                        }
                                    }
                                    meta.parameters.len()
                                }
                                K::ApplyNormalForm {
                                    callee, input_ids, ..
                                } => {
                                    let (_, opened_result) = crate::normal_forms::open_signature(
                                        &callee.ty,
                                        input_ids,
                                        inspection_fuel,
                                    )
                                    .map_err(|failure| match failure {
                                        crate::type_computations::Error::Budget => Error::budget(
                                            expr.source,
                                            "normal-form interface inspection exhausted its budget",
                                        ),
                                        _ => error("invalid normal-form application interface"),
                                    })?;
                                    let mut referenced = BTreeSet::new();
                                    crate::type_computations::rigid_declarations_bounded(
                                        &opened_result,
                                        inspection_fuel,
                                        &mut referenced,
                                    )
                                    .map_err(|failure| match failure {
                                        crate::type_computations::Error::Budget => Error::budget(
                                            expr.source,
                                            "normal-form result inspection exhausted its budget",
                                        ),
                                        _ => error("invalid normal-form application result"),
                                    })?;
                                    if input_ids.iter().all(|id| !referenced.contains(id))
                                        && expr.ty != Term::Primitive(Primitive::Never)
                                        && expr.ty != opened_result
                                    {
                                        return Err(error(
                                            "normal-form application result disagrees with callable signature",
                                        ));
                                    }
                                    work.push(Work::Expr(callee, false, ct), inspection_fuel)?;
                                    input_ids.len()
                                }
                                _ => unreachable!(),
                            };
                            if arguments.len() != count {
                                return Err(error(
                                    "normal-form constructor argument count mismatch",
                                ));
                            }
                            work.extend(
                                arguments.iter().map(|a| Work::Argument(a, ct)),
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
                        K::WithTypeInputs {
                            projections,
                            preparations,
                            parameters,
                            arguments,
                            body,
                        } => {
                            work.extend(
                                preparations
                                    .iter()
                                    .map(|p| Work::Expr(&p.expression, false, ct)),
                                inspection_fuel,
                            )?;
                            let schedule = crate::program::InputSchedule {
                                preparations,
                                ..Default::default()
                            };
                            if schedule
                                .check(arguments.len(), |n| {
                                    Worklist::charge(inspection_fuel, n, expr.source)
                                })?
                                .is_none()
                            {
                                return Err(error("invalid lexical Type input preparations"));
                            }
                            let valid = crate::program::check_lexical_type_inputs(
                                parameters,
                                arguments,
                                &expr.ty,
                                &body.ty,
                                projections,
                                inspection_fuel,
                            )
                            .map_err(|_| {
                                Error::budget(
                                    expr.source,
                                    "lexical Type input inspection exhausted",
                                )
                            })?;
                            if !ct || !valid {
                                return Err(error("invalid lexical Type input scope"));
                            }
                            work.push(Work::Expr(body, false, ct), inspection_fuel)?;
                            work.extend(
                                arguments.iter().map(|a| Work::Argument(a, ct)),
                                inspection_fuel,
                            )?;
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
                            let admitted = aliases
                                .get(declaration)
                                .ok_or_else(|| error("missing alias formation contract"))?;
                            let contract = &package.alias_contracts[admitted.position];
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
                                    !admitted.dependent_slots.contains(&index)
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
                                arguments.iter().map(|a| Work::Argument(a, ct)),
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
                            if !ct {
                                return Err(error(
                                    "nominal type construction requires comptime execution",
                                ));
                            }
                            work.extend(
                                arguments.iter().map(|a| Work::Argument(a, ct)),
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
                                arithmetic = true;
                            }
                            if expr.ty != Term::Primitive(Primitive::Void)
                                || !crate::program::checkable_predicate(condition, inspection_fuel)
                                    .map_err(|_| {
                                        Error::budget(
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
                        K::ComptimeFunctionInstance { declaration, .. }
                        | K::PlannedFunctionInstance { declaration, .. } => {
                            let member = trait_index
                                .retained_method(package, *declaration, inspection_fuel)
                                .map_err(|failure| Error::member(expr.source, failure))?;
                            let target = if let Some(member) = member {
                                let signature = member.signature.ordinary().map_err(|_| {
                                    error("contextual method cannot supply an ordinary function instance")
                                })?;
                                (
                                    if signature.is_comptime_callable() {
                                        Phase::Comptime
                                    } else {
                                        Phase::Runtime
                                    },
                                    !member.program.preconditions.is_empty(),
                                    member.is_unsafe,
                                )
                            } else {
                                callables
                                    .phase(declaration)
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
                                arithmetic = true;
                                if !_callee {
                                    return Err(error(
                                        "function value would erase program preconditions; callable program contracts are not implemented",
                                    ));
                                }
                            }
                            match &expr.kind {
                                K::ComptimeFunctionInstance { inputs, .. } => {
                                    for input in inputs.expressions().rev() {
                                        work.push(Work::Expr(input, false, true), inspection_fuel)?;
                                    }
                                }
                                K::PlannedFunctionInstance {
                                    arguments, plan, ..
                                } => {
                                    if member.is_some() {
                                        return Err(error(
                                            "named callable input plan cannot select a trait member",
                                        ));
                                    }
                                    let target =
                                        callables.source.declaration(declaration).ok_or_else(
                                            || error("callable input declaration is missing"),
                                        )?;
                                    if arguments.len() != target.type_parameters.len() {
                                        return Err(error(
                                            "callable declaration input count mismatch",
                                        ));
                                    }
                                    work.extend(
                                        plan.iter()
                                            .flat_map(|p| &p.preparations)
                                            .map(|p| Work::Expr(&p.expression, false, true)),
                                        inspection_fuel,
                                    )?;
                                    work.extend(
                                        arguments.iter().map(|a| Work::Argument(a, true)),
                                        inspection_fuel,
                                    )?;
                                }
                                _ => unreachable!(),
                            }
                        }
                        K::BuildStruct(value)
                        | K::BuildNewtype(value)
                        | K::BuildEnum(value)
                        | K::EnumStructure { subject: value, .. }
                        | K::ObjectQuery { subject: value, .. }
                        | K::ObjectEdit { subject: value, .. }
                        | K::NewtypeRepresentation(value)
                        | K::DefinitionAttributes(value)
                        | K::StructFieldTypes(value)
                        | K::StructFieldNames(value)
                        | K::RecordFieldTypes(value)
                        | K::RecordFieldNames(value)
                        | K::ReflectCallable(value)
                        | K::TypeId(value) => {
                            if !ct {
                                return Err(fail(
                                    expr.source,
                                    "definition construction or struct reflection requires comptime invocation",
                                ));
                            }
                            work.push(Work::Expr(value, false, true), inspection_fuel)?;
                        }
                        K::AddressOf(value)
                        | K::ConstAddress { value, .. }
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
                        | K::ErrorConvert { value, .. }
                        | K::NewtypeConvert(value)
                        | K::TakeBits(value)
                        | K::SymbolName(value)
                        | K::EnumName(value)
                        | K::TypeKind { operand: value, .. }
                        | K::TypeNotSized { operand: value }
                        | K::TypeLayout { operand: value, .. }
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
                        K::TryIntegerConvert(value) => {
                            if !crate::program::try_integer_convert_valid(&value.ty, &expr.ty) {
                                return Err(fail(
                                    expr.source,
                                    "try integer conversion domain mismatch",
                                ));
                            }
                            work.push(Work::Expr(value, false, ct), inspection_fuel)?;
                        }
                        K::TextQuery { subject, query } => {
                            if query.is_comptime_only() && !ct {
                                return Err(fail(
                                    expr.source,
                                    "comptime text operation is unavailable in runtime execution",
                                ));
                            }
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
                                    Error::budget(
                                        expr.source,
                                        "bit selection validation budget exhausted",
                                    )
                                })?;
                            let Term::Pointer(storage) = &pointer.ty else {
                                return Err(error("bit writeback requires a pointer"));
                            };
                            if let Some(message) = crate::program::bit_update_error_with_world(
                                &crate::PackageLayoutWorld(package),
                                storage,
                                selections,
                                &value.ty,
                                &expr.ty,
                                inspection_fuel,
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
                                    Error::budget(
                                        expr.source,
                                        "bit selection validation budget exhausted",
                                    )
                                })?;
                            if let Some(message) = crate::program::bit_update_error_with_world(
                                &crate::PackageLayoutWorld(package),
                                storage_type,
                                selections,
                                &value.ty,
                                &expr.ty,
                                inspection_fuel,
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
                                work.push(
                                    Work::Pattern(&arm.pattern, ct, expr.source),
                                    inspection_fuel,
                                )?;
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
                                        .map_err(|failure| {
                                            Error::preparation(statement.source, failure)
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
                                            | K::DynCall(_)
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
                            .map_err(|failure| Error::preparation(expr.source, failure))?
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
                            work.push(Work::Pattern(pattern, ct, expr.source), inspection_fuel)?;
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
                            let Some(Term::Function { parameters, .. }) = inspection.signature
                            else {
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
                            .map_err(|error| Error::captured(expr.source, error))?;
                        }
                        K::ComptimeValue(value) => {
                            crate::comptime_values::validate_in(
                                value,
                                &expr.ty,
                                package.target,
                                inspection_fuel,
                            )
                            .map_err(|error| Error::captured(expr.source, error))?;
                        }
                        K::TypeValue(_)
                        | K::Literal(_)
                        | K::Local(_)
                        | K::OptionalAbsent
                        | K::Unit
                        | K::Break(_)
                        | K::Continue(_)
                        | K::StaticAddress(_) => {}
                    }
                }
            }
        }
        if let Some(Term::Function { parameters, result }) = inspection.signature {
            for domain in parameters
                .iter()
                .filter(|parameter| ct || parameter.sort != Sort::Runtime)
                .map(|parameter| &parameter.ty)
                .chain(ct.then_some(result.as_ref()))
            {
                storage_scope
                    .check_comptime(domain, inspection_fuel)
                    .map_err(|error| Error::storage(function.source, error))?;
            }
        }
        if let Some(failure) = safety_error {
            return Err(Error::call_safety(function.source, failure));
        }
        Ok(arithmetic)
    }
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

/// Reconstruct the callable telescope for a normal-form head using its declared
/// result signature when available in `package.functions`.
pub fn normal_form_callable_type_in(
    descriptor: &crate::normal_forms::Head,
    package: &Package,
    fuel: &mut usize,
) -> Result<Term, crate::type_computations::Error> {
    if let Some(function) = package
        .functions
        .iter()
        .find(|function| function.declaration == descriptor.declaration)
    {
        let Term::Function { result, .. } = &function.signature else {
            return Err(crate::type_computations::Error::Scope(
                crate::TypeError::InvalidNormalFormFunctionSignature,
            ));
        };
        return crate::normal_forms::callable_type_with_result_from_declarations(
            descriptor.parameters.iter().map(|input| {
                (
                    input.declaration,
                    if input.kind == crate::normal_forms::Kind::Type {
                        Sort::Type
                    } else {
                        Sort::of_comptime_domain(&input.ty)
                    },
                    &input.ty,
                )
            }),
            result,
            descriptor.collector.as_ref().map(|entry| entry.slot),
            fuel,
        );
    }
    descriptor.callable_type_in(fuel)
}

fn validate_runtime_storage_inner(
    ty: &Term,
    package: &Package,
    frames: &[Vec<crate::Sort>],
    resolve: NominalStorageResolver<'_>,
    fuel: &mut usize,
) -> Result<(), StorageError> {
    runtime_storage::validate(
        ty,
        package,
        frames,
        resolve,
        &storage_domains::Domains::new(package),
        fuel,
    )
}

#[cfg(test)]
mod higher_order_nf_tests {
    use super::*;
    use crate::normal_forms::{Head, Kind, Parameter as NfParameter, Role};
    use crate::program::{Function, FunctionDeclaration, LocalId};
    use crate::{DeclarationId, Parameter};

    #[test]
    fn normal_form_callable_type_preserves_higher_order_non_type_result() {
        let helper_decl = DeclarationId {
            package: 1,
            local: 9,
        };
        let nf_decl = DeclarationId {
            package: 1,
            local: 10,
        };
        let input_decl = DeclarationId {
            package: 1,
            local: 11,
        };
        let caller_decl = DeclarationId {
            package: 1,
            local: 12,
        };
        let helper_binding = LocalId { owner: 9, index: 0 };
        let input_binding = LocalId {
            owner: 10,
            index: 0,
        };
        let inner_callable = Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            result: Box::new(Term::Primitive(Primitive::Type)),
        }));
        let helper_fn = Function {
            interface: FunctionDeclaration {
                source: SourceRange { start: 1, end: 5 },
                declaration: helper_decl,
                name: "id_type".to_string(),
                phase: Phase::Comptime,
                inline: false,
                pure: true,
                is_unsafe: false,
                signature: inner_callable.clone(),
                parameters: vec![helper_binding],
                env_parameters: vec![],
                type_parameters: vec![],
                defaults: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
                requirements: vec![],
                capabilities: vec![],
            },
            body: Expr {
                source: SourceRange { start: 2, end: 4 },
                ty: Term::Primitive(Primitive::Type),
                kind: K::Local(helper_binding),
            },
            runtime_deliveries: Default::default(),
        };
        let outer_sig = Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            result: Box::new(inner_callable.clone()),
        };
        let body_sig = Term::Function {
            parameters: vec![],
            result: Box::new(inner_callable.clone()),
        };
        let nf_head = Head {
            collector: None,
            role: Role::Opaque,
            declaration: nf_decl,
            parameters: vec![NfParameter {
                declaration: input_decl,
                binding: input_binding,
                kind: Kind::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            requirements: vec![],
        };
        let expected_callable = Term::NormalFormFunction(Box::new(outer_sig), None);
        let nf_function = Function {
            interface: FunctionDeclaration {
                source: SourceRange { start: 6, end: 15 },
                declaration: nf_decl,
                name: "ho_nf".to_string(),
                phase: Phase::NormalForm,
                inline: false,
                pure: true,
                is_unsafe: false,
                signature: body_sig,
                parameters: vec![],
                env_parameters: vec![],
                type_parameters: vec![input_decl],
                defaults: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
                requirements: vec![],
                capabilities: vec![],
            },
            body: Expr {
                source: SourceRange { start: 7, end: 14 },
                ty: inner_callable,
                kind: K::Function(helper_decl),
            },
            runtime_deliveries: Default::default(),
        };
        let caller = Function {
            interface: FunctionDeclaration {
                source: SourceRange { start: 20, end: 40 },
                declaration: caller_decl,
                name: "caller".to_string(),
                phase: Phase::Comptime,
                inline: false,
                pure: true,
                is_unsafe: false,
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(expected_callable.clone()),
                },
                parameters: vec![],
                env_parameters: vec![],
                type_parameters: vec![],
                defaults: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
                requirements: vec![],
                capabilities: vec![],
            },
            body: Expr {
                source: SourceRange { start: 25, end: 39 },
                ty: expected_callable.clone(),
                kind: K::Function(nf_decl),
            },
            runtime_deliveries: Default::default(),
        };
        let package = Package {
            functions: vec![helper_fn, nf_function, caller],
            normal_forms: vec![nf_head.clone()],
            ..Default::default()
        };
        let mut fuel = 10_000;
        let reconstructed = normal_form_callable_type_in(&nf_head, &package, &mut fuel).unwrap();
        assert_eq!(reconstructed, expected_callable);
        let empty = Package::default();
        assert_eq!(
            normal_form_callable_type_in(&nf_head, &empty, &mut fuel).unwrap(),
            nf_head.callable_type_in(&mut fuel).unwrap()
        );
        Inspection::new(&package)
            .check_package(&mut 10_000)
            .unwrap();
    }
}
