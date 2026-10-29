//! Invocation-local code transport, distinct from ordinary data domains.
//!
//! Transport preserves checked code descriptions without running their bodies.
//! Host construction is not an elaboration certificate: splice consumers still
//! establish contextual substitutions, typing, phase and resource obligations.
use super::*;
use nir::code_values::{ExpressionCode, PatternCode};

#[derive(Clone, Debug)]
pub enum CodeValue {
    Expression(Arc<ExpressionCode>),
    Pattern(Arc<PatternCode>),
}

// This is representation identity for internal Value comparisons, not a source
// Eq protocol or an equivalence decision for two pieces of generated code.
impl PartialEq for CodeValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Expression(a), Self::Expression(b)) => Arc::ptr_eq(a, b),
            (Self::Pattern(a), Self::Pattern(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl fmt::Display for CodeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expression(code) => code.fmt(f),
            Self::Pattern(code) => code.fmt(f),
        }
    }
}

impl CodeValue {
    fn charge(&self, steps: &mut u64) -> Result<(), ErrorKind> {
        fn entry(steps: &mut u64, size: usize) -> Result<(), ErrorKind> {
            *steps = steps
                .checked_sub(size as u64)
                .ok_or(ErrorKind::StepsExhausted)?;
            Ok(())
        }
        fn ty(steps: &mut u64, value: &Term) -> Result<(), ErrorKind> {
            let before = usize::try_from(*steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let result = nir::type_computations::charge_term(value, &mut fuel);
            *steps -= (before - fuel) as u64;
            result.map_err(|_| ErrorKind::StepsExhausted)
        }
        let body_result = match self {
            Self::Expression(code) => {
                let body = code.body.as_ref().ok_or(ErrorKind::Unsupported)?;
                entry(steps, code.context.len())?;
                entry(steps, code.defaults.len())?;
                for (name, binding) in &code.ports {
                    entry(steps, name.len().saturating_add(1))?;
                    if !code.context.contains_key(binding) {
                        return Err(ErrorKind::Type);
                    }
                }
                if code
                    .defaults
                    .keys()
                    .any(|id| !code.context.contains_key(id))
                {
                    return Err(ErrorKind::Type);
                }
                for value in code.context.values().chain(std::iter::once(&code.result)) {
                    ty(steps, value)?;
                }
                nir::copy_budget::charge_expression(body, steps)
            }
            Self::Pattern(code) => {
                let body = code.body.as_ref().ok_or(ErrorKind::Unsupported)?;
                ty(steps, &code.subject)?;
                for (name, (_, value)) in &code.outputs {
                    entry(steps, name.len().saturating_add(1))?;
                    ty(steps, value)?;
                }
                nir::copy_budget::charge_pattern_rewrite(body, steps)
            }
        };
        body_result.map_err(|error| match error {
            nir::copy_budget::CopyBudgetExhausted::Steps => ErrorKind::StepsExhausted,
            nir::copy_budget::CopyBudgetExhausted::Depth => ErrorKind::DepthExhausted,
        })?;
        // A present outer body may still hide a definition-side pattern marker.
        // Walk all nested runtime scopes without evaluating any observation.
        let before = usize::try_from(*steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let ready = (|| match self {
            Self::Expression(code) => {
                nir::visit::expression(code.body.as_ref().unwrap(), &mut fuel, |_, _| Ok(()))
            }
            Self::Pattern(code) => {
                for root in
                    nir::visit::pattern_observations(code.body.as_ref().unwrap(), &mut fuel)?
                {
                    nir::visit::expression(root, &mut fuel, |_, _| Ok(()))?;
                }
                Ok(())
            }
        })();
        *steps -= (before - fuel) as u64;
        ready.map_err(|message| {
            if message.contains("budget") {
                ErrorKind::StepsExhausted
            } else {
                ErrorKind::Unsupported
            }
        })
    }
}

impl Interpreter<'_> {
    pub(super) fn copy_code_value(
        &mut self,
        code: &CodeValue,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        code.charge(&mut self.budget.steps)
            .map_err(|kind| self.failure(kind, Some(source)))?;
        Ok(Value::Code(code.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::{LocalId, Pattern};

    fn id(index: u32) -> LocalId {
        LocalId { owner: 7, index }
    }

    fn expression(pending: bool, name: &str) -> Value {
        let ty = Term::Primitive(Primitive::Bool);
        Value::Code(CodeValue::Expression(Arc::new(ExpressionCode {
            body: (!pending).then(|| {
                Arc::new(Expr {
                    ty: ty.clone(),
                    source: SourceRange { start: 0, end: 1 },
                    kind: K::Local(id(1)),
                })
            }),
            result: ty.clone(),
            context: BTreeMap::from([(id(1), ty.clone()), (id(2), ty)]),
            ports: BTreeMap::from([(name.into(), id(1))]),
            defaults: BTreeMap::from([(id(1), id(9)), (id(2), id(2))]),
        })))
    }

    fn pattern(pending: bool, name: &str) -> Value {
        Value::Code(CodeValue::Pattern(Arc::new(PatternCode {
            body: (!pending).then(|| Pattern::Binding {
                binding: id(3),
                inner: Box::new(Pattern::Wild),
            }),
            subject: Term::Primitive(Primitive::Bool),
            outputs: BTreeMap::from([(name.into(), (id(3), Term::Primitive(Primitive::Bool)))]),
        })))
    }

    #[test]
    fn owned_transfers_preserve_code_category_and_binding_identity() {
        let original = Value::Tuple(vec![
            expression(false, "port\"\n"),
            pattern(false, "out\"\n"),
        ]);
        let owner = Storage::new(original.clone());
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for value in [
            vm.read_owned_value(&owner).unwrap(),
            vm.copy_owned_value(&owner).unwrap(),
        ] {
            assert_eq!(value, original);
            let Value::Tuple(values) = value else {
                panic!()
            };
            let Value::Code(CodeValue::Expression(code)) = &values[0] else {
                panic!()
            };
            assert_eq!(code.defaults[&id(1)], id(9));
            assert!(code.context.contains_key(&id(2)));
            assert!(!code.ports.values().any(|port| *port == id(2)));
            let Value::Code(CodeValue::Pattern(code)) = &values[1] else {
                panic!()
            };
            assert_eq!(code.outputs.values().next().unwrap().0, id(3));
        }
        assert_eq!(owner.read().unwrap(), original);
        assert!(original.to_string().contains("\"port\\\"\\n\""));
        assert!(original.to_string().contains("\"out\\\"\\n\""));
    }

    #[test]
    fn code_bodies_are_retained_without_execution() {
        let body = Expr {
            ty: Term::Primitive(Primitive::Never),
            source: SourceRange { start: 0, end: 1 },
            kind: K::Loop {
                identity: id(1),
                body: Box::new(Expr {
                    ty: Term::Primitive(Primitive::Void),
                    source: SourceRange { start: 0, end: 1 },
                    kind: K::Unit,
                }),
            },
        };
        let code = Value::Code(CodeValue::Expression(Arc::new(ExpressionCode {
            result: body.ty.clone(),
            body: Some(Arc::new(body)),
            context: BTreeMap::new(),
            ports: BTreeMap::new(),
            defaults: BTreeMap::new(),
        })));
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 100,
                depth: 128,
            },
        )
        .unwrap();
        assert_eq!(
            vm.read_owned_value(&Storage::new(code.clone())).unwrap(),
            code
        );
        assert!(vm.remaining_steps() > 0);
    }

    #[test]
    fn pending_code_cannot_enter_owned_transfers() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for code in [expression(true, "input"), pattern(true, "output")] {
            let original = Value::Tuple(vec![code]);
            let owner = Storage::new(original.clone());
            assert_eq!(
                vm.read_owned_value(&owner).unwrap_err().kind,
                ErrorKind::Unsupported
            );
            assert_eq!(
                vm.copy_owned_value(&owner).unwrap_err().kind,
                ErrorKind::Unsupported
            );
            assert_eq!(owner.read().unwrap(), original);
        }
    }

    #[test]
    fn nested_definition_evidence_and_deep_bodies_are_rejected() {
        let marker = nir::program::Pattern::CodeEvidence { outputs: vec![] };
        let leaf = Expr {
            ty: Term::Primitive(Primitive::Bool),
            source: SourceRange { start: 0, end: 1 },
            kind: K::Literal(Literal::Bool(true)),
        };
        let mut code = match expression(false, "input") {
            Value::Code(CodeValue::Expression(code)) => Arc::try_unwrap(code).unwrap(),
            _ => unreachable!(),
        };
        code.body = Some(Arc::new(Expr {
            kind: K::Match {
                value: Box::new(leaf.clone()),
                arms: vec![nir::program::MatchArm {
                    pattern: marker.clone(),
                    guard: None,
                    body: leaf.clone(),
                }],
            },
            ..leaf.clone()
        }));
        let pattern = PatternCode {
            body: Some(marker),
            subject: leaf.ty.clone(),
            outputs: BTreeMap::new(),
        };
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for value in [
            CodeValue::Expression(Arc::new(code)),
            CodeValue::Pattern(Arc::new(pattern)),
        ] {
            assert_eq!(
                vm.read_owned_value(&Storage::new(Value::Code(value)))
                    .unwrap_err()
                    .kind,
                ErrorKind::Unsupported
            );
        }
        let mut body = leaf.clone();
        for _ in 0..170 {
            body = Expr {
                kind: K::Not(Box::new(body)),
                ..leaf.clone()
            };
        }
        let deep = Value::Code(CodeValue::Expression(Arc::new(ExpressionCode {
            body: Some(Arc::new(body)),
            result: leaf.ty,
            context: BTreeMap::new(),
            ports: BTreeMap::new(),
            defaults: BTreeMap::new(),
        })));
        assert_eq!(
            vm.read_owned_value(&Storage::new(deep)).unwrap_err().kind,
            ErrorKind::DepthExhausted
        );
        let valid = expression(false, "input");
        assert_eq!(
            vm.read_owned_value(&Storage::new(valid.clone())).unwrap(),
            valid
        );
    }

    #[test]
    fn code_is_not_runtime_data_or_a_normal_term_cache_key() {
        let package = Package::default();
        let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for code in [expression(false, "input"), pattern(false, "output")] {
            for value in [code.clone(), Value::Tuple(vec![code])] {
                let owner = Storage::new(value.clone());
                assert_eq!(
                    runtime.read_owned_value(&owner).unwrap_err().kind,
                    ErrorKind::PhaseMismatch
                );
                assert_eq!(value.immutable_copy_work(), None);
                for ty in [Primitive::Type, Primitive::Bool, Primitive::Object] {
                    assert!(
                        vm.validate_immutable_input(&value, &Term::Primitive(ty))
                            .is_err()
                    );
                }
                assert_eq!(
                    value.into_normal_term(&mut Default::default()),
                    Err(ErrorKind::Unsupported)
                );
            }
        }
    }

    #[test]
    fn budget_failure_and_rollback_preserve_original_code() {
        let package = Package::default();
        for code in [
            expression(false, &"x".repeat(1024)),
            pattern(false, &"x".repeat(1024)),
        ] {
            let root = Storage::new(Value::Tuple(vec![code.clone()]));
            let mut vm = Interpreter::for_comptime(
                &package,
                Budget {
                    steps: 32,
                    depth: 128,
                },
            )
            .unwrap();
            assert_eq!(
                vm.read_owned_value(&root).unwrap_err().kind,
                ErrorKind::StepsExhausted
            );
            assert_eq!(root.read().unwrap(), Value::Tuple(vec![code.clone()]));
            vm.budget.steps = 10_000;
            let saved = vm
                .checkpoint_owned_graph(&[Value::Storage(root.clone())])
                .unwrap();
            *root.0.lock().unwrap() = Value::Bool(false);
            saved.restore().unwrap();
            assert_eq!(
                vm.read_owned_value(&root).unwrap(),
                Value::Tuple(vec![code])
            );
        }
    }
}
