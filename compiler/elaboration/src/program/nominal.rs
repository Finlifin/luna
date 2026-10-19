//! Demand-driven nominal layouts for required computations and forward uses.
use super::*;

pub(super) fn enum_leaves<'h>(
    variants: &'h [hir::item::Variant<'h>],
) -> Result<(Vec<(&'h hir::item::Variant<'h>, String)>, Vec<String>), Error> {
    let mut pending = variants
        .iter()
        .rev()
        .map(|variant| (variant, String::new(), 0usize))
        .collect::<Vec<_>>();
    let (mut leaves, mut groups, mut seen) = (Vec::new(), Vec::new(), BTreeSet::new());
    while let Some((variant, prefix, depth)) = pending.pop() {
        if depth >= 64 {
            return Err(error(variant.span, "enum group nesting limit exceeded"));
        }
        let name = format!("{prefix}{}", variant.ident.name);
        if !seen.insert(name.clone()) {
            return Err(error(variant.span, "duplicate enum member"));
        }
        if let hir::item::VariantKind::SubEnum(children) = &variant.kind {
            groups.push(name.clone());
            for child in children.iter().rev() {
                pending.push((child, format!("{name}."), depth + 1));
            }
        } else {
            leaves.push((variant, name));
        }
    }
    Ok((leaves, groups))
}

impl Checker<'_, '_> {
    // Source clause arity excludes this lexical environment. Keep the same
    // suffix in declaration telescopes and every nominal value constructor.
    pub(super) fn nominal_captures(
        &self,
        owner: OwnerId,
        _span: Span,
    ) -> Result<Vec<DeclarationId>, Error> {
        if !matches!(
            self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Newtype(..))
        ) {
            return Ok(Vec::new());
        }
        let Some(parent) = self.self_containers.get(&owner) else {
            return Ok(Vec::new());
        };
        let mut captures = Vec::new();
        if matches!(
            self.package.item(*parent).map(|item| &item.kind),
            Some(ItemKind::Trait(_))
        ) {
            captures.push(declaration(self.package, &self.package_identities, *parent));
        }
        for parameter in &self.type_parameters[parent] {
            if !captures.contains(parameter) {
                captures.push(*parameter);
            }
        }
        Ok(captures)
    }

    pub(super) fn nominal_parameter_types(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Vec<Term>, Error> {
        if let Some(parent) = self.self_containers.get(&owner).copied() {
            self.trait_domains(parent)?;
        }
        let mut domains = self.trait_domains(owner)?;
        if domains.is_empty() {
            domains.resize(self.type_parameters[&owner].len(), p(P::Type));
        }
        domains.extend(
            self.nominal_captures(owner, span)?
                .into_iter()
                .map(|id| self.trait_input_domain(id).unwrap_or_else(|| p(P::Type))),
        );
        Ok(domains)
    }

    pub(super) fn nominal_capture_expressions(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Vec<n::Expr>, Error> {
        self.nominal_captures(owner, span)?
            .into_iter()
            .map(|parameter| {
                let domain = self
                    .trait_input_domain(parameter)
                    .unwrap_or_else(|| p(P::Type));
                if domain != p(P::Type) {
                    let ty = self.substituted_trait_domain(&domain, span)?;
                    if let Some(binding) = self
                        .alias_value_bindings
                        .get(&parameter)
                        .copied()
                        .or_else(|| {
                            self.normal_default_bindings
                                .get(&parameter)
                                .map(|(id, _)| *id)
                        })
                    {
                        return Ok(n::Expr {
                            source: source(span),
                            ty,
                            kind: K::Local(binding),
                        });
                    }
                    let value = self
                        .alias_substitution
                        .get(&parameter)
                        .cloned()
                        .ok_or_else(|| {
                            error(
                                span,
                                "captured value input is not yet available in this definition",
                            )
                        })?;
                    let id = self.spread_binding(span)?;
                    let value = self.copy_alias_type(&value, span)?;
                    let root = n::Expr {
                        source: source(span),
                        ty: ty.clone(),
                        kind: K::Local(id),
                    };
                    let site = Expr {
                        hir_id: hir::HirId::new(
                            hir::OwnerId::INVALID,
                            hir::ItemLocalId::new(id.index),
                        ),
                        span,
                        kind: H::Unit,
                    };
                    let (value, ty) =
                        self.required_normal_input(&site, root, vec![(id, ty, value)])?;
                    return self.materialize_generator_data(value, ty, span);
                }
                let kind = if let Some(binding) = self.alias_value_bindings.get(&parameter) {
                    K::Local(*binding)
                } else {
                    let ty = if let Some(value) = self.alias_substitution.get(&parameter).cloned() {
                        self.copy_alias_type(&value, span)?
                    } else {
                        Term::Rigid {
                            declaration: parameter,
                            sort: Sort::Type,
                        }
                    };
                    K::TypeValue(ty)
                };
                Ok(n::Expr {
                    source: source(span),
                    ty: p(P::Type),
                    kind,
                })
            })
            .collect()
    }

    pub(super) fn nominal_capture_arguments(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Vec<Term>, Error> {
        self.nominal_captures(owner, span)?
            .into_iter()
            .map(|parameter| {
                if self.alias_value_bindings.contains_key(&parameter) {
                    return Err(error(
                        span,
                        "captured input is not yet available in this definition",
                    ));
                }
                if let Some(value) = self.alias_substitution.get(&parameter).cloned() {
                    self.copy_alias_type(&value, span)
                } else {
                    Ok(Term::Rigid {
                        declaration: parameter,
                        sort: self.input_binders(&[parameter])[0].sort,
                    })
                }
            })
            .collect()
    }

    pub(super) fn ensure_nominal_type(&mut self, ty: &Term, span: Span) -> Result<(), Error> {
        let Term::Nominal {
            declaration: id, ..
        } = ty
        else {
            return Ok(());
        };
        let owner = self
            .package
            .owners()
            .find_map(|(owner, _)| {
                (matches_declaration(self.package, &self.package_identities, owner, *id))
                    .then_some(owner)
            })
            .ok_or_else(|| error(span, "nominal declaration is missing"))?;
        self.ensure_nominal(owner, span)
    }
    pub(super) fn ensure_nominal(&mut self, owner: OwnerId, span: Span) -> Result<(), Error> {
        let id = declaration(self.package, &self.package_identities, owner);
        if self.structs.contains_key(&id)
            || self.enums.contains_key(&id)
            || self.newtypes.contains_key(&id)
        {
            return Ok(());
        }
        if self.active_nominals.len() >= 64 || !self.active_nominals.insert(owner) {
            return Err(error(
                span,
                "cyclic or excessively deep nominal representation dependency",
            ));
        }
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_locals = std::mem::take(&mut self.locals);
        // Layouts describe the declaration telescope, never the first caller's
        // concrete environment. Instantiation happens at each nominal use.
        let saved_substitution = std::mem::take(&mut self.alias_substitution);
        let saved_bindings = std::mem::take(&mut self.alias_value_bindings);
        self.context = self.binder_context(owner);
        let result = (|| {
            let item = self.package.item(owner).unwrap();
            match &item.kind {
                ItemKind::Newtype(value, ..) => {
                    let representation = self.type_expr(value)?;
                    // Direct representation cycles have no finite underlying value.
                    // Keep nominal identities intact while checking this edge.
                    if let Term::Nominal {
                        declaration: target,
                        ..
                    } = &representation
                    {
                        if let Some(target_owner) =
                            self.package.owners().find_map(|(owner, info)| {
                                (matches_declaration(
                                    self.package,
                                    &self.package_identities,
                                    owner,
                                    *target,
                                ) && matches!(info.node.expect_item().kind, ItemKind::Newtype(..)))
                                .then_some(owner)
                            })
                        {
                            self.ensure_nominal(target_owner, value.span)?;
                        }
                    }
                    let mut type_parameters = self.type_parameters[&owner].clone();
                    let captures = self.nominal_captures(owner, span)?;
                    let capture_count = captures.len();
                    type_parameters.extend(captures);
                    let parameter_types = self.nominal_parameter_types(owner, span)?;
                    self.newtypes.insert(
                        id,
                        nir::newtypes::Newtype {
                            capture_count,
                            parameter_types,
                            declaration: id,
                            name: item.ident.name.to_string(),
                            type_parameters,
                            requirements: self
                                .requirements
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            capabilities: self
                                .capabilities
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            representation,
                        },
                    );
                }
                ItemKind::Enum(def) => {
                    let (leaves, groups) = enum_leaves(def.variants)?;
                    let encoded = self.enum_encoding(def, &leaves, item.span)?;
                    let mut names = BTreeSet::new();
                    for nested in &def.nested_items {
                        if let Some(item) = self.package.item(*nested) {
                            names.insert(item.ident.name.to_string());
                        }
                    }
                    for group in &groups {
                        if !names.insert(group.clone()) {
                            return Err(error(item.span, "duplicate enum member"));
                        }
                    }
                    let mut variants = Vec::new();
                    for (variant_index, (variant, full_name)) in leaves.iter().enumerate() {
                        let (shape, fields) = match &variant.kind {
                            hir::item::VariantKind::Pattern(_)
                            | hir::item::VariantKind::Const(_)
                                if encoded.is_some() =>
                            {
                                let encoding = encoded.as_ref().unwrap();
                                let fields = encoding.payloads[variant_index].clone();
                                let names = encoding
                                    .bits
                                    .as_ref()
                                    .map(|bits| {
                                        bits.variants()[variant_index]
                                            .fields()
                                            .iter()
                                            .map(|field| field.name.clone())
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default();
                                (
                                    if names.is_empty() {
                                        n::VariantShape::Unit
                                    } else {
                                        n::VariantShape::Named(names)
                                    },
                                    fields,
                                )
                            }
                            hir::item::VariantKind::Unit => (n::VariantShape::Unit, Vec::new()),
                            hir::item::VariantKind::Tuple(types) => (
                                n::VariantShape::Tuple,
                                types
                                    .iter()
                                    .map(|ty| self.type_expr(ty))
                                    .collect::<Result<Vec<_>, _>>()?,
                            ),
                            hir::item::VariantKind::Struct(fields) => {
                                let mut names = BTreeSet::new();
                                let mut types = Vec::new();
                                let mut ordered = Vec::new();
                                for field in *fields {
                                    let name = field.ident.name.to_string();
                                    if !names.insert(name.clone()) {
                                        return Err(error(
                                            field.span,
                                            "duplicate enum payload field",
                                        ));
                                    }
                                    ordered.push(name);
                                    types.push(self.type_expr(field.ty)?);
                                }
                                (n::VariantShape::Named(ordered), types)
                            }
                            _ => {
                                return Err(error(
                                    variant.span,
                                    "enum representation elaboration is not implemented",
                                ));
                            }
                        };
                        let name = full_name.clone();
                        if !names.insert(name.clone()) {
                            return Err(error(variant.span, "duplicate enum member"));
                        }
                        variants.push(n::Variant {
                            name,
                            shape,
                            fields,
                        });
                    }
                    let (encoding, symbol_encoding) = encoded
                        .map(|encoding| (encoding.bits, encoding.symbols))
                        .unwrap_or_default();
                    self.enums.insert(
                        declaration(self.package, &self.package_identities, owner),
                        n::Enum {
                            parameter_types: self
                                .trait_parameter_types
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            capabilities: self
                                .capabilities
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            requirements: self
                                .requirements
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            type_parameters: self.type_parameters[&owner].clone(),
                            declaration: declaration(self.package, &self.package_identities, owner),
                            name: item.ident.name.to_string(),
                            variants,
                            groups,
                            encoding,
                            symbol_encoding,
                        },
                    );
                    self.enum_sources
                        .insert(id, leaves.into_iter().map(|(variant, _)| variant).collect());
                }
                ItemKind::Struct(def) => {
                    let mut names = BTreeSet::new();
                    let mut fields = Vec::new();
                    for field in def.fields {
                        let name = field.ident.name.as_str().to_string();
                        if !names.insert(name.clone()) {
                            return Err(error(field.span, "duplicate struct field"));
                        }
                        fields.push((name, self.type_expr(field.ty)?));
                    }
                    self.structs.insert(
                        declaration(self.package, &self.package_identities, owner),
                        n::Struct {
                            parameter_types: self
                                .trait_parameter_types
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            capabilities: self
                                .capabilities
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            requirements: self
                                .requirements
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            type_parameters: self.type_parameters[&owner].clone(),
                            declaration: declaration(self.package, &self.package_identities, owner),
                            name: item.ident.name.as_str().into(),
                            fields,
                        },
                    );
                }
                _ => return Err(error(span, "declaration is not a struct or enum")),
            }
            Ok(())
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        self.alias_substitution = saved_substitution;
        self.alias_value_bindings = saved_bindings;
        self.active_nominals.remove(&owner);
        result
    }
}
