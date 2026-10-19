//! Source effect formation, requests and direct handler installations.
//! Like other type applications, obligations
//! join the caller's pending formation transaction before NIR publication.
use super::*;

mod handlers;
mod inference;

impl Checker<'_, '_> {
    pub(super) fn effect_row(
        &mut self,
        expr: &Expr<'_>,
        depth: usize,
    ) -> Result<Vec<nir::effects::Operation>, Error> {
        self.effect_row_inner(expr, depth, &mut 100_000)
    }

    fn effect_row_inner(
        &mut self,
        expr: &Expr<'_>,
        depth: usize,
        fuel: &mut usize,
    ) -> Result<Vec<nir::effects::Operation>, Error> {
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(expr.span, "effect row formation budget exhausted"))?;
        if depth >= 64 {
            return Err(error(expr.span, "effect row formation depth exhausted"));
        }
        if let H::List(items) = &expr.kind {
            if items.len() > 4096 {
                return Err(error(expr.span, "effect row formation width exhausted"));
            }
            let mut result = Vec::new();
            for item in *items {
                let operations = self.effect_row_inner(item, depth + 1, fuel)?;
                if result.len().saturating_add(operations.len()) > 4096 {
                    return Err(error(expr.span, "effect row formation width exhausted"));
                }
                result.extend(operations);
            }
            return Ok(result);
        }
        let (head, inputs) = if let H::NFApplication(head, inputs) = &expr.kind {
            (*head, *inputs)
        } else {
            (expr, &[][..])
        };
        if inputs.len() > *fuel {
            return Err(error(expr.span, "effect row formation budget exhausted"));
        }
        let mut arguments = Vec::new();
        for input in inputs {
            let Arg::Positional(value) = input else {
                return Err(error(
                    expr.span,
                    "effect instance requires explicit positional Type arguments",
                ));
            };
            let argument = self.type_expr(value)?;
            crate::associated::charge_projection_copy(&argument, fuel)
                .map_err(|message| error(expr.span, message))?;
            arguments.push(argument);
        }
        let owner = if let H::Projection(group, member) = &head.kind {
            if !arguments.is_empty() {
                return Err(error(
                    expr.span,
                    "effect group operation arguments belong on the group instance",
                ));
            }
            let (group, group_inputs) = if let H::NFApplication(group, inputs) = &group.kind {
                (*group, *inputs)
            } else {
                (*group, &[][..])
            };
            let group_owner = self.effect_head_owner(group)?;
            let Some(ItemKind::EffectGroup(definition)) =
                self.package.item(group_owner).map(|item| &item.kind)
            else {
                return Err(error(
                    group.span,
                    "effect projection requires an effect group",
                ));
            };
            let owner = definition
                .operations
                .iter()
                .copied()
                .find(|owner| {
                    self.package
                        .item(*owner)
                        .is_some_and(|item| item.ident.name == member.name)
                })
                .ok_or_else(|| error(expr.span, "unknown effect group operation"))?;
            if group_inputs.len() > *fuel {
                return Err(error(expr.span, "effect row formation budget exhausted"));
            }
            for input in group_inputs {
                let Arg::Positional(value) = input else {
                    return Err(error(
                        expr.span,
                        "effect group requires explicit positional Type arguments",
                    ));
                };
                let argument = self.type_expr(value)?;
                crate::associated::charge_projection_copy(&argument, fuel)
                    .map_err(|message| error(expr.span, message))?;
                arguments.push(argument);
            }
            owner
        } else {
            self.effect_head_owner(head)?
        };
        let item = self
            .package
            .item(owner)
            .ok_or_else(|| error(expr.span, "missing effect declaration"))?;
        let operations = match &item.kind {
            ItemKind::EffectGroup(group) => {
                if self.type_parameters[&owner].len() != arguments.len() {
                    return Err(error(
                        expr.span,
                        "effect group Type argument count mismatch",
                    ));
                }
                self.ensure_contract(owner, expr.span)?;
                self.check_type_application(owner, &arguments, expr.span)?;
                if group.operations.len() > 4096 || group.operations.len() > *fuel {
                    return Err(error(expr.span, "effect row formation width exhausted"));
                }
                group.operations.clone()
            }
            ItemKind::Effect(..) => vec![owner],
            _ => {
                return Err(error(
                    expr.span,
                    "effect row entry is not an effect operation or group",
                ));
            }
        };
        if operations.len() > 4096 {
            return Err(error(expr.span, "effect row formation width exhausted"));
        }
        let mut result = Vec::new();
        for operation in operations {
            // A recursive occurrence label does not depend on the result type
            // currently being formed. Its telescope has already been installed.
            if !self.active_signatures.contains(&operation) {
                self.ensure_signature(operation, expr.span)?;
            }
            if self.type_parameters[&operation].len() != arguments.len() {
                return Err(error(
                    expr.span,
                    "effect operation Type argument count mismatch",
                ));
            }
            self.ensure_contract(operation, expr.span)?;
            self.check_type_application(operation, &arguments, expr.span)?;
            *fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| error(expr.span, "effect row formation budget exhausted"))?;
            for argument in &arguments {
                crate::associated::charge_projection_copy(argument, fuel)
                    .map_err(|message| error(expr.span, message))?;
            }
            result.push(nir::effects::Operation {
                declaration: declaration(self.package, &self.package_identities, operation),
                arguments: arguments.clone(),
            });
        }
        Ok(result)
    }

    fn effect_head_owner(&self, expr: &Expr<'_>) -> Result<OwnerId, Error> {
        let H::ResolvedDef { def_id, .. } = &expr.kind else {
            return Err(error(
                expr.span,
                "effect row requires a resolved operation or group; open rows are not yet implemented",
            ));
        };
        self.package
            .definition_owner(*def_id)
            .ok_or_else(|| error(expr.span, "effect declaration has no HIR owner"))
    }
}

impl Checker<'_, '_> {
    pub(super) fn effect_request(
        &mut self,
        expr: &Expr<'_>,
        call: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if self.in_required_comptime || self.signature_owner.and_then(|owner| self.package.item(owner)).is_some_and(|item| matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) if sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head)) {
            return Err(error(expr.span, "comptime effect requests are not implemented"));
        }
        let H::Application(head, arguments) = &call.kind else {
            return Err(error(
                expr.span,
                "effect request requires an operation application",
            ));
        };
        let Some(allowed) = self.active_effect_environment() else {
            return Err(error(
                expr.span,
                "effect request requires an explicit allowed effect row",
            ));
        };
        let allowed = allowed.to_vec();
        let declaration_head = if let H::NFApplication(base, _) = &head.kind {
            *base
        } else {
            *head
        };
        if let Ok(owner) = self.effect_head_owner(declaration_head) {
            if matches!(
                self.package.item(owner).map(|item| &item.kind),
                Some(ItemKind::EffectGroup(_))
            ) {
                return Err(error(
                    expr.span,
                    "effect group is not an operation; select a member",
                ));
            }
        }
        let structural_result = self
            .structural_result_context
            .as_ref()
            .filter(|(id, _)| *id == expr.hir_id)
            .map(|(_, context)| context.clone());
        let inferred = self.infer_effect_request(
            head,
            arguments,
            expected,
            structural_result.as_ref(),
            expr.span,
        )?;
        let (operation, inferred_arguments) = if let Some((operation, checked)) = inferred {
            (operation, Some(checked))
        } else {
            let operations = self.effect_row(head, 0)?;
            if operations.len() != 1 {
                return Err(error(
                    expr.span,
                    "effect request requires one independent operation",
                ));
            }
            (operations.into_iter().next().unwrap(), None)
        };
        let operation = &operation;
        let metadata = self.effect_metadata(operation, expr.span)?;
        let signature = self.formed_effect_signature(&metadata, operation, expr.span)?;
        let Term::Function { parameters, result } = &signature else {
            return Err(error(expr.span, "invalid effect signature"));
        };
        if parameters.len() != arguments.len() {
            return Err(error(expr.span, "effect request argument count mismatch"));
        }
        // Source runtime-dependent type formation has its own identity routing;
        // never invent a binding identity when that source path is unavailable.
        if result.check_scope(&[]).is_err()
            || parameters.iter().any(|p| p.ty.check_scope(&[]).is_err())
        {
            return Err(error(
                expr.span,
                "runtime-dependent effect request source lowering is not yet implemented",
            ));
        }
        let checked = if let Some(checked) = inferred_arguments {
            checked
        } else {
            let mut checked = Vec::new();
            for (argument, parameter) in arguments.iter().zip(parameters) {
                let Arg::Positional(value) = argument else {
                    return Err(error(
                        expr.span,
                        "effect request requires positional arguments",
                    ));
                };
                checked.push(self.expr(value, Some(&parameter.ty))?);
            }
            checked
        };
        let symbolic: Vec<_> = checked
            .iter()
            .enumerate()
            .map(|(index, value)| crate::calls::Argument {
                value: Term::Runtime(nir::RuntimeId {
                    instance: u64::from(expr.hir_id.owner.def_id.raw()),
                    local: index as u32,
                }),
                // A diverging input never reaches the request. Its source IR keeps
                // Never; the (unreachable) completed input has the expected type.
                ty: if is_never(&value.ty) {
                    parameters[index].ty.clone()
                } else {
                    value.ty.clone()
                },
            })
            .collect();
        let assumptions = self
            .signature_owner
            .and_then(|owner| self.capabilities.get(&owner))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let world = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions,
            context: &self.context,
        };
        let instance = crate::effects::instantiate(
            &metadata,
            &operation.arguments,
            &world,
            crate::ptc::Budget::default(),
        )
        .map_err(|failure| error(expr.span, failure.to_string()))?;
        let request = instance
            .request(&symbolic, &allowed, 0, crate::ptc::Budget::default())
            .map_err(|failure| error(expr.span, failure.to_string()))?;
        let body = n::Expr {
            source: source(expr.span),
            ty: request.result().clone(),
            kind: K::EffectRequest {
                operation: operation.clone(),
                abi: request.abi(),
                allowed_index: request.allowed_index(),
                signature: Box::new(signature),
                arguments: checked,
            },
        };
        if let Some(expected) = expected {
            self.require_expression(&body.ty, expected, expr.span)?;
        }
        Ok(body)
    }
}

impl Checker<'_, '_> {
    pub(super) fn active_effect_environment(&self) -> Option<&[nir::effects::Operation]> {
        self.effect_environment.as_deref()
    }
    pub(super) fn computation_body(
        &mut self,
        body: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        self.computation_body_with_input(body, expected, None)
    }
    pub(super) fn computation_body_with_input(
        &mut self,
        body: &Expr<'_>,
        expected: Option<&Term>,
        input: Option<&crate::calls::InputContext>,
    ) -> Result<n::Expr, Error> {
        let (effects, value_type) = match expected {
            Some(Term::Computation { effects, result }) => {
                (Some(effects.clone()), Some(result.as_ref()))
            }
            other => (None, other),
        };
        let saved_resume = self.effect_resume_result.take();
        let saved_environment = std::mem::replace(&mut self.effect_environment, effects.clone());
        let saved_result = std::mem::replace(&mut self.result, value_type.cloned());
        let saved_partial_return = std::mem::replace(
            &mut self.partial_return_context,
            if value_type.is_none() {
                input.cloned()
            } else {
                None
            },
        );
        let checked = match (value_type, input) {
            (None, Some(input)) => self.input_context_expression(body, input),
            _ => self.expr(body, value_type),
        };
        self.result = saved_result;
        self.partial_return_context = saved_partial_return;

        self.effect_resume_result = saved_resume;
        self.effect_environment = saved_environment;
        let checked = checked?;
        Ok(if let Some(effects) = effects {
            n::Expr {
                source: source(body.span),
                ty: expected.unwrap().clone(),
                kind: K::EffectComputation {
                    effects,
                    body: Box::new(checked),
                },
            }
        } else {
            checked
        })
    }
    pub(super) fn require_expression(
        &self,
        actual: &Term,
        expected: &Term,
        span: Span,
    ) -> Result<(), Error> {
        if self.active_effect_environment().is_some()
            && !matches!(expected, Term::Computation { .. })
        {
            if let Term::Computation { effects, result } = actual {
                crate::effects::embed_row(
                    effects,
                    self.active_effect_environment().unwrap(),
                    &self.context,
                    crate::ptc::Budget::default(),
                )
                .map_err(|failure| error(span, failure.to_string()))?;
                return self.require(result, expected, span);
            }
        }
        self.require(actual, expected, span)
    }
    pub(super) fn run_computation(
        &self,
        value: n::Expr,
        expected: Option<&Term>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let Some(allowed) = self.active_effect_environment() else {
            return Ok(value);
        };
        if matches!(expected, Some(Term::Computation { .. })) {
            return Ok(value);
        }
        let Term::Computation { effects, result } = &value.ty else {
            return Ok(value);
        };
        let mapping = crate::effects::embed_row(
            effects,
            allowed,
            &self.context,
            crate::ptc::Budget::default(),
        )
        .map_err(|failure| error(span, failure.to_string()))?;
        let slots = (0..effects.len())
            .map(|i| mapping.target_index(i).unwrap())
            .collect();
        let ty = result.as_ref().clone();
        if let Some(expected) = expected {
            self.require(&ty, expected, span)?;
        }
        Ok(n::Expr {
            source: value.source,
            ty,
            kind: K::EffectRun {
                computation: Box::new(value),
                slots,
            },
        })
    }
}

impl Checker<'_, '_> {
    fn effect_metadata(
        &self,
        operation: &nir::effects::Operation,
        span: Span,
    ) -> Result<nir::effects::Declaration, Error> {
        let owner = self
            .package
            .owners()
            .find_map(|(owner, info)| {
                (matches!(info.node.expect_item().kind, ItemKind::Effect(..))
                    && matches_declaration(
                        self.package,
                        &self.package_identities,
                        owner,
                        operation.declaration,
                    ))
                .then_some(owner)
            })
            .ok_or_else(|| error(span, "missing effect operation metadata"))?;
        // Validate the copy before constructing the owned formation descriptor.
        crate::associated::charge_projection_copy(&self.signatures[&owner], &mut 100_000)
            .map_err(|message| error(span, message))?;
        let ItemKind::Effect(_, modifiers) = &self.package.item(owner).unwrap().kind else {
            unreachable!()
        };
        Ok(nir::effects::Declaration {
            declaration: operation.declaration,
            group: self
                .effect_parents
                .get(&owner)
                .map(|group| declaration(self.package, &self.package_identities, *group)),
            name: self.package.item(owner).unwrap().ident.name.as_str().into(),
            type_parameters: self.type_parameters[&owner].clone(),
            signature: self.signatures[&owner].clone(),
            abi: if modifiers.is_direct_passing {
                nir::effects::Abi::DirectPassing
            } else if modifiers.is_async {
                nir::effects::Abi::Async
            } else {
                nir::effects::Abi::Direct
            },
            inline: modifiers.is_inline,
            requirements: self.requirements[&owner].clone(),
            capabilities: self.capabilities.get(&owner).cloned().unwrap_or_default(),
        })
    }

    fn formed_effect_signature(
        &self,
        metadata: &nir::effects::Declaration,
        operation: &nir::effects::Operation,
        span: Span,
    ) -> Result<Term, Error> {
        let assumptions = self
            .signature_owner
            .and_then(|owner| self.capabilities.get(&owner))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let world = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions,
            context: &self.context,
        };
        let instance = crate::effects::instantiate(
            metadata,
            &operation.arguments,
            &world,
            crate::ptc::Budget::default(),
        )
        .map_err(|failure| error(span, failure.to_string()))?;
        Ok(instance.signature().clone())
    }
}
