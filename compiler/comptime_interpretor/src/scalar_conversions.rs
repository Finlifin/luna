//! Exact integer conversion and representation reads after operand evaluation.
use super::*;
impl Interpreter<'_> {
    pub(super) fn convert_comptime_integer(
        &mut self,
        scalar: Scalar<'_>,
        value: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if !self.has_type(&value, scalar.left) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        let integer = match value {
            Value::Integer(value) => value,
            Value::UnsignedInteger(value) => comptime_runtime::Integer::from_u128(value),
            Value::SignedInteger(value) => comptime_runtime::Integer::from_i128(value),
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        };
        let Term::Primitive(target) = *scalar.result else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        if target == Primitive::Integer {
            return Ok(Value::Integer(integer));
        }
        let (signed, width) = self
            .integer_layout(target)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        self.tick(Some(source))?;
        if signed {
            integer.to_signed(width).map(Value::SignedInteger)
        } else {
            integer.to_unsigned(width).map(Value::UnsignedInteger)
        }
        .map_err(|_| self.failure(ErrorKind::MachineOverflow, Some(source)))
    }
    pub(super) fn read_integer_representation(
        &self,
        scalar: Scalar<'_>,
        value: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let Term::Primitive(primitive) = *scalar.left else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        let (_, width) = self
            .integer_layout(primitive)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        if !self.has_type(&value, scalar.left) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        let bits = match value {
            Value::UnsignedInteger(value) => value,
            Value::SignedInteger(value) => value as u128,
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        };
        Ok(Value::UnsignedInteger(bits & (u128::MAX >> (128 - width))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::Place;
    const SOURCE: SourceRange = SourceRange { start: 7, end: 11 };
    const INPUT: LocalId = LocalId { owner: 1, index: 0 };
    const TRACE: LocalId = LocalId { owner: 1, index: 1 };
    fn expr(ty: Primitive, kind: K) -> Expr {
        Expr {
            source: SOURCE,
            ty: Term::Primitive(ty),
            kind,
        }
    }
    fn body(representation: bool, input: Primitive, output: Primitive) -> Expr {
        let operand = Box::new(expr(
            input,
            K::Block {
                statements: vec![expr(
                    Primitive::Void,
                    K::Assign {
                        place: Place {
                            root: TRACE,
                            fields: vec![],
                        },
                        value: Box::new(expr(
                            Primitive::Bool,
                            K::Not(Box::new(expr(Primitive::Bool, K::Local(TRACE)))),
                        )),
                    },
                )],
                result: Some(Box::new(expr(input, K::Local(INPUT)))),
            },
        ));
        expr(
            output,
            if representation {
                K::IntegerRepresentation(operand)
            } else {
                K::ComptimeIntegerConvert(operand)
            },
        )
    }
    fn execute(
        vm: &mut Interpreter<'_>,
        body: &Expr,
        env: &mut Environment,
        compiled: bool,
    ) -> Result<Flow, Error> {
        if compiled {
            let code = Arc::new(comptime_compiler::compile(body, &mut 10000).unwrap());
            vm.execute_code(&code, env)
        } else {
            vm.eval(body, env)
        }
    }
    #[test]
    fn scalar_conversion_admission_and_value_errors_keep_operand_effect_order() {
        let package = Package::default();
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            for (representation, input, output, value, error, observed) in [
                (
                    false,
                    Primitive::Bool,
                    Primitive::U8,
                    Value::Bool(true),
                    ErrorKind::Type,
                    false,
                ),
                (
                    true,
                    Primitive::I8,
                    Primitive::U8,
                    Value::SignedInteger(-1),
                    ErrorKind::Type,
                    false,
                ),
                (
                    false,
                    Primitive::Integer,
                    Primitive::U8,
                    Value::Integer(comptime_runtime::Integer::from_i128(256)),
                    ErrorKind::MachineOverflow,
                    true,
                ),
                (
                    false,
                    Primitive::Integer,
                    Primitive::Usize,
                    Value::Integer(comptime_runtime::Integer::from_i128(1)),
                    ErrorKind::Type,
                    true,
                ),
                (
                    true,
                    Primitive::Isize,
                    Primitive::U128,
                    Value::SignedInteger(-1),
                    ErrorKind::Type,
                    true,
                ),
            ] {
                let mut env = Environment::from([(INPUT, value), (TRACE, Value::Bool(false))]);
                assert_eq!(
                    execute(
                        &mut vm,
                        &body(representation, input, output),
                        &mut env,
                        compiled
                    )
                    .unwrap_err()
                    .kind,
                    error
                );
                assert_eq!(
                    vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                    Value::Bool(observed)
                );
            }
            let query = body(false, Primitive::Integer, Primitive::U8);
            let mut env = Environment::from([
                (
                    INPUT,
                    Value::Integer(comptime_runtime::Integer::from_i128(255)),
                ),
                (TRACE, Value::Bool(false)),
            ]);
            assert!(matches!(
                execute(&mut vm, &query, &mut env, compiled).unwrap(),
                Flow::Value(Value::UnsignedInteger(255))
            ));
            env.insert(TRACE, Value::Bool(false));
            vm.comptime_only = false;
            assert_eq!(
                execute(&mut vm, &query, &mut env, compiled)
                    .unwrap_err()
                    .kind,
                ErrorKind::PhaseMismatch
            );
            assert_eq!(
                vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                Value::Bool(false)
            );
        }
    }
}
