//! Constructor identity and declaration-order binding for named fields.
use super::*;
use hir::pattern::FieldPat;

pub(super) struct RecordHead {
    pub(super) ty: Term,
    pub(super) variant: Option<usize>,
}

impl Checker<'_, '_> {
    pub(super) fn named_record_head(
        &mut self,
        callee: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<RecordHead, Error> {
        if !self.is_type_head(callee)
            && let Some((ty, index)) = self.variant_target(callee, expected)?
        {
            return Ok(RecordHead {
                ty,
                variant: Some(index),
            });
        }
        // Check the head in type position, including ordinary comptime Type
        // values. Field checking requests the layout after admitting the label.
        Ok(RecordHead {
            ty: self.type_expr(callee)?,
            variant: None,
        })
    }

    pub(super) fn is_copy_type_for_pattern(&self, ty: &Term) -> bool {
        if self.intrinsic_pointer_input(ty) {
            return true;
        }
        let Some(copy_protocol) = self.protocols.copy else {
            return false;
        };
        if !self
            .checked_traits
            .iter()
            .any(|interface| interface.declaration == copy_protocol)
        {
            return false;
        }
        let assumptions = self.capability_assumptions();
        let world = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        };
        matches!(
            crate::verification::copy::classify(
                &world,
                copy_protocol,
                ty,
                crate::ptc::Budget::default(),
            ),
            Ok(crate::verification::copy::Classification::Intrinsic
                | crate::verification::copy::Classification::Declared { .. }
                | crate::verification::copy::Classification::Assumed(_))
        )
    }

    pub(super) fn has_custom_drop(&self, ty: &Term) -> bool {
        if !self.interfaces_ready {
            return false;
        }
        let Some(drop_protocol) = self.protocols.drop else {
            return false;
        };
        if !self
            .checked_traits
            .iter()
            .any(|interface| interface.declaration == drop_protocol)
        {
            return false;
        }
        let assumptions = self.capability_assumptions();
        let world = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        };
        world
            .resolve(drop_protocol, ty, crate::ptc::Budget::default())
            .is_ok_and(|resolution| {
                matches!(
                    resolution.types.selection,
                    crate::implementation::Selection::Selected(_)
                )
            })
    }

    pub(super) fn pattern_moves_non_copy(
        &mut self,
        pattern: &n::Pattern,
        ty: &Term,
        span: Span,
    ) -> Result<bool, Error> {
        self.pattern_moves_non_copy_at(pattern, ty, span, 0)
    }

    fn pattern_moves_non_copy_at(
        &mut self,
        pattern: &n::Pattern,
        ty: &Term,
        span: Span,
        depth: usize,
    ) -> Result<bool, Error> {
        if depth >= 128 {
            return Err(error(span, "pattern ownership check depth exhausted"));
        }
        match pattern {
            n::Pattern::Wild
            | n::Pattern::Unit
            | n::Pattern::Literal(_)
            | n::Pattern::OptionalAbsent => Ok(false),
            n::Pattern::Binding { inner, .. } => {
                if !self.is_copy_type_for_pattern(ty) {
                    return Ok(true);
                }
                self.pattern_moves_non_copy_at(inner, ty, span, depth + 1)
            }
            n::Pattern::Guard { pattern, .. } | n::Pattern::Remap { pattern, .. } => {
                self.pattern_moves_non_copy_at(pattern, ty, span, depth + 1)
            }
            n::Pattern::Observe { inner, .. } => {
                // Observe creates a candidate-local non-owning alias, except
                // Then transfers below may consume if non-Copy.
                if let n::Pattern::Then { first, value, .. } = inner.as_ref()
                    && matches!(value.kind, K::ReadPointer(_))
                {
                    return self.pattern_moves_non_copy_at(first, ty, span, depth + 1);
                }
                self.pattern_moves_non_copy_at(inner, ty, span, depth + 1)
            }
            n::Pattern::Then { first, value, next } => {
                Ok(self.pattern_moves_non_copy_at(first, ty, span, depth + 1)?
                    || self.pattern_moves_non_copy_at(next, &value.ty, span, depth + 1)?)
            }
            n::Pattern::Or(alts) => {
                for alt in alts {
                    if self.pattern_moves_non_copy_at(alt, ty, span, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            n::Pattern::Tuple(fields) => {
                let shape = self.structural_type(ty, span)?;
                let Some(types) = self.bound_tuple_elements(&shape).or_else(|| match &shape {
                    Term::Tuple(types) => Some(types.as_slice()),
                    _ => None,
                }) else {
                    return Ok(false);
                };
                for (field, field_ty) in fields.iter().zip(types) {
                    if self.pattern_moves_non_copy_at(field, field_ty, span, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            n::Pattern::AnonymousRecord { fields, .. } => {
                let Term::Record(types) = self.structural_type(ty, span)? else {
                    return Ok(false);
                };
                for (field, (_, field_ty)) in fields.iter().zip(&types) {
                    if self.pattern_moves_non_copy_at(field, field_ty, span, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            n::Pattern::Record { fields, .. } => {
                let shape = self.structural_type(ty, span)?;
                let types = self.instantiated_fields(&shape, span)?;
                for (field, (_, field_ty)) in fields.iter().zip(&types) {
                    if self.pattern_moves_non_copy_at(field, field_ty, span, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            n::Pattern::OptionalPresent(inner) => {
                let Term::Optional(inner_ty) = self.type_identity_view(ty, span)? else {
                    return Ok(false);
                };
                self.pattern_moves_non_copy_at(inner, &inner_ty, span, depth + 1)
            }
            n::Pattern::Variant { index, fields, .. } => {
                let shape = self.structural_type(ty, span)?;
                let variants = self.instantiated_variants(&shape, span)?;
                let Some(variant) = variants.get(*index) else {
                    return Ok(false);
                };
                for (field, field_ty) in fields.iter().zip(&variant.fields) {
                    if self.pattern_moves_non_copy_at(field, field_ty, span, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            n::Pattern::ErrorResult {
                label: Some(label),
                inner,
            } => self.pattern_moves_non_copy_at(inner, label, span, depth + 1),
            n::Pattern::ErrorResult { label: None, inner } => {
                let Term::ErrorResult { result, .. } = ty else {
                    return Ok(false);
                };
                self.pattern_moves_non_copy_at(inner, result, span, depth + 1)
            }
            _ => Ok(false),
        }
    }

    pub(super) fn check_rest_drop_partial_move(
        &mut self,
        ty: &Term,
        checked_fields: &[n::Pattern],
        field_types: &[Term],
        has_omitted: bool,
        span: Span,
    ) -> Result<(), Error> {
        if !has_omitted || !self.has_custom_drop(ty) {
            return Ok(());
        }
        for (field, field_ty) in checked_fields.iter().zip(field_types) {
            if self.pattern_moves_non_copy(field, field_ty, span)? {
                return Err(error(
                    span,
                    "cannot partially move non-Copy field from a type that implements Drop under `...`",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn record_pattern(
        &mut self,
        ty: &Term,
        fields: &[FieldPat<'_>],
        rest: bool,
        span: Span,
    ) -> Result<n::Pattern, Error> {
        let has_map = self
            .protocols
            .map_pattern
            .is_some_and(|id| self.has_record_protocol_impl(id, ty, None));
        let has_struct = self
            .protocols
            .struct_pattern
            .is_some_and(|id| self.has_record_protocol_impl(id, ty, None));
        if has_map && has_struct {
            return Err(error(
                span,
                "ambiguous record pattern view: type implements both StructPatternView and MapPatternView",
            ));
        }
        if has_struct {
            return self.record_view_pattern(
                ty,
                fields,
                rest,
                span,
                self.protocols.struct_pattern.unwrap(),
                false,
            );
        }
        if has_map {
            return self.record_view_pattern(
                ty,
                fields,
                rest,
                span,
                self.protocols.map_pattern.unwrap(),
                true,
            );
        }

        let shape = self.structural_type(ty, span)?;
        let (declaration, declared) = match shape {
            Term::Record(fields) => (None, fields),
            Term::Nominal { declaration, .. } if self.structs.contains_key(&declaration) => {
                (Some(declaration), self.instantiated_fields(&shape, span)?)
            }
            _ => {
                return Err(error(
                    span,
                    "record pattern requires a struct type; custom pattern view elaboration is not implemented",
                ));
            }
        };
        let mut routed = vec![None; declared.len()];
        for field in fields {
            let slot = declared
                .iter()
                .position(|(name, _)| name == field.ident.name.as_str())
                .ok_or_else(|| error(field.span, "unknown struct pattern field"))?;
            if routed[slot].replace(field).is_some() {
                return Err(error(field.span, "duplicate struct pattern field"));
            }
        }
        let has_omitted = routed.iter().any(Option::is_none);
        // A guard in a later declared field can observe an earlier binding.
        // Unmentioned fields act as wildcards whether omitted silently or via `...`.
        let mut checked = Vec::with_capacity(declared.len());
        for (field, (_, ty)) in routed.into_iter().zip(&declared) {
            checked.push(match field {
                Some(field) => self.pattern(&field.pat, ty)?,
                None => n::Pattern::Wild,
            });
        }
        if rest && declaration.is_some() {
            let field_types: Vec<Term> = declared.iter().map(|(_, ty)| ty.clone()).collect();
            self.check_rest_drop_partial_move(ty, &checked, &field_types, has_omitted, span)?;
        }
        Ok(match declaration {
            Some(declaration) => n::Pattern::Record {
                declaration,
                fields: checked,
            },
            None => n::Pattern::AnonymousRecord {
                names: declared.into_iter().map(|(name, _)| name).collect(),
                fields: checked,
            },
        })
    }

    fn has_record_protocol_impl(
        &self,
        protocol: DeclarationId,
        ty: &Term,
        required_phase: Option<n::TraitPhase>,
    ) -> bool {
        let iface = n::InterfaceInstance {
            declaration: protocol,
            arguments: vec![],
        };
        if self.symbolic_interface_evidence(ty, &iface).ok() == Some(true) {
            return true;
        }
        self.resolve_trait_capability(&iface, ty, crate::ptc::Budget::default())
            .ok()
            .is_some_and(|res| {
                matches!(
                    res.selection,
                    crate::implementation::Selection::Selected(impl_id)
                        if self
                            .implementation_positions
                            .get(&impl_id)
                            .and_then(|pos| self.checked_implementations.get(*pos))
                            .is_some_and(|imp| required_phase.is_none_or(|p| imp.phases.contains(p)))
                )
            })
    }

    fn record_view_pattern(
        &mut self,
        ty: &Term,
        fields: &[FieldPat<'_>],
        rest: bool,
        span: Span,
        protocol: DeclarationId,
        is_map: bool,
    ) -> Result<n::Pattern, Error> {
        let comptime = !self.runtime_value_context() && !self.in_code_body;
        let active_phase = if comptime {
            n::TraitPhase::Comptime
        } else {
            n::TraitPhase::Runtime
        };
        let contract = if is_map {
            crate::record_patterns::map_for_phase(&self.checked_traits, protocol, comptime)
        } else {
            crate::record_patterns::struct_for_phase(&self.checked_traits, protocol, comptime)
        }
        .map_err(|message| error(span, message))?;
        if contract.comptime != comptime
            || !self.has_record_protocol_impl(protocol, ty, Some(active_phase))
        {
            return Err(error(
                span,
                if is_map {
                    if comptime {
                        "map patterns require the comptime MapPatternView phase"
                    } else {
                        "map patterns require the runtime MapPatternView phase"
                    }
                } else if comptime {
                    "struct view patterns require the comptime StructPatternView phase"
                } else {
                    "struct view patterns require the runtime StructPatternView phase"
                },
            ));
        }
        if is_map && !rest {
            return Err(error(
                span,
                "open MapPatternView pattern requires trailing `...` to ignore remaining keys",
            ));
        }
        let mut seen = BTreeSet::new();
        for field in fields {
            if !seen.insert(field.ident.name.as_str()) {
                return Err(error(field.span, "duplicate struct pattern field"));
            }
        }
        let binding = n::LocalId {
            owner: u32::MAX,
            index: self.next_temporary_binding,
        };
        self.next_temporary_binding = self
            .next_temporary_binding
            .checked_add(1)
            .ok_or_else(|| error(span, "pattern temporary binding space exhausted"))?;
        self.locals.insert(binding, ty.clone());
        let subject = n::Expr {
            source: source(span),
            ty: ty.clone(),
            kind: K::Local(binding),
        };
        let receiver = if contract.by_value {
            subject
        } else if comptime {
            return Err(error(
                span,
                "comptime record view patterns require value-reference self; replace *self with self",
            ));
        } else {
            n::Expr {
                source: source(span),
                ty: Term::Pointer(Box::new(ty.clone())),
                kind: K::AddressOf(Box::new(subject)),
            }
        };
        let iface = n::InterfaceInstance {
            declaration: protocol,
            arguments: vec![],
        };
        let assoc_contract = self
            .checked_traits
            .by_id(protocol)
            .and_then(|t| t.associated.iter().find(|a| a.name == contract.field_type))
            .ok_or_else(|| error(span, "record pattern view protocol lacks FieldType member"))?
            .clone();
        let has_rigids = {
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let res = nir::type_computations::has_rigid_parameters_bounded(ty, &mut fuel)
                .map_err(|failure| error(span, failure.to_string()))?;
            self.required_steps -= (before - fuel) as u64;
            res
        };
        let bridge_span = span.shrink_to_hi();
        let mut steps = Vec::with_capacity(fields.len());
        for field in fields {
            let prop_name = field.ident.name.as_str();
            let key_expr = n::Expr {
                source: source(field.span),
                ty: p(P::Symbol),
                kind: K::Literal(n::Literal::Symbol(prop_name.to_owned())),
            };
            let concrete_field_ty = if !has_rigids {
                let (parameters, inputs) = self.interface_substitution(&iface, ty);
                let fn_ty = self.instantiate_input_types(
                    &assoc_contract.ty,
                    &parameters,
                    &inputs,
                    field.span,
                )?;
                let assoc_expr = n::Expr {
                    source: source(field.span),
                    ty: fn_ty,
                    kind: K::AssociatedValue {
                        interface: iface.clone(),
                        member: assoc_contract.declaration,
                        subject: ty.clone(),
                    },
                };
                let call_site = Expr {
                    hir_id: field.pat.hir_id,
                    span: field.span,
                    kind: H::Unit,
                };
                let field_type_call = self.with_staged_input(true, |this| {
                    this.finish_call(
                        &call_site,
                        assoc_expr,
                        &[],
                        vec![key_expr.clone()],
                        Some(&p(P::Type)),
                        None,
                    )
                })?;
                let (value, _) =
                    self.required_normal_input(&call_site, field_type_call, Vec::new())?;
                let resolved_ty =
                    crate::program::type_data::CompletedType::from_value(value, field.span)?
                        .identity(field.span)?;
                if is_never(&resolved_ty) {
                    return Err(error(
                        field.span,
                        format!(
                            "unknown {} field `{prop_name}` has NoReturn type",
                            if is_map {
                                "MapPatternView"
                            } else {
                                "StructPatternView"
                            }
                        ),
                    ));
                }
                Some(resolved_ty)
            } else {
                None
            };
            let site = Expr {
                hir_id: field.pat.hir_id,
                span: field.span,
                kind: H::Unit,
            };
            let method = self
                .selected_method(ty.clone(), Some(iface.clone()), contract.get, field.span)?
                .ok_or_else(|| {
                    error(field.span, "record pattern view has no selected get method")
                })?;
            let call_value = self.finish_call(
                &site,
                method,
                &[],
                vec![receiver.clone(), key_expr],
                None,
                None,
            )?;
            let next = if is_map {
                let payload = self.optional_payload(&call_value.ty, field.span)?;
                if let Some(expected_ty) = &concrete_field_ty {
                    self.require(&payload, expected_ty, field.span)?;
                }
                if is_never(&payload) {
                    return Err(error(
                        field.span,
                        format!("unknown MapPatternView field `{prop_name}` has NoReturn type"),
                    ));
                }
                let inner = self.pattern(&field.pat, &payload)?;
                n::Pattern::OptionalPresent(Box::new(inner))
            } else {
                if let Some(expected_ty) = &concrete_field_ty {
                    self.require(&call_value.ty, expected_ty, field.span)?;
                }
                if is_never(&call_value.ty) {
                    return Err(error(
                        field.span,
                        format!("unknown StructPatternView field `{prop_name}` has NoReturn type"),
                    ));
                }
                self.pattern(&field.pat, &call_value.ty)?
            };
            steps.push(n::Pattern::Then {
                first: Box::new(n::Pattern::Wild),
                value: Box::new(call_value),
                next: Box::new(next),
            });
        }
        while steps.len() > 1 {
            let mut balanced = Vec::with_capacity(steps.len().div_ceil(2));
            let mut pending = steps.into_iter();
            while let Some(first) = pending.next() {
                balanced.push(match pending.next() {
                    Some(next) => n::Pattern::Then {
                        first: Box::new(first),
                        value: Box::new(n::Expr {
                            source: source(bridge_span),
                            ty: p(P::Void),
                            kind: K::Unit,
                        }),
                        next: Box::new(next),
                    },
                    None => first,
                });
            }
            steps = balanced;
        }
        let result = steps.pop().unwrap_or(n::Pattern::Wild);
        Ok(n::Pattern::Observe {
            binding,
            inner: Box::new(result),
        })
    }
}
