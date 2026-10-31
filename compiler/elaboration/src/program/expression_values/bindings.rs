//! Local, declaration and callable values.
use super::*;

impl Checker<'_, '_> {
    #[inline(never)]
    pub(super) fn bindings_value_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let (ty, kind) = match &expr.kind {
            H::SelfValue => {
                let item = self
                    .package
                    .item(expr.hir_id.owner)
                    .ok_or_else(|| error(expr.span, "self requires a method receiver"))?;
                let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) = &item.kind else {
                    return Err(error(expr.span, "self requires a method receiver"));
                };
                let param = sig
                    .params
                    .first()
                    .filter(|p| matches!(p.ty.kind, TyParamKind::Itself { .. }))
                    .ok_or_else(|| error(expr.span, "self requires a method receiver"))?;
                let resolved = Expr {
                    kind: H::ResolvedFnParam(param.name.name, param.hir_id),
                    ..expr.clone()
                };
                return self.expr_context(&resolved, expected, partial);
            }
            H::TacticArgument(value) => return self.tactic_argument(value, expected, partial),
            H::PostLambdaBody(body) => {
                return self.closure(expr, &[], None, *body, expected, partial);
            }
            H::TacticParameter(index) => {
                let id = self.tactic_parameter(expr, *index)?;
                let resolved = Expr {
                    kind: H::ResolvedLocal(hir::Symbol::intern("$tactic"), id),
                    ..expr.clone()
                };
                return self.expr_context(&resolved, expected, partial);
            }
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                if let Some((symbol, domain)) = self.captured_staged_input(*id, expr.span)? {
                    if let Some(expected) = expected {
                        self.require_expression(&domain, expected, expr.span)?;
                    }
                    let kind = if domain == p(P::Type) {
                        K::TypeValue(symbol)
                    } else {
                        K::ComptimeValue(symbol)
                    };
                    return Ok(n::Expr {
                        source: source(expr.span),
                        ty: domain,
                        kind,
                    });
                }
                let source_binding = *id;
                let id = local(*id);
                if self.in_code_body && !self.checking_code_generator {
                    let live_receiver = self
                        .generator_data
                        .get(&id)
                        .and_then(Option::as_ref)
                        .filter(|value| matches!(value.kind, K::Local(binding) if binding == id))
                        .and_then(|value| {
                            self.generator_cells
                                .get(&id)
                                .filter(|(_, cell)| matches!(cell, GeneratorCell::Reference(_)))
                                .map(|(_, cell)| (value.ty.clone(), cell.clone()))
                        });
                    if let Some((ty, cell)) = live_receiver {
                        // Quoting observes the receiver now; its invocation-local
                        // reference is never captured by generated runtime code.
                        let data = self.read_generator_cell(&cell, expr.span)?;
                        let value = self.materialize_generator_data(data, ty, expr.span)?;
                        if let Some(expected) = expected {
                            self.require_expression(&value.ty, expected, expr.span)?;
                        }
                        return Ok(value);
                    }
                }
                if let Some(Some(value)) = self.generator_data.get(&id) {
                    constants::charge_copy(value, &mut self.required_steps, expr.span)?;
                    let mut value = value.clone();
                    if matches!(value.kind, K::Block { .. }) {
                        self.freshen_constant_graph(&mut value, expr.span)?;
                    }
                    if let Some(expected) = expected {
                        self.require_expression(&value.ty, expected, expr.span)?;
                    }
                    return Ok(value);
                }
                if self.code_values.contains_key(&id) {
                    return Err(error(
                        expr.span,
                        "quoted code cannot be delivered as a runtime value; use a code splice",
                    ));
                }
                if self.in_required_comptime && self.required_runtime_parameter(id) {
                    return Err(error(
                        expr.span,
                        "required comptime computation cannot read a runtime parameter; defer does not change the parameter's phase",
                    ));
                }
                let ty = self
                    .locals
                    .get(&id)
                    .cloned()
                    .or_else(|| self.signature_comptime_input(id).map(|(_, ty)| ty))
                    .ok_or_else(|| {
                        error(expr.span, self.unavailable_lexical_input(source_binding))
                    })?;
                // Concrete open instances already own these staged Type
                // inputs. Preserve the immutable identity for type observations
                // instead of rebuilding a runtime-looking local read.
                if ty == p(P::Type)
                    && !self.old_scope.as_ref().is_some_and(|scope| scope.active)
                    && let Some((_, _, symbol)) = self
                        .specialization_inputs
                        .iter()
                        .find(|(binding, _, _)| *binding == id)
                {
                    if let Some(expected) = expected {
                        self.require_expression(&ty, expected, expr.span)?;
                    }
                    return Ok(n::Expr {
                        source: source(expr.span),
                        ty,
                        kind: K::TypeValue(symbol.clone()),
                    });
                }
                // An ordinary Type input is already supplied during staging.
                // Runtime lambdas refer to its symbolic type, not a Type-valued
                // environment cell; substitution closes it with the caller.
                if (self.captures.last().is_some_and(|frame| !frame.comptime)
                    || (self.captures.is_empty()
                        && !self.in_staged_input
                        && !self.in_required_comptime
                        && self
                            .signature_owner
                            .is_some_and(|owner| self.open_functions.contains_key(&owner))))
                    && !self.old_scope.as_ref().is_some_and(|scope| scope.active)
                    && let Some(symbol) = self.type_input_bindings.get(&id)
                {
                    if let Some(expected) = expected {
                        self.require_expression(&ty, expected, expr.span)?;
                    }
                    return Ok(n::Expr {
                        source: source(expr.span),
                        ty,
                        kind: K::TypeValue(symbol.clone()),
                    });
                }
                if self.in_staged_input
                    && self.captures.last().is_some_and(|frame| !frame.comptime)
                    && self.value_input_bindings.contains_key(&id)
                    && !self.old_scope.as_ref().is_some_and(|scope| scope.active)
                {
                    // This use disappears when the enclosing staged input is
                    // substituted. A runtime use still takes the capture path.
                    if let Some(expected) = expected {
                        self.require_expression(&ty, expected, expr.span)?;
                    }
                    return Ok(n::Expr {
                        source: source(expr.span),
                        ty,
                        kind: K::Local(id),
                    });
                }
                let mut id = id;
                if self
                    .old_scope
                    .as_ref()
                    .is_some_and(|scope| scope.active && Some(scope.owner) == self.signature_owner)
                {
                    if !matches!(expr.kind, H::ResolvedFnParam(..)) {
                        return Err(error(
                            expr.span,
                            "old requires an entry parameter, not an exit result or local",
                        ));
                    }
                    let scope = self.old_scope.as_mut().unwrap();
                    id = if let Some(binding) = scope.bindings.get(&id) {
                        *binding
                    } else {
                        let index = self.next_temporary_binding;
                        self.next_temporary_binding = index.checked_add(1).ok_or_else(|| {
                            error(expr.span, "entry snapshot binding limit exceeded")
                        })?;
                        let binding = n::LocalId {
                            owner: u32::MAX,
                            index,
                        };
                        scope.bindings.insert(id, binding);
                        self.locals.insert(binding, ty.clone());
                        binding
                    };
                }
                for frame in &mut self.captures {
                    if frame.outer.contains(&id)
                        && !frame.values.iter().any(|capture| capture.binding == id)
                    {
                        frame.values.push(n::Capture {
                            value: None,
                            binding: id,
                            ty: ty.clone(),
                        });
                    }
                }
                (ty, K::Local(id))
            }
            H::NFApplication(callee, args) => {
                return self.explicit_function_instance(expr, callee, args, expected, false);
            }
            H::ResolvedDef { def_id, .. } => {
                let owner = self.package.definition_owner(*def_id).ok_or_else(|| {
                    error(expr.span, "resolved function declaration has no HIR owner")
                })?;
                if matches!(
                    self.package.item(owner).map(|i| &i.kind),
                    Some(ItemKind::Associated(_))
                ) {
                    let value = self.lexical_associated_value(owner, expr.span)?;
                    if let Some(expected) = expected {
                        self.require(&value.ty, expected, expr.span)?;
                    }
                    return Ok(value);
                }
                if self.is_build_context(owner) {
                    let value = self.build_context_value(expr, owner)?;
                    if self.runtime_value_context() {
                        return self.splice_value(expr, value, expected);
                    }
                    if let Some(expected) = expected {
                        self.require(&value.ty, expected, expr.span)?;
                    }
                    return Ok(value);
                }
                if matches!(
                    self.package.item(owner).map(|i| &i.kind),
                    Some(ItemKind::Const(..))
                ) {
                    if self.runtime_value_context() {
                        return self.splice_constant(expr, owner, expected);
                    }
                    let mut value = self.constant_value(owner, expr.span)?;
                    value.source = source(expr.span);
                    if let Some(expected) = expected {
                        self.require(&value.ty, expected, expr.span)?;
                    }
                    return Ok(value);
                }
                if matches!(&self.package.item(owner).unwrap().kind,
                    ItemKind::Fn(signature, _) if signature.modifiers.is_normal_form_head)
                {
                    return self.normal_form_value(expr, owner, expected);
                }
                self.require_non_trait_body_reference(owner, expr.span)?;
                self.ensure_signature(owner, expr.span)?;
                if self
                    .type_parameters
                    .get(&owner)
                    .is_some_and(|types| !types.is_empty())
                {
                    return self.contextual_function_instance(expr, owner, expected, partial);
                }
                if let Some(item) = self.package.item(owner) {
                    if let ItemKind::Fn(signature, _) = &item.kind {
                        arguments::require_function_value_parameters(signature.params, expr.span)?;
                    }
                }
                let ty = self
                    .known_callable_signature(owner)
                    .ok_or_else(|| error(expr.span, "declaration is not a supported callable"))?;
                let ty = self.callable_type(owner, ty);
                self.check_type_application(owner, &[], expr.span)?;
                (
                    ty,
                    if self.requirements.get(&owner).is_some_and(Vec::is_empty) {
                        K::Function(declaration(self.package, &self.package_identities, owner))
                    } else {
                        K::FunctionInstance {
                            declaration: declaration(self.package, &self.package_identities, owner),
                            arguments: vec![],
                        }
                    },
                )
            }
            H::Closure(parameters, annotation, body) => {
                return self.closure(expr, parameters, *annotation, *body, expected, partial);
            }
            H::Item(owner) => {
                // Declaration owners are checked by the package-wide declaration
                // and body passes, including owners nested in a block. Their
                // lexical occurrence does not evaluate a runtime value.
                let item = self.package.item(*owner).ok_or_else(|| {
                    error(expr.span, "block declaration refers to a missing owner")
                })?;
                if !matches!(item.kind, ItemKind::Use(_))
                    && self.package.owner_definition(*owner).is_none()
                {
                    return Err(error(
                        expr.span,
                        "block declaration requires a scanned identity",
                    ));
                }
                (p(P::Void), K::Unit)
            }
            _ => unreachable!("value expression category was checked by the dispatcher"),
        };
        self.finish_value_expression(expr, expected, ty, kind)
    }
}
