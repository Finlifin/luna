//! Conditional memory inference for data-only direct effect delivery. These premises do
//! not claim that a handler is pure: its captured storage is checked separately
//! by the installer. They cannot authorize pointer arguments or escaping results.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EffectInput {
    pub operation: nir::effects::Operation,
    pub abi: nir::effects::Abi,
    pub signature: Term,
}
impl fmt::Display for EffectInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(effect-input {} (abi {}) (signature {}) (capture-contract stable-scalar-storage))",
            self.operation, self.abi, self.signature
        )
    }
}
fn scalar(ty: &Term) -> bool {
    matches!(ty, Term::Primitive(_))
        && (intrinsic_pointer_value(ty) || *ty == Term::Primitive(Primitive::Void))
}
/// Only a representation/provenance check. This does not grant Copy, Drop or
/// ownership permission; the resource checker separately validates delivery.
pub(crate) fn effect_data_layout(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
    depth: usize,
) -> Result<bool, String> {
    if depth >= 128 {
        return Err("effect data layout depth exhausted".into());
    }
    bound_type(ty, fuel)?;
    if scalar(ty) {
        return Ok(true);
    }
    let fields = if let Some(fields) = tuple_fields(package, ty) {
        fields.to_vec()
    } else {
        match ty {
            Term::Record(fields) => fields.iter().map(|(_, ty)| ty.clone()).collect(),
            Term::Nominal {
                declaration,
                arguments,
            } => {
                let Some(package) = package else {
                    return Ok(false);
                };
                let mut fields = None;
                for definition in &package.structs {
                    *fuel = fuel
                        .checked_sub(1)
                        .ok_or("effect data declaration budget exhausted")?;
                    if definition.declaration == *declaration {
                        *fuel = fuel
                            .checked_sub(definition.fields.len())
                            .ok_or("effect data field budget exhausted")?;
                        for (_, field) in &definition.fields {
                            bound_type(field, fuel)?;
                            for argument in arguments {
                                bound_type(argument, fuel)?;
                            }
                        }
                        fields = Some(
                            definition
                                .instantiate_fields(arguments)
                                .map_err(|e| e.to_string())?
                                .into_iter()
                                .map(|(_, ty)| ty)
                                .collect(),
                        );
                        break;
                    }
                }
                if fields.is_none() {
                    for definition in &package.enums {
                        *fuel = fuel
                            .checked_sub(1)
                            .ok_or("effect data declaration budget exhausted")?;
                        if definition.declaration == *declaration {
                            *fuel = fuel
                                .checked_sub(definition.variants.len())
                                .ok_or("effect data variant budget exhausted")?;
                            for variant in &definition.variants {
                                for field in &variant.fields {
                                    bound_type(field, fuel)?;
                                    for argument in arguments {
                                        bound_type(argument, fuel)?;
                                    }
                                }
                            }
                            fields = Some(
                                definition
                                    .instantiate_variants(arguments)
                                    .map_err(|e| e.to_string())?
                                    .into_iter()
                                    .flat_map(|variant| variant.fields)
                                    .collect(),
                            );
                            break;
                        }
                    }
                }
                let Some(fields) = fields else {
                    return Ok(false);
                };
                fields
            }
            _ => return Ok(false),
        }
    };
    for field in fields {
        if !effect_data_layout(package, &field, fuel, depth + 1)? {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(super) fn charge_inputs(inputs: &[EffectInput], fuel: &mut usize) -> Result<(), String> {
    for input in inputs {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("effect memory premise budget exhausted")?;
        bound_type(&input.signature, fuel)?;
        for argument in &input.operation.arguments {
            bound_type(argument, fuel)?;
        }
    }
    Ok(())
}
pub(super) fn copy_inputs(
    inputs: &[EffectInput],
    fuel: &mut usize,
) -> Result<Vec<EffectInput>, String> {
    charge_inputs(inputs, fuel)?;
    Ok(inputs.to_vec())
}
pub(super) fn validate_inputs(
    package: &Package,
    inputs: &[EffectInput],
    fuel: &mut usize,
) -> Result<(), String> {
    for input in inputs {
        let Term::Function { parameters, result } = &input.signature else {
            return Err("effect memory premise requires an operation signature".into());
        };
        if !matches!(
            input.abi,
            nir::effects::Abi::Direct | nir::effects::Abi::DirectPassing
        ) || input.abi == nir::effects::Abi::DirectPassing
            && (!parameters.is_empty() || !scalar(result))
        {
            return Err("effect memory premise requires data-only direct delivery or a scalar direct-passing slot".into());
        }
        if !effect_data_layout(Some(package), result, fuel, 0)? {
            return Err("effect memory premise requires data-only direct delivery".into());
        }
        for parameter in parameters {
            if !effect_data_layout(Some(package), &parameter.ty, fuel, 0)? {
                return Err("effect memory premise requires data-only direct delivery".into());
            }
        }
        let mut declaration = None;
        for candidate in &package.effects {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("effect memory declaration budget exhausted")?;
            if candidate.declaration == input.operation.declaration {
                declaration = Some(candidate);
                break;
            }
        }
        let declaration = declaration.ok_or("effect memory operation is missing")?;
        if declaration.type_parameters.len() != input.operation.arguments.len() {
            return Err("effect memory instance argument count mismatch".into());
        }
        bound_type(&declaration.signature, fuel)?;
        let signature = declaration
            .signature
            .instantiate_types(&declaration.type_parameters, &input.operation.arguments)
            .map_err(|e| e.to_string())?;
        bound_type(&signature, fuel)?;
        if declaration.abi != input.abi || signature != input.signature {
            return Err("effect memory premise disagrees with its declaration".into());
        }
    }
    Ok(())
}

/// Infer a conditional footprint. The caller must establish every returned
/// effect input at the installation: data-only delivery and live, stable captured
/// storage whose writes cannot invalidate pointer provenance or ownership.
/// This API does not discharge those obligations or publish unconditional facts.
pub fn infer_with_effect_inputs(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    effects: &[EffectInput],
    fuel: &mut usize,
) -> Result<Summary, String> {
    let effects = copy_inputs(effects, fuel)?;
    validate_inputs(package, &effects, fuel)?;
    for value in inputs.values() {
        if !matches!(
            value,
            CallableResult::Known(_) | CallableResult::Candidates(_)
        ) {
            return Err("memory callable premise requires named code identities".into());
        }
    }
    let code = own_code_inputs(code_inputs, &[], &[], fuel)?;
    infer_context(
        package,
        declaration,
        arguments,
        inputs,
        &code,
        &effects,
        fuel,
    )
}

impl<'a> Infer<'a> {
    pub(super) fn bind_data_pattern(
        &mut self,
        pattern: &'a Pattern,
        ty: &Term,
        depth: usize,
    ) -> Result<(), String> {
        self.step(depth)?;
        match pattern {
            Pattern::Then { first, value, next } => {
                self.bind_data_pattern(first, ty, depth + 1)?;
                let computed_ty = self.ty(&value.ty)?;
                if !matches!(computed_ty, Term::Pointer(_)) && !Self::guard_value_type(&computed_ty)
                {
                    return Err(
                        "computed memory pattern needs independent result provenance".into(),
                    );
                }
                // The computed value is not a projection of the subject. Walk
                // it normally to retain call/read footprints; resource analysis
                // separately checks candidate observation and rollback safety.
                self.walk(value, depth + 1)?;
                if matches!(computed_ty, Term::Pointer(_)) {
                    let targets = self.optional_target(value, depth + 1)?;
                    self.bind_match_subject(next, &targets, true, &computed_ty, true, depth + 1)?;
                    self.bind_owned_callees(
                        next,
                        &computed_ty,
                        &BTreeMap::new(),
                        &BTreeMap::new(),
                        depth + 1,
                    )
                } else if matches!(computed_ty, Term::Function { .. }) {
                    let named = self.value_callees(value)?;
                    let code = self.local_fields_for(value)?;
                    self.bind_owned_callees(next, &computed_ty, &named, &code, depth + 1)
                } else {
                    self.bind_data_pattern(next, &computed_ty, depth + 1)
                }
            }
            Pattern::Tuple(patterns)
            | Pattern::Record {
                fields: patterns, ..
            }
            | Pattern::AnonymousRecord {
                fields: patterns, ..
            } => {
                match pattern {
                    Pattern::Tuple(_) if tuple_fields(self.package, ty).is_none() => {
                        return Err("data tuple pattern requires a tuple layout".into());
                    }
                    Pattern::Record { declaration, .. } if !matches!(ty,Term::Nominal {declaration:actual,..} if actual==declaration) =>
                    {
                        return Err("data record pattern declaration mismatch".into());
                    }
                    Pattern::AnonymousRecord { names, .. } if !matches!(ty,Term::Record(fields) if names.len()==fields.len() && names.iter().zip(fields).all(|(name,(expected,_))|name==expected)) =>
                    {
                        return Err("data record pattern labels mismatch".into());
                    }
                    _ => (),
                }
                let fields = owned_fields(self.package, ty, self.fuel)?;
                if fields.len() != patterns.len() {
                    return Err("handler memory pattern arity mismatch".into());
                }
                for (pattern, ty) in patterns.iter().zip(fields) {
                    self.bind_data_pattern(pattern, &ty, depth + 1)?;
                }
                Ok(())
            }
            Pattern::Variant {
                declaration,
                index,
                fields: patterns,
            } => {
                let Term::Nominal {
                    declaration: actual,
                    arguments,
                } = ty
                else {
                    return Err("handler memory variant type mismatch".into());
                };
                if declaration != actual {
                    return Err("handler memory variant declaration mismatch".into());
                }
                let definition = self
                    .package
                    .and_then(|p| p.enums.iter().find(|item| item.declaration == *declaration))
                    .ok_or("handler memory variant metadata missing")?;
                *self.fuel = self
                    .fuel
                    .checked_sub(definition.variants.len())
                    .ok_or("handler memory variant budget exhausted")?;
                for variant in &definition.variants {
                    for field in &variant.fields {
                        bound_type(field, self.fuel)?;
                        for argument in arguments {
                            bound_type(argument, self.fuel)?;
                        }
                    }
                }
                let variants = definition
                    .instantiate_variants(arguments)
                    .map_err(|e| e.to_string())?;
                let fields = &variants
                    .get(*index)
                    .ok_or("handler memory variant index invalid")?
                    .fields;
                if fields.len() != patterns.len() {
                    return Err("handler memory variant arity mismatch".into());
                }
                for (pattern, ty) in patterns.iter().zip(fields) {
                    self.bind_data_pattern(pattern, ty, depth + 1)?;
                }
                Ok(())
            }
            Pattern::Guard { pattern, condition } => {
                self.bind_data_pattern(pattern, ty, depth + 1)?;
                self.walk_guard(condition, depth + 1)
            }
            Pattern::Remap { pattern, bindings } => {
                self.bind_data_pattern(pattern, ty, depth + 1)?;
                let environment = self.save_environment()?;
                for (from, to) in bindings {
                    let ty = self
                        .locals
                        .get(from)
                        .ok_or("handler memory pattern remap is missing")?;
                    bound_type(ty, self.fuel)?;
                    self.locals.insert(*to, ty.clone());
                    if let Some(targets) = environment.pointers.get(from) {
                        if let Some(targets) = targets {
                            self.target_work(targets)?;
                        }
                        self.set_pointer(*to, targets.clone())?;
                    }
                }
                Ok(())
            }
            Pattern::Binding { binding, inner } => {
                bound_type(ty, self.fuel)?;
                self.locals.insert(*binding, ty.clone());
                self.bind_data_pattern(inner, ty, depth + 1)
            }
            Pattern::Or(patterns) => {
                let base = self.save_environment()?;
                let mut joined = None;
                for pattern in patterns {
                    self.environment = self.clone_environment(&base)?;
                    self.bind_data_pattern(pattern, ty, depth + 1)?;
                    let next = self.save_environment()?;
                    joined = Some(if let Some(previous) = joined {
                        self.join_environment(previous, next)?
                    } else {
                        next
                    });
                }
                self.environment = joined.unwrap_or(base);
                Ok(())
            }
            _ => self.bind_match_subject(pattern, &None, false, ty, false, depth + 1),
        }
    }
    fn walk_handler(
        &mut self,
        expr: &'a Expr,
        computation: &'a Expr,
        handlers: &'a [nir::program::EffectHandler],
        outer: &[nir::effects::Operation],
        depth: usize,
    ) -> Result<(), String> {
        self.step(depth)?;
        let mut outer_row = vec![];
        for operation in outer {
            let operation = self.effect_operation(operation)?;
            self.require_effect(&operation)?;
            outer_row.push(operation);
        }
        if outer_row != self.effect_row {
            return Err("handler memory outer row mismatch".into());
        }
        let base = self.save_environment()?;
        let mut inputs = copy_inputs(&self.round.effects, self.fuel)?;
        let mut row = vec![];
        let mut pointer_captures = BTreeSet::new();
        for handler in handlers {
            self.step(depth)?;
            let operation = self.effect_operation(&handler.operation)?;
            let signature = self.ty(&handler.signature)?;
            let input = EffectInput {
                operation: operation.clone(),
                abi: handler.abi,
                signature: signature.clone(),
            };
            validate_inputs(
                self.package.ok_or("handler memory requires a package")?,
                std::slice::from_ref(&input),
                self.fuel,
            )?;
            if let Some(slot) = handler.fallback {
                if outer_row.get(slot) != Some(&operation) {
                    return Err("handler memory fallback row mismatch".into());
                }
                let previous = self.require_effect(&operation)?;
                if previous.signature != signature || previous.abi != handler.abi {
                    return Err("handler memory fallback premise mismatch".into());
                }
            }
            let Term::Function { parameters, .. } = &signature else {
                unreachable!()
            };
            let argument_ty = Term::Tuple(
                parameters
                    .iter()
                    .map(|parameter| parameter.ty.clone())
                    .collect(),
            );
            for arm in &handler.arms {
                self.environment = self.clone_environment(&base)?;
                for binding in nir::free_bindings::collect_arm(arm, self.fuel)? {
                    let ty = self
                        .locals
                        .get(&binding)
                        .ok_or("handler memory capture is not in scope")?;
                    bound_type(ty, self.fuel)?;
                    if let Term::Pointer(subject) = ty {
                        if !scalar(subject) {
                            return Err(
                                "handler memory pointer capture needs scalar storage".into()
                            );
                        }
                        pointer_captures.insert(binding);
                    } else if !effect_data_layout(self.package, ty, self.fuel, 0)? {
                        return Err(
                            "handler memory capture needs a data-only or stable pointer contract"
                                .into(),
                        );
                    }
                }
                let (pattern, mut guards) = outer_guards(&arm.pattern);
                guards.extend(arm.guard.as_ref());
                self.bind_data_pattern(pattern, &argument_ty, depth + 1)?;
                let mut reachable = true;
                for guard in guards {
                    self.walk_guard(guard, depth + 1)?;
                    if constant_bool(guard, 0) == Some(false) {
                        reachable = false;
                        break;
                    }
                }
                if reachable {
                    self.walk(&arm.body, depth + 1)?;
                }
            }
            // Equivalent premises are a set, not an ever-growing activation
            // stack. Recursive reinstalls must reach a finite worklist key.
            *self.fuel = self
                .fuel
                .checked_sub(inputs.len() + 1)
                .ok_or("handler memory premise budget exhausted")?;
            if !inputs.contains(&input) {
                inputs.push(input);
            }
            row.push(operation);
        }
        validate_borrowed_capture_storage(false, self.package, expr, &pointer_captures, self.fuel)?;
        row.extend(outer_row);
        let Term::Computation { effects, result } = self.ty(&computation.ty)? else {
            return Err("handler memory subject must be a computation".into());
        };
        if row != effects || self.ty(&expr.ty)? != *result {
            return Err("handler memory completion mismatch".into());
        }
        if !effect_data_layout(self.package, &result, self.fuel, 0)? {
            return Err("handler memory completion needs data-only delivery".into());
        }
        charge_inputs(&inputs, self.fuel)?;
        inputs.sort_by_cached_key(ToString::to_string);
        self.environment = self.clone_environment(&base)?;
        let previous = std::mem::replace(&mut self.round.effects, inputs);
        let walked = self.walk(computation, depth + 1);
        self.round.effects = previous;
        walked
    }
    fn effect_operation(
        &mut self,
        operation: &nir::effects::Operation,
    ) -> Result<nir::effects::Operation, String> {
        let mut arguments = vec![];
        for argument in &operation.arguments {
            // Operation labels use exact instantiated type identity. A layout
            // view inferred for memory access must not rewrite a label's head.
            bound_type(argument, self.fuel)?;
            let argument = argument
                .instantiate_types(self.parameters, self.arguments)
                .map_err(|e| e.to_string())?;
            bound_type(&argument, self.fuel)?;
            arguments.push(argument);
        }
        Ok(nir::effects::Operation {
            declaration: operation.declaration,
            arguments,
        })
    }
    fn require_effect(
        &mut self,
        operation: &nir::effects::Operation,
    ) -> Result<&EffectInput, String> {
        for input in &self.round.effects {
            *self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or("effect memory premise lookup budget exhausted")?;
            bound_type(&input.signature, self.fuel)?;
            for argument in &input.operation.arguments {
                bound_type(argument, self.fuel)?;
            }
            if &input.operation == operation {
                return Ok(input);
            }
        }
        Err("effect request requires an installed handler memory premise".into())
    }
    pub(super) fn effect_completion(&mut self, ty: &Term) -> Result<Term, String> {
        let ty = self.type_view(ty)?;
        if let Term::Computation { effects, result } = &ty {
            for operation in effects {
                self.require_effect(operation)?;
            }
            // Pointer/closure delivery needs a result-provenance lattice across
            // suspension. Do not erase those dependencies behind a wrapper.
            if !effect_data_layout(self.package, result, self.fuel, 0)? {
                return Err("effect memory completion requires data-only delivery".into());
            }
            return Ok(result.as_ref().clone());
        }
        Ok(ty)
    }
    pub(super) fn walk_effect(&mut self, expr: &'a Expr, depth: usize) -> Result<(), String> {
        self.step(depth)?;
        match &expr.kind {
            K::EffectHandle {
                computation,
                handlers,
                outer,
            } => self.walk_handler(expr, computation, handlers, outer, depth + 1)?,
            K::EffectResume(value) => {
                let ty = self.ty(&value.ty)?;
                if !effect_data_layout(self.package, &ty, self.fuel, 0)? {
                    return Err("handler memory resume needs data-only delivery".into());
                }
                self.walk(value, depth + 1)?;
            }
            K::EffectRequest {
                operation,
                abi,
                allowed_index,
                signature,
                arguments,
            } => {
                let operation = self.effect_operation(operation)?;
                if self.effect_row.get(*allowed_index) != Some(&operation) {
                    return Err("effect memory request row mismatch".into());
                }
                let signature = self.ty(signature)?;
                let input = self.require_effect(&operation)?;
                if input.signature != signature || input.abi != *abi {
                    return Err("effect memory request disagrees with its premise".into());
                }
                let Term::Function { parameters, result } = signature else {
                    unreachable!()
                };
                if parameters.len() != arguments.len() || self.ty(&expr.ty)? != *result {
                    return Err("effect memory request signature mismatch".into());
                }
                for (argument, parameter) in arguments.iter().zip(parameters) {
                    let ty = self.ty(&argument.ty)?;
                    if ty != parameter.ty && ty != Term::Primitive(Primitive::Never) {
                        return Err("effect memory request argument mismatch".into());
                    }
                    self.walk(argument, depth + 1)?;
                }
                // Handler predicate failure/lexical escape is not normal return.
                // Keep possible rejection even though delivery introduces no aliases.
                self.may_reject = true;
            }
            K::EffectComputation { effects, body } => {
                let ty = self.ty(&expr.ty)?;
                self.effect_completion(&ty)?;
                let Term::Computation {
                    effects: declared,
                    result,
                } = ty
                else {
                    return Err("effect memory boundary requires a computation type".into());
                };
                let mut row = vec![];
                for operation in effects {
                    row.push(self.effect_operation(operation)?);
                }
                let body_ty = self.ty(&body.ty)?;
                if row != declared
                    || body_ty != *result && body_ty != Term::Primitive(Primitive::Never)
                {
                    return Err("effect memory boundary signature mismatch".into());
                }
                let previous = std::mem::replace(&mut self.effect_row, row);
                let result = self.walk(body, depth + 1);
                self.effect_row = previous;
                result?;
            }
            K::EffectRun { computation, slots } => {
                let ty = self.ty(&computation.ty)?;
                let Term::Computation { effects, result } = &ty else {
                    return Err("effect memory run requires a computation type".into());
                };
                self.effect_completion(&ty)?;
                if effects.len() != slots.len()
                    || effects
                        .iter()
                        .zip(slots)
                        .any(|(operation, slot)| self.effect_row.get(*slot) != Some(operation))
                    || self.ty(&expr.ty)? != **result
                {
                    return Err("effect memory run routing mismatch".into());
                }
                self.walk(computation, depth + 1)?;
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}

/// Capture-relative memory accesses together with their handler assumptions.
/// Neither the summary nor those assumptions prove actual pointer validity.
#[derive(Clone, Debug, PartialEq)]
pub struct ConditionalClosureSummary {
    pub effect_inputs: Vec<EffectInput>,
    pub memory: ArgumentSummary,
}
impl fmt::Display for ConditionalClosureSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(conditional-closure-memory (effect-inputs")?;
        for input in &self.effect_inputs {
            write!(f, " {input}")?;
        }
        write!(f, ") {})", self.memory)
    }
}
/// Infer only under explicit handler premises, retaining them on the returned
/// artifact. Pointer captures become virtual inputs after ordinary parameters;
/// their real origins must still be checked at each invocation.
pub fn infer_closure_with_effect_inputs(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<CallablePlace, CallableResult>,
    prepared_captures: &PreparedCaptures,
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    effects: &[EffectInput],
    fuel: &mut usize,
) -> Result<ConditionalClosureSummary, String> {
    let effect_inputs = copy_inputs(effects, fuel)?;
    validate_inputs(package, &effect_inputs, fuel)?;
    let memory = infer_closure_with_borrowed_effect_context(
        package,
        definition,
        callees,
        prepared_captures,
        inputs,
        code_inputs,
        &effect_inputs,
        fuel,
    )?;
    Ok(ConditionalClosureSummary {
        effect_inputs,
        memory,
    })
}
