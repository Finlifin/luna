//! IEEE comparison coverage via a discrete ordered-value quotient.
//! Signed zeros share a key; all NaNs share one unordered key. No user code runs.
use crate::interval_coverage::{Edge, Interval};
use comptime_runtime::Integer;
use nir::{
    Primitive, Term,
    program::{Expr, ExprKind as K, IntegerOp, Literal, Pattern},
};

#[derive(Clone, Copy)]
pub(crate) struct Domain {
    primitive: Primitive,
    sign: u64,
    mask: u64,
    infinity: u64,
}
impl Domain {
    pub(crate) fn new(primitive: Primitive) -> Option<Self> {
        let (sign, mask, infinity) = match primitive {
            Primitive::F32 => (1 << 31, u32::MAX as u64, 0x7f800000),
            Primitive::F64 => (1 << 63, u64::MAX, 0x7ff0000000000000),
            _ => return None,
        };
        Some(Self {
            primitive,
            sign,
            mask,
            infinity,
        })
    }
    fn nan(self) -> u64 {
        self.sign | self.infinity
    }
    fn key(self, bits: u64) -> u64 {
        let magnitude = bits & (self.sign - 1);
        if magnitude > self.infinity {
            self.nan()
        } else if bits & self.sign != 0 {
            !bits & self.mask
        } else {
            (bits | self.sign) - 1
        }
    }
    pub(crate) fn ranges(self) -> Vec<Interval> {
        vec![Interval {
            start: Edge::Finite(Integer::from_u128(
                (!self.infinity & (self.sign - 1)) as u128,
            )),
            end: Edge::Finite(Integer::from_u128(self.nan() as u128 + 1)),
        }]
    }
    fn bits(self, literal: &Literal) -> Option<u64> {
        match (self.primitive, literal) {
            (Primitive::F32, Literal::Float32(bits)) => Some(*bits as u64),
            (Primitive::F64, Literal::Float64(bits)) => Some(*bits),
            _ => None,
        }
    }
    pub(crate) fn extract(self, pattern: &Pattern, fuel: &mut usize) -> Result<Vec<Interval>, ()> {
        crate::interval_coverage::extract_exact(&self.pattern(pattern, fuel, 0)?, fuel)
    }
    fn pattern(self, pattern: &Pattern, fuel: &mut usize, depth: usize) -> Result<Pattern, ()> {
        self.pattern_subject(pattern, false, fuel, depth)
    }
    fn pattern_subject(
        self,
        pattern: &Pattern,
        boolean_subject: bool,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Pattern, ()> {
        charge(fuel, depth)?;
        Ok(match pattern {
            Pattern::Wild => Pattern::Wild,
            Pattern::Literal(Literal::Bool(value)) if boolean_subject => {
                Pattern::Literal(Literal::Bool(*value))
            }
            Pattern::Literal(literal) if !boolean_subject => {
                let key = self.key(self.bits(literal).ok_or(())?);
                if key == self.nan() {
                    Pattern::Or(vec![])
                } else {
                    Pattern::Literal(Literal::UnsignedInteger(key as u128))
                }
            }
            Pattern::Binding { binding, inner } => Pattern::Binding {
                binding: *binding,
                inner: Box::new(self.pattern_subject(inner, boolean_subject, fuel, depth + 1)?),
            },
            Pattern::Observe { binding, inner } => Pattern::Observe {
                binding: *binding,
                inner: Box::new(self.pattern_subject(inner, boolean_subject, fuel, depth + 1)?),
            },
            Pattern::Or(choices) => Pattern::Or(
                choices
                    .iter()
                    .map(|p| self.pattern_subject(p, boolean_subject, fuel, depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            Pattern::Then { first, value, next } => {
                let result_is_bool = value.ty == Term::Primitive(Primitive::Bool);
                if !result_is_bool && value.ty != Term::Primitive(self.primitive) {
                    return Err(());
                }
                Pattern::Then {
                    first: Box::new(self.pattern_subject(
                        first,
                        boolean_subject,
                        fuel,
                        depth + 1,
                    )?),
                    value: Box::new(self.expr(value, fuel, depth + 1)?),
                    next: Box::new(self.pattern_subject(next, result_is_bool, fuel, depth + 1)?),
                }
            }
            Pattern::Guard { pattern, condition } => Pattern::Guard {
                pattern: Box::new(self.pattern_subject(
                    pattern,
                    boolean_subject,
                    fuel,
                    depth + 1,
                )?),
                condition: Box::new(self.expr(condition, fuel, depth + 1)?),
            },
            Pattern::Remap { pattern, bindings } => {
                *fuel = fuel.checked_sub(bindings.len()).ok_or(())?;
                Pattern::Remap {
                    pattern: Box::new(self.pattern_subject(
                        pattern,
                        boolean_subject,
                        fuel,
                        depth + 1,
                    )?),
                    bindings: bindings.clone(),
                }
            }
            _ => return Err(()),
        })
    }
    fn constant(self, expr: &Expr, fuel: &mut usize, depth: usize) -> Result<Option<u64>, ()> {
        charge(fuel, depth)?;
        Ok(match &expr.kind {
            K::Literal(literal) => self.bits(literal),
            K::FloatNegate(value) => self
                .constant(value, fuel, depth + 1)?
                .map(|bits| bits ^ self.sign),
            _ => None,
        })
    }
    // Calls remain unknown predicates. Their inputs are copied without the
    // IEEE-to-integer projection: interval evaluation never invokes a callee or
    // observes an argument. Bound the accepted copy shapes explicitly instead
    // of recursively cloning arbitrary program graphs before checking fuel.
    fn opaque_argument(self, expr: &Expr, fuel: &mut usize, depth: usize) -> Result<Expr, ()> {
        charge(fuel, depth)?;
        crate::associated::charge_projection_copy(&expr.ty, fuel).map_err(|_| ())?;
        let kind = match &expr.kind {
            K::Local(id) => K::Local(*id),
            K::Function(id) => K::Function(*id),
            K::FunctionInstance {
                declaration,
                arguments,
            } => {
                for argument in arguments {
                    crate::associated::charge_projection_copy(argument, fuel).map_err(|_| ())?;
                }
                K::FunctionInstance {
                    declaration: *declaration,
                    arguments: arguments.clone(),
                }
            }
            K::Literal(literal) => {
                let size = match literal {
                    Literal::Integer(s)
                    | Literal::Float(s)
                    | Literal::String(s)
                    | Literal::Symbol(s) => s.len(),
                    Literal::BitVec { words, .. } => words.len(),
                    _ => 0,
                };
                *fuel = fuel.checked_sub(size).ok_or(())?;
                K::Literal(literal.clone())
            }
            K::Unit => K::Unit,
            K::Tuple(fields) => K::Tuple(self.opaque_arguments(fields, fuel, depth + 1)?),
            K::AnonymousRecord { fields } => K::AnonymousRecord {
                fields: self.opaque_arguments(fields, fuel, depth + 1)?,
            },
            K::Record {
                declaration,
                fields,
            } => K::Record {
                declaration: *declaration,
                fields: self.opaque_arguments(fields, fuel, depth + 1)?,
            },
            K::Variant {
                declaration,
                index,
                fields,
            } => K::Variant {
                declaration: *declaration,
                index: *index,
                fields: self.opaque_arguments(fields, fuel, depth + 1)?,
            },
            K::NewtypeConvert(value) => {
                K::NewtypeConvert(Box::new(self.opaque_argument(value, fuel, depth + 1)?))
            }
            // Existing scalar expressions retain their established projection.
            // Nothing underneath the opaque Call participates in the proof.
            _ => return self.expr(expr, fuel, depth + 1),
        };
        Ok(Expr {
            ty: expr.ty.clone(),
            source: expr.source,
            kind,
        })
    }
    fn opaque_arguments(
        self,
        values: &[Expr],
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Vec<Expr>, ()> {
        if values.len() > *fuel {
            return Err(());
        }
        values
            .iter()
            .map(|value| self.opaque_argument(value, fuel, depth))
            .collect()
    }
    fn expr(self, expr: &Expr, fuel: &mut usize, depth: usize) -> Result<Expr, ()> {
        charge(fuel, depth)?;
        let integer = Term::Primitive(Primitive::Integer);
        let boolean = Term::Primitive(Primitive::Bool);
        if expr.ty == Term::Primitive(self.primitive) {
            if let Some(bits) = self.constant(expr, fuel, depth + 1)? {
                return Ok(Expr {
                    source: expr.source,
                    ty: integer,
                    kind: K::Literal(Literal::UnsignedInteger(self.key(bits) as u128)),
                });
            }
        }
        let kind = match &expr.kind {
            K::Local(id) => K::Local(*id),
            K::Literal(Literal::Bool(value)) => K::Literal(Literal::Bool(*value)),
            K::Not(value) => K::Not(Box::new(self.expr(value, fuel, depth + 1)?)),
            K::If {
                condition,
                then_branch,
                else_branch: Some(else_branch),
            } if expr.ty == boolean => K::If {
                condition: Box::new(self.expr(condition, fuel, depth + 1)?),
                then_branch: Box::new(self.expr(then_branch, fuel, depth + 1)?),
                else_branch: Some(Box::new(self.expr(else_branch, fuel, depth + 1)?)),
            },
            K::And(a, b) | K::Or(a, b) => {
                let a = Box::new(self.expr(a, fuel, depth + 1)?);
                let b = Box::new(self.expr(b, fuel, depth + 1)?);
                if matches!(expr.kind, K::And(..)) {
                    K::And(a, b)
                } else {
                    K::Or(a, b)
                }
            }
            K::ScalarEqual(a, b) if a.ty == boolean && b.ty == boolean => K::ScalarEqual(
                Box::new(self.expr(a, fuel, depth + 1)?),
                Box::new(self.expr(b, fuel, depth + 1)?),
            ),
            K::ScalarEqual(a, b)
            | K::ScalarLess(a, b)
            | K::ScalarLessEqual(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b) => {
                if a.ty != Term::Primitive(self.primitive) || b.ty != a.ty {
                    return Err(());
                }
                let op = match expr.kind {
                    K::ScalarEqual(..) => IntegerOp::Equal,
                    K::ScalarLess(..) => IntegerOp::Less,
                    K::ScalarLessEqual(..) => IntegerOp::LessEqual,
                    K::ScalarGreater(..) => IntegerOp::Greater,
                    _ => IntegerOp::GreaterEqual,
                };
                let compare = |op, left, right| Expr {
                    source: expr.source,
                    ty: boolean.clone(),
                    kind: K::IntegerBinary {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                };
                let nan = || Expr {
                    source: expr.source,
                    ty: integer.clone(),
                    kind: K::Literal(Literal::UnsignedInteger(self.nan() as u128)),
                };
                // Translate each pure operand twice rather than cloning an
                // unbounded tree. Every expansion consumes the same budget.
                let ordered_a = compare(IntegerOp::Less, self.expr(a, fuel, depth + 1)?, nan());
                let ordered_b = compare(IntegerOp::Less, self.expr(b, fuel, depth + 1)?, nan());
                let comparison = compare(
                    op,
                    self.expr(a, fuel, depth + 1)?,
                    self.expr(b, fuel, depth + 1)?,
                );
                let ordered = Expr {
                    source: expr.source,
                    ty: boolean.clone(),
                    kind: K::And(Box::new(ordered_a), Box::new(ordered_b)),
                };
                K::And(Box::new(ordered), Box::new(comparison))
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } if expr.ty == boolean => {
                // Keep calls opaque to interval evaluation. Projecting their
                // scalar inputs does not execute or summarize the callee;
                // only an empty incoming region can skip this node.
                let kind = match &callee.kind {
                    K::Function(id) => K::Function(*id),
                    K::Local(id) => K::Local(*id),
                    K::FunctionInstance {
                        declaration,
                        arguments,
                    } => {
                        *fuel = fuel.checked_sub(arguments.len()).ok_or(())?;
                        for ty in arguments {
                            crate::associated::charge_projection_copy(ty, fuel).map_err(|_| ())?;
                        }
                        K::FunctionInstance {
                            declaration: *declaration,
                            arguments: arguments.clone(),
                        }
                    }
                    _ => return Err(()),
                };
                charge(fuel, depth + 1)?;
                crate::associated::charge_projection_copy(&callee.ty, fuel).map_err(|_| ())?;
                *fuel = fuel
                    .checked_sub(arguments.len().saturating_add(parameter_bindings.len()))
                    .ok_or(())?;
                K::Call {
                    callee: Box::new(Expr {
                        source: callee.source,
                        ty: callee.ty.clone(),
                        kind,
                    }),
                    arguments: arguments
                        .iter()
                        .map(|argument| {
                            Ok(nir::program::CallArgument {
                                parameter: argument.parameter,
                                is_default: argument.is_default,
                                value: self.opaque_argument(&argument.value, fuel, depth + 1)?,
                            })
                        })
                        .collect::<Result<_, ()>>()?,
                    parameter_bindings: parameter_bindings.clone(),
                }
            }
            K::Let { binding, value } => K::Let {
                binding: *binding,
                value: Box::new(self.expr(value, fuel, depth + 1)?),
            },
            K::Assign { place, value } if place.fields.is_empty() => K::Assign {
                place: place.clone(),
                value: Box::new(self.expr(value, fuel, depth + 1)?),
            },
            K::Discard(value) => K::Discard(Box::new(self.expr(value, fuel, depth + 1)?)),
            K::Block { statements, result } => K::Block {
                statements: statements
                    .iter()
                    .map(|e| self.expr(e, fuel, depth + 1))
                    .collect::<Result<_, _>>()?,
                result: result
                    .as_ref()
                    .map(|e| self.expr(e, fuel, depth + 1).map(Box::new))
                    .transpose()?,
            },
            K::Match { value, arms } => K::Match {
                value: Box::new(self.expr(value, fuel, depth + 1)?),
                arms: arms
                    .iter()
                    .map(|arm| {
                        if arm.guard.is_some() {
                            return Err(());
                        }
                        Ok(nir::program::MatchArm {
                            pattern: self.pattern_subject(
                                &arm.pattern,
                                value.ty == boolean,
                                fuel,
                                depth + 1,
                            )?,
                            guard: None,
                            body: self.expr(&arm.body, fuel, depth + 1)?,
                        })
                    })
                    .collect::<Result<_, _>>()?,
            },
            _ => return Err(()),
        };
        let ty = if expr.ty == Term::Primitive(self.primitive) {
            integer
        } else if matches!(expr.ty, Term::Primitive(Primitive::Bool | Primitive::Void)) {
            expr.ty.clone()
        } else {
            return Err(());
        };
        Ok(Expr {
            source: expr.source,
            ty,
            kind,
        })
    }
}
fn charge(fuel: &mut usize, depth: usize) -> Result<(), ()> {
    *fuel = fuel.checked_sub(1).ok_or(())?;
    if depth > 128 { Err(()) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quotient_preserves_adjacent_values_zeros_and_unordered_class() {
        for primitive in [Primitive::F32, Primitive::F64] {
            let domain = Domain::new(primitive).unwrap();
            let sign = domain.sign;
            assert_eq!(domain.key(0), domain.key(sign));
            assert_eq!(domain.key(sign | 1) + 1, domain.key(0));
            assert_eq!(domain.key(0) + 1, domain.key(1));
            assert_eq!(domain.key(domain.infinity) + 1, domain.nan());
            for payload in [1, 2, (sign - 1) ^ domain.infinity] {
                assert_eq!(domain.key(domain.infinity | payload), domain.nan());
                assert_eq!(domain.key(sign | domain.infinity | payload), domain.nan());
            }
        }
    }
    #[test]
    fn opaque_argument_copy_exhaustion_remains_unknown_and_retry_is_independent() {
        use crate::coverage::{Coverage, check};
        use nir::program::{CallArgument, SourceRange};
        let source = SourceRange { start: 0, end: 1 };
        let boolean = Term::Primitive(Primitive::Bool);
        let string = Term::Primitive(Primitive::Str);
        let call = Expr {
            source,
            ty: boolean.clone(),
            kind: K::Call {
                callee: Box::new(Expr {
                    source,
                    ty: Term::Function {
                        parameters: vec![nir::Parameter {
                            sort: nir::Sort::Runtime,
                            ty: string.clone(),
                        }],
                        result: Box::new(boolean.clone()),
                    },
                    kind: K::Function(nir::DeclarationId {
                        package: 0,
                        local: 0,
                    }),
                }),
                arguments: vec![CallArgument {
                    parameter: 0,
                    is_default: false,
                    value: Expr {
                        source,
                        ty: string,
                        kind: K::Literal(Literal::String("x".repeat(4096))),
                    },
                }],
                parameter_bindings: vec![],
            },
        };
        let predicate = |skip| Pattern::Guard {
            pattern: Box::new(Pattern::Wild),
            condition: Box::new(Expr {
                source,
                ty: boolean.clone(),
                kind: K::Or(
                    Box::new(Expr {
                        source,
                        ty: boolean.clone(),
                        kind: K::Literal(Literal::Bool(skip)),
                    }),
                    Box::new(call.clone()),
                ),
            }),
        };
        for primitive in [Primitive::F32, Primitive::F64] {
            let ty = Term::Primitive(primitive);
            let skipped = predicate(true);
            assert_eq!(
                check(&Default::default(), &ty, &[&skipped], 1000),
                Coverage::Unknown
            );
            assert_eq!(
                check(&Default::default(), &ty, &[&skipped], 100000),
                Coverage::Exhaustive
            );
            assert_ne!(
                check(&Default::default(), &ty, &[&predicate(false)], 100000),
                Coverage::Exhaustive
            );
        }
    }
    #[test]
    fn budget_failure_does_not_poison_coverage_retry() {
        use crate::coverage::{Coverage, check};
        use nir::program::{LocalId, SourceRange};
        let binding = LocalId { owner: 0, index: 0 };
        let ty = Term::Primitive(Primitive::F64);
        let local = || Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: ty.clone(),
            kind: K::Local(binding),
        };
        let compare = Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::ScalarEqual(Box::new(local()), Box::new(local())),
        };
        let predicate = |condition| Pattern::Guard {
            pattern: Box::new(Pattern::Binding {
                binding,
                inner: Box::new(Pattern::Wild),
            }),
            condition: Box::new(condition),
        };
        let ordered = predicate(compare.clone());
        let unordered = predicate(Expr {
            kind: K::Not(Box::new(compare.clone())),
            ..compare
        });
        let patterns = [&ordered, &unordered];
        assert_eq!(
            check(&Default::default(), &ty, &patterns, 1),
            Coverage::Unknown
        );
        assert_eq!(
            check(&Default::default(), &ty, &patterns, 100000),
            Coverage::Exhaustive
        );
        assert_eq!(
            check(&Default::default(), &ty, &[&ordered], 100000),
            Coverage::Missing
        );
    }
}
