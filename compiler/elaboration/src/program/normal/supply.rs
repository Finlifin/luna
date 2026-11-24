//! Resolve the supply interface separately from the selected opaque declaration.
//! A constant can adapt the same code repeatedly through different public slots.
use super::*;
use nir::normal_forms::Collector;

pub(in crate::program) struct Application {
    pub(in crate::program) head: OwnerId,
    pub(in crate::program) interface: Supply,
}

pub(in crate::program) enum Supply {
    Declaration(OwnerId),
    Callable { collector: Option<Collector> },
}

impl std::fmt::Display for Application {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(normal-application (head {}) (interface ", self.head)?;
        match &self.interface {
            Supply::Declaration(owner) => write!(f, "(declaration {owner})")?,
            Supply::Callable { collector } => {
                f.write_str("(callable (collector")?;
                if let Some(entry) = collector {
                    write!(f, " {entry}")?;
                }
                f.write_str("))")?;
            }
        }
        f.write_str("))")
    }
}

fn applied_declaration(value: &Expr<'_>) -> Option<hir::common::DefId> {
    let head = match value.kind {
        H::NFApplication(head, _) => head,
        _ => value,
    };
    match head.kind {
        H::ResolvedDef { def_id, .. } => Some(def_id),
        _ => None,
    }
}

impl<'pkg, 'arena> Checker<'pkg, 'arena> {
    /// The constant can forward the complete declaration interface (including
    /// defaults) only through an unannotated reference to that declaration.
    pub(super) fn constant_normal_supply(
        &mut self,
        mut owner: OwnerId,
        slot: Option<usize>,
        span: Span,
    ) -> Result<Supply, Error> {
        let annotation = loop {
            self.charge_supply_lookup(1, span)?;
            match &self.package.item(owner).expect("resolved declaration").kind {
                ItemKind::Fn(sig, _) if sig.modifiers.is_normal_form_head => {
                    return Ok(Supply::Declaration(owner));
                }
                ItemKind::Const(ty, value) if matches!(ty.kind, H::TyPlaceholder) => {
                    if let H::ResolvedDef { def_id, .. } = value.kind
                        && let Some(next) = self.package.definition_owner(def_id)
                    {
                        owner = next;
                        continue;
                    }
                    break self.value_supply_annotation(value, span)?;
                }
                ItemKind::Const(ty, _) => break Some(*ty),
                _ => break None,
            }
        };
        let collector = self.annotation_collector(annotation, slot, span)?;
        Ok(Supply::Callable { collector })
    }

    pub(super) fn value_collector(
        &mut self,
        source: &Expr<'_>,
        slot: Option<usize>,
        span: Span,
    ) -> Result<Option<Collector>, Error> {
        let annotation = self.value_supply_annotation(source, span)?;
        self.annotation_collector(annotation, slot, span)
    }

    fn value_supply_annotation(
        &mut self,
        source: &Expr<'_>,
        span: Span,
    ) -> Result<Option<&'arena Expr<'arena>>, Error> {
        Ok(match source.kind {
            H::ResolvedFnParam(_, id) => {
                let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
                    self.package.item(id.owner).map(|item| &item.kind)
                else {
                    return Ok(None);
                };
                self.charge_supply_lookup(sig.params.len(), span)?;
                sig.params
                    .iter()
                    .find(|p| p.hir_id == id)
                    .and_then(|p| match p.ty.kind {
                        TyParamKind::Positional(ty)
                        | TyParamKind::PositionalDependencyCatched(_, ty)
                        | TyParamKind::Optional(_, ty, _)
                        | TyParamKind::Varadic(_, ty) => Some(ty),
                        TyParamKind::Itself { .. } | TyParamKind::Case(_) => None,
                    })
            }
            H::Application(callee, _) => match applied_declaration(callee)
                .and_then(|id| self.package.definition_owner(id))
                .and_then(|owner| self.package.item(owner))
                .map(|item| &item.kind)
            {
                Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) => sig.return_ty,
                _ => None,
            },
            _ => None,
        })
    }

    /// Transparent aliases keep their target's supply interface. A computed
    /// Type for which only the telescope is available exposes its checked slot.
    fn annotation_collector(
        &mut self,
        mut annotation: Option<&'arena Expr<'arena>>,
        slot: Option<usize>,
        span: Span,
    ) -> Result<Option<Collector>, Error> {
        let Some(slot) = slot else { return Ok(None) };
        while let Some(ty) = annotation {
            self.charge_supply_lookup(1, span)?;
            if let Some(parameters) = function_types::normal_form_inputs(ty) {
                return match parameters.get(slot).map(|p| &p.kind) {
                    Some(TyParamKind::Varadic(name, _)) => {
                        Ok(Some(Collector::named(slot, name.name.as_str())))
                    }
                    _ => Err(error(
                        span,
                        "public collector disagrees with its checked telescope",
                    )),
                };
            }
            annotation = match applied_declaration(ty)
                .and_then(|id| self.package.definition_owner(id))
                .and_then(|owner| self.package.item(owner))
                .map(|item| &item.kind)
            {
                Some(ItemKind::TypeAlias(target, ..)) => Some(*target),
                _ => None,
            };
        }
        Ok(Some(Collector::positional(slot)))
    }

    fn charge_supply_lookup(&mut self, work: usize, span: Span) -> Result<(), Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(work as u64)
            .ok_or_else(|| error(span, "normal-form interface observation budget exhausted"))?;
        Ok(())
    }
}
