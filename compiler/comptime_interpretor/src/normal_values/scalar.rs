//! Scalar and callable input reconstruction does not recurse through containers.
use super::*;

impl Interpreter<'_> {
    #[inline(never)]
    pub(super) fn decode_scalar_normal_value(
        &mut self,
        term: &Term,
        ty: &Term,
    ) -> Result<Value, Error> {
        let value = match (term, ty) {
            (Term::Bool(value), _) => Value::Bool(*value),
            (Term::Char(value), _) => Value::Char(*value),
            (Term::String(value), _) => Value::String(value.clone()),
            (Term::Symbol(value), _) => Value::Symbol(value.clone()),
            (Term::ClosureSnapshot { factory, arguments }, ty) if ty.is_comptime_callable() => {
                let function = self
                    .functions
                    .get(factory)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                if function.phase != nir::program::FunctionPhase::Comptime {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                for argument in arguments {
                    let work = self
                        .type_description_work(argument)
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    self.integer_operation(None, |budget| budget.consume(work))?;
                }
                self.admit_function_inputs(
                    &function.type_parameters,
                    arguments,
                    function.body.source,
                )?;
                let binders = self.input_binders(&function.type_parameters);
                self.charge_copy_work(Some(function.body.source), |fuel| {
                    nir::copy_budget::charge_type_substitution(
                        &function.signature,
                        &binders,
                        arguments,
                        fuel,
                    )
                })?;
                let substitution = self.function_substitution(
                    &function.type_parameters,
                    arguments,
                    function.body.source,
                )?;
                let signature = substitution
                    .instantiate(&function.signature)
                    .map_err(|_| self.failure(ErrorKind::Type, None))?;
                if !matches!(signature.callable_signature(), Term::Function { parameters, result }
                        if parameters.is_empty() && result.as_ref() == ty)
                {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                let expression = Expr {
                    source: function.body.source,
                    ty: signature.with_comptime_phase(true),
                    kind: K::FunctionInstance {
                        declaration: *factory,
                        arguments: arguments.clone(),
                    },
                };
                let Flow::Value(callable) = self.eval(&expression, &mut Environment::new())? else {
                    return Err(
                        self.failure(ErrorKind::InvalidControlFlow, Some(expression.source))
                    );
                };
                self.invoke_value(callable, vec![])?
            }
            (
                Term::FunctionValue {
                    declaration,
                    arguments,
                },
                ty @ Term::NormalFormFunction(..),
            ) => {
                // A frozen head carries identity, not an executable body or
                // applied generic arguments. Its telescope belongs to metadata.
                let value = Value::Function(*declaration);
                if !arguments.is_empty() || !self.has_type(&value, ty) {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                value
            }
            (
                Term::FunctionValue {
                    declaration,
                    arguments,
                },
                ty,
            ) if matches!(
                ty,
                Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_)
            ) =>
            {
                let source =
                    if let Some(identity) = self.runtime_function_identities.get(declaration) {
                        if ty.is_comptime_callable() || identity.signature.is_comptime_callable() {
                            return Err(self.failure(ErrorKind::Type, None));
                        }
                        SourceRange { start: 0, end: 0 }
                    } else {
                        let function = self
                            .functions
                            .get(declaration)
                            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                        if function.phase == nir::program::FunctionPhase::NormalForm
                            || (function.phase == nir::program::FunctionPhase::Comptime)
                                != ty.is_comptime_callable()
                            || function.is_unsafe != ty.is_unsafe_callable()
                        {
                            return Err(self.failure(ErrorKind::Type, None));
                        }
                        function.body.source
                    };
                for argument in arguments {
                    let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let checked = nir::type_computations::charge_term(argument, &mut fuel);
                    self.budget.steps -= (before - fuel) as u64;
                    checked.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
                    argument
                        .check_scope(&[])
                        .map_err(|_| self.failure(ErrorKind::Type, None))?;
                    if argument.sort() == nir::Sort::Type {
                        let work = self
                            .type_description_work(argument)
                            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                        self.integer_operation(None, |budget| budget.consume(work))?;
                    }
                }
                let expression = Expr {
                    source,
                    ty: ty.clone(),
                    kind: K::FunctionInstance {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                    },
                };
                match self.eval(&expression, &mut Environment::new())? {
                    Flow::Value(value) => value,
                    _ => {
                        return Err(
                            self.failure(ErrorKind::InvalidControlFlow, Some(expression.source))
                        );
                    }
                }
            }
            (Term::Integer(value), Term::Primitive(Primitive::Integer)) => {
                Value::Integer(value.clone())
            }
            (
                Term::Integer(value),
                Term::Primitive(
                    Primitive::U8
                    | Primitive::U16
                    | Primitive::U32
                    | Primitive::U64
                    | Primitive::U128
                    | Primitive::Usize,
                ),
            ) => Value::UnsignedInteger(
                value
                    .to_u128()
                    .map_err(|_| self.failure(ErrorKind::Type, None))?,
            ),
            (Term::Integer(value), _) => Value::SignedInteger(
                value
                    .to_i128()
                    .map_err(|_| self.failure(ErrorKind::Type, None))?,
            ),
            (Term::BitVec(value), _) => Value::BitVec(value.clone()),
            (Term::Float(value), Term::Primitive(Primitive::Float)) => Value::Float(value.clone()),
            (Term::Float16(bits), Term::Primitive(Primitive::F16)) => Value::Float16(*bits),
            (Term::BFloat16(bits), Term::Primitive(Primitive::Bf16)) => Value::BFloat16(*bits),
            (Term::Float32(bits), Term::Primitive(Primitive::F32)) => Value::Float32(*bits),
            (Term::Float64(bits), Term::Primitive(Primitive::F64)) => Value::Float64(*bits),
            (Term::Unit, _) => Value::Unit,
            _ => return Err(self.failure(ErrorKind::Type, None)),
        };
        Ok(value)
    }
}
