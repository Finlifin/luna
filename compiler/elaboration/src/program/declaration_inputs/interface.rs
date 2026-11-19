//! Call-local supply projection and source default providers. This is not the
//! stable callable interface: source references stay in the checking region.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct DefaultSource<'h> {
    pub owner: OwnerId,
    pub slot: usize,
    pub expression: &'h Expr<'h>,
}

pub(super) struct Interface<'h> {
    entries: Vec<input_routing::Entry<hir::Symbol>>,
    defaults: Vec<Option<DefaultSource<'h>>>,
}

impl<'h> Interface<'h> {
    pub(super) fn callable(
        count: usize,
        collector: Option<&nir::normal_forms::Collector>,
        span: Span,
    ) -> Result<Self, Error> {
        if collector.is_some_and(|entry| entry.slot.checked_add(1) != Some(count)) {
            return Err(error(
                span,
                "normal-form callable collector must be the final input",
            ));
        }
        let mut entries = vec![
            input_routing::Entry {
                post_target: false,
                ..input_routing::Entry::anchored()
            };
            count
        ];
        if let Some(collector) = collector {
            let entry = &mut entries[collector.slot];
            entry.variadic = true;
            entry.name = collector.name.as_deref().map(hir::Symbol::intern);
        }
        Ok(Self {
            entries,
            defaults: vec![None; count],
        })
    }

    pub(super) fn declaration(owner: OwnerId, clauses: &[hir::clause::ClauseParam<'h>]) -> Self {
        Self {
            entries: clause_inputs::entries(clauses),
            defaults: clauses
                .iter()
                .enumerate()
                .map(|(slot, clause)| match clause.kind {
                    hir::clause::ClauseParamKind::Optional(_, _, expression) => {
                        Some(DefaultSource {
                            owner,
                            slot,
                            expression,
                        })
                    }
                    _ => None,
                })
                .collect(),
        }
    }

    pub(super) fn entries(&self) -> &[input_routing::Entry<hir::Symbol>] {
        &self.entries
    }

    pub(super) fn default(&self, slot: usize) -> Option<DefaultSource<'h>> {
        self.defaults[slot]
    }

    pub(super) fn is_collector(&self, slot: usize) -> bool {
        self.entries[slot].variadic
    }
}

impl std::fmt::Display for Interface<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(input-interface-projection")?;
        for (slot, entry) in self.entries.iter().enumerate() {
            write!(f, " (slot {slot} (name")?;
            if let Some(name) = entry.name {
                write!(f, " {:?}", name.as_str())?;
            }
            write!(
                f,
                ") (optional {}) (collector {})",
                entry.optional, entry.variadic
            )?;
            if let Some(provider) = self.defaults[slot] {
                write!(
                    f,
                    " (default (owner {}) (slot {}) (source {}))",
                    provider.owner,
                    provider.slot,
                    source(provider.expression.span)
                )?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positional_projection_has_no_invented_names_or_default_providers() {
        let span = Span::default();
        let interface =
            Interface::callable(2, Some(&nir::normal_forms::Collector::positional(1)), span)
                .unwrap();
        assert!(interface.entries().iter().all(|entry| entry.name.is_none()));
        assert!(interface.default(0).is_none());
        assert!(interface.default(1).is_none());
        assert!(!interface.is_collector(0));
        assert!(interface.is_collector(1));
        assert!(
            Interface::callable(2, Some(&nir::normal_forms::Collector::positional(0)), span)
                .is_err()
        );
        assert!(
            Interface::callable(0, Some(&nir::normal_forms::Collector::positional(0)), span)
                .is_err()
        );
        assert!(
            Interface::callable(
                2,
                Some(&nir::normal_forms::Collector::positional(usize::MAX)),
                span
            )
            .is_err()
        );
        assert_eq!(
            interface.to_string(),
            "(input-interface-projection (slot 0 (name) (optional false) (collector false)) (slot 1 (name) (optional false) (collector true)))"
        );
    }

    #[test]
    fn declaration_projection_retains_the_default_source_and_escapes_names() {
        let span = Span::default();
        let owner = hir::OwnerId::new(hir::LocalDefId::new(1));
        let hir_id = hir::HirId::new(owner, hir::ItemLocalId::new(2));
        let expression = Expr {
            hir_id,
            span,
            kind: H::Unit,
        };
        let name = hir::common::Ident::new(hir::Symbol::intern("default\"\n"), span);
        let interface = Interface::declaration(
            owner,
            &[hir::clause::ClauseParam {
                hir_id,
                definition: None,
                name,
                kind: hir::clause::ClauseParamKind::Optional(name, &expression, &expression),
                span,
            }],
        );
        let provider = interface.default(0).unwrap();
        assert_eq!(provider.owner, owner);
        assert_eq!(provider.slot, 0);
        assert!(std::ptr::eq(provider.expression, &expression));
        let dump = interface.to_string();
        assert!(dump.contains("default\\\"\\n"), "{dump}");
        assert!(dump.contains("(default (owner "), "{dump}");
        assert!(!dump.contains('\n'), "{dump}");
    }
}
