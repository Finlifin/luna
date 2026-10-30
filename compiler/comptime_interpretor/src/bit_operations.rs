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
        let (operation, operands): (_, Vec<&Expr>) = match &expr.kind {
            K::BitSlice { value, start, end } => (
                B::Slice {
                    start: *start,
                    end: *end,
                },
                vec![value],
            ),
            K::BitIndex(value, index) => (B::Index, vec![value, index]),
            K::BitWindow { value, start, end } => (B::Window, vec![value, start, end]),
            K::BitConcat(left, right) => (B::Concat, vec![left, right]),
            K::BitEqual(left, right) => (B::Equal, vec![left, right]),
            K::BitConvert(value) => (B::Convert, vec![value]),
            K::TakeBits(value) => (B::Take, vec![value]),
            _ => unreachable!("bit operation dispatcher"),
        };
        let mut values = Vec::new();
        for (index, operand) in operands.iter().enumerate() {
            let value = match self.eval(operand, environment)? {
                Flow::Value(value) => value,
                control => return Ok(control),
            };
            values.push(self.prepare_bit_operand(
                value,
                operation.requires_bits(index),
                expr.source,
            )?);
        }
        let types = operands
            .iter()
            .map(|operand| &operand.ty)
            .collect::<Vec<_>>();
        self.bit_values(operation, &types, &expr.ty, values, expr.source)
            .map(Flow::Value)
    }

    pub(super) fn bit_values(
        &mut self,
        operation: B,
        types: &[&Term],
        output: &Term,
        values: Vec<Value>,
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
        let mut values = values.into_iter();
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
                return Ok(match *output {
                    Term::Primitive(Primitive::BitVec(width)) => Value::BitVec(
                        self.bit_operation(source, |budget| bits.widen(width, budget))?,
                    ),
                    Term::Primitive(primitive) => {
                        let Some(width) = primitive.unrestricted_scalar_width_in(self.target)
                        else {
                            return Err(self.failure(ErrorKind::Type, Some(source)));
                        };
                        let widening = self
                            .integer_layout(primitive)
                            .is_some_and(|(signed, _)| !signed);
                        if (width as usize) < bits.width()
                            || (!widening && width as usize != bits.width())
                        {
                            return Err(self.failure(ErrorKind::Type, Some(source)));
                        }
                        let value = self.bit_operation(source, |budget| {
                            budget.consume(bits.words().len().saturating_add(1))?;
                            Ok(bits
                                .words()
                                .iter()
                                .rev()
                                .fold(0u128, |acc, word| (acc << 32) | u128::from(*word)))
                        })?;
                        match primitive {
                            Primitive::F16 => Value::Float16(value as u16),
                            Primitive::Bf16 => Value::BFloat16(value as u16),
                            Primitive::F32 => Value::Float32(value as u32),
                            Primitive::F64 => Value::Float64(value as u64),
                            _ if self
                                .integer_layout(primitive)
                                .is_some_and(|(signed, _)| signed) =>
                            {
                                let shift = 128 - width;
                                Value::SignedInteger(((value << shift) as i128) >> shift)
                            }
                            _ => Value::UnsignedInteger(value),
                        }
                    }
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
                if !self.has_type(&index, types[1])
                    || *types[0] != Term::Primitive(Primitive::BitVec(bits.width()))
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
                if !self.has_type(&lower, types[1])
                    || !self.has_type(&upper, types[2])
                    || *types[0] != Term::Primitive(Primitive::BitVec(bits.width()))
                {
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
