//! Contextual constants are selected by resolved declarations, never source names.
use super::*;

/// Explicit host pipeline facts. The execution host is deliberately absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub arch: String,
    pub os: String,
    pub layout: nir::target::TargetData,
    pub features: BTreeMap<String, bool>,
}
impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(build-target (arch {}) (os {}) {} (features",
            nir::QuotedDisplay(&self.arch),
            nir::QuotedDisplay(&self.os),
            self.layout
        )?;
        for (name, enabled) in &self.features {
            write!(f, " ({} {enabled})", nir::QuotedDisplay(name))?;
        }
        f.write_str("))")
    }
}

impl Checker<'_, '_> {
    pub(super) fn is_build_context(&self, owner: OwnerId) -> bool {
        matches!(self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Const(_, initializer))
                if matches!(&initializer.kind, H::Builtin(name) if matches!(name.as_str(), "build_mode" | "build_target")))
    }

    pub(super) fn validate_build_context(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<(), Error> {
        let Some(ItemKind::Const(annotation, initializer)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            return Err(error(
                span,
                "build mode marker requires a constant declaration",
            ));
        };
        if matches!(&initializer.kind, H::Builtin(name) if name.as_str() == "build_target") {
            let ty = self.type_expr(annotation)?;
            self.target_shape(&ty, span)?;
            return Ok(());
        }
        if !matches!(annotation.kind, H::TyPlaceholder) {
            let ty = self.type_expr(annotation)?;
            self.require(&ty, &p(P::Symbol), span)?;
        }
        Ok(())
    }

    pub(super) fn build_context_value(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
    ) -> Result<n::Expr, Error> {
        self.validate_build_context(owner, expr.span)?;
        if matches!(self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Const(_, initializer))
                if matches!(&initializer.kind, H::Builtin(name) if name.as_str() == "build_target"))
        {
            return self.build_target(expr, owner);
        }
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

impl Checker<'_, '_> {
    // Check the trusted marker's public value layout before constructing any NIR.
    fn target_shape(
        &mut self,
        ty: &Term,
        span: Span,
    ) -> Result<(DeclarationId, Term, DeclarationId), Error> {
        self.ensure_nominal_type(ty, span)?;
        let invalid = || {
            error(
                span,
                "build target requires Target { arch: Symbol, os: Symbol, pointer_bits: Integer, features: TargetFeatures { entries: Object } }",
            )
        };
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Err(invalid());
        };
        if !arguments.is_empty() {
            return Err(invalid());
        }
        let fields = self
            .structs
            .get(declaration)
            .ok_or_else(invalid)?
            .fields
            .clone();
        if fields.len() != 4
            || fields[0] != ("arch".into(), p(P::Symbol))
            || fields[1] != ("os".into(), p(P::Symbol))
            || fields[2] != ("pointer_bits".into(), p(P::Integer))
            || fields[3].0 != "features"
        {
            return Err(invalid());
        }
        let feature_ty = fields[3].1.clone();
        self.ensure_nominal_type(&feature_ty, span)?;
        let Term::Nominal {
            declaration: feature_id,
            arguments,
        } = &feature_ty
        else {
            return Err(invalid());
        };
        if !arguments.is_empty()
            || self.structs.get(feature_id).ok_or_else(invalid)?.fields
                != vec![("entries".into(), p(P::Object))]
        {
            return Err(invalid());
        }
        Ok((*declaration, feature_ty.clone(), *feature_id))
    }

    fn build_target(&mut self, expr: &Expr<'_>, owner: OwnerId) -> Result<n::Expr, Error> {
        let origin = declaration(self.package, &self.package_identities, expr.hir_id.owner);
        let target = self
            .protocols
            .build_targets
            .get(&origin.package)
            .cloned()
            .ok_or_else(|| {
                error(
                    expr.span,
                    format!(
                        "build target is unavailable for source package {}",
                        origin.package
                    ),
                )
            })?;
        if target.arch.is_empty() || target.os.is_empty() || target.features.contains_key("") {
            return Err(error(
                expr.span,
                "build target contains an empty configuration name",
            ));
        }
        if self.target.is_some_and(|layout| layout != target.layout) {
            return Err(error(
                expr.span,
                "build target layout conflicts with the compilation target",
            ));
        }
        let Some(ItemKind::Const(annotation, _)) = self.package.item(owner).map(|item| &item.kind)
        else {
            unreachable!()
        };
        let ty = self.type_expr(annotation)?;
        let (declaration, feature_ty, feature_id) = self.target_shape(&ty, expr.span)?;
        let make = |ty, kind| n::Expr {
            source: source(expr.span),
            ty,
            kind,
        };
        let entries = make(
            p(P::Object),
            K::Object {
                names: target.features.keys().cloned().map(Some).collect(),
                fields: target
                    .features
                    .values()
                    .map(|value| make(p(P::Bool), K::Literal(n::Literal::Bool(*value))))
                    .collect(),
            },
        );
        Ok(make(
            ty,
            K::Record {
                declaration,
                fields: vec![
                    make(p(P::Symbol), K::Literal(n::Literal::Symbol(target.arch))),
                    make(p(P::Symbol), K::Literal(n::Literal::Symbol(target.os))),
                    make(
                        p(P::Integer),
                        K::Literal(n::Literal::Integer(
                            target.layout.pointer_bits().to_string(),
                        )),
                    ),
                    make(
                        feature_ty,
                        K::Record {
                            declaration: feature_id,
                            fields: vec![entries],
                        },
                    ),
                ],
            },
        ))
    }
}
