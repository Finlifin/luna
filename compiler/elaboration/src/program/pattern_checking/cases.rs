//! A user case computes a boolean predicate or optional payload once for the current candidate.
use super::*;

enum CaseTarget {
    Method {
        declaration: DeclarationId,
        method: n::Expr,
        extra_count: usize,
        is_declared_stable: bool,
    },
    Free {
        owner: OwnerId,
        declaration: DeclarationId,
        extra_count: usize,
        is_declared_stable: bool,
    },
}

fn case_extra_arg_count(params: &[hir::common::FnParam<'_>]) -> usize {
    params
        .iter()
        .skip(1)
        .take_while(|param| {
            !param.ty.is_implicit()
                && !param.ty.is_optional()
                && !matches!(param.ty.kind, TyParamKind::Optional(..))
        })
        .count()
}

fn pattern_as_case_arg<'hir>(pattern: &hir::pattern::Pattern<'hir>) -> Result<Arg<'hir>, Error> {
    match &pattern.kind {
        hir::pattern::PatternKind::Const(expr) => Ok(Arg::Positional(*expr)),
        _ => Err(error(
            pattern.span,
            "case pattern extra arguments must be expressions or constants (wrap variables as `<expr>`)",
        )),
    }
}

fn expr_is_dag_stable_value(
    expr: &n::Expr,
    pure_temps: &BTreeMap<n::LocalId, n::Expr>,
) -> Option<n::Expr> {
    let mut current = expr;
    while let K::Local(id) = &current.kind
        && let Some(bound) = pure_temps.get(id)
    {
        current = bound;
    }
    let kind = match &current.kind {
        K::Unit => K::Unit,
        K::Literal(lit) => K::Literal(lit.clone()),
        K::Local(id) => K::Local(*id),
        K::Tuple(elems) => {
            let resolved = elems
                .iter()
                .map(|elem| expr_is_dag_stable_value(elem, pure_temps))
                .collect::<Option<Vec<_>>>()?;
            K::Tuple(resolved)
        }
        K::Record {
            declaration,
            fields,
        } => {
            let resolved = fields
                .iter()
                .map(|value| expr_is_dag_stable_value(value, pure_temps))
                .collect::<Option<Vec<_>>>()?;
            K::Record {
                declaration: *declaration,
                fields: resolved,
            }
        }
        K::Variant {
            declaration,
            index,
            fields,
        } => {
            let resolved = fields
                .iter()
                .map(|field| expr_is_dag_stable_value(field, pure_temps))
                .collect::<Option<Vec<_>>>()?;
            K::Variant {
                declaration: *declaration,
                index: *index,
                fields: resolved,
            }
        }
        K::TypeValue(ty) => K::TypeValue(ty.clone()),
        K::Function(id) => K::Function(*id),
        K::FunctionInstance {
            declaration,
            arguments,
        } => K::FunctionInstance {
            declaration: *declaration,
            arguments: arguments.clone(),
        },
        _ => return None,
    };
    Some(n::Expr {
        ty: current.ty.clone(),
        source: current.source,
        kind,
    })
}

fn extract_stable_case_call(
    expr: &n::Expr,
    subject_binding: n::LocalId,
) -> Option<(n::Expr, Vec<n::Expr>)> {
    let mut pure_temps = BTreeMap::new();
    let mut cur = expr;
    while let K::Block { statements, result } = &cur.kind {
        for stmt in statements {
            match &stmt.kind {
                K::Let { binding, value, .. } => {
                    if let K::Local(id) = &value.kind
                        && *id == subject_binding
                    {
                        continue;
                    }
                    if let K::AddressOf(inner) = &value.kind
                        && let K::Local(id) = &inner.kind
                        && *id == subject_binding
                    {
                        continue;
                    }
                    let resolved = expr_is_dag_stable_value(value, &pure_temps)?;
                    pure_temps.insert(*binding, resolved);
                }
                _ => return None,
            }
        }
        cur = result.as_deref()?;
    }
    let K::Call {
        callee, arguments, ..
    } = &cur.kind
    else {
        return None;
    };
    if arguments.is_empty() {
        return None;
    }
    let mut extra_args = Vec::with_capacity(arguments.len().saturating_sub(1));
    for arg in arguments.iter().skip(1) {
        extra_args.push(expr_is_dag_stable_value(&arg.value, &pure_temps)?);
    }
    Some((callee.as_ref().clone(), extra_args))
}

impl Checker<'_, '_> {
    fn case_modifiers_are_stable(
        &self,
        owner: Option<OwnerId>,
        modifiers: &hir::item::FnModifiers,
    ) -> bool {
        let attr_stable = owner.is_some_and(|id| self.pattern_stable_cases.contains(&id));
        let comptime_stable =
            modifiers.is_comptime && !self.runtime_value_context() && !self.in_code_body;
        modifiers.is_pure || attr_stable || comptime_stable
    }

    fn resolve_free_case_owner(&self, expr: &Expr<'_>) -> Option<OwnerId> {
        let target_expr = match &expr.kind {
            H::NFApplication(head, _) => *head,
            _ => expr,
        };
        let def_id = match &target_expr.kind {
            H::ResolvedDef { def_id, .. } => *def_id,
            H::Projection(..) => *self.package.member_projections.get(&target_expr.hir_id)?,
            _ => return None,
        };
        let owner = self.package.definition_owner(def_id)?;
        matches!(
            self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Case(..))
        )
        .then_some(owner)
    }

    fn resolve_case_target(
        &mut self,
        head: &Expr<'_>,
        ty: &Term,
    ) -> Result<Option<CaseTarget>, Error> {
        if let H::Lit(hir::Lit {
            kind: hir::LitKind::Symbol(name),
            ..
        }) = &head.kind
        {
            if !self.source_declarations.has_case_name(*name) {
                return Ok(None);
            }
            let Some(method) = self.selected_method(ty.clone(), None, name.as_str(), head.span)?
            else {
                return Ok(None);
            };
            let id = match &method.kind {
                K::Function(id)
                | K::FunctionInstance {
                    declaration: id, ..
                } => *id,
                _ => return Ok(None),
            };
            let Some(item) = self.source_declarations.item(id) else {
                return Ok(None);
            };
            let ItemKind::Case(sig, _) = &item.kind else {
                return Ok(None);
            };
            if sig.params.is_empty() {
                return Err(error(head.span, "case pattern needs a subject parameter"));
            }
            let extra_count = case_extra_arg_count(sig.params);
            let owner = self.source_declarations.owner(id);
            let is_declared_stable = self.case_modifiers_are_stable(owner, &sig.modifiers);
            return Ok(Some(CaseTarget::Method {
                declaration: id,
                method,
                extra_count,
                is_declared_stable,
            }));
        }

        let Some(owner) = self.resolve_free_case_owner(head) else {
            return Ok(None);
        };
        let item = self.package.item(owner).expect("verified case owner");
        let ItemKind::Case(sig, _) = &item.kind else {
            return Ok(None);
        };
        if sig.params.is_empty() {
            return Err(error(head.span, "case pattern needs a subject parameter"));
        }
        let extra_count = case_extra_arg_count(sig.params);
        let decl = declaration(self.package, &self.package_identities, owner);
        let is_declared_stable = self.case_modifiers_are_stable(Some(owner), &sig.modifiers);
        Ok(Some(CaseTarget::Free {
            owner,
            declaration: decl,
            extra_count,
            is_declared_stable,
        }))
    }

    pub(super) fn case_pattern(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        head: &Expr<'_>,
        fields: Option<&[hir::pattern::Pattern<'_>]>,
        ty: &Term,
    ) -> Result<Option<n::Pattern>, Error> {
        let Some(target) = self.resolve_case_target(head, ty)? else {
            return Ok(None);
        };

        let (decl_id, extra_count, is_declared_stable) = match &target {
            CaseTarget::Method {
                declaration,
                extra_count,
                is_declared_stable,
                ..
            } => (*declaration, *extra_count, *is_declared_stable),
            CaseTarget::Free {
                declaration,
                extra_count,
                is_declared_stable,
                ..
            } => (*declaration, *extra_count, *is_declared_stable),
        };

        let (extra_arg_pats, payload_fields): (
            &[hir::pattern::Pattern<'_>],
            Option<&[hir::pattern::Pattern<'_>]>,
        ) = match fields {
            None => {
                if extra_count > 0 {
                    return Err(error(
                        pattern.span,
                        "case pattern missing required arguments",
                    ));
                }
                (&[], None)
            }
            Some(all_fields) => {
                if all_fields.len() < extra_count {
                    return Err(error(
                        pattern.span,
                        "case pattern missing required arguments",
                    ));
                }
                (&all_fields[..extra_count], Some(&all_fields[extra_count..]))
            }
        };

        let mut extra_args = Vec::with_capacity(extra_arg_pats.len());
        for arg_pat in extra_arg_pats {
            extra_args.push(pattern_as_case_arg(arg_pat)?);
        }

        // Determine subject parameter passing convention (`self` vs `*self`).
        let first_param_ty = match &target {
            CaseTarget::Method { method, .. } => {
                let Term::Function { parameters, .. } = method.ty.callable_signature() else {
                    return Err(error(head.span, "case has no callable signature"));
                };
                parameters
                    .first()
                    .ok_or_else(|| error(head.span, "case pattern needs a subject parameter"))?
                    .ty
                    .clone()
            }
            CaseTarget::Free { owner, .. } => {
                self.ensure_signature(*owner, head.span)?;
                let sig_ty = self
                    .signatures
                    .get(owner)
                    .ok_or_else(|| error(head.span, "case has no callable signature"))?;
                let Term::Function { parameters, .. } = sig_ty.callable_signature() else {
                    return Err(error(head.span, "case has no callable signature"));
                };
                parameters
                    .first()
                    .ok_or_else(|| error(head.span, "case pattern needs a subject parameter"))?
                    .ty
                    .clone()
            }
        };
        let by_address = matches!(&first_param_ty, Term::Pointer(_)) && &first_param_ty != ty;

        let binding = n::LocalId {
            owner: u32::MAX,
            index: self.next_temporary_binding,
        };
        self.next_temporary_binding = self
            .next_temporary_binding
            .checked_add(1)
            .ok_or_else(|| error(pattern.span, "pattern temporary binding space exhausted"))?;
        let subject = n::Expr {
            ty: ty.clone(),
            source: source(pattern.span),
            kind: K::Local(binding),
        };
        let argument = if by_address {
            n::Expr {
                ty: Term::Pointer(Box::new(ty.clone())),
                source: subject.source,
                kind: K::AddressOf(Box::new(subject)),
            }
        } else {
            subject
        };

        self.locals.insert(binding, ty.clone());
        let delivery = match target {
            CaseTarget::Method { method, .. } => {
                self.finish_call(head, method, &extra_args, vec![argument], None, None)
            }
            CaseTarget::Free { owner, .. } => {
                if let H::NFApplication(nf_head, nf_args) = &head.kind {
                    self.ensure_signature(owner, head.span)?;
                    match self.select_explicit_instance(head, nf_head, nf_args, None, true) {
                        Ok(block_regions::ExplicitInstance::Ready(instantiated)) => self
                            .finish_planned_prefix_call(
                                head,
                                instantiated,
                                &extra_args,
                                vec![argument],
                                None,
                                None,
                            ),
                        Ok(block_regions::ExplicitInstance::Checking(_)) => Err(error(
                            head.span,
                            "case pattern cannot infer active checking region",
                        )),
                        Err(err) => Err(err),
                    }
                } else {
                    self.ensure_signature(owner, head.span)?;
                    let has_type_params = self
                        .type_parameters
                        .get(&owner)
                        .is_some_and(|params| !params.is_empty());
                    if has_type_params {
                        let known = self.lexical_function_arguments(owner, head.span)?;
                        self.generic_call_with_receiver(
                            head,
                            owner,
                            &extra_args,
                            None,
                            None,
                            vec![argument],
                            known,
                            None,
                        )
                    } else {
                        let sig_ty = self
                            .signatures
                            .get(&owner)
                            .ok_or_else(|| {
                                error(head.span, "declaration is not a supported callable")
                            })?
                            .clone();
                        let callee_ty = self.callable_type(owner, sig_ty);
                        self.check_type_application(owner, &[], head.span)?;
                        let callee_expr = n::Expr {
                            ty: callee_ty,
                            source: source(head.span),
                            kind: if self.requirements.get(&owner).is_some_and(Vec::is_empty) {
                                K::Function(decl_id)
                            } else {
                                K::FunctionInstance {
                                    declaration: decl_id,
                                    arguments: vec![],
                                }
                            },
                        };
                        self.finish_planned_prefix_call(
                            head,
                            callee_expr,
                            &extra_args,
                            vec![argument],
                            None,
                            None,
                        )
                    }
                }
            }
        };
        self.locals.remove(&binding);
        let value = delivery?;

        let result_structural = self.structural_type(&value.ty, pattern.span)?;
        let (is_predicate, payload_ty, inner_payload_pattern, next_pat) = if result_structural
            == p(P::Bool)
        {
            if let Some(rest) = payload_fields
                && !rest.is_empty()
            {
                return Err(error(
                    pattern.span,
                    "predicate case pattern takes no payload bindings",
                ));
            }
            (
                true,
                Term::Tuple(Vec::new()),
                n::Pattern::Tuple(Vec::new()),
                n::Pattern::Literal(n::Literal::Bool(true)),
            )
        } else {
            let payload = self.optional_payload(&value.ty, pattern.span)?;
            let element_types = self.structural_type(&payload, pattern.span)?;
            let inner = match payload_fields {
                None => n::Pattern::Wild,
                Some([]) => {
                    if element_types == p(P::Void) {
                        n::Pattern::Unit
                    } else if matches!(&element_types, Term::Tuple(elements) if elements.is_empty())
                    {
                        n::Pattern::Tuple(Vec::new())
                    } else {
                        return Err(error(pattern.span, "case pattern payload arity mismatch"));
                    }
                }
                Some(rem) => match &element_types {
                    Term::Tuple(elements) if elements.len() == rem.len() => {
                        let children = rem
                            .iter()
                            .zip(elements)
                            .map(|(pat, elem_ty)| self.pattern(pat, elem_ty))
                            .collect::<Result<Vec<_>, _>>()?;
                        n::Pattern::Tuple(children)
                    }
                    Term::Tuple(_) => {
                        return Err(error(pattern.span, "case pattern payload arity mismatch"));
                    }
                    void_ty if void_ty == &p(P::Void) => {
                        return Err(error(pattern.span, "case pattern payload arity mismatch"));
                    }
                    _ if rem.len() == 1 => self.pattern(&rem[0], &payload)?,
                    _ => {
                        return Err(error(
                            pattern.span,
                            "destructuring case must return an optional tuple",
                        ));
                    }
                },
            };
            (
                false,
                payload,
                inner.clone(),
                n::Pattern::OptionalPresent(Box::new(inner)),
            )
        };

        if is_declared_stable
            && let Some((checked_callee, checked_extra_args)) =
                extract_stable_case_call(&value, binding)
        {
            self.case_pattern_infos.insert(
                binding,
                crate::automata::CasePatternInfo {
                    declaration: decl_id,
                    binding,
                    callee: checked_callee,
                    extra_args: checked_extra_args,
                    is_predicate,
                    payload_type: payload_ty,
                    next_pattern: inner_payload_pattern,
                },
            );
        }

        Ok(Some(n::Pattern::Observe {
            binding,
            inner: Box::new(n::Pattern::Then {
                first: Box::new(n::Pattern::Wild),
                value: Box::new(value),
                next: Box::new(next_pat),
            }),
        }))
    }
}
