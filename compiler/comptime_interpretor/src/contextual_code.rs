//! Bounded compiled-input reuse. Entries contain immutable descriptions and
//! code, never input values, proof outcomes, or partially executed frames.
use super::*;
use comptime_compiler::{Program, normal_types::Shape};
use nir::program::NormalFormArgument;

#[derive(Clone)]
pub(super) struct Code {
    pub program: Arc<Program>,
    pub shape: Option<Arc<Shape>>,
}
struct Entry {
    input: NormalFormArgument,
    expected: Option<Term>,
    code: Option<Code>,
    key_work: u64,
    weight: u64,
}
#[derive(Default)]
pub(super) struct Cache {
    entries: Vec<Entry>,
    weight: u64,
}
impl Cache {
    fn lookup(
        &self,
        input: &NormalFormArgument,
        expected: Option<&Term>,
        fuel: &mut u64,
    ) -> Result<Option<Option<Code>>, ()> {
        for entry in self.entries.iter().rev() {
            *fuel = fuel.checked_sub(1).ok_or(())?;
            if entry.input.source() != input.source()
                || entry.expected.is_some() != expected.is_some()
            {
                continue;
            }
            *fuel = fuel.checked_sub(entry.key_work).ok_or(())?;
            if &entry.input == input && entry.expected.as_ref() == expected {
                return Ok(Some(entry.code.clone()));
            }
        }
        Ok(None)
    }
    fn insert(&mut self, entry: Entry) {
        if entry.weight > 100_000 {
            return;
        }
        if self.entries.len() >= 64 || self.weight.saturating_add(entry.weight) > 100_000 {
            self.entries.clear();
            self.weight = 0;
        }
        self.weight += entry.weight;
        self.entries.push(entry);
    }
}
impl Interpreter<'_> {
    pub(super) fn contextual_input_code(
        &mut self,
        input: &NormalFormArgument,
        expected: Option<&Term>,
    ) -> Result<Option<Code>, Error> {
        let source = input.source();
        // Bound recursive comparison and copying before looking at owned keys.
        let before = self.budget.steps;
        nir::copy_budget::charge_argument_type_rewrite(input, &mut self.budget.steps).map_err(
            |error| {
                self.failure(
                    match error {
                        nir::copy_budget::CopyBudgetExhausted::Steps => ErrorKind::StepsExhausted,
                        nir::copy_budget::CopyBudgetExhausted::Depth => ErrorKind::DepthExhausted,
                    },
                    Some(source),
                )
            },
        )?;
        if let Some(expected) = expected {
            self.charge_copy_work(Some(source), |fuel| {
                nir::copy_budget::charge_type_substitution(expected, &[], &[], fuel)
            })?;
        }
        let key_work = before - self.budget.steps;
        if let Some(code) = self
            .contextual_code_cache
            .lookup(input, expected, &mut self.budget.steps)
            .map_err(|()| self.failure(ErrorKind::StepsExhausted, Some(source)))?
        {
            return Ok(code);
        }
        let before = self.budget.steps;
        let code = if let Some(expected) = expected {
            self.compile_bytecode(|fuel| {
                comptime_compiler::compile_contextual_input(input, expected, fuel)
            })?
            .map(|program| Code {
                program: Arc::new(program),
                shape: None,
            })
        } else {
            self.compile_bytecode(|fuel| comptime_compiler::compile_staged_input(input, fuel))?
                .map(|(program, shape)| Code {
                    program: Arc::new(program),
                    shape: Some(Arc::new(shape)),
                })
        };
        let weight = (before - self.budget.steps)
            .saturating_add(key_work)
            .saturating_add(1);
        if weight <= 100_000 {
            self.budget.steps = self
                .budget
                .steps
                .checked_sub(key_work)
                .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
            self.contextual_code_cache.insert(Entry {
                input: input.clone(),
                expected: expected.cloned(),
                code: code.clone(),
                key_work,
                weight,
            });
        }
        Ok(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
    fn literal(value: &str) -> NormalFormArgument {
        NormalFormArgument::Integer {
            source: SOURCE,
            spelling: value.into(),
            negative: false,
        }
    }
    #[test]
    fn input_and_domain_identity_control_code_reuse() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let input = literal("17");
        let u8 = Term::Primitive(Primitive::U8);
        let first = vm
            .contextual_input_code(&input, Some(&u8))
            .unwrap()
            .unwrap();
        let count = vm.bytecode_compilations();
        let copy = vm
            .contextual_input_code(&input.clone(), Some(&u8))
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&first.program, &copy.program));
        assert_eq!(vm.bytecode_compilations(), count);
        let other = vm
            .contextual_input_code(&input, Some(&Term::Primitive(Primitive::U16)))
            .unwrap()
            .unwrap();
        assert!(!Arc::ptr_eq(&first.program, &other.program));
        let changed = vm
            .contextual_input_code(&literal("18"), Some(&u8))
            .unwrap()
            .unwrap();
        assert!(!Arc::ptr_eq(&first.program, &changed.program));
        let staged = vm.contextual_input_code(&input, None).unwrap().unwrap();
        let repeated = vm.contextual_input_code(&input, None).unwrap().unwrap();
        assert!(Arc::ptr_eq(&staged.program, &repeated.program));
        assert!(Arc::ptr_eq(
            staged.shape.as_ref().unwrap(),
            repeated.shape.as_ref().unwrap()
        ));
        assert_eq!(vm.bytecode_compilations(), count + 3);
    }
    #[test]
    fn failed_preparation_does_not_poison_code_cache() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let input = literal("17");
        let expected = Term::Primitive(Primitive::U8);
        let mut failed = 0;
        for steps in 0..20 {
            vm.contextual_code_cache = Cache::default();
            vm.budget.steps = steps;
            if vm.contextual_input_code(&input, Some(&expected)).is_err() {
                failed += 1;
                assert!(vm.contextual_code_cache.entries.is_empty());
                vm.budget = Budget::default();
                assert!(
                    vm.contextual_input_code(&input, Some(&expected))
                        .unwrap()
                        .is_some()
                );
            }
        }
        assert!(failed > 1);
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;
    #[test]
    fn shared_code_reads_fresh_environments_and_stays_in_its_source_world() {
        let package = Arc::new(Package::default());
        let mut session = CodeSession::new(package.clone());
        let binding = LocalId { owner: 1, index: 0 };
        let expected = Term::Primitive(Primitive::Bool);
        let input = NormalFormArgument::Value(Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: expected.clone(),
            kind: K::Local(binding),
        });
        for (index, value) in [true, false, true].into_iter().enumerate() {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut environment = Environment::new();
            environment.insert(binding, Value::Bool(value));
            let result = vm
                .with_code_session(&mut session, |vm| {
                    vm.evaluate_contextual_argument(&input, &expected, &mut environment)
                })
                .unwrap();
            assert!(matches!(result,Flow::Value(Value::Bool(actual)) if actual==value));
            assert_eq!(vm.bytecode_compilations(), usize::from(index == 0));
            assert_eq!(vm.bytecode_contextual_inputs(), 1);
        }
        let other = (*package).clone();
        let mut vm = Interpreter::for_comptime(&other, Budget::default()).unwrap();
        vm.with_code_session(&mut session, |vm| {
            vm.contextual_input_code(&input, Some(&expected))
        })
        .unwrap();
        assert_eq!(vm.bytecode_compilations(), 1);
    }
}
