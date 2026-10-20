//! Reference execution of published cleanup operations on real owner storage.
use super::*;
use iir::cleanup::{Program, Step};
pub(super) struct PreparedFrame {
    pub(super) source: SourceRange,
    pub(super) values: Vec<(usize, SourceRange, Option<Storage>)>,
}
impl Interpreter<'_> {
    fn cleanup_requires_context(&mut self, signature: &Term) -> Result<bool, Error> {
        let Term::Function { parameters, result } = signature.callable_signature() else {
            return Err(self.failure(ErrorKind::InvalidCallPlan, None));
        };
        for parameter in parameters {
            if self.cleanup_callable_value(&parameter.ty, 0)? {
                return Ok(true);
            }
        }
        if let Term::Computation { effects, .. } = result.as_ref() {
            for operation in effects {
                self.tick(None)?;
                let required = self
                    .cleanup_effect_context
                    .get(operation)
                    .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, None))?;
                if *required {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    fn index_cleanup_effect_context(&mut self) -> Result<(), Error> {
        let graph = self
            .graph
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, None))?;
        let mut signatures = HashMap::new();
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let checked = (|| -> Result<(), String> {
            for function in &graph.functions {
                nir::visit::expressions(&function.function, &mut fuel, |expr, fuel| {
                    let mut insert =
                        |operation: &nir::effects::Operation, ty: &Term| -> Result<(), String> {
                            let work = type_value_work(ty, &self.normal_forms)
                                .ok_or("effect cleanup contract budget exhausted")?;
                            *fuel = fuel
                                .checked_sub(work + 1)
                                .ok_or("effect cleanup contract budget exhausted")?;
                            for argument in &operation.arguments {
                                let work = type_value_work(argument, &self.normal_forms)
                                    .ok_or("effect cleanup contract budget exhausted")?;
                                *fuel = fuel
                                    .checked_sub(work)
                                    .ok_or("effect cleanup contract budget exhausted")?;
                            }
                            if signatures
                                .get(operation)
                                .is_some_and(|previous| previous != ty)
                            {
                                return Err("conflicting effect cleanup contracts".into());
                            }
                            signatures
                                .entry(operation.clone())
                                .or_insert_with(|| ty.clone());
                            Ok(())
                        };
                    match &expr.kind {
                        K::EffectRequest {
                            operation,
                            signature,
                            ..
                        } => insert(operation, signature)?,
                        K::EffectHandle { handlers, .. } => {
                            for handler in handlers {
                                insert(&handler.operation, &handler.signature)?;
                            }
                        }
                        _ => {}
                    }
                    Ok(())
                })?;
            }
            Ok(())
        })();
        self.budget.steps -= (before - fuel) as u64;
        checked.map_err(|_| self.failure(ErrorKind::InvalidCallPlan, None))?;
        let mut contexts = HashMap::new();
        for (operation, signature) in signatures {
            let Term::Function { result, .. } = signature else {
                return Err(self.failure(ErrorKind::InvalidCallPlan, None));
            };
            contexts.insert(operation, self.cleanup_callable_value(&result, 0)?);
        }
        self.cleanup_effect_context = contexts;
        Ok(())
    }
    fn cleanup_callable_value(&mut self, ty: &Term, depth: usize) -> Result<bool, Error> {
        self.tick(None)?;
        if depth >= self.budget.depth {
            return Err(self.failure(ErrorKind::DepthExhausted, None));
        }
        if let Some(fields) = self.tuple_input_elements(ty) {
            for field in fields {
                if self.cleanup_callable_value(field, depth + 1)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        match ty {
            Term::Function { .. } | Term::Closure { .. } => return Ok(true),
            Term::Record(fields) => {
                for (_, field) in fields {
                    if self.cleanup_callable_value(field, depth + 1)? {
                        return Ok(true);
                    }
                }
            }
            Term::Nominal { .. } => {
                let graph = self
                    .graph
                    .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, None))?;
                let mut found = None;
                for nominal in &graph.nominals {
                    self.tick(None)?;
                    if &nominal.ty == ty {
                        found = Some(&nominal.definition);
                        break;
                    }
                }
                let definition =
                    found.ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, None))?;
                match definition {
                    iir::NominalDefinition::Newtype(definition) => {
                        return self.cleanup_callable_value(&definition.representation, depth + 1);
                    }
                    iir::NominalDefinition::Struct(definition) => {
                        for (_, field) in &definition.fields {
                            if self.cleanup_callable_value(field, depth + 1)? {
                                return Ok(true);
                            }
                        }
                    }
                    iir::NominalDefinition::Enum(definition) => {
                        for variant in &definition.variants {
                            for field in &variant.fields {
                                if self.cleanup_callable_value(field, depth + 1)? {
                                    return Ok(true);
                                }
                            }
                        }
                    }
                }
            }
            _ => (),
        }
        Ok(false)
    }
    /// Execute a resource-verified program. A failed destructor stops subsequent
    /// operations; storage retains completed writes/releases. No automatic retry
    /// or exception cleanup is implied. Currently requires a fully typed input
    /// value; partial-value holes and closure-environment projection are pending.
    pub fn execute_cleanup(&mut self, program: &Program, storage: &Storage) -> Result<(), Error> {
        if let Some(graph) = self.graph {
            let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let before = fuel;
            let control = graph.validate_cleanup_control(&mut fuel);
            self.budget.steps -= (before - fuel) as u64;
            control.map_err(|_| self.failure(ErrorKind::Unsupported, None))?;
        }
        self.execute_cleanup_at(program, storage, &[])
    }
    fn execute_cleanup_at(
        &mut self,
        program: &Program,
        storage: &Storage,
        base: &[usize],
    ) -> Result<(), Error> {
        if self.graph.is_none() || self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        self.tick(None)?;
        self.cleanup_path_work(base)?;
        let valid = storage
            .owned_pointer(program.ty.clone(), base)
            .and_then(|pointer| pointer.inspect(|value| self.has_type(value, &program.ty)))
            .map_err(|e| self.failure(e, None))?;
        if !valid {
            return Err(self.failure(ErrorKind::Type, None));
        }
        for (path, tag) in &program.guards {
            self.tick(None)?;
            let path: Vec<_> = base.iter().chain(path).copied().collect();
            self.cleanup_path_work(&path)?;
            let pointer = storage
                .owned_pointer(Term::Primitive(Primitive::Void), &path)
                .map_err(|e| self.failure(e, None))?;
            if !pointer
                .inspect(|value| matches!(value, Value::Variant {index,..} if index == tag))
                .map_err(|e| self.failure(e, None))?
            {
                return Err(self.failure(ErrorKind::InvalidCallPlan, None));
            }
        }
        self.cleanup_steps(&program.steps, storage, base, 0)
    }
    fn cleanup_path_work(&mut self, path: &[usize]) -> Result<(), Error> {
        for _ in path {
            self.tick(None)?;
        }
        Ok(())
    }
    fn cleanup_steps(
        &mut self,
        steps: &[Step],
        storage: &Storage,
        base: &[usize],
        depth: usize,
    ) -> Result<(), Error> {
        if depth >= self.budget.depth {
            return Err(self.failure(ErrorKind::DepthExhausted, None));
        }
        for step in steps {
            self.tick(None)?;
            match step {
                Step::Select { cases } => {
                    let mut selected = None;
                    for case in cases {
                        self.tick(None)?;
                        let mut matches = true;
                        for (fields, tag) in &case.guards {
                            self.tick(None)?;
                            let path: Vec<_> = base.iter().chain(fields).copied().collect();
                            self.cleanup_path_work(&path)?;
                            let index = storage
                                .owned_pointer(Term::Primitive(Primitive::Void), &path)
                                .and_then(|pointer| {
                                    pointer.inspect(|value| match value {
                                        Value::Variant { index, .. } => Some(*index),
                                        _ => None,
                                    })
                                })
                                .map_err(|e| self.failure(e, None))?
                                .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, None))?;
                            if index != *tag {
                                matches = false;
                                break;
                            }
                        }
                        if matches {
                            if selected.replace(case).is_some() {
                                return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                            }
                        }
                    }
                    let case =
                        selected.ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, None))?;
                    self.cleanup_steps(&case.steps, storage, base, depth + 1)?;
                }
                Step::IfConstructed { path, steps } => {
                    let path: Vec<_> = base.iter().chain(path).copied().collect();
                    self.cleanup_path_work(&path)?;
                    let pointer = storage
                        .owned_pointer(Term::Primitive(Primitive::Void), &path)
                        .map_err(|e| self.failure(e, None))?;
                    if pointer
                        .is_constructed()
                        .map_err(|e| self.failure(e, None))?
                    {
                        self.cleanup_steps(steps, storage, base, depth + 1)?;
                    }
                }
                Step::Invoke {
                    path,
                    subject,
                    method,
                } => {
                    let path: Vec<_> = base.iter().chain(path).copied().collect();
                    self.cleanup_path_work(&path)?;
                    let pointer = storage
                        .owned_pointer(subject.clone(), &path)
                        .map_err(|e| self.failure(e, None))?;
                    let result = self.invoke_instance(method, vec![Value::Pointer(pointer)])?;
                    if result != Value::Unit {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                }
                Step::Switch { path, branches } => {
                    let path: Vec<_> = base.iter().chain(path).copied().collect();
                    self.cleanup_path_work(&path)?;
                    let pointer = storage
                        .owned_pointer(Term::Primitive(Primitive::Void), &path)
                        .map_err(|e| self.failure(e, None))?;
                    let index = pointer
                        .inspect(|value| match value {
                            Value::Variant { index, .. } => Some(*index),
                            _ => None,
                        })
                        .map_err(|e| self.failure(e, None))?
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    let branch = branches
                        .get(index)
                        .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, None))?;
                    self.cleanup_steps(branch, storage, base, depth + 1)?;
                }
                Step::Release { path, ty } => {
                    let path: Vec<_> = base.iter().chain(path).copied().collect();
                    self.cleanup_path_work(&path)?;
                    let pointer = storage
                        .owned_pointer(ty.clone(), &path)
                        .map_err(|e| self.failure(e, None))?;
                    pointer.release().map_err(|e| self.failure(e, None))?;
                }
            }
        }
        Ok(())
    }
}

impl<'a> Interpreter<'a> {
    pub(super) fn cleanup_delivery(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
        value: &mut Value,
    ) -> Result<(), Error> {
        if self.cleanup_schedule.is_none() {
            return Ok(());
        }
        let mut base = expr;
        let mut fields = vec![];
        loop {
            match &base.kind {
                K::Local(binding) => {
                    fields.reverse();
                    self.cleanup_copy_delivery(expr.source, *binding, &fields, value)?;
                    return self.cleanup_transfer(expr.source, *binding, &fields, environment);
                }
                K::Field { value, index, .. } | K::AnonymousField { value, index, .. } => {
                    self.tick(Some(expr.source))?;
                    fields.push(*index);
                    base = value;
                }
                _ => return Ok(()),
            }
        }
    }
    pub(super) fn cleanup_copy_delivery(
        &mut self,
        source: SourceRange,
        binding: LocalId,
        fields: &[usize],
        value: &mut Value,
    ) -> Result<(), Error> {
        let Some(schedule) = self.cleanup_schedule else {
            return Ok(());
        };
        let mut matched = false;
        for scope in &schedule.functions {
            self.tick(Some(source))?;
            if Some(&scope.instance) != self.cleanup_frame.as_ref()
                || scope.context != self.cleanup_context
            {
                continue;
            }
            for copy in &scope.copies {
                self.tick(Some(source))?;
                if copy.source == source && copy.binding == binding && copy.fields == fields {
                    if matched {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
                    }
                    matched = true;
                }
            }
            break;
        }
        if matched {
            *value = self.copy_representation(value, source, 0)?;
        }
        Ok(())
    }
    // Code and external pointer targets retain identity. Owned capture storage
    // receives fresh cells, recursively, so copying a callable does not make its
    // receiver state alias the source. Capability checking happens in publication.
    fn copy_work(&mut self, work: usize, source: SourceRange) -> Result<(), Error> {
        let work = u64::try_from(work)
            .map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
        self.budget.steps = self
            .budget
            .steps
            .checked_sub(work)
            .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
        Ok(())
    }
    fn copy_type_work(&mut self, ty: &Term, source: SourceRange) -> Result<(), Error> {
        let work = type_value_work(ty, &self.normal_forms)
            .ok_or_else(|| self.failure(ErrorKind::RepresentationSizeLimit, Some(source)))?;
        self.copy_work(work, source)
    }
    fn copy_invocation_type_work(
        &mut self,
        ty: &Term,
        source: SourceRange,
        invocation: bool,
    ) -> Result<(), Error> {
        if !invocation {
            return self.copy_type_work(ty, source);
        }
        let work = self
            .type_description_work(ty)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        self.copy_work(work, source)
    }

    fn copy_instance_arguments(
        &mut self,
        arguments: &[Term],
        source: SourceRange,
    ) -> Result<Vec<Term>, Error> {
        self.copy_work(arguments.len(), source)?;
        for argument in arguments {
            let work = scoped_term_value_work_with_layouts(
                argument,
                &self.normal_forms,
                &self.abstract_type_parameters,
                |ty, variant, fuel| self.normal_value_fields(ty, variant, fuel),
            )
            .ok_or_else(|| self.failure(ErrorKind::RepresentationSizeLimit, Some(source)))?;
            self.copy_work(work, source)?;
        }
        Ok(arguments.to_vec())
    }
    pub(super) fn copy_representation(
        &mut self,
        value: &Value,
        source: SourceRange,
        depth: usize,
    ) -> Result<Value, Error> {
        self.clone_representation(value, source, depth, false)
    }

    pub(super) fn checkpoint_representation(
        &mut self,
        value: &Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        self.clone_representation(value, source, 0, true)
    }

    fn clone_representation(
        &mut self,
        value: &Value,
        source: SourceRange,
        depth: usize,
        preserve_identity: bool,
    ) -> Result<Value, Error> {
        self.tick(Some(source))?;
        if depth >= 128
            || self
                .depth
                .checked_add(depth)
                .is_none_or(|depth| depth >= self.budget.depth)
        {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        Ok(match value {
            Value::Storage(_) | Value::Closure(_) if preserve_identity => value.clone(),
            Value::Storage(storage) => {
                let limit = self
                    .budget
                    .depth
                    .saturating_sub(self.depth.saturating_add(depth));
                let contents = storage
                    .contents_for_copy(&mut self.budget.steps, limit)
                    .map_err(|kind| self.failure(kind, Some(source)))?;
                Value::Storage(Storage::new(self.clone_representation(
                    contents.value(),
                    source,
                    depth + 1,
                    preserve_identity,
                )?))
            }
            Value::Closure(closure) => {
                self.copy_work(closure.environment.len(), source)?;
                self.copy_work(closure.parameters.len(), source)?;
                self.copy_work(closure.capture_order.len(), source)?;
                self.copy_type_work(&closure.ty, source)?;
                let arguments = closure
                    .instance_arguments
                    .as_ref()
                    .map(|arguments| self.copy_instance_arguments(arguments, source))
                    .transpose()?;
                let mut environment = Environment::new();
                for (binding, value) in &closure.environment {
                    environment.insert(
                        *binding,
                        self.clone_representation(value, source, depth + 1, preserve_identity)?,
                    );
                }
                Value::Closure(std::sync::Arc::new(Closure {
                    definition_source: closure.definition_source,
                    origin: closure.origin,
                    instance_arguments: arguments,
                    parameters: closure.parameters.clone(),
                    environment,
                    capture_order: closure.capture_order.clone(),
                    body: closure.body.clone(),
                    ty: closure.ty.clone(),
                }))
            }
            Value::Tuple(values) => {
                self.copy_work(values.len(), source)?;
                Value::Tuple(
                    values
                        .iter()
                        .map(|value| {
                            self.clone_representation(value, source, depth + 1, preserve_identity)
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
            Value::AnonymousRecord(values) => {
                self.copy_work(values.len(), source)?;
                Value::AnonymousRecord(
                    values
                        .iter()
                        .map(|(name, value)| {
                            self.copy_work(name.len(), source)?;
                            Ok((
                                name.clone(),
                                self.clone_representation(
                                    value,
                                    source,
                                    depth + 1,
                                    preserve_identity,
                                )?,
                            ))
                        })
                        .collect::<Result<_, Error>>()?,
                )
            }
            Value::Newtype { ty, value } => {
                self.copy_type_work(ty, source)?;
                Value::Newtype {
                    ty: ty.clone(),
                    value: Box::new(self.clone_representation(
                        value,
                        source,
                        depth + 1,
                        preserve_identity,
                    )?),
                }
            }
            Value::Record {
                declaration,
                arguments,
                fields,
            } => {
                self.copy_work(fields.len(), source)?;
                let arguments = self.copy_instance_arguments(arguments, source)?;
                Value::Record {
                    declaration: *declaration,
                    arguments,
                    fields: fields
                        .iter()
                        .map(|value| {
                            self.clone_representation(value, source, depth + 1, preserve_identity)
                        })
                        .collect::<Result<_, _>>()?,
                }
            }
            Value::Variant {
                declaration,
                arguments,
                index,
                fields,
            } => {
                self.copy_work(fields.len(), source)?;
                let arguments = self.copy_instance_arguments(arguments, source)?;
                Value::Variant {
                    declaration: *declaration,
                    arguments,
                    index: *index,
                    fields: fields
                        .iter()
                        .map(|value| {
                            self.clone_representation(value, source, depth + 1, preserve_identity)
                        })
                        .collect::<Result<_, _>>()?,
                }
            }
            Value::Pointer(pointer) => {
                self.copy_type_work(&pointer.pointee, source)?;
                self.copy_work(
                    pointer.copy_metadata_work().ok_or_else(|| {
                        self.failure(ErrorKind::RepresentationSizeLimit, Some(source))
                    })?,
                    source,
                )?;
                value.clone()
            }
            Value::Type(ty) | Value::AbstractValue { ty, .. } => {
                self.copy_invocation_type_work(ty, source, preserve_identity)?;
                value.clone()
            }
            Value::TypeList(types) => {
                self.copy_work(types.len(), source)?;
                for ty in types {
                    self.copy_invocation_type_work(ty, source, preserve_identity)?;
                }
                Value::TypeList(types.clone())
            }
            Value::RuntimeFunction {
                declaration,
                arguments,
            } => Value::RuntimeFunction {
                declaration: *declaration,
                arguments: self.copy_instance_arguments(arguments, source)?,
            },
            Value::RuntimeTraitMethod(method) => {
                self.copy_invocation_type_work(&method.subject, source, true)?;
                let interface_arguments =
                    self.copy_instance_arguments(&method.interface.arguments, source)?;
                let method_arguments =
                    self.copy_instance_arguments(&method.method_arguments, source)?;
                Value::RuntimeTraitMethod(Box::new(TraitMethodIdentity {
                    interface: InterfaceInstance {
                        declaration: method.interface.declaration,
                        arguments: interface_arguments,
                    },
                    subject: method.subject.clone(),
                    member: method.member,
                    method_arguments,
                }))
            }
            Value::Float(number) => {
                self.copy_work(number.digits().len(), source)?;
                self.copy_work(number.exponent().bit_len().div_ceil(32), source)?;
                value.clone()
            }
            Value::Integer(integer) => {
                self.copy_work(integer.bit_len().div_ceil(32), source)?;
                value.clone()
            }
            Value::BitVec(bits) => {
                self.copy_work(bits.words().len(), source)?;
                value.clone()
            }
            // Strings and code bodies are immutable shared representations.
            value => value.clone(),
        })
    }
    pub(super) fn cleanup_transfer(
        &mut self,
        source: SourceRange,
        binding: LocalId,
        fields: &[usize],
        environment: &mut Environment,
    ) -> Result<(), Error> {
        let Some(schedule) = self.cleanup_schedule else {
            return Ok(());
        };
        let mut matched = false;
        let mut found_scope = false;
        for scope in &schedule.functions {
            self.tick(Some(source))?;
            if Some(&scope.instance) != self.cleanup_frame.as_ref()
                || scope.context != self.cleanup_context
            {
                continue;
            }
            found_scope = true;
            for transfer in &scope.transfers {
                self.tick(Some(source))?;
                if transfer.source == source
                    && transfer.binding == binding
                    && transfer.fields == fields
                {
                    if matched {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
                    }
                    matched = true;
                }
            }
            break;
        }
        if !found_scope {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
        }
        if !matched {
            return Ok(());
        }
        self.cleanup_path_work(fields)?;
        let value = environment
            .get_mut(&binding)
            .ok_or_else(|| self.failure(ErrorKind::MissingBinding, Some(source)))?;
        if !matches!(value, Value::Storage(_)) {
            *value = Value::Storage(Storage::new(std::mem::replace(value, Value::Unit)));
        }
        let Value::Storage(storage) = value else {
            unreachable!()
        };
        storage
            .owned_pointer(Term::Primitive(Primitive::Void), fields)
            .and_then(|pointer| pointer.transfer())
            .map_err(|e| self.failure(e, Some(source)))
    }
    pub(super) fn invocation_cleanup_context(
        &mut self,
        function: &Value,
        source: SourceRange,
    ) -> Result<Option<(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>, Error> {
        let Some(schedule) = self.cleanup_schedule else {
            return Ok(None);
        };
        let named = match function {
            Value::Function(declaration) => Some(iir::InstanceKey {
                declaration: *declaration,
                arguments: vec![],
            }),
            Value::Closure(closure) => closure.origin.map(|declaration| iir::InstanceKey {
                declaration,
                arguments: closure.instance_arguments.clone().unwrap_or_default(),
            }),
            _ => None,
        };
        if let Some(callee) = named {
            let definition = self
                .graph
                .and_then(|g| g.function(&callee))
                .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
            if !self.cleanup_requires_context(&definition.function.signature)? {
                return Ok(None);
            }
            let instance = self
                .cleanup_frame
                .clone()
                .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
            let mut context = self.cleanup_context.clone();
            if context.len() >= self.budget.depth {
                return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
            }
            context.push(iir::cleanup::ContextKey::Function {
                call: source,
                callee,
            });
            for alias in &schedule.aliases {
                self.tick(Some(source))?;
                if alias.instance == instance && alias.context == context {
                    return Ok(Some((instance, alias.target.clone())));
                }
            }
            for scope in &schedule.functions {
                self.tick(Some(source))?;
                if scope.instance == instance && scope.context == context {
                    return Ok(Some((instance, context)));
                }
            }
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
        }
        let Value::Closure(closure) = function else {
            return Ok(None);
        };
        let invalid = || Error {
            kind: ErrorKind::InvalidCallPlan,
            source: Some(source),
        };
        let instance = self.cleanup_frame.clone().ok_or_else(invalid)?;
        let definition = closure.definition_source.ok_or_else(invalid)?;
        if self.cleanup_context.len() >= self.budget.depth {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        let mut context = self.cleanup_context.clone();
        context.push(iir::cleanup::ContextKey::Closure {
            call: source,
            definition,
        });
        for scope in &schedule.functions {
            self.tick(Some(source))?;
            if scope.instance == instance && scope.context == context {
                return Ok(Some((instance, context)));
            }
        }
        Err(invalid())
    }
    pub(super) fn cleanup_block(
        &mut self,
        source: SourceRange,
        environment: &mut Environment,
    ) -> Result<(), Error> {
        let Some(schedule) = self.cleanup_schedule else {
            return Ok(());
        };
        let mut boundaries = vec![];
        for scope in &schedule.functions {
            self.tick(Some(source))?;
            if Some(&scope.instance) != self.cleanup_frame.as_ref()
                || scope.context != self.cleanup_context
            {
                continue;
            }
            for site in &scope.sites {
                self.tick(Some(source))?;
                if site.source == source
                    && matches!(
                        site.boundary,
                        iir::cleanup::Boundary::ScopeEnd | iir::cleanup::Boundary::TemporaryEnd
                    )
                {
                    boundaries.push(site.boundary.clone());
                }
            }
        }
        // Function-result and default/full-expression scopes end in different
        // orders. Preserve the analyzer's order instead of imposing one here.
        for boundary in boundaries {
            self.cleanup_site(source, boundary, environment)?;
        }
        Ok(())
    }
    pub(super) fn retain_residual(
        &mut self,
        source: SourceRange,
        value: &Value,
    ) -> Result<(), Error> {
        self.retain_residual_with_bindings(source, value, None)
    }
    pub(super) fn retain_pattern_residual(
        &mut self,
        source: SourceRange,
        value: &Value,
        bindings: &PatternBindings,
    ) -> Result<(), Error> {
        self.retain_residual_with_bindings(source, value, Some(bindings))
    }
    fn retain_residual_with_bindings(
        &mut self,
        source: SourceRange,
        value: &Value,
        bindings: Option<&PatternBindings>,
    ) -> Result<(), Error> {
        let Some(schedule) = self.cleanup_schedule else {
            return Ok(());
        };
        let mut needed = false;
        let mut moved = BTreeMap::<LocalId, Vec<Vec<usize>>>::new();
        for scope in &schedule.functions {
            self.tick(Some(source))?;
            if Some(&scope.instance) != self.cleanup_frame.as_ref()
                || scope.context != self.cleanup_context
            {
                continue;
            }
            for order in &scope.pattern_orders {
                self.tick(Some(source))?;
                if order.source != source {
                    continue;
                }
                let receipt = bindings
                    .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
                let mut ranked = Vec::new();
                for (binding, rank) in &receipt.1 {
                    self.tick(Some(source))?;
                    ranked.push((*rank, *binding));
                }
                ranked.sort();
                let actual: Vec<_> = ranked.into_iter().map(|(_, binding)| binding).collect();
                let mut valid = false;
                for permitted in &order.alternatives {
                    for _ in permitted {
                        self.tick(Some(source))?;
                    }
                    valid |= permitted == &actual;
                }
                if !valid {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
                }
                self.pattern_binding_orders
                    .insert((source.start, source.end), actual);
            }
            for transfer in &scope.pattern_transfers {
                self.tick(Some(source))?;
                if transfer.source == source {
                    self.cleanup_path_work(&transfer.fields)?;
                    moved
                        .entry(transfer.binding)
                        .or_default()
                        .push(transfer.fields.clone());
                }
            }
            for site in &scope.sites {
                for action in &site.actions {
                    self.tick(Some(source))?;
                    if matches!(action.target, iir::cleanup::Target::Temporary {source: origin, ..} if origin==source)
                    {
                        needed = true;
                    }
                }
            }
        }
        if needed {
            let storage = Storage::new(value.clone());
            for (binding, permitted) in moved {
                let actual = bindings
                    .and_then(|bindings| bindings.0.get(&binding))
                    .and_then(|path| path.as_ref())
                    .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
                self.cleanup_path_work(actual)?;
                if !permitted.contains(actual) {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
                }
                storage
                    .owned_pointer(Term::Primitive(Primitive::Void), actual)
                    .and_then(|pointer| pointer.transfer())
                    .map_err(|e| self.failure(e, Some(source)))?;
            }
            self.residual_temporaries.push((source, storage));
        }
        Ok(())
    }

    pub(super) fn check_scheduled_result(
        &mut self,
        value: &Value,
        origin: SourceRange,
        environment: &mut Environment,
    ) -> Result<(), Error> {
        if self.cleanup_schedule.is_none() {
            return Ok(());
        }
        let mut key = self.cleanup_frame.as_ref();
        for context in self.cleanup_context.iter().rev() {
            match context {
                iir::cleanup::ContextKey::Closure { .. } => return Ok(()),
                // Lexical return from a handler must first leave its private
                // resource scope, then check the installation's caller result.
                iir::cleanup::ContextKey::EffectHandler { .. } => {
                    return Ok(());
                }
                iir::cleanup::ContextKey::Function { callee, .. } => {
                    key = Some(callee);
                    break;
                }
                iir::cleanup::ContextKey::Default { .. } => (),
            }
        }
        let key = key.ok_or(Error {
            kind: ErrorKind::InvalidCallPlan,
            source: Some(origin),
        })?;
        let function = &self
            .graph
            .and_then(|g| g.function(key))
            .ok_or(Error {
                kind: ErrorKind::InvalidCallPlan,
                source: Some(origin),
            })?
            .function;
        let conditions = function.postconditions.clone();
        if conditions.is_empty() {
            return Ok(());
        }
        let binding = function.result_binding;
        if let Some(binding) = binding {
            environment.insert(binding, value.clone());
        }
        let result = self.with_prepared_frame(origin, |this| {
            if binding.is_none() {
                this.retain_prepared(0, origin, value)?;
            }
            for condition in &conditions {
                if !nir::program::checkable_predicate(condition) {
                    return Err(this.failure(ErrorKind::Type, Some(condition.source)));
                }
                let Flow::Value(Value::Bool(passed)) = this.eval(condition, environment)? else {
                    return Err(this.failure(ErrorKind::Type, Some(condition.source)));
                };
                this.events.push(Event::CheckPredicate {
                    source: condition.source,
                    passed,
                });
                if !passed {
                    this.cleanup_site(
                        condition.source,
                        iir::cleanup::Boundary::PostconditionFailure(origin),
                        environment,
                    )?;
                    return Err(
                        this.failure(ErrorKind::ProgramContractRejected, Some(condition.source))
                    );
                }
            }
            Ok(Flow::Value(Value::Unit))
        });
        if let Some(binding) = binding {
            environment.remove(&binding);
        }
        result.map(|_| ())
    }
    pub(super) fn with_prepared_frame(
        &mut self,
        source: SourceRange,
        run: impl FnOnce(&mut Self) -> Result<Flow, Error>,
    ) -> Result<Flow, Error> {
        if self.cleanup_schedule.is_none() {
            return run(self);
        }
        let depth = self.prepared_frames.len();
        self.prepared_frames.push(PreparedFrame {
            source,
            values: vec![],
        });
        let result = run(self);
        self.prepared_frames.truncate(depth);
        result
    }
    pub(super) fn retain_prepared(
        &mut self,
        slot: usize,
        source: SourceRange,
        value: &Value,
    ) -> Result<(), Error> {
        if self.cleanup_schedule.is_none() {
            return Ok(());
        }
        self.tick(Some(source))?;
        let frame = self.prepared_frames.last_mut().ok_or(Error {
            kind: ErrorKind::InvalidCallPlan,
            source: Some(source),
        })?;
        if frame
            .values
            .iter()
            .any(|(previous, _, _)| *previous == slot)
        {
            return Err(Error {
                kind: ErrorKind::InvalidCallPlan,
                source: Some(source),
            });
        }
        frame
            .values
            .push((slot, source, Some(Storage::new(value.clone()))));
        Ok(())
    }
    pub(super) fn prepare_argument(
        &mut self,
        slot: usize,
        source: SourceRange,
        value: Value,
    ) -> Result<Value, Error> {
        if self.cleanup_schedule.is_none() {
            return Ok(value);
        }
        self.tick(Some(source))?;
        let storage = Storage::new(value);
        let frame = self.prepared_frames.last_mut().ok_or(Error {
            kind: ErrorKind::InvalidCallPlan,
            source: Some(source),
        })?;
        if frame.values.iter().any(|(index, _, _)| *index == slot) {
            return Err(Error {
                kind: ErrorKind::InvalidCallPlan,
                source: Some(source),
            });
        }
        frame.values.push((slot, source, Some(storage.clone())));
        Ok(Value::Storage(storage))
    }
    pub(super) fn evaluate_default(
        &mut self,
        construction: SourceRange,
        argument: &nir::program::CallArgument,
        defaults: &mut Environment,
        caller: &mut Environment,
    ) -> Result<Flow, Error> {
        if self.cleanup_schedule.is_none() {
            return self.eval(&argument.value, defaults);
        }
        if self.cleanup_context.len() >= self.budget.depth {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(argument.value.source)));
        }
        // Validate the environment before evaluating even a literal default.
        // Otherwise an absent schedule can go unnoticed until the first cleanup
        // hook, after effects in the default have already run.
        let key = iir::cleanup::ContextKey::Default {
            construction,
            parameter: argument.parameter,
        };
        let mut found = false;
        for scope in &self.cleanup_schedule.unwrap().functions {
            self.tick(Some(argument.value.source))?;
            if Some(&scope.instance) == self.cleanup_frame.as_ref()
                && scope.context.len() == self.cleanup_context.len() + 1
                && scope.context.starts_with(&self.cleanup_context)
                && scope.context.last() == Some(&key)
            {
                found = true;
                break;
            }
        }
        if !found {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(argument.value.source)));
        }
        let caller_residuals = std::mem::take(&mut self.residual_temporaries);
        let caller_orders = std::mem::take(&mut self.pattern_binding_orders);
        let caller_frames = std::mem::take(&mut self.prepared_frames);
        self.cleanup_context.push(key);
        let result = self.eval(&argument.value, defaults);
        self.cleanup_context.pop();
        self.prepared_frames = caller_frames;
        self.residual_temporaries = caller_residuals;
        self.pattern_binding_orders = caller_orders;
        match result {
            Err(error)
                if error.kind == ErrorKind::ProgramContractRejected && !self.cleanup_failed =>
            {
                self.cleanup_site(
                    construction,
                    iir::cleanup::Boundary::DefaultFailure(argument.parameter),
                    caller,
                )?;
                Err(error)
            }
            other => other,
        }
    }
    pub(super) fn transfer_prepared(&mut self) {
        if let Some(frame) = self.prepared_frames.last_mut() {
            frame.values.clear();
        }
    }

    /// Attach the schedule published with this exact concrete instance graph.
    pub fn with_cleanup_schedule(
        mut self,
        schedule: &'a iir::cleanup::Schedule,
    ) -> Result<Self, Error> {
        let graph = self
            .graph
            .ok_or_else(|| self.failure(ErrorKind::PhaseMismatch, None))?;
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let control = graph.validate_cleanup_control(&mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        control.map_err(|_| self.failure(ErrorKind::Unsupported, None))?;
        self.index_cleanup_effect_context()?;
        for function in &graph.functions {
            let mut matches = 0;
            for candidate in &schedule.functions {
                self.tick(None)?;
                matches +=
                    usize::from(candidate.instance == function.key && candidate.context.is_empty());
            }
            let contextual = self.cleanup_requires_context(&function.function.signature)?;
            if matches != usize::from(!contextual) {
                return Err(self.failure(ErrorKind::InvalidCallPlan, None));
            }
        }
        for function in &schedule.functions {
            self.tick(None)?;
            if graph.function(&function.instance).is_none() {
                return Err(self.failure(ErrorKind::InvalidCallPlan, None));
            }
            for context in &function.context {
                if let iir::cleanup::ContextKey::Function { callee, .. } = context
                    && graph.function(callee).is_none()
                {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                }
            }
            for (index, copy) in function.copies.iter().enumerate() {
                self.cleanup_path_work(&copy.fields)?;
                for previous in function.copies[..index].iter().chain(&function.transfers) {
                    self.tick(Some(copy.source))?;
                    if previous == copy {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, Some(copy.source)));
                    }
                }
            }
            for (index, transfer) in function.transfers.iter().enumerate() {
                self.cleanup_path_work(&transfer.fields)?;
                for previous in &function.transfers[..index] {
                    self.tick(Some(transfer.source))?;
                    if previous == transfer {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, Some(transfer.source)));
                    }
                }
            }
            let mut pattern_positions = BTreeSet::new();
            for transfer in &function.pattern_transfers {
                self.tick(Some(transfer.source))?;
                self.cleanup_path_work(&transfer.fields)?;
                if !pattern_positions.insert((
                    transfer.source.start,
                    transfer.source.end,
                    transfer.binding,
                    &transfer.fields,
                )) {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, Some(transfer.source)));
                }
            }
            let mut order_sources = BTreeSet::new();
            let mut order_members = BTreeSet::new();
            for group in &function.pattern_orders {
                self.tick(Some(group.source))?;
                if group.alternatives.len() < 2
                    || !order_sources.insert((group.source.start, group.source.end))
                {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, Some(group.source)));
                }
                let mut common = None;
                let mut distinct = BTreeSet::new();
                for order in &group.alternatives {
                    let mut members = BTreeSet::new();
                    for binding in order {
                        self.tick(Some(group.source))?;
                        if !members.insert(*binding) {
                            return Err(
                                self.failure(ErrorKind::InvalidCallPlan, Some(group.source))
                            );
                        }
                    }
                    if members.is_empty()
                        || !distinct.insert(order)
                        || common.as_ref().is_some_and(|previous| previous != &members)
                    {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, Some(group.source)));
                    }
                    common = Some(members);
                }
                for binding in common.unwrap() {
                    if !order_members.insert(binding) {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, Some(group.source)));
                    }
                }
            }
        }
        for (index, function) in schedule.functions.iter().enumerate() {
            for previous in &schedule.functions[..index] {
                self.tick(None)?;
                if previous.instance == function.instance && previous.context == function.context {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                }
            }
            if !function.context.is_empty() {
                let mut parent = false;
                for candidate in &schedule.functions {
                    self.tick(None)?;
                    parent |= candidate.instance == function.instance
                        && candidate.context == function.context[..function.context.len() - 1];
                }
                if !parent {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                }
            }
        }
        for (index, alias) in schedule.aliases.iter().enumerate() {
            for _ in alias.context.iter().chain(&alias.target) {
                self.tick(None)?;
            }
            let same_callee = matches!((alias.context.last(),alias.target.last()),
                (Some(iir::cleanup::ContextKey::Function {callee:left,..}),Some(iir::cleanup::ContextKey::Function {callee:right,..})) if left==right);
            if !same_callee
                || alias.target.len() >= alias.context.len()
                || !alias.context.starts_with(&alias.target)
            {
                return Err(self.failure(ErrorKind::InvalidCallPlan, None));
            }
            let mut target = false;
            let mut parent = false;
            for scope in &schedule.functions {
                self.tick(None)?;
                if scope.instance != alias.instance {
                    continue;
                }
                if scope.context == alias.context {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                }
                target |= scope.context == alias.target;
                parent |= scope.context == alias.context[..alias.context.len() - 1];
            }
            if !target || !parent {
                return Err(self.failure(ErrorKind::InvalidCallPlan, None));
            }
            for previous in &schedule.aliases[..index] {
                self.tick(None)?;
                if previous.instance == alias.instance && previous.context == alias.context {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                }
            }
        }
        self.cleanup_schedule = Some(schedule);
        Ok(self)
    }
    pub(super) fn cleanup_discard(
        &mut self,
        source: SourceRange,
        value_source: SourceRange,
        value: Value,
    ) -> Result<(), Error> {
        let Some(schedule) = self.cleanup_schedule else {
            return Ok(());
        };
        let key = self
            .cleanup_frame
            .clone()
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
        for function in &schedule.functions {
            self.tick(Some(source))?;
            if function.instance != key || function.context != self.cleanup_context {
                continue;
            }
            for discard in &function.discards {
                self.tick(Some(source))?;
                if discard.source != source {
                    continue;
                }
                if discard.value_source != value_source {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
                }
                let storage = Storage::new(value);
                if let Err(error) = self.execute_cleanup(&discard.program, &storage) {
                    self.cleanup_failed = true;
                    return Err(error);
                }
                return Ok(());
            }
            return Ok(());
        }
        Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)))
    }
    pub(super) fn cleanup_site(
        &mut self,
        source: SourceRange,
        boundary: iir::cleanup::Boundary,
        environment: &mut Environment,
    ) -> Result<(), Error> {
        let Some(schedule) = self.cleanup_schedule else {
            return Ok(());
        };
        let key = self
            .cleanup_frame
            .clone()
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
        let mut found = None;
        for candidate in &schedule.functions {
            self.tick(Some(source))?;
            if candidate.instance == key && candidate.context == self.cleanup_context {
                found = Some(candidate);
                break;
            }
        }
        let function =
            found.ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
        for site in &function.sites {
            self.tick(Some(source))?;
            if site.source != source || site.boundary != boundary {
                continue;
            }
            let mut ordered: Vec<_> = site.actions.iter().collect();
            if site.boundary != iir::cleanup::Boundary::Replacement {
                for group in &function.pattern_orders {
                    self.tick(Some(source))?;
                    let Some(bindings) = group.alternatives.first() else {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
                    };
                    let members: BTreeSet<_> = bindings.iter().copied().collect();
                    let mut positions = vec![];
                    for (index, action) in ordered.iter().enumerate() {
                        self.tick(Some(source))?;
                        if matches!(&action.target, iir::cleanup::Target::Local { binding, fields }
                            if fields.is_empty() && members.contains(binding))
                        {
                            positions.push(index);
                        }
                    }
                    if positions.len() < 2 {
                        continue;
                    }
                    let actual = self
                        .pattern_binding_orders
                        .get(&(group.source.start, group.source.end))
                        .cloned()
                        .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
                    let ranks: BTreeMap<_, _> = actual
                        .into_iter()
                        .enumerate()
                        .map(|(rank, binding)| (binding, rank))
                        .collect();
                    let mut actions = vec![];
                    for index in &positions {
                        self.tick(Some(source))?;
                        let action = ordered[*index];
                        let iir::cleanup::Target::Local { binding, .. } = action.target else {
                            unreachable!()
                        };
                        let rank = ranks.get(&binding).copied().ok_or_else(|| {
                            self.failure(ErrorKind::InvalidCallPlan, Some(source))
                        })?;
                        actions.push((std::cmp::Reverse(rank), action));
                    }
                    actions.sort_by_key(|(rank, _)| *rank);
                    for (index, (_, action)) in positions.into_iter().zip(actions) {
                        ordered[index] = action;
                    }
                }
            }
            for action in ordered {
                self.tick(Some(source))?;
                let (storage, fields) = match &action.target {
                    iir::cleanup::Target::Local { binding, fields } => {
                        let value = environment.get_mut(binding).ok_or(Error {
                            kind: ErrorKind::MissingBinding,
                            source: Some(source),
                        })?;
                        let storage = match value {
                            Value::Storage(storage) => storage.clone(),
                            value => {
                                let storage = Storage::new(std::mem::replace(value, Value::Unit));
                                *value = Value::Storage(storage.clone());
                                storage
                            }
                        };
                        (storage, fields.as_slice())
                    }
                    iir::cleanup::Target::Temporary {
                        source: origin,
                        optional,
                    } => {
                        let mut found = None;
                        for index in (0..self.residual_temporaries.len()).rev() {
                            self.tick(Some(source))?;
                            if self.residual_temporaries[index].0 == *origin {
                                found = Some(index);
                                break;
                            }
                        }
                        if found.is_none() && *optional {
                            continue;
                        }
                        let index = found.ok_or(Error {
                            kind: ErrorKind::InvalidCallPlan,
                            source: Some(source),
                        })?;
                        (self.residual_temporaries.remove(index).1, &[][..])
                    }
                    iir::cleanup::Target::Prepared {
                        construction,
                        slot,
                        value_source,
                    } => {
                        let mut found = None;
                        let fuel = &mut self.budget.steps;
                        for frame in self.prepared_frames.iter_mut().rev() {
                            *fuel = fuel.checked_sub(1).ok_or(Error {
                                kind: ErrorKind::StepsExhausted,
                                source: Some(source),
                            })?;
                            if frame.source == *construction {
                                for (index, origin, storage) in &mut frame.values {
                                    *fuel = fuel.checked_sub(1).ok_or(Error {
                                        kind: ErrorKind::StepsExhausted,
                                        source: Some(source),
                                    })?;
                                    if index == slot && origin == value_source {
                                        found = storage.take();
                                        break;
                                    }
                                }
                                break;
                            }
                        }
                        let storage = found.ok_or(Error {
                            kind: ErrorKind::InvalidCallPlan,
                            source: Some(source),
                        })?;
                        (storage, &[][..])
                    }
                };
                if let Err(error) = self.execute_cleanup_at(&action.program, &storage, fields) {
                    self.cleanup_failed = true;
                    return Err(error);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tuple_context_tests {
    use super::*;
    use nir::Sort;
    use nir::normal_forms::{Head, Kind, Parameter as Input, Role};

    #[test]
    fn callable_context_scan_uses_trusted_tuple_fields_and_bounds_recursion() {
        let id = |local| DeclarationId { package: 71, local };
        let scalar = Term::Primitive(Primitive::Bool);
        let mut package = Package {
            normal_forms: vec![Head {
                declaration: id(1),
                role: Role::Tuple,
                requirements: vec![],
                parameters: vec![Input {
                    declaration: id(2),
                    binding: LocalId { owner: 1, index: 0 },
                    kind: Kind::TypeList,
                    ty: Term::Nominal {
                        declaration: id(3),
                        arguments: vec![Term::Primitive(Primitive::Type)],
                    },
                }],
            }],
            ..Default::default()
        };
        let tuple = |fields| Term::NormalForm {
            head: id(1),
            sort: Sort::Type,
            arguments: vec![Term::TypeListValue(fields)],
        };
        let code = Term::Function {
            parameters: vec![],
            result: Box::new(scalar.clone()),
        };
        let nested = tuple(vec![scalar.clone(), tuple(vec![code])]);
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert!(vm.cleanup_callable_value(&nested, 0).unwrap());
        assert!(!vm.cleanup_callable_value(&tuple(vec![scalar]), 0).unwrap());
        assert!(!vm.cleanup_callable_value(&tuple(vec![]), 0).unwrap());
        let depth = vm.budget.depth;
        assert_eq!(
            vm.cleanup_callable_value(&nested, depth).unwrap_err().kind,
            ErrorKind::DepthExhausted
        );
        assert!(vm.cleanup_callable_value(&nested, 0).unwrap());
        vm.budget.steps = 0;
        assert!(vm.cleanup_callable_value(&nested, 0).is_err());
        drop(vm);
        package.normal_forms[0].role = Role::Opaque;
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert!(!vm.cleanup_callable_value(&nested, 0).unwrap());
    }
}

#[cfg(test)]
mod representation_copy_tests {
    use super::*;
    #[test]
    fn copy_preserves_value_instance_arguments_without_relaxing_type_lists() {
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let source = SourceRange { start: 1, end: 2 };
        let declaration = DeclarationId {
            package: 0,
            local: 1,
        };
        let arguments = vec![Term::Bool(true), Term::Primitive(Primitive::Bool)];
        for value in [
            Value::RuntimeFunction {
                declaration,
                arguments: arguments.clone(),
            },
            Value::Record {
                declaration,
                arguments: arguments.clone(),
                fields: vec![Value::Bool(false)],
            },
            Value::Variant {
                declaration,
                arguments: arguments.clone(),
                index: 0,
                fields: vec![],
            },
        ] {
            assert_eq!(vm.copy_representation(&value, source, 0).unwrap(), value);
        }
        let mut value = closure(vec![Value::Bool(true)]);
        let Value::Closure(original) = &mut value else {
            unreachable!()
        };
        Arc::make_mut(original).instance_arguments = Some(arguments.clone());
        let Value::Closure(copied) = vm.copy_representation(&value, source, 0).unwrap() else {
            panic!("closure")
        };
        assert_eq!(copied.instance_arguments, Some(arguments));
        assert!(
            vm.copy_representation(&Value::TypeList(vec![Term::Bool(true)]), source, 0)
                .is_err()
        );
        assert!(
            vm.copy_representation(&Value::Type(Term::Bool(true)), source, 0)
                .is_err()
        );
    }

    fn closure(captures: Vec<Value>) -> Value {
        let ids = (0..captures.len())
            .map(|index| LocalId {
                owner: 81,
                index: index as u32,
            })
            .collect::<Vec<_>>();
        Value::Closure(Arc::new(Closure {
            definition_source: Some(SourceRange { start: 0, end: 1 }),
            origin: None,
            instance_arguments: None,
            parameters: vec![],
            environment: ids.iter().copied().zip(captures).collect(),
            capture_order: ids,
            body: Arc::new(Expr {
                source: SourceRange { start: 0, end: 1 },
                ty: Term::Primitive(Primitive::Void),
                kind: K::Unit,
            }),
            ty: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::Void)),
            },
        }))
    }
    #[test]
    fn authorized_pointer_read_detaches_owned_capture_storage() {
        let source = SourceRange { start: 1, end: 2 };
        let cell = Storage::new(Value::Bool(true));
        let mut original = closure(vec![Value::Storage(cell.clone())]);
        let Value::Closure(original_closure) = &mut original else {
            unreachable!()
        };
        let ty = Term::Closure {
            environment: vec![Term::Primitive(Primitive::Bool)],
            signature: Box::new(original_closure.ty.clone()),
        };
        Arc::make_mut(original_closure).ty = ty.clone();
        let storage = Storage::new(original);
        let binding = LocalId {
            owner: 99,
            index: 0,
        };
        let mut environment = Environment::new();
        environment.insert(binding, Value::Pointer(storage.pointer(ty.clone(), vec![])));
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default())
            .unwrap()
            .with_pointer_copy_permission(&|_| true);
        let expr = Expr {
            ty: ty.clone(),
            source,
            kind: K::ReadPointer(Box::new(Expr {
                ty: Term::Pointer(Box::new(ty)),
                source,
                kind: K::Local(binding),
            })),
        };
        let Flow::Value(Value::Closure(copied)) = vm.eval(&expr, &mut environment).unwrap() else {
            panic!("closure")
        };
        let Value::Storage(copied_cell) = copied.environment.values().next().unwrap() else {
            panic!("capture storage")
        };
        copied_cell
            .pointer(Term::Primitive(Primitive::Bool), vec![])
            .write(Value::Bool(false), |v| matches!(v, Value::Bool(_)))
            .unwrap();
        assert_eq!(copied_cell.read().unwrap(), Value::Bool(false));
        assert_eq!(cell.read().unwrap(), Value::Bool(true));
    }

    #[test]
    fn representation_copy_detaches_nested_capture_cells_and_preserves_pointer_targets() {
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let source = SourceRange { start: 0, end: 1 };
        let external = Storage::new(Value::Bool(false));
        let pointer = external.pointer(Term::Primitive(Primitive::Bool), vec![]);
        let original_slot = Storage::new(Value::Bool(false));
        let original = closure(vec![Value::Storage(Storage::new(Value::Tuple(vec![
            closure(vec![Value::Storage(original_slot.clone())]),
            Value::Pointer(pointer.clone()),
        ])))]);
        let copied = vm.copy_representation(&original, source, 0).unwrap();
        let Value::Closure(outer) = copied else {
            panic!("closure")
        };
        let Value::Storage(aggregate) = outer.captured_values().next().unwrap().1 else {
            panic!("storage")
        };
        let Value::Tuple(fields) = aggregate.read().unwrap() else {
            panic!("tuple")
        };
        let Value::Closure(inner) = &fields[0] else {
            panic!("nested closure")
        };
        let Value::Storage(copied_slot) = inner.captured_values().next().unwrap().1 else {
            panic!("nested storage")
        };
        copied_slot
            .pointer(Term::Primitive(Primitive::Bool), vec![])
            .write(Value::Bool(true), |value| matches!(value, Value::Bool(_)))
            .unwrap();
        assert_eq!(copied_slot.read().unwrap(), Value::Bool(true));
        assert_eq!(original_slot.read().unwrap(), Value::Bool(false));
        let Value::Pointer(copied_pointer) = &fields[1] else {
            panic!("pointer")
        };
        copied_pointer
            .write(Value::Bool(true), |value| matches!(value, Value::Bool(_)))
            .unwrap();
        assert_eq!(pointer.read().unwrap(), Value::Bool(true));
        assert_eq!(external.read().unwrap(), Value::Bool(true));
        assert!(vm.copy_representation(&original, source, 128).is_err());
        vm.budget.steps = 0;
        assert!(vm.copy_representation(&original, source, 0).is_err());
        let mut fresh = Interpreter::new(&package, Budget::default()).unwrap();
        assert!(fresh.copy_representation(&original, source, 0).is_ok());
    }
    #[test]
    fn representation_copy_rejects_capture_storage_with_moved_children() {
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let storage = Storage::new(Value::Tuple(vec![Value::Bool(true), Value::Bool(false)]));
        storage
            .owned_pointer(Term::Primitive(Primitive::Bool), &[1])
            .unwrap()
            .transfer()
            .unwrap();
        let error = vm
            .copy_representation(
                &closure(vec![Value::Storage(storage)]),
                SourceRange { start: 0, end: 1 },
                0,
            )
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidPointer);
    }
}

#[cfg(test)]
mod copy_budget_tests {
    #[test]
    fn invocation_type_copy_uses_scope_without_granting_constant_export() {
        use super::*;
        let package = Package::default();
        let source = SourceRange { start: 0, end: 1 };
        let parameter = DeclarationId {
            package: 77,
            local: 1,
        };
        let ty = Term::Record(vec![(
            "element".into(),
            Term::Rigid {
                declaration: parameter,
                sort: nir::Sort::Type,
            },
        )]);
        for value in [Value::Type(ty.clone()), Value::TypeList(vec![ty.clone()])] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            assert!(vm.checkpoint_representation(&value, source).is_err());
            vm.authorize_abstract_type_parameters(&[parameter]).unwrap();
            assert_eq!(vm.checkpoint_representation(&value, source).unwrap(), value);
            assert!(vm.copy_representation(&value, source, 0).is_err());
            vm.budget.steps = 1;
            assert_eq!(
                vm.checkpoint_representation(&value, source)
                    .unwrap_err()
                    .kind,
                ErrorKind::StepsExhausted
            );
        }
    }

    use super::*;
    #[test]
    fn representation_copy_charges_width_and_type_metadata_before_cloning() {
        let package = Package::default();
        let location = SourceRange { start: 1, end: 2 };
        let source = Value::Tuple(vec![Value::Bool(true); 1024]);
        let mut vm = Interpreter::new(
            &package,
            Budget {
                steps: 16,
                depth: 128,
            },
        )
        .unwrap();
        assert_eq!(
            vm.copy_representation(&source, location, 0)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        // Only the outer visit occurred; no child visitor or destination buffer.
        assert_eq!(vm.budget.steps, 15);
        let Value::Tuple(values) = &source else {
            panic!("tuple")
        };
        assert_eq!(values.len(), 1024);
        let ty = Value::Type(Term::Tuple(vec![Term::Primitive(Primitive::Bool); 64]));
        let mut vm = Interpreter::new(
            &package,
            Budget {
                steps: 16,
                depth: 128,
            },
        )
        .unwrap();
        assert_eq!(
            vm.copy_representation(&ty, location, 0).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        let mut fresh = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            fresh.copy_representation(&source, location, 0).unwrap(),
            source
        );
        assert_eq!(fresh.copy_representation(&ty, location, 0).unwrap(), ty);
    }
    #[test]
    fn representation_copy_rejects_owned_storage_cycles_and_releases_locks() {
        let package = Package::default();
        let location = SourceRange { start: 1, end: 2 };
        let storage = Storage::new(Value::Unit);
        *storage.0.lock().unwrap() = Value::Tuple(vec![Value::Storage(storage.clone())]);
        let source = Value::Storage(storage.clone());
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.copy_representation(&source, location, 0)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidPointer
        );
        // Break the deliberately malformed host-owned cycle and retry in the
        // same interpreter; no lock or partial copied payload may escape.
        *storage.0.lock().unwrap() = Value::Bool(true);
        let Value::Storage(copied) = vm.copy_representation(&source, location, 0).unwrap() else {
            panic!("storage")
        };
        assert_eq!(copied.read().unwrap(), Value::Bool(true));
        assert_ne!(copied, storage);
    }
    #[test]
    fn representation_copy_respects_configured_depth_without_consuming_source() {
        let package = Package::default();
        let mut vm = Interpreter::new(
            &package,
            Budget {
                steps: 1000,
                depth: 2,
            },
        )
        .unwrap();
        let source = Value::Tuple(vec![Value::Tuple(vec![Value::Bool(true)])]);
        let error = vm
            .copy_representation(&source, SourceRange { start: 1, end: 2 }, 0)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::DepthExhausted);
        assert_eq!(
            source,
            Value::Tuple(vec![Value::Tuple(vec![Value::Bool(true)])])
        );
    }
}

#[cfg(test)]
mod variant_selection_tests {
    use super::*;
    use iir::cleanup::VariantCase;
    fn subject() -> Storage {
        Storage::new(Value::Variant {
            declaration: DeclarationId {
                package: 1,
                local: 1,
            },
            arguments: vec![],
            index: 0,
            fields: vec![Value::Bool(true)],
        })
    }
    fn case(guards: Vec<(Vec<usize>, usize)>) -> VariantCase {
        VariantCase {
            guards,
            steps: vec![Step::Release {
                path: vec![],
                ty: Term::Primitive(Primitive::Void),
            }],
        }
    }
    fn constructed(storage: &Storage) -> bool {
        storage
            .owned_pointer(Term::Primitive(Primitive::Void), &[])
            .unwrap()
            .is_constructed()
            .unwrap()
    }
    #[test]
    fn selection_rejects_missing_or_ambiguous_cases_before_effects() {
        let package = Package::default();
        for cases in [
            vec![],
            vec![case(vec![(vec![], 1)])],
            vec![case(vec![]), case(vec![(vec![], 0)])],
        ] {
            let storage = subject();
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            let failure = vm
                .cleanup_steps(&[Step::Select { cases }], &storage, &[], 0)
                .unwrap_err();
            assert_eq!(failure.kind, ErrorKind::InvalidCallPlan);
            assert!(constructed(&storage));
        }
    }
    #[test]
    fn enclosing_tag_failure_skips_inactive_payload_paths() {
        let package = Package::default();
        let storage = subject();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let step = Step::Select {
            cases: vec![
                case(vec![(vec![], 1), (vec![99], 0)]),
                case(vec![(vec![], 0)]),
            ],
        };
        assert!(step.to_string().contains("variant-case"));
        vm.cleanup_steps(&[step], &storage, &[], 0).unwrap();
        assert!(!constructed(&storage));
    }
    #[test]
    fn selection_budget_failure_preserves_storage_for_fresh_invocation() {
        let package = Package::default();
        for budget in [
            Budget {
                steps: 1,
                ..Budget::default()
            },
            Budget {
                depth: 1,
                ..Budget::default()
            },
        ] {
            let storage = subject();
            let steps = vec![Step::Select {
                cases: vec![case(vec![(vec![], 0)])],
            }];
            let mut vm = Interpreter::new(&package, budget).unwrap();
            let error = vm.cleanup_steps(&steps, &storage, &[], 0).unwrap_err();
            assert!(matches!(
                error.kind,
                ErrorKind::StepsExhausted | ErrorKind::DepthExhausted
            ));
            assert!(constructed(&storage));
            let mut recovered = Interpreter::new(&package, Budget::default()).unwrap();
            recovered.cleanup_steps(&steps, &storage, &[], 0).unwrap();
            assert!(!constructed(&storage));
        }
    }
}
