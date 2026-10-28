//! Contextual constants are selected by resolved declarations, never source names.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn is_build_mode(&self, owner: OwnerId) -> bool {
        matches!(self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Const(_, initializer))
                if matches!(&initializer.kind, H::Builtin(name) if name.as_str() == "build_mode"))
    }

    pub(super) fn validate_build_mode(&mut self, owner: OwnerId, span: Span) -> Result<(), Error> {
        let Some(ItemKind::Const(annotation, _)) = self.package.item(owner).map(|item| &item.kind)
        else {
            return Err(error(
                span,
                "build mode marker requires a constant declaration",
            ));
        };
        if !matches!(annotation.kind, H::TyPlaceholder) {
            let ty = self.type_expr(annotation)?;
            self.require(&ty, &p(P::Symbol), span)?;
        }
        Ok(())
    }

    pub(super) fn build_mode(&mut self, expr: &Expr<'_>, owner: OwnerId) -> Result<n::Expr, Error> {
        self.validate_build_mode(owner, expr.span)?;
        let origin = declaration(self.package, &self.package_identities, expr.hir_id.owner);
        let mode = self
            .protocols
            .build_modes
            .get(&origin.package)
            .filter(|mode| !mode.is_empty())
            .ok_or_else(|| {
                error(
                    expr.span,
                    format!(
                        "build mode is unavailable for source package {}",
                        origin.package
                    ),
                )
            })?;
        Ok(n::Expr {
            source: source(expr.span),
            ty: p(P::Symbol),
            kind: K::Literal(n::Literal::Symbol(mode.clone())),
        })
    }
}
