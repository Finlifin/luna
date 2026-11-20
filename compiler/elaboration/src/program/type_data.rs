//! Type-position computation preserves data until a consumer demands an identity.
use super::*;
use comptime_interpretor::Value;

/// A completed Type-valued computation at the elaboration boundary.
/// An identity consumer must explicitly request the identity.
#[derive(Clone)]
pub(super) enum CompletedType {
    Identity(Term),
    Description(n::DescriptionLiteral),
}

impl std::fmt::Display for CompletedType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Identity(ty) => write!(f, "(type-value {ty})"),
            Self::Description(leaf) => std::fmt::Display::fmt(leaf, f),
        }
    }
}

impl CompletedType {
    pub(super) fn from_value(value: Value, span: Span) -> Result<Self, Error> {
        match value {
            Value::Type(ty) => Ok(Self::Identity(ty)),
            Value::Description(leaf) => Ok(Self::Description(leaf)),
            _ => Err(error(span, "expected a completed Type value")),
        }
    }

    pub(super) fn identity(self, span: Span) -> Result<Term, Error> {
        match self {
            Self::Identity(ty) => Ok(ty),
            Self::Description(_) => Err(error(
                span,
                "this use requires a type identity; the completed Type value is a gradual description",
            )),
        }
    }

    pub(super) fn into_expression(self, span: Span) -> n::Expr {
        n::Expr {
            source: source(span),
            ty: p(P::Type),
            kind: match self {
                Self::Identity(ty) => K::TypeValue(ty),
                Self::Description(leaf) => K::Literal(n::Literal::Description(leaf)),
            },
        }
    }
}

impl Checker<'_, '_> {
    pub(super) fn fresh_type_binder(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<nir::DeclarationParameter, Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "Type binder budget exhausted"))?;
        let index = self
            .definition_identities
            .reserve()
            .ok_or_else(|| error(span, "Type binder identity space exhausted"))?;
        Ok(nir::DeclarationParameter {
            declaration: DeclarationId {
                package: declaration(self.package, &self.package_identities, owner).package,
                local: index,
            },
            sort: Sort::Type,
        })
    }

    pub(super) fn fresh_value_binder(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<nir::DeclarationParameter, Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "ComptimeValue binder budget exhausted"))?;
        let index = self
            .definition_identities
            .reserve()
            .ok_or_else(|| error(span, "ComptimeValue binder identity space exhausted"))?;
        Ok(nir::DeclarationParameter {
            declaration: DeclarationId {
                package: declaration(self.package, &self.package_identities, owner).package,
                local: index,
            },
            sort: Sort::ComptimeValue,
        })
    }

    pub(super) fn type_expr(&mut self, expr: &Expr<'_>) -> Result<Term, Error> {
        self.completed_type(expr)?.identity(expr.span)
    }

    pub(super) fn complete_checked_type(
        &mut self,
        expr: &Expr<'_>,
        checked: n::Expr,
    ) -> Result<CompletedType, Error> {
        match checked.kind {
            K::TypeValue(ty) => Ok(CompletedType::Identity(ty)),
            K::Literal(n::Literal::Description(leaf)) => Ok(CompletedType::Description(leaf)),
            _ => CompletedType::from_value(
                self.required_normal_input(expr, checked, Vec::new())?.0,
                expr.span,
            ),
        }
    }

    pub(super) fn completed_type(&mut self, expr: &Expr<'_>) -> Result<CompletedType, Error> {
        if let H::DeliveredType(ty) = &expr.kind {
            return self
                .delivered_type(expr.hir_id, ty, expr.span)
                .map(CompletedType::Identity);
        }
        if let H::ResolvedLocal(_, id) = expr.kind
            && !self.callable_type_inputs.contains_key(&local(id))
            && !self.generator_data.contains_key(&local(id))
            && let Some(ty) = self.observed_local_type(local(id), expr.span)?
        {
            return Ok(CompletedType::Identity(ty));
        }
        if let H::Projection(value, field) = &expr.kind {
            if field.name.as_str() == "Closure"
                && let H::Projection(base, namespace) = &value.kind
                && namespace.name.as_str() == "vars"
                && self.metatype_namespace_receiver(base)?
            {
                return Ok(CompletedType::Identity(Term::ClosureContextType));
            }
        }
        // This is a type position: computed member receivers are required
        // inputs, not runtime values to deliver through Splice. Ordinary
        // expression projections retain the caller's phase in their probe.
        let saved_code = std::mem::replace(&mut self.in_code_body, false);
        let saved_mode = std::mem::replace(&mut self.in_required_comptime, true);
        let projected = self.projected_member_data(expr);
        self.in_required_comptime = saved_mode;
        self.in_code_body = saved_code;
        if let Some(data) = projected? {
            return Ok(data);
        }
        if let Some(ty) = self.observed_projection_type(expr)? {
            return Ok(CompletedType::Identity(ty));
        }
        if self.projected_constant_owner(expr)?.is_some() {
            return CompletedType::from_value(
                self.required_expression(expr, Some(&p(P::Type)))?,
                expr.span,
            );
        }
        if let H::RefinementSubject = &expr.kind {
            return self
                .refinement_subjects
                .last()
                .cloned()
                .map(CompletedType::Identity)
                .ok_or_else(|| error(expr.span, "Itself has no active refinement subject"));
        }
        if let H::RefinedType { requirement, base } = &expr.kind {
            return self.complete_refined_type(expr, base, requirement);
        }
        if let H::Defer(value) = &expr.kind {
            // Signature annotations prepare open computations separately. Other
            // type positions still demand a completed computation here; never
            // manufacture an opaque normal-form head for unavailable inputs.
            return self.completed_type(value);
        }
        if let H::Projection(value, field) = &expr.kind {
            if let Some(value) =
                self.associated_projection(value, field.name.as_str(), expr.span, expr.hir_id)?
            {
                return Self::associated_type_data(value, expr.span);
            }
        }
        match &expr.kind {
            H::ResolvedLocal(_, id)
                if !self.callable_type_inputs.contains_key(&local(*id))
                    && self.generator_data.contains_key(&local(*id)) =>
            {
                match self.generator_data.get(&local(*id)) {
                    Some(Some(value)) if matches!(value.kind, K::TypeValue(_)) => {
                        constants::charge_copy(value, &mut self.required_steps, expr.span)?;
                        let K::TypeValue(ty) = &value.kind else {
                            unreachable!()
                        };
                        Ok(CompletedType::Identity(ty.clone()))
                    }
                    Some(Some(n::Expr {
                        kind: K::Literal(n::Literal::Description(leaf)),
                        ..
                    })) => Ok(CompletedType::Description(leaf.clone())),
                    _ => Err(error(
                        expr.span,
                        "local constant used as a type must contain a completed Type value",
                    )),
                }
            }
            H::Application(..)
            | H::ExtendedApplication(..)
            | H::Projection(..)
            | H::Index(..)
            | H::If(..)
            | H::When(_)
            | H::Match(..)
            | H::Block(_) => {
                // After member/associated-type and observed-field selection,
                // remaining projections are ordinary computations. Evaluate
                // the complete receiver and projection at the required stage;
                // a Type field's declared domain alone is not its identity.
                if !self.generator_data.is_empty() && self.data_consumer_call(expr)?.is_none() {
                    return self.generator_type_data(expr);
                }
                let value = self.required_value(expr, P::Type)?;
                CompletedType::from_value(value, expr.span)
            }
            H::NFApplication(head, args) => self.applied_type_data(expr, head, args),
            H::ResolvedDef { def_id, .. } => {
                let owner = self.package.definition_owner(*def_id).ok_or_else(|| {
                    error(expr.span, "resolved type declaration has no HIR owner")
                })?;
                self.declared_type_data(expr, owner)
            }
            H::Builtin(name) if name.as_str() == "Any" => {
                Ok(CompletedType::Description(n::DescriptionLiteral::Any))
            }
            _ => self.form_type_identity(expr).map(CompletedType::Identity),
        }
    }

    fn declared_type_data(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
    ) -> Result<CompletedType, Error> {
        match self.package.item(owner).map(|item| &item.kind) {
            Some(ItemKind::Associated(_)) => {
                let value = self.lexical_associated_value(owner, expr.span)?;
                Self::associated_type_data(value, expr.span)
            }
            Some(ItemKind::Const(..)) => {
                let value = self.constant_value(owner, expr.span)?;
                self.complete_checked_type(expr, value)
            }
            Some(ItemKind::TypeAlias(target, ..)) => self.bare_alias(owner, target, expr.span),
            Some(ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..)) => {
                if !self.aliases.insert(owner) {
                    return Err(error(expr.span, "cyclic type alias"));
                }
                let result = self.bare_nominal_identity(owner, expr.span);
                self.aliases.remove(&owner);
                result.map(CompletedType::Identity)
            }
            _ => Err(error(
                expr.span,
                "declaration does not produce a supported type",
            )),
        }
    }

    fn associated_type_data(value: n::Expr, span: Span) -> Result<CompletedType, Error> {
        match value.kind {
            K::TypeValue(ty) => Ok(CompletedType::Identity(ty)),
            K::Literal(n::Literal::Description(leaf)) => Ok(CompletedType::Description(leaf)),
            _ => Err(error(
                span,
                "associated value used as a type does not contain a completed Type",
            )),
        }
    }

    fn bare_nominal_identity(&mut self, owner: OwnerId, span: Span) -> Result<Term, Error> {
        let mut arguments =
            self.type_clause_arguments(owner, &[], None, span)
                .map_err(|mut failure| {
                    if failure.message.contains("argument count mismatch") {
                        failure.message = "generic type requires explicit type arguments".into();
                    }
                    failure
                })?;
        self.check_type_application(owner, &arguments, span)?;
        arguments.extend(self.nominal_capture_arguments(owner, span)?);
        Ok(Term::Nominal {
            declaration: declaration(self.package, &self.package_identities, owner),
            arguments,
        })
    }

    fn applied_type_data<'h>(
        &mut self,
        expr: &Expr<'h>,
        head: &Expr<'h>,
        args: &[Arg<'h>],
    ) -> Result<CompletedType, Error> {
        if let H::Builtin(name) = &head.kind {
            if name.as_str() == "BitVec" {
                let [Arg::Positional(width)] = args else {
                    return Err(error(
                        expr.span,
                        "builtin BitVec requires exactly one positional comptime width",
                    ));
                };
                if matches!(
                    &width.kind,
                    H::Lit(hir::Lit {
                        kind: hir::LitKind::Float(_),
                        ..
                    })
                ) {
                    return Err(error(
                        width.span,
                        "BitVec width requires a nonnegative comptime integer",
                    ));
                }
                let exact = if let H::Lit(hir::Lit {
                    kind: hir::LitKind::Integer(spelling),
                    ..
                }) = &width.kind
                {
                    comptime_runtime::Integer::parse_literal(
                        spelling,
                        &mut comptime_runtime::Budget::default(),
                    )
                    .map_err(|e| error(width.span, e.to_string()))?
                } else {
                    let comptime_interpretor::Value::Integer(value) =
                        self.required_value(width, P::Integer)?
                    else {
                        return Err(error(
                            width.span,
                            "BitVec width is still an abstract comptime value; symbolic widths are not supported at this type formation boundary",
                        ));
                    };
                    value
                };
                if exact.is_negative() {
                    return Err(error(
                        width.span,
                        "BitVec width must be a nonnegative comptime integer",
                    ));
                }
                let width = exact
                    .to_u128()
                    .ok()
                    .and_then(|width| usize::try_from(width).ok())
                    .ok_or_else(|| {
                        error(
                            width.span,
                            "BitVec width exceeds the implementation size limit",
                        )
                    })?;
                return Ok(CompletedType::Identity(p(P::BitVec(width))));
            }
        }
        let H::ResolvedDef { def_id, .. } = head.kind else {
            return CompletedType::from_value(self.required_value(expr, P::Type)?, expr.span);
        };
        let owner = self
            .package
            .definition_owner(def_id)
            .ok_or_else(|| error(expr.span, "type definition has no HIR owner"))?;
        if let Some(application) = self.static_normal_application(owner, head.span)? {
            return self.normal_type_annotation(expr, application, args);
        }
        if let Some(ItemKind::TypeAlias(target, ..)) =
            self.package.item(owner).map(|item| &item.kind)
        {
            return self.completed_alias_application(owner, target, args, expr);
        }
        if !matches!(
            self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..))
        ) {
            return Err(error(
                expr.span,
                "this type application requires comptime elaboration not yet implemented",
            ));
        }
        if self.type_parameters[&owner].is_empty() {
            return Err(error(
                expr.span,
                "generic type type argument count mismatch",
            ));
        }
        let mut arguments = self.type_clause_arguments(owner, args, None, expr.span)?;
        self.check_type_application(owner, &arguments, expr.span)?;
        arguments.extend(self.nominal_capture_arguments(owner, expr.span)?);
        Ok(CompletedType::Identity(Term::Nominal {
            declaration: declaration(self.package, &self.package_identities, owner),
            arguments,
        }))
    }
}
