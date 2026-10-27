//! Invocation-time clause inputs for already-known comptime callable signatures.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn comptime_function_instance(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        allow_optional: bool,
    ) -> Result<Option<n::Expr>, Error> {
        if self.runtime_value_context() || self.in_code_body {
            return Ok(None);
        }
        let Some(ItemKind::Fn(sig, _)) = self.package.item(owner).map(|item| &item.kind) else {
            return Ok(None);
        };
        if !sig.modifiers.is_comptime
            || sig.modifiers.is_normal_form_head
            || self.open_functions.contains_key(&owner)
        {
            return Ok(None);
        }
        // Clause defaults, inherited inputs, and dependent callable signatures
        // continue through declaration-side elaboration until their plans can
        // also be represented as invocation operands.
        let parameters = self
            .type_parameters
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        if parameters.is_empty()
            || args.len() != parameters.len()
            || self.type_clause_params(owner).len() != parameters.len()
            || !args.iter().all(|arg| matches!(arg, Arg::Positional(_)))
            || self.has_optional_type_clauses(owner)
            || sig
                .params
                .iter()
                .any(|param| matches!(param.ty.kind, TyParamKind::Optional(..)))
        {
            return Ok(None);
        }
        let domains = self.trait_domains(owner)?;
        if domains.iter().all(|domain| *domain == p(P::Type)) {
            return Ok(None);
        }
        let Some(signature) = self.known_callable_signature(owner) else {
            return Ok(None);
        };
        let value_parameters: Vec<_> = parameters
            .iter()
            .zip(&domains)
            .filter_map(|(id, domain)| (*domain != p(P::Type)).then_some(*id))
            .collect();
        if value_parameters.iter().any(|id| {
            signature.mentions_rigid(*id) || domains.iter().any(|domain| domain.mentions_rigid(*id))
        }) {
            return Ok(None);
        }
        if !allow_optional {
            arguments::require_function_value_parameters(sig.params, expr.span)?;
        }
        let mut operands = Vec::with_capacity(args.len());
        let mut symbols = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            let Arg::Positional(value) = arg else {
                unreachable!()
            };
            let domain = domains[index]
                .instantiate_parameters_in_scope(
                    &self.input_binders(&parameters[..index]),
                    &symbols,
                    &self.call_input_frames(),
                )
                .map_err(|failure| error(value.span, failure.to_string()))?;
            let operand = if domain == p(P::Type) {
                let ty = self.type_expr(value)?;
                symbols.push(ty.clone());
                n::Expr {
                    source: source(value.span),
                    ty: p(P::Type),
                    kind: K::TypeValue(ty),
                }
            } else {
                symbols.push(Term::Rigid {
                    declaration: parameters[index],
                    sort: Sort::ComptimeValue,
                });
                self.expr(value, Some(&domain))?
            };
            operands.push(operand);
        }
        let signature = signature
            .instantiate_parameters_in_scope(
                &self.input_binders(&parameters),
                &symbols,
                &self.call_input_frames(),
            )
            .map_err(|failure| error(expr.span, failure.to_string()))?;
        let ty = self.callable_type(owner, signature);
        if let Some(expected) = expected {
            self.require(&ty, expected, expr.span)?;
        }
        Ok(Some(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::ComptimeFunctionInstance {
                declaration: declaration(self.package, &self.package_identities, owner),
                subject: Box::new(n::Expr {
                    source: source(expr.span),
                    ty: Term::Tuple(operands.iter().map(|value| value.ty.clone()).collect()),
                    kind: K::Tuple(operands),
                }),
            },
        }))
    }
}
