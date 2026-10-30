//! Private required invocation during concrete staged-input preparation.
use super::*;
use comptime_interpretor::{ContractOutcome, Interpreter, Value};

// Reuse checked code and metadata, never an interpreter, environment or result.
// The scratch root and dispatch table are cleared between independent invocations.
pub(super) struct Snapshot {
    package: n::Package,
    entry_index: usize,
}

impl Snapshot {
    fn clear(&mut self) {
        self.package.dispatch_instances.clear();
        let entry = &mut self.package.functions[self.entry_index];
        entry.signature = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(nir::Primitive::Void)),
        };
        entry.body = n::Expr {
            kind: K::Unit,
            ty: Term::Primitive(nir::Primitive::Void),
            source: n::SourceRange { start: 0, end: 0 },
        };
    }
}

impl Builder<'_> {
    pub(super) fn invoke_staged_input(&mut self, expression: &n::Expr) -> Result<n::Expr, String> {
        self.tick(0)?;
        let mut snapshot = match self.invocation_snapshot.take() {
            Some(snapshot) => snapshot,
            None => self.new_invocation_snapshot()?,
        };
        let result = self.invoke_with_snapshot(expression, &mut snapshot);
        snapshot.clear();
        self.invocation_snapshot = Some(snapshot);
        result
    }

    fn new_invocation_snapshot(&mut self) -> Result<Snapshot, String> {
        // The source package is already checked. Preserve its implementation
        // world in this private snapshot; publish only the frozen result.
        for function in self
            .package
            .functions
            .iter()
            .chain(&self.package.quote_templates)
        {
            self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or("staged snapshot budget exhausted")?;
            iir::visit::expressions(function, &mut self.fuel, |_, _| Ok(()))?;
            crate::associated::charge_projection_copy(&function.signature, &mut self.fuel)
                .map_err(|_| "staged snapshot type budget exhausted")?;
        }
        let mut snapshot = self.package.clone();
        // Retained quote bodies still implement trait members in this private
        // world. Preserve their full contracts for interpreter validation;
        // their runtime phase prevents invocation during comptime evaluation.
        // No signature-only identity may erase a quote method precondition.
        snapshot.functions.append(&mut snapshot.quote_templates);
        let mut entry = Id {
            package: u64::MAX,
            local: u32::MAX,
        };
        while snapshot.functions.iter().any(|f| f.declaration == entry)
            || snapshot
                .runtime_function_identities
                .iter()
                .any(|f| f.declaration == entry)
        {
            entry.local = entry
                .local
                .checked_sub(1)
                .ok_or("no staged invocation identity available")?;
        }
        let entry_index = snapshot.functions.len();
        snapshot.functions.push(n::Function {
            declaration: entry,
            name: "<staged-input>".into(),
            phase: n::FunctionPhase::Comptime,
            inline: false,
            pure: false,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(nir::Primitive::Void)),
            },
            parameters: vec![],
            type_parameters: vec![],
            body: n::Expr {
                kind: K::Unit,
                ty: Term::Primitive(nir::Primitive::Void),
                source: n::SourceRange { start: 0, end: 0 },
            },
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
        });
        snapshot.dispatch_instances.clear();
        Ok(Snapshot {
            package: snapshot,
            entry_index,
        })
    }

    fn invoke_with_snapshot(
        &mut self,
        expression: &n::Expr,
        scratch: &mut Snapshot,
    ) -> Result<n::Expr, String> {
        iir::visit::expression(expression, &mut self.fuel, |_, _| Ok(()))?;
        crate::associated::charge_projection_copy(&expression.ty, &mut self.fuel)
            .map_err(|_| "staged result type budget exhausted")?;
        let snapshot = &mut scratch.package;
        let root = &mut snapshot.functions[scratch.entry_index];
        root.signature = Term::Function {
            parameters: vec![],
            result: Box::new(expression.ty.clone()),
        };
        root.body = expression.clone();
        let entry = root.declaration;
        let roots = [iir::InstanceKey {
            declaration: entry,
            arguments: vec![],
        }];
        let prepared = build(
            snapshot,
            Budget {
                steps: self.fuel,
                ..self.budget
            },
            Roots::Dispatch(&roots),
            BTreeSet::new(),
            &[],
            Some((self.world.context(), self.world.assumptions())),
            None,
        )?;
        snapshot.dispatch_instances = prepared.0;
        self.fuel = prepared.4;
        let solver = |predicate: &n::TypePredicate| {
            let world = crate::capabilities::World {
                index: self.world.index(),
                implementations: &snapshot.implementations,
                traits: &snapshot.traits,
                assumptions: self.world.assumptions(),
                context: self.world.context(),
            };
            match world.check_predicate(predicate, self.budget) {
                Ok(crate::ptc::Outcome::Entailed(_)) => ContractOutcome::Satisfied,
                Ok(crate::ptc::Outcome::NotEntailed(_)) => ContractOutcome::Rejected,
                _ => ContractOutcome::Unknown,
            }
        };
        let mut interpreter = Interpreter::for_comptime_with_assumptions(
            snapshot,
            self.world.assumptions(),
            comptime_interpretor::Budget {
                steps: self.fuel as u64,
                depth: 128,
            },
        )
        .map_err(|error| format!("staged comptime preparation: {error}"))?
        .with_type_solver(&solver);
        let value = interpreter.invoke(entry, vec![]);
        self.fuel = usize::try_from(interpreter.remaining_steps()).unwrap_or(usize::MAX);
        let value = value.map_err(|error| format!("staged comptime invocation: {error}"))?;
        self.freeze_staged_value(value, &expression.ty, expression.source, 0)
    }

    fn freeze_staged_value(
        &mut self,
        value: Value,
        ty: &Term,
        source: n::SourceRange,
        depth: usize,
    ) -> Result<n::Expr, String> {
        self.tick(depth)?;
        let kind = match value {
            Value::RuntimeFunction {
                declaration,
                arguments,
            } if !ty.is_comptime_callable()
                && matches!(ty.callable_signature(), Term::Function { .. }) =>
            {
                for argument in &arguments {
                    nir::type_computations::charge_term(argument, &mut self.fuel)
                        .map_err(|error| error.to_string())?;
                    iir::completed_type(argument, &BTreeSet::new(), &mut self.fuel)?;
                }
                K::FunctionInstance {
                    declaration,
                    arguments,
                }
            }
            Value::RuntimeTraitMethod(identity)
                if !ty.is_comptime_callable()
                    && matches!(ty.callable_signature(), Term::Function { .. }) =>
            {
                let comptime_interpretor::TraitMethodIdentity {
                    interface,
                    subject,
                    member,
                    method_arguments,
                } = *identity;
                for argument in std::iter::once(&subject)
                    .chain(&interface.arguments)
                    .chain(&method_arguments)
                {
                    nir::type_computations::charge_term(argument, &mut self.fuel)
                        .map_err(|error| error.to_string())?;
                    iir::completed_type(argument, &BTreeSet::new(), &mut self.fuel)?;
                }
                K::TraitMethod {
                    interface: interface.declaration,
                    interface_arguments: interface.arguments,
                    subject,
                    member,
                    method_arguments,
                }
            }
            Value::Function(declaration)
                if matches!(ty.callable_signature(), Term::Function { .. }) =>
            {
                K::Function(declaration)
            }
            Value::Closure(closure)
                if matches!(ty.callable_signature(), Term::Function { .. })
                    && ty.is_comptime_callable() == closure.ty().is_comptime_callable()
                    && closure.captured_values().len() == 0
                    && closure.declaration_instance().is_some() =>
            {
                let (declaration, arguments) = closure.declaration_instance().unwrap();
                for argument in arguments {
                    nir::type_computations::charge_term(argument, &mut self.fuel)
                        .map_err(|error| error.to_string())?;
                    iir::completed_type(argument, &BTreeSet::new(), &mut self.fuel)?;
                }
                K::FunctionInstance {
                    declaration,
                    arguments: arguments.to_vec(),
                }
            }
            Value::Closure(closure)
                if matches!(ty.callable_signature(), Term::Function { .. })
                    && ty.is_comptime_callable() == closure.ty().is_comptime_callable()
                    && closure.captured_values().len() == 0 =>
            {
                // Retain the checked anonymous body; freezing creates a callable
                // and must not execute it or manufacture a declaration identity.
                let mut steps = self.fuel as u64;
                let checked = nir::copy_budget::charge_type_rewrite(closure.body(), &mut steps);
                self.fuel = usize::try_from(steps).unwrap_or(usize::MAX);
                checked.map_err(|error| {
                    format!("staged closure body exceeds copy limits: {error:?}")
                })?;
                K::Closure {
                    parameters: closure.parameters().to_vec(),
                    captures: Vec::new(),
                    body: closure.body().clone(),
                }
            }
            Value::Optional { ty: actual, value } => {
                if actual != *ty {
                    return Err("staged optional identity mismatch".into());
                }
                let Term::Optional(payload) = ty else {
                    return Err("staged optional type missing".into());
                };
                match value {
                    None => K::OptionalAbsent,
                    Some(value) => K::OptionalPresent(Box::new(self.freeze_staged_value(
                        *value,
                        payload,
                        source,
                        depth + 1,
                    )?)),
                }
            }
            Value::ErrorResult {
                ty: actual,
                error,
                value,
            } => {
                if actual != *ty {
                    return Err("staged error result identity mismatch".into());
                }
                let Term::ErrorResult { errors, result } = ty else {
                    return Err("staged error result type missing".into());
                };
                let (conversion, domain) = match error {
                    None => (nir::error_results::Conversion::Success, result.as_ref()),
                    Some(index) => (
                        nir::error_results::Conversion::Failure,
                        errors.get(index).ok_or("staged error result tag invalid")?,
                    ),
                };
                K::ErrorConvert {
                    conversion,
                    value: Box::new(self.freeze_staged_value(*value, domain, source, depth + 1)?),
                }
            }
            Value::Newtype { ty: actual, value } => {
                if actual != *ty {
                    return Err("staged newtype identity mismatch".into());
                }
                let Term::Nominal {
                    declaration,
                    arguments,
                } = ty
                else {
                    return Err("staged newtype requires a nominal type".into());
                };
                let definition = self
                    .package
                    .newtypes
                    .iter()
                    .find(|definition| definition.declaration == *declaration)
                    .ok_or("staged newtype declaration is missing")?;
                self.fuel = self
                    .fuel
                    .checked_sub(self.package.newtypes.len())
                    .ok_or("staged newtype layout budget exhausted")?;
                let representation = definition
                    .representation_budgeted(arguments, &[], &mut self.fuel)
                    .map_err(|error| error.to_string())?;
                K::NewtypeConvert(Box::new(self.freeze_staged_value(
                    *value,
                    &representation,
                    source,
                    depth + 1,
                )?))
            }
            Value::Unit => K::Unit,
            Value::Bool(v) => K::Literal(n::Literal::Bool(v)),
            Value::Char(v) => K::Literal(n::Literal::Char(v)),
            Value::Float16(bits) if *ty == Term::Primitive(nir::Primitive::F16) => {
                K::Literal(n::Literal::Float16(bits))
            }
            Value::BFloat16(bits) if *ty == Term::Primitive(nir::Primitive::Bf16) => {
                K::Literal(n::Literal::BFloat16(bits))
            }
            Value::Float32(bits) if *ty == Term::Primitive(nir::Primitive::F32) => {
                K::Literal(n::Literal::Float32(bits))
            }
            Value::Float64(bits) if *ty == Term::Primitive(nir::Primitive::F64) => {
                K::Literal(n::Literal::Float64(bits))
            }
            Value::Symbol(v) => {
                self.fuel = self
                    .fuel
                    .checked_sub(v.len())
                    .ok_or("staged string budget exhausted")?;
                K::Literal(n::Literal::Symbol(v.to_string()))
            }
            Value::String(v) => {
                self.fuel = self
                    .fuel
                    .checked_sub(v.len())
                    .ok_or("staged string budget exhausted")?;
                K::Literal(n::Literal::String(v.to_string()))
            }
            Value::SignedInteger(v) => K::Literal(n::Literal::SignedInteger(v)),
            Value::UnsignedInteger(v) => K::Literal(n::Literal::UnsignedInteger(v)),
            Value::Type(v) => K::TypeValue(v),
            Value::List { ty: actual, values } => {
                if actual != *ty {
                    return Err("staged list type identity mismatch".into());
                }
                let element = self
                    .package
                    .normal_forms
                    .iter()
                    .find_map(|head| head.list_element(ty))
                    .cloned()
                    .ok_or("staged list requires trusted meta.List")?;
                self.fuel = self
                    .fuel
                    .checked_sub(values.len())
                    .ok_or("staged list budget exhausted")?;
                K::TypeList(
                    values
                        .into_iter()
                        .map(|value| self.freeze_staged_value(value, &element, source, depth + 1))
                        .collect::<Result<_, _>>()?,
                )
            }
            Value::BitVec(v) => {
                self.fuel = self
                    .fuel
                    .checked_sub(v.words().len())
                    .ok_or("staged bit-vector budget exhausted")?;
                K::Literal(n::Literal::BitVec {
                    width: v.width(),
                    words: v.words().to_vec(),
                })
            }
            Value::Integer(v) => {
                let mut budget = comptime_runtime::Budget {
                    steps: self.fuel,
                    max_limbs: 4096,
                };
                let spelling = v.to_hex_literal(&mut budget);
                self.fuel = budget.steps;
                K::Literal(n::Literal::Integer(spelling.map_err(|e| e.to_string())?))
            }
            Value::AnonymousRecord(values) => {
                let Term::Record(types) = ty else {
                    return Err("staged record result type mismatch".into());
                };
                if values.len() != types.len() {
                    return Err("staged record result arity mismatch".into());
                }
                let mut fields = Vec::new();
                for ((name, value), (expected_name, ty)) in values.into_iter().zip(types) {
                    self.fuel = self
                        .fuel
                        .checked_sub(name.len())
                        .ok_or("staged record budget exhausted")?;
                    if name != *expected_name {
                        return Err("staged record result label mismatch".into());
                    }
                    fields.push(self.freeze_staged_value(value, ty, source, depth + 1)?);
                }
                K::AnonymousRecord { fields }
            }
            Value::Tuple(values) => {
                let Some(types) = nir::normal_forms::tuple_fields(Some(self.package), ty) else {
                    return Err("staged tuple result type mismatch".into());
                };
                if types.len() != values.len() {
                    return Err("staged tuple result arity mismatch".into());
                }
                K::Tuple(
                    values
                        .into_iter()
                        .zip(types)
                        .map(|(v, t)| self.freeze_staged_value(v, t, source, depth + 1))
                        .collect::<Result<_, _>>()?,
                )
            }
            Value::Record {
                declaration,
                arguments,
                fields,
            } => {
                let fields = self.freeze_staged_nominal_fields(
                    declaration,
                    &arguments,
                    fields,
                    None,
                    ty,
                    source,
                    depth,
                )?;
                K::Record {
                    declaration,
                    fields,
                }
            }
            Value::Variant {
                declaration,
                arguments,
                index,
                fields,
            } => {
                let variant = u32::try_from(index)
                    .map_err(|_| "staged enum variant index exceeds representation")?;
                let fields = self.freeze_staged_nominal_fields(
                    declaration,
                    &arguments,
                    fields,
                    Some(variant),
                    ty,
                    source,
                    depth,
                )?;
                K::Variant {
                    declaration,
                    index,
                    fields,
                }
            }
            _ => {
                return Err(
                    "staged invocation result cannot yet be frozen as an instance input".into(),
                );
            }
        };
        Ok(n::Expr {
            kind,
            ty: ty.clone(),
            source,
        })
    }

    fn freeze_staged_nominal_fields(
        &mut self,
        declaration: Id,
        arguments: &[Term],
        fields: Vec<Value>,
        variant: Option<u32>,
        ty: &Term,
        source: n::SourceRange,
        depth: usize,
    ) -> Result<Vec<n::Expr>, String> {
        nir::type_computations::charge_term(ty, &mut self.fuel)
            .map_err(|error| error.to_string())?;
        if !matches!(ty, Term::Nominal { declaration: expected, arguments: expected_arguments }
            if *expected == declaration && expected_arguments == arguments)
        {
            return Err("staged nominal result identity mismatch".into());
        }
        let types = nir::normal_forms::nominal_value_fields_budgeted(
            self.package,
            ty,
            variant,
            &mut self.fuel,
        )
        .ok_or("staged nominal layout is invalid or exceeds the budget")?;
        if fields.len() != types.len() {
            return Err("staged nominal field count mismatch".into());
        }
        fields
            .into_iter()
            .zip(types)
            .map(|(value, ty)| self.freeze_staged_value(value, &ty, source, depth + 1))
            .collect()
    }
}

// Inline argument frames retain ordinary evaluation order. A named template
// reference itself has no runtime evaluation, storage or environment, so its
// immutable aliases can be substituted before specializing the exposed calls.
// Do not propagate through writes, address-taking or closure capture.
pub(super) fn forward_template_aliases(
    expression: &mut n::Expr,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    fuel: &mut usize,
) -> Result<(), String> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut unavailable = BTreeSet::new();
    let mut declarations = Vec::new();
    iir::visit::expression_pruned(expression, fuel, |expr, fuel| {
        if matches!(
            expr.kind,
            K::Assign { .. } | K::AddressOf(_) | K::Closure { .. } | K::TemplateEqual(..)
        ) {
            unavailable.extend(nir::free_bindings::collect(expr, fuel)?);
            return Ok(false);
        }
        if let K::Let { binding, value } = &expr.kind {
            if matches!(value.ty.callable_signature(), Term::Function { parameters, .. }
                if parameters.iter().any(|p| p.sort != nir::Sort::Runtime))
                && matches!(
                    value.kind,
                    K::Function(_)
                        | K::FunctionInstance { .. }
                        | K::TraitMethod { .. }
                        | K::Local(_)
                        | K::If { .. }
                        | K::Match { .. }
                        | K::Block { .. }
                        | K::CallPreparation { .. }
                )
            {
                let mut remaining = *fuel as u64;
                nir::copy_budget::charge_expression(value, &mut remaining)
                    .map_err(|_| "template alias collection budget exhausted")?;
                *fuel = remaining as usize;
                declarations.push((*binding, value.as_ref().clone()));
            }
        }
        Ok(true)
    })?;
    let mut values = BTreeMap::new();
    let mut selections = BTreeMap::new();
    loop {
        let mut changed = false;
        for (binding, initializer) in &declarations {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("template alias budget exhausted")?;
            if unavailable.contains(binding) || values.contains_key(binding) {
                continue;
            }
            if matches!(
                initializer.kind,
                K::If { .. } | K::Match { .. } | K::Block { .. } | K::CallPreparation { .. }
            ) {
                let mut remaining = *fuel as u64;
                nir::copy_budget::charge_expression(initializer, &mut remaining)
                    .map_err(|_| "template selection alias budget exhausted")?;
                *fuel = remaining as usize;
                let mut candidate = initializer.clone();
                iir::visit::expression_mut_pruned(&mut candidate, fuel, |expr, fuel| {
                    if matches!(expr.kind, K::Closure { .. }) {
                        return Ok(false);
                    }
                    if let K::Local(binding) = expr.kind {
                        if let Some(value) = values.get(&binding) {
                            let mut remaining = *fuel as u64;
                            nir::copy_budget::charge_expression(value, &mut remaining)
                                .map_err(|_| "template selection alias copy budget exhausted")?;
                            *fuel = remaining as usize;
                            let source = expr.source;
                            *expr = value.clone();
                            expr.source = source;
                        }
                    }
                    Ok(true)
                })?;
                if let Some((selection, dispatch)) =
                    super::conditional_calls::freeze_template_selection(
                        &candidate, next, sources, fuel,
                    )?
                {
                    selections.insert(*binding, selection);
                    values.insert(*binding, dispatch);
                    changed = true;
                }
                continue;
            }
            let value = if let K::Local(source) = initializer.kind {
                let Some(value) = values.get(&source) else {
                    continue;
                };
                value
            } else {
                initializer
            };
            let mut remaining = *fuel as u64;
            nir::copy_budget::charge_expression(value, &mut remaining)
                .map_err(|_| "template alias copy budget exhausted")?;
            *fuel = remaining as usize;
            values.insert(*binding, value.clone());
            changed = true;
        }
        if !changed {
            break;
        }
    }
    iir::visit::expression_mut_pruned(expression, fuel, |expr, fuel| {
        if matches!(expr.kind, K::Closure { .. }) {
            return Ok(false);
        }
        if let K::Local(binding) = expr.kind {
            if let Some(value) = values.get(&binding) {
                let mut remaining = *fuel as u64;
                nir::copy_budget::charge_expression(value, &mut remaining)
                    .map_err(|_| "template alias copy budget exhausted")?;
                *fuel = remaining as usize;
                let source = expr.source;
                *expr = value.clone();
                expr.source = source;
            }
        } else if let K::Let { binding, .. } = expr.kind {
            if values.contains_key(&binding) {
                if let Some(selection) = selections.remove(&binding) {
                    *expr = selection;
                } else {
                    expr.kind = K::Unit;
                }
            }
        }
        Ok(true)
    })
}
