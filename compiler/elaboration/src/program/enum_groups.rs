//! Enum groups organize leaf names without adding types, values or payloads.
use super::*;

pub(super) fn symbol_path(mut expr: &Expr<'_>) -> Option<String> {
    let mut names = Vec::new();
    for _ in 0..128 {
        match &expr.kind {
            H::Projection(base,name) => {names.push(name.name.to_string());expr=base;}
            H::Lit(hir::Lit {kind:hir::LitKind::Symbol(name),..}) => {
                names.push(name.to_string()); names.reverse(); return Some(names.join("."));
            }
            _ => return None,
        }
    }
    None
}

impl Checker<'_, '_> {
    pub(super) fn enum_member_reference(&mut self, expr: &Expr<'_>, expected: Option<&Term>)
        -> Result<Option<(Term,String)>,Error> {
        let mut names = Vec::new();
        let mut cursor = expr;
        let mut target = None;
        for _ in 0..128 {
            match &cursor.kind {
                H::Projection(base,name) => {
                    names.push(name.name.to_string());
                    if self.is_type_head(base) {target=Some(self.type_expr(base)?);break;}
                    cursor=base;
                }
                H::Lit(hir::Lit {kind:hir::LitKind::Symbol(name),..}) => {
                    names.push(name.to_string()); target=expected.cloned(); break;
                }
                _ => return Ok(None),
            }
        }
        let Some(ty) = target else {return Ok(None)};
        let ty = self.structural_type(&ty,expr.span)?;
        self.ensure_nominal_type(&ty,expr.span)?;
        let Term::Nominal {declaration,arguments} = &ty else {return Ok(None)};
        let Some(definition) = self.enums.get(declaration) else {return Ok(None)};
        definition.instantiate_variants_in_scope(arguments,&self.call_input_frames())
            .map_err(|failure|error(expr.span,failure.to_string()))?;
        names.reverse();
        Ok(Some((ty,names.join("."))))
    }

    pub(super) fn enum_group_pattern(&mut self, expr: &Expr<'_>, ty: &Term)
        -> Result<Option<n::Pattern>,Error> {
        let Some((target,name)) = self.enum_member_reference(expr,Some(ty))? else {return Ok(None)};
        let Term::Nominal {declaration,..} = &target else {unreachable!()};
        let definition = &self.enums[declaration];
        if !definition.groups.contains(&name) {return Ok(None)};
        let prefix = format!("{name}.");
        let cost = definition.variants.iter().filter(|variant|variant.name.starts_with(&prefix))
            .fold(definition.variants.len() as u64,|cost,variant|cost.saturating_add(variant.fields.len() as u64 + 1));
        self.required_steps = self.required_steps.checked_sub(cost)
            .ok_or_else(||error(expr.span,"enum group expansion budget exhausted"))?;
        let variants = definition.variants.iter().enumerate()
            .filter(|(_,variant)|variant.name.starts_with(&prefix))
            .map(|(index,variant)|n::Pattern::Variant {declaration:*declaration,index,
                fields:vec![n::Pattern::Wild;variant.fields.len()]}).collect();
        self.require(&target,ty,expr.span)?;
        Ok(Some(n::Pattern::Or(variants)))
    }
}
