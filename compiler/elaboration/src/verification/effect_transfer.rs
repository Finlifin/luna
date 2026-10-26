//! Bounded identity-transfer proofs shared by effect resource and memory analysis.
//! This module establishes only result origin, never permission to copy, drop,
//! dereference or outlive an input. The installer must check all handler arms.
use nir::program::{Expr, ExprKind as K, LocalId, Pattern};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn forwarded_parameters(
    handler: &nir::program::EffectHandler,
    fuel: &mut usize,
) -> Result<Option<BTreeSet<usize>>, String> {
    forwarded_parameters_in(handler, &[], fuel)
}

pub(crate) fn forwarded_parameters_in(
    handler: &nir::program::EffectHandler,
    effects: &[super::memory::EffectInput],
    fuel: &mut usize,
) -> Result<Option<BTreeSet<usize>>, String> {
    let result = local_forwarded_parameters(effects, handler, fuel)?;
    Ok(result.filter(|origins| !origins.is_empty()))
}

fn local_forwarded_parameters(
    effects: &[super::memory::EffectInput],
    handler: &nir::program::EffectHandler,
    fuel: &mut usize,
) -> Result<Option<BTreeSet<usize>>, String> {
    let result = forwarded_parameters_inner(effects, handler, fuel);
    if result.is_none() && *fuel == 0 {
        return Err("effect transfer proof budget exhausted".into());
    }
    Ok(result)
}

// This evidence is issued only after the complete arm's ownership check. It
// preserves a moved operand's provenance; equal input/output types alone never
// establish a transfer. Other handler bodies retain their completion summaries.
fn forwarded_parameters_inner(
    effects: &[super::memory::EffectInput],
    handler: &nir::program::EffectHandler,
    fuel: &mut usize,
) -> Option<BTreeSet<usize>> {
    let mut common = BTreeSet::new();
    for arm in &handler.arms {
        *fuel = fuel.checked_sub(1)?;
        let parameter = arm_forwarded_parameters(effects, arm, fuel)?;
        common = join_origins(common, parameter, fuel)?;
    }
    Some(common)
}

/// An empty local completion set is bottom, not unknown. Only publish a
/// nonempty transfer after composing the required outer handler evidence.
pub(crate) fn with_handler_fallback(
    handler: &nir::program::EffectHandler,
    outer: Option<&BTreeSet<usize>>,
    fuel: &mut usize,
) -> Result<Option<BTreeSet<usize>>, String> {
    with_handler_fallback_in(handler, outer, &[], fuel)
}

pub(crate) fn with_handler_fallback_in(
    handler: &nir::program::EffectHandler,
    outer: Option<&BTreeSet<usize>>,
    effects: &[super::memory::EffectInput],
    fuel: &mut usize,
) -> Result<Option<BTreeSet<usize>>, String> {
    if handler.fallback.is_none() {
        return forwarded_parameters_in(handler, effects, fuel);
    }
    let local = local_forwarded_parameters(effects, handler, fuel)?;
    Ok(with_fallback(local, outer, fuel)?.filter(|origins| !origins.is_empty()))
}

fn arm_forwarded_parameters(
    effects: &[super::memory::EffectInput],
    arm: &nir::program::MatchArm,
    fuel: &mut usize,
) -> Option<BTreeSet<usize>> {
    if let Some(guard) = &arm.guard {
        observe_scalar(guard, fuel, 0)?;
    }
    let mut pattern = &arm.pattern;
    while let Pattern::Guard {
        pattern: inner,
        condition,
    } = pattern
    {
        observe_scalar(condition, fuel, 0)?;
        pattern = inner;
    }
    let Pattern::Tuple(fields) = pattern else {
        return None;
    };
    let mut inputs = BTreeMap::new();
    for (index, field) in fields.iter().enumerate() {
        *fuel = fuel.checked_sub(1)?;
        observe_pattern(field, Some(&BTreeSet::from([index])), &mut inputs, fuel, 0)?;
    }
    forwarded_value(effects, &arm.body, &mut inputs, fuel, 0)
}

#[cfg(test)]
mod transfer_tests {
    use super::*;
    use nir::{Term, program::SourceRange};

    #[test]
    fn aggregate_fields_do_not_gain_whole_operand_identity() {
        let binding = LocalId {
            owner: 0,
            index: 10,
        };
        let candidate = LocalId {
            owner: 0,
            index: 11,
        };
        let whole = Pattern::Binding {
            binding,
            inner: Box::new(Pattern::Wild),
        };
        let field = Pattern::Remap {
            pattern: Box::new(Pattern::Tuple(vec![Pattern::Binding {
                binding: candidate,
                inner: Box::new(Pattern::Wild),
            }])),
            bindings: vec![(candidate, binding)],
        };
        let origins = BTreeSet::from([2]);
        for patterns in [
            vec![whole.clone(), field.clone()],
            vec![field.clone(), whole.clone()],
        ] {
            let mut inputs = BTreeMap::new();
            observe_pattern(
                &Pattern::Or(patterns),
                Some(&origins),
                &mut inputs,
                &mut 200,
                0,
            )
            .unwrap();
            assert!(!inputs.contains_key(&binding));
            assert!(!inputs.contains_key(&candidate));
        }
        let pattern = Pattern::Binding {
            binding,
            inner: Box::new(Pattern::Tuple(vec![Pattern::Wild])),
        };
        let mut inputs = BTreeMap::new();
        observe_pattern(&pattern, Some(&origins), &mut inputs, &mut 200, 0).unwrap();
        assert_eq!(inputs.get(&binding), Some(&origins));
        assert!(
            observe_pattern(&pattern, Some(&origins), &mut BTreeMap::new(), &mut 0, 0).is_none()
        );
    }

    #[test]
    fn branch_transfer_unions_origins_and_rejects_unproved_observations() {
        let ty = Term::Primitive(nir::Primitive::Bool);
        let first = LocalId { owner: 0, index: 0 };
        let second = LocalId { owner: 0, index: 1 };
        let source = SourceRange { start: 0, end: 1 };
        let arm = |result| nir::program::MatchArm {
            pattern: Pattern::Tuple(vec![
                Pattern::Binding {
                    binding: first,
                    inner: Box::new(Pattern::Wild),
                },
                Pattern::Binding {
                    binding: second,
                    inner: Box::new(Pattern::Wild),
                },
            ]),
            guard: None,
            body: Expr {
                source,
                ty: ty.clone(),
                kind: K::Local(result),
            },
        };
        let mut handler = nir::program::EffectHandler {
            operation: nir::effects::Operation {
                declaration: nir::DeclarationId {
                    package: 0,
                    local: 0,
                },
                arguments: vec![],
            },
            abi: nir::effects::Abi::Direct,
            signature: ty.clone(),
            arms: vec![arm(first), arm(first)],
            fallback: None,
        };
        assert_eq!(
            forwarded_parameters(&handler, &mut 100).unwrap(),
            Some(BTreeSet::from([0]))
        );
        handler.arms[1] = arm(second);
        assert_eq!(
            forwarded_parameters(&handler, &mut 100).unwrap(),
            Some(BTreeSet::from([0, 1]))
        );
        handler.arms[1] = arm(first);
        handler.arms[1].guard = Some(Expr {
            source,
            ty: ty.clone(),
            kind: K::Literal(nir::program::Literal::Bool(true)),
        });
        assert_eq!(
            forwarded_parameters(&handler, &mut 100).unwrap(),
            Some(BTreeSet::from([0]))
        );
        let guard = handler.arms[1].guard.take().unwrap();
        handler.arms[1].guard = Some(Expr {
            source,
            ty: guard.ty.clone(),
            kind: K::Call {
                callee: Box::new(guard),
                arguments: vec![],
                parameter_bindings: vec![],
            },
        });
        assert_eq!(forwarded_parameters(&handler, &mut 100).unwrap(), None);
        handler.arms[1].guard = None;
        assert!(forwarded_parameters(&handler, &mut 0).is_err());
        assert_eq!(
            forwarded_parameters(&handler, &mut 100).unwrap(),
            Some(BTreeSet::from([0]))
        );
        let left = arm(first).body;
        let right = arm(second).body;
        handler.arms = vec![arm(first)];
        handler.arms[0].body = Expr {
            source,
            ty: left.ty.clone(),
            kind: K::If {
                condition: Box::new(left.clone()),
                then_branch: Box::new(left.clone()),
                else_branch: Some(Box::new(left.clone())),
            },
        };
        assert_eq!(
            forwarded_parameters(&handler, &mut 100).unwrap(),
            Some(BTreeSet::from([0]))
        );
        assert!(forwarded_parameters(&handler, &mut 6).is_err());
        assert_eq!(
            forwarded_parameters(&handler, &mut 100).unwrap(),
            Some(BTreeSet::from([0]))
        );
        if let K::If { else_branch, .. } = &mut handler.arms[0].body.kind {
            *else_branch = Some(Box::new(right));
        }
        assert_eq!(
            forwarded_parameters(&handler, &mut 100).unwrap(),
            Some(BTreeSet::from([0, 1]))
        );

        let selected = LocalId { owner: 0, index: 2 };
        let nested = LocalId { owner: 0, index: 3 };
        let local = |binding| Expr {
            source,
            ty: ty.clone(),
            kind: K::Local(binding),
        };
        handler.arms[0].body = Expr {
            source,
            ty: ty.clone(),
            kind: K::Match {
                value: Box::new(local(first)),
                arms: vec![nir::program::MatchArm {
                    pattern: Pattern::Binding {
                        binding: selected,
                        inner: Box::new(Pattern::Wild),
                    },
                    guard: None,
                    body: Expr {
                        source,
                        ty: ty.clone(),
                        kind: K::Match {
                            value: Box::new(local(selected)),
                            arms: vec![nir::program::MatchArm {
                                pattern: Pattern::Binding {
                                    binding: nested,
                                    inner: Box::new(Pattern::Wild),
                                },
                                guard: Some(local(nested)),
                                body: local(nested),
                            }],
                        },
                    },
                }],
            },
        };
        assert_eq!(
            forwarded_parameters(&handler, &mut 200).unwrap(),
            Some(BTreeSet::from([0]))
        );
        // Equal scalar types do not make a computed selector an identity transfer.
        if let K::Match { value, .. } = &mut handler.arms[0].body.kind {
            *value = Box::new(Expr {
                source,
                ty: ty.clone(),
                kind: K::Not(Box::new(local(first))),
            });
        }
        assert_eq!(forwarded_parameters(&handler, &mut 200).unwrap(), None);
        if let K::Match { value, arms } = &mut handler.arms[0].body.kind {
            *value = Box::new(local(first));
            // A sibling arm must not inherit the first arm's bindings.
            arms.push(nir::program::MatchArm {
                pattern: Pattern::Wild,
                guard: None,
                body: local(selected),
            });
        }
        assert_eq!(forwarded_parameters(&handler, &mut 200).unwrap(), None);

        let resume = |binding| Expr {
            source,
            ty: Term::Primitive(nir::Primitive::Never),
            kind: K::EffectResume(Box::new(local(binding))),
        };
        handler.arms[0].body = Expr {
            source,
            ty: ty.clone(),
            kind: K::Block {
                statements: vec![Expr {
                    source,
                    ty: Term::Primitive(nir::Primitive::Void),
                    kind: K::If {
                        condition: Box::new(local(first)),
                        then_branch: Box::new(resume(first)),
                        else_branch: None,
                    },
                }],
                result: Some(Box::new(local(second))),
            },
        };
        assert_eq!(
            forwarded_parameters(&handler, &mut 200).unwrap(),
            Some(BTreeSet::from([0, 1]))
        );
        // Discarding a local in the final statement is not a resume, even when
        // another path already delivered a valid operand.
        if let K::Block { statements, result } = &mut handler.arms[0].body.kind {
            statements.push(local(second));
            *result = None;
        }
        assert_eq!(forwarded_parameters(&handler, &mut 200).unwrap(), None);
        if let K::Block { statements, .. } = &mut handler.arms[0].body.kind {
            *statements.last_mut().unwrap() = resume(second);
        }
        assert_eq!(
            forwarded_parameters(&handler, &mut 200).unwrap(),
            Some(BTreeSet::from([0, 1]))
        );
        assert!(forwarded_parameters(&handler, &mut 3).is_err());
        assert_eq!(
            forwarded_parameters(&handler, &mut 200).unwrap(),
            Some(BTreeSet::from([0, 1]))
        );
        // Lexical returns do not deliver their operand to a request. A whole
        // arm that returns can coexist with another arm that forwards input 0.
        let lexical = |value| Expr {
            source,
            ty: Term::Primitive(nir::Primitive::Never),
            kind: K::Return(Some(Box::new(value))),
        };
        handler.arms = vec![arm(first), arm(second)];
        handler.arms[1].body = lexical(local(second));
        assert_eq!(
            forwarded_parameters(&handler, &mut 200).unwrap(),
            Some(BTreeSet::from([0]))
        );
        handler.arms[0].body = lexical(local(first));
        assert_eq!(forwarded_parameters(&handler, &mut 200).unwrap(), None);
        // Evaluation of a return operand can resume first; keep that delivery.
        handler.arms[1].body = lexical(resume(second));
        assert_eq!(
            forwarded_parameters(&handler, &mut 200).unwrap(),
            Some(BTreeSet::from([1]))
        );
        assert!(forwarded_parameters(&handler, &mut 1).is_err());
        assert_eq!(
            forwarded_parameters(&handler, &mut 200).unwrap(),
            Some(BTreeSet::from([1]))
        );
        let call = Expr {
            source,
            ty: ty.clone(),
            kind: K::Call {
                callee: Box::new(Expr {
                    source,
                    ty: Term::Function {
                        parameters: vec![],
                        result: Box::new(ty.clone()),
                    },
                    kind: K::Function(nir::DeclarationId {
                        package: 0,
                        local: 99,
                    }),
                }),
                arguments: vec![],
                parameter_bindings: vec![],
            },
        };
        handler.arms[0].body = local(first);
        handler.arms[1].body = lexical(call.clone());
        assert_eq!(
            forwarded_parameters(&handler, &mut 200).unwrap(),
            Some(BTreeSet::from([0]))
        );
        // The same call on a completion path is still opaque, not an identity.
        handler.arms[1].body = call.clone();
        assert_eq!(forwarded_parameters(&handler, &mut 200).unwrap(), None);
        handler.fallback = Some(0);
        let outer = BTreeSet::from([1]);
        // An unknown local completion cannot borrow the outer handler's proof.
        assert_eq!(
            with_handler_fallback(&handler, Some(&outer), &mut 300).unwrap(),
            None
        );
        handler.arms[1].body = lexical(call);
        assert_eq!(
            with_handler_fallback(&handler, Some(&outer), &mut 300).unwrap(),
            Some(BTreeSet::from([0, 1]))
        );
        handler.arms[0].body = lexical(local(first));
        assert_eq!(
            with_handler_fallback(&handler, Some(&outer), &mut 300).unwrap(),
            Some(outer.clone())
        );
        assert_eq!(
            with_handler_fallback(&handler, None, &mut 300).unwrap(),
            None
        );
        assert!(with_handler_fallback(&handler, Some(&outer), &mut 0).is_err());
        assert_eq!(
            with_handler_fallback(&handler, Some(&outer), &mut 300).unwrap(),
            Some(outer)
        );
        handler.fallback = None;
        assert_eq!(forwarded_parameters(&handler, &mut 300).unwrap(), None);
        handler.arms = vec![arm(first)];
        let product = |fields| Expr {
            source,
            ty: Term::Tuple(vec![ty.clone(), ty.clone()]),
            kind: K::Tuple(fields),
        };
        let constructed = product(vec![local(first), local(second)]);
        handler.arms[0].body = Expr {
            source,
            ty: ty.clone(),
            kind: K::Block {
                statements: vec![Expr {
                    source,
                    ty: Term::Primitive(nir::Primitive::Void),
                    kind: K::Let {
                        binding: LocalId {
                            owner: 0,
                            index: 99,
                        },
                        value: Box::new(constructed.clone()),
                    },
                }],
                result: Some(Box::new(local(first))),
            },
        };
        assert_eq!(
            forwarded_parameters(&handler, &mut 300).unwrap(),
            Some(BTreeSet::from([0]))
        );
        handler.arms[0].body = constructed;
        assert_eq!(forwarded_parameters(&handler, &mut 300).unwrap(), None);
        // Evaluated fields are not delivered. Only resume supplies the result,
        // and no field after a nonlocal exit may contribute an origin.
        handler.arms[0].body = product(vec![local(first), resume(second)]);
        assert_eq!(
            forwarded_parameters(&handler, &mut 300).unwrap(),
            Some(BTreeSet::from([1]))
        );
        handler.arms[0].body = product(vec![lexical(local(first)), resume(second)]);
        assert_eq!(forwarded_parameters(&handler, &mut 300).unwrap(), None);
        let observed = Expr {
            source,
            ty: ty.clone(),
            kind: K::ReadPointer(Box::new(Expr {
                source,
                ty: Term::Pointer(Box::new(ty.clone())),
                kind: K::Local(LocalId {
                    owner: 0,
                    index: 98,
                }),
            })),
        };
        handler.arms = vec![arm(first)];
        handler.arms[0].guard = Some(observed.clone());
        assert_eq!(
            forwarded_parameters(&handler, &mut 300).unwrap(),
            Some(BTreeSet::from([0]))
        );
        handler.arms[0].guard = None;
        handler.arms[0].body = observed.clone();
        // The observed scalar itself is not the whole forwarded input.
        assert_eq!(forwarded_parameters(&handler, &mut 300).unwrap(), None);
        handler.arms[0].body = local(first);
        let mut produced = observed;
        if let K::ReadPointer(pointer) = &mut produced.kind {
            pointer.kind = K::AddressOf(Box::new(local(second)));
        }
        // Exposing an input's storage address requires separate evidence.
        handler.arms[0].guard = Some(produced);
        assert_eq!(forwarded_parameters(&handler, &mut 300).unwrap(), None);
        handler.arms[0].guard = None;
        let loop_id = LocalId {
            owner: 0,
            index: 100,
        };
        for exit in [K::Break(loop_id), K::Continue(loop_id)] {
            let terminal = Expr {
                source,
                ty: Term::Primitive(nir::Primitive::Never),
                kind: exit,
            };
            handler.arms = vec![arm(first), arm(second)];
            handler.arms[1].body = terminal.clone();
            assert_eq!(
                forwarded_parameters(&handler, &mut 300).unwrap(),
                Some(BTreeSet::from([0]))
            );
            // No later expression can supply a completion after the loop exit.
            handler.arms[0].body = product(vec![terminal, resume(second)]);
            assert_eq!(forwarded_parameters(&handler, &mut 300).unwrap(), None);
        }
    }
}

// Aliases preserve origin; branches union their possible operand origins. Branch selectors may only
// observe scalar storage. Calls, assignments and computed patterns require
// their own state-transform summary and must not inherit this evidence.
// Keep normal completion separate from resume: a discarded value is not a
// delivery, and an early resume must not inherit a later statement's origin.
struct TransferFlow {
    resumed: BTreeSet<usize>,
    normal: Option<BTreeSet<usize>>,
    continues: bool,
}
impl TransferFlow {
    fn normal(origins: Option<BTreeSet<usize>>) -> Self {
        Self {
            resumed: BTreeSet::new(),
            normal: origins,
            continues: true,
        }
    }
    fn delivered(self, fuel: &mut usize) -> Option<BTreeSet<usize>> {
        let result = if self.continues {
            join_origins(self.resumed, self.normal?, fuel)?
        } else {
            self.resumed
        };
        Some(result)
    }
    fn join(self, other: Self, fuel: &mut usize) -> Option<Self> {
        let normal = match (self.continues, other.continues) {
            (true, true) => match (self.normal, other.normal) {
                (Some(left), Some(right)) => Some(join_origins(left, right, fuel)?),
                _ => None,
            },
            (true, false) => self.normal,
            (false, true) => other.normal,
            (false, false) => None,
        };
        Some(Self {
            resumed: join_origins(self.resumed, other.resumed, fuel)?,
            normal,
            continues: self.continues || other.continues,
        })
    }
}
fn forwarded_value(
    effects: &[super::memory::EffectInput],
    value: &Expr,
    inputs: &mut BTreeMap<LocalId, BTreeSet<usize>>,
    fuel: &mut usize,
    depth: usize,
) -> Option<BTreeSet<usize>> {
    transfer_flow(effects, value, inputs, fuel, depth)?.delivered(fuel)
}
fn transfer_flow(
    effects: &[super::memory::EffectInput],
    value: &Expr,
    inputs: &mut BTreeMap<LocalId, BTreeSet<usize>>,
    fuel: &mut usize,
    depth: usize,
) -> Option<TransferFlow> {
    if depth >= 128 {
        return None;
    }
    spend(fuel, 1)?;
    match &value.kind {
        K::EffectRequest {
            operation,
            abi,
            signature,
            arguments,
            ..
        } => {
            // Only outer, checked identity premises may justify this result.
            // No candidate-local handler fact is introduced into this walk.
            let nir::Term::Function { parameters, result } = signature.as_ref() else {
                return None;
            };
            if *abi != nir::effects::Abi::Direct
                || parameters.len() != arguments.len()
                || value.ty != **result
            {
                return None;
            }
            spend(fuel, effects.len())?;
            let mut selected = None;
            for input in effects {
                super::memory::bound_type(&input.signature, fuel).ok()?;
                for argument in &input.operation.arguments {
                    super::memory::bound_type(argument, fuel).ok()?;
                }
                if input.operation == *operation
                    && input.abi == *abi
                    && input.signature == **signature
                {
                    let origins = input.forwarded_parameters.as_ref()?;
                    spend(fuel, origins.len())?;
                    if origins.is_empty()
                        || origins.iter().any(|slot| {
                            parameters
                                .get(*slot)
                                .is_none_or(|parameter| parameter.ty != **result)
                        })
                    {
                        return None;
                    }
                    if selected.is_some_and(|previous| previous != origins) {
                        return None;
                    }
                    selected = Some(origins);
                }
            }
            let selected = selected?;
            let mut resumed = BTreeSet::new();
            let mut delivered = BTreeSet::new();
            for (slot, argument) in arguments.iter().enumerate() {
                if argument.ty != parameters[slot].ty {
                    return None;
                }
                let flow = transfer_flow(effects, argument, inputs, fuel, depth + 1)?;
                resumed = join_origins(resumed, flow.resumed, fuel)?;
                if !flow.continues {
                    return Some(TransferFlow {
                        resumed,
                        normal: None,
                        continues: false,
                    });
                }
                if selected.contains(&slot) {
                    delivered = join_origins(delivered, flow.normal?, fuel)?;
                }
            }
            Some(TransferFlow {
                resumed,
                normal: Some(delivered),
                continues: true,
            })
        }
        K::Local(binding) => {
            let origins = if let Some(origins) = inputs.get(binding) {
                spend(fuel, origins.len())?;
                Some(origins.clone())
            } else {
                None
            };
            Some(TransferFlow::normal(origins))
        }
        K::UnsafeScope(value) => transfer_flow(effects, value, inputs, fuel, depth + 1),
        K::Discard(value) => {
            let mut flow = transfer_flow(effects, value, inputs, fuel, depth + 1)?;
            flow.normal = None;
            Some(flow)
        }
        K::Return(value) => {
            // A lexical return delivers to the installer, never to the
            // suspended request. Its operand may itself resume, so preserve
            // those exits while excluding the lexical value from completion.
            // Provenance and lifetime of that value are checked separately.
            let mut flow = if let Some(value) = value {
                let mut resumes = false;
                nir::visit::expression_in_scope(value, fuel, |node, _| {
                    resumes |= matches!(node.kind, K::EffectResume(_));
                    Ok(())
                })
                .ok()?;
                if resumes {
                    transfer_flow(effects, value, inputs, fuel, depth + 1)?
                } else {
                    // Arbitrary checked construction/calls on a lexical-exit
                    // path cannot contribute a normal completion operand.
                    // Nested closures and handler arms own separate resumes.
                    TransferFlow::normal(None)
                }
            } else {
                TransferFlow::normal(None)
            };
            flow.normal = None;
            flow.continues = false;
            Some(flow)
        }
        K::Break(_) | K::Continue(_) => {
            // These exit to a lexical loop, not to the suspended request.
            // Loop-target validity and unwind ownership are checked separately.
            // Local loop expressions are not inferred by this transfer walk.
            Some(TransferFlow {
                resumed: BTreeSet::new(),
                normal: None,
                continues: false,
            })
        }
        K::EffectResume(value) => {
            let flow = transfer_flow(effects, value, inputs, fuel, depth + 1)?;
            Some(TransferFlow {
                resumed: flow.delivered(fuel)?,
                normal: None,
                continues: false,
            })
        }
        K::If {
            condition,
            then_branch,
            else_branch,
        } => {
            observe_scalar(condition, fuel, depth + 1)?;
            let left = transfer_flow(
                effects,
                then_branch,
                &mut copy_inputs(inputs, fuel)?,
                fuel,
                depth + 1,
            )?;
            let right = if let Some(branch) = else_branch {
                transfer_flow(
                    effects,
                    branch,
                    &mut copy_inputs(inputs, fuel)?,
                    fuel,
                    depth + 1,
                )?
            } else {
                TransferFlow::normal(None)
            };
            left.join(right, fuel)
        }
        K::Match {
            value: scrutinee,
            arms,
        } => {
            let subject = transfer_flow(
                effects,
                scrutinee,
                &mut copy_inputs(inputs, fuel)?,
                fuel,
                depth + 1,
            )?;
            if !subject.continues {
                return Some(subject);
            }
            let origins = subject.normal;
            let mut combined: Option<TransferFlow> = None;
            for arm in arms {
                let mut branch = copy_inputs(inputs, fuel)?;
                observe_pattern(&arm.pattern, origins.as_ref(), &mut branch, fuel, depth + 1)?;
                if let Some(guard) = &arm.guard {
                    observe_scalar(guard, fuel, depth + 1)?;
                }
                let flow = transfer_flow(effects, &arm.body, &mut branch, fuel, depth + 1)?;
                combined = Some(match combined {
                    Some(previous) => previous.join(flow, fuel)?,
                    None => flow,
                });
            }
            let mut combined = combined?;
            combined.resumed = join_origins(subject.resumed, combined.resumed, fuel)?;
            Some(combined)
        }
        K::Tuple(fields)
        | K::Record { fields, .. }
        | K::Object { fields, .. }
        | K::AnonymousRecord { fields }
        | K::Variant { fields, .. } => {
            // Construction evaluates fields in order but is not a transfer of
            // any whole input. A local constructed owner may precede a later
            // identity delivery; its cleanup remains the resource checker's
            // responsibility. Calls and slot-address exposure still need their
            // own state-transform evidence and are not admitted by this walk.
            let mut resumed = BTreeSet::new();
            for field in fields {
                let flow = transfer_flow(effects, field, inputs, fuel, depth + 1)?;
                resumed = join_origins(resumed, flow.resumed, fuel)?;
                if !flow.continues {
                    return Some(TransferFlow {
                        resumed,
                        normal: None,
                        continues: false,
                    });
                }
            }
            Some(TransferFlow {
                resumed,
                normal: None,
                continues: true,
            })
        }
        K::Field { .. } | K::AnonymousField { .. } => {
            // A direct projection may be observed, but it is not the complete
            // input. It needs field-aware evidence before it can be delivered.
            observe_place(value, fuel, depth + 1)?;
            Some(TransferFlow::normal(None))
        }
        K::Block { statements, result } => {
            let mut locals = copy_inputs(inputs, fuel)?;
            let mut resumed = BTreeSet::new();
            for statement in statements {
                let (binding, initializer) = match &statement.kind {
                    K::Let { binding, value } => (Some(*binding), value.as_ref()),
                    K::LetPattern { value, .. } => (None, value.as_ref()),
                    _ => (None, statement),
                };
                spend(fuel, 1)?;
                let flow = transfer_flow(effects, initializer, &mut locals, fuel, depth + 1)?;
                resumed = join_origins(resumed, flow.resumed, fuel)?;
                if !flow.continues {
                    return Some(TransferFlow {
                        resumed,
                        normal: None,
                        continues: false,
                    });
                }
                if let Some(binding) = binding {
                    if let Some(origins) = flow.normal {
                        locals.insert(binding, origins);
                    } else {
                        locals.remove(&binding);
                    }
                } else if let K::LetPattern {
                    pattern,
                    guard,
                    otherwise,
                    ..
                } = &statement.kind
                {
                    let mut matched = copy_inputs(&locals, fuel)?;
                    observe_pattern(pattern, flow.normal.as_ref(), &mut matched, fuel, depth + 1)?;
                    if let Some(guard) = guard {
                        observe_scalar(guard, fuel, depth + 1)?;
                    }
                    if let Some(otherwise) = otherwise {
                        // Failure cannot see successful pattern bindings and
                        // must leave this path, as required by let-else typing.
                        let failed = transfer_flow(
                            effects,
                            otherwise,
                            &mut copy_inputs(&locals, fuel)?,
                            fuel,
                            depth + 1,
                        )?;
                        if failed.continues {
                            return None;
                        }
                        resumed = join_origins(resumed, failed.resumed, fuel)?;
                    }
                    locals = matched;
                }
            }
            let tail = if let Some(result) = result {
                transfer_flow(effects, result, &mut locals, fuel, depth + 1)?
            } else {
                TransferFlow::normal(None)
            };
            Some(TransferFlow {
                resumed: join_origins(resumed, tail.resumed, fuel)?,
                normal: tail.normal,
                continues: tail.continues,
            })
        }
        _ => {
            observe_scalar(value, fuel, depth + 1)?;
            Some(TransferFlow::normal(None))
        }
    }
}

fn copy_inputs(
    inputs: &BTreeMap<LocalId, BTreeSet<usize>>,
    fuel: &mut usize,
) -> Option<BTreeMap<LocalId, BTreeSet<usize>>> {
    let cost = inputs.values().fold(inputs.len(), |cost, origins| {
        cost.saturating_add(origins.len())
    });
    spend(fuel, cost)?;
    Some(inputs.clone())
}

// This is deliberately an origin-preservation check, not a purity inference:
// No calls, stores, or user-defined operations are inferred safe. Primitive
// reads through existing pointer positions do not change operand identity;
// this proof grants no permission to read or extend the lifetime of a target.
// The enclosing resource/memory analyses still check types and observations.
fn observe_scalar(value: &Expr, fuel: &mut usize, depth: usize) -> Option<()> {
    if depth >= 128 || !matches!(value.ty, nir::Term::Primitive(_)) {
        return None;
    }
    *fuel = fuel.checked_sub(1)?;
    match &value.kind {
        K::Literal(_) | K::Unit | K::Local(_) => Some(()),
        K::ReadPointer(pointer) => observe_place(pointer, fuel, depth + 1),
        K::Field { .. } | K::AnonymousField { .. } => observe_place(value, fuel, depth + 1),
        K::Not(value) | K::IntegerNegate(value) | K::FloatNegate(value) => {
            observe_scalar(value, fuel, depth + 1)
        }
        K::And(left, right)
        | K::Or(left, right)
        | K::TemplateEqual(left, right)
        | K::ScalarEqual(left, right)
        | K::ScalarLess(left, right)
        | K::ScalarLessEqual(left, right)
        | K::ScalarGreater(left, right)
        | K::ScalarGreaterEqual(left, right)
        | K::IntegerBinary { left, right, .. }
        | K::FloatBinary { left, right, .. } => {
            observe_scalar(left, fuel, depth + 1)?;
            observe_scalar(right, fuel, depth + 1)
        }
        K::If {
            condition,
            then_branch,
            else_branch,
        } => {
            observe_scalar(condition, fuel, depth + 1)?;
            observe_scalar(then_branch, fuel, depth + 1)?;
            if let Some(branch) = else_branch {
                observe_scalar(branch, fuel, depth + 1)?;
            }
            Some(())
        }
        K::Match { value, arms } => {
            observe_subject(value, fuel, depth + 1)?;
            for arm in arms {
                observe_pattern(&arm.pattern, None, &mut BTreeMap::new(), fuel, depth + 1)?;
                if let Some(guard) = &arm.guard {
                    observe_scalar(guard, fuel, depth + 1)?;
                }
                observe_scalar(&arm.body, fuel, depth + 1)?;
            }
            Some(())
        }
        K::UnsafeScope(value) | K::Discard(value) => observe_scalar(value, fuel, depth + 1),
        K::Block { statements, result } => {
            for statement in statements {
                spend(fuel, 1)?;
                match &statement.kind {
                    K::Let { value, .. } => observe_scalar(value, fuel, depth + 1)?,
                    K::LetPattern {
                        pattern,
                        value,
                        guard: None,
                        otherwise: None,
                    } => {
                        observe_scalar(value, fuel, depth + 1)?;
                        observe_pattern(pattern, None, &mut BTreeMap::new(), fuel, depth + 1)?;
                    }
                    _ => observe_scalar(statement, fuel, depth + 1)?,
                }
            }
            if let Some(result) = result {
                observe_scalar(result, fuel, depth + 1)?;
            }
            Some(())
        }
        _ => None,
    }
}
// Structural matching can observe an existing aggregate or pointer without
// evaluating a producer or reading through a pointer. The resource pass still
// checks any consumption and the pattern proof distinguishes fields from whole.
fn observe_subject(value: &Expr, fuel: &mut usize, depth: usize) -> Option<()> {
    if matches!(value.ty, nir::Term::Primitive(_)) {
        observe_scalar(value, fuel, depth)
    } else {
        observe_place(value, fuel, depth)
    }
}
fn observe_place(value: &Expr, fuel: &mut usize, depth: usize) -> Option<()> {
    if depth >= 128 {
        return None;
    }
    *fuel = fuel.checked_sub(1)?;
    match &value.kind {
        K::Local(_) => Some(()),
        K::Field { value, .. } | K::AnonymousField { value, .. } => {
            observe_place(value, fuel, depth + 1)
        }
        _ => None,
    }
}
fn observe_pattern(
    pattern: &Pattern,
    origins: Option<&BTreeSet<usize>>,
    inputs: &mut BTreeMap<LocalId, BTreeSet<usize>>,
    fuel: &mut usize,
    depth: usize,
) -> Option<()> {
    if depth >= 128 {
        return None;
    }
    *fuel = fuel.checked_sub(1)?;
    match pattern {
        Pattern::Wild | Pattern::Literal(_) | Pattern::Unit => Some(()),
        Pattern::Binding { binding, inner } => {
            // The scalar subject may be an operand alias, or a computed selector.
            // Only the former carries identity. Keep bindings local to this arm.
            if let Some(origins) = origins {
                spend(fuel, origins.len())?;
                inputs.insert(*binding, origins.clone());
            } else {
                inputs.remove(binding);
            }
            observe_pattern(inner, origins, inputs, fuel, depth + 1)
        }
        Pattern::Remap { pattern, bindings } => {
            // Candidate bindings are renamed simultaneously, before becoming
            // visible to the arm. Do not read origins from surrounding locals.
            let mut candidate = BTreeMap::new();
            observe_pattern(pattern, origins, &mut candidate, fuel, depth + 1)?;
            let mut renamed = Vec::new();
            for (from, to) in bindings {
                spend(fuel, 1)?;
                let value = candidate.get(from);
                if let Some(value) = value {
                    spend(fuel, value.len())?;
                }
                renamed.push((*from, *to, value.cloned()));
            }
            for (from, _, _) in &renamed {
                candidate.remove(from);
            }
            for (_, to, value) in renamed {
                if let Some(value) = value {
                    candidate.insert(to, value);
                } else {
                    inputs.remove(&to);
                }
            }
            spend(fuel, candidate.len())?;
            inputs.extend(candidate);
            Some(())
        }
        Pattern::Tuple(fields)
        | Pattern::Record { fields, .. }
        | Pattern::AnonymousRecord { fields, .. }
        | Pattern::Variant { fields, .. } => {
            // Decomposition observes shape, but a field binding is not an alias
            // of the complete operand. Only an enclosing Binding keeps identity.
            for field in fields {
                observe_pattern(field, None, inputs, fuel, depth + 1)?;
            }
            Some(())
        }
        Pattern::Observe { inner, .. } => observe_pattern(inner, origins, inputs, fuel, depth + 1),
        Pattern::Or(patterns) => {
            let mut common: Option<BTreeMap<LocalId, BTreeSet<usize>>> = None;
            for pattern in patterns {
                let mut candidate = copy_inputs(inputs, fuel)?;
                observe_pattern(pattern, origins, &mut candidate, fuel, depth + 1)?;
                if let Some(common) = &mut common {
                    // A canonical binding carries identity only if every
                    // alternative supplies that same complete operand.
                    spend(
                        fuel,
                        common.values().fold(common.len(), |cost, origins| {
                            cost.saturating_add(origins.len())
                        }),
                    )?;
                    common.retain(|binding, origins| candidate.get(binding) == Some(origins));
                } else {
                    common = Some(candidate);
                }
            }
            *inputs = common?;
            Some(())
        }
        Pattern::Guard { pattern, condition } => {
            observe_pattern(pattern, origins, inputs, fuel, depth + 1)?;
            observe_scalar(condition, fuel, depth + 1)
        }
        _ => None,
    }
}

fn spend(fuel: &mut usize, amount: usize) -> Option<()> {
    if *fuel < amount {
        *fuel = 0;
        return None;
    }
    *fuel -= amount;
    Some(())
}
fn join_origins(
    mut left: BTreeSet<usize>,
    right: BTreeSet<usize>,
    fuel: &mut usize,
) -> Option<BTreeSet<usize>> {
    spend(fuel, right.len())?;
    left.extend(right);
    Some(left)
}
/// Fallback can deliver any input delivered by either matching installation.
/// An unknown premise on either path keeps the combined result unknown.
pub(crate) fn with_fallback(
    local: Option<BTreeSet<usize>>,
    outer: Option<&BTreeSet<usize>>,
    fuel: &mut usize,
) -> Result<Option<BTreeSet<usize>>, String> {
    let (Some(mut local), Some(outer)) = (local, outer) else {
        return Ok(None);
    };
    spend(fuel, outer.len()).ok_or("effect transfer fallback budget exhausted")?;
    local.extend(outer.iter().copied());
    Ok(Some(local))
}

#[cfg(test)]
mod outer_request_tests {
    use super::*;
    use nir::{
        DeclarationId, Parameter, Primitive, Sort, Term,
        effects::{Abi, Operation},
        program::SourceRange,
    };

    #[test]
    fn outer_request_identity_requires_matching_evidence_and_recovers_after_exhaustion() {
        let pointer = Term::Pointer(Box::new(Term::Primitive(Primitive::Bool)));
        let signature = Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: pointer.clone(),
            }],
            result: Box::new(pointer.clone()),
        };
        let operation = Operation {
            declaration: DeclarationId {
                package: 1,
                local: 2,
            },
            arguments: vec![],
        };
        let binding = LocalId { owner: 1, index: 0 };
        let source = SourceRange { start: 0, end: 1 };
        let request = Expr {
            ty: pointer.clone(),
            source,
            kind: K::EffectRequest {
                operation: operation.clone(),
                abi: Abi::Direct,
                signature: Box::new(signature.clone()),
                allowed_index: 0,
                arguments: vec![Expr {
                    ty: pointer,
                    source,
                    kind: K::Local(binding),
                }],
            },
        };
        let evidence = super::super::memory::EffectInput {
            operation,
            abi: Abi::Direct,
            signature,
            forwarded_parameters: Some(BTreeSet::from([0])),
            copy_parameters: BTreeSet::new(),
            owned_inputs: false,
            parameter_accesses: BTreeSet::new(),
        };
        let run = |effects: &[super::super::memory::EffectInput], fuel| {
            forwarded_value(
                effects,
                &request,
                &mut BTreeMap::from([(binding, BTreeSet::from([7]))]),
                &mut { fuel },
                0,
            )
        };
        assert_eq!(
            run(std::slice::from_ref(&evidence), 1000),
            Some(BTreeSet::from([7]))
        );
        assert_eq!(run(&[], 1000), None);
        assert_eq!(run(std::slice::from_ref(&evidence), 0), None);
        for case in 0..5 {
            let mut invalid = evidence.clone();
            match case {
                0 => invalid.operation.declaration.local += 1,
                1 => invalid.abi = Abi::DirectPassing,
                2 => invalid.forwarded_parameters = None,
                3 => invalid.forwarded_parameters = Some(BTreeSet::from([1])),
                _ => invalid.signature = Term::Primitive(Primitive::Bool),
            }
            assert_eq!(run(&[invalid], 1000), None, "case {case}");
        }
        assert_eq!(run(&[evidence], 1000), Some(BTreeSet::from([7])));
    }
}
