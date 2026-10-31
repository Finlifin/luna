//! Transport invocation results into a caller telescope without mutating their VM.
use super::*;

impl Value {
    /// Restore types in an owned value graph, including executable closure data.
    /// Shared capture cells remain shared in the new graph, but never alias the
    /// producer's cells. Code keeps its runtime identities; cycles need separate rules.
    pub fn restore_scoped_types(
        &self,
        parameters: &[nir::DeclarationParameter],
        arguments: &[Term],
        frames: &[Vec<nir::Sort>],
        steps: &mut u64,
    ) -> Result<Value, ErrorKind> {
        Restore {
            parameters,
            arguments,
            frames,
            steps,
            storage: BTreeMap::new(),
            closures: BTreeMap::new(),
            codes: BTreeMap::new(),
        }
        .value(self, 0)
    }
}

struct Restore<'a> {
    parameters: &'a [nir::DeclarationParameter],
    arguments: &'a [Term],
    frames: &'a [Vec<nir::Sort>],
    steps: &'a mut u64,
    storage: BTreeMap<usize, Option<Storage>>,
    closures: BTreeMap<usize, Option<Arc<Closure>>>,
    codes: BTreeMap<(u8, usize), CodeValue>,
}
impl Restore<'_> {
    fn spend(&mut self, count: usize) -> Result<(), ErrorKind> {
        *self.steps = self
            .steps
            .checked_sub(count as u64)
            .ok_or(ErrorKind::StepsExhausted)?;
        Ok(())
    }
    fn ty(&mut self, ty: &Term) -> Result<Term, ErrorKind> {
        let mut fuel = usize::try_from(*self.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let check = nir::copy_budget::charge_type_substitution(
            ty,
            self.parameters,
            self.arguments,
            &mut fuel,
        );
        *self.steps -= (before - fuel) as u64;
        check.map_err(copy_error)?;
        ty.instantiate_parameters_in_scope(self.parameters, self.arguments, self.frames)
            .map_err(|_| ErrorKind::Type)
    }
    fn types(&mut self, values: &[Term]) -> Result<Vec<Term>, ErrorKind> {
        self.spend(values.len())?;
        values.iter().map(|ty| self.ty(ty)).collect()
    }
    fn values(&mut self, values: &[Value], depth: usize) -> Result<Vec<Value>, ErrorKind> {
        self.spend(values.len())?;
        values
            .iter()
            .map(|value| self.value(value, depth + 1))
            .collect()
    }
    #[inline(never)]
    fn object(
        &mut self,
        object: &comptime_runtime::object::Object<ObjectValue>,
        depth: usize,
    ) -> Result<Value, ErrorKind> {
        Ok({
            use comptime_runtime::object::{Entry, Object};
            self.spend(object.entries().len())?;
            let mut entries = Vec::new();
            for entry in object.entries() {
                let (key, value) = match entry {
                    Entry::Value(value) => (None, value),
                    Entry::Property { key, value } => (Some(key), value),
                };
                let value = ObjectValue {
                    ty: self.ty(&value.ty)?,
                    value: self.value(&value.value, depth + 1)?,
                };
                entries.push(if let Some(key) = key {
                    self.spend(key.len())?;
                    Entry::Property {
                        key: key.clone(),
                        value,
                    }
                } else {
                    Entry::Value(value)
                });
            }
            Value::Object(Object::from_entries(entries))
        })
    }

    #[inline(never)]
    fn cell(&mut self, cell: &Storage, depth: usize) -> Result<Value, ErrorKind> {
        Ok({
            let key = Arc::as_ptr(&cell.0) as usize;
            if let Some(saved) = self.storage.get(&key) {
                return saved
                    .clone()
                    .map(Value::Storage)
                    .ok_or(ErrorKind::Unsupported);
            }
            self.storage.insert(key, None);
            let payload = cell.contents_for_copy(self.steps, 128 - depth)?;
            let restored = Storage::new(self.value(payload.value(), depth + 1)?);
            self.storage.insert(key, Some(restored.clone()));
            Value::Storage(restored)
        })
    }

    #[inline(never)]
    fn code(&mut self, code: &CodeValue) -> Result<Value, ErrorKind> {
        use nir::code_values::{ExpressionCode, PatternCode};
        let key = match code {
            CodeValue::Expression(code) => (0, Arc::as_ptr(code) as usize),
            CodeValue::Pattern(code) => (1, Arc::as_ptr(code) as usize),
        };
        if let Some(restored) = self.codes.get(&key) {
            return Ok(Value::Code(restored.clone()));
        }
        // Validate complete bodies and context membership before copying. This
        // is transport evidence, never permission to execute or splice the code.
        code.charge(self.steps)?;
        let restored = match code {
            CodeValue::Expression(code) => CodeValue::Expression(Arc::new(ExpressionCode {
                body: Some(self.body(code.body.as_ref().ok_or(ErrorKind::Unsupported)?)?),
                result: self.ty(&code.result)?,
                context: code
                    .context
                    .iter()
                    .map(|(id, ty)| Ok((*id, self.ty(ty)?)))
                    .collect::<Result<_, ErrorKind>>()?,
                ports: code.ports.clone(),
                defaults: code.defaults.clone(),
            })),
            CodeValue::Pattern(code) => {
                let mut body = code.body.as_ref().ok_or(ErrorKind::Unsupported)?.clone();
                body.rewrite_types(&mut |ty| self.ty(ty))?;
                CodeValue::Pattern(Arc::new(PatternCode {
                    body: Some(body),
                    subject: self.ty(&code.subject)?,
                    outputs: code
                        .outputs
                        .iter()
                        .map(|(name, (id, ty))| Ok((name.clone(), (*id, self.ty(ty)?))))
                        .collect::<Result<_, ErrorKind>>()?,
                }))
            }
        };
        self.codes.insert(key, restored.clone());
        Ok(Value::Code(restored))
    }

    #[inline(never)]
    fn body(&mut self, body: &Expr) -> Result<Arc<Expr>, ErrorKind> {
        let mut fuel = usize::try_from(*self.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let check =
            nir::copy_budget::charge_substitution(body, self.parameters, self.arguments, &mut fuel);
        *self.steps -= (before - fuel) as u64;
        check.map_err(copy_error)?;
        let body = body
            .instantiate_parameters_in_scope(self.parameters, self.arguments, self.frames)
            .map_err(|_| ErrorKind::Type)?;
        Ok(Arc::new(body))
    }

    #[inline(never)]
    fn closure(&mut self, closure: &Arc<Closure>, depth: usize) -> Result<Value, ErrorKind> {
        Ok({
            if let Some((declaration, inputs)) = closure.declaration_instance() {
                return Ok(Value::FunctionInstance {
                    declaration,
                    arguments: self.types(inputs)?,
                });
            }
            let key = Arc::as_ptr(closure) as usize;
            if let Some(saved) = self.closures.get(&key) {
                return saved
                    .clone()
                    .map(Value::Closure)
                    .ok_or(ErrorKind::Unsupported);
            }
            self.closures.insert(key, None);
            let body = self.body(&closure.body)?;
            self.spend(
                closure.parameters.len() + closure.capture_order.len() + closure.environment.len(),
            )?;
            let environment = closure
                .environment
                .iter()
                .map(|(id, value)| Ok((*id, self.value(value, depth + 1)?)))
                .collect::<Result<_, ErrorKind>>()?;
            let restored = Arc::new(Closure {
                definition_source: closure.definition_source,
                origin: closure.origin,
                instance_arguments: closure
                    .instance_arguments
                    .as_ref()
                    .map(|args| self.types(args))
                    .transpose()?,
                parameters: closure.parameters.clone(),
                capture_order: closure.capture_order.clone(),
                environment,
                body,
                ty: self.ty(&closure.ty)?,
            });
            self.closures.insert(key, Some(restored.clone()));
            Value::Closure(restored)
        })
    }

    fn value(&mut self, value: &Value, depth: usize) -> Result<Value, ErrorKind> {
        self.spend(1)?;
        if depth >= 128 {
            return Err(ErrorKind::DepthExhausted);
        }
        Ok(match value {
            Value::TypeParameter(declaration) => Value::Type(self.ty(&Term::Rigid {
                declaration: *declaration,
                sort: nir::Sort::Type,
            })?),
            Value::Type(ty) => Value::Type(self.ty(ty)?),
            Value::AbstractValue { declaration, ty } => Value::AbstractValue {
                declaration: *declaration,
                ty: self.ty(ty)?,
            },
            Value::List { ty, values } => Value::List {
                ty: self.ty(ty)?,
                values: self.values(values, depth)?.into(),
            },
            Value::Tuple(values) => Value::Tuple(self.values(values, depth)?),
            Value::AnonymousRecord(fields) => {
                self.spend(fields.len())?;
                Value::AnonymousRecord(
                    fields
                        .iter()
                        .map(|(name, value)| {
                            self.spend(name.len())?;
                            Ok((name.clone(), self.value(value, depth + 1)?))
                        })
                        .collect::<Result<_, ErrorKind>>()?,
                )
            }
            Value::Record {
                declaration,
                arguments,
                fields,
            } => Value::Record {
                declaration: *declaration,
                arguments: self.types(arguments)?,
                fields: self.values(fields, depth)?,
            },
            Value::Variant {
                declaration,
                arguments,
                index,
                fields,
            } => Value::Variant {
                declaration: *declaration,
                arguments: self.types(arguments)?,
                index: *index,
                fields: self.values(fields, depth)?,
            },
            Value::Optional { ty, value } => Value::Optional {
                ty: self.ty(ty)?,
                value: value
                    .as_ref()
                    .map(|value| self.value(value, depth + 1).map(Box::new))
                    .transpose()?,
            },
            Value::ErrorResult { ty, error, value } => Value::ErrorResult {
                ty: self.ty(ty)?,
                error: *error,
                value: Box::new(self.value(value, depth + 1)?),
            },
            Value::Newtype { ty, value } => Value::Newtype {
                ty: self.ty(ty)?,
                value: Box::new(self.value(value, depth + 1)?),
            },
            Value::FunctionInstance {
                declaration,
                arguments,
            } => Value::FunctionInstance {
                declaration: *declaration,
                arguments: self.types(arguments)?,
            },
            Value::Object(object) => self.object(object, depth)?,

            Value::Storage(cell) => self.cell(cell, depth)?,

            Value::Closure(closure) => self.closure(closure, depth)?,

            Value::Code(code) => self.code(code)?,
            Value::Pointer(_) | Value::ComptimeReference(_) | Value::RuntimeTraitMethod(_) => {
                return Err(ErrorKind::Unsupported);
            }
            value => {
                self.spend(
                    value
                        .immutable_copy_work()
                        .ok_or(ErrorKind::RepresentationSizeLimit)?,
                )?;
                value.clone()
            }
        })
    }
}
fn copy_error(error: nir::copy_budget::CopyBudgetExhausted) -> ErrorKind {
    match error {
        nir::copy_budget::CopyBudgetExhausted::Steps => ErrorKind::StepsExhausted,
        nir::copy_budget::CopyBudgetExhausted::Depth => ErrorKind::DepthExhausted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inputs() -> (Vec<nir::DeclarationParameter>, Vec<Term>) {
        (
            vec![nir::DeclarationParameter {
                declaration: DeclarationId {
                    package: 99,
                    local: 1,
                },
                sort: nir::Sort::Type,
            }],
            vec![Term::Primitive(Primitive::Bool)],
        )
    }
    #[test]
    fn shared_cells_remain_shared_but_do_not_alias_the_producer() {
        let (parameters, arguments) = inputs();
        let original = Storage::new(Value::TypeParameter(parameters[0].declaration));
        let packet = Value::Tuple(vec![
            Value::Storage(original.clone()),
            Value::Storage(original.clone()),
        ]);
        let restore =
            |steps: &mut u64| packet.restore_scoped_types(&parameters, &arguments, &[], steps);
        let mut steps = 1000;
        let Value::Tuple(values) = restore(&mut steps).unwrap() else {
            panic!("tuple")
        };
        let [Value::Storage(a), Value::Storage(b)] = values.as_slice() else {
            panic!("cells")
        };
        assert_eq!(a, b);
        assert_ne!(a, &original);
        assert_eq!(a.read().unwrap(), Value::Type(arguments[0].clone()));
        *a.0.lock().unwrap() = Value::Bool(false);
        assert_eq!(b.read().unwrap(), Value::Bool(false));
        assert_eq!(
            original.read().unwrap(),
            Value::TypeParameter(parameters[0].declaration)
        );
        let used = 1000 - steps;
        assert_eq!(restore(&mut (used - 1)), Err(ErrorKind::StepsExhausted));
        assert!(restore(&mut used.clone()).is_ok());
    }
    #[test]
    fn closure_body_signature_and_shared_capture_are_restored_together() {
        let (parameters, arguments) = inputs();
        let rigid = Term::Rigid {
            declaration: parameters[0].declaration,
            sort: nir::Sort::Type,
        };
        let slot = LocalId { owner: 1, index: 0 };
        let cell = Storage::new(Value::TypeParameter(parameters[0].declaration));
        let closure = Arc::new(Closure {
            definition_source: None,
            origin: None,
            instance_arguments: None,
            parameters: vec![LocalId { owner: 1, index: 1 }],
            capture_order: vec![slot],
            environment: [(slot, Value::Storage(cell.clone()))].into_iter().collect(),
            ty: Term::Closure {
                environment: vec![Term::Primitive(Primitive::Type)],
                signature: Box::new(
                    Term::Function {
                        parameters: vec![nir::Parameter {
                            sort: nir::Sort::ComptimeValue,
                            ty: rigid.clone(),
                        }],
                        result: Box::new(Term::Primitive(Primitive::Type)),
                    }
                    .with_comptime_phase(true),
                ),
            },
            body: Arc::new(Expr {
                source: SourceRange { start: 0, end: 1 },
                ty: Term::Primitive(Primitive::Type),
                kind: K::TypeValue(rigid.clone()),
            }),
        });
        let packet = Value::Tuple(vec![
            Value::Closure(closure.clone()),
            Value::Storage(cell.clone()),
            Value::Closure(closure.clone()),
        ]);
        let Value::Tuple(values) = packet
            .restore_scoped_types(&parameters, &arguments, &[], &mut 10000)
            .unwrap()
        else {
            panic!("tuple")
        };
        let [
            Value::Closure(a),
            Value::Storage(restored),
            Value::Closure(b),
        ] = values.as_slice()
        else {
            panic!("graph")
        };
        assert!(Arc::ptr_eq(a, b));
        assert!(!Arc::ptr_eq(a, &closure));
        assert_eq!(
            a.ty,
            closure
                .ty
                .instantiate_parameters(&parameters, &arguments)
                .unwrap()
        );
        assert!(matches!(&a.body.kind, K::TypeValue(ty) if ty == &arguments[0]));
        assert_eq!(a.environment[&slot], Value::Storage(restored.clone()));
        assert!(matches!(&closure.body.kind, K::TypeValue(ty) if ty == &rigid));
        assert_ne!(restored, &cell);
    }

    #[test]
    fn code_restores_types_and_preserves_ports_defaults_and_output_identity() {
        use nir::code_values::{ExpressionCode, PatternCode};
        use nir::program::Pattern;
        let (parameters, arguments) = inputs();
        let rigid = Term::Rigid {
            declaration: parameters[0].declaration,
            sort: nir::Sort::Type,
        };
        let input = LocalId { owner: 1, index: 0 };
        let output = LocalId { owner: 1, index: 1 };
        let external = LocalId { owner: 2, index: 0 };
        let body = Arc::new(Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: rigid.clone(),
            kind: K::Local(input),
        });
        let expression = Arc::new(ExpressionCode {
            body: Some(body.clone()),
            result: rigid.clone(),
            context: [(input, rigid.clone())].into_iter().collect(),
            ports: [("value".into(), input)].into_iter().collect(),
            defaults: [(input, external)].into_iter().collect(),
        });
        let pattern = Arc::new(PatternCode {
            subject: rigid.clone(),
            outputs: [("bound".into(), (output, rigid.clone()))]
                .into_iter()
                .collect(),
            body: Some(Pattern::Observe {
                binding: input,
                inner: Box::new(Pattern::Then {
                    first: Box::new(Pattern::Wild),
                    value: Box::new((*body).clone()),
                    next: Box::new(Pattern::Binding {
                        binding: output,
                        inner: Box::new(Pattern::Wild),
                    }),
                }),
            }),
        });
        let packet = Value::Tuple(vec![
            Value::Code(CodeValue::Expression(expression.clone())),
            Value::Code(CodeValue::Pattern(pattern.clone())),
            Value::Code(CodeValue::Expression(expression.clone())),
        ]);
        let mut steps = 10000;
        let Value::Tuple(values) = packet
            .restore_scoped_types(&parameters, &arguments, &[], &mut steps)
            .unwrap()
        else {
            panic!("tuple")
        };
        let [
            Value::Code(CodeValue::Expression(a)),
            Value::Code(CodeValue::Pattern(b)),
            Value::Code(CodeValue::Expression(c)),
        ] = values.as_slice()
        else {
            panic!("codes")
        };
        assert!(Arc::ptr_eq(a, c));
        assert!(!Arc::ptr_eq(a, &expression));
        assert_eq!(a.result, arguments[0]);
        assert_eq!(a.context[&input], arguments[0]);
        assert_eq!(a.ports, expression.ports);
        assert_eq!(a.defaults, expression.defaults);
        assert_eq!(a.body.as_ref().unwrap().ty, arguments[0]);
        assert_eq!(b.subject, arguments[0]);
        assert_eq!(b.outputs["bound"], (output, arguments[0].clone()));
        let Pattern::Observe { inner, .. } = b.body.as_ref().unwrap() else {
            panic!("observe")
        };
        let Pattern::Then { value, .. } = inner.as_ref() else {
            panic!("then")
        };
        assert_eq!(value.ty, arguments[0]);
        assert_eq!(expression.result, rigid);
        assert_eq!(pattern.subject, rigid);
        let used = 10000 - steps;
        assert_eq!(
            packet.restore_scoped_types(&parameters, &arguments, &[], &mut (used - 1)),
            Err(ErrorKind::StepsExhausted)
        );
        assert!(
            packet
                .restore_scoped_types(&parameters, &arguments, &[], &mut used.clone())
                .is_ok()
        );
    }

    #[test]
    fn code_transport_rejects_pending_bodies_and_invalid_ports() {
        use nir::code_values::ExpressionCode;
        let (parameters, arguments) = inputs();
        for pending in [true, false] {
            let code = Value::Code(CodeValue::Expression(Arc::new(ExpressionCode {
                body: (!pending).then(|| {
                    Arc::new(Expr {
                        source: SourceRange { start: 0, end: 1 },
                        ty: arguments[0].clone(),
                        kind: K::Literal(nir::program::Literal::Bool(true)),
                    })
                }),
                result: arguments[0].clone(),
                context: BTreeMap::new(),
                defaults: BTreeMap::new(),
                ports: [("missing".into(), LocalId { owner: 1, index: 0 })]
                    .into_iter()
                    .collect(),
            })));
            assert_eq!(
                code.restore_scoped_types(&parameters, &arguments, &[], &mut 1000),
                Err(if pending {
                    ErrorKind::Unsupported
                } else {
                    ErrorKind::Type
                })
            );
        }
    }

    #[test]
    fn cycles_fail_without_locking_or_mutating_the_producer() {
        let (parameters, arguments) = inputs();
        let cell = Storage::new(Value::Unit);
        *cell.0.lock().unwrap() = Value::Storage(cell.clone());
        let result = Value::Storage(cell.clone()).restore_scoped_types(
            &parameters,
            &arguments,
            &[],
            &mut 1000,
        );
        assert_eq!(result, Err(ErrorKind::Unsupported));
        // Break the intentionally constructed test cycle without recursive equality.
        *cell.0.lock().unwrap() = Value::Unit;
        assert_eq!(cell.read().unwrap(), Value::Unit);
    }
    #[test]
    fn nested_packets_stop_at_the_depth_limit_and_recover() {
        let (parameters, arguments) = inputs();
        let mut value = Value::Bool(true);
        for _ in 0..128 {
            value = Value::Tuple(vec![value]);
        }
        assert_eq!(
            value.restore_scoped_types(&parameters, &arguments, &[], &mut 10000),
            Err(ErrorKind::DepthExhausted)
        );
        assert_eq!(
            Value::Bool(true).restore_scoped_types(&parameters, &arguments, &[], &mut 100),
            Ok(Value::Bool(true))
        );
    }
}
