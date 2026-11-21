//! Enum groups organize leaf names without adding types, values or payloads.
use super::*;

// A literal Symbol has no independent call signature. Its constructor and
// the literal itself need the destination before selecting a nominal member.
// Projections can also name ordinary methods on the Symbol receiver.
pub(super) fn has_bare_symbol_head(expr: &Expr<'_>) -> bool {
    let head = match expr.kind {
        H::Application(head, _) | H::ExtendedApplication(head, _) => head,
        _ => expr,
    };
    matches!(
        head.kind,
        H::Lit(hir::Lit {
            kind: hir::LitKind::Symbol(_),
            ..
        })
    )
}

pub(super) fn symbol_path(mut expr: &Expr<'_>) -> Option<String> {
    let mut names = Vec::new();
    for _ in 0..128 {
        match &expr.kind {
            H::Projection(base, name) => {
                names.push(name.name.to_string());
                expr = base;
            }
            H::Lit(hir::Lit {
                kind: hir::LitKind::Symbol(name),
                ..
            }) => {
                names.push(name.to_string());
                names.reverse();
                return Some(names.join("."));
            }
            _ => return None,
        }
    }
    None
}

impl Checker<'_, '_> {
    // Discover the namespace before demanding a Type identity. Only an
    // established enum member commits to ordinary required type formation.
    // An unknown value is not proof of a non-enum type; ordinary member/call
    // checking still owns the remaining route.
    pub(super) fn observed_enum_member_reference(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<Option<(Term, String)>, Error> {
        self.enum_member_reference_in(expr, expected, true)
    }

    pub(super) fn enum_member_reference(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<Option<(Term, String)>, Error> {
        self.enum_member_reference_in(expr, expected, false)
    }

    fn enum_member_reference_in(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        observe_only: bool,
    ) -> Result<Option<(Term, String)>, Error> {
        let mut names = Vec::new();
        let mut cursor = expr;
        let mut target = None;
        for _ in 0..128 {
            match &cursor.kind {
                H::Projection(base, name) => {
                    names.push(name.name.to_string());
                    if self.is_type_receiver(base)? {
                        target = if observe_only {
                            self.observed_enum_subject(base, &names)?
                        } else {
                            // A pattern already commits to an enum namespace;
                            // its subject identity must be established here.
                            Some(self.type_expr(base)?)
                        };
                        break;
                    }
                    cursor = base;
                }
                H::Lit(hir::Lit {
                    kind: hir::LitKind::Symbol(name),
                    ..
                }) => {
                    names.push(name.to_string());
                    target = expected.cloned();
                    break;
                }
                _ => return Ok(None),
            }
        }
        let Some(ty) = target else { return Ok(None) };
        let ty = self.structural_type(&ty, expr.span)?;
        self.ensure_nominal_type(&ty, expr.span)?;
        let Term::Nominal { declaration, .. } = &ty else {
            return Ok(None);
        };
        if !self.enums.contains_key(declaration) {
            return Ok(None);
        }
        self.validate_enum_payloads(&ty, expr.span)?;
        names.reverse();
        Ok(Some((ty, names.join("."))))
    }

    fn observed_enum_subject(
        &mut self,
        base: &Expr<'_>,
        reversed_names: &[String],
    ) -> Result<Option<Term>, Error> {
        let Some(checked) =
            self.checking_attempt(base.span, true, CheckingDisposition::Probe, |checker| {
                checker.with_staged_input(true, |checker| checker.expr(base, Some(&p(P::Type))))
            })?
        else {
            return Ok(None);
        };
        let observed = self.observed_expression_type(&checked, base.span)?;
        let declaration = if let Some(ty) = observed {
            let shape = self.structural_type(&ty, base.span)?;
            self.ensure_nominal_type(&shape, base.span)?;
            match shape {
                Term::Nominal { declaration, .. } => Some(declaration),
                _ => None,
            }
        } else {
            // A nominal constructor fixes its namespace even while argument
            // values are pending. Retain that precedence; do not reroute a
            // declared variant to a same-named method on the Type value.
            let mut value = &checked;
            loop {
                self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                    error(base.span, "enum namespace observation budget exhausted")
                })?;
                match &value.kind {
                    K::NominalType { declaration, .. } => break Some(*declaration),
                    K::Block {
                        result: Some(result),
                        ..
                    }
                    | K::CallPreparation {
                        result: Some(result),
                        ..
                    } => value = result,
                    _ => break None,
                }
            }
        };
        let Some(definition) = declaration.and_then(|declaration| self.enums.get(&declaration))
        else {
            return Ok(None);
        };
        let work = reversed_names.iter().try_fold(
            definition
                .groups
                .len()
                .saturating_add(definition.variants.len()) as u64,
            |work, name| work.checked_add(name.len() as u64 + 1),
        );
        self.required_steps = work
            .and_then(|work| self.required_steps.checked_sub(work))
            .ok_or_else(|| error(base.span, "enum namespace observation budget exhausted"))?;
        let name = reversed_names
            .iter()
            .rev()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(".");
        if !definition.groups.contains(&name)
            && !definition
                .variants
                .iter()
                .any(|variant| variant.name == name)
        {
            return Ok(None);
        }
        // Namespace observation is not execution. Once this is a constructor
        // or group, use ordinary required type formation so input producers,
        // contracts and failures are not erased by a known identity.
        self.type_expr(base).map(Some)
    }

    pub(super) fn enum_group_pattern(
        &mut self,
        expr: &Expr<'_>,
        ty: &Term,
    ) -> Result<Option<n::Pattern>, Error> {
        let Some((target, name)) = self.enum_member_reference(expr, Some(ty))? else {
            return Ok(None);
        };
        let Term::Nominal { declaration, .. } = &target else {
            unreachable!()
        };
        let definition = &self.enums[declaration];
        if !definition.groups.contains(&name) {
            return Ok(None);
        };
        let prefix = format!("{name}.");
        let cost = definition
            .variants
            .iter()
            .filter(|variant| variant.name.starts_with(&prefix))
            .fold(definition.variants.len() as u64, |cost, variant| {
                cost.saturating_add(variant.fields.len() as u64 + 1)
            });
        self.required_steps = self
            .required_steps
            .checked_sub(cost)
            .ok_or_else(|| error(expr.span, "enum group expansion budget exhausted"))?;
        let variants = definition
            .variants
            .iter()
            .enumerate()
            .filter(|(_, variant)| variant.name.starts_with(&prefix))
            .map(|(index, variant)| n::Pattern::Variant {
                declaration: *declaration,
                index,
                fields: vec![n::Pattern::Wild; variant.fields.len()],
            })
            .collect();
        self.require(&target, ty, expr.span)?;
        Ok(Some(n::Pattern::Or(variants)))
    }
}
