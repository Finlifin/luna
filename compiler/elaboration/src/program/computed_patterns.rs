//! Closed observation regions for owned computed subjects. Only checked Copy
//! scalar data leaves the region; the subject's owner never does.
use super::*;

impl Checker<'_, '_> {
    // Recover candidate-local evidence without replacing the original guard:
    // its construction plan and formation contracts must still execute.
    pub(super) fn known_type_equality_pattern(
        &mut self,
        pattern: &n::Pattern,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        self.known_type_equality_pattern_at(pattern, span, 0)
    }

    fn known_type_equality_pattern_at(
        &mut self,
        pattern: &n::Pattern,
        span: Span,
        depth: usize,
    ) -> Result<Option<Term>, Error> {
        if depth >= 128 {
            return Err(error(span, "type pattern evidence depth exhausted"));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "type pattern evidence budget exhausted"))?;
        match pattern {
            n::Pattern::Binding { inner, .. } | n::Pattern::Remap { pattern: inner, .. } => {
                self.known_type_equality_pattern_at(inner, span, depth + 1)
            }
            n::Pattern::Guard { pattern, condition } => {
                if let Some(ty) = self.known_type_equality_pattern_at(pattern, span, depth + 1)? {
                    return Ok(Some(ty));
                }
                let n::Pattern::Binding { binding, inner } = pattern.as_ref() else {
                    return Ok(None);
                };
                if !matches!(inner.as_ref(), n::Pattern::Wild) {
                    return Ok(None);
                }
                let mut fuel = self.required_steps;
                let evidence = pattern_guard_evidence(
                    condition,
                    *binding,
                    true,
                    &mut fuel,
                    span,
                    depth + 1,
                    &mut |expression, truth, fuel| {
                        let K::TypeEqual(left, right) = &expression.kind else {
                            return Ok(None);
                        };
                        let (id, value) = match (&left.kind, &right.kind) {
                            (K::Local(id), _) => (*id, right),
                            (_, K::Local(id)) => (*id, left),
                            _ => return Ok(None),
                        };
                        let mut budget = comptime_runtime::Budget {
                            steps: *fuel as usize,
                            max_limbs: 4096,
                        };
                        let before = budget.steps;
                        let known = self.known_normal_input(value, 0, &mut budget);
                        *fuel -= (before - budget.steps) as u64;
                        let Some(ty) = known.filter(|ty| ty.sort() == Sort::Type) else {
                            return Ok(None);
                        };
                        Ok(Some(if truth && id == *binding {
                            PatternGuard::Exact(ty)
                        } else {
                            PatternGuard::Unknown
                        }))
                    },
                );
                self.required_steps = fuel;
                Ok(match evidence? {
                    Some(PatternGuard::Exact(ty)) => Some(ty),
                    _ => None,
                })
            }
            n::Pattern::Then { first, value, next } => {
                if let Some(ty) = self.known_type_equality_pattern_at(first, span, depth + 1)? {
                    return Ok(Some(ty));
                }
                // Only re-observing the original binding constrains its domain.
                if let (n::Pattern::Binding { binding, inner }, K::Local(id)) =
                    (first.as_ref(), &value.kind)
                {
                    if binding == id && matches!(inner.as_ref(), n::Pattern::Wild) {
                        return self.known_type_equality_pattern_at(next, span, depth + 1);
                    }
                }
                Ok(None)
            }
            n::Pattern::Or(arms) => {
                let mut common = None;
                for arm in arms {
                    let Some(ty) = self.known_type_equality_pattern_at(arm, span, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    if common.as_ref().is_some_and(|previous| previous != &ty) {
                        return Ok(None);
                    }
                    common = Some(ty);
                }
                Ok(common)
            }
            _ => Ok(None),
        }
    }

    fn scalar_export_shape(&mut self, ty: &Term, span: Span, depth: usize) -> Result<bool, Error> {
        if depth >= 128 {
            return Err(error(span, "computed export depth exhausted"));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "computed export budget exhausted"))?;
        if matches!(
            ty,
            Term::Rigid { .. } | Term::AssociatedType { .. } | Term::NormalForm { .. }
        ) && self.bound_tuple_elements(ty).is_none()
        {
            let (view, used) = self.context.structural_view_with_heads_metered(
                ty,
                crate::ptc::Budget {
                    steps: (self.required_steps / 2).min(10_000) as usize,
                    max_depth: 128,
                },
                |head| Some(head) == self.protocols.tuple,
            );
            self.required_steps = self.required_steps.saturating_sub(used as u64);
            let Some((view, _)) = view
                .map_err(|reason| error(span, format!("computed export type view: {reason:?}")))?
            else {
                return Ok(false);
            };
            if view == *ty {
                return Ok(false);
            }
            return self.scalar_export_shape(&view, span, depth + 1);
        }
        if let Some(fields) = self.bound_tuple_elements(ty) {
            for field in fields {
                if !self.scalar_export_shape(field, span, depth + 1)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        match ty {
            Term::Primitive(
                P::Bool
                | P::Char
                | P::I8
                | P::I16
                | P::I32
                | P::I64
                | P::I128
                | P::U8
                | P::U16
                | P::U32
                | P::U64
                | P::U128
                | P::F32
                | P::F64
                | P::Unit
                | P::Void,
            ) => Ok(true),
            Term::Tuple(fields) => {
                for field in fields {
                    if !self.scalar_export_shape(field, span, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Term::Record(fields) => {
                for (_, field) in fields {
                    if !self.scalar_export_shape(field, span, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Term::ErrorResult { errors, result } => {
                // Labels are complete Type instances, not flattened payloads
                // and not effect operations. Every possible tag is checked.
                for field in std::iter::once(result.as_ref()).chain(errors) {
                    if !self.scalar_export_shape(field, span, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Term::Nominal { declaration, .. } => {
                let variants = self.enums.get(declaration).map(|item| item.variants.len());
                for index in 0..variants.unwrap_or(1) {
                    let allowance = self.required_steps.min(10_000) as usize;
                    let mut fuel = allowance;
                    let fields = nir::normal_forms::nominal_value_fields_with_lookup_in(
                        ty,
                        variants.map(|_| index as u32),
                        &mut fuel,
                        &|id, fuel| {
                            *fuel = fuel.checked_sub(1)?;
                            use nir::normal_forms::NominalValueDeclaration as D;
                            self.structs
                                .get(&id)
                                .map(D::Struct)
                                .or_else(|| self.enums.get(&id).map(D::Enum))
                                .or_else(|| self.newtypes.get(&id).map(D::Newtype))
                        },
                        self.target,
                    );
                    self.required_steps -= (allowance - fuel) as u64;
                    let Some(fields) = fields else {
                        return Ok(false);
                    };
                    for field in fields {
                        if !self.scalar_export_shape(&field, span, depth + 1)? {
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(super) fn lower_computed_pattern(
        &mut self,
        first: n::Pattern,
        value: n::Expr,
        next: n::Pattern,
        span: Span,
    ) -> Result<n::Pattern, Error> {
        let unchanged = |first, value, next| n::Pattern::Then {
            first: Box::new(first),
            value: Box::new(value),
            next: Box::new(next),
        };
        if nir::program::scalar_observation_shape(&value.ty)
            || matches!(value.ty, Term::Pointer(_) | Term::Function { .. })
        {
            return Ok(unchanged(first, value, next));
        }
        let Some(bindings) = self.checked_pattern_exports(&next, span, 0)? else {
            return Ok(unchanged(first, value, next));
        };
        let mut exports = Vec::new();
        let mut parameters = BTreeSet::new();
        for binding in bindings {
            let Some(ty) = self.locals.get(&binding).cloned() else {
                return Ok(unchanged(first, value, next));
            };
            if !self.scalar_export_shape(&ty, span, 0)? {
                return Ok(unchanged(first, value, next));
            }
            let allowance = self.required_steps.min(10_000) as usize;
            let mut fuel = allowance;
            let charged = nir::type_computations::charge_term(&ty, &mut fuel);
            self.required_steps -= (allowance - fuel) as u64;
            charged.map_err(|reason| error(span, reason.to_string()))?;
            if !ty.runtime_dependencies().is_empty()
                || !ty
                    .substitution_parameters()
                    .map_err(|reason| error(span, reason.to_string()))?
                    .is_empty()
            {
                return Ok(unchanged(first, value, next));
            }
            for (parameter, sort) in ty
                .rigid_parameters()
                .map_err(|reason| error(span, reason.to_string()))?
            {
                if sort != Sort::Type {
                    return Ok(unchanged(first, value, next));
                }
                parameters.insert(parameter);
            }
            exports.push((binding, ty));
        }
        let exhaustive = self.pattern_exhaustive(&next, &value.ty);
        let packet = if exports.is_empty() {
            None
        } else {
            let declaration = DeclarationId {
                package: u64::MAX - 2,
                local: self.next_temporary_binding,
            };
            self.next_temporary_binding = self
                .next_temporary_binding
                .checked_add(1)
                .ok_or_else(|| error(span, "computed packet identity exhausted"))?;
            let mut variants = vec![n::Variant {
                name: "accepted".into(),
                shape: n::VariantShape::Tuple,
                fields: exports.iter().map(|(_, ty)| ty.clone()).collect(),
            }];
            // Rejection carries no payload: do not fabricate aggregate values
            // or introduce cleanup responsibilities for inactive exports.
            if !exhaustive {
                variants.push(n::Variant {
                    name: "rejected".into(),
                    shape: n::VariantShape::Tuple,
                    fields: vec![],
                });
            }
            self.enums.insert(
                declaration,
                n::Enum {
                    parameter_types: vec![],
                    capabilities: vec![],
                    requirements: vec![],
                    type_parameters: parameters.iter().copied().collect(),
                    declaration,
                    name: "<computed-scalar-packet>".into(),
                    variants,
                    groups: vec![],
                    encoding: None,
                    is_intrinsic: false,
                    symbol_encoding: None,
                },
            );
            self.code_packet_nominals.insert(declaration);
            self.computed_scalar_packets.insert(declaration);
            Some(declaration)
        };
        let output = packet
            .map(|declaration| Term::Nominal {
                declaration,
                arguments: parameters
                    .iter()
                    .map(|declaration| Term::Rigid {
                        declaration: *declaration,
                        sort: Sort::Type,
                    })
                    .collect(),
            })
            .unwrap_or_else(|| p(P::Bool));
        let mut accepted = Vec::new();
        let mut remap = Vec::new();
        let mut output_fields = Vec::new();
        for (binding, ty) in exports {
            let exported = self.spread_binding(span)?;
            remap.push((binding, exported));
            accepted.push(self.computed_node(ty, K::Local(exported), span)?);
            output_fields.push(n::Pattern::Binding {
                binding,
                inner: Box::new(n::Pattern::Wild),
            });
        }
        let next = if packet.is_some() {
            n::Pattern::Remap {
                pattern: Box::new(next),
                bindings: remap,
            }
        } else {
            next
        };
        let accepted = self.computed_node(
            output.clone(),
            match packet {
                Some(declaration) => K::Variant {
                    declaration,
                    index: 0,
                    fields: accepted,
                },
                None => K::Literal(n::Literal::Bool(true)),
            },
            span,
        )?;
        let mut arms = vec![n::MatchArm {
            pattern: next,
            guard: None,
            body: accepted,
        }];
        if !exhaustive {
            let rejected = self.computed_node(
                output.clone(),
                match packet {
                    Some(declaration) => K::Variant {
                        declaration,
                        index: 1,
                        fields: vec![],
                    },
                    None => K::Literal(n::Literal::Bool(false)),
                },
                span,
            )?;
            arms.push(n::MatchArm {
                pattern: n::Pattern::Wild,
                guard: None,
                body: rejected,
            });
        }
        let matched = self.computed_node(
            output.clone(),
            K::Match {
                value: Box::new(value),
                arms,
            },
            span,
        )?;
        let region = self.computed_node(
            output,
            K::Block {
                statements: vec![],
                result: Some(Box::new(matched)),
            },
            span,
        )?;
        let exported_pattern = match packet {
            Some(declaration) => n::Pattern::Variant {
                declaration,
                index: 0,
                fields: output_fields,
            },
            None if exhaustive => n::Pattern::Wild,
            None => n::Pattern::Literal(n::Literal::Bool(true)),
        };
        Ok(unchanged(first, region, exported_pattern))
    }

    fn computed_node(&mut self, ty: Term, kind: K, span: Span) -> Result<n::Expr, Error> {
        Ok(n::Expr {
            source: self.error_generated_source(span)?,
            ty,
            kind,
        })
    }
}
