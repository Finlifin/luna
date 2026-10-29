//! One checked primitive implementation for tree interpretation and engine code.
//! Signature checks precede operand work. Integer operands are checked in source
//! order; comparisons preserve their existing evaluate-both-then-check rule.
use super::*;
use comptime_compiler::scalar::{Scalar, ScalarKind};
use comptime_runtime::machine::{Error as MError, MachineInteger};
use nir::program::IntegerOp;

#[derive(Clone, Copy)]
pub(super) enum Plan {
    Integer(Option<IntegerOp>),
    Machine {
        signed: bool,
        width: u32,
        op: Option<IntegerOp>,
    },
    Compare(IntegerOp),
}
fn machine(value: &Value, signed: bool, width: u32) -> Result<MachineInteger, MError> {
    match value {
        Value::SignedInteger(value) if signed => MachineInteger::from_signed(*value, width as u16),
        Value::UnsignedInteger(value) if !signed => {
            MachineInteger::from_unsigned(*value, width as u16)
        }
        _ => Err(MError::RepresentationMismatch),
    }
}
impl Interpreter<'_> {
    pub(super) fn prepare_scalar(
        &self,
        scalar: Scalar<'_>,
        source: SourceRange,
    ) -> Result<Plan, Error> {
        let invalid = || self.failure(ErrorKind::Type, Some(source));
        let never = Term::Primitive(Primitive::Never);
        match scalar.kind {
            ScalarKind::Integer(op) => {
                if scalar.right.is_some() != op.is_some() {
                    return Err(invalid());
                }
                let operand = if *scalar.left == never {
                    scalar.right.unwrap_or(scalar.left)
                } else {
                    scalar.left
                };
                let machine_type = [operand, scalar.result].into_iter().find_map(|ty| {
                    if let Term::Primitive(primitive) = ty {
                        self.integer_layout(*primitive)
                            .map(|layout| (*primitive, layout))
                    } else {
                        None
                    }
                });
                if let Some((primitive, (signed, width))) = machine_type {
                    if op.is_some_and(|op| {
                        !matches!(
                            op,
                            IntegerOp::Add
                                | IntegerOp::Subtract
                                | IntegerOp::Multiply
                                | IntegerOp::DivideTrunc
                                | IntegerOp::RemainderTrunc
                        )
                    }) {
                        return Err(invalid());
                    }
                    let target = Term::Primitive(primitive);
                    let expected =
                        if *scalar.left == never || scalar.right.is_some_and(|ty| *ty == never) {
                            &never
                        } else {
                            &target
                        };
                    if scalar.result != expected
                        || (*scalar.left != target && *scalar.left != never)
                        || scalar.right.is_some_and(|ty| *ty != target && *ty != never)
                    {
                        return Err(invalid());
                    }
                    Ok(Plan::Machine { signed, width, op })
                } else {
                    let expected =
                        if *scalar.left == never || scalar.right.is_some_and(|ty| *ty == never) {
                            Primitive::Never
                        } else if op.is_some_and(|op| op.is_comparison()) {
                            Primitive::Bool
                        } else {
                            Primitive::Integer
                        };
                    let supported = |ty: &Term| {
                        matches!(ty, Term::Primitive(Primitive::Integer | Primitive::Never))
                    };
                    if *scalar.result != Term::Primitive(expected)
                        || !supported(scalar.left)
                        || scalar.right.is_some_and(|ty| !supported(ty))
                    {
                        return Err(invalid());
                    }
                    Ok(Plan::Integer(op))
                }
            }
            ScalarKind::Compare(op) => {
                let right = scalar.right.ok_or_else(invalid)?;
                let expected = if *scalar.left == never || *right == never {
                    never.clone()
                } else {
                    Term::Primitive(Primitive::Bool)
                };
                let supported = |ty: &Term| matches!(ty,Term::Primitive(p) if matches!(p,Primitive::Bool|Primitive::Char|Primitive::Str|Primitive::Symbol|Primitive::Float|Primitive::F16|Primitive::Bf16|Primitive::F32|Primitive::F64|Primitive::Never) || self.integer_layout(*p).is_some());
                if *scalar.result != expected
                    || !supported(scalar.left)
                    || !supported(right)
                    || (op != IntegerOp::Equal
                        && [scalar.left, right].iter().any(|ty| {
                            matches!(
                                ty,
                                Term::Primitive(
                                    Primitive::Bool | Primitive::Str | Primitive::Symbol
                                )
                            )
                        }))
                    || (*scalar.left != never && *right != never && scalar.left != right)
                {
                    return Err(invalid());
                }
                Ok(Plan::Compare(op))
            }
        }
    }

    pub(super) fn scalar_operand(
        &self,
        plan: Plan,
        value: &Value,
        source: SourceRange,
    ) -> Result<(), Error> {
        if matches!(plan, Plan::Compare(_)) {
            return Ok(());
        }
        if matches!(value, Value::AbstractValue { .. }) {
            return Err(self.failure(ErrorKind::AbstractValueUnresolved, Some(source)));
        }
        let valid = match plan {
            Plan::Integer(_) => matches!(value, Value::Integer(_)),
            Plan::Machine { signed, width, .. } => machine(value, signed, width).is_ok(),
            Plan::Compare(_) => unreachable!(),
        };
        if valid {
            Ok(())
        } else {
            Err(self.failure(ErrorKind::Type, Some(source)))
        }
    }

    pub(super) fn scalar_values(
        &mut self,
        scalar: Scalar<'_>,
        plan: Plan,
        a: Value,
        b: Option<Value>,
        source: SourceRange,
    ) -> Result<Value, Error> {
        match plan {
            Plan::Integer(op) => {
                let Value::Integer(left) = a else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                if let Some(op) = op {
                    let Some(Value::Integer(right)) = b else {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    };
                    let result = self.integer_operation(Some(source), |budget| {
                        op.evaluate(&left, &right, budget)
                    })?;
                    match result {
                        Term::Integer(value) => Ok(Value::Integer(value)),
                        Term::Bool(value) => Ok(Value::Bool(value)),
                        _ => unreachable!(),
                    }
                } else {
                    Ok(Value::Integer(
                        self.integer_operation(Some(source), |budget| left.negate(budget))?,
                    ))
                }
            }
            Plan::Machine { signed, width, op } => {
                let left = machine(&a, signed, width)
                    .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
                let right = b
                    .as_ref()
                    .map(|value| machine(value, signed, width))
                    .transpose()
                    .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
                if op.is_some() != right.is_some() {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let value = self
                    .integer_operation(Some(source), |budget| {
                        Ok(match op {
                            Some(IntegerOp::Add) => left.add(right.unwrap(), budget),
                            Some(IntegerOp::Subtract) => left.subtract(right.unwrap(), budget),
                            Some(IntegerOp::Multiply) => left.multiply(right.unwrap(), budget),
                            Some(IntegerOp::DivideTrunc) => {
                                left.div_rem_trunc(right.unwrap(), budget).map(|(q, _)| q)
                            }
                            Some(IntegerOp::RemainderTrunc) => {
                                left.div_rem_trunc(right.unwrap(), budget).map(|(_, r)| r)
                            }
                            None => left.negate(budget),
                            _ => unreachable!(),
                        })
                    })?
                    .map_err(|error| {
                        self.failure(
                            match error {
                                MError::Overflow => ErrorKind::MachineOverflow,
                                MError::DivisionByZero => ErrorKind::DivisionByZero,
                                MError::Resource(comptime_runtime::Error::BudgetExhausted) => {
                                    ErrorKind::StepsExhausted
                                }
                                _ => ErrorKind::Type,
                            },
                            Some(source),
                        )
                    })?;
                Ok(if signed {
                    Value::SignedInteger(value.signed_value().unwrap())
                } else {
                    Value::UnsignedInteger(value.unsigned_value().unwrap())
                })
            }
            Plan::Compare(op) => self.compare_scalar_values(
                scalar,
                op,
                a,
                b.ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?,
                source,
            ),
        }
    }

    pub(super) fn eval_scalar_operation(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let (scalar, left, right) = Scalar::expression(expr)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
        let plan = self.prepare_scalar(scalar, expr.source)?;
        let a = match self.eval(left, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        self.scalar_operand(plan, &a, expr.source)?;
        let b = if let Some(right) = right {
            let value = match self.eval(right, environment)? {
                Flow::Value(value) => value,
                flow => return Ok(flow),
            };
            self.scalar_operand(plan, &value, expr.source)?;
            Some(value)
        } else {
            None
        };
        self.scalar_values(scalar, plan, a, b, expr.source)
            .map(Flow::Value)
    }
    fn compare_scalar_values(
        &mut self,
        scalar: Scalar<'_>,
        op: IntegerOp,
        a: Value,
        b: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        use std::cmp::Ordering::*;
        if !self.has_type(&a, scalar.left) || !self.has_type(&b, scalar.right.unwrap()) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        if matches!(a, Value::AbstractValue { .. }) || matches!(b, Value::AbstractValue { .. }) {
            return Err(self.failure(ErrorKind::AbstractValueUnresolved, Some(source)));
        }
        let compare = |ordering| match op {
            IntegerOp::Equal => matches!(ordering, Some(Equal)),
            IntegerOp::Less => matches!(ordering, Some(Less)),
            IntegerOp::LessEqual => matches!(ordering, Some(Less | Equal)),
            IntegerOp::Greater => matches!(ordering, Some(Greater)),
            IntegerOp::GreaterEqual => matches!(ordering, Some(Greater | Equal)),
            _ => unreachable!(),
        };
        let equal = match (&a, &b) {
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Char(a), Value::Char(b)) => compare(a.partial_cmp(b)),
            (Value::SignedInteger(a), Value::SignedInteger(b)) => compare(a.partial_cmp(b)),
            (Value::UnsignedInteger(a), Value::UnsignedInteger(b)) => compare(a.partial_cmp(b)),
            (Value::Float(a), Value::Float(b)) => {
                compare(Some(self.integer_operation(Some(source), |budget| {
                    a.compare(b, budget)
                })?))
            }
            (Value::Float16(a), Value::Float16(b)) => {
                compare(comptime_runtime::float16::Float16Format::Binary16.compare(*a, *b))
            }
            (Value::BFloat16(a), Value::BFloat16(b)) => {
                compare(comptime_runtime::float16::Float16Format::BFloat16.compare(*a, *b))
            }
            (Value::Float32(a), Value::Float32(b)) => {
                compare(f32::from_bits(*a).partial_cmp(&f32::from_bits(*b)))
            }
            (Value::Float64(a), Value::Float64(b)) => {
                compare(f64::from_bits(*a).partial_cmp(&f64::from_bits(*b)))
            }
            (Value::String(a), Value::String(b)) | (Value::Symbol(a), Value::Symbol(b)) => {
                self.integer_operation(Some(source), |budget| {
                    budget.consume(a.len().min(b.len()).saturating_add(1))
                })?;
                a == b
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        };
        Ok(Value::Bool(equal))
    }
}
