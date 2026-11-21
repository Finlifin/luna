//! Ordinary call preparation belongs to the original invocation. Opening a
//! slot selects its source position; only its normal result can fill the slot.
use super::*;

pub(super) struct CallOperands {
    slots: Vec<Option<Value>>,
    active: Option<usize>,
    last_default: Option<usize>,
    default_bindings: Vec<LocalId>,
    unimported: BTreeMap<LocalId, usize>,
    pub(super) defaults: Environment,
}

impl CallOperands {
    pub(super) fn open(
        vm: &mut Interpreter<'_>,
        callee: &Value,
        interface: Option<&[LocalId]>,
        bindings: &[LocalId],
        needs_defaults: bool,
        source: SourceRange,
    ) -> Result<Self, Error> {
        let parameters = vm.call_parameters(callee, source)?;
        let count = parameters.len();
        if !bindings.is_empty() && bindings.len() != count {
            return Err(vm.failure(ErrorKind::InvalidCallPlan, Some(source)));
        }
        // Charge the slot scan before comparing even a malformed routing table.
        let remaining = vm
            .remaining_steps()
            .checked_sub(count as u64)
            .ok_or_else(|| vm.failure(ErrorKind::StepsExhausted, Some(source)))?;
        let valid = bindings.is_empty() || bindings == interface.unwrap_or(parameters);
        vm.budget.steps = remaining;
        if !valid {
            return Err(vm.failure(ErrorKind::InvalidCallPlan, Some(source)));
        }
        Ok(Self {
            slots: vec![None; count],
            active: None,
            last_default: None,
            default_bindings: if needs_defaults {
                bindings.to_vec()
            } else {
                Vec::new()
            },
            unimported: BTreeMap::new(),
            defaults: Environment::new(),
        })
    }

    pub(super) fn begin(
        &mut self,
        vm: &Interpreter<'_>,
        slot: usize,
        is_default: bool,
        passing: nir::program::ArgumentPassing,
        argument: SourceRange,
        call: SourceRange,
    ) -> Result<(), Error> {
        if passing != nir::program::ArgumentPassing::Value
            && (!vm.comptime_only || is_default || slot != 0)
        {
            return Err(vm.failure(ErrorKind::InvalidCallPlan, Some(argument)));
        }
        if self.active.is_some() || !matches!(self.slots.get(slot), Some(None)) {
            return Err(vm.failure(ErrorKind::InvalidCallPlan, Some(call)));
        }
        if is_default {
            if self.default_bindings.is_empty()
                || self.last_default.is_some_and(|last| last >= slot)
            {
                return Err(vm.failure(ErrorKind::InvalidCallPlan, Some(call)));
            }
            self.last_default = Some(slot);
        } else if self.last_default.is_some() {
            return Err(vm.failure(ErrorKind::InvalidCallPlan, Some(call)));
        }
        self.active = Some(slot);
        Ok(())
    }

    pub(super) fn supply(
        &mut self,
        vm: &mut Interpreter<'_>,
        slot: usize,
        value: Value,
        source: SourceRange,
    ) -> Result<(), Error> {
        if self.active != Some(slot) {
            return Err(vm.failure(ErrorKind::InvalidCallPlan, Some(source)));
        }
        let value = vm.prepare_argument(slot, source, value)?;
        if let Some(&binding) = self.default_bindings.get(slot) {
            if vm.cleanup_schedule.is_some() {
                // Scheduled cleanup can demand bindings outside the default's
                // lexical expression. Its slots already hold storage handles.
                self.defaults.insert(binding, value.clone());
            } else {
                self.unimported.insert(binding, slot);
            }
        }
        self.slots[slot] = Some(value);
        self.active = None;
        Ok(())
    }

    /// A delivered slot owns its value. Publish into the separate, writable
    /// default environment only on first demand. A later default sees writes
    /// and moves in that environment, including a binding removed entirely.
    pub(super) fn import(
        &mut self,
        vm: &mut Interpreter<'_>,
        bindings: &[LocalId],
        source: SourceRange,
    ) -> Result<(), Error> {
        if self.unimported.is_empty() {
            return Ok(());
        }
        // Preflight the complete request before publishing any copies. Failure
        // spends its work and leaves both the prefix and prior writes intact.
        vm.charge_preparation(bindings.len().saturating_mul(2))
            .map_err(|e| vm.failure(e.kind, Some(source)))?;
        for binding in bindings {
            if let Some(&slot) = self.unimported.get(binding) {
                let supplied = self.slots[slot].as_ref().expect("delivered input");
                memory::charge_retained_clone(supplied, &mut vm.budget.steps)
                    .map_err(|kind| vm.failure(kind, Some(source)))?;
            }
        }
        for binding in bindings {
            if let Some(slot) = self.unimported.remove(binding) {
                let supplied = self.slots[slot].as_ref().expect("charged input");
                self.defaults.insert(*binding, supplied.clone());
            }
        }
        Ok(())
    }

    pub(super) fn import_default(
        &mut self,
        vm: &mut Interpreter<'_>,
        expression: &Expr,
    ) -> Result<(), Error> {
        if self.unimported.is_empty() {
            return Ok(());
        }
        let before = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let imports =
            comptime_compiler::default_imports(expression, &self.default_bindings, &mut fuel);
        vm.budget.steps -= (before - fuel) as u64;
        let imports =
            imports.map_err(|_| vm.failure(ErrorKind::StepsExhausted, Some(expression.source)))?;
        self.import(vm, &imports, expression.source)
    }

    pub(super) fn finish(
        self,
        vm: &Interpreter<'_>,
        source: SourceRange,
    ) -> Result<(Vec<Value>, Environment), Error> {
        let arguments = self
            .slots
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| vm.failure(ErrorKind::Arity, Some(source)))?;
        Ok((arguments, self.defaults))
    }
}

impl fmt::Display for CallOperands {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(operands (arguments")?;
        for (slot, value) in self.slots.iter().enumerate() {
            match value {
                Some(value) => write!(f, " (ready {value})")?,
                None if self.active == Some(slot) => f.write_str(" (evaluating)")?,
                None => f.write_str(" (pending)")?,
            }
        }
        f.write_str(") (unimported")?;
        for (binding, slot) in &self.unimported {
            write!(f, " ({binding} {slot})")?;
        }
        f.write_str(") (defaults")?;
        for (binding, value) in &self.defaults {
            write!(f, " ({binding} {value})")?;
        }
        f.write_str(") (last-default")?;
        if let Some(slot) = self.last_default {
            write!(f, " {slot}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::ArgumentPassing::Value as ByValue;

    #[test]
    fn invalid_binding_routes_pay_for_the_slot_scan() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let source = SourceRange { start: 12, end: 16 };
        let bindings: Vec<_> = (0..128).map(|index| LocalId { owner: 1, index }).collect();
        let ty = Term::Primitive(Primitive::Void);
        let callee = Value::Closure(Arc::new(Closure {
            definition_source: None,
            parameters: bindings.clone(),
            environment: Environment::new(),
            capture_order: vec![],
            body: Arc::new(Expr {
                source,
                ty: ty.clone(),
                kind: K::Unit,
            }),
            ty: Term::ComptimeFunction(Box::new(Term::Function {
                parameters: bindings
                    .iter()
                    .map(|_| nir::Parameter {
                        sort: nir::Sort::Runtime,
                        ty: ty.clone(),
                    })
                    .collect(),
                result: Box::new(ty),
            })),
        }));
        let mut invalid = bindings.clone();
        invalid.last_mut().unwrap().owner += 1;
        for (fuel, expected, remaining) in [
            (127, ErrorKind::StepsExhausted, 127),
            (128, ErrorKind::InvalidCallPlan, 0),
        ] {
            vm.budget.steps = fuel;
            let error = CallOperands::open(&mut vm, &callee, None, &invalid, false, source)
                .err()
                .expect("invalid late binding");
            assert_eq!(
                (error.kind, error.source, vm.remaining_steps()),
                (expected, Some(source), remaining)
            );
        }
    }

    #[test]
    fn only_the_active_operand_can_deliver_and_defaults_own_their_writes() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let source = SourceRange { start: 12, end: 16 };
        let bindings = [0, 1, 2].map(|index| LocalId { owner: 1, index });
        let mut frame = CallOperands {
            slots: vec![None; 3],
            active: None,
            last_default: None,
            default_bindings: bindings.to_vec(),
            unimported: BTreeMap::new(),
            defaults: Environment::new(),
        };
        frame.begin(&vm, 1, false, ByValue, source, source).unwrap();
        assert_eq!(
            frame
                .supply(&mut vm, 0, Value::Bool(false), source)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidCallPlan
        );
        assert!(frame.begin(&vm, 0, false, ByValue, source, source).is_err());
        assert!(
            frame
                .to_string()
                .contains("(pending) (evaluating) (pending)")
        );
        frame.supply(&mut vm, 1, Value::Bool(true), source).unwrap();
        assert!(
            frame
                .supply(&mut vm, 1, Value::Bool(false), source)
                .is_err()
        );
        frame.begin(&vm, 0, true, ByValue, source, source).unwrap();
        frame.import(&mut vm, &bindings[1..2], source).unwrap();
        assert_eq!(frame.defaults.get(&bindings[1]), Some(&Value::Bool(true)));
        frame.defaults.insert(bindings[1], Value::Bool(false));
        frame
            .supply(&mut vm, 0, Value::Bool(false), source)
            .unwrap();
        assert!(frame.begin(&vm, 2, false, ByValue, source, source).is_err());
        frame.begin(&vm, 2, true, ByValue, source, source).unwrap();
        frame.import(&mut vm, &bindings[1..2], source).unwrap();
        frame.supply(&mut vm, 2, Value::Unit, source).unwrap();
        let (arguments, defaults) = frame.finish(&vm, source).unwrap();
        assert_eq!(
            arguments,
            [Value::Bool(false), Value::Bool(true), Value::Unit]
        );
        assert_eq!(defaults.get(&bindings[1]), Some(&Value::Bool(false)));
    }
    #[test]
    fn only_reached_imports_copy_owned_values_and_never_revive_moved_locals() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let source = SourceRange { start: 10, end: 14 };
        let names = [0, 1, 2].map(|index| LocalId { owner: 7, index });
        let mut frame = CallOperands {
            slots: vec![None; names.len()],
            active: None,
            last_default: None,
            default_bindings: names.to_vec(),
            unimported: BTreeMap::new(),
            defaults: Environment::new(),
        };
        let large = Value::Type(Term::Tuple(vec![Term::Primitive(Primitive::Bool); 4096]));
        vm.budget.steps = 32;
        for (index, value) in [(1, Value::Bool(true)), (0, large)] {
            frame
                .begin(&vm, index, false, ByValue, source, source)
                .unwrap();
            frame.supply(&mut vm, index, value, source).unwrap();
        }
        frame.begin(&vm, 2, true, ByValue, source, source).unwrap();
        assert!(frame.defaults.is_empty());
        let failed = frame
            .import(&mut vm, &[names[1], names[0]], source)
            .unwrap_err();
        assert_eq!(failed.kind, ErrorKind::StepsExhausted);
        assert!(frame.defaults.is_empty());
        assert_eq!(frame.unimported.len(), 2);
        vm.budget.steps = 32;
        frame.import(&mut vm, &names[1..2], source).unwrap();
        assert_eq!(frame.defaults.remove(&names[1]), Some(Value::Bool(true)));
        frame.import(&mut vm, &names[1..2], source).unwrap();
        assert!(frame.defaults.is_empty());
        let error = frame.import(&mut vm, &names[..1], source).unwrap_err();
        assert_eq!(
            (error.kind, error.source),
            (ErrorKind::StepsExhausted, Some(source))
        );
        assert!(!frame.defaults.contains_key(&names[0]));
        assert_eq!(frame.unimported.get(&names[0]), Some(&0));
        assert!(
            matches!(&frame.slots[0], Some(Value::Type(Term::Tuple(fields))) if fields.len() == 4096)
        );
    }
}
