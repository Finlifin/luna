//! Check the source-selected push iteration contract before lowering consumers.
use super::*;
use crate::ptc::Budget;

impl Checker<'_, '_> {
    pub(super) fn iterator_loop(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let H::For {
            pat,
            guard,
            iter,
            body,
            ..
        } = expr.kind
        else {
            unreachable!()
        };
        let protocol = self.protocols.iterator.ok_or_else(|| {
            error(
                expr.span,
                "for requires iteration protocol elaboration with a bound Iterator",
            )
        })?;
        if self.in_required_comptime
            || self
                .signature_owner
                .and_then(|owner| self.package.item(owner))
                .is_some_and(|item| {
                    matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                if sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head)
                })
        {
            return Err(error(
                expr.span,
                "comptime effect iteration is not implemented",
            ));
        }
        // Evaluate the iterable before installing the consumer's delivery
        // handler, and outside both the element scope and the loop target.
        let mut iterable = self.expr(iter, None)?;
        if let Some(conversion) = self.protocols.into_iterator {
            let interface = n::InterfaceInstance {
                declaration: protocol,
                arguments: vec![],
            };
            let direct = if self.symbolic_interface_evidence(&iterable.ty, &interface) {
                true
            } else if self.symbolic_interface_evidence(
                &iterable.ty,
                &n::InterfaceInstance {
                    declaration: conversion,
                    arguments: vec![],
                },
            ) {
                // A generic body's declared conversion evidence determines
                // its operation; possible future Iterator implementations do
                // not change that body when it is instantiated.
                false
            } else {
                let resolution = self
                    .resolve_trait_capability(&interface, &iterable.ty, Budget::default())
                    .map_err(|reason| {
                        error(
                            expr.span,
                            format!("iteration capability query failed: {reason:?}"),
                        )
                    })?;
                match resolution.selection {
                    crate::implementation::Selection::Selected(_) => true,
                    crate::implementation::Selection::NoApplicableCandidate => false,
                    other => {
                        return Err(error(
                            expr.span,
                            format!("iteration capability is not established: {other}"),
                        ));
                    }
                }
            };
            if !direct {
                let method = self
                    .selected_method(
                        iterable.ty.clone(),
                        Some(n::InterfaceInstance {
                            declaration: conversion,
                            arguments: vec![],
                        }),
                        "into_iter",
                        expr.span,
                    )?
                    .ok_or_else(|| error(expr.span, "IntoIterator into_iter method unavailable"))?;
                iterable = self.finish_call(expr, method, &[], vec![iterable], None, None)?;
            }
        }
        let saved_locals = self.locals.clone();
        let saved_immutable = self.immutable.clone();
        let saved_context = self.context.clone();
        let saved_data = self.generator_data.clone();
        let saved_partial: BTreeSet<_> = self.partial_staged_bindings.keys().copied().collect();
        let saved_environment = self.effect_environment.clone();
        let saved_resume = self.effect_resume_result.clone();
        let first = self.next_temporary_binding;
        self.next_temporary_binding = first
            .checked_add(1)
            .ok_or_else(|| error(expr.span, "iterator identity space exhausted"))?;
        let receiver = n::LocalId {
            owner: u32::MAX,
            index: first,
        };
        let identity = local(expr.hir_id);
        let at = source(expr.span);
        let node = |ty, kind| n::Expr {
            source: at,
            ty,
            kind,
        };
        self.locals.insert(receiver, iterable.ty.clone());
        self.loops.push(identity);
        let checked = (|| {
            let method = self
                .selected_method(
                    iterable.ty.clone(),
                    Some(n::InterfaceInstance {
                        declaration: protocol,
                        arguments: vec![],
                    }),
                    "iterate",
                    expr.span,
                )?
                .ok_or_else(|| error(expr.span, "Iterator iterate method unavailable"))?;
            let call = self.finish_call(
                expr,
                method,
                &[],
                vec![node(iterable.ty.clone(), K::Local(receiver))],
                None,
                None,
            )?;
            let Term::Computation { effects, result } = &call.ty else {
                return Err(error(
                    expr.span,
                    "Iterator iterate did not produce a computation",
                ));
            };
            let [operation] = effects.as_slice() else {
                return Err(error(
                    expr.span,
                    "Iterator iterate requires one delivery operation",
                ));
            };
            if **result != p(P::Void) {
                return Err(error(expr.span, "Iterator iterate must complete with void"));
            }
            let operation = operation.clone();
            let metadata = self.effect_metadata(&operation, expr.span)?;
            let signature = self.formed_effect_signature(&metadata, &operation, expr.span)?;
            let Term::Function { parameters, .. } = &signature else {
                return Err(error(expr.span, "invalid Iterator delivery signature"));
            };
            let [parameter] = parameters.as_slice() else {
                return Err(error(
                    expr.span,
                    "Iterator delivery requires one Item argument",
                ));
            };
            let item_type = self.callable_input_view(&parameter.ty, expr.span)?;
            // Resume currently names the nearest handler in NIR. Do not let
            // a source resume for an outer branch target this synthetic one.
            // Explicit nested source handlers establish their own resume scope.
            self.effect_resume_result = None;
            self.immutable.extend(pattern_binding_ids(pat));
            let pattern = self.pattern(pat, &item_type)?;
            let guard = guard
                .map(|guard| self.expr(guard, Some(&p(P::Bool))))
                .transpose()?
                .map(Box::new);
            let checked = discard_result(self.block(body, None)?);
            let pattern = n::Pattern::Tuple(vec![pattern]);
            let pattern = if let Some(condition) = guard {
                n::Pattern::Guard {
                    pattern: Box::new(pattern),
                    condition,
                }
            } else {
                pattern
            };
            let mut connections: BTreeMap<_, _> = saved_locals
                .keys()
                .chain(self.loops.iter())
                .map(|id| (*id, *id))
                .collect();
            connections.extend(pattern_binding_ids(pat).into_iter().map(|id| (id, id)));
            let iteration = self.wrap_inline_iteration(
                checked,
                vec![],
                identity,
                expr.span,
                body.span,
                connections,
            )?;
            let handler = n::EffectHandler {
                operation: operation.clone(),
                abi: metadata.abi,
                signature,
                // Match directly on delivery arguments. Rejection keeps the
                // original item intact until the catch-all arm retires it;
                // successful bindings use the ordinary handler move protocol.
                arms: vec![
                    n::MatchArm {
                        pattern,
                        guard: None,
                        body: iteration,
                    },
                    n::MatchArm {
                        pattern: n::Pattern::Tuple(vec![n::Pattern::Wild]),
                        guard: None,
                        body: node(p(P::Void), K::Unit),
                    },
                ],
                fallback: None,
            };
            // The consumer executes in the outer environment. Only the
            // producer runs with the newly installed delivery operation.
            let outer = saved_environment.clone().unwrap_or_default();
            let mut installed = vec![operation];
            installed.extend_from_slice(&outer);
            self.effect_environment = Some(installed.clone());
            let running = self.run_computation(call, Some(&p(P::Void)), expr.span)?;
            self.effect_environment = saved_environment.clone();
            let handled = node(
                p(P::Void),
                K::EffectHandle {
                    computation: Box::new(node(
                        Term::computation(installed.clone(), p(P::Void)),
                        K::EffectComputation {
                            effects: installed,
                            body: Box::new(running),
                        },
                    )),
                    handlers: vec![handler],
                    outer,
                },
            );
            let looping = self.expanded_loop(
                node(
                    p(P::Never),
                    K::Block {
                        statements: vec![handled, node(p(P::Never), K::Break(identity))],
                        result: None,
                    },
                ),
                identity,
                expr.span,
            )?;
            let mut result = node(
                looping.ty.clone(),
                K::Block {
                    statements: vec![node(
                        p(P::Void),
                        K::Let {
                            binding: receiver,
                            value: Box::new(iterable),
                        },
                    )],
                    result: Some(Box::new(looping)),
                },
            );
            let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let freshened = control::freshen_loop_sites(
                &mut result,
                at,
                &mut self.expansion_sources,
                &mut fuel,
            );
            self.required_steps -= (before - fuel) as u64;
            freshened.map_err(|reason| error(expr.span, reason))?;
            if let Some(expected) = expected {
                self.require_expression(&result.ty, expected, expr.span)?;
            }
            Ok(result)
        })();
        self.loops.pop();
        self.locals = saved_locals;
        self.immutable = saved_immutable;
        self.context = saved_context;
        self.generator_data = saved_data;
        self.partial_staged_bindings
            .retain(|binding, _| saved_partial.contains(binding));
        self.effect_environment = saved_environment;
        self.effect_resume_result = saved_resume;
        checked
    }

    pub(super) fn validate_iterator_protocol(&self, id: DeclarationId) -> Result<(), Error> {
        let span = rustc_span::DUMMY_SP;
        let fail = |message: &str| error(span, message);
        let mut candidates = self
            .checked_traits
            .iter()
            .filter(|item| item.declaration == id);
        let interface = candidates
            .next()
            .ok_or_else(|| fail("Iterator binding is not a trait"))?;
        if candidates.next().is_some() || !interface.type_parameters.is_empty() {
            return Err(fail(
                "Iterator requires one trait identity without explicit parameters",
            ));
        }
        let mut items = interface
            .associated
            .iter()
            .filter(|item| item.name == "Item");
        let item = items
            .next()
            .ok_or_else(|| fail("Iterator requires an associated Item"))?;
        let mut fuel = 100_000;
        nir::type_computations::charge_term(&item.ty, &mut fuel)
            .map_err(|reason| error(span, reason.to_string()))?;
        if items.next().is_some() || item.ty != p(P::Type) {
            return Err(fail("Iterator requires one Type-valued Item"));
        }
        let mut methods = interface
            .methods
            .iter()
            .filter(|method| method.name == "iterate");
        let method = methods
            .next()
            .ok_or_else(|| fail("Iterator requires an iterate method"))?;
        if methods.next().is_some()
            || !method.by_value_receiver
            || method.by_reference_receiver
            || !method.type_parameters.is_empty()
            || !method.type_arguments.is_empty()
            || method.signature.is_comptime_callable()
            || method.is_quote
            || method.parameter_bindings.len() != 1
        {
            return Err(fail(
                "Iterator requires runtime fn iterate(self) without method parameters",
            ));
        }
        let signature = method
            .signature
            .callable_signature()
            .map_err(|_| fail("Iterator iterate must be callable"))?;
        nir::type_computations::charge_term(signature, &mut fuel)
            .map_err(|reason| error(span, reason.to_string()))?;
        let subject = Term::Rigid {
            declaration: interface.self_parameter,
            sort: Sort::Type,
        };
        let Term::Function { parameters, result } = signature else {
            return Err(fail("Iterator iterate must be callable"));
        };
        if parameters.as_slice()
            != [nir::Parameter {
                sort: Sort::Runtime,
                ty: subject.clone(),
            }]
        {
            return Err(fail(
                "Iterator iterate requires exactly one by-value Self receiver",
            ));
        }
        let Term::Computation { effects, result } = result.as_ref() else {
            return Err(fail(
                "Iterator iterate must return an element-delivery computation",
            ));
        };
        let [operation] = effects.as_slice() else {
            return Err(fail(
                "Iterator iterate requires exactly one element-delivery operation",
            ));
        };
        if **result != p(P::Void) {
            return Err(fail("Iterator iterate computation must return void"));
        }
        let metadata = self.effect_metadata(operation, span)?;
        if !metadata.inline || metadata.abi != nir::effects::Abi::Direct {
            return Err(fail(
                "Iterator element delivery requires a direct inline effect",
            ));
        }
        let signature = self.formed_effect_signature(&metadata, operation, span)?;
        let element = Term::AssociatedType {
            origin: Box::new(nir::AssociatedTypeOrigin {
                interface: id,
                member: item.declaration,
            }),
            arguments: vec![],
            subject: Box::new(subject),
        };
        let expected = Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Runtime,
                ty: element,
            }],
            result: Box::new(p(P::Void)),
        };
        if signature != expected {
            return Err(fail(
                "Iterator element delivery requires one Item argument and a void result",
            ));
        }
        Ok(())
    }
    pub(super) fn validate_into_iterator_protocol(&self, id: DeclarationId) -> Result<(), Error> {
        let span = rustc_span::DUMMY_SP;
        let fail = |message: &str| error(span, message);
        let mut candidates = self
            .checked_traits
            .iter()
            .filter(|item| item.declaration == id);
        let interface = candidates
            .next()
            .ok_or_else(|| fail("IntoIterator binding is not a trait"))?;
        if candidates.next().is_some() || !interface.type_parameters.is_empty() {
            return Err(fail(
                "IntoIterator requires one trait identity without explicit parameters",
            ));
        }
        let mut items = interface
            .associated
            .iter()
            .filter(|item| item.name == "Iter");
        let item = items
            .next()
            .ok_or_else(|| fail("IntoIterator requires an associated Iter"))?;
        let mut fuel = 100_000;
        nir::type_computations::charge_term(&item.ty, &mut fuel)
            .map_err(|reason| error(span, reason.to_string()))?;
        if items.next().is_some() || item.ty != p(P::Type) {
            return Err(fail("IntoIterator requires one Type-valued Iter"));
        }
        let mut methods = interface
            .methods
            .iter()
            .filter(|method| method.name == "into_iter");
        let method = methods
            .next()
            .ok_or_else(|| fail("IntoIterator requires an into_iter method"))?;
        if methods.next().is_some()
            || !method.by_value_receiver
            || method.by_reference_receiver
            || !method.type_parameters.is_empty()
            || !method.type_arguments.is_empty()
            || method.signature.is_comptime_callable()
            || method.is_quote
            || method.parameter_bindings.len() != 1
        {
            return Err(fail(
                "IntoIterator requires runtime fn into_iter(self) without method parameters",
            ));
        }
        let signature = method
            .signature
            .callable_signature()
            .map_err(|_| fail("IntoIterator into_iter must be callable"))?;
        nir::type_computations::charge_term(signature, &mut fuel)
            .map_err(|reason| error(span, reason.to_string()))?;
        let subject = Term::Rigid {
            declaration: interface.self_parameter,
            sort: Sort::Type,
        };
        let Term::Function { parameters, result } = signature else {
            return Err(fail("IntoIterator into_iter must be callable"));
        };
        if parameters.as_slice()
            != [nir::Parameter {
                sort: Sort::Runtime,
                ty: subject.clone(),
            }]
        {
            return Err(fail(
                "IntoIterator into_iter requires exactly one by-value Self receiver",
            ));
        }
        let expected = Term::AssociatedType {
            origin: Box::new(nir::AssociatedTypeOrigin {
                interface: id,
                member: item.declaration,
            }),
            arguments: vec![],
            subject: Box::new(subject),
        };
        if **result != expected {
            return Err(fail(
                "IntoIterator into_iter must return its associated Iter",
            ));
        }
        Ok(())
    }
}
