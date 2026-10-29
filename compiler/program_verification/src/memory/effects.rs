//! Conditional memory inference for direct effect delivery. Captured storage is
//! checked separately by the installer. An explicit identity-transfer premise
//! can carry an operand's pointer and callable provenance; it grants neither dereference nor
//! lifetime extension. Unreturned alias-bearing inputs require checked Copy
//! obligations or explicit owned-input admission, including when the result is independent data.
use super::*;

/// Conditional accesses through a pointer supplied in an operation input.
/// Paths address owned input fields, not storage behind another pointer.
/// Variant-qualified paths apply only while the corresponding payload exists.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EffectParameterAccess {
    pub parameter: usize,
    pub input_fields: Vec<usize>,
    pub read_value: bool,
    pub write: bool,
}
impl fmt::Display for EffectParameterAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(parameter-access (parameter {}) (input-fields",
            self.parameter
        )?;
        display_owned_path(&self.input_fields, f)?;
        write!(f, ") (read {}) (write {}))", self.read_value, self.write)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EffectInput {
    pub operation: nir::effects::Operation,
    pub abi: nir::effects::Abi,
    pub signature: Term,
    /// Possible complete input origins; grants no dereference or lifetime extension.
    pub forwarded_parameters: Option<BTreeSet<usize>>,
    /// Conditional checked Copy obligations, including cleanup compatibility.
    pub copy_parameters: BTreeSet<usize>,
    /// All inputs are consumed by a resource-checked direct handler with a data-only result.
    /// This premise grants no Copy permission.
    pub owned_inputs: bool,
    pub parameter_accesses: BTreeSet<EffectParameterAccess>,
}
impl EffectInput {
    pub(crate) fn charge_parameter_accesses(&self, fuel: &mut usize) -> Result<(), String> {
        for access in &self.parameter_accesses {
            *fuel = fuel
                .checked_sub(access.input_fields.len().saturating_add(1))
                .ok_or("effect parameter access budget exhausted")?;
        }
        Ok(())
    }
}
impl fmt::Display for EffectInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(effect-input {} (abi {}) (signature {}) (capture-contract stable-scalar-storage) (forwarded-parameters",
            self.operation, self.abi, self.signature
        )?;
        if let Some(parameters) = &self.forwarded_parameters {
            for parameter in parameters {
                write!(f, " {parameter}")?;
            }
        }
        f.write_str(") (copy-inputs")?;
        for parameter in &self.copy_parameters {
            write!(f, " {parameter}")?;
        }
        write!(
            f,
            ") (owned-inputs {}) (parameter-accesses",
            self.owned_inputs
        )?;
        for access in &self.parameter_accesses {
            write!(f, " {access}")?;
        }
        f.write_str("))")
    }
}
fn access_pointee(
    package: Option<&Package>,
    ty: &Term,
    fields: &[usize],
    fuel: &mut usize,
) -> Result<Term, String> {
    let layout = pointer_input_layout(package, ty, fuel)?;
    let leaf = layout
        .into_iter()
        .find(|leaf| leaf.input_fields == fields)
        .ok_or("effect parameter access does not select a pointer input")?;
    if !scalar(&leaf.pointee) {
        return Err("effect parameter access requires scalar storage".into());
    }
    Ok(leaf.pointee)
}
fn scalar(ty: &Term) -> bool {
    matches!(ty, Term::Primitive(_))
        && (intrinsic_pointer_value(ty) || *ty == Term::Primitive(Primitive::Void))
}
/// Values whose origins can be transported unchanged by the identity premise.
/// This authorizes no call or dereference of the delivered value.
pub(crate) fn effect_transfer_layout(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<bool, String> {
    if !matches!(
        ty,
        Term::Pointer(_)
            | Term::Optional(_)
            | Term::Function { .. }
            | Term::UnsafeFunction(_)
            | Term::Closure { .. }
            | Term::Record(_)
            | Term::Nominal { .. }
    ) && tuple_fields(package, ty).is_none()
    {
        return Ok(false);
    }
    // Validate every field under the same bounded layout rules used by the
    // provenance maps. Identity transfer never interprets or rebuilds fields.
    pointer_input_layout(package, ty, fuel)?;
    Ok(true)
}
/// Pointer-free installed values; Copy permission is checked separately.
pub(crate) fn data_passing_signature(
    package: Option<&Package>,
    signature: &Term,
    fuel: &mut usize,
) -> Result<bool, String> {
    let Term::Function { parameters, result } = signature else {
        return Ok(false);
    };
    if !parameters.is_empty() {
        return Ok(false);
    }
    effect_data_layout(package, result, fuel, 0)
}

/// Only a representation/provenance check. This does not grant Copy, Drop or
/// ownership permission; the resource checker separately validates delivery.
pub(crate) fn effect_data_layout(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
    depth: usize,
) -> Result<bool, String> {
    effect_storage_layout(package, ty, fuel, depth, false)
}

/// Layout only: borrowing code-bearing captures also requires complete code
/// candidates, stable storage and all-exit ownership preservation.
pub(crate) fn effect_borrowed_code_layout(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<bool, String> {
    effect_storage_layout(package, ty, fuel, 0, true)
}

// Code-bearing layouts need separate candidate and storage evidence.
// Operation inputs continue to use the stricter data-only predicate.
fn effect_storage_layout(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
    depth: usize,
    allow_code: bool,
) -> Result<bool, String> {
    if depth >= 128 {
        return Err("effect data layout depth exhausted".into());
    }
    bound_type(ty, fuel)?;
    if scalar(ty) || (allow_code && matches!(ty, Term::Function { .. } | Term::UnsafeFunction(_))) {
        return Ok(true);
    }
    let fields = if let Some(fields) = tuple_fields(package, ty) {
        fields.to_vec()
    } else {
        match ty {
            Term::Optional(payload) => {
                if **payload == Term::Primitive(Primitive::Never) {
                    vec![]
                } else {
                    vec![(**payload).clone()]
                }
            }
            Term::Closure { environment, .. } if allow_code => environment.clone(),
            Term::Record(fields) => fields.iter().map(|(_, ty)| ty.clone()).collect(),
            Term::ErrorResult { errors, result } => {
                // Inspect all inhabited alternatives; an error-only result has
                // no success payload. This does not grant Copy to either arm.
                let mut fields = errors.clone();
                if **result != Term::Primitive(Primitive::Never) {
                    fields.push((**result).clone());
                }
                fields
            }
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
                if fields.is_none() {
                    for definition in &package.newtypes {
                        *fuel = fuel
                            .checked_sub(1)
                            .ok_or("effect data newtype lookup budget exhausted")?;
                        if definition.declaration == *declaration {
                            // Inspect the checked representation edge for layout
                            // only. The nominal result still needs its own Copy proof.
                            fields = Some(vec![
                                definition
                                    .representation_budgeted(arguments, &[], fuel)
                                    .map_err(|error| error.to_string())?,
                            ]);
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
        if !effect_storage_layout(package, &field, fuel, depth + 1, allow_code)? {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(super) fn charge_inputs(inputs: &[EffectInput], fuel: &mut usize) -> Result<(), String> {
    for input in inputs {
        input.charge_parameter_accesses(fuel)?;
        *fuel = fuel
            .checked_sub(1)
            .ok_or("effect memory premise budget exhausted")?;
        *fuel = fuel
            .checked_sub(
                input
                    .forwarded_parameters
                    .as_ref()
                    .map_or(0, BTreeSet::len)
                    .saturating_add(input.copy_parameters.len()),
            )
            .ok_or("effect transfer premise budget exhausted")?;
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
            && !data_passing_signature(Some(package), &input.signature, fuel)?
        {
            return Err("effect memory premise requires data-only direct delivery or a pointer-free direct-passing slot".into());
        }
        if let Some(indices) = &input.forwarded_parameters {
            if indices.is_empty() {
                return Err("effect transfer premise requires nonempty origins".into());
            }
            *fuel = fuel
                .checked_sub(indices.len())
                .ok_or("effect transfer validation budget exhausted")?;
            for parameter in indices {
                if input.abi != nir::effects::Abi::Direct
                    || parameters.get(*parameter).is_none_or(|p| p.ty != **result)
                {
                    return Err(
                        "effect transfer premise requires matching input/result domains".into(),
                    );
                }
            }
        }
        *fuel = fuel
            .checked_sub(input.copy_parameters.len())
            .ok_or("effect copy premise budget exhausted")?;
        if input
            .copy_parameters
            .iter()
            .any(|index| *index >= parameters.len())
        {
            return Err("effect copy premise parameter is out of range".into());
        }
        input.charge_parameter_accesses(fuel)?;
        for access in &input.parameter_accesses {
            if !access.read_value && !access.write {
                return Err("effect parameter access must read or write".into());
            }
            let parameter = parameters
                .get(access.parameter)
                .ok_or("effect parameter access index out of range")?;
            access_pointee(Some(package), &parameter.ty, &access.input_fields, fuel)?;
        }
        let identity_transfer = input.forwarded_parameters.is_some()
            && (effect_transfer_layout(Some(package), result, fuel)?
                || effect_data_layout(Some(package), result, fuel, 0)?);
        let data_result = effect_data_layout(Some(package), result, fuel, 0)?;
        if input.owned_inputs && (input.abi != nir::effects::Abi::Direct || !data_result) {
            return Err(
                "owned effect inputs require direct delivery with a data-only result".into(),
            );
        }
        if !identity_transfer && !data_result {
            return Err("effect memory premise requires data-only direct delivery".into());
        }
        for (index, parameter) in parameters.iter().enumerate() {
            if identity_transfer
                && input
                    .forwarded_parameters
                    .as_ref()
                    .is_some_and(|indices| indices.contains(&index))
            {
                continue;
            }
            // Direct requests can transfer owned inputs into an admitted
            // handler. Memory premises describe accesses, not Copy permission;
            // ownership and cleanup are checked when publishing the premise.
            if (input.copy_parameters.contains(&index) || input.owned_inputs)
                && effect_transfer_layout(Some(package), &parameter.ty, fuel)?
            {
                continue;
            }
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
            .instantiate_parameters(
                &declaration.parameter_binders().map_err(|e| e.to_string())?,
                &input.operation.arguments,
            )
            .map_err(|e| e.to_string())?;
        bound_type(&signature, fuel)?;
        if declaration.abi != input.abi || signature != input.signature {
            return Err("effect memory premise disagrees with its declaration".into());
        }
    }
    Ok(())
}

/// Infer a conditional footprint. The caller must establish every returned
/// effect input at the installation: data-only delivery or proven identity
/// transfer, checked representation-copy permission (including cleanup
/// compatibility) for every copy input, and live, stable captured storage whose
/// writes cannot invalidate pointer provenance or ownership.
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

impl<'a> Infer<'a, '_> {
    fn join_effect_value(&mut self, destination: &Expr, value: &Expr) -> Result<(), String> {
        let key = destination as *const Expr as usize;
        let pointers = self.pointer_fields_for(value)?;
        let code = self.local_fields_for(value)?;
        let callees = self.value_callees(value)?;
        let callable = self.callee_key(value)?;
        let previous = self.aggregate_pointers.remove(&key).unwrap_or_default();
        let pointers = self.join_pointer_fields(previous, pointers)?;
        let previous = self.local_code_fields.remove(&key).unwrap_or_default();
        let code = self.join_local_fields(previous, code)?;
        let previous = self.aggregate_callees.remove(&key).unwrap_or_default();
        let callees = self.join_callee_fields(previous, callees)?;
        let mut previous = self
            .callable_results
            .remove(&key)
            .unwrap_or(CallableResult::Unknown);
        previous.join(&callable, self.fuel)?;
        self.aggregate_pointers.insert(key, pointers);
        self.local_code_fields.insert(key, code);
        self.aggregate_callees.insert(key, callees);
        self.callable_results.insert(key, previous);
        Ok(())
    }
    fn forward_effect_value(&mut self, destination: &Expr, value: &Expr) -> Result<(), String> {
        let key = destination as *const Expr as usize;
        let pointers = self.pointer_fields_for(value)?;
        let code = self.local_fields_for(value)?;
        let callees = self.value_callees(value)?;
        let callable = self.callee_key(value)?;
        self.aggregate_pointers.insert(key, pointers);
        self.local_code_fields.insert(key, code);
        self.aggregate_callees.insert(key, callees);
        self.callable_results.insert(key, callable);
        Ok(())
    }
    pub(super) fn bind_data_pattern(
        &mut self,
        pattern: &'a Pattern,
        ty: &Term,
        depth: usize,
    ) -> Result<(), String> {
        self.step(depth)?;
        match pattern {
            Pattern::OptionalAbsent | Pattern::OptionalPresent(_) | Pattern::ErrorResult { .. } => {
                // Data-only classification includes both success and error
                // payloads. Retain the selected row label and payload type
                // instead of falling back to scalar/pointer binding.
                let (_, patterns, fields) =
                    owned_variant_pattern(self.package, pattern, ty, self.fuel)?;
                for (pattern, field) in patterns.iter().zip(&fields) {
                    self.bind_data_pattern(pattern, field, depth + 1)?;
                }
                Ok(())
            }
            Pattern::Then { first, value, next } => {
                self.bind_data_pattern(first, ty, depth + 1)?;
                let computed_ty = self.ty(&value.ty)?;
                if !matches!(computed_ty, Term::Pointer(_))
                    && !Self::guard_value_type(self.package, &computed_ty)
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
                } else if matches!(computed_ty, Term::Function { .. } | Term::UnsafeFunction(_)) {
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
            Pattern::Binding { binding, inner } | Pattern::Observe { binding, inner } => {
                bound_type(ty, self.fuel)?;
                self.locals.insert(*binding, ty.clone());
                self.bind_data_pattern(inner, ty, depth + 1)?;
                if matches!(pattern, Pattern::Observe { .. }) {
                    self.forget_pattern_alias(*binding)?;
                }
                Ok(())
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
        let mut code_captures = BTreeSet::new();
        let mut stable_writes = BTreeSet::new();
        for handler in handlers {
            self.step(depth)?;
            let operation = self.effect_operation(&handler.operation)?;
            let signature = self.ty(&handler.signature)?;
            let Term::Function { parameters, .. } = &signature else {
                return Err("handler memory requires an operation signature".into());
            };
            // Memory analysis computes footprints under ordinary resource
            // validation. Copy obligations must be discharged by the installer.
            let forwarded_parameters = super::super::effect_transfer::forwarded_parameters_in(
                handler,
                &self.round.effects,
                self.fuel,
            )?;
            let mut copy_parameters = BTreeSet::new();
            for (index, parameter) in parameters.iter().enumerate() {
                if !forwarded_parameters
                    .as_ref()
                    .is_some_and(|origins| origins.contains(&index))
                    && effect_transfer_layout(self.package, &parameter.ty, self.fuel)?
                {
                    copy_parameters.insert(index);
                }
            }
            let mut input = EffectInput {
                parameter_accesses: Default::default(),
                copy_parameters,
                owned_inputs: false,
                forwarded_parameters,
                operation: operation.clone(),
                abi: handler.abi,
                signature: signature.clone(),
            };
            if let Some(slot) = handler.fallback {
                if outer_row.get(slot) != Some(&operation) {
                    return Err("handler memory fallback row mismatch".into());
                }
                let previous = self.require_effect(&operation)?;
                if previous.signature != signature || previous.abi != handler.abi {
                    return Err("handler memory fallback premise mismatch".into());
                }
                let origins = previous.forwarded_parameters.clone();
                input.forwarded_parameters =
                    super::super::effect_transfer::with_handler_fallback_in(
                        handler,
                        origins.as_ref(),
                        &self.round.effects,
                        self.fuel,
                    )?;
            }
            validate_inputs(
                self.package.ok_or("handler memory requires a package")?,
                std::slice::from_ref(&input),
                self.fuel,
            )?;
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
                let mut captures = Vec::new();
                let mut captured_code = BTreeMap::new();
                let mut prepared_code = BTreeMap::new();
                for binding in nir::free_bindings::collect_arm(arm, self.fuel)? {
                    let ty = self
                        .locals
                        .get(&binding)
                        .ok_or("handler memory capture is not in scope")?;
                    bound_type(ty, self.fuel)?;
                    let ty = ty.clone();
                    captures.push((binding, ty.clone()));
                    let layout = pointer_input_layout(self.package, &ty, self.fuel)?;
                    if !layout.is_empty() {
                        pointer_captures.insert(binding);
                    } else if !effect_storage_layout(self.package, &ty, self.fuel, 0, true)? {
                        return Err(
                            "handler memory capture needs tracked data, code or stable pointers"
                                .into(),
                        );
                    }
                    let capture = nir::program::Capture {
                        binding,
                        ty,
                        value: None,
                    };
                    if self.snapshot_capture_code(
                        &capture,
                        &mut captured_code,
                        &mut prepared_code,
                    )? {
                        return Err(
                            "handler code capture requires complete candidate evidence".into()
                        );
                    }
                    if captured_code.keys().any(|place| place.binding == binding)
                        || prepared_code.keys().any(|place| place.binding == binding)
                    {
                        // Code identity must remain stable throughout the installed
                        // computation, including suspended outer handler calls.
                        code_captures.insert(binding);
                    }
                }
                if !code_captures.is_empty() {
                    validate_capture_storage(
                        false,
                        false,
                        self.package,
                        self.scope,
                        &code_captures,
                        self.fuel,
                    )?;
                    validate_borrowed_capture_storage(
                        false,
                        self.package,
                        expr,
                        &code_captures,
                        self.fuel,
                    )?;
                }
                if let Some(summary) = direct_handler_accesses_with_code(
                    self.package,
                    handler,
                    arm,
                    &captures,
                    &captured_code,
                    &prepared_code,
                    &self.round.effects,
                    &outer_row,
                    self.fuel,
                )? {
                    stable_writes.extend(stationary_handler_write_sites(
                        self.package,
                        arm,
                        parameters.len(),
                        &captures,
                        &summary.memory,
                        self.fuel,
                    )?);
                    self.may_reject |= summary.memory.may_reject;
                    self.may_run_cleanup |= summary.memory.may_run_cleanup;
                    for (binding, access) in summary.capture_accesses {
                        let targets = if access.target.input_fields.is_empty() {
                            self.environment.pointers.get(&binding)
                        } else {
                            self.environment.field_pointers.get(&CallablePlace {
                                binding,
                                fields: access.target.input_fields.clone(),
                            })
                        }
                        .cloned()
                        .flatten()
                        .ok_or("handler capture access requires known pointer provenance")?;
                        let mut projected = BTreeSet::new();
                        for mut target in targets {
                            self.step(
                                target.path_cost().saturating_add(access.target.path_cost()),
                            )?;
                            target.append_resolved_path(&access.target)?;
                            projected.insert(target);
                        }
                        self.record_access(projected, access.write, access.read_value, access.ty)?;
                    }
                    input.parameter_accesses.extend(summary.accesses);
                    continue;
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
        validate_capture_storage_except(
            true,
            false,
            self.package,
            expr,
            &pointer_captures,
            self.fuel,
            &stable_writes,
        )?;
        row.extend(outer_row);
        let Term::Computation { effects, result } = self.ty(&computation.ty)? else {
            return Err("handler memory subject must be a computation".into());
        };
        if row != effects || self.ty(&expr.ty)? != *result {
            return Err("handler memory completion mismatch".into());
        }
        if !effect_transfer_layout(self.package, &result, self.fuel)?
            && !effect_data_layout(self.package, &result, self.fuel, 0)?
        {
            return Err("handler memory completion needs data-only delivery".into());
        }
        charge_inputs(&inputs, self.fuel)?;
        inputs.sort_by_cached_key(ToString::to_string);
        self.environment = self.clone_environment(&base)?;
        let previous = std::mem::replace(&mut self.round.effects, inputs);
        let walked = (|| {
            self.walk(computation, depth + 1)?;
            self.forward_effect_value(expr, computation)?;
            if self.local_pointer_value(&result)? {
                let targets = self.optional_target(computation, depth + 1)?;
                self.expression_results
                    .insert(expr as *const Expr as usize, targets);
            }
            Ok(())
        })();
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
                .instantiate_parameters(&self.declarations, self.arguments)
                .map_err(|e| e.to_string())?;
            bound_type(&argument, self.fuel)?;
            arguments.push(argument);
        }
        Ok(nir::effects::Operation {
            declaration: operation.declaration,
            arguments,
        })
    }
    // A worklist can retain several scoped providers for one operation. Until
    // its memory key distinguishes occurrences, conservatively join their
    // origins/obligations; picking the first premise would underapproximate.
    fn require_effect(
        &mut self,
        operation: &nir::effects::Operation,
    ) -> Result<EffectInput, String> {
        let mut matched: Option<EffectInput> = None;
        for input in &self.round.effects {
            *self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or("effect memory premise lookup budget exhausted")?;
            *self.fuel = self
                .fuel
                .checked_sub(
                    input
                        .forwarded_parameters
                        .as_ref()
                        .map_or(0, BTreeSet::len)
                        .saturating_add(input.copy_parameters.len()),
                )
                .ok_or("effect transfer lookup budget exhausted")?;
            input.charge_parameter_accesses(self.fuel)?;
            bound_type(&input.signature, self.fuel)?;
            for argument in &input.operation.arguments {
                bound_type(argument, self.fuel)?;
            }
            if &input.operation == operation {
                if let Some(previous) = &mut matched {
                    if previous.abi != input.abi || previous.signature != input.signature {
                        return Err("conflicting effect memory premises".into());
                    }
                    previous.forwarded_parameters = super::super::effect_transfer::with_fallback(
                        previous.forwarded_parameters.take(),
                        input.forwarded_parameters.as_ref(),
                        self.fuel,
                    )?;
                    previous
                        .copy_parameters
                        .extend(input.copy_parameters.iter().copied());
                    previous
                        .parameter_accesses
                        .extend(input.parameter_accesses.iter().cloned());
                } else {
                    matched = Some(input.clone());
                }
            }
        }
        if let Some(input) = matched {
            return Ok(input);
        }
        // A pointer-free direct-passing request reads an already supplied value; it
        // does not invoke the provider or access its captured storage. Derive
        // only that request-side footprint from the concrete declaration. The
        // installer must still check provider effects and Copy/Drop delivery.
        // Ordinary direct handlers, pointers and owning payloads need premises.
        if let Some(package) = self.package {
            for declaration in &package.effects {
                *self.fuel = self
                    .fuel
                    .checked_sub(1)
                    .ok_or("effect memory declaration lookup budget exhausted")?;
                if declaration.declaration != operation.declaration {
                    continue;
                }
                if declaration.abi != nir::effects::Abi::DirectPassing {
                    break;
                }
                bound_type(&declaration.signature, self.fuel)?;
                for argument in &operation.arguments {
                    bound_type(argument, self.fuel)?;
                }
                let signature = declaration
                    .signature
                    .instantiate_parameters(
                        &declaration.parameter_binders().map_err(|e| e.to_string())?,
                        &operation.arguments,
                    )
                    .map_err(|e| e.to_string())?;
                bound_type(&signature, self.fuel)?;
                if !data_passing_signature(self.package, &signature, self.fuel)? {
                    break;
                }
                return Ok(EffectInput {
                    operation: operation.clone(),
                    abi: declaration.abi,
                    signature,
                    forwarded_parameters: None,
                    copy_parameters: BTreeSet::new(),
                    owned_inputs: false,
                    parameter_accesses: BTreeSet::new(),
                });
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
            // Pointer and callable origins cross the boundary through the
            // result caches; the computation wrapper grants no extra lifetime.
            if !effect_transfer_layout(self.package, &result, self.fuel)?
                && !effect_data_layout(self.package, result, self.fuel, 0)?
            {
                return Err(
                    "effect memory completion requires data-only delivery or tracked origins"
                        .into(),
                );
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
                if !effect_transfer_layout(self.package, &ty, self.fuel)?
                    && !effect_data_layout(self.package, &ty, self.fuel, 0)?
                {
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
                let forwarded = input.forwarded_parameters.clone();
                let Term::Function { parameters, result } = signature else {
                    unreachable!()
                };
                if parameters.len() != arguments.len() || self.ty(&expr.ty)? != *result {
                    return Err("effect memory request signature mismatch".into());
                }
                let mut returned = None;
                let mut have_result = false;
                let mut pending_accesses = Vec::new();
                for (index, (argument, parameter)) in arguments.iter().zip(parameters).enumerate() {
                    let ty = self.ty(&argument.ty)?;
                    if ty != parameter.ty && ty != Term::Primitive(Primitive::Never) {
                        return Err("effect memory request argument mismatch".into());
                    }
                    self.walk(argument, depth + 1)?;
                    for access in &input.parameter_accesses {
                        self.step(access.input_fields.len().saturating_add(1))?;
                        if access.parameter != index {
                            continue;
                        }
                        let pointee = access_pointee(
                            self.package,
                            &parameter.ty,
                            &access.input_fields,
                            self.fuel,
                        )?;
                        let targets = if access.input_fields.is_empty() {
                            self.optional_target(argument, depth + 1)?
                        } else {
                            self.pointer_fields_for(argument)?
                                .remove(&access.input_fields)
                                .flatten()
                        }
                        .ok_or_else(|| format!(
                            "effect parameter access requires known pointer provenance: operation {operation}, argument {index}, fields {:?}, source {}",
                            access.input_fields, argument.source,
                        ))?;
                        self.target_work(&targets)?;
                        pending_accesses.push((targets, access.write, access.read_value, pointee));
                    }
                    if forwarded
                        .as_ref()
                        .is_some_and(|indices| indices.contains(&index))
                    {
                        if have_result {
                            self.join_effect_value(expr, argument)?;
                        } else {
                            self.forward_effect_value(expr, argument)?;
                        }
                        if self.local_pointer_value(&result)? {
                            let next = self.optional_target(argument, depth + 1)?;
                            returned = if have_result {
                                self.join_targets(returned, next)?
                            } else {
                                next
                            };
                        }
                        have_result = true;
                    }
                }
                // Snapshot each argument before later operands run, then apply
                // the operation footprint after argument preparation completes.
                for (targets, write, read, pointee) in pending_accesses {
                    self.record_access(targets, write, read, pointee)?;
                }
                if self.local_pointer_value(&result)? {
                    self.expression_results
                        .insert(expr as *const Expr as usize, returned);
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
                let walked = self.walk(body, depth + 1);
                self.effect_row = previous;
                walked?;
                self.forward_effect_value(expr, body)?;
                if self.local_pointer_value(&result)? {
                    let targets = self.optional_target(body, depth + 1)?;
                    self.expression_results
                        .insert(expr as *const Expr as usize, targets);
                }
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
                self.forward_effect_value(expr, computation)?;
                if self.local_pointer_value(&result)? {
                    let targets = self.optional_target(computation, depth + 1)?;
                    self.expression_results
                        .insert(expr as *const Expr as usize, targets);
                }
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

/// Parameter-relative memory analysis of a closed handler arm. This is a
/// conditional summary, not permission to borrow actual request arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct HandlerParameterSummary {
    pub memory: ArgumentSummary,
    pub accesses: BTreeSet<EffectParameterAccess>,
    /// Borrowed environment accesses retain their lexical binding separately
    /// from operation parameters. Virtual slots exist only in `memory`.
    pub capture_accesses: Vec<(LocalId, Access)>,
}
impl fmt::Display for HandlerParameterSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(handler-parameter-memory (accesses")?;
        for access in &self.accesses {
            write!(f, " {access}")?;
        }
        f.write_str(") (capture-accesses")?;
        for (binding, access) in &self.capture_accesses {
            write!(f, " (capture {binding} {access})")?;
        }
        write!(f, ") {})", self.memory)
    }
}

/// Reuse normal memory inference and pattern binding for closed direct arms.
/// Input identities belong to the complete parameters, even when a pattern
/// exposes only their fields. Captures still require an environment adapter.
pub fn infer_handler_arm_memory(
    package: &Package,
    handler: &nir::program::EffectHandler,
    arm: &nir::program::MatchArm,
    fuel: &mut usize,
) -> Result<HandlerParameterSummary, String> {
    infer_handler_arm_memory_with_captures(
        package,
        handler,
        arm,
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &[],
        fuel,
    )
}

fn infer_handler_arm_memory_with_captures(
    package: &Package,
    handler: &nir::program::EffectHandler,
    arm: &nir::program::MatchArm,
    captures: &[(LocalId, Term)],
    captured_code: &BTreeMap<CallablePlace, CallableResult>,
    prepared_code: &PreparedCaptures,
    effects: &[EffectInput],
    effect_row: &[nir::effects::Operation],
    fuel: &mut usize,
) -> Result<HandlerParameterSummary, String> {
    if handler.abi != nir::effects::Abi::Direct {
        return Err("handler parameter inference requires direct ABI".into());
    }
    bound_type(&handler.signature, fuel)?;
    let Term::Function { parameters, .. } = &handler.signature else {
        return Err("handler parameter inference requires an operation signature".into());
    };
    let free = nir::free_bindings::collect_arm(arm, fuel)?;
    let mut capture_bindings = BTreeSet::new();
    for (binding, ty) in captures {
        bound_type(ty, fuel)?;
        let layout = pointer_input_layout(Some(package), ty, fuel)?;
        // Pointer captures retain their declared pointee layout. Validate each
        // accessed projection below instead of requiring the entire pointee
        // to be scalar; unrelated aggregate fields need not be accessed.
        if !capture_bindings.insert(*binding)
            || (layout.is_empty() && !effect_storage_layout(Some(package), ty, fuel, 0, true)?)
        {
            return Err("handler parameter inference requires data or pointer captures".into());
        }
    }
    if free != capture_bindings {
        return Err("handler parameter inference needs an explicit capture environment".into());
    }
    let (pattern, mut guards) = outer_guards(&arm.pattern);
    guards.extend(arm.guard.as_ref());
    let Pattern::Tuple(fields) = pattern else {
        return Err("handler parameter inference requires a parameter tuple".into());
    };
    if fields.len() != parameters.len() {
        return Err("handler parameter inference arity mismatch".into());
    }
    // The private closure receives complete inputs, then uses ordinary pattern
    // binding to expose the source arm's locals. Reserve an owner absent from
    // the arm's bindings and uses so analysis inputs cannot shadow source names.
    fn pattern_owners(
        pattern: &Pattern,
        owners: &mut BTreeSet<u32>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        let mut pending = vec![pattern];
        while let Some(pattern) = pending.pop() {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("handler pattern budget exhausted")?;
            match pattern {
                Pattern::Binding { binding, inner } | Pattern::Observe { binding, inner } => {
                    owners.insert(binding.owner);
                    pending.push(inner);
                }
                Pattern::Remap { pattern, bindings } => {
                    *fuel = fuel
                        .checked_sub(bindings.len())
                        .ok_or("handler remap budget exhausted")?;
                    for (from, to) in bindings {
                        owners.insert(from.owner);
                        owners.insert(to.owner);
                    }
                    pending.push(pattern);
                }
                Pattern::Tuple(fields)
                | Pattern::Record { fields, .. }
                | Pattern::AnonymousRecord { fields, .. }
                | Pattern::Variant { fields, .. }
                | Pattern::Or(fields)
                | Pattern::TypeTuple(fields)
                | Pattern::NominalType { fields, .. }
                | Pattern::NormalForm {
                    arguments: fields, ..
                } => {
                    *fuel = fuel
                        .checked_sub(fields.len())
                        .ok_or("handler field budget exhausted")?;
                    pending.extend(fields);
                }
                Pattern::OptionalPresent(pattern)
                | Pattern::ErrorResult { inner: pattern, .. }
                | Pattern::Guard { pattern, .. } => pending.push(pattern),
                Pattern::Then { first, next, .. } => {
                    pending.push(first);
                    pending.push(next);
                }
                Pattern::CodeEvidence { .. } => {
                    return Err("handler has unresolved pattern code".into());
                }
                Pattern::OptionalAbsent | Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => {}
            }
        }
        Ok(())
    }
    let mut owners: BTreeSet<_> = captures.iter().map(|(binding, _)| binding.owner).collect();
    // Nested summaries recurse through their own handler environment. Bound
    // structural nesting before inference, in addition to the shared work fuel.
    let mut nested_handlers = 0usize;
    pattern_owners(pattern, &mut owners, fuel)?;
    let observations = nir::visit::pattern_observations(pattern, fuel)?;
    for expression in std::iter::once(&arm.body)
        .chain(guards.iter().copied())
        .chain(observations)
    {
        nir::visit::expression_in_closure_environment(expression, fuel, |node, fuel| {
            bound_type(&node.ty, fuel)?;
            match &node.kind {
                K::Local(binding) | K::Let { binding, .. } => {
                    owners.insert(binding.owner);
                }
                K::LetPattern { pattern, .. } => pattern_owners(pattern, &mut owners, fuel)?,
                K::Match { arms, .. } => {
                    for arm in arms {
                        pattern_owners(&arm.pattern, &mut owners, fuel)?;
                    }
                }
                K::Closure { captures, .. } => {
                    // Local formation may retain request pointers. The ordinary
                    // closure analysis tracks these origins through invocation;
                    // the lexical-return check below independently gates escape.
                    *fuel = fuel
                        .checked_sub(captures.len())
                        .ok_or("handler capture owner budget exhausted")?;
                    for capture in captures {
                        owners.insert(capture.binding.owner);
                    }
                }
                K::EffectHandle { handlers, .. } => {
                    nested_handlers += 1;
                    if nested_handlers > 64 {
                        return Err("handler memory nesting budget exhausted".into());
                    }
                    for handler in handlers {
                        for arm in &handler.arms {
                            pattern_owners(&arm.pattern, &mut owners, fuel)?;
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
    }
    let mut owner = u32::MAX;
    while owners.contains(&owner) {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("handler analysis owner budget exhausted")?;
        owner = owner
            .checked_sub(1)
            .ok_or("handler analysis owner space exhausted")?;
    }
    let mut bindings = Vec::new();
    let mut statements = Vec::new();
    for (index, (field, parameter)) in fields.iter().zip(parameters).enumerate() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("handler parameter binding budget exhausted")?;
        let binding = LocalId {
            owner,
            index: u32::try_from(index).map_err(|_| "handler parameter index overflow")?,
        };
        bindings.push(binding);
        statements.push(Expr {
            ty: Term::Primitive(Primitive::Void),
            source: arm.body.source,
            kind: K::LetPattern {
                pattern: field.clone(),
                value: Box::new(Expr {
                    ty: parameter.ty.clone(),
                    source: arm.body.source,
                    kind: K::Local(binding),
                }),
                guard: None,
                otherwise: None,
            },
        });
    }
    for guard in guards {
        statements.push(Expr {
            ty: Term::Primitive(Primitive::Void),
            source: guard.source,
            kind: K::Discard(Box::new(guard.clone())),
        });
    }
    // Guard failure excludes this body at runtime. Including it here is a
    // conservative footprint; candidate ownership is checked independently.
    let mut body = Expr {
        ty: arm.body.ty.clone(),
        source: arm.body.source,
        kind: K::Block {
            statements,
            result: Some(Box::new(arm.body.clone())),
        },
    };
    nir::visit::expression_mut_in_handler_scope(&mut body, fuel, |node, fuel| {
        match &node.kind {
            K::Return(Some(value)) => {
                if !effect_storage_layout(Some(package), &value.ty, fuel, 0, true)? {
                    return Err("handler lexical return requires a separate exit summary".into());
                }
                // This private body describes memory accesses, not handler
                // control flow. Observe the lexical result's evaluation without
                // joining it into the operation's ordinary completion value.
                // Data pointers still require an exit-specific substitution.
                // Pointer-free closure bodies retain their own returns and
                // are not evaluated during formation. Code identity is retained by
                // the original arm's resource analysis, not by this copy.
                let K::Return(Some(value)) = std::mem::replace(&mut node.kind, K::Unit) else {
                    unreachable!()
                };
                node.kind = K::Block {
                    statements: vec![Expr {
                        source: value.source,
                        ty: Term::Primitive(Primitive::Void),
                        kind: K::Discard(value),
                    }],
                    result: Some(Box::new(Expr {
                        source: node.source,
                        ty: Term::Primitive(Primitive::Never),
                        kind: K::Return(None),
                    })),
                };
            }
            K::EffectResume(_) => {
                let K::EffectResume(value) = std::mem::replace(&mut node.kind, K::Unit) else {
                    unreachable!()
                };
                // The private closure's return target represents only this
                // handler's resume; source lexical returns remain distinct.
                node.kind = K::Return(Some(value));
            }
            _ => {}
        }
        Ok(())
    })?;
    // Requests in the arm address the installer's outer row. Carry that row
    // into the private analysis closure as well as its memory premises.
    let mut signature = handler.signature.clone();
    if !effect_row.is_empty() {
        *fuel = fuel
            .checked_sub(effect_row.len())
            .ok_or("handler outer effect row budget exhausted")?;
        for operation in effect_row {
            for argument in &operation.arguments {
                bound_type(argument, fuel)?;
            }
        }
        let Term::Function { result, .. } = &mut signature else {
            unreachable!()
        };
        bound_type(result, fuel)?;
        *result = Box::new(Term::Computation {
            effects: effect_row.to_vec(),
            result: result.clone(),
        });
        bound_type(&signature, fuel)?;
    }
    let definition = Expr {
        ty: Term::Closure {
            environment: captures.iter().map(|(_, ty)| ty.clone()).collect(),
            signature: Box::new(signature),
        },
        source: arm.body.source,
        kind: K::Closure {
            parameters: bindings,
            captures: captures
                .iter()
                .map(|(binding, ty)| nir::program::Capture {
                    binding: *binding,
                    ty: ty.clone(),
                    value: None,
                })
                .collect(),
            body: std::sync::Arc::new(body),
        },
    };
    let memory = infer_closure_with_borrowed_effect_context(
        package,
        &definition,
        captured_code,
        prepared_code,
        &BTreeMap::new(),
        &BTreeMap::new(),
        effects,
        fuel,
    )?;
    let mut pointer_captures = Vec::new();
    for capture in captures {
        if !pointer_input_layout(Some(package), &capture.1, fuel)?.is_empty() {
            pointer_captures.push(capture);
        }
    }
    let mut capture_accesses = Vec::new();
    let mut accesses = BTreeSet::new();
    for access in &memory.accesses {
        *fuel = fuel
            .checked_sub(access.target.path_cost())
            .ok_or("handler parameter access budget exhausted")?;
        if !access.target.conditions.is_empty() {
            return Err("handler parameter access requires unguarded storage".into());
        }
        if access.target.parameter >= parameters.len() {
            let (binding, ty) = pointer_captures
                .get(access.target.parameter - parameters.len())
                .ok_or("handler memory access is not a parameter or capture")?;
            let layout = pointer_input_layout(Some(package), ty, fuel)?;
            let leaf = layout
                .iter()
                .find(|leaf| leaf.input_fields == access.target.input_fields)
                .ok_or("handler capture access does not select a pointer input")?;
            let mut traversal = StorageTraversal::new(Some(package), &leaf.pointee, &[], fuel)?;
            // Each segment selects a pointer slot in the current pointee, then
            // loads it. Validate every owner/type boundary before the final field.
            for fields in &access.target.through {
                traversal.then_guarded(Some(package), fields, &[], fuel)?;
                traversal.load(Some(package), &[], fuel)?;
            }
            traversal.then_guarded(Some(package), &access.target.fields, &[], fuel)?;
            if traversal.target.ty != access.ty {
                return Err("handler capture access pointee mismatch".into());
            }
            capture_accesses.push((*binding, access.clone()));
            continue;
        }
        if !access.target.fields.is_empty() || !access.target.through.is_empty() {
            return Err("handler operation parameter access requires a scalar pointee".into());
        }
        let parameter = &parameters[access.target.parameter];
        let pointee = access_pointee(
            Some(package),
            &parameter.ty,
            &access.target.input_fields,
            fuel,
        )?;
        if pointee != access.ty {
            return Err("handler parameter access pointee mismatch".into());
        }
        accesses.insert(EffectParameterAccess {
            parameter: access.target.parameter,
            input_fields: access.target.input_fields.clone(),
            read_value: access.read_value,
            write: access.write,
        });
    }
    Ok(HandlerParameterSummary {
        memory,
        accesses,
        capture_accesses,
    })
}

/// Admission shared by source memory inference and resource installation.
/// Direct arms with data results and explicit scalar input accesses can combine
/// their parameter contract with independently validated lexical captures.
pub(crate) fn direct_handler_accesses(
    package: Option<&Package>,
    handler: &nir::program::EffectHandler,
    arm: &nir::program::MatchArm,
    captures: &[(LocalId, Term)],
    fuel: &mut usize,
) -> Result<Option<HandlerParameterSummary>, String> {
    direct_handler_accesses_with_code(
        package,
        handler,
        arm,
        captures,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &[],
        fuel,
    )
}

pub(crate) fn direct_handler_accesses_with_code(
    package: Option<&Package>,
    handler: &nir::program::EffectHandler,
    arm: &nir::program::MatchArm,
    captures: &[(LocalId, Term)],
    captured_code: &BTreeMap<CallablePlace, CallableResult>,
    prepared_code: &PreparedCaptures,
    effects: &[EffectInput],
    effect_row: &[nir::effects::Operation],
    fuel: &mut usize,
) -> Result<Option<HandlerParameterSummary>, String> {
    let Some(package) = package else {
        return Ok(None);
    };
    let Term::Function { parameters, result } = &handler.signature else {
        return Ok(None);
    };
    if handler.abi != nir::effects::Abi::Direct
        || !effect_data_layout(Some(package), result, fuel, 0)?
    {
        return Ok(None);
    }
    let mut has_pointer = false;
    for parameter in parameters {
        if !pointer_input_layout(Some(package), &parameter.ty, fuel)?.is_empty() {
            has_pointer = true;
            break;
        }
    }
    if !has_pointer {
        for (_, ty) in captures {
            if !pointer_input_layout(Some(package), ty, fuel)?.is_empty() {
                has_pointer = true;
                break;
            }
        }
    }
    if !has_pointer {
        return Ok(None);
    }
    let mut summary = match infer_handler_arm_memory_with_captures(
        package,
        handler,
        arm,
        captures,
        captured_code,
        prepared_code,
        effects,
        effect_row,
        fuel,
    ) {
        Ok(summary) => summary,
        Err(error) if *fuel == 0 => return Err(error),
        Err(_) => return Ok(None),
    };
    // Memory inference does not resolve implicit Drop bodies. Conservatively
    // require every supplied pointer leaf to permit reads and writes when
    // cleanup is present. The subsequent resource analysis must still prove
    // Copy/scalar borrowing and check the selected Drop against these origins.
    // This is an invocation premise, not a verified cleanup summary.
    if summary.memory.may_run_cleanup {
        for (parameter, input) in parameters.iter().enumerate() {
            for leaf in pointer_input_layout(Some(package), &input.ty, fuel)? {
                *fuel = fuel
                    .checked_sub(leaf.input_fields.len().saturating_add(1))
                    .ok_or("handler cleanup parameter budget exhausted")?;
                summary.accesses.insert(EffectParameterAccess {
                    parameter,
                    input_fields: leaf.input_fields,
                    read_value: true,
                    write: true,
                });
            }
        }
    }
    if (summary.accesses.is_empty()
        && summary.capture_accesses.is_empty()
        && !handler_capture_origins_preserved(
            Some(package),
            &summary.memory,
            parameters.len(),
            captures,
            fuel,
        )?)
        || summary.memory.result_targets.is_some()
        || !summary.memory.pointer_fields.is_empty()
    {
        return Ok(None);
    }
    // Variant-qualified parameter paths retain their conditional identity.
    // Guarded captures require stable tag evidence from the installer.
    Ok(Some(summary))
}

/// A stationary environment needs no caller-relative pointer writeback. Every
/// pointer leaf must have one exact self origin on every completing path.
/// Missing, local, joined, traversed and variant-dependent origins do not prove
/// this invariant. Callable identities have their separate stability checks.
fn handler_capture_origins_preserved(
    package: Option<&Package>,
    summary: &ArgumentSummary,
    parameter_count: usize,
    captures: &[(LocalId, Term)],
    fuel: &mut usize,
) -> Result<bool, String> {
    let mut slot = parameter_count;
    let mut found = false;
    for (_, ty) in captures {
        let leaves = pointer_input_layout(package, ty, fuel)?;
        if leaves.is_empty() {
            continue;
        }
        found = true;
        for leaf in leaves {
            *fuel = fuel
                .checked_sub(leaf.input_fields.len().saturating_add(1))
                .ok_or("handler capture exit budget exhausted")?;
            if !leaf.guards.is_empty() {
                return Ok(false);
            }
            let input = CallableInput {
                parameter: slot,
                fields: leaf.input_fields.clone(),
            };
            let Some(Some(origins)) = summary.input_pointer_exits.get(&input) else {
                return Ok(false);
            };
            if origins.len() != 1 {
                return Ok(false);
            }
            let origin = origins.first().unwrap();
            if origin.parameter != slot
                || origin.input_fields != leaf.input_fields
                || !origin.fields.is_empty()
                || !origin.through.is_empty()
                || !origin.conditions.is_empty()
            {
                return Ok(false);
            }
        }
        slot += 1;
    }
    Ok(found)
}

// An exit invariant is insufficient if another handler can observe a temporary
// rebinding during suspension. Effectful calls and nested installations need
// a stronger all-suspension writeback contract; keep them on the old path.
fn handler_arm_cannot_suspend(body: &Expr, fuel: &mut usize) -> Result<bool, String> {
    let mut stable = true;
    nir::visit::expression_in_closure_environment(body, fuel, |node, _| {
        if matches!(node.kind, K::EffectRequest { .. } | K::EffectHandle { .. })
            || matches!(node.ty, Term::Computation { .. })
        {
            stable = false;
        }
        Ok(())
    })?;
    Ok(stable)
}

/// Return only writes belonging to this arm, with no persisted node identity.
pub(crate) fn stationary_handler_write_sites(
    package: Option<&Package>,
    arm: &nir::program::MatchArm,
    parameter_count: usize,
    captures: &[(LocalId, Term)],
    summary: &ArgumentSummary,
    fuel: &mut usize,
) -> Result<BTreeSet<usize>, String> {
    let mut writes = BTreeSet::new();
    let (_, guards) = outer_guards(&arm.pattern);
    if !guards.is_empty()
        || arm.guard.is_some()
        || !nir::visit::pattern_observations(&arm.pattern, fuel)?.is_empty()
        || !handler_arm_cannot_suspend(&arm.body, fuel)?
        || !handler_capture_origins_preserved(package, summary, parameter_count, captures, fuel)?
    {
        return Ok(writes);
    }
    *fuel = fuel
        .checked_sub(captures.len())
        .ok_or("handler stable capture budget exhausted")?;
    let bindings: BTreeSet<_> = captures.iter().map(|(binding, _)| *binding).collect();
    nir::visit::expression_in_closure_environment(&arm.body, fuel, |node, _| {
        if matches!(&node.kind, K::Assign {place, ..} if bindings.contains(&place.root)) {
            writes.insert(node as *const Expr as usize);
        }
        Ok(())
    })?;
    Ok(writes)
}
