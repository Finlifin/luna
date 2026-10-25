//! A method invocation has interface predicates as well as a selected body.
use super::*;

/// A footprint for this full method identity, not for arbitrary calls of the
/// implementation function. It does not certify the predicates or cleanup.
#[derive(Clone, Debug, PartialEq)]
pub struct MethodSummary {
    pub callee: Expr,
    pub summary: Summary,
}
impl fmt::Display for MethodSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(method-memory-summary (callee {}) {})",
            self.callee, self.summary
        )
    }
}

/// Infer a concrete invocation with no external callable premises. The composed
/// body is private to this query: no body-only cache entry gains interface facts.
pub fn infer_method_in_package(
    package: &Package,
    callee: &Expr,
    fuel: &mut usize,
) -> Result<MethodSummary, String> {
    infer_method_with_code_inputs(package, callee, &BTreeMap::new(), &BTreeMap::new(), fuel)
}

/// Compose interface and body footprints under concrete callback premises.
/// The returned summary retains these premises; it cannot authorize another
/// invocation with different callbacks or captured environments.
pub fn infer_method_with_code_inputs(
    package: &Package,
    callee: &Expr,
    callable_inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    fuel: &mut usize,
) -> Result<MethodSummary, String> {
    for value in callable_inputs.values() {
        if !matches!(
            value,
            CallableResult::Known(_) | CallableResult::Candidates(_)
        ) {
            return Err("memory callable premise requires named code identities".into());
        }
    }
    let prepared = own_code_inputs(code_inputs, &[], &[], fuel)?;
    let invocation = prepare_method_invocation(package, callee, fuel)?;
    let composed = &invocation.function;
    let declaration = &invocation.implementation.declaration;
    let arguments = &invocation.implementation.arguments;
    // Subcalls use the existing complete fixed-point query. Never publish its
    // provisional cycle seeds as a composed root summary.
    let mut round = Round {
        complete_calls: true,
        ..Default::default()
    };
    let mut summary = infer_body(
        composed,
        &[],
        callable_inputs,
        &prepared,
        &BTreeMap::new(),
        Some(package),
        fuel,
        &mut round,
    )?;
    if !round.pending.is_empty() || !round.queue.is_empty() {
        return Err("composed memory summary retained an incomplete fixed point".into());
    }
    summary.may_run_cleanup |= round.used_external_cleanup && round.mutates_pointer_storage;
    finish_instance_summary(package, *declaration, arguments, &mut summary, fuel)?;
    Ok(MethodSummary {
        callee: invocation.callee,
        summary,
    })
}

/// Query-local interface invocation frame. The function declaration is only an
/// analysis label: this must never be registered as a replacement implementation
/// or cached by that label without the full `callee` identity and input premises.
#[derive(Clone, Debug, PartialEq)]
pub struct MethodInvocation {
    pub callee: Expr,
    pub implementation: iir::InstanceKey,
    pub function: Function,
}
impl fmt::Display for MethodInvocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(method-invocation (callee {}) (implementation {}) {})",
            self.callee, self.implementation, self.function
        )
    }
}

/// Prepare one interface frame around the selected runtime implementation call.
/// This is not memory/ownership certification and does not evaluate arguments.
pub fn prepare_method_invocation(
    package: &Package,
    callee: &Expr,
    fuel: &mut usize,
) -> Result<MethodInvocation, String> {
    let K::TraitMethod {
        interface,
        interface_arguments,
        member,
        subject,
        method_arguments,
    } = &callee.kind
    else {
        return Err("composed memory summary requires a trait method".into());
    };
    let definition = package
        .traits
        .iter()
        .find(|t| t.declaration == *interface)
        .ok_or("composed memory summary lacks its interface")?;
    let contract = definition
        .methods
        .iter()
        .find(|m| m.declaration == *member)
        .ok_or("composed memory summary lacks its method")?;
    if contract.is_quote {
        return Err("quote method requires expansion before memory composition".into());
    }
    // Identity-only selection is kept inside this transaction. Its result is
    // never published as a replacement for the interface invocation.
    let selected = selected_method_callee(package, callee, fuel, true)?;
    let K::FunctionInstance {
        declaration,
        arguments,
    } = &selected.kind
    else {
        return Err("composed memory summary lacks a selected instance".into());
    };
    let (body, _) = package
        .runtime_body(*declaration, arguments)
        .ok_or("composed memory summary lacks its body")?;
    // Build only the invocation frame. The concrete implementation remains in
    // the package and is analyzed through the ordinary complete call query.
    *fuel = fuel
        .checked_sub(
            body.parameters
                .len()
                .saturating_add(body.name.len())
                .saturating_add(1),
        )
        .ok_or("composed memory frame budget exhausted")?;
    bound_type(selected.ty.callable_signature(), fuel)?;
    let mut composed = Function {
        phase: FunctionPhase::Runtime,
        inline: false,
        pure: contract.pure,
        is_unsafe: contract.is_unsafe,
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        capabilities: vec![],
        requirements: vec![],
        type_parameters: vec![],
        declaration: *declaration,
        name: body.name.clone(),
        signature: selected.ty.callable_signature().clone(),
        parameters: body.parameters.clone(),
        body: Expr {
            source: callee.source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::Unit,
        },
        defaults: vec![],
    };

    let binders: Vec<_> = definition
        .binders_with_self(fuel)?
        .into_iter()
        .chain(memory_binders(
            Some(package),
            &contract.type_parameters,
            fuel,
        )?)
        .collect();
    for ty in std::iter::once(subject)
        .chain(interface_arguments)
        .chain(method_arguments)
    {
        bound_type(ty, fuel)?;
    }
    let inputs: Vec<_> = std::iter::once(subject.clone())
        .chain(interface_arguments.iter().cloned())
        .chain(method_arguments.iter().cloned())
        .collect();
    *fuel = fuel
        .checked_sub(contract.program.entry_snapshots.len().saturating_add(1))
        .ok_or("composed memory contract budget exhausted")?;
    for expr in contract
        .program
        .preconditions
        .iter()
        .chain(&contract.program.postconditions)
    {
        nir::copy_budget::charge_substitution(expr, &binders, &inputs, fuel)
            .map_err(|_| "composed memory predicate budget exhausted")?;
    }
    let mut program = contract
        .program
        .instantiate_parameters(&binders, &inputs)
        .map_err(|e| e.to_string())?;
    let plan = program
        .binding_alignment(
            &contract.parameter_bindings,
            &composed.parameters,
            None,
            &[],
            fuel,
        )
        .map_err(str::to_owned)?;
    for predicate in program
        .preconditions
        .iter_mut()
        .chain(&mut program.postconditions)
    {
        // Remapping accepts only the supported effect-free predicate forms.
        // A failed candidate is discarded; it cannot authorize a body-only call.
        nir::contracts::remap_predicate(predicate, &plan.aliases).map_err(str::to_owned)?;
    }
    let Term::Function { parameters, result } = composed.signature.callable_signature() else {
        return Err("composed memory method lacks a runtime signature".into());
    };
    if parameters.iter().any(|p| p.sort != nir::Sort::Runtime) {
        return Err("method memory composition requires a runtime-only parameter frame".into());
    }
    let frames = vec![parameters.iter().map(|p| p.sort).collect()];
    for requirement in &contract.requirements {
        nir::copy_budget::charge_predicate_substitution(requirement, &binders, &inputs, fuel)
            .map_err(|_| "composed memory type premise budget exhausted")?;
        composed.requirements.push(
            requirement
                .instantiate_parameters_in_scope(&binders, &inputs, &frames)
                .map_err(|e| e.to_string())?,
        );
    }
    // Keep the implementation invocation as a call boundary. In particular,
    // interface exit observations follow the implementation's own postchecks
    // and cleanup, rather than being spliced into its local exit frame.
    if parameters.len() != composed.parameters.len() {
        return Err("composed memory parameter layout mismatch".into());
    }
    let mut call_arguments = Vec::with_capacity(parameters.len());
    for (parameter, (binding, input)) in composed.parameters.iter().zip(parameters).enumerate() {
        bound_type(&input.ty, fuel)?;
        call_arguments.push(nir::program::CallArgument {
            parameter,
            is_default: false,
            value: Expr {
                source: callee.source,
                ty: input.ty.clone(),
                kind: K::Local(*binding),
            },
        });
    }
    bound_type(result, fuel)?;
    nir::copy_budget::charge_substitution(&selected, &[], &[], fuel)
        .map_err(|_| "composed memory invocation budget exhausted")?;
    composed.body = Expr {
        source: callee.source,
        ty: (**result).clone(),
        kind: K::Call {
            callee: Box::new(selected.clone()),
            arguments: call_arguments,
            parameter_bindings: composed.parameters.clone(),
        },
    };
    composed.defaults.clear();
    composed.preconditions = program.preconditions;
    composed.postconditions = program.postconditions;
    composed.result_binding = plan.result_binding;
    composed.entry_snapshots = plan.entry_snapshots;

    nir::copy_budget::charge_substitution(callee, &[], &[], fuel)
        .map_err(|_| "composed memory identity budget exhausted")?;
    Ok(MethodInvocation {
        callee: callee.clone(),
        implementation: iir::InstanceKey {
            declaration: *declaration,
            arguments: arguments.clone(),
        },
        function: composed,
    })
}
