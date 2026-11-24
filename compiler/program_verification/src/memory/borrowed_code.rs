//! Conditional code identities in pointer-reachable storage. These are not
//! owned argument leaves and must never grant Copy or pointee validity.
use super::*;

/// Code identity or a caller-established inactive variant at one code leaf.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BorrowedCallableValue {
    Named(CallableResult),
    /// The caller proved an enclosing variant absent, not an unknown target.
    Absent,
}

impl From<CallableResult> for BorrowedCallableValue {
    fn from(value: CallableResult) -> Self {
        Self::Named(value)
    }
}

impl fmt::Display for BorrowedCallableValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(value) => write!(f, "(named-code {value})"),
            Self::Absent => f.write_str("(absent-code)"),
        }
    }
}

impl BorrowedCallableValue {
    fn charge(&self, fuel: &mut usize) -> Result<(), String> {
        match self {
            Self::Named(value) => value.charge(fuel),
            Self::Absent => {
                *fuel = fuel
                    .checked_sub(1)
                    .ok_or("borrowed absence budget exhausted")?;
                Ok(())
            }
        }
    }
}

/// Outer key selects a pointer-bearing owned argument leaf; inner paths select
/// function leaves in its pointee using the usual guarded owned-layout paths.
/// Missing entries mean unknown, never an empty set of candidate functions.
pub type BorrowedCallableInputs =
    BTreeMap<CallableInput, BTreeMap<Vec<usize>, BorrowedCallableValue>>;

/// The caller must establish identities and inactive variants after all arguments.
/// A returned summary is conditional on them, and preserves them in its dump
/// and instance identity. Until storage transitions are modeled, any explicit
/// write or possible implicit cleanup makes this query fail closed.
pub fn infer_with_borrowed_callable_inputs(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    effects: &[EffectInput],
    borrowed: &BorrowedCallableInputs,
    fuel: &mut usize,
) -> Result<Summary, String> {
    infer_with_borrowed_callable_inputs_and_cleanup(
        package,
        declaration,
        arguments,
        inputs,
        code_inputs,
        effects,
        borrowed,
        None,
        fuel,
    )
}

fn callable_targets(value: &CallableResult) -> Option<Vec<iir::InstanceKey>> {
    match value {
        CallableResult::Known(key) => Some(vec![key.clone()]),
        CallableResult::Candidates(keys) if !keys.is_empty() => Some(keys.clone()),
        _ => None,
    }
}

fn convert_text_callback_inputs(
    inputs: &BTreeMap<CallableInput, CallableResult>,
    borrowed: &BorrowedCallableInputs,
) -> Option<(Vec<TextCallbackInput>, Vec<iir::InstanceKey>)> {
    let mut converted = Vec::new();
    let mut instances = Vec::new();
    for (input, value) in inputs {
        let targets = callable_targets(value)?;
        instances.extend(targets.iter().cloned());
        converted.push(TextCallbackInput::Owned {
            parameter: input.parameter,
            fields: input.fields.clone(),
            targets,
        });
    }
    for (input, fields) in borrowed {
        for (path, value) in fields {
            let BorrowedCallableValue::Named(value) = value else {
                return None;
            };
            let targets = callable_targets(value)?;
            instances.extend(targets.iter().cloned());
            converted.push(TextCallbackInput::Borrowed {
                parameter: input.parameter,
                input_fields: input.fields.clone(),
                fields: path.clone(),
                targets,
            });
        }
    }
    Some((converted, instances))
}

/// Refine possible cleanup with independent complete-layout memory-effect evidence. The policy
/// must belong to the caller's immutable Drop selection world. It does not grant
/// Copy, authorize writes, or validate borrowed storage. Query caches stay local.
pub fn infer_with_borrowed_callable_inputs_and_cleanup(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    effects: &[EffectInput],
    borrowed: &BorrowedCallableInputs,
    cleanup: Option<&mut dyn super::super::moves::CleanupPolicy>,
    fuel: &mut usize,
) -> Result<Summary, String> {
    let mut provenance = TextProvenance::Unchecked;
    if effects.is_empty()
        && code_inputs.is_empty()
        && let Some((function, body_arguments)) = package.runtime_body(declaration, arguments)
        && let Some((callbacks, extra_instances)) = convert_text_callback_inputs(inputs, borrowed)
        && reachable_contains_str(
            package,
            &[(function, body_arguments)],
            arguments,
            &extra_instances,
            fuel,
        )?
    {
        infer_text_regions_with_callbacks(package, declaration, arguments, &callbacks, fuel)?;
        provenance = TextProvenance::VerifiedByRegionAnalysis;
    }
    infer_with_borrowed_callable_inputs_and_cleanup_cached(
        package,
        declaration,
        arguments,
        inputs,
        code_inputs,
        effects,
        borrowed,
        cleanup,
        None,
        provenance,
        None,
        fuel,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_with_borrowed_callable_inputs_and_cleanup_cached<'a>(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    effects: &[EffectInput],
    borrowed: &BorrowedCallableInputs,
    cleanup: Option<&'a mut dyn super::super::moves::CleanupPolicy>,
    declarations: Option<&'a crate::declarations::Metadata<'a>>,
    text_provenance: TextProvenance,
    summary_cache: Option<&'a mut SummaryCache>,
    fuel: &mut usize,
) -> Result<Summary, String> {
    infer_with_borrowed_callable_inputs_and_cleanup_full_context(
        package,
        declaration,
        arguments,
        inputs,
        code_inputs,
        effects,
        borrowed,
        cleanup,
        None,
        None,
        declarations,
        text_provenance,
        summary_cache,
        None,
        fuel,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_with_borrowed_callable_inputs_and_cleanup_full_context<'a>(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    effects: &[EffectInput],
    borrowed: &BorrowedCallableInputs,
    cleanup: Option<&'a mut dyn super::super::moves::CleanupPolicy>,
    source: Option<::materialization::DeclarationSource<'a>>,
    methods: Option<&'a mut MethodCallees>,
    declarations: Option<&'a crate::declarations::Metadata<'a>>,
    text_provenance: TextProvenance,
    summary_cache: Option<&'a mut SummaryCache>,
    layout_cache: Option<&'a mut TypeLayoutCache>,
    fuel: &mut usize,
) -> Result<Summary, String> {
    for value in inputs.values() {
        if !matches!(
            value,
            CallableResult::Known(_) | CallableResult::Candidates(_)
        ) {
            return Err("borrowed memory premise requires named code identities".into());
        }
    }
    for value in borrowed.values().flat_map(BTreeMap::values) {
        if let BorrowedCallableValue::Named(value) = value {
            if !matches!(
                value,
                CallableResult::Known(_) | CallableResult::Candidates(_)
            ) {
                return Err("borrowed memory premise requires named code identities".into());
            }
        }
    }
    charge(borrowed, fuel)?;
    let prepared = own_code_inputs(code_inputs, &[], &[], fuel)?;
    infer_borrowed_context_with_full_context(
        package,
        declaration,
        arguments,
        inputs,
        &prepared,
        effects,
        borrowed,
        cleanup,
        source,
        methods,
        declarations,
        text_provenance,
        summary_cache,
        layout_cache,
        fuel,
    )
}

pub(super) fn charge(inputs: &BorrowedCallableInputs, fuel: &mut usize) -> Result<(), String> {
    for (input, fields) in inputs {
        *fuel = fuel
            .checked_sub(input.fields.len().saturating_add(1))
            .ok_or("borrowed callable premise budget exhausted")?;
        for (path, value) in fields {
            *fuel = fuel
                .checked_sub(path.len().saturating_add(1))
                .ok_or("borrowed callable path budget exhausted")?;
            value.charge(fuel)?;
        }
    }
    Ok(())
}

pub(super) fn validate(
    package: Option<&Package>,
    round: &mut Round,
    parameters: &[nir::Parameter],
    inputs: &BorrowedCallableInputs,
    fuel: &mut usize,
) -> Result<BTreeMap<PointerPlace, CallableResult>, String> {
    charge(inputs, fuel)?;
    let mut slots = BTreeMap::new();
    for (input, fields) in inputs {
        let package = package.ok_or("borrowed callable premise requires a package")?;
        let parameter = parameters
            .get(input.parameter)
            .ok_or("borrowed callable parameter is missing")?;
        if parameter.sort != nir::Sort::Runtime {
            return Err("borrowed callable parameter must be runtime storage".into());
        }
        let input_projection =
            project_owned_storage(Some(package), &parameter.ty, &input.fields, fuel)?;
        let pointer = input_projection.ty;
        let (_, pointee) = pointer_representation(Some(package), &pointer, fuel)?
            .ok_or("borrowed callable input must select a pointer")?;
        if fields.is_empty() {
            return Err("borrowed callable premise has no code leaves".into());
        }
        for (path, value) in fields {
            let projection = project_owned_storage(Some(package), &pointee, path, fuel)?;
            let value = match value {
                BorrowedCallableValue::Named(value) => {
                    // Private fixed-point propagation may carry bottom from a
                    // validated absent branch. The public entry rejects it.
                    validate_code_premise(package, round, &projection.ty, value, fuel)?;
                    value.clone()
                }
                BorrowedCallableValue::Absent => {
                    if input_projection.guards.is_empty() && projection.guards.is_empty() {
                        return Err("borrowed code absence requires a variant-guarded path".into());
                    }
                    if !matches!(
                        projection.ty,
                        Term::Function { .. } | Term::UnsafeFunction(_)
                    ) {
                        return Err("borrowed code absence must select a function leaf".into());
                    }
                    CallableResult::NoReturn
                }
            };
            let conditions = if projection.guards.is_empty() {
                BTreeMap::new()
            } else {
                BTreeMap::from([(0, projection.guards)])
            };
            let slot = PointerPlace {
                root: PointerRoot::Parameter(input.parameter),
                input_fields: input.fields.clone(),
                through: vec![],
                fields: projection.fields,
                conditions,
            };
            if slots.insert(slot, value.clone()).is_some() {
                return Err("duplicate borrowed callable storage premise".into());
            }
        }
    }
    Ok(slots)
}

impl Infer<'_, '_> {
    /// Rebase only established borrowed identities into the next argument
    /// frame. Different pointer arguments may alias; a candidate is complete
    /// only when every possible actual target supplies matching evidence.
    pub(super) fn forward_borrowed_callables(
        &mut self,
        parameters: &[nir::Parameter],
        targets: &BTreeMap<usize, BTreeMap<Vec<usize>, BTreeSet<PointerPlace>>>,
    ) -> Result<BorrowedCallableInputs, String> {
        let mut output = BTreeMap::new();
        if self.borrowed_callable_slots.is_empty() {
            return Ok(output);
        }
        for (index, parameter) in parameters.iter().enumerate() {
            self.step(0)?;
            let Some(actuals) = targets.get(&index) else {
                continue;
            };
            let leaves = if let Some((_, pointee)) = self.pointer_representation(&parameter.ty)? {
                vec![(vec![], pointee)]
            } else {
                self.pointer_input_layout(&parameter.ty)?
                    .into_iter()
                    .map(|leaf| (leaf.input_fields, leaf.pointee))
                    .collect()
            };
            for (input_fields, pointee) in leaves {
                let Some(actual) = actuals
                    .get(&input_fields)
                    .filter(|places| !places.is_empty())
                else {
                    continue;
                };
                self.target_work(actual)?;
                let mut fields = BTreeMap::new();
                for path in self.owned_callable_paths(&pointee, true)? {
                    let projection =
                        project_owned_storage(self.package, &pointee, &path, self.fuel)?;
                    let mut value = CallableResult::NoReturn;
                    let mut complete = true;
                    for target in actual {
                        self.step(target.path_cost().saturating_add(path.len()))?;
                        let slot = project_slot(target, &projection);
                        if let Some(next) = self.borrowed_callable_slots.get(&slot) {
                            value.join(next, self.fuel)?;
                        } else {
                            complete = false;
                            break;
                        }
                    }
                    if complete {
                        fields.insert(path, BorrowedCallableValue::Named(value));
                    }
                }
                if !fields.is_empty() {
                    output.insert(
                        CallableInput {
                            parameter: index,
                            fields: input_fields,
                        },
                        fields,
                    );
                }
            }
        }
        Ok(output)
    }
}

pub(super) fn project_slot(target: &PointerPlace, projection: &StorageProjection) -> PointerPlace {
    let mut slot = target.clone();
    let prefix: Vec<_> = slot.fields.iter().map(|(_, index)| *index).collect();
    for (path, tag) in &projection.guards {
        let mut rebased = prefix.clone();
        rebased.extend(path);
        slot.conditions
            .entry(slot.through.len())
            .or_default()
            .push((rebased, *tag));
    }
    slot.fields.extend(&projection.fields);
    slot
}
