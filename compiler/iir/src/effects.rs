//! Concrete direct-effect environments. Requests retain explicit occurrence
//! slots; callable preparation is checked in the caller's lexical environment.
use crate::{Package, visit};
use nir::effects::{Abi, Operation};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Pattern};
use nir::{Primitive, Sort, Term};

/// Potential effect-control exits of an evaluated residual expression.
/// Calls with a nonempty effect row retain this boundary even when evidence
/// is forwarded through another function or a runtime-selected callable.
/// This classifies where cleanup can be published, not whether a particular
/// handler actually exits or whether its control ABI is implemented.
/// Publication and native cleanup admission must use the same classification.
pub fn suspension_boundary(expr: &Expr) -> bool {
    match &expr.kind {
        K::EffectRequest {
            abi: Abi::Direct, ..
        } => true,
        K::Call { callee, .. } => matches!(callee.ty.callable_signature(),
            Term::Function {result, ..} if matches!(result.as_ref(),
                Term::Computation {effects, ..} if !effects.is_empty())),
        K::EffectHandle { handlers, .. } => handlers.iter().any(|h| h.abi == Abi::DirectPassing),
        _ => false,
    }
}

/// Concrete pointer-free installation payloads. This is a layout contract,
/// not Copy evidence; resource admission must prove delivery separately.
pub fn data_passing_signature(
    package: &Package,
    signature: &Term,
    fuel: &mut usize,
) -> Result<bool, String> {
    let Term::Function { parameters, result } = signature else {
        return Ok(false);
    };
    if !parameters.is_empty() {
        return Ok(false);
    }
    passing_data(package, result, fuel, 0)
}
fn passing_data(
    package: &Package,
    ty: &Term,
    fuel: &mut usize,
    depth: usize,
) -> Result<bool, String> {
    spend(fuel)?;
    if depth >= 128 {
        return Err("IIR effect data depth exhausted".into());
    }
    nir::type_computations::charge_term(ty, fuel).map_err(|e| e.to_string())?;
    match ty {
        Term::Primitive(_) => Ok(
            nir::program::intrinsic_pointer_value(ty) || *ty == Term::Primitive(Primitive::Void)
        ),
        Term::ErrorResult { errors, result } => {
            for error in errors {
                if !passing_data(package, error, fuel, depth + 1)? {
                    return Ok(false);
                }
            }
            if **result == Term::Primitive(Primitive::Never) {
                return Ok(true);
            }
            passing_data(package, result, fuel, depth + 1)
        }
        Term::Tuple(fields) => {
            for field in fields {
                if !passing_data(package, field, fuel, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Term::Record(fields) => {
            for (_, field) in fields {
                if !passing_data(package, field, fuel, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Term::NormalForm { head, .. } => {
            for definition in &package.normal_forms {
                spend(fuel)?;
                if definition.declaration == *head {
                    if let Some(fields) = definition.tuple_elements(ty) {
                        for field in fields {
                            if !passing_data(package, field, fuel, depth + 1)? {
                                return Ok(false);
                            }
                        }
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
        Term::Nominal { .. } => {
            for instance in &package.nominals {
                spend(fuel)?;
                if instance.ty != *ty {
                    continue;
                }
                match &instance.definition {
                    crate::NominalDefinition::Struct(definition) => {
                        for (_, field) in &definition.fields {
                            if !passing_data(package, field, fuel, depth + 1)? {
                                return Ok(false);
                            }
                        }
                    }
                    crate::NominalDefinition::Enum(definition) => {
                        for variant in &definition.variants {
                            spend(fuel)?;
                            for field in &variant.fields {
                                if !passing_data(package, field, fuel, depth + 1)? {
                                    return Ok(false);
                                }
                            }
                        }
                    }
                    crate::NominalDefinition::Newtype(definition) => {
                        return passing_data(package, &definition.representation, fuel, depth + 1);
                    }
                }
                return Ok(true);
            }
            Err("IIR effect data nominal layout missing".into())
        }
        _ => Ok(false),
    }
}

fn spend(fuel: &mut usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or("IIR effect validation budget exhausted")?;
    Ok(())
}
fn compatible(actual: &Term, expected: &Term) -> bool {
    actual == expected || *actual == Term::Primitive(Primitive::Never)
}
fn row_result(ty: &Term) -> (&[Operation], &Term) {
    if let Term::Computation { effects, result } = ty {
        (effects, result)
    } else {
        (&[], ty)
    }
}
// Function values may advertise an effectful result. A computation is not yet
// a storable data value in the executable reference representation.
fn data_type(package: &Package, ty: &Term, fuel: &mut usize, depth: usize) -> Result<(), String> {
    spend(fuel)?;
    if depth >= 128 {
        return Err("IIR effect type depth exhausted".into());
    }
    match ty {
        Term::Computation { .. } => {
            return Err("IIR stored computation values require value/evidence lowering".into());
        }
        Term::Function { parameters, result } => {
            for p in parameters {
                data_type(package, &p.ty, fuel, depth + 1)?;
            }
            data_type(package, row_result(result).1, fuel, depth + 1)?;
        }
        Term::ComptimeFunction(inner) | Term::UnsafeFunction(inner) | Term::Pointer(inner) => {
            data_type(package, inner, fuel, depth + 1)?
        }
        Term::Tuple(_) | Term::Record(_) => {
            if let Term::Tuple(items) = ty {
                for item in items {
                    data_type(package, item, fuel, depth + 1)?;
                }
            }
            if let Term::Record(items) = ty {
                for (_, item) in items {
                    data_type(package, item, fuel, depth + 1)?;
                }
            }
        }
        Term::Closure {
            environment,
            signature,
        } => {
            for item in environment {
                data_type(package, item, fuel, depth + 1)?;
            }
            data_type(package, signature, fuel, depth + 1)?;
        }
        Term::NormalForm { head, .. } => {
            // Only an explicitly registered Tuple head grants a field layout.
            // Other normal-form arguments remain opaque type metadata.
            if let Some(fields) = package
                .normal_forms
                .iter()
                .find(|definition| definition.declaration == *head)
                .and_then(|definition| definition.tuple_elements(ty))
            {
                for field in fields {
                    data_type(package, field, fuel, depth + 1)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn validate(
    package: &Package,
    function: &Function,
    fuel: &mut usize,
) -> Result<(), String> {
    data_type(package, &function.signature, fuel, 0)?;
    let Term::Function { result, .. } = function.signature.callable_signature() else {
        return Err("IIR effect function signature missing".into());
    };
    let (row, _) = row_result(result);
    let runtime = function.phase == FunctionPhase::Runtime;
    expression(package, &function.body, row, None, runtime, true, fuel, 0)?;
    for value in function
        .preconditions
        .iter()
        .chain(&function.postconditions)
        .chain(function.defaults.iter().map(|(_, value)| value))
    {
        expression(package, value, &[], None, runtime, false, fuel, 0)?;
    }
    Ok(())
}
fn operation(
    package: &Package,
    op: &Operation,
    signature: &Term,
    fuel: &mut usize,
) -> Result<(), String> {
    for arg in &op.arguments {
        if arg.sort() == Sort::Type {
            package.validate_type(arg, fuel)?;
        } else if arg.is_structural_index_value() {
            crate::completed_dependencies(arg, &package.runtime_bindings, fuel)?;
            package.type_links(arg, fuel)?;
        } else {
            return Err("IIR effect label argument is not a completed input".into());
        }
    }
    package.validate_type(signature, fuel)?;
    let Term::Function { parameters, result } = signature else {
        return Err("IIR effect signature is not a function".into());
    };
    for parameter in parameters {
        if parameter.sort != Sort::Runtime || parameter.ty.check_scope(&[]).is_err() {
            return Err("IIR dependent effect input requires identity lowering".into());
        }
        data_type(package, &parameter.ty, fuel, 0)?;
    }
    result
        .check_scope(&[])
        .map_err(|_| "IIR dependent effect result requires identity lowering")?;
    data_type(package, result, fuel, 0)
}
fn pattern(
    package: &Package,
    p: &Pattern,
    row: &[Operation],
    resume: Option<&Term>,
    runtime: bool,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    spend(fuel)?;
    if depth >= 128 {
        return Err("IIR effect pattern depth exhausted".into());
    }
    match p {
        Pattern::Then { first, value, next } => {
            pattern(package, first, row, resume, runtime, fuel, depth + 1)?;
            expression(package, value, row, resume, runtime, false, fuel, depth + 1)?;
            pattern(package, next, row, resume, runtime, fuel, depth + 1)?;
        }
        Pattern::Guard {
            pattern: p,
            condition,
        } => {
            pattern(package, p, row, resume, runtime, fuel, depth + 1)?;
            expression(
                package,
                condition,
                row,
                resume,
                runtime,
                false,
                fuel,
                depth + 1,
            )?;
        }
        Pattern::ErrorResult { inner, .. }
        | Pattern::Observe { inner, .. }
        | Pattern::Binding { inner, .. }
        | Pattern::Remap { pattern: inner, .. } => {
            pattern(package, inner, row, resume, runtime, fuel, depth + 1)?
        }
        Pattern::Tuple(items)
        | Pattern::TypeTuple(items)
        | Pattern::Or(items)
        | Pattern::NominalType { fields: items, .. }
        | Pattern::Record { fields: items, .. }
        | Pattern::AnonymousRecord { fields: items, .. }
        | Pattern::Variant { fields: items, .. }
        | Pattern::NormalForm {
            arguments: items, ..
        } => {
            for item in items {
                pattern(package, item, row, resume, runtime, fuel, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn expression(
    package: &Package,
    root: &Expr,
    row: &[Operation],
    resume: Option<&Term>,
    runtime: bool,
    computation: bool,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("IIR effect environment depth exhausted".into());
    }
    visit::expression_pruned(root, fuel, |expr, fuel| {
        let allowed = computation && std::ptr::eq(expr, root);
        if matches!(expr.ty, Term::Computation { .. }) && !allowed {
            return Err("IIR computation requires an explicit row mapping".into());
        }
        if !matches!(expr.ty, Term::Computation { .. }) {
            data_type(package, &expr.ty, fuel, depth)?;
        }
        if !runtime
            && matches!(
                expr.kind,
                K::EffectHandle { .. }
                    | K::EffectRun { .. }
                    | K::EffectComputation { .. }
                    | K::EffectRequest { .. }
                    | K::EffectResume(_)
            )
        {
            return Err("IIR comptime effects are not supported".into());
        }
        match &expr.kind {
            K::EffectComputation { effects, body } => {
                if effects != row
                    || row_result(&expr.ty).0 != effects
                    || !compatible(&body.ty, row_result(&expr.ty).1)
                {
                    return Err("IIR computation environment/result mismatch".into());
                }
                expression(package, body, row, resume, runtime, false, fuel, depth + 1)?;
                Ok(false)
            }
            K::EffectRun {
                computation: body,
                slots,
            } => {
                let Term::Computation { effects, result } = &body.ty else {
                    return Err("IIR effect-run input is not a computation".into());
                };
                if effects.len() != slots.len() || !compatible(result, &expr.ty) {
                    return Err("IIR residual computation mapping is not supported".into());
                }
                for (i, (op, slot)) in effects.iter().zip(slots).enumerate() {
                    spend(fuel)?;
                    if row.get(*slot) != Some(op) {
                        return Err("IIR effect mapping selects the wrong occurrence".into());
                    }
                    for j in 0..i {
                        spend(fuel)?;
                        if slots[j] == *slot || (effects[j] == *op && slots[j] >= *slot) {
                            return Err(
                                "IIR effect mapping duplicates or reverses an occurrence".into()
                            );
                        }
                    }
                }
                mapped(
                    package,
                    body,
                    row,
                    effects,
                    resume,
                    runtime,
                    fuel,
                    depth + 1,
                )?;
                Ok(false)
            }
            K::EffectHandle {
                computation: body,
                handlers,
                outer,
            } => {
                if outer != row {
                    return Err("IIR handler outer environment mismatch".into());
                }
                let mut installed = Vec::new();
                for handler in handlers {
                    if !matches!(handler.abi, Abi::Direct | Abi::DirectPassing) {
                        return Err("IIR non-direct effects require control ABI lowering".into());
                    }
                    operation(package, &handler.operation, &handler.signature, fuel)?;
                    for prior in &installed {
                        spend(fuel)?;
                        if prior == &handler.operation {
                            return Err("IIR duplicate handler operation group".into());
                        }
                    }
                    installed.push(handler.operation.clone());
                    let Term::Function { parameters, result } = &handler.signature else {
                        unreachable!()
                    };
                    if handler.abi == Abi::DirectPassing && !parameters.is_empty() {
                        return Err("IIR direct passing operation has parameters".into());
                    }
                    if let Some(slot) = handler.fallback {
                        if outer.get(slot) != Some(&handler.operation) {
                            return Err("IIR handler fallback selects the wrong occurrence".into());
                        }
                    }
                    for arm in &handler.arms {
                        if !compatible(&arm.body.ty, result) {
                            return Err("IIR handler branch result mismatch".into());
                        }
                        pattern(
                            package,
                            &arm.pattern,
                            outer,
                            Some(result),
                            runtime,
                            fuel,
                            depth + 1,
                        )?;
                        if let Some(guard) = &arm.guard {
                            expression(
                                package,
                                guard,
                                outer,
                                Some(result),
                                runtime,
                                false,
                                fuel,
                                depth + 1,
                            )?;
                        }
                        expression(
                            package,
                            &arm.body,
                            outer,
                            Some(result),
                            runtime,
                            false,
                            fuel,
                            depth + 1,
                        )?;
                    }
                }
                installed.extend_from_slice(outer);
                if row_result(&body.ty).0 != installed
                    || !compatible(row_result(&body.ty).1, &expr.ty)
                {
                    return Err("IIR handler computation/result mismatch".into());
                }
                expression(
                    package,
                    body,
                    &installed,
                    resume,
                    runtime,
                    true,
                    fuel,
                    depth + 1,
                )?;
                Ok(false)
            }
            K::EffectRequest {
                operation: op,
                abi,
                allowed_index,
                signature,
                arguments,
            } => {
                if !matches!(abi, Abi::Direct | Abi::DirectPassing) {
                    return Err("IIR non-direct effects require control ABI lowering".into());
                }
                operation(package, op, signature, fuel)?;
                if row.get(*allowed_index) != Some(op) {
                    return Err("IIR effect request has no matching occurrence".into());
                }
                let Term::Function { parameters, result } = signature.as_ref() else {
                    unreachable!()
                };
                if *abi == Abi::DirectPassing && !parameters.is_empty() {
                    return Err("IIR direct passing operation has parameters".into());
                }
                if parameters.len() != arguments.len()
                    || !(compatible(result, &expr.ty)
                        || expr.ty == Term::Primitive(Primitive::Never)
                            && arguments
                                .iter()
                                .any(|argument| argument.ty == Term::Primitive(Primitive::Never)))
                    || parameters
                        .iter()
                        .zip(arguments)
                        .any(|(p, a)| !compatible(&a.ty, &p.ty))
                {
                    return Err("IIR effect request signature mismatch".into());
                }
                Ok(true)
            }
            K::EffectResume(value) => {
                if !resume.is_some_and(|ty| compatible(&value.ty, ty))
                    || expr.ty != Term::Primitive(Primitive::Never)
                {
                    return Err("IIR resume has no matching lexical branch result".into());
                }
                Ok(true)
            }
            K::Closure { captures, body, .. } => {
                for capture in captures {
                    data_type(package, &capture.ty, fuel, depth + 1)?;
                    if let Some(value) = &capture.value {
                        expression(package, value, row, resume, runtime, false, fuel, depth + 1)?;
                    }
                }
                let Term::Function { result, .. } = expr.ty.callable_signature() else {
                    return Err("IIR effect closure signature missing".into());
                };
                let own_row = row_result(result).0;
                expression(
                    package,
                    body,
                    own_row,
                    None,
                    !expr.ty.is_comptime_callable(),
                    true,
                    fuel,
                    depth + 1,
                )?;
                Ok(false)
            }
            _ if matches!(expr.ty, Term::Computation { .. }) => {
                Err("IIR computation requires an explicit environment boundary".into())
            }
            _ => Ok(true),
        }
    })
}
fn mapped(
    package: &Package,
    expr: &Expr,
    caller: &[Operation],
    callee: &[Operation],
    resume: Option<&Term>,
    runtime: bool,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    spend(fuel)?;
    if depth >= 128 {
        return Err("IIR mapped computation depth exhausted".into());
    }
    if !matches!(&expr.ty, Term::Computation { effects, .. } if effects == callee) {
        return Err("IIR mapped computation row mismatch".into());
    }
    match &expr.kind {
        K::Call {
            callee: target,
            arguments,
            ..
        } => {
            let Term::Function { result, .. } = target.ty.callable_signature() else {
                return Err("IIR effect callee signature missing".into());
            };
            if row_result(result).0 != callee || result.as_ref() != &expr.ty {
                return Err("IIR effect call result row mismatch".into());
            }
            expression(
                package,
                target,
                caller,
                resume,
                runtime,
                false,
                fuel,
                depth + 1,
            )?;
            for argument in arguments {
                expression(
                    package,
                    &argument.value,
                    caller,
                    resume,
                    runtime,
                    false,
                    fuel,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        K::Block { statements, result } | K::CallPreparation { statements, result } => {
            let result = result
                .as_deref()
                .ok_or("IIR computation block has no result")?;
            if result.ty != expr.ty {
                return Err("IIR computation block result mismatch".into());
            }
            for statement in statements {
                expression(
                    package,
                    statement,
                    caller,
                    resume,
                    runtime,
                    false,
                    fuel,
                    depth + 1,
                )?;
            }
            mapped(
                package,
                result,
                caller,
                callee,
                resume,
                runtime,
                fuel,
                depth + 1,
            )
        }
        K::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let else_branch = else_branch
                .as_deref()
                .ok_or("IIR computation conditional has no fallback")?;
            if then_branch.ty != expr.ty || else_branch.ty != expr.ty {
                return Err("IIR computation conditional result mismatch".into());
            }
            expression(
                package,
                condition,
                caller,
                resume,
                runtime,
                false,
                fuel,
                depth + 1,
            )?;
            mapped(
                package,
                then_branch,
                caller,
                callee,
                resume,
                runtime,
                fuel,
                depth + 1,
            )?;
            mapped(
                package,
                else_branch,
                caller,
                callee,
                resume,
                runtime,
                fuel,
                depth + 1,
            )
        }
        K::Match { value, arms } => {
            expression(
                package,
                value,
                caller,
                resume,
                runtime,
                false,
                fuel,
                depth + 1,
            )?;
            for arm in arms {
                if arm.body.ty != expr.ty {
                    return Err("IIR computation match result mismatch".into());
                }
                pattern(
                    package,
                    &arm.pattern,
                    caller,
                    resume,
                    runtime,
                    fuel,
                    depth + 1,
                )?;
                if let Some(guard) = &arm.guard {
                    expression(
                        package,
                        guard,
                        caller,
                        resume,
                        runtime,
                        false,
                        fuel,
                        depth + 1,
                    )?;
                }
                mapped(
                    package,
                    &arm.body,
                    caller,
                    callee,
                    resume,
                    runtime,
                    fuel,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        K::EffectComputation { .. } => expression(
            package,
            expr,
            callee,
            resume,
            runtime,
            true,
            fuel,
            depth + 1,
        ),
        _ => Err("IIR computation value requires execution/evidence lowering".into()),
    }
}

/// A concrete operation contract. Row position is deliberately not stored here:
/// repeated occurrences have the same contract but separate runtime entries.
#[derive(Clone, Debug, PartialEq)]
pub struct Contract {
    pub operation: Operation,
    pub abi: Abi,
    pub signature: Term,
}
impl std::fmt::Display for Contract {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(effect-contract {} (abi {}) (signature {}))",
            self.operation, self.abi, self.signature
        )
    }
}

/// Borrowed, validated contracts for one immutable concrete graph. This table
/// describes the operation ABI; it does not select handler occurrences, grant
/// Copy permission, or authorize a control/resource transfer.
pub struct Contracts<'a> {
    graph: &'a Package,
    entries: Vec<Contract>,
}
impl<'a> Contracts<'a> {
    pub fn acquire(graph: &'a Package, fuel: &mut usize) -> Result<Self, String> {
        graph.validate(fuel)?;
        Ok(Self {
            graph,
            entries: collect_contracts(graph, fuel)?,
        })
    }

    pub fn belongs_to(&self, graph: &Package) -> bool {
        std::ptr::eq(self.graph, graph)
    }

    pub fn get(&self, operation: &Operation, fuel: &mut usize) -> Result<&Contract, String> {
        for argument in &operation.arguments {
            self.graph.validate_type(argument, fuel)?;
        }
        for entry in &self.entries {
            spend(fuel)?;
            for argument in &entry.operation.arguments {
                self.graph.validate_type(argument, fuel)?;
            }
            if &entry.operation == operation {
                return Ok(entry);
            }
        }
        Err(format!(
            "IIR effect operation contract missing: {operation}"
        ))
    }

    /// Preserve the exact row order and multiplicity. A declaration identity
    /// alone cannot identify either a generic operation or its row occurrence.
    pub fn row(&self, row: &[Operation], fuel: &mut usize) -> Result<Vec<&Contract>, String> {
        *fuel = fuel
            .checked_sub(row.len())
            .ok_or("IIR effect row lookup budget exhausted")?;
        row.iter()
            .map(|operation| self.get(operation, fuel))
            .collect()
    }
}
impl std::fmt::Display for Contracts<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(effect-contracts")?;
        for entry in &self.entries {
            write!(f, " {entry}")?;
        }
        f.write_str(")")
    }
}

pub(crate) fn validate_contracts(package: &Package, fuel: &mut usize) -> Result<(), String> {
    collect_contracts(package, fuel).map(|_| ())
}

fn collect_contracts(package: &Package, fuel: &mut usize) -> Result<Vec<Contract>, String> {
    // The index accelerates lookup; only first source traversal order is exposed
    // in the resulting table/dump, never hash iteration or pointer ordering.
    let mut index = std::collections::HashMap::<Operation, usize>::new();
    let mut contracts: Vec<Contract> = Vec::new();
    for contract in &package.effect_contracts {
        spend(fuel)?;
        operation(package, &contract.operation, &contract.signature, fuel)?;
        if contract.abi == Abi::DirectPassing
            && !matches!(&contract.signature, Term::Function {parameters, ..} if parameters.is_empty())
        {
            return Err("IIR direct passing operation contract has parameters".into());
        }
        if index
            .insert(contract.operation.clone(), contracts.len())
            .is_some()
        {
            return Err("duplicate IIR declared effect contract".into());
        }
        contracts.push(contract.clone());
    }
    for instance in &package.functions {
        visit::expressions(&instance.function, fuel, |expr, fuel| {
            let mut insert = |op: &Operation, abi: Abi, signature: &Term| -> Result<(), String> {
                spend(fuel)?;
                // Validate/charge the complete key before hashing or comparing it.
                operation(package, op, signature, fuel)?;
                if let Some(&position) = index.get(op) {
                    let prior = &contracts[position];
                    if prior.abi != abi || &prior.signature != signature {
                        return Err(
                            "IIR effect occurrence has conflicting operation contracts".into()
                        );
                    }
                } else {
                    index.insert(op.clone(), contracts.len());
                    contracts.push(Contract {
                        operation: op.clone(),
                        abi,
                        signature: signature.clone(),
                    });
                }
                Ok(())
            };
            match &expr.kind {
                K::EffectRequest {
                    operation,
                    abi,
                    signature,
                    ..
                } => insert(operation, *abi, signature)?,
                K::EffectHandle { handlers, .. } => {
                    for handler in handlers {
                        insert(&handler.operation, handler.abi, &handler.signature)?;
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
    }
    Ok(contracts)
}

#[cfg(test)]
mod tuple_tests {
    use super::*;
    use nir::DeclarationId;
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    use nir::program::LocalId;

    #[test]
    fn tuple_storage_checks_fields_without_unfolding_opaque_heads() {
        let id = DeclarationId {
            package: 1,
            local: 0,
        };
        let mut package = Package {
            normal_forms: vec![Head {
                declaration: id,
                role: Role::Tuple,
                parameters: vec![Parameter {
                    declaration: DeclarationId {
                        package: 1,
                        local: 1,
                    },
                    binding: LocalId { owner: 0, index: 0 },
                    kind: Kind::TypeList,
                    ty: Term::Nominal {
                        declaration: DeclarationId {
                            package: 1,
                            local: 2,
                        },
                        arguments: vec![Term::Primitive(Primitive::Type)],
                    },
                }],
                requirements: vec![],
            }],
            ..Default::default()
        };
        let tuple = |field| Term::NormalForm {
            head: id,
            sort: Sort::Type,
            arguments: vec![Term::TypeListValue(vec![field])],
        };
        let scalar = Term::Primitive(Primitive::Bool);
        let computation = Term::Computation {
            effects: vec![],
            result: Box::new(scalar.clone()),
        };
        let stored = tuple(tuple(computation.clone()));
        assert!(
            data_type(&package, &stored, &mut 100, 0)
                .unwrap_err()
                .contains("stored computation")
        );
        let callable = tuple(Term::Function {
            parameters: vec![],
            result: Box::new(computation),
        });
        data_type(&package, &callable, &mut 100, 0).unwrap();
        let valid = tuple(tuple(scalar));
        assert!(data_type(&package, &valid, &mut 1, 0).is_err());
        data_type(&package, &valid, &mut 100, 0).unwrap();
        assert!(data_type(&package, &valid, &mut 100, 128).is_err());
        package.normal_forms[0].role = Role::Opaque;
        // A lookalike application's arguments do not define stored fields.
        data_type(&package, &stored, &mut 100, 0).unwrap();
    }
}
