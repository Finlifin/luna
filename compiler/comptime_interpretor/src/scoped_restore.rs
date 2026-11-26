//! Transport invocation results into a caller telescope without mutating their VM.
use super::*;

mod packets;

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

    fn leaf(&mut self, value: &Value) -> Result<Value, ErrorKind> {
        Ok(match value {
            Value::TypeParameter(declaration) => Value::Type(self.ty(&Term::Rigid {
                declaration: *declaration,
                sort: nir::Sort::Type,
            })?),
            Value::Type(ty) => Value::Type(self.ty(ty)?),
            Value::NormalForm { term, ty } => Value::NormalForm {
                term: self.ty(term)?,
                ty: self.ty(ty)?,
            },
            Value::AbstractValue { declaration, ty } => {
                let symbol = self.ty(&Term::Rigid {
                    declaration: *declaration,
                    sort: nir::Sort::of_comptime_domain(ty),
                })?;
                let Term::Rigid { declaration, .. } = symbol else {
                    // An outer telescope slot has no standalone invocation
                    // value before application. Its use inside a Type is
                    // restored structurally by the Type case above.
                    return Err(ErrorKind::AbstractValueUnresolved);
                };
                Value::AbstractValue {
                    declaration,
                    ty: self.ty(ty)?,
                }
            }
            Value::FunctionInstance {
                declaration,
                arguments,
            } => Value::FunctionInstance {
                declaration: *declaration,
                arguments: self.types(arguments)?,
            },
            Value::IirCallable(handle) => Value::FunctionInstance {
                declaration: handle.key.declaration,
                arguments: self.types(&handle.key.arguments)?,
            },

            Value::Code(code) => self.code(code)?,
            Value::RuntimeTraitMethod(method) => {
                Value::RuntimeTraitMethod(Box::new(TraitMethodIdentity {
                    interface: InterfaceInstance {
                        declaration: method.interface.declaration,
                        arguments: self.types(&method.interface.arguments)?,
                    },
                    subject: self.ty(&method.subject)?,
                    member: method.member,
                    method_arguments: self.types(&method.method_arguments)?,
                }))
            }
            Value::Pointer(_) | Value::ComptimeReference(_) => {
                return Err(ErrorKind::Unsupported);
            }
            Value::Description(_) => return Err(ErrorKind::TypeContractUnresolved),
            Value::List { .. }
            | Value::Array { .. }
            | Value::Tuple(_)
            | Value::AnonymousRecord(_)
            | Value::Record { .. }
            | Value::Variant { .. }
            | Value::Optional { .. }
            | Value::ErrorResult { .. }
            | Value::Newtype { .. }
            | Value::Object(_)
            | Value::Storage(_)
            | Value::Closure(_) => unreachable!("graph nodes are restored by the packet traversal"),
            value => {
                let before = usize::try_from(*self.steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let size = value.inspect_immutable(&mut fuel);
                *self.steps -= (before - fuel) as u64;
                self.spend(size?.owned)?;
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
    fn locked_cells_respect_the_same_depth_limit_and_release_their_guards() {
        let (parameters, arguments) = inputs();
        let mut value = Value::TypeParameter(parameters[0].declaration);
        for _ in 0..127 {
            value = Value::Storage(Storage::new(value));
        }
        let restore =
            |value: &Value| value.restore_scoped_types(&parameters, &arguments, &[], &mut 10000);
        let mut restored = restore(&value).unwrap();
        for _ in 0..127 {
            let Value::Storage(cell) = restored else {
                panic!("missing cell")
            };
            restored = cell.read().unwrap();
        }
        assert_eq!(restored, Value::Type(arguments[0].clone()));
        let root = Storage::new(value);
        assert_eq!(
            restore(&Value::Storage(root.clone())),
            Err(ErrorKind::DepthExhausted)
        );
        assert!(restore(&root.read().unwrap()).is_ok());
    }

    #[test]
    fn nested_object_entries_keep_their_own_domains_and_source_order() {
        let (parameters, arguments) = inputs();
        let rigid = Term::Rigid {
            declaration: parameters[0].declaration,
            sort: nir::Sort::Type,
        };
        let wrap = |name: &str, ty: Term, value| ObjectEntry::Property {
            key: name.into(),
            value: ObjectValue { ty, value },
        };
        let packet = |field_ty: Term| {
            Value::Object(Object::from_entries(vec![
                wrap("tag", Term::Primitive(Primitive::Bool), Value::Bool(true)),
                wrap(
                    "inner",
                    Term::Primitive(Primitive::Object),
                    Value::Object(Object::from_entries(vec![
                        wrap("first", field_ty.clone(), Value::Unit),
                        wrap(
                            "second",
                            Term::Primitive(Primitive::Type),
                            Value::Type(field_ty.clone()),
                        ),
                    ])),
                ),
                wrap("tag", field_ty, Value::Unit),
            ]))
        };
        let restored = packet(rigid)
            .restore_scoped_types(&parameters, &arguments, &[], &mut 10000)
            .unwrap();
        assert_eq!(restored, packet(arguments[0].clone()));
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

#[cfg(test)]
mod method_identity_tests {
    use super::*;

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 98, local }
    }

    #[test]
    fn method_results_restore_every_input_into_the_caller_telescope() {
        let parameters = [
            nir::DeclarationParameter {
                declaration: id(1),
                sort: nir::Sort::Type,
            },
            nir::DeclarationParameter {
                declaration: id(2),
                sort: nir::Sort::ComptimeValue,
            },
        ];
        let rigid = |index: usize| Term::Rigid {
            declaration: parameters[index].declaration,
            sort: parameters[index].sort,
        };
        let identity = TraitMethodIdentity {
            interface: InterfaceInstance {
                declaration: id(3),
                arguments: vec![rigid(0), rigid(1)],
            },
            subject: Term::Optional(Box::new(rigid(0))),
            member: id(4),
            method_arguments: vec![rigid(1), rigid(0)],
        };
        let input = Value::Tuple(vec![Value::RuntimeTraitMethod(Box::new(identity.clone()))]);
        let arguments = [
            Term::Bound {
                depth: 0,
                index: 0,
                sort: nir::Sort::Type,
            },
            Term::Bool(true),
        ];
        let frames = [vec![nir::Sort::Type]];
        let expected = Value::Tuple(vec![Value::RuntimeTraitMethod(Box::new(
            TraitMethodIdentity {
                interface: InterfaceInstance {
                    declaration: id(3),
                    arguments: arguments.to_vec(),
                },
                subject: Term::Optional(Box::new(arguments[0].clone())),
                member: id(4),
                method_arguments: vec![arguments[1].clone(), arguments[0].clone()],
            },
        ))]);
        let restore =
            |steps: &mut u64| input.restore_scoped_types(&parameters, &arguments, &frames, steps);
        let mut steps = 10_000;
        assert_eq!(restore(&mut steps).unwrap(), expected);
        let cost = 10_000 - steps;
        for mut budget in 0..cost {
            assert_eq!(restore(&mut budget), Err(ErrorKind::StepsExhausted));
        }
        assert_eq!(restore(&mut cost.clone()).unwrap(), expected);
        assert_eq!(
            input,
            Value::Tuple(vec![Value::RuntimeTraitMethod(Box::new(identity))])
        );
        assert_eq!(
            input.restore_scoped_types(&parameters, &arguments, &[], &mut 10_000),
            Err(ErrorKind::Type)
        );
    }
}
