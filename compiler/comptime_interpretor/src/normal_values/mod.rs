//! Decode canonical inputs without carrying unrelated representation branches
//! through recursive calls. Entry checks and budget/depth order are shared.
use super::*;
mod containers;
mod scalar;

impl Interpreter<'_> {
    pub(super) fn normal_pattern_value(&mut self, term: &Term, ty: &Term) -> Result<Value, Error> {
        self.normal_pattern_value_at(term, ty, 0, true)
    }
    #[inline(never)]
    pub(super) fn normal_pattern_value_at(
        &mut self,
        term: &Term,
        ty: &Term,
        depth: usize,
        reconstruct_callables: bool,
    ) -> Result<Value, Error> {
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, None));
        }
        self.tick(None)?;
        if !reconstruct_callables
            && matches!(
                term,
                Term::ClosureSnapshot { .. } | Term::FunctionValue { .. }
            )
        {
            return Err(self.failure(ErrorKind::Type, None));
        }

        if term.has_pending_error_branch() {
            return Err(self.failure(ErrorKind::AbstractValueUnresolved, None));
        }
        if let Term::Rigid {
            declaration,
            sort: nir::Sort::ComptimeValue,
        } = term
        {
            if self.comptime_only && self.abstract_value_parameters.get(declaration) == Some(ty) {
                return Ok(Value::AbstractValue {
                    declaration: *declaration,
                    ty: ty.clone(),
                });
            }
            return Err(self.failure(ErrorKind::Type, None));
        }
        let work = if *ty == Term::Primitive(Primitive::Type) {
            self.type_description_work(term)
                .ok_or_else(|| self.failure(ErrorKind::Type, None))?
        } else {
            match term {
                Term::Integer(value) => value.bit_len().div_ceil(32),
                Term::Float(value) => value
                    .digits()
                    .len()
                    .saturating_add(value.exponent().bit_len().div_ceil(32)),
                Term::String(value) | Term::Symbol(value) => value.len(),
                Term::BitVec(value) => value.words().len(),
                Term::TupleValue(values) => values.len(),
                Term::OptionalValue { ty, value } => self
                    .type_description_work(ty)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?
                    .saturating_add(usize::from(value.is_some())),
                Term::ErrorValue { ty, label, .. } => {
                    let mut work = self
                        .type_description_work(ty)
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    if let Some(label) = label {
                        work = work.saturating_add(
                            self.type_description_work(label)
                                .ok_or_else(|| self.failure(ErrorKind::Type, None))?,
                        );
                    }
                    work.saturating_add(1)
                }
                Term::NominalValue { ty, fields, .. } => fields.len().saturating_add(
                    self.type_description_work(ty)
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?,
                ),
                _ => 0,
            }
        };
        self.integer_operation(None, |budget| budget.consume(work))?;
        let value = if *ty == Term::Primitive(Primitive::Type) {
            Value::Type(term.clone())
        } else {
            match term {
                Term::OptionalValue { .. } => {
                    self.decode_optional_normal_value(term, ty, depth, reconstruct_callables)?
                }
                Term::ErrorValue { .. } => {
                    self.decode_error_normal_value(term, ty, depth, reconstruct_callables)?
                }
                Term::NominalValue { .. } => match ty {
                    Term::Primitive(Primitive::Object) => {
                        self.decode_object_normal_value(term, ty, depth, reconstruct_callables)?
                    }
                    Term::NormalForm { .. } => {
                        self.decode_list_normal_value(term, ty, depth, reconstruct_callables)?
                    }
                    _ => {
                        self.decode_nominal_normal_value(term, ty, depth, reconstruct_callables)?
                    }
                },
                Term::RecordValue(_) => {
                    self.decode_record_normal_value(term, ty, depth, reconstruct_callables)?
                }
                Term::TupleValue(_) => {
                    self.decode_tuple_normal_value(term, ty, depth, reconstruct_callables)?
                }
                _ => self.decode_scalar_normal_value(term, ty)?,
            }
        };
        if !self.has_type(&value, ty) {
            return Err(self.failure(ErrorKind::Type, None));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested_tuple(depth: usize) -> (Term, Term) {
        let mut ty = Term::Primitive(Primitive::Bool);
        let mut value = Term::Bool(true);
        for _ in 0..depth {
            ty = Term::Tuple(vec![ty]);
            value = Term::TupleValue(vec![value]);
        }
        (value, ty)
    }

    #[test]
    fn tuple_inputs_keep_the_exact_depth_boundary_and_recover() {
        let package = nir::program::Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let (value, ty) = nested_tuple(127);
        let mut decoded = vm.normal_pattern_value(&value, &ty).unwrap();
        for _ in 0..127 {
            let Value::Tuple(mut fields) = decoded else {
                panic!("missing tuple layer")
            };
            assert_eq!(fields.len(), 1);
            decoded = fields.pop().unwrap();
        }
        assert_eq!(decoded, Value::Bool(true));
        for depth in [128, 140] {
            let (value, ty) = nested_tuple(depth);
            assert_eq!(
                vm.normal_pattern_value(&value, &ty).unwrap_err().kind,
                ErrorKind::DepthExhausted
            );
            assert_eq!(
                vm.normal_pattern_value(&Term::Bool(false), &Term::Primitive(Primitive::Bool))
                    .unwrap(),
                Value::Bool(false)
            );
        }
    }

    #[test]
    fn tuple_input_budget_and_field_errors_do_not_publish_partial_values() {
        let package = nir::program::Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let (value, ty) = nested_tuple(16);
        vm.budget.steps = 8;
        assert_eq!(
            vm.normal_pattern_value(&value, &ty).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        vm.budget.steps = 100_000;
        let wrong = Term::TupleValue(vec![Term::Bool(true), Term::Char('x')]);
        let expected = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 2]);
        assert_eq!(
            vm.normal_pattern_value(&wrong, &expected).unwrap_err().kind,
            ErrorKind::Type
        );
        assert_eq!(
            vm.normal_pattern_value(&Term::Bool(true), &Term::Primitive(Primitive::Bool))
                .unwrap(),
            Value::Bool(true)
        );
    }
}
