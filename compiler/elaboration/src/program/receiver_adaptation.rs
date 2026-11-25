//! Three-layer receiver adaptation, type-view tracking, and Deref/DerefMut/Reference resolution.
use super::*;
use crate::implementation::Selection;
use crate::ptc::Budget;
use std::fmt;

pub(super) const MAX_DEREF_DEPTH: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CopyStatus {
    Copy,
    NotCopy,
    Unknown,
}

impl fmt::Display for CopyStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Copy => "(copy-status copy)",
            Self::NotCopy => "(copy-status not-copy)",
            Self::Unknown => "(copy-status unknown)",
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct DerefStep {
    pub(super) target: Term,
    pub(super) interface: n::InterfaceInstance,
    pub(super) method: n::Expr,
    pub(super) mutable: bool,
}

impl fmt::Display for DerefStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(deref-step (mutable {}) (target {}))",
            self.mutable, self.target
        )
    }
}

#[derive(Clone, Debug)]
pub(super) struct ReferenceStep {
    pub(super) target: Term,
    pub(super) method: n::Expr,
}

impl fmt::Display for ReferenceStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(reference-step (target {}))", self.target)
    }
}

#[derive(Clone, Debug)]
pub(super) enum Layer2Step {
    BuiltinPtr(Term),
    TraitDeref(DerefStep),
}

impl fmt::Display for Layer2Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BuiltinPtr(ty) => write!(f, "(layer2-step (builtin-ptr {ty}))"),
            Self::TraitDeref(step) => write!(f, "(layer2-step {step})"),
        }
    }
}

impl Checker<'_, '_> {
    pub(super) fn comptime_pointer_forbidden(&self) -> bool {
        !self.runtime_value_context() && !self.in_code_body
    }

    pub(super) fn require_runtime_pointer_context(
        &self,
        span: Span,
        op: &str,
    ) -> Result<(), Error> {
        if self.comptime_pointer_forbidden() {
            return Err(error(
                span,
                format!(
                    "pointer operations cannot execute at comptime (pointer {op} is forbidden in comptime contexts)"
                ),
            ));
        }
        Ok(())
    }

    pub(super) fn type_copy_status(&mut self, ty: &Term, span: Span) -> Result<CopyStatus, Error> {
        if self.intrinsic_pointer_input(ty) {
            return Ok(CopyStatus::Copy);
        }
        let Some(copy_decl) = self.protocols.copy else {
            return Ok(if self.copy_read_input(ty, span) {
                CopyStatus::Copy
            } else {
                CopyStatus::NotCopy
            });
        };
        let instance = n::InterfaceInstance {
            declaration: copy_decl,
            arguments: vec![],
        };
        match self.symbolic_interface_evidence(ty, &instance) {
            Ok(true) => return Ok(CopyStatus::Copy),
            Ok(false) => {}
            Err(nir::NominalAdmissionError::Unresolved(_)) => return Ok(CopyStatus::Unknown),
            Err(reason) => return Err(error(span, reason.to_string())),
        }
        let resolution = self
            .resolve_trait_capability(&instance, ty, Budget::default())
            .map_err(|reason| capability_query_error(span, reason))?;
        match resolution.selection {
            Selection::Selected(_) => Ok(CopyStatus::Copy),
            Selection::NoApplicableCandidate => Ok(CopyStatus::NotCopy),
            Selection::Deferred { .. } | Selection::UnresolvedInterface | Selection::StaleIndex => {
                Ok(CopyStatus::Unknown)
            }
            Selection::Conflict(_) | Selection::InvalidCandidate(_) => Ok(CopyStatus::NotCopy),
        }
    }

    pub(super) fn normalize_adaptation_key(&mut self, ty: &Term) -> Term {
        self.structural_type(ty, rustc_span::DUMMY_SP)
            .unwrap_or_else(|_| ty.clone())
    }

    pub(super) fn check_deref_cycle(
        &mut self,
        visited: &mut Vec<Term>,
        next_ty: &Term,
        depth: usize,
        span: Span,
    ) -> Result<(), Error> {
        if depth >= MAX_DEREF_DEPTH {
            return Err(error(
                span,
                format!("receiver dereference depth exceeded limit of {MAX_DEREF_DEPTH}"),
            ));
        }
        let key = self.normalize_adaptation_key(next_ty);
        if visited.contains(&key) {
            return Err(error(
                span,
                format!("cyclic Deref chain detected at type `{}`", key),
            ));
        }
        visited.push(key);
        Ok(())
    }

    pub(super) fn query_single_trait_candidate(
        &self,
        subject: &Term,
        interface: &n::InterfaceInstance,
        label: &str,
        span: Span,
    ) -> Result<bool, Error> {
        let mut unresolved = false;
        match self.symbolic_interface_evidence(subject, interface) {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(nir::NominalAdmissionError::Unresolved(_)) => {
                unresolved = true;
            }
            Err(reason) => return Err(error(span, reason.to_string())),
        }
        let resolution = self
            .resolve_trait_capability(interface, subject, Budget::default())
            .map_err(|reason| capability_query_error(span, reason))?;
        match resolution.selection {
            Selection::Selected(_) => Ok(true),
            Selection::NoApplicableCandidate => {
                if unresolved {
                    Err(context_error(
                        span,
                        format!("{label} lookup blocked on unresolved type `{subject}`"),
                    ))
                } else {
                    Ok(false)
                }
            }
            Selection::Deferred { .. } | Selection::UnresolvedInterface | Selection::StaleIndex => {
                Err(context_error(
                    span,
                    format!("{label} lookup blocked on unresolved obligations for `{subject}`"),
                ))
            }
            Selection::Conflict(candidates) => Err(error(
                span,
                format!(
                    "ambiguous {label} implementations for `{subject}` ({} candidates)",
                    candidates.len()
                ),
            )),
            Selection::InvalidCandidate(_) => Err(error(
                span,
                format!("invalid {label} implementation for `{subject}`"),
            )),
        }
    }

    pub(super) fn query_deref_step(
        &mut self,
        subject: &Term,
        mutable: bool,
        span: Span,
    ) -> Result<Option<DerefStep>, Error> {
        if matches!(subject, Term::Pointer(_)) {
            return Ok(None);
        }
        let Some(deref_decl) = self.protocols.deref else {
            return Ok(None);
        };
        let deref_iface = n::InterfaceInstance {
            declaration: deref_decl,
            arguments: vec![],
        };
        if !self.query_single_trait_candidate(subject, &deref_iface, "Deref", span)? {
            return Ok(None);
        }
        self.require_runtime_pointer_context(span, "Deref")?;

        let (selected_iface, method_name) = if mutable {
            let Some(deref_mut_decl) = self.protocols.deref_mut else {
                return Err(error(
                    span,
                    format!(
                        "`{subject}` implements `Deref`, but mutable dereference requires a bound `DerefMut` protocol"
                    ),
                ));
            };
            let deref_mut_iface = n::InterfaceInstance {
                declaration: deref_mut_decl,
                arguments: vec![],
            };
            if !self.query_single_trait_candidate(subject, &deref_mut_iface, "DerefMut", span)? {
                return Err(error(
                    span,
                    format!(
                        "`{subject}` implements `Deref` but not `DerefMut`; cannot mutably dereference"
                    ),
                ));
            }
            (deref_mut_iface, "deref_mut")
        } else {
            (deref_iface, "deref")
        };

        let method = self
            .selected_method(
                subject.clone(),
                Some(selected_iface.clone()),
                method_name,
                span,
            )?
            .ok_or_else(|| {
                error(
                    span,
                    format!("`{method_name}` method on `{subject}` did not resolve"),
                )
            })?;
        let Some(Term::Function { result, .. }) = Some(method.ty.callable_signature()) else {
            return Err(error(
                span,
                format!("`{method_name}` did not resolve to a function signature"),
            ));
        };
        let resolved_result = self.callable_input_view(result.as_ref(), span)?;
        let Term::Pointer(target) = resolved_result else {
            return Err(error(
                span,
                format!("`{method_name}` must return `*Target`, found `{resolved_result}`"),
            ));
        };
        Ok(Some(DerefStep {
            target: *target,
            interface: selected_iface,
            method,
            mutable,
        }))
    }

    pub(super) fn query_reference_step(
        &mut self,
        subject: &Term,
        span: Span,
    ) -> Result<Option<ReferenceStep>, Error> {
        let Some(ref_decl) = self.protocols.reference else {
            return Ok(None);
        };
        let ref_iface = n::InterfaceInstance {
            declaration: ref_decl,
            arguments: vec![],
        };
        if !self.query_single_trait_candidate(subject, &ref_iface, "Reference", span)? {
            return Ok(None);
        }
        self.require_runtime_pointer_context(span, "Reference")?;
        let method = self
            .selected_method(subject.clone(), Some(ref_iface), "reference", span)?
            .ok_or_else(|| {
                error(
                    span,
                    format!("`reference` method on `{subject}` did not resolve"),
                )
            })?;
        let Term::Function { result, .. } = method.ty.callable_signature() else {
            return Err(error(
                span,
                "`Reference.reference` did not resolve to a function signature",
            ));
        };
        let resolved_result = self.callable_input_view(result.as_ref(), span)?;
        let Term::Pointer(target) = resolved_result else {
            return Err(error(
                span,
                format!("`Reference.reference` must return `*Target`, found `{resolved_result}`"),
            ));
        };
        Ok(Some(ReferenceStep {
            target: *target,
            method,
        }))
    }

    pub(super) fn emit_deref_call(
        &mut self,
        site: &Expr<'_>,
        addr: n::Expr,
        step: &DerefStep,
    ) -> Result<n::Expr, Error> {
        self.require_runtime_pointer_context(site.span, "Deref")?;
        self.finish_call(site, step.method.clone(), &[], vec![addr], None, None)
    }

    pub(super) fn emit_reference_call(
        &mut self,
        site: &Expr<'_>,
        addr: n::Expr,
        step: &ReferenceStep,
    ) -> Result<n::Expr, Error> {
        self.require_runtime_pointer_context(site.span, "Reference")?;
        self.finish_call(site, step.method.clone(), &[], vec![addr], None, None)
    }

    pub(super) fn ensure_address_of_value(
        &mut self,
        value: n::Expr,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if let K::ReadPointer(pointer) = value.kind {
            return Ok(*pointer);
        }
        Ok(n::Expr {
            source: source(span),
            ty: Term::Pointer(Box::new(value.ty.clone())),
            kind: K::AddressOf(Box::new(value)),
        })
    }

    pub(super) fn emit_place_read_with_copy_check(
        &mut self,
        pointer: n::Expr,
        span: Span,
        reason: &str,
    ) -> Result<n::Expr, Error> {
        let Term::Pointer(pointee) = &pointer.ty else {
            return Err(error(span, "dereferenced target must have pointer type"));
        };
        match self.type_copy_status(pointee, span)? {
            CopyStatus::Copy => Ok(n::Expr {
                source: source(span),
                ty: (**pointee).clone(),
                kind: K::ReadPointer(Box::new(pointer)),
            }),
            CopyStatus::NotCopy => Err(error(span, format!("{reason}: `{pointee}` is not `Copy`"))),
            CopyStatus::Unknown => Err(context_error(
                span,
                format!("{reason}: `{pointee}` lacks established `Copy` evidence"),
            )),
        }
    }

    pub(super) fn validate_type_view(
        &mut self,
        base: &Term,
        view: &Term,
        span: Span,
    ) -> Result<(), Error> {
        let frames = self.call_input_frames();
        let fuel_limit = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = fuel_limit;
        let forward = nir::newtypes::conversion_with(base, view, &frames, &mut fuel, |id, _| {
            Ok(self.newtypes.get(&id))
        });
        let backward = nir::newtypes::conversion_with(view, base, &frames, &mut fuel, |id, _| {
            Ok(self.newtypes.get(&id))
        });
        self.required_steps -= (fuel_limit - fuel) as u64;
        let forward = forward.map_err(|failure| error(span, failure.to_string()))?;
        let backward = backward.map_err(|failure| error(span, failure.to_string()))?;
        if forward.is_none() || backward.is_none() {
            return Err(error(
                span,
                format!(
                    "type view `{view}` requires a bidirectional newtype relationship with `{base}`"
                ),
            ));
        }
        Ok(())
    }

    pub(super) fn resolve_type_annotation_with_views(
        &mut self,
        expr: &Expr<'_>,
    ) -> Result<(Term, Vec<Term>), Error> {
        if let H::Cast(base, view_expr) = expr.kind {
            let (base_ty, mut views) = self.resolve_type_annotation_with_views(base)?;
            let view_ty = self.type_expr(view_expr)?;
            self.validate_type_view(&base_ty, &view_ty, view_expr.span)?;
            views.push(view_ty);
            return Ok((base_ty, views));
        }
        let ty = self.type_expr(expr)?;
        let alias_views = self.alias_views_for_expr(expr)?;
        Ok((ty, alias_views))
    }

    fn alias_views_for_expr(&mut self, expr: &Expr<'_>) -> Result<Vec<Term>, Error> {
        let def_id = match &expr.kind {
            H::ResolvedDef { def_id, .. } => Some(*def_id),
            H::NFApplication(head, _) | H::Application(head, _) => {
                if let H::ResolvedDef { def_id, .. } = head.kind {
                    Some(def_id)
                } else {
                    None
                }
            }
            _ => None,
        };
        let Some(owner) = def_id.and_then(|id| self.package.definition_owner(id)) else {
            return Ok(Vec::new());
        };
        let Some(views) = self.alias_type_views.get(&owner).cloned() else {
            return Ok(Vec::new());
        };
        Ok(views)
    }

    pub(super) fn receiver_type_views(&self, expr: &Expr<'_>) -> Vec<Term> {
        let binding = match expr.kind {
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => Some(local(id)),
            _ => None,
        };
        binding
            .and_then(|id| self.local_type_views.get(&id))
            .map(|views| views.iter().rev().cloned().collect())
            .unwrap_or_default()
    }
}
