//! Preserve dereferenced storage through field selection. This builds addresses;
//! reading or transferring the contents is a separate operation at the use site.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn intrinsic_pointer_input(&self, ty: &Term) -> bool {
        n::intrinsic_pointer_value(ty)
            || self
                .context
                .structural_view(ty, crate::ptc::Budget::default())
                .is_ok_and(|view| view.is_some_and(|(ty, _)| n::intrinsic_pointer_value(&ty)))
    }

    pub(super) fn dereferenced_place(&mut self, expr: &Expr<'_>) -> Result<Option<n::Expr>, Error> {
        let mut base = expr;
        let mut fields = Vec::new();
        while let H::Projection(inner, field) = &base.kind {
            if fields.len() == 128 {
                return Err(error(expr.span, "dereferenced place depth exhausted"));
            }
            fields.push(*field);
            base = inner;
        }
        let H::Deref(pointer) = &base.kind else {
            return Ok(None);
        };
        let mut pointer = self.expr(pointer, None)?;
        if !matches!(pointer.ty, Term::Pointer(_)) {
            return Err(error(base.span, "dereference requires a pointer"));
        }
        for field in fields.into_iter().rev() {
            let Term::Pointer(subject) = &pointer.ty else {
                unreachable!()
            };
            let shape = self.structural_type(subject, field.span)?;
            self.ensure_nominal_type(&shape, field.span)?;
            if let Term::Record(fields) = &shape {
                let index = fields
                    .iter()
                    .position(|(name, _)| name == field.name.as_str())
                    .ok_or_else(|| {
                        error(field.span, format!("unknown record field {}", field.name))
                    })?;
                pointer = n::Expr {
                    source: source(expr.span),
                    ty: Term::Pointer(Box::new(fields[index].1.clone())),
                    kind: K::PointerAnonymousField {
                        pointer: Box::new(pointer),
                        name: field.name.to_string(),
                        index,
                    },
                };
                continue;
            }

            let Term::Nominal {
                declaration,
                arguments,
            } = shape
            else {
                return Err(error(
                    field.span,
                    "dereferenced field place requires a struct; spell pointer indirection explicitly",
                ));
            };
            let definition = self
                .structs
                .get(&declaration)
                .ok_or_else(|| error(field.span, "dereferenced field place requires a struct"))?;
            let index = definition
                .fields
                .iter()
                .position(|(name, _)| name == field.name.as_str())
                .ok_or_else(|| error(field.span, format!("unknown struct field {}", field.name)))?;
            let ty = self.instantiate_input_types(
                &definition.fields[index].1,
                &definition.type_parameters,
                &arguments,
                field.span,
            )?;
            pointer = n::Expr {
                source: source(expr.span),
                ty: Term::Pointer(Box::new(ty)),
                kind: K::PointerField {
                    pointer: Box::new(pointer),
                    declaration,
                    index,
                },
            };
        }
        Ok(Some(pointer))
    }
}

impl Checker<'_, '_> {
    /// Reuse ordinary operator selection and writeback. Pointer-producing
    /// targets are prepared once; local/field paths have no address effects.
    pub(super) fn compound_assignment(
        &mut self,
        expression: &Expr<'_>,
        op: &BinOp,
        target: &Expr<'_>,
        value: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let Some(pointer) = self.dereferenced_place(target)? else {
            // Restrict this rewrite to stable local field paths. Indexing needs
            // its own prepared location protocol, not duplicated source code.
            let mut base = target;
            while let H::Projection(inner, _) = &base.kind {
                base = inner;
            }
            if !matches!(
                base.kind,
                H::ResolvedLocal(..) | H::ResolvedFnParam(..) | H::TacticParameter(_)
            ) {
                return Err(error(
                    target.span,
                    "compound assignment requires a local field or dereferenced place; indexed locations need a prepared writeback protocol",
                ));
            }
            let binary = Expr {
                kind: H::Binary(*op, target, value),
                ..expression.clone()
            };
            let assign = Expr {
                kind: H::Assign(target, &binary),
                ..expression.clone()
            };
            return self.expr(&assign, expected);
        };
        let binding = self.spread_binding(target.span)?;
        let id = hir::HirId::new(hir::OwnerId::INVALID, hir::ItemLocalId::new(binding.index));
        let local_expr = Expr {
            hir_id: id,
            kind: H::ResolvedLocal(hir::Symbol::invalid(), id),
            span: target.span,
        };
        let prepared_target = Expr {
            kind: H::Deref(&local_expr),
            ..target.clone()
        };
        let binary = Expr {
            kind: H::Binary(*op, &prepared_target, value),
            ..expression.clone()
        };
        let assign = Expr {
            kind: H::Assign(&prepared_target, &binary),
            ..expression.clone()
        };
        self.locals.insert(binding, pointer.ty.clone());
        let checked = self.expr(&assign, expected);
        self.locals.remove(&binding);
        let mut checked = checked?;
        let original = source(expression.span);
        let mut generated = || -> Result<n::SourceRange, Error> {
            let index = u32::try_from(self.expansion_sources.len()).map_err(|_| {
                error(
                    expression.span,
                    "compound assignment source space exhausted",
                )
            })?;
            self.expansion_sources.push(n::ExpansionSource {
                original,
                site: original,
            });
            Ok(n::SourceRange::generated(index))
        };
        checked.source = generated()?;
        let prepare = n::Expr {
            ty: p(P::Void),
            source: generated()?,
            kind: K::Let {
                binding,
                value: Box::new(pointer),
            },
        };
        Ok(n::Expr {
            ty: checked.ty.clone(),
            source: original,
            kind: K::Block {
                statements: vec![prepare],
                result: Some(Box::new(checked)),
            },
        })
    }
}
