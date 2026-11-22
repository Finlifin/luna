//! Shared BitVec value semantics for tree interpretation and bytecode.
use super::*;
use comptime_compiler::bits::Operation as B;

impl Interpreter<'_> {
    pub(super) fn prepare_bit_operand(
        &self,
        value: Value,
        requires_bits: bool,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let value = Self::concrete_value(value, source)?;
        if requires_bits && !matches!(value, Value::BitVec(_)) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(value)
    }

    pub(super) fn eval_bit_values(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        match &expr.kind {
            K::BitSlice { value, start, end } => self.eval_bit_operands(
                B::Slice {
                    start: *start,
                    end: *end,
                },
                [value],
                &expr.ty,
                expr.source,
                environment,
            ),
            K::BitIndex(value, index) => {
                self.eval_bit_operands(B::Index, [value, index], &expr.ty, expr.source, environment)
            }
            K::BitWindow { value, start, end } => self.eval_bit_operands(
                B::Window,
                [value, start, end],
                &expr.ty,
                expr.source,
                environment,
            ),
            K::BitConcat(left, right) => {
                self.eval_bit_operands(B::Concat, [left, right], &expr.ty, expr.source, environment)
            }
            K::BitEqual(left, right) => {
                self.eval_bit_operands(B::Equal, [left, right], &expr.ty, expr.source, environment)
            }
            K::BitConvert(value) => {
                self.eval_bit_operands(B::Convert, [value], &expr.ty, expr.source, environment)
            }
            K::TakeBits(value) => {
                self.eval_bit_operands(B::Take, [value], &expr.ty, expr.source, environment)
            }
            _ => unreachable!("bit operation dispatcher"),
        }
    }

    fn eval_bit_operands<const N: usize>(
        &mut self,
        operation: B,
        operands: [&Expr; N],
        output: &Term,
        source: SourceRange,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let mut values = std::array::from_fn::<_, N, _>(|_| Value::Unit);
        for (index, operand) in operands.iter().enumerate() {
            let value = match self.eval(operand, environment)? {
                Flow::Value(value) => value,
                control => return Ok(control),
            };
            values[index] =
                self.prepare_bit_operand(value, operation.requires_bits(index), source)?;
        }
        self.bit_values(
            operation,
            &operands.map(|operand| &operand.ty),
            output,
            &mut values,
            source,
        )
        .map(Flow::Value)
    }

    pub(super) fn bit_registers(
        &mut self,
        operation: B,
        inputs: &[(usize, Term)],
        output: &Term,
        slots: &mut [Option<Value>],
        source: SourceRange,
    ) -> Result<Value, Error> {
        match inputs {
            [a] => self.bit_register_operands(operation, [a], output, slots, source),
            [a, b] => self.bit_register_operands(operation, [a, b], output, slots, source),
            [a, b, c] => self.bit_register_operands(operation, [a, b, c], output, slots, source),
            _ => Err(self.failure(ErrorKind::InvalidCallPlan, Some(source))),
        }
    }

    fn bit_register_operands<const N: usize>(
        &mut self,
        operation: B,
        inputs: [&(usize, Term); N],
        output: &Term,
        slots: &mut [Option<Value>],
        source: SourceRange,
    ) -> Result<Value, Error> {
        let mut values = std::array::from_fn::<_, N, _>(|_| Value::Unit);
        for ((slot, _), value) in inputs.iter().zip(&mut values) {
            *value = slots[*slot]
                .take()
                .ok_or_else(|| self.failure(ErrorKind::MissingBinding, Some(source)))?;
        }
        self.bit_values(
            operation,
            &inputs.map(|(_, ty)| ty),
            output,
            &mut values,
            source,
        )
    }

    pub(super) fn bit_values(
        &mut self,
        operation: B,
        types: &[&Term],
        output: &Term,
        values: &mut [Value],
        source: SourceRange,
    ) -> Result<Value, Error> {
        let count = match operation {
            B::Slice { .. } | B::Convert | B::Take => 1,
            B::Index | B::Concat | B::Equal | B::And | B::Or => 2,
            B::Window => 3,
        };
        if values.len() != count || types.len() != count {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
        }
        // Values are moved out of the caller's fixed-size operand buffer. This
        // keeps one semantic kernel for tree execution and register code.
        let mut values = values
            .iter_mut()
            .map(|value| std::mem::replace(value, Value::Unit));
        if matches!(operation, B::Take) {
            return self.take_bits_value(values.next().unwrap(), types[0], output, source);
        }
        let Some(Value::BitVec(bits)) = values.next() else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        let position = |value: Value| {
            match value {
                Value::Integer(n) => n.to_u128().ok(),
                Value::UnsignedInteger(n) => Some(n),
                Value::SignedInteger(n) => u128::try_from(n).ok(),
                _ => None,
            }
            .and_then(|n| usize::try_from(n).ok())
        };
        let result = match operation {
            B::Take => unreachable!("representation input handled before requiring bits"),
            B::And | B::Or => {
                let Some(Value::BitVec(right)) = values.next() else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                if *output != Term::Primitive(Primitive::BitVec(bits.width())) {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                self.bit_operation(source, |budget| match operation {
                    B::And => bits.and(&right, budget),
                    B::Or => bits.or(&right, budget),
                    _ => unreachable!(),
                })?
            }
            B::Equal => {
                let Some(Value::BitVec(right)) = values.next() else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                if bits.width() != right.width() || *output != Term::Primitive(Primitive::Bool) {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                return self.bit_operation(source, |budget| {
                    budget.consume(bits.words().len().saturating_add(1))?;
                    Ok(Value::Bool(bits == right))
                });
            }
            B::Convert => {
                return Ok(match output {
                    Term::Primitive(Primitive::BitVec(width)) => Value::BitVec(
                        self.bit_operation(source, |budget| bits.widen(*width, budget))?,
                    ),
                    Term::Primitive(primitive) => {
                        self.decode_unrestricted_scalar(&bits, *primitive, true, source)?
                    }
                    Term::Nominal { .. } => self.decode_bit_packed_value(&bits, output, source)?,
                    _ => return Err(self.failure(ErrorKind::Type, Some(source))),
                });
            }
            B::Slice { start, end } => {
                self.bit_operation(source, |budget| bits.slice(start, end, budget))?
            }
            B::Concat => {
                let Some(Value::BitVec(right)) = values.next() else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                self.bit_operation(source, |budget| bits.concat(&right, budget))?
            }
            B::Index => {
                let index = values.next().unwrap();
                self.check_retained_value(&index, types[1], Some(source))?;
                if *types[0] != Term::Primitive(Primitive::BitVec(bits.width()))
                    || *output != Term::Primitive(Primitive::BitVec(1))
                {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                if !matches!(
                    index,
                    Value::Integer(_) | Value::UnsignedInteger(_) | Value::SignedInteger(_)
                ) {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let index = position(index)
                    .filter(|index| *index < bits.width())
                    .ok_or_else(|| self.failure(ErrorKind::IndexOutOfBounds, Some(source)))?;
                self.bit_operation(source, |budget| bits.slice(index, index + 1, budget))?
            }
            B::Window => {
                let lower = values.next().unwrap();
                let upper = values.next().unwrap();
                self.check_retained_value(&lower, types[1], Some(source))?;
                self.check_retained_value(&upper, types[2], Some(source))?;
                if *types[0] != Term::Primitive(Primitive::BitVec(bits.width())) {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let (Some(start), Some(end)) = (position(lower), position(upper)) else {
                    return Err(self.failure(ErrorKind::IndexOutOfBounds, Some(source)));
                };
                if start > end || end > bits.width() {
                    return Err(self.failure(ErrorKind::IndexOutOfBounds, Some(source)));
                }
                if *output != Term::Primitive(Primitive::BitVec(end - start)) {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                self.bit_operation(source, |budget| bits.slice(start, end, budget))?
            }
        };
        if *output != Term::Primitive(Primitive::BitVec(result.width())) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(Value::BitVec(result))
    }
}

#[cfg(test)]
mod tests;
