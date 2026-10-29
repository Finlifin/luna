//! Invocation admission, owned frame state, delivery and caller restoration.
//! These boundaries are shared by the migration interpreter and linear bodies.
use super::*;

enum Body<'a> {
    Package(&'a Expr),
    Owned(Arc<Expr>),
}
impl Body<'_> {
    fn expression(&self) -> &Expr {
        match self {
            Self::Package(body) => body,
            Self::Owned(body) => body,
        }
    }
}

struct InvocationFrame<'a> {
    function: Value,
    environment: Environment,
    body: Body<'a>,
    result_type: Term,
    type_scope: Option<nir::FunctionInputBinding>,
    origin: Option<DeclarationId>,
    stable_code: Option<DeclarationId>,
}

// The monotonic effect identity allocator and pending transfers are intentionally
// not restored. They belong to the engine, not the suspended caller activation.
struct SavedInvocation {
    effect_invocation: u64,
    effect_resume: Option<u64>,
    effect_environment: Vec<effects::Entry>,
    cleanup_frame: Option<iir::InstanceKey>,
    prepared_frames: Vec<cleanup::PreparedFrame>,
    residuals: Vec<(SourceRange, memory::Storage)>,
    pattern_orders: BTreeMap<(u32, u32), Vec<LocalId>>,
    cleanup_context: Vec<iir::cleanup::ContextKey>,
}

struct Activation<'a> {
    frame: InvocationFrame<'a>,
    saved: SavedInvocation,
}

impl<'a> Interpreter<'a> {
    pub(super) fn invoke_value_scoped(
        &mut self,
        function: Value,
        arguments: Vec<Value>,
        context: Option<(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
    ) -> Result<Value, Error> {
        let mut activation = self.begin_invocation(function, arguments, context)?;
        let frame = &mut activation.frame;
        let result = self.invocation_body(
            frame.body.expression(),
            frame.stable_code,
            &mut frame.environment,
        );
        self.complete_invocation(activation, result)
    }

    fn begin_invocation(
        &mut self,
        function: Value,
        arguments: Vec<Value>,
        context: Option<(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
    ) -> Result<Box<Activation<'a>>, Error> {
        let origin = self.admit_invocation(&function, context.as_ref())?;
        let saved = self.enter_invocation(&function, origin, context)?;
        match self.prepare_invocation(function, arguments, origin) {
            Ok(frame) => Ok(Box::new(Activation { frame, saved })),
            Err(error) => {
                self.leave_invocation(saved);
                Err(error)
            }
        }
    }

    fn complete_invocation(
        &mut self,
        mut activation: Box<Activation<'_>>,
        result: Result<Flow, Error>,
    ) -> Result<Value, Error> {
        let result = result.and_then(|flow| self.finish_invocation(&mut activation.frame, flow));
        self.leave_invocation(activation.saved);
        result
    }

    fn admit_invocation(
        &mut self,
        function: &Value,
        context: Option<&(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
    ) -> Result<Option<DeclarationId>, Error> {
        if matches!(
            function,
            Value::RuntimeFunction { .. } | Value::RuntimeTraitMethod(_)
        ) {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        let origin = match &function {
            Value::Function(id) => Some(*id),
            Value::Closure(closure) => closure.origin,
            _ => None,
        };
        if origin.is_some_and(|id| self.normal_forms.contains_key(&id)) {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        if self.comptime_only {
            let declared_comptime = |id| {
                self.invocation_definition(id, &function)
                    .is_some_and(|f| f.phase == nir::program::FunctionPhase::Comptime)
            };
            let authorized = match &function {
                Value::Function(id) => declared_comptime(*id),
                Value::Closure(closure) => {
                    closure.ty.is_comptime_callable()
                        && closure.origin.is_none_or(declared_comptime)
                }
                _ => false,
            };
            if !authorized {
                return Err(self.failure(ErrorKind::PhaseMismatch, None));
            }
        }
        if self.cleanup_schedule.is_some() && context.is_none() {
            let named_instance = match &function {
                Value::Function(declaration) => Some((*declaration, &[][..])),
                Value::Closure(closure) => closure.origin.map(|declaration| {
                    (
                        declaration,
                        closure.instance_arguments.as_deref().unwrap_or(&[]),
                    )
                }),
                _ => None,
            };
            if let Some((declaration, arguments)) = named_instance {
                for key in &self.cleanup_schedule.unwrap().borrowed_callable_functions {
                    self.tick(None)?;
                    if key.declaration == declaration && key.arguments == arguments {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                    }
                }
            }
            if let Some(declaration) = origin {
                let definition = self
                    .invocation_definition(declaration, &function)
                    .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
                if matches!(definition.signature.callable_signature(), Term::Function {parameters,..} if parameters.iter().any(|p|matches!(p.ty.callable_signature(),Term::Function {..})))
                {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                }
            }
        }
        Ok(origin)
    }

    fn enter_invocation(
        &mut self,
        function: &Value,
        origin: Option<DeclarationId>,
        context: Option<(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
    ) -> Result<SavedInvocation, Error> {
        self.tick(None)?;
        self.effect_charge(self.effect_state.environment.len(), None)?;
        let next_effect_identity = self
            .effect_state
            .next_identity
            .checked_add(1)
            .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, None))?;
        let previous_effect_invocation = self.effect_state.invocation;
        let previous_effect_resume = self.effect_state.resume.take();
        let previous_effect_environment = self.effect_state.environment.clone();
        self.effect_state.next_identity = next_effect_identity;
        self.effect_state.invocation = next_effect_identity;
        if self.depth == 0 {
            self.cleanup_failed = false;
            self.effect_state.transfer = None;
        }
        self.depth += 1;
        let previous_cleanup_context = std::mem::take(&mut self.cleanup_context);
        let previous_residuals = std::mem::take(&mut self.residual_temporaries);
        let previous_pattern_orders = std::mem::take(&mut self.pattern_binding_orders);
        let previous_prepared_frames = std::mem::take(&mut self.prepared_frames);
        let previous_cleanup_frame = self.cleanup_frame.take();
        self.cleanup_frame = if self.cleanup_schedule.is_some() {
            origin.map(|declaration| iir::InstanceKey {
                declaration,
                arguments: match &function {
                    Value::Closure(closure) => {
                        closure.instance_arguments.clone().unwrap_or_default()
                    }
                    _ => vec![],
                },
            })
        } else {
            None
        };
        if let Some((instance, context)) = context {
            self.cleanup_frame = Some(instance);
            self.cleanup_context = context;
        }
        Ok(SavedInvocation {
            effect_invocation: previous_effect_invocation,
            effect_resume: previous_effect_resume,
            effect_environment: previous_effect_environment,
            cleanup_frame: previous_cleanup_frame,
            prepared_frames: previous_prepared_frames,
            residuals: previous_residuals,
            pattern_orders: previous_pattern_orders,
            cleanup_context: previous_cleanup_context,
        })
    }

    fn leave_invocation(&mut self, saved: SavedInvocation) {
        self.effect_state.invocation = saved.effect_invocation;
        self.effect_state.resume = saved.effect_resume;
        self.effect_state.environment = saved.effect_environment;
        self.cleanup_frame = saved.cleanup_frame;
        self.prepared_frames = saved.prepared_frames;
        self.residual_temporaries = saved.residuals;
        self.pattern_binding_orders = saved.pattern_orders;
        self.cleanup_context = saved.cleanup_context;
        self.depth -= 1;
    }

    fn prepare_invocation(
        &mut self,
        function: Value,
        arguments: Vec<Value>,
        origin: Option<DeclarationId>,
    ) -> Result<InvocationFrame<'a>, Error> {
        let (parameters, mut environment, body, signature) = match &function {
            Value::Function(id) => {
                let definition = *self
                    .functions
                    .get(id)
                    .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
                if !definition.type_parameters.is_empty()
                    || (!self.comptime_only
                        && (!definition.requirements.is_empty()
                            || !definition.capabilities.is_empty()))
                {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                (
                    definition.parameters.as_slice(),
                    Environment::with_capacity(definition.parameters.len()),
                    &definition.body,
                    &definition.signature,
                )
            }
            Value::Closure(closure) => {
                let signature = match &closure.ty {
                    Term::Closure { signature, .. } => signature.as_ref(),
                    ty => ty,
                };
                (
                    closure.parameters.as_slice(),
                    self.clone_invocation_environment(&closure.environment, closure.body.source)?,
                    closure.body.as_ref(),
                    signature,
                )
            }
            _ => return Err(self.failure(ErrorKind::Type, None)),
        };
        let Term::Function {
            parameters: types,
            result: result_type,
        } = signature.callable_signature()
        else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        let type_scope = self.bind_invocation_inputs(
            parameters,
            arguments,
            signature,
            &mut environment,
            body.source,
        )?;
        if self.comptime_only {
            if let Some(id) = origin {
                let definition = self
                    .invocation_definition(id, &function)
                    .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
                let generic_arguments = match &function {
                    _ if self.graph.is_some() => &[][..],
                    Value::Closure(closure) => closure.instance_arguments.as_deref().unwrap_or(&[]),
                    _ => &[],
                };
                for requirement in &definition.requirements {
                    let binders = self.input_binders(&definition.type_parameters);
                    self.charge_copy_work(Some(body.source), |fuel| {
                        nir::copy_budget::charge_predicate_substitution(
                            requirement,
                            &binders,
                            generic_arguments,
                            fuel,
                        )
                    })?;
                    let requirement = requirement
                        .instantiate_parameters(
                            &self.input_binders(&definition.type_parameters),
                            generic_arguments,
                        )
                        .map_err(|_| self.failure(ErrorKind::Type, Some(body.source)))?;
                    let requirement = self
                        .bind_invocation_contract(
                            &requirement,
                            type_scope.as_ref(),
                            body.source,
                            0,
                        )?
                        .ok_or_else(|| {
                            self.failure(ErrorKind::TypeContractUnresolved, Some(body.source))
                        })?;
                    self.check_type_contract(&requirement, body.source)?;
                }
                self.check_deferred_capabilities(
                    definition,
                    generic_arguments,
                    type_scope.as_ref(),
                    body.source,
                )?;
            }
        }
        let result_type = match &type_scope {
            Some(scope) => scope.rebind(result_type),
            None => result_type.substitute_partial(&vec![None; types.len()]),
        }
        .map_err(|_| self.failure(ErrorKind::Type, None))?;
        result_type
            .check_scope(&[])
            .map_err(|_| self.failure(ErrorKind::Type, None))?;
        if let Term::Computation { effects, .. } = &result_type {
            self.check_effect_environment(effects, None)?;
        } else {
            self.effect_state.environment.clear();
        }
        let specialized_body = if let Some(scope) = &type_scope {
            let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let before = fuel;
            let checked = iir::visit::expression(body, &mut fuel, |_, _| Ok(()));
            self.budget.steps -= (before - fuel) as u64;
            checked.map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(body.source)))?;
            let mut body = body.clone();
            self.specialize_input_types(&mut body, scope)?;
            Some(body)
        } else {
            None
        };
        let origin = match &function {
            Value::Function(id) => Some(*id),
            Value::Closure(closure) => closure.origin,
            _ => None,
        };
        if let Some(id) = origin {
            let definition = self
                .invocation_definition(id, &function)
                .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
            for snapshot in &definition.entry_snapshots {
                let value = self.local_value(&environment, snapshot.parameter, None)?;
                environment.insert(snapshot.binding, value);
            }
            for condition in &definition.preconditions {
                let mut condition =
                    self.copy_invocation_condition(condition, definition, &function)?;
                if let Some(scope) = &type_scope {
                    self.specialize_input_types(&mut condition, scope)?;
                }
                let check = Expr {
                    source: condition.source,
                    ty: Term::Primitive(Primitive::Void),
                    kind: K::CheckPredicate(Box::new(condition)),
                };
                match self.eval(&check, &mut environment)? {
                    Flow::Value(Value::Unit) => {}
                    _ => {
                        return Err(self.failure(ErrorKind::InvalidControlFlow, Some(check.source)));
                    }
                }
            }
            self.events.push(Event::EnterFunction(id));
        }
        let stable_code = match &function {
            Value::Function(id) if specialized_body.is_none() => Some(*id),
            _ => None,
        };
        let body = if let Some(body) = specialized_body {
            Body::Owned(Arc::new(body))
        } else {
            match &function {
                Value::Function(id) => Body::Package(&self.functions[id].body),
                Value::Closure(closure) => Body::Owned(closure.body.clone()),
                _ => unreachable!("callable admitted before binding"),
            }
        };
        Ok(InvocationFrame {
            function,
            environment,
            body,
            result_type,
            type_scope,
            origin,
            stable_code,
        })
    }

    fn finish_invocation(
        &mut self,
        frame: &mut InvocationFrame<'_>,
        flow: Flow,
    ) -> Result<Value, Error> {
        let InvocationFrame {
            function,
            environment,
            body,
            result_type,
            type_scope,
            origin,
            ..
        } = frame;
        let source = body.expression().source;
        let (value, returned) = match flow {
            Flow::Value(value) => (value, false),
            Flow::Return(value) => (value, true),
            _ => return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source))),
        };
        let delivered_type = if let Term::Computation { result, .. } = &*result_type {
            result.as_ref()
        } else {
            result_type
        };
        if !self.has_type(&value, delivered_type) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        if self.cleanup_schedule.is_some() {
            if !returned {
                self.check_scheduled_result(&value, source, environment)?;
            }
        } else if let Some(id) = *origin {
            let definition = self
                .invocation_definition(id, function)
                .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
            if !definition.postconditions.is_empty()
                && let Some(binding) = definition.result_binding
            {
                environment.insert(binding, value.clone());
            }
            for condition in &definition.postconditions {
                let mut condition =
                    self.copy_invocation_condition(condition, definition, function)?;
                if let Some(scope) = &type_scope {
                    self.specialize_input_types(&mut condition, scope)?;
                }
                let check = Expr {
                    source: condition.source,
                    ty: Term::Primitive(Primitive::Void),
                    kind: K::CheckPredicate(Box::new(condition)),
                };
                match self.eval(&check, environment)? {
                    Flow::Value(Value::Unit) => {}
                    _ => {
                        return Err(self.failure(ErrorKind::InvalidControlFlow, Some(check.source)));
                    }
                }
            }
        }
        if !returned {
            self.cleanup_site(source, iir::cleanup::Boundary::FunctionEnd, environment)?;
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_activations_restore_callers_without_reusing_effect_identities() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let id = DeclarationId {
            package: 1,
            local: 0,
        };
        let key = iir::InstanceKey {
            declaration: id,
            arguments: vec![],
        };
        let local = LocalId { owner: 1, index: 0 };
        vm.depth = 1;
        vm.effect_state.invocation = 7;
        vm.effect_state.next_identity = 10;
        vm.effect_state.resume = Some(3);
        vm.cleanup_frame = Some(key.clone());
        vm.pattern_binding_orders.insert((1, 2), vec![local]);
        vm.prepared_frames.push(cleanup::PreparedFrame {
            source: SourceRange { start: 1, end: 2 },
            values: vec![],
        });
        let callable = Value::Function(id);
        let caller = vm.enter_invocation(&callable, Some(id), None).unwrap();
        assert_eq!(vm.depth, 2);
        assert_eq!(vm.effect_state.invocation, 11);
        assert_eq!(vm.effect_state.resume, None);
        assert!(vm.pattern_binding_orders.is_empty());
        assert!(vm.prepared_frames.is_empty());
        assert!(vm.cleanup_frame.is_none());
        vm.effect_state.resume = Some(4);
        vm.pattern_binding_orders.insert((3, 4), vec![local]);
        let nested = vm.enter_invocation(&callable, Some(id), None).unwrap();
        assert_eq!(vm.effect_state.invocation, 12);
        vm.leave_invocation(nested);
        assert_eq!(vm.depth, 2);
        assert_eq!(vm.effect_state.invocation, 11);
        assert_eq!(vm.effect_state.resume, Some(4));
        assert_eq!(vm.pattern_binding_orders.get(&(3, 4)), Some(&vec![local]));
        vm.leave_invocation(caller);
        assert_eq!(vm.depth, 1);
        assert_eq!(vm.effect_state.invocation, 7);
        assert_eq!(vm.effect_state.resume, Some(3));
        assert_eq!(vm.effect_state.next_identity, 12);
        assert_eq!(vm.cleanup_frame, Some(key));
        assert_eq!(vm.prepared_frames.len(), 1);
        assert_eq!(vm.pattern_binding_orders.get(&(1, 2)), Some(&vec![local]));
        assert!(!vm.pattern_binding_orders.contains_key(&(3, 4)));
    }

    #[test]
    fn failed_entry_and_failed_preparation_restore_caller_state() {
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let id = DeclarationId {
            package: 1,
            local: 0,
        };
        vm.effect_state.resume = Some(9);
        vm.effect_state.invocation = 5;
        vm.effect_state.next_identity = u64::MAX;
        let error = vm
            .invoke_value_scoped(Value::Function(id), vec![], None)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.effect_state.resume, Some(9));
        assert_eq!(vm.effect_state.invocation, 5);
        vm.effect_state.next_identity = 10;
        // Admission accepts the runtime carrier, but preparation cannot resolve
        // this declaration. The entered activation must still be unwound.
        let error = vm
            .invoke_value_scoped(Value::Function(id), vec![], None)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnknownFunction);
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.effect_state.resume, Some(9));
        assert_eq!(vm.effect_state.invocation, 5);
        assert_eq!(vm.effect_state.next_identity, 11);
    }
    #[test]
    fn prepared_closure_survives_nested_failed_delivery() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let captured = LocalId { owner: 1, index: 0 };
        let callable = |body| {
            let mut environment = Environment::new();
            environment.insert(captured, Value::Bool(true));
            Value::Closure(Arc::new(Closure {
                definition_source: None,
                origin: None,
                instance_arguments: None,
                parameters: vec![],
                environment,
                capture_order: vec![captured],
                body: Arc::new(Expr {
                    source: SourceRange { start: 1, end: 2 },
                    ty: Term::Primitive(Primitive::Bool),
                    kind: body,
                }),
                ty: Term::ComptimeFunction(Box::new(Term::Function {
                    parameters: vec![],
                    result: Box::new(Term::Primitive(Primitive::Bool)),
                })),
            }))
        };
        let mut parent = vm
            .begin_invocation(callable(K::Local(captured)), vec![], None)
            .unwrap();
        let parent_identity = vm.effect_state.invocation;
        let child = vm
            .begin_invocation(callable(K::Unit), vec![], None)
            .unwrap();
        // A delivery type error must unwind the child activation while leaving
        // the suspended parent's owned closure and captured environment usable.
        assert_eq!(
            vm.complete_invocation(child, Ok(Flow::Value(Value::Unit)))
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        assert_eq!(vm.depth, 1);
        assert_eq!(vm.effect_state.invocation, parent_identity);
        let frame = &mut parent.frame;
        let flow = vm.invocation_body(
            frame.body.expression(),
            frame.stable_code,
            &mut frame.environment,
        );
        assert_eq!(
            vm.complete_invocation(parent, flow).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.effect_state.invocation, 0);
    }
}
