//! Both execution carriers use the same Type construction contracts.
use super::*;
use comptime_compiler::type_constructors::Kind;

impl Interpreter<'_> {
    pub(super) fn prepare_type_constructor(
        &mut self,
        kind: &Kind,
        ty: &Term,
        inputs: &[&Term],
        source: SourceRange,
    ) -> Result<(), Error> {
        let expected = if inputs
            .iter()
            .any(|ty| **ty == Term::Primitive(Primitive::Never))
        {
            Primitive::Never
        } else {
            Primitive::Type
        };
        if *ty != Term::Primitive(expected)
            || inputs
                .iter()
                .any(|ty| !matches!(**ty, Term::Primitive(Primitive::Type | Primitive::Never)))
        {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        self.integer_operation(Some(source), |budget| {
            budget.consume(inputs.len().saturating_add(1))
        })?;
        let valid = match kind {
            Kind::Function { parameters, .. } => parameters.len() == inputs.len(),
            Kind::Result => inputs.len() == 2,
            Kind::Optional | Kind::Pointer => inputs.len() == 1,
            Kind::Closure => !inputs.is_empty(),
        };
        if !valid {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(())
    }

    pub(super) fn check_type_constructor_input(
        &self,
        kind: &Kind,
        index: usize,
        count: usize,
        value: Value,
        source: SourceRange,
    ) -> Result<Term, Error> {
        let ty = self.type_description_term(value, source)?;
        let valid = match kind {
            Kind::Result if index == 0 => matches!(ty.callable_signature(), Term::Function { .. }),
            Kind::Closure if count > 1 && index + 1 == count => matches!(
                ty,
                Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_)
            ),
            _ => true,
        };
        if !valid {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(ty)
    }

    pub(super) fn complete_type_constructor(
        &self,
        kind: &Kind,
        mut inputs: Vec<Term>,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let failure = || self.failure(ErrorKind::Type, Some(source));
        let ty = match kind {
            Kind::Function {
                comptime,
                parameters,
            } => {
                if parameters.len() != inputs.len() {
                    return Err(failure());
                }
                let parameters = inputs
                    .into_iter()
                    .zip(parameters)
                    .map(|(ty, staged)| nir::Parameter {
                        sort: if !staged {
                            nir::Sort::Runtime
                        } else if ty == Term::Primitive(Primitive::Type) {
                            nir::Sort::Type
                        } else {
                            nir::Sort::ComptimeValue
                        },
                        ty,
                    })
                    .collect();
                Term::Function {
                    parameters,
                    result: Box::new(Term::Primitive(Primitive::Void)),
                }
                .with_comptime_phase(*comptime)
            }
            Kind::Result => {
                let output = inputs.pop().ok_or_else(failure)?;
                let input = inputs.pop().ok_or_else(failure)?;
                input.with_callable_result(output).ok_or_else(failure)?
            }
            Kind::Optional => Term::Optional(Box::new(inputs.pop().ok_or_else(failure)?)),
            Kind::Pointer => Term::Pointer(Box::new(inputs.pop().ok_or_else(failure)?)),
            Kind::Closure => {
                let signature = inputs.pop().ok_or_else(failure)?;
                if inputs.is_empty() {
                    signature
                } else {
                    Term::Closure {
                        environment: inputs,
                        signature: Box::new(signature),
                    }
                }
            }
        };
        Ok(Value::Type(ty))
    }

    pub(super) fn eval_type_constructor(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let (kind, fields): (_, Vec<&Expr>) = match &expr.kind {
            K::FunctionType {
                comptime,
                comptime_parameters,
                parameters,
            } => (
                Kind::Function {
                    comptime: *comptime,
                    parameters: comptime_parameters.clone(),
                },
                parameters.iter().collect(),
            ),
            K::FunctionResultType { signature, result } => (Kind::Result, vec![signature, result]),
            K::OptionalType(inner) => (Kind::Optional, vec![inner]),
            K::PointerType(inner) => (Kind::Pointer, vec![inner]),
            K::ClosureType {
                environment,
                signature,
            } => (
                Kind::Closure,
                environment
                    .iter()
                    .chain(std::iter::once(signature.as_ref()))
                    .collect(),
            ),
            _ => unreachable!(),
        };
        let domains: Vec<_> = fields.iter().map(|field| &field.ty).collect();
        self.prepare_type_constructor(&kind, &expr.ty, &domains, expr.source)?;
        let mut inputs = Vec::with_capacity(fields.len());
        for (index, field) in fields.iter().enumerate() {
            let value = value!(self.eval(field, environment));
            inputs.push(self.check_type_constructor_input(
                &kind,
                index,
                fields.len(),
                value,
                field.source,
            )?);
        }
        self.complete_type_constructor(&kind, inputs, expr.source)
            .map(Flow::Value)
    }
}
