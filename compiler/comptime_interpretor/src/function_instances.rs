//! Checked generic function formation shared by reference and bytecode execution.
use super::*;

// Separate declaration lookup allows borrowed slice queries without constructing
// an owned tuple key. Stored frames are immutable and shared with their closure.
#[derive(Default)]
pub(super) struct Instances {
    entries: HashMap<DeclarationId, HashMap<Arc<[Term]>, Arc<Closure>>>,
}
impl Instances {
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.values().map(HashMap::len).sum()
    }
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn lookup(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
    ) -> Option<&Arc<Closure>> {
        self.entries.get(&declaration)?.get(arguments)
    }
    pub(super) fn insert(&mut self, key: (DeclarationId, Arc<[Term]>), code: Arc<Closure>) {
        self.entries.entry(key.0).or_default().insert(key.1, code);
    }
}

#[derive(Default)]
pub(super) struct Cache {
    entries: HashMap<DeclarationId, HashMap<Arc<[Term]>, Retained>>,
    count: usize,
    weight: u64,
}
struct Retained {
    code: Arc<Closure>,
    weight: u64,
}
impl Cache {
    // Callers preflight the query's term depth and compute its structural work.
    // Charge hashing/comparison before looking up the borrowed input slice.
    pub(super) fn lookup(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
        key_work: u64,
        fuel: &mut u64,
    ) -> Result<Option<Arc<Closure>>, ()> {
        *fuel = fuel.checked_sub(1).ok_or(())?;
        let Some(entries) = self.entries.get(&declaration) else {
            return Ok(None);
        };
        *fuel = fuel.checked_sub(key_work).ok_or(())?;
        Ok(entries.get(arguments).map(|entry| entry.code.clone()))
    }

    pub(super) fn insert(
        &mut self,
        key: (DeclarationId, Arc<[Term]>),
        code: Arc<Closure>,
        key_work: u64,
        work: u64,
    ) {
        let weight = work.saturating_add(key_work).saturating_add(1);
        if weight > 100_000 {
            return;
        }
        if let Some(previous) = self
            .entries
            .get_mut(&key.0)
            .and_then(|entries| entries.remove(key.1.as_ref()))
        {
            self.count -= 1;
            self.weight -= previous.weight;
        }
        if self.count >= 64 || self.weight.saturating_add(weight) > 100_000 {
            self.entries.clear();
            self.count = 0;
            self.weight = 0;
        }
        self.weight += weight;
        self.count += 1;
        self.entries
            .entry(key.0)
            .or_default()
            .insert(key.1, Retained { code, weight });
    }
}

impl Interpreter<'_> {
    pub(super) fn function_instance_value(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
        output: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if self.graph.is_some() {
            return self.iir_function_value(declaration, arguments, output, source);
        }
        if let Some(identity) = self.runtime_function_identities.get(&declaration) {
            self.admit_function_inputs(&identity.type_parameters, arguments, source)?;
            let binders = self.input_binders(&identity.type_parameters);
            self.charge_copy_work(Some(source), |fuel| {
                nir::copy_budget::charge_type_substitution(
                    &identity.signature,
                    &binders,
                    arguments,
                    fuel,
                )
            })?;
            let substitution =
                self.function_substitution(&identity.type_parameters, arguments, source)?;
            let signature = substitution
                .instantiate(&identity.signature)
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            if !selected_types::same(&signature, output, &self.associated_values) {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            }
            return Ok(Value::FunctionInstance {
                declaration,
                arguments: arguments.to_vec(),
            });
        }
        let definition = self
            .functions
            .get(&declaration)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
        self.admit_function_inputs(&definition.type_parameters, arguments, source)?;
        let substitution =
            self.function_substitution(&definition.type_parameters, arguments, source)?;
        let before_key = self.budget.steps;
        self.charge_copy_work(Some(source), |fuel| {
            for argument in arguments {
                nir::copy_budget::charge_type_substitution(argument, &[], &[], fuel)?;
            }
            Ok(())
        })?;
        let key_work = before_key - self.budget.steps;
        let existing = self.instances.lookup(declaration, arguments).cloned();
        let persistent = self.abstract_type_parameters.is_empty()
            && self.abstract_value_parameters.is_empty()
            && arguments
                .iter()
                .all(|argument| argument.check_scope(&[]).is_ok());
        let shared = if persistent && existing.is_none() {
            self.prepared_instance_cache
                .lookup(declaration, arguments, key_work, &mut self.budget.steps)
                .map_err(|()| self.failure(ErrorKind::StepsExhausted, Some(source)))?
        } else {
            None
        };
        let before_preparation = self.budget.steps;
        let binders = self.input_binders(&definition.type_parameters);
        if existing.is_none() && shared.is_none() {
            self.charge_copy_work(Some(source), |fuel| {
                nir::copy_budget::charge_function_substitution(
                    definition, &binders, arguments, fuel,
                )
            })?;
        }
        if self.comptime_only {
            let definition = self
                .functions
                .get(&declaration)
                .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
            for requirement in &definition.requirements {
                // Cached bodies do not suppress current invocation obligations.
                let requirement =
                    self.specialize_function_predicate(definition, arguments, requirement, source)?;
                if let Some(requirement) =
                    self.bind_invocation_contract(&requirement, None, source, 0)?
                {
                    self.check_type_contract(&requirement, source)?;
                }
            }
        }
        for capability in &definition.capabilities {
            let nir::program::TypePredicate::Trait(bound) = self.specialize_function_predicate(
                definition,
                arguments,
                &nir::program::TypePredicate::Trait(capability.clone()),
                source,
            )?
            else {
                unreachable!("trait substitution preserves predicate kind");
            };
            let open = bound.subject.check_scope(&[]).is_err()
                || bound
                    .interface_arguments
                    .iter()
                    .any(|ty| ty.check_scope(&[]).is_err());
            if self.comptime_only && open {
                continue;
            }
            if !self.capabilities_hold_parameters(std::slice::from_ref(&bound), &[], &[]) {
                return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
            }
        }
        let instance = if let Some(instance) = existing.or(shared) {
            instance
        } else {
            let definition = self
                .functions
                .get(&declaration)
                .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
            let signature = substitution
                .instantiate(&definition.signature)
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            let mut body = definition.body.clone();
            // Body types live under the function's parameter telescope;
            // a declaration substitution must retain that frame.
            let Term::Function { parameters, .. } = signature.callable_signature() else {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            };
            let frames = vec![parameters.iter().map(|p| p.sort).collect()];
            let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let rewritten = self
                .function_substitution_bounded(
                    &definition.type_parameters,
                    arguments,
                    &frames,
                    &mut fuel,
                )
                .and_then(|substitution| {
                    body.rewrite_types(&mut |ty| substitution.instantiate(ty))
                        .map_err(nir::type_computations::Error::Scope)
                });
            self.budget.steps -= (before - fuel) as u64;
            rewritten.map_err(|failure| {
                self.failure(
                    if matches!(failure, nir::type_computations::Error::Budget) {
                        ErrorKind::StepsExhausted
                    } else {
                        ErrorKind::Type
                    },
                    Some(source),
                )
            })?;
            let key = (declaration, Arc::<[Term]>::from(arguments));
            let instance = Arc::new(Closure {
                definition_source: None,
                origin: Some(declaration),
                instance_arguments: Some(key.1.clone()),
                parameters: definition.parameters.clone(),
                environment: Environment::new(),
                capture_order: vec![],
                body: Arc::new(body),
                ty: signature
                    .with_comptime_phase(definition.phase == nir::program::FunctionPhase::Comptime)
                    .with_unsafe_call_requirement(definition.is_unsafe),
            });
            self.events.push(Event::InstantiateFunction {
                declaration,
                arguments: arguments.to_vec(),
            });
            if persistent && self.budget.steps >= key_work {
                self.budget.steps -= key_work;
                self.prepared_instance_cache.insert(
                    key.clone(),
                    instance.clone(),
                    key_work,
                    before_preparation - self.budget.steps,
                );
            }
            self.instances.insert(key, instance.clone());
            instance
        };
        if !selected_types::same(&instance.ty, output, &self.associated_values) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(Value::Closure(instance))
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[test]
    fn borrowed_lookup_preserves_shared_frames_and_distinguishes_inputs() {
        let declaration = DeclarationId {
            package: 1,
            local: 1,
        };
        let frame: Arc<[Term]> = vec![Term::Bool(true)].into();
        let code = Arc::new(Closure {
            definition_source: None,
            origin: Some(declaration),
            instance_arguments: Some(frame.clone()),
            parameters: vec![],
            environment: Environment::new(),
            capture_order: vec![],
            body: Arc::new(Expr {
                source: SourceRange { start: 0, end: 1 },
                ty: Term::Primitive(Primitive::Void),
                kind: K::Unit,
            }),
            ty: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::Void)),
            },
        });
        let mut instances = Instances::default();
        instances.insert((declaration, frame.clone()), code.clone());
        let query = vec![Term::Bool(true)];
        let found = instances.lookup(declaration, &query).unwrap();
        assert!(Arc::ptr_eq(found, &code));
        let (retained, _) = instances.entries[&declaration]
            .get_key_value(query.as_slice())
            .unwrap();
        assert!(Arc::ptr_eq(
            retained,
            found.instance_arguments.as_ref().unwrap()
        ));
        assert!(
            instances
                .lookup(declaration, &[Term::Bool(false)])
                .is_none()
        );
        assert!(
            instances
                .lookup(
                    DeclarationId {
                        local: 2,
                        ..declaration
                    },
                    &query
                )
                .is_none()
        );
        let mut cache = Cache::default();
        cache.insert((declaration, frame.clone()), code.clone(), 1, 1);
        assert!(Arc::ptr_eq(
            cache.entries[&declaration].keys().next().unwrap(),
            &frame
        ));
        assert!(cache.lookup(declaration, &query, 1, &mut 0).is_err());
        let restored = cache
            .lookup(declaration, &query, 1, &mut 100)
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&restored, &code));
        assert!(Arc::ptr_eq(
            restored.instance_arguments.as_ref().unwrap(),
            &frame
        ));
        for local in 2..=64 {
            cache.insert(
                (
                    DeclarationId {
                        local,
                        ..declaration
                    },
                    frame.clone(),
                ),
                code.clone(),
                1,
                1,
            );
        }
        assert_eq!(cache.count, 64);
        // Unrelated declarations no longer consume a scan of their input frames.
        assert!(
            cache
                .lookup(declaration, &query, 1, &mut 2)
                .unwrap()
                .is_some()
        );
        cache.insert((declaration, frame.clone()), code.clone(), 1, 5);
        assert_eq!(cache.count, 64);
        assert_eq!(cache.weight, 63 * 3 + 7);
        cache.insert((declaration, frame.clone()), code.clone(), 1, 100_000);
        assert_eq!(cache.count, 64);
        assert_eq!(cache.weight, 63 * 3 + 7);
        cache.insert(
            (
                DeclarationId {
                    local: 65,
                    ..declaration
                },
                frame,
            ),
            code.clone(),
            1,
            1,
        );
        assert_eq!(cache.count, 1);
        assert_eq!(cache.weight, 3);
        assert!(
            cache
                .lookup(declaration, &query, 1, &mut 2)
                .unwrap()
                .is_none()
        );

        let mut specializations = Cache::default();
        for value in 0..64 {
            let inputs: Arc<[Term]> = (0..6)
                .map(|bit| Term::Bool(value & (1 << bit) != 0))
                .collect();
            specializations.insert((declaration, inputs), code.clone(), 6, 1);
        }
        let last = vec![Term::Bool(true); 6];
        assert!(
            specializations
                .lookup(declaration, &last, 6, &mut 6)
                .is_err()
        );
        let mut remaining = 7;
        assert!(
            specializations
                .lookup(declaration, &last, 6, &mut remaining)
                .unwrap()
                .is_some()
        );
        assert_eq!(remaining, 0);
        assert_eq!(specializations.count, 64);
        assert!(
            specializations
                .lookup(declaration, &[Term::Bool(true)], 1, &mut 2)
                .unwrap()
                .is_none()
        );
    }
}
