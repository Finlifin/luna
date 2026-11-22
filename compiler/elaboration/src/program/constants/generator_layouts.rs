//! Checked generator statements can describe a fresh struct before it
//! exists. Its rigid Type and layout are scoped to definition checking.
use super::*;
use std::sync::Arc;

pub(in crate::program) struct SchemaSequence {
    frame: ObservationScope,
}

impl Default for SchemaSequence {
    fn default() -> Self {
        Self {
            frame: ObservationScope {
                unknown_outer_locals: true,
                ..Default::default()
            },
        }
    }
}

pub(in crate::program) struct SchemaStep {
    site: hir::HirId,
    sequence: SchemaSequence,
    advanced: bool,
}

impl Checker<'_, '_> {
    pub(in crate::program) fn struct_layout_head<'t>(
        &self,
        ty: &'t Term,
    ) -> Option<(&'t DeclarationId, &'t [Term])> {
        match ty {
            Term::Nominal {
                declaration,
                arguments,
            } => Some((declaration, arguments)),
            Term::Rigid {
                declaration,
                sort: Sort::Type,
            } if self.checking_code_generator
                && self
                    .generator_layouts
                    .as_ref()
                    .is_some_and(|layouts| layouts.contains_key(declaration)) =>
            {
                Some((declaration, &[]))
            }
            _ => None,
        }
    }

    pub(in crate::program) fn with_schema_statement<R>(
        &mut self,
        sequence: &mut SchemaSequence,
        statement: &Expr<'_>,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        if !self.checking_code_generator {
            return check(self);
        }
        let site = match &statement.kind {
            // A computed annotation or pattern can call user code outside the
            // initializer. Only carry mutable facts across a complete known step.
            H::Let(decl)
                if decl.guard.is_none()
                    && decl.otherwise.is_none()
                    && decl.ty.is_none_or(|ty| {
                        matches!(
                            ty.kind,
                            H::ResolvedDef { .. }
                                | H::ResolvedLocal(..)
                                | H::ResolvedFnParam(..)
                                | H::ResolvedClauseParam(..)
                        )
                    })
                    && matches!(
                        decl.pattern.kind,
                        hir::pattern::PatternKind::Binding(_, _, None)
                            | hir::pattern::PatternKind::Wild
                    ) =>
            {
                decl.init.map(|v| v.hir_id)
            }
            H::Application(..) | H::ExtendedApplication(..) => Some(statement.hir_id),
            _ => None,
        };
        let previous = std::mem::take(sequence);
        let active = site.map(|site| SchemaStep {
            site,
            sequence: previous,
            advanced: false,
        });
        let outer = std::mem::replace(&mut self.generator_schema_step, active);
        let result = check(self);
        let finished = std::mem::replace(&mut self.generator_schema_step, outer);
        if let Some(step) = finished.filter(|s| s.advanced) {
            *sequence = step.sequence;
        }
        result
    }

    pub(in crate::program) fn fork_generator_branch(
        &mut self,
        sequence: &mut SchemaSequence,
        condition: Option<&n::Expr>,
        span: Span,
    ) -> Result<SchemaSequence, Error> {
        self.schema_fuel(span, |_, fuel| {
            if let Some(condition) = condition
                && !super::structural_calls::structural_operand(
                    condition,
                    Some(&sequence.frame),
                    fuel,
                )?
            {
                *sequence = SchemaSequence::default();
            }
            Ok(SchemaSequence {
                frame: sequence.frame.fork(fuel)?,
            })
        })
    }

    pub(in crate::program) fn join_generator_schemas(
        &mut self,
        yes: &mut SchemaSequence,
        no: SchemaSequence,
        yes_continues: bool,
        no_continues: bool,
        span: Span,
    ) -> Result<(), Error> {
        match (yes_continues, no_continues) {
            (true, false) => {}
            (false, true) => *yes = no,
            (false, false) => *yes = SchemaSequence::default(),
            (true, true) => self.schema_fuel(span, |this, fuel| {
                yes.frame.join(&no.frame, this.protocols.meta_list, fuel)
            })?,
        }
        Ok(())
    }

    pub(in crate::program) fn retain_visible_schema(
        &mut self,
        sequence: &mut SchemaSequence,
        span: Span,
    ) -> Result<(), Error> {
        let frame = &mut sequence.frame;
        self.schema_fuel(span, |this, fuel| {
            frame.retain(|id| this.generator_data.contains_key(id), fuel)
        })
    }

    pub(in crate::program) fn observe_schema_step(
        &mut self,
        site: &Expr<'_>,
        expr: &n::Expr,
        pattern: Option<&n::Pattern>,
    ) -> Result<Option<Arc<PartialStagedValue>>, Error> {
        if !self
            .generator_schema_step
            .as_ref()
            .is_some_and(|step| step.site == site.hir_id)
        {
            return Ok(None);
        }
        let mut step = self.generator_schema_step.take().unwrap();
        let frame = &mut step.sequence.frame;
        let result = if let Some(pattern) = pattern {
            let observed = self.structural_value(expr, frame, site.span, 0)?;
            if let Some(observed) = &observed {
                let exports = self.schema_fuel(site.span, |this, fuel| {
                    this.known_staged_pattern_bindings(pattern, observed, 0, fuel)
                        .map(|(_, values)| values)
                })?;
                self.schema_fuel(site.span, |_, fuel| {
                    for (binding, value) in exports {
                        frame.insert(binding, value, fuel)?;
                    }
                    Ok(())
                })?;
                step.advanced = true;
            }
            observed
        } else {
            step.advanced = self.structural_statement(expr, frame, site.span, 0)?;
            None
        };
        self.generator_schema_step = Some(step);
        Ok(result)
    }

    pub(super) fn observe_built_struct(
        &mut self,
        packet: &n::Expr,
        scope: &ObservationScope,
        span: Span,
    ) -> Result<Option<Arc<PartialStagedValue>>, Error> {
        let Some(owner) = self
            .signature_owner
            .filter(|_| self.generator_layouts.is_some() && self.checking_code_generator)
        else {
            return Ok(None);
        };
        let fields = self.schema_fuel(span, |this, fuel| {
            if !super::structural_calls::structural_operand(packet, Some(scope), fuel)? {
                return Ok(None);
            }
            let packet = this.partial_staged_value_in(packet, 0, fuel, scope)?;
            packet.charge(fuel)?;
            let head = this.protocols.meta_list;
            let Some((PartialStagedShape::Tuple, entries)) = packet.aggregate(head) else {
                return Ok(None);
            };
            let [_name, labels, types] = entries.as_slice() else {
                return Ok(None);
            };
            let Some((
                PartialStagedShape::List {
                    exact: true,
                    ty: label_domain,
                },
                labels,
            )) = labels.aggregate(head)
            else {
                return Ok(None);
            };
            let Some((
                PartialStagedShape::List {
                    exact: true,
                    ty: type_domain,
                },
                types,
            )) = types.aggregate(head)
            else {
                return Ok(None);
            };
            if this.list_element_type(&label_domain) != Some(&p(P::Symbol))
                || this.list_element_type(&type_domain) != Some(&p(P::Type))
                || labels.len() != types.len()
            {
                return Ok(None);
            }
            let mut fields = Vec::with_capacity(types.len());
            let mut unique = BTreeSet::new();
            for (label, ty) in labels.iter().zip(&types) {
                let (Some(Term::Symbol(label)), Some(ty)) = (label.known_term(), ty.known_term())
                else {
                    return Ok(None);
                };
                if ty.sort() != Sort::Type || !unique.insert(label) {
                    return Ok(None);
                }
                fields.push((label.to_string(), ty.clone()));
            }
            Ok(Some(fields))
        })?;
        let Some(fields) = fields else {
            return Ok(None);
        };
        let binder = self.fresh_type_binder(owner, span)?;
        self.generator_layouts.as_mut().unwrap().insert(
            binder.declaration,
            n::Struct {
                declaration: binder.declaration,
                // A diagnostic label for the scoped descriptor. The public
                // nominal name comes from the executed construction packet.
                name: binder.declaration.to_string(),
                fields,
                packed: false,
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
            },
        );
        Ok(Some(PartialStagedValue::known(Term::Rigid {
            declaration: binder.declaration,
            sort: Sort::Type,
        })))
    }
}
