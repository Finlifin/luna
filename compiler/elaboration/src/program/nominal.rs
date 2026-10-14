//! Demand-driven nominal layouts for required computations and forward uses.
use super::*;

pub(super) fn enum_leaves<'h>(variants: &'h [hir::item::Variant<'h>])
    -> Result<(Vec<(&'h hir::item::Variant<'h>, String)>, Vec<String>), Error> {
    let mut pending = variants.iter().rev().map(|variant|(variant,String::new(),0usize)).collect::<Vec<_>>();
    let (mut leaves,mut groups,mut seen) = (Vec::new(),Vec::new(),BTreeSet::new());
    while let Some((variant,prefix,depth)) = pending.pop() {
        if depth >= 64 { return Err(error(variant.span,"enum group nesting limit exceeded")); }
        let name = format!("{prefix}{}",variant.ident.name);
        if !seen.insert(name.clone()) { return Err(error(variant.span,"duplicate enum member")); }
        if let hir::item::VariantKind::SubEnum(children) = &variant.kind {
            groups.push(name.clone());
            for child in children.iter().rev() { pending.push((child,format!("{name}."),depth+1)); }
        } else { leaves.push((variant,name)); }
    }
    Ok((leaves,groups))
}

impl Checker<'_, '_> {
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
                (declaration(self.package, &self.package_identities, owner) == *id).then_some(owner)
            })
            .ok_or_else(|| error(span, "nominal declaration is missing"))?;
        self.ensure_nominal(owner, span)
    }
    pub(super) fn ensure_nominal(&mut self, owner: OwnerId, span: Span) -> Result<(), Error> {
        let id = declaration(self.package, &self.package_identities, owner);
        if self.structs.contains_key(&id) || self.enums.contains_key(&id) {
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
        self.context = self.binder_context(owner);
        let result = (|| {
            let item = self.package.item(owner).unwrap();
            match &item.kind {
                ItemKind::Enum(def) => {
                    let (leaves,groups) = enum_leaves(def.variants)?;
                    let encoded = self.enum_encoding(def, &leaves, item.span)?;
                    let mut names = BTreeSet::new();
                    for nested in &def.nested_items {
                        if let Some(item) = self.package.item(*nested) {
                            names.insert(item.ident.name.to_string());
                        }
                    }
                    for group in &groups {
                        if !names.insert(group.clone()) { return Err(error(item.span,"duplicate enum member")); }
                    }
                    let mut variants = Vec::new();
                    for (variant_index, (variant, full_name)) in leaves.iter().enumerate() {
                        let (shape, fields) = match &variant.kind {
                            hir::item::VariantKind::Pattern(_) | hir::item::VariantKind::Const(_) if encoded.is_some() => {
                                let encoding = encoded.as_ref().unwrap();
                                let fields = encoding.payloads[variant_index].clone();
                                let names = encoding.bits.as_ref().map(|bits| bits.variants()[variant_index].fields()
                                    .iter().map(|field|field.name.clone()).collect::<Vec<_>>()).unwrap_or_default();
                                (if names.is_empty() {n::VariantShape::Unit} else {n::VariantShape::Named(names)}, fields)
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
                    let (encoding, symbol_encoding) = encoded.map(|encoding| (encoding.bits, encoding.symbols)).unwrap_or_default();
                    self.enums.insert(
                        declaration(self.package, &self.package_identities, owner),
                        n::Enum {
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
                    self.enum_sources.insert(id, leaves.into_iter().map(|(variant,_)|variant).collect());
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
        self.active_nominals.remove(&owner);
        result
    }
}
