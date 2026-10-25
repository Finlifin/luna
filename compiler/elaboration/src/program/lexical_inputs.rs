//! Lift enclosing staged inputs into the local declaration's parameter scope.
use super::*;

impl Checker<'_, '_> {
    /// Distinguish an unimplemented invocation environment from an unresolved
    /// name. Only diagnose after ordinary locals/closure captures were tried:
    /// lambdas already carry invocation values through their own environments.
    pub(super) fn unavailable_lexical_input(&self, binding: hir::HirId) -> &'static str {
        let fallback = "local binding is unavailable in this scope";
        let Some(mut owner) = self.signature_owner else {
            return fallback;
        };
        let mut seen = BTreeSet::new();
        while let Some(parent) = self.package.lexical_parents.get(&owner).copied() {
            if !seen.insert(parent) {
                return fallback;
            }
            if parent == binding.owner {
                if let Some(ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                    self.package.item(parent).map(|item| &item.kind)
                {
                    if signature.modifiers.is_comptime
                        && signature.params.iter().any(|parameter| {
                            parameter.hir_id == binding && !parameter.ty.is_comptime()
                        })
                    {
                        return "capturing an ordinary comptime function parameter in a local declaration requires an invocation environment, which is not yet implemented";
                    }
                }
                return fallback;
            }
            owner = parent;
        }
        fallback
    }

    pub(super) fn lift_lexical_type(
        &self,
        parent: OwnerId,
        ty: &Term,
        span: Span,
    ) -> Result<Term, Error> {
        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            self.package.item(parent).map(|item| &item.kind)
        else {
            return Ok(ty.clone());
        };
        let mut frame = vec![None; sig.params.len()];
        for (binding, id, sort, _) in self.lexical_staged_inputs(parent, span)? {
            let index = sig
                .params
                .iter()
                .position(|parameter| parameter.hir_id == binding)
                .unwrap();
            frame[index] = Some(Term::Rigid {
                declaration: id,
                sort,
            });
        }
        let lifted = ty
            .substitute_partial(&frame)
            .map_err(|e| error(span, e.to_string()))?;
        if !lifted
            .substitution_parameters()
            .map_err(|e| error(span, e.to_string()))?
            .is_empty()
        {
            return Err(error(
                span,
                "lexical type requirement depends on an unavailable runtime input",
            ));
        }
        Ok(lifted)
    }

    pub(super) fn lexical_staged_inputs(
        &self,
        parent: OwnerId,
        span: Span,
    ) -> Result<Vec<(hir::HirId, DeclarationId, Sort, Term)>, Error> {
        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            self.package.item(parent).map(|item| &item.kind)
        else {
            return Ok(Vec::new());
        };
        let domains = if let Some(signature) = self
            .signatures
            .get(&parent)
            .or_else(|| self.interface_signatures.get(&parent))
        {
            let Term::Function { parameters, .. } = signature.callable_signature() else {
                return Ok(Vec::new());
            };
            parameters
                .iter()
                .map(|p| Some((p.sort, p.ty.clone())))
                .collect::<Vec<_>>()
        } else if let Some(definition) = self.open_functions.get(&parent) {
            definition
                .input_domains()
                .into_iter()
                .map(|(sort, domain)| domain.map(|domain| (sort, domain)))
                .collect()
        } else {
            return Err(error(span, "lexical parent signature is unavailable"));
        };
        let mut result = Vec::new();
        let mut frame = vec![None; sig.params.len()];
        for (index, parameter) in sig.params.iter().enumerate() {
            if !parameter.ty.is_comptime() {
                continue;
            }
            let (sort, domain) = domains
                .get(index)
                .and_then(Option::as_ref)
                .ok_or_else(|| error(span, "lexical staged input requires a formed domain"))?;
            let id = self
                .package
                .staged_parameter_definitions
                .get(&parameter.hir_id)
                .ok_or_else(|| error(span, "staged parameter has no source identity"))?;
            let id = DeclarationId {
                package: semantic_package(&self.package_identities, id.pkg),
                local: id.index,
            };
            let domain = domain
                .substitute_partial(&frame)
                .map_err(|e| error(span, e.to_string()))?;
            if !domain
                .substitution_parameters()
                .map_err(|e| error(span, e.to_string()))?
                .is_empty()
            {
                return Err(error(
                    span,
                    "lexical staged input domain depends on an unavailable runtime input",
                ));
            }
            frame[index] = Some(Term::Rigid {
                declaration: id,
                sort: *sort,
            });
            result.push((parameter.hir_id, id, *sort, domain));
        }
        Ok(result)
    }

    pub(super) fn captured_staged_input(
        &mut self,
        binding: hir::HirId,
        span: Span,
    ) -> Result<Option<(Term, Term)>, Error> {
        let Some(mut owner) = self.signature_owner else {
            return Ok(None);
        };
        if owner == binding.owner {
            return Ok(None);
        }
        let Some(id) = self.package.staged_parameter_definitions.get(&binding) else {
            return Ok(None);
        };
        let id = DeclarationId {
            package: semantic_package(&self.package_identities, id.pkg),
            local: id.index,
        };
        let mut seen = BTreeSet::new();
        while let Some(parent) = self.package.lexical_parents.get(&owner).copied() {
            if !seen.insert(parent) {
                return Err(error(span, "cyclic lexical definition environment"));
            }
            if parent == binding.owner {
                let domain = self
                    .lexical_input_domains
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| error(span, "lexical staged input domain is unavailable"))?;
                let domain = self.substituted_trait_domain(&domain, span)?;
                let symbol = if let Some(value) = self.alias_substitution.get(&id).cloned() {
                    self.copy_alias_type(&value, span)?
                } else {
                    Term::Rigid {
                        declaration: id,
                        sort: if domain == p(P::Type) {
                            Sort::Type
                        } else {
                            Sort::ComptimeValue
                        },
                    }
                };
                return Ok(Some((symbol, domain)));
            }
            owner = parent;
        }
        Ok(None)
    }
}
