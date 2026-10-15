//! Source trait input domains and declaration-ordered supplied/default indices.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn trait_domains(&mut self, owner: OwnerId) -> Result<Vec<Term>, Error> {
        if let Some(domains) = self.trait_parameter_types.get(&owner) {
            return Ok(domains.clone());
        }
        if !matches!(
            self.package.item(owner).map(|i| &i.kind),
            Some(ItemKind::Trait(_))
        ) {
            return Ok(Vec::new());
        }
        let mut domains = Vec::new();
        for clause in self.type_clause_params(owner) {
            domains.push(match clause.kind {
                hir::clause::ClauseParamKind::Positional(_, ty)
                | hir::clause::ClauseParamKind::Optional(_, ty, _) => self.type_expr(ty)?,
                _ => p(P::Type),
            });
        }
        self.trait_parameter_types.insert(owner, domains.clone());
        Ok(domains)
    }

    pub(super) fn declaration_binders(&self, owner: OwnerId) -> Vec<nir::DeclarationParameter> {
        self.input_binders(&self.type_parameters[&owner])
    }

    pub(super) fn input_binders(
        &self,
        parameters: &[DeclarationId],
    ) -> Vec<nir::DeclarationParameter> {
        parameters
            .iter()
            .map(|id| nir::DeclarationParameter {
                declaration: *id,
                sort: if self
                    .trait_input_domain(*id)
                    .is_some_and(|ty| ty != p(P::Type))
                {
                    Sort::ComptimeValue
                } else {
                    Sort::Type
                },
            })
            .collect()
    }

    pub(super) fn trait_input_domain(&self, id: DeclarationId) -> Option<Term> {
        self.trait_parameter_types
            .iter()
            .find_map(|(owner, types)| {
                self.type_parameters[owner]
                    .iter()
                    .position(|input| *input == id)
                    .map(|index| types[index].clone())
            })
    }

    pub(super) fn check_trait_defaults(
        &mut self,
        owner: OwnerId,
        domains: &[Term],
    ) -> Result<(), Error> {
        let clauses = self.type_clause_params(owner);
        let parameters = self.type_parameters[&owner].clone();
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_bindings = std::mem::take(&mut self.normal_default_bindings);
        let saved_aliases = std::mem::take(&mut self.alias_substitution);
        let saved_forbidden = self.forbidden_default_inputs.clone();
        let saved_mode = std::mem::replace(&mut self.in_required_comptime, true);
        self.context = self.definition_context(owner);
        let result = (|| {
            for (slot, clause) in clauses.iter().enumerate() {
                if let hir::clause::ClauseParamKind::Optional(_, _, value) = clause.kind {
                    self.forbidden_default_inputs
                        .extend(parameters[slot..].iter().copied());
                    self.expr(value, Some(&domains[slot]))?;
                    self.forbidden_default_inputs = saved_forbidden.clone();
                }
                if domains[slot] != p(P::Type) {
                    let binding = self.spread_binding(clause.span)?;
                    self.locals.insert(binding, domains[slot].clone());
                    self.normal_default_bindings
                        .insert(parameters[slot], (binding, domains[slot].clone()));
                }
            }
            Ok(())
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        self.normal_default_bindings = saved_bindings;
        self.alias_substitution = saved_aliases;
        self.forbidden_default_inputs = saved_forbidden;
        self.in_required_comptime = saved_mode;
        result
    }

    pub(super) fn trait_clause_arguments(
        &mut self,
        owner: OwnerId,
        args: &[Arg<'_>],
        subject: Option<&Term>,
        span: Span,
        domains: &[Term],
    ) -> Result<Vec<Term>, Error> {
        let clauses = self.type_clause_params(owner);
        let parameters = self.type_parameters[&owner].clone();
        let mut supplied = vec![None; clauses.len()];
        let mut positional = 0;
        // Explicit inputs run in caller/source order. Defaults run afterwards
        // in the definition environment; no caller name becomes a default.
        for arg in args {
            let (name, value) = match arg {
                Arg::Positional(value) => (None, *value),
                Arg::Named(name, value) => (Some(*name), *value),
                _ => {
                    return Err(error(
                        span,
                        "mixed trait input spreads require a typed input packet",
                    ));
                }
            };
            let slot = if let Some(name) = name {
                clauses
                    .iter()
                    .position(|clause| {
                        clause.name.name == name.name
                            && matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..))
                    })
                    .ok_or_else(|| error(span, "unknown named trait argument"))?
            } else {
                while positional < clauses.len()
                    && matches!(
                        clauses[positional].kind,
                        hir::clause::ClauseParamKind::Optional(..)
                    )
                {
                    positional += 1;
                }
                let slot = positional;
                positional += 1;
                slot
            };
            let domain = domains
                .get(slot)
                .ok_or_else(|| error(span, "trait argument count mismatch"))?;
            if supplied[slot].is_some() {
                return Err(error(span, "duplicate trait argument"));
            }
            supplied[slot] = Some(self.normal_value_argument(value, domain)?);
        }
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_aliases = self.alias_substitution.clone();
        let saved_forbidden = self.forbidden_default_inputs.clone();
        self.context = self.definition_context(owner);
        if let Some(subject) = subject {
            self.alias_substitution.insert(
                declaration(self.package, &self.package_identities, owner),
                std::sync::Arc::new(subject.clone()),
            );
        }
        let result = (|| {
            let mut arguments = Vec::new();
            for (slot, supplied) in supplied.into_iter().enumerate() {
                let value = if let Some(value) = supplied {
                    value
                } else {
                    let hir::clause::ClauseParamKind::Optional(_, _, value) = clauses[slot].kind
                    else {
                        return Err(error(span, "trait argument count mismatch"));
                    };
                    if self.active_type_defaults.len() >= 64
                        || !self.active_type_defaults.insert((owner, slot))
                    {
                        return Err(error(span, "cyclic or excessively deep trait default"));
                    }
                    self.forbidden_default_inputs
                        .extend(parameters[slot..].iter().copied());
                    let result = self.normal_value_argument(value, &domains[slot]);
                    self.forbidden_default_inputs = saved_forbidden.clone();
                    self.active_type_defaults.remove(&(owner, slot));
                    result?
                };
                self.alias_substitution
                    .insert(parameters[slot], std::sync::Arc::new(value.clone()));
                arguments.push(value);
            }
            Ok(arguments)
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.alias_substitution = saved_aliases;
        self.forbidden_default_inputs = saved_forbidden;
        result
    }
}
