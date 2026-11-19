//! Declaration-owned checking and publication of module and member constants.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn constant_value(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let item = self.package.item(owner).unwrap();
        let associated = self.initializer_bindings(owner).is_some();
        let parent = self.self_containers.get(&owner).copied().filter(|parent| {
            matches!(item.kind, ItemKind::Const(..))
                && matches!(
                    self.package.item(*parent).map(|item| &item.kind),
                    Some(ItemKind::Struct(_) | ItemKind::Enum(_))
                )
                && (self
                    .type_parameters
                    .get(parent)
                    .is_some_and(|inputs| !inputs.is_empty())
                    || !self.nominal_capture_ids(*parent).is_empty())
        });
        // The selected implementation owns its own cache and recursion key.
        // A projected member can retain its container's symbolic bindings.
        let closed = !associated && parent.is_none();
        if closed {
            // These values name declarations. Looking up an operation also
            // makes its function body available in the current generated world.
            if let Some(value) = self.builtin_normal_form_constant(owner, span)? {
                return Ok(value);
            }
            if let Some(value) = self.compiler_operation_value(owner, span)? {
                return Ok(value);
            }
        }
        if closed && let Some(value) = self.constants.get(&owner) {
            charge_copy(value, &mut self.required_steps, span)?;
            return Ok(value.clone());
        }
        if !associated
            && (self.active_constants.len() >= 64 || !self.active_constants.insert(owner))
        {
            return Err(error(
                span,
                "cyclic or excessively deep constant dependency",
            ));
        }
        let result = if let Some(parent) = parent {
            self.projected_initializer(owner, parent, span)
        } else {
            self.in_formal_declaration(owner, |this| {
                this.in_required_comptime = true;
                this.context = this
                    .initializer_implementation()
                    .and_then(|implementation| this.associated_owner(implementation.declaration))
                    .map(|implementation| this.definition_context(implementation))
                    .unwrap_or_else(|| this.nominal_context());
                this.materialize_initializer(owner, span)
            })
        };
        if !associated {
            self.active_constants.remove(&owner);
        }
        let value = result?;
        if closed {
            charge_copy(&value, &mut self.required_steps, span)?;
            self.constants.insert(owner, value.clone());
        }
        Ok(value)
    }

    fn projected_initializer(
        &mut self,
        owner: OwnerId,
        parent: OwnerId,
        span: Span,
    ) -> Result<n::Expr, Error> {
        // Project the receiver's selected arguments and their operand bindings.
        // The lexical scope, return target and quote holes belong to the member.
        let mut types = DeclarationBindings::default();
        let mut values = DeclarationBindings::default();
        let mut normal = DeclarationBindings::default();
        let mut locals = local_bindings::LocalBindings::default();
        for parameter in
            std::iter::once(declaration(self.package, &self.package_identities, parent))
                .chain(self.type_parameters[&parent].iter().copied())
                .chain(self.nominal_capture_ids(parent))
        {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "constant container input budget exhausted"))?;
            if let Some(value) = self.alias_substitution.shared(&parameter) {
                types.insert(parameter, value.clone());
            }
            if let Some(binding) = self.alias_value_bindings.get(&parameter) {
                values.insert(parameter, *binding);
                if let Some(domain) = self.locals.shared(binding) {
                    locals.insert(*binding, domain.clone());
                }
            }
            if let Some(input) = self.normal_default_bindings.shared(&parameter) {
                normal.insert(parameter, input.clone());
                if let Some(domain) = self.locals.shared(&input.0) {
                    locals.insert(input.0, domain.clone());
                }
            }
        }
        self.in_formal_declaration(owner, |this| {
            this.in_required_comptime = true;
            this.ensure_contract(parent, span)?;
            this.context = this.definition_context(parent);
            this.alias_substitution = types;
            this.alias_value_bindings = values;
            this.normal_default_bindings = normal;
            this.locals = locals;
            let ItemKind::Const(annotation, initializer) = this.package.item(owner).unwrap().kind
            else {
                unreachable!("projected constant")
            };
            this.check_projected_initializer(annotation, initializer)
        })
    }

    fn check_projected_initializer(
        &mut self,
        annotation: &Expr<'_>,
        initializer: &Expr<'_>,
    ) -> Result<n::Expr, Error> {
        let expected = if matches!(annotation.kind, H::TyPlaceholder) {
            None
        } else {
            Some(self.type_expr(annotation)?)
        };
        let value = self.expr(initializer, expected.as_ref())?;
        // Check the symbolic initializer without selecting dispatch or
        // invoking it: those operations may need the container inputs.
        // A runtime function identity may be carried, but not called.
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let checked = nir::visit::expression(&value, &mut fuel, |node, _| {
            if let K::Call { callee, .. } = &node.kind
                && !callee.ty.is_comptime_callable()
            {
                return Err(
                    "runtime function is unavailable in a comptime constant initializer".into(),
                );
            }
            Ok(())
        });
        self.required_steps -= (before - fuel) as u64;
        checked.map_err(|message| error(initializer.span, message))?;
        Ok(value)
    }

    fn materialize_initializer(&mut self, owner: OwnerId, span: Span) -> Result<n::Expr, Error> {
        let item = self.package.item(owner).unwrap();
        let (annotation, initializer) = match &item.kind {
            ItemKind::Const(annotation, initializer) => (*annotation, *initializer),
            ItemKind::Associated(associated) => (
                associated.ty,
                associated.default.ok_or_else(|| {
                    error(
                        span,
                        "required associated value needs a selected implementation",
                    )
                })?,
            ),
            _ => {
                return Err(error(
                    span,
                    "declaration is not a constant or associated value",
                ));
            }
        };
        let expected = if matches!(annotation.kind, H::TyPlaceholder) {
            None
        } else {
            Some(self.type_expr(annotation)?)
        };
        let (value, ty) = self.required_typed_expression(initializer, expected.as_ref())?;
        let mut snapshot = Materialization::default();
        let result = self.constant_expression(&value, ty, initializer.span, 0, &mut snapshot)?;
        if snapshot.bindings.is_empty() {
            Ok(result)
        } else {
            let span = source(initializer.span);
            let ty = result.ty.clone();
            let body = n::Expr {
                source: span,
                ty: ty.clone(),
                kind: K::Block {
                    statements: snapshot.bindings,
                    result: Some(Box::new(result)),
                },
            };
            // Reconstruct the graph in a private invocation namespace. A
            // cached graph's binding IDs must never overwrite caller locals.
            let callee = n::Expr {
                source: span,
                ty: Term::ComptimeFunction(Box::new(Term::Function {
                    parameters: vec![],
                    result: Box::new(ty.clone()),
                })),
                kind: K::Closure {
                    parameters: vec![],
                    captures: vec![],
                    body: std::sync::Arc::new(body),
                },
            };
            Ok(n::Expr {
                source: span,
                ty,
                kind: K::Call {
                    callee: Box::new(callee),
                    arguments: vec![],
                    parameter_bindings: vec![],
                },
            })
        }
    }
}
