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

/// A driver snapshot; keys are semantic package identities, never resolution IDs.
#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub aliases: BTreeMap<u64, BTreeMap<String, u64>>,
    pub inputs: BTreeMap<u64, BTreeMap<String, (Term, Term)>>,
}

impl Checker<'_, '_> {
    fn build_context_origin(&self, expr: &Expr<'_>) -> Result<u64, Error> {
        let original = self.package.source_origin(expr.hir_id).ok_or_else(|| {
            error(
                expr.span,
                "build context read has no originating source definition",
            )
        })?;
        Ok(semantic_package(&self.package_identities, original.pkg))
    }

    pub(super) fn is_build_context(&self, owner: OwnerId) -> bool {
        matches!(self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Const(_, initializer))
                if matches!(&initializer.kind, H::Builtin(name) if matches!(name.as_str(), "build_mode" | "build_target" | "build_package_metadata")))
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
        if matches!(&initializer.kind, H::Builtin(name) if name.as_str() == "build_package_metadata")
        {
            let ty = self.type_expr(annotation)?;
            self.metadata_shape(&ty, span)?;
            return Ok(());
        }
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
                if matches!(&initializer.kind, H::Builtin(name) if name.as_str() == "build_package_metadata"))
        {
            return self.build_package_metadata(expr, owner);
        }

        if matches!(self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Const(_, initializer))
                if matches!(&initializer.kind, H::Builtin(name) if name.as_str() == "build_target"))
        {
            return self.build_target(expr, owner);
        }
        let origin = self.build_context_origin(expr)?;
        let mode = self
            .protocols
            .build_modes
            .get(&origin)
            .filter(|mode| !mode.is_empty())
            .ok_or_else(|| {
                error(
                    expr.span,
                    format!("build mode is unavailable for source package {}", origin),
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
        let origin = self.build_context_origin(expr)?;
        let target = self
            .protocols
            .build_targets
            .get(&origin)
            .cloned()
            .ok_or_else(|| {
                error(
                    expr.span,
                    format!("build target is unavailable for source package {}", origin),
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

impl Checker<'_, '_> {
    fn metadata_shape(&mut self, ty: &Term, span: Span) -> Result<(Term, Term), Error> {
        let invalid = || {
            error(
                span,
                "package metadata requires comptime fn(str) -> PackageMetadata { features: MetadataFeatures { entries: Object } }",
            )
        };
        let Term::ComptimeFunction(signature) = ty else {
            return Err(invalid());
        };
        let Term::Function { parameters, result } = signature.as_ref() else {
            return Err(invalid());
        };
        if parameters.len() != 1 || parameters[0].ty != p(P::Str) {
            return Err(invalid());
        }
        self.ensure_nominal_type(result, span)?;
        let Term::Nominal {
            declaration,
            arguments,
        } = result.as_ref()
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
        if fields.len() != 1 || fields[0].0 != "features" {
            return Err(invalid());
        }
        let features = fields[0].1.clone();
        self.ensure_nominal_type(&features, span)?;
        let Term::Nominal {
            declaration,
            arguments,
        } = &features
        else {
            return Err(invalid());
        };
        if !arguments.is_empty()
            || self.structs.get(declaration).ok_or_else(invalid)?.fields
                != vec![("entries".into(), p(P::Object))]
        {
            return Err(invalid());
        }
        Ok((result.as_ref().clone(), features))
    }

    fn build_package_metadata(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
    ) -> Result<n::Expr, Error> {
        let origin = self.build_context_origin(expr)?;
        let metadata = self
            .protocols
            .build_metadata
            .clone()
            .ok_or_else(|| error(expr.span, "package metadata input is unavailable"))?;
        let aliases = metadata
            .aliases
            .get(&origin)
            .ok_or_else(|| error(expr.span, "package metadata source owner is unavailable"))?;
        let Some(ItemKind::Const(annotation, _)) = self.package.item(owner).map(|item| &item.kind)
        else {
            unreachable!()
        };
        let ty = self.type_expr(annotation)?;
        let (result, features) = self.metadata_shape(&ty, expr.span)?;
        let mut preparation_fuel = 100_000usize;
        for (alias, provider) in aliases {
            preparation_fuel = preparation_fuel
                .checked_sub(alias.len().saturating_add(1))
                .ok_or_else(|| error(expr.span, "package metadata preparation budget exceeded"))?;
            if let Some(fields) = metadata.inputs.get(provider) {
                for (name, (domain, value)) in fields {
                    preparation_fuel = preparation_fuel
                        .checked_sub(name.len().saturating_add(1))
                        .ok_or_else(|| {
                        error(expr.span, "package metadata preparation budget exceeded")
                    })?;
                    for term in [domain, value] {
                        nir::type_computations::charge_term(term, &mut preparation_fuel).map_err(
                            |_| error(expr.span, "package metadata preparation budget exceeded"),
                        )?;
                    }
                }
            }
        }
        let mut entries = Vec::new();
        for (alias, provider) in aliases {
            let value = if let Some(fields) = metadata.inputs.get(provider) {
                let mut values = Vec::new();
                for (name, (domain, value)) in fields {
                    self.ensure_nominal_type(domain, expr.span)?;
                    values.push(nir::objects::entry(
                        Some(name),
                        domain.clone(),
                        value.clone(),
                    ));
                }
                Some(Term::NominalValue {
                    ty: Box::new(result.clone()),
                    variant: None,
                    fields: vec![Term::NominalValue {
                        ty: Box::new(features.clone()),
                        variant: None,
                        fields: vec![nir::objects::value(values)],
                    }],
                })
            } else {
                None
            };
            entries.push((alias.clone(), *provider, value));
        }
        let binding = local(expr.hir_id);
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::Closure {
                parameters: vec![binding],
                captures: vec![],
                body: std::sync::Arc::new(n::Expr {
                    source: source(expr.span),
                    ty: result,
                    kind: K::BuildMetadata {
                        owner: origin,
                        selector: Box::new(n::Expr {
                            source: source(expr.span),
                            ty: p(P::Str),
                            kind: K::Local(binding),
                        }),
                        entries,
                    },
                }),
            },
        })
    }
}
